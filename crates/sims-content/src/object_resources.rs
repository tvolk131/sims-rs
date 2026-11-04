//! Mounted file service for the native object-resource reader. Paths resolve
//! through the already mounted directory maps, preserving archive/loose-source
//! identity. Original startup registration remains the caller's responsibility.
use crate::{Index, avatar_catalog::Directories};
use sims_sim::object_catalog::access::{self, Access};
use sims_sim::object_catalog::iff::{Stream, WriteStream};
use std::sync::Arc;
mod overlay;
mod owner;
pub use overlay::Overlay;
pub use owner::{
    Checkpoint, NOOP_CALLBACK, Resources, RetainedComponent, RetainedIdentities,
    RetainedResources, RetainedStream,
};

#[derive(Clone)]
enum IndexView<'a> {
    Borrowed(&'a Index),
    Shared(Arc<Index>),
}
impl std::ops::Deref for IndexView<'_> {
    type Target = Index;
    fn deref(&self) -> &Index {
        match self {
            Self::Borrowed(i) => i,
            Self::Shared(i) => i,
        }
    }
}

enum DirectoryView<'a> {
    Borrowed(&'a Directories),
    Shared(Arc<Directories>),
}
impl DirectoryView<'_> {
    fn get(&self) -> &Directories {
        match self {
            Self::Borrowed(d) => d,
            Self::Shared(d) => d,
        }
    }
}

pub struct MountedStream<'a> {
    access: Access,
    file: MountedFile<'a>,
}
struct MountedFile<'a> {
    index: IndexView<'a>,
    directories: DirectoryView<'a>,
    bytes: Option<Vec<u8>>,
    position: usize,
    source: Option<usize>,
    filename: Vec<u8>,
    overlay: overlay::Shared,
}
impl<'a> MountedStream<'a> {
    pub fn new(index: &'a Index, directories: &'a Directories) -> Self {
        Self {
            access: Access::default(),
            file: MountedFile {
                index: IndexView::Borrowed(index),
                directories: DirectoryView::Borrowed(directories),
                bytes: None,
                position: 0,
                source: None,
                filename: vec![],
                overlay: Arc::new(std::sync::Mutex::new(Overlay::default())),
            },
        }
    }
    pub fn shared(index: &'a Index, directories: Arc<Directories>) -> Self {
        Self::with_overlay(
            IndexView::Borrowed(index),
            directories,
            Arc::new(std::sync::Mutex::new(Overlay::default())),
        )
    }
    fn with_overlay(
        index: IndexView<'a>,
        directories: Arc<Directories>,
        overlay: overlay::Shared,
    ) -> Self {
        Self {
            access: Access::default(),
            file: MountedFile {
                index,
                directories: DirectoryView::Shared(directories),
                bytes: None,
                position: 0,
                source: None,
                filename: vec![],
                overlay,
            },
        }
    }
    pub fn source(&self) -> Option<usize> {
        self.file.source
    }
    pub fn filename(&self) -> Option<&[u8]> {
        self.file
            .bytes
            .as_ref()
            .map(|_| self.file.filename.as_slice())
    }
    /// 4174dc returns the stream flag, even after close; it is not validity.
    pub fn writable(&self) -> bool {
        self.access.writable != 0
    }
}
impl WriteStream for MountedStream<'_> {
    fn writable(&self) -> bool {
        self.writable()
    }
    fn write(&mut self, bytes: &[u8]) -> Result<i32, String> {
        if !self.access.opened {
            return Ok(-49);
        }
        if !self.writable() {
            return Ok(-46);
        }
        let end = self
            .file
            .position
            .checked_add(bytes.len())
            .filter(|&n| n as u64 <= crate::MAX_RESOURCE)
            .ok_or("IFF write exceeds 128 MiB resource limit")?;
        let buffer = self.file.bytes.as_mut().ok_or("opened IFF buffer absent")?;
        if end > buffer.len() {
            buffer
                .try_reserve(end - buffer.len())
                .map_err(|_| "IFF write allocation failed")?;
            buffer.resize(end, 0);
        }
        buffer[self.file.position..end].copy_from_slice(bytes);
        self.file.position = end;
        Ok(0)
    }
    fn position(&mut self) -> Result<Result<u32, i32>, String> {
        Ok(if self.access.opened {
            Ok(self.file.position as u32)
        } else {
            Err(-49)
        })
    }
    fn flush(&mut self) -> Result<i32, String> {
        if !self.access.opened {
            return Ok(-49);
        }
        if self.writable() {
            let source = self.file.source.ok_or("opened IFF source absent")?;
            let locator = &self.file.index.resources[source].locator;
            self.file
                .overlay
                .lock()
                .map_err(|_| "resource overlay lock poisoned")?
                .commit(
                    locator,
                    self.file.bytes.as_ref().ok_or("opened IFF buffer absent")?,
                )?;
        }
        Ok(0)
    }
}
impl Stream for MountedStream<'_> {
    fn open(&mut self, path: &[u8], writable: u32) -> Result<i32, String> {
        self.access.open(
            writable,
            &mut FileAccess {
                file: &mut self.file,
                path,
            },
        )
    }
    fn seek(&mut self, offset: u32) -> Result<i32, String> {
        if !self.access.opened {
            return Ok(-49);
        }
        self.file.position = offset as usize;
        Ok(0)
    }
    fn read(&mut self, output: &mut [u8]) -> Result<i32, String> {
        let Some(bytes) = self.file.bytes.as_ref() else {
            return Ok(-49);
        };
        let available = bytes.get(self.file.position..).unwrap_or_default();
        let n = output.len().min(available.len());
        output[..n].copy_from_slice(&available[..n]);
        self.file.position += n;
        Ok(if n == output.len() { 0 } else { -48 })
    }
    fn close(&mut self) -> Result<i32, String> {
        self.access.close(&mut FileAccess {
            file: &mut self.file,
            path: &[],
        })
    }
}
struct FileAccess<'a, 'b> {
    file: &'a mut MountedFile<'b>,
    path: &'a [u8],
}
impl access::Host for FileAccess<'_, '_> {
    fn open(&mut self, writable: bool) -> Result<bool, String> {
        let file = &mut self.file;
        let path = self.path.split(|&b| b == 0).next().unwrap_or_default();
        let split = path
            .iter()
            .rposition(|b| matches!(b, b'/' | b'\\'))
            .map_or(0, |i| i + 1);
        let Some(source) = file
            .directories
            .get()
            .source(&path[..split], &path[split..])
            .map_err(|e| e.to_string())?
        else {
            return Ok(false);
        };
        let resource = file
            .index
            .resources
            .get(source)
            .ok_or("mounted IFF source is not indexed")?;
        let copy = file
            .overlay
            .lock()
            .map_err(|_| "resource overlay lock poisoned")?
            .read(&resource.locator)
            .map(<[u8]>::to_vec);
        if writable && copy.is_none() {
            return Ok(false);
        };
        let bytes = match copy {
            Some(bytes) => bytes,
            None => resource.read().map_err(|e| e.to_string())?,
        };
        file.bytes = Some(bytes);
        file.position = 0;
        file.source = Some(source);
        file.filename = path.to_vec();
        Ok(true)
    }
    fn buffer(&mut self, _read: u32, _write: u32) -> Result<(), String> {
        // The host owns a complete memory buffer; no filesystem buffering.
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        self.file.bytes = None;
        self.file.source = None;
        self.file.filename.clear();
        Ok(())
    }
}
