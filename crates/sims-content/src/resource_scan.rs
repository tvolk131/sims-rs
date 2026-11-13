//! Bounded handles for a descriptor scan or mounted resource owner. Bytes, parse results,
//! errors and resource IDs are never cached. Each read revalidates the current
//! pathname, then uses Resource's ordinary bounds/seek/read checks. This avoids
//! repeatedly opening the same FAR without assuming its contents are immutable.
use crate::Resource;
use anyhow::Result;

#[cfg(unix)]
use {
    anyhow::Context,
    std::{
        collections::VecDeque,
        fs::{File, Metadata},
        os::unix::fs::MetadataExt,
        path::PathBuf,
    },
};

#[cfg(unix)]
const CAPACITY: usize = 8;

#[cfg(unix)]
#[derive(PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
    user: u32,
    group: u32,
}
#[cfg(unix)]
impl From<Metadata> for Stamp {
    fn from(m: Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
            mode: m.mode(),
            user: m.uid(),
            group: m.gid(),
        }
    }
}
#[cfg(unix)]
struct Opened {
    source: PathBuf,
    stamp: Stamp,
    file: File,
}

#[derive(Default)]
pub(crate) struct Scan {
    inputs: Option<Observed>,
    #[cfg(unix)]
    files: VecDeque<Opened>,
    #[cfg(test)]
    opens: usize,
}
struct Observed {
    inputs: crate::startup_inputs::Session,
    scope: u64,
}
impl Drop for Observed {
    fn drop(&mut self) {
        self.inputs.end(self.scope);
    }
}
impl Scan {
    pub(crate) fn observed(inputs: &crate::startup_inputs::Session) -> Result<Self> {
        Ok(Self {
            inputs: Some(Observed {
                inputs: inputs.clone(),
                scope: inputs.begin()?,
            }),
            ..Self::default()
        })
    }
    #[cfg(all(test, unix))]
    pub(crate) fn open_count(&self) -> usize {
        self.opens
    }

    pub(crate) fn read(&mut self, resource: &Resource) -> Result<Vec<u8>> {
        if let Some(observed) = &self.inputs {
            return observed.inputs.read(observed.scope, resource);
        }
        #[cfg(not(unix))]
        return resource.read();
        #[cfg(unix)]
        {
            resource.validate_storage()?;
            if resource.owned.is_some() || !resource.archived {
                return resource.read();
            }
            // A failed pathname lookup must use the ordinary open/error path.
            // Revalidating metadata also catches replacement, symlink changes,
            // writes and permission/ACL changes before considering a retained fd.
            let stamp = std::fs::metadata(&resource.source)
                .ok()
                .filter(|m| m.is_file())
                .map(Stamp::from);
            if stamp.is_none() {
                self.files.retain(|f| f.source != resource.source);
                return resource.read();
            }
            if let Some(at) = self
                .files
                .iter()
                .position(|f| f.source == resource.source && stamp.as_ref() == Some(&f.stamp))
            {
                let mut opened = self.files.remove(at).unwrap();
                let result = resource.read_file(&mut opened.file);
                if result.is_ok() {
                    self.files.push_back(opened);
                }
                return result;
            }
            self.files.retain(|f| f.source != resource.source);
            #[cfg(test)]
            {
                self.opens += 1;
            }
            let mut file = File::open(&resource.source)
                .with_context(|| format!("open {}", resource.source.display()))?;
            // Use the opened file's identity, not the earlier pathname's: a
            // replacement between stat and open must not label the fd incorrectly.
            let opened_stamp = file
                .metadata()
                .ok()
                .filter(|m| m.is_file())
                .map(Stamp::from);
            let result = resource.read_file(&mut file);
            if result.is_ok() {
                if let Some(stamp) = opened_stamp {
                    if self.files.len() == CAPACITY {
                        self.files.pop_front();
                    }
                    self.files.push_back(Opened {
                        source: resource.source.clone(),
                        stamp,
                        file,
                    });
                }
            }
            result
        }
    }
}


#[cfg(test)]
mod tests;

// Discovery needs complete IFF validation, without unused decoded metadata.
mod object_guids;
pub(crate) use object_guids::matching as matching_objects;
pub(crate) use object_guids::parse as object_guids;
