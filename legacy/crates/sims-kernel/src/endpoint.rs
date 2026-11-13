//! Physical endpoint ancestry under the port's POSIX ownership/mode policy.
//! Broker UID and the system administrator are trusted. An unrelated UID must
//! not control a directory entry above the endpoint. Trusted sticky directories
//! (for example the system temp directory) permit traversal to a protected child.
//! Same-UID filesystem adversaries remain outside this deployment profile.
use std::os::unix::fs::MetadataExt;
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

pub(crate) fn trusted_directory(meta: &fs::Metadata, expected_uid: u32) -> bool {
    meta.is_dir()
        && (meta.uid() == 0 || meta.uid() == expected_uid)
        && (meta.mode() & 0o022 == 0 || meta.mode() & 0o1000 != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}
impl Identity {
    fn read(meta: &fs::Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
            mode: meta.mode(),
            uid: meta.uid(),
            gid: meta.gid(),
        }
    }
}

pub(crate) struct Ancestors {
    expected_uid: u32,
    path: PathBuf,
    chain: Vec<(PathBuf, Identity)>,
}
impl Ancestors {
    pub fn capture(path: &Path, expected_uid: u32) -> io::Result<Self> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        if absolute.as_os_str().as_encoded_bytes().len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "endpoint ancestry exceeds path bound",
            ));
        }
        let mut current = PathBuf::new();
        let mut chain = Vec::new();
        for component in absolute.components() {
            match component {
                Component::RootDir | Component::Normal(_) => current.push(component.as_os_str()),
                Component::CurDir => continue,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "endpoint ancestry contains parent traversal",
                    ));
                }
            }
            if chain.len() == 256 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "endpoint has too many ancestors",
                ));
            }
            let meta = fs::symlink_metadata(&current)?;
            if !trusted_directory(&meta, expected_uid) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "untrusted endpoint ancestor",
                ));
            }
            chain.push((current.clone(), Identity::read(&meta)));
        }
        let result = Self {
            expected_uid,
            path: current,
            chain,
        };
        if !result.matches()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "endpoint ancestry changed during admission",
            ));
        }
        Ok(result)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn matches(&self) -> io::Result<bool> {
        for (path, expected) in &self.chain {
            let meta = match fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            };
            if !trusted_directory(&meta, self.expected_uid) || Identity::read(&meta) != *expected {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
