//! Exact mounted readonly snapshots. Capabilities retain the bytes actually
//! read; this is a Rust snapshot policy, not full Windows sharing semantics.
use crate::{Index, avatar_catalog::Directories, resource_scan::Scan};
use sha2::{Digest, Sha256};
use sims_sim::saved_matrix::Stop;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    ScenarioProbe,
    ResourceReadonly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub resource: usize,
    pub locator: String,
    pub source: PathBuf,
    pub offset: u64,
    pub declared_bytes: u64,
    pub archived: bool,
    pub bytes_sha256: String,
}

pub(super) struct BoundFile {
    pub role: FileRole,
    pub identity: SourceIdentity,
    pub bytes: Arc<[u8]>,
    pub position: usize,
}

pub struct FileView<'a> {
    pub role: FileRole,
    pub identity: &'a SourceIdentity,
    pub bytes: &'a [u8],
    pub position: usize,
}

pub struct MountedFiles<'a> {
    index: &'a Index,
    directories: &'a Directories,
    scan: Scan,
}

impl<'a> MountedFiles<'a> {
    /// The caller supplies its completed mount view. This does not reconstruct
    /// provider precedence or certify original initialization history.
    pub fn new(index: &'a Index, directories: &'a Directories) -> Self {
        Self {
            index,
            directories,
            scan: Scan::default(),
        }
    }

    pub(super) fn resolve(
        &mut self,
        path: &[u8],
        role: FileRole,
    ) -> Result<Option<BoundFile>, Stop> {
        if path.is_empty() || path.len() > 259 || !path.is_ascii() || path.contains(&0) {
            return Err(Stop::Unsupported(
                "mounted raw file path is outside the bounded counted ASCII profile".into(),
            ));
        }
        let split = path
            .iter()
            .rposition(|byte| matches!(byte, b'/' | b'\\'))
            .map_or(0, |at| at + 1);
        if split == path.len() {
            return Err(Stop::Unsupported("mounted raw file has no filename".into()));
        }
        let directory = &path[..split];
        if !self
            .directories
            .contains_directory(directory)
            .map_err(|e| Stop::Unavailable(e.to_string()))?
        {
            return Err(Stop::Unavailable(
                "raw file directory has no supplied mounted provider map".into(),
            ));
        }
        let source = match self
            .directories
            .source_entry(directory, &path[split..])
            .map_err(|e| Stop::Unavailable(e.to_string()))?
        {
            None => return Ok(None),
            Some(None) => {
                return Err(Stop::Unavailable(
                    "mounted raw file node has no supplied readable resource binding".into(),
                ));
            }
            Some(Some(source)) => source,
        };
        let resource =
            self.index.resources.get(source).ok_or_else(|| {
                Stop::Unsupported("mounted raw file source is not indexed".into())
            })?;
        let bytes = self.scan.read(resource).map_err(|e| {
            Stop::Unavailable(format!(
                "mounted readonly source did not return bytes: {e:#}"
            ))
        })?;
        let identity = SourceIdentity {
            resource: source,
            locator: resource.locator.clone(),
            source: resource.source.clone(),
            offset: resource.offset,
            declared_bytes: resource.size,
            archived: resource.archived,
            bytes_sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        Ok(Some(BoundFile {
            role,
            identity,
            bytes: bytes.into(),
            position: 0,
        }))
    }
}
