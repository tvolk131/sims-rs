//! Concrete ownership joining the verified aggregate/cache and component readers.
use super::*;
use crate::avatar_installation::DirectorySearch;
use sims_sim::object_catalog::{
    composite::{self, Composite, Reply},
    directory::Directory,
    iff::Iff,
    resources,
};
use std::collections::BTreeMap;
#[path = "../object_catalog/retained/resources.rs"]
mod retained;
pub use retained::{RetainedComponent, RetainedIdentities, RetainedResources, RetainedStream};

pub const NOOP_CALLBACK: u32 = 0x46fb64;

/// Clean portrait-service boundary: bytes have been detached/written, while
/// native allocation cursors/errors and modified-file state must survive reload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub overlay: Overlay,
    files: BTreeMap<u32, FileCheckpoint>,
    errors: BTreeMap<u32, i32>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileCheckpoint {
    source: Option<String>,
    error: i32,
    map_offset: u32,
    unused: u32,
    write_cursor: u32,
    modified: bool,
}

enum Component<'a> {
    Iff {
        state: Iff,
        stream: MountedStream<'a>,
    },
    Directory(Directory),
}
struct Components<'a> {
    index: IndexView<'a>,
    directories: Arc<Directories>,
    search: DirectorySearch,
    entries: BTreeMap<u32, Component<'a>>,
    next_component: u32,
    handles: BTreeMap<u32, (u32, u32)>,
    reverse_handles: BTreeMap<(u32, u32), u32>,
    next_handle: u32,
    opens: Vec<(Vec<u8>, i32)>,
    overlay: overlay::Shared,
    buffers: BTreeMap<u32, Vec<u8>>,
}
impl<'a> Components<'a> {
    fn data(&self, handle: u32) -> Result<&[u8], String> {
        if let Some(bytes) = self.buffers.get(&handle) {
            return Ok(bytes);
        }
        let &(component, local) = self
            .handles
            .get(&handle)
            .ok_or("resource chunk handle absent")?;
        let Some(Component::Iff { state, .. }) = self.entries.get(&component) else {
            return Err("resource chunk component absent".into());
        };
        state
            .payload(local)
            .ok_or_else(|| "resource chunk payload absent".into())
    }
    fn component(&mut self, id: u32) -> Result<&mut Component<'a>, String> {
        self.entries
            .get_mut(&id)
            .ok_or_else(|| "object resource component absent".into())
    }
    fn promote(&mut self, component: u32, local: u32) -> Result<u32, String> {
        if local == 0 {
            return Ok(0);
        }
        if let Some(&handle) = self.reverse_handles.get(&(component, local)) {
            return Ok(handle);
        }
        let handle = self.next_handle;
        self.next_handle = handle
            .checked_add(1)
            .ok_or("object resource handle space exhausted")?;
        self.handles.insert(handle, (component, local));
        self.reverse_handles.insert((component, local), handle);
        Ok(handle)
    }
    fn local(&self, component: u32, handle: u32) -> u32 {
        if handle == 0 {
            return 0;
        }
        // A foreign/unknown nonzero handle must not become zero: native
        // metadata lookup can match an unloaded record whose handle is zero.
        self.handles
            .get(&handle)
            .filter(|(c, _)| *c == component)
            .map_or(u32::MAX, |(_, h)| *h)
    }
    fn forget(&mut self, component: u32) {
        self.handles.retain(|_, (c, _)| *c != component);
        self.reverse_handles.retain(|(c, _), _| *c != component);
    }
    fn callback(callback: u32) -> Result<bool, String> {
        match callback {
            0 => Ok(false),
            NOOP_CALLBACK => Ok(true),
            _ => Err("unsupported object resource transformation callback".into()),
        }
    }
}
impl composite::Host for Components<'_> {
    fn construct(&mut self, kind: u8) -> Result<u32, String> {
        let component = match kind {
            2 => Component::Iff {
                state: Iff::default(),
                stream: MountedStream::with_overlay(
                    self.index.clone(),
                    self.directories.clone(),
                    self.overlay.clone(),
                ),
            },
            3 => Component::Directory(Directory::default()),
            _ => return Err("legacy IFFResFile component is not implemented".into()),
        };
        let id = self.next_component;
        self.next_component = id
            .checked_add(1)
            .ok_or("object component identity exhausted")?;
        self.entries.insert(id, component);
        Ok(id)
    }
    fn open(&mut self, id: u32, path: &[u8], writable: u32) -> Result<i32, String> {
        let component = self
            .entries
            .get_mut(&id)
            .ok_or("object resource component absent")?;
        let code = match component {
            Component::Iff { state, stream } => state.open(path, writable, stream)?,
            Component::Directory(state) => state.open(path, writable, &mut self.search)?,
        };
        self.opens.push((path.to_vec(), code));
        Ok(code)
    }
    fn valid(&mut self, id: u32) -> Result<bool, String> {
        match self
            .entries
            .get(&id)
            .ok_or("object resource component absent")?
        {
            Component::Iff { state, .. } => Ok(state.valid),
            Component::Directory(state) => state.valid(&mut self.search),
        }
    }
    fn writable(&mut self, id: u32) -> Result<bool, String> {
        match self.component(id)? {
            Component::Iff { stream, .. } => Ok(stream.writable()),
            Component::Directory(_) => Ok(false),
        }
    }
    fn flush(&mut self, id: u32) -> Result<i32, String> {
        match self.component(id)? {
            Component::Iff { state, stream } => {
                state.flush(stream)?;
                Ok(state.error)
            }
            Component::Directory(_) => Err("DIR flush is not implemented".into()),
        }
    }
    fn write(&mut self, id: u32, handle: u32) -> Result<i32, String> {
        let local = self.local(id, handle);
        match self.component(id)? {
            Component::Iff { state, stream } => {
                state.write_chunk(local, stream)?;
                Ok(state.error)
            }
            Component::Directory(_) => Err("DIR write is not implemented".into()),
        }
    }
    fn delete(&mut self, id: u32, handle: u32) -> Result<i32, String> {
        let local = self.local(id, handle);
        match self.component(id)? {
            Component::Iff { state, stream } => {
                state.delete(local, stream)?;
                Ok(state.error)
            }
            Component::Directory(_) => Err("DIR deletion is not implemented".into()),
        }
    }
    fn add(&mut self, id: u32, chunk: composite::Chunk<'_>) -> Result<i32, String> {
        let bytes = if chunk.handle == 0 {
            None
        } else {
            Some(self.data(chunk.handle)?.to_vec())
        };
        let current = self
            .handles
            .get(&chunk.handle)
            .filter(|(c, _)| *c == id)
            .map(|(_, l)| *l);
        let (local, error) = match self.component(id)? {
            Component::Iff { state, stream } => {
                let payload = bytes
                    .map(|bytes| match current {
                        Some(handle) => {
                            Ok(sims_sim::object_catalog::iff::Payload { handle, bytes })
                        }
                        None => state.allocate_payload(bytes),
                    })
                    .transpose()?;
                let local = payload.as_ref().map_or(0, |p| p.handle);
                state.add(
                    sims_sim::object_catalog::iff::Add {
                        payload,
                        tag: chunk.tag,
                        id: chunk.id,
                        variant: chunk.variant,
                        name: chunk.name,
                        converted: chunk.converted,
                    },
                    stream,
                )?;
                (local, state.error)
            }
            Component::Directory(_) => return Err("DIR addition is not implemented".into()),
        };
        if error == 0 && chunk.handle != 0 {
            self.buffers.remove(&chunk.handle);
            self.handles.insert(chunk.handle, (id, local));
            self.reverse_handles.insert((id, local), chunk.handle);
        }
        Ok(error)
    }
    fn filename(&mut self, id: u32, output: &mut Vec<u8>) -> Result<(), String> {
        match self.component(id)? {
            Component::Iff { stream, .. } => {
                if let Some(path) = stream.filename() {
                    *output = path.to_vec();
                }
            }
            Component::Directory(state) => *output = state.filename()?.to_vec(),
        }
        Ok(())
    }
    fn count(&mut self, id: u32, tag: u32) -> Result<Reply, String> {
        match self.component(id)? {
            Component::Iff { state, .. } => Ok(Reply {
                value: state.count_raw(tag),
                error: state.error,
            }),
            Component::Directory(_) => {
                Err("present DIR resource enumeration is not implemented".into())
            }
        }
    }
    fn by_id(
        &mut self,
        component: u32,
        tag: u32,
        id: u32,
        _options: u32,
        callback: u32,
    ) -> Result<Reply, String> {
        let callback = Self::callback(callback)?;
        let (handle, error) = match self.component(component)? {
            Component::Iff { state, stream } => {
                (state.by_id(tag, id, callback, stream)?, state.error)
            }
            Component::Directory(_) => {
                return Err("present DIR resource lookup is not implemented".into());
            }
        };
        Ok(Reply {
            value: self.promote(component, handle)?,
            error,
        })
    }
    fn ordinal(
        &mut self,
        component: u32,
        tag: u32,
        ordinal: u32,
        callback: u32,
    ) -> Result<Reply, String> {
        let callback = Self::callback(callback)?;
        let (handle, error) = match self.component(component)? {
            Component::Iff { state, stream } => {
                (state.ordinal(tag, ordinal, callback, stream)?, state.error)
            }
            Component::Directory(_) => {
                return Err("present DIR resource enumeration is not implemented".into());
            }
        };
        Ok(Reply {
            value: self.promote(component, handle)?,
            error,
        })
    }
    fn name(&mut self, id: u32, handle: u32, output: &mut Vec<u8>) -> Result<i32, String> {
        let local = self.local(id, handle);
        match self.component(id)? {
            Component::Iff { state, .. } => {
                state.name(local, output);
                Ok(state.error)
            }
            Component::Directory(_) => {
                Err("present DIR resource metadata is not implemented".into())
            }
        }
    }
    fn id(&mut self, id: u32, handle: u32, output: &mut u32) -> Result<i32, String> {
        let local = self.local(id, handle);
        match self.component(id)? {
            Component::Iff { state, .. } => {
                state.id(local, output);
                Ok(state.error)
            }
            Component::Directory(_) => {
                Err("present DIR resource metadata is not implemented".into())
            }
        }
    }
    fn close(&mut self, id: u32) -> Result<i32, String> {
        let code = match self.component(id)? {
            Component::Iff { state, stream } => state.close(stream)?,
            Component::Directory(state) => state.close(),
        };
        self.forget(id);
        Ok(code)
    }
    fn release_chunk(&mut self, id: u32, handle: u32) -> Result<i32, String> {
        let local = self.local(id, handle);
        let (error, buffer) = match self.component(id)? {
            Component::Iff { state, stream } => {
                let payload = state.detach_payload(local, stream)?;
                (state.error, payload.map(|p| p.bytes))
            }
            Component::Directory(_) => {
                return Err("present DIR chunk release is not implemented".into());
            }
        };
        if error == 0 && handle != 0 {
            self.handles.remove(&handle);
            self.reverse_handles.remove(&(id, local));
            if let Some(buffer) = buffer {
                self.buffers.insert(handle, buffer);
            }
        }
        Ok(error)
    }
    fn destroy(&mut self, id: u32) -> Result<(), String> {
        // Destruction must also close a stream retained by a failed IFF open.
        self.close(id)?;
        self.entries
            .remove(&id)
            .ok_or("destroyed object component absent")?;
        Ok(())
    }
}

struct Owner<'a> {
    aggregates: BTreeMap<u32, Composite>,
    next: u32,
    components: Components<'a>,
}
impl resources::Host for Owner<'_> {
    type Resource = u32;
    fn construct(&mut self) -> Result<u32, String> {
        let id = self.next;
        self.next = id
            .checked_add(1)
            .ok_or("aggregate resource identity exhausted")?;
        self.aggregates.insert(id, Composite::default());
        Ok(id)
    }
    fn open(&mut self, id: u32, path: &[u8], mode: u8) -> Result<(), String> {
        // 465949's two catalog modes: no create/retry flags; bit 4 disables
        // writable opening. A native open failure is checked via valid later.
        if !matches!(mode, 0 | 4) {
            return Err("unsupported catalog resource open mode".into());
        }
        self.aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?
            .open(path, u32::from(mode & 4 == 0), &mut self.components)?;
        Ok(())
    }
    fn valid(&mut self, id: u32) -> Result<bool, String> {
        self.aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?
            .valid(&mut self.components)
    }
    fn filename(&mut self, id: u32) -> Result<Vec<u8>, String> {
        let mut output = vec![];
        self.aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?
            .filename(&mut output, &mut self.components)?;
        Ok(output)
    }
    fn close(&mut self, id: u32) -> Result<(), String> {
        self.aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?
            .close(&mut self.components)?;
        Ok(())
    }
    fn destroy(&mut self, id: u32) -> Result<(), String> {
        self.aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?
            .destroy(&mut self.components)?;
        self.aggregates
            .remove(&id)
            .ok_or("destroyed aggregate resource absent")?;
        Ok(())
    }
}

pub struct Resources<'a> {
    pub cache: resources::Cache<u32>,
    owner: Owner<'a>,
    fault: Option<String>,
    // Rust capability identity only: borrowed services must not silently attach
    // to an independently reconstructed equal-looking resource owner.
    retained_identity: Arc<()>,
}
impl<'a> Resources<'a> {
    pub fn new(index: &'a Index, directories: Directories, search: DirectorySearch) -> Self {
        Self::build(
            IndexView::Borrowed(index),
            directories,
            search,
            Overlay::default(),
        )
    }
    pub fn with_overlay(
        index: &'a Index,
        directories: Directories,
        search: DirectorySearch,
        overlay: Overlay,
    ) -> Result<Self, String> {
        overlay.validate(index)?;
        Ok(Self::build(
            IndexView::Borrowed(index),
            directories,
            search,
            overlay,
        ))
    }
    /// Retain the immutable installation index without a self-referential client.
    pub fn owned(
        index: Arc<Index>,
        directories: Directories,
        search: DirectorySearch,
        overlay: Overlay,
    ) -> Result<Resources<'static>, String> {
        overlay.validate(&index)?;
        Ok(Resources::build(
            IndexView::Shared(index),
            directories,
            search,
            overlay,
        ))
    }
    fn build(
        index: IndexView<'a>,
        directories: Directories,
        search: DirectorySearch,
        overlay: Overlay,
    ) -> Self {
        Self {
            cache: resources::Cache::default(),
            fault: None,
            retained_identity: Arc::new(()),
            owner: Owner {
                aggregates: BTreeMap::new(),
                next: 1,
                components: Components {
                    index,
                    directories: Arc::new(directories),
                    search,
                    entries: BTreeMap::new(),
                    next_component: 1,
                    handles: BTreeMap::new(),
                    reverse_handles: BTreeMap::new(),
                    next_handle: 1,
                    opens: vec![],
                    overlay: Arc::new(std::sync::Mutex::new(overlay)),
                    buffers: BTreeMap::new(),
                },
            },
        }
    }
    fn ready(&self) -> Result<(), String> {
        self.fault.as_ref().map_or(Ok(()), |e| {
            Err(format!(
                "object resource host stopped at unresolved boundary: {e}"
            ))
        })
    }
    fn completed<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        if let Err(e) = &result {
            self.fault = Some(e.clone());
        }
        result
    }
    pub fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String> {
        self.ready()?;
        let result = self.cache.acquire(path, people, &mut self.owner);
        self.completed(result)
    }
    pub fn release(&mut self, id: u32) -> Result<(), String> {
        self.ready()?;
        let result = self.cache.release(id, &mut self.owner);
        self.completed(result)
    }
    pub fn valid(&mut self, id: u32) -> Result<bool, String> {
        use resources::Host;
        self.ready()?;
        let result = self.owner.valid(id);
        self.completed(result)
    }
    pub fn writable(&mut self, id: u32) -> Result<bool, String> {
        self.with(id, |r, c| r.writable(c))
    }
    /// Owned copied resources, suitable for serializing alongside a session.
    /// Resident chunk edits are not persisted until a native flush is serviced.
    pub fn overlay(&self) -> Result<Overlay, String> {
        Ok(self
            .owner
            .components
            .overlay
            .lock()
            .map_err(|_| "resource overlay lock poisoned")?
            .clone())
    }
    pub fn checkpoint(&self) -> Result<Checkpoint, String> {
        self.ready()?;
        if !self.owner.components.buffers.is_empty() {
            return Err("resource checkpoint has an unattached buffer".into());
        }
        let overlay = self.overlay()?;
        let mut files = BTreeMap::new();
        for (&id, component) in &self.owner.components.entries {
            let Component::Iff { state, stream } = component else {
                continue;
            };
            if state.records().any(|r| r.offset == u32::MAX) {
                return Err("resource checkpoint has an unwritten chunk".into());
            }
            let source = stream
                .file
                .source
                .map(|i| stream.file.index.resources[i].locator.clone());
            if stream.writable()
                && let Some(source) = &source
            {
                if stream.file.bytes.as_deref() != overlay.read(source) {
                    return Err("resource checkpoint has unflushed bytes".into());
                }
            }
            files.insert(
                id,
                FileCheckpoint {
                    source,
                    error: state.error,
                    map_offset: state.map_offset,
                    unused: state.unused,
                    write_cursor: state.write_cursor,
                    modified: state.modified,
                },
            );
        }
        Ok(Checkpoint {
            overlay,
            files,
            errors: self
                .owner
                .aggregates
                .iter()
                .map(|(&id, c)| (id, c.error))
                .collect(),
        })
    }
    /// Rebind only after rebuilding the same catalog from checkpoint.overlay.
    /// Chunk handles do not escape portrait service calls; definition/resource
    /// identities and the file layout are checked before committing metadata.
    pub fn restore_checkpoint(&mut self, saved: &Checkpoint) -> Result<(), String> {
        let current = self.checkpoint()?;
        if current.overlay != saved.overlay
            || current.files.len() != saved.files.len()
            || !current.errors.keys().eq(saved.errors.keys())
            || current.files.iter().any(|(id, file)| {
                saved
                    .files
                    .get(id)
                    .is_none_or(|s| s.source != file.source || s.map_offset != file.map_offset)
            })
        {
            return Err("resource checkpoint catalog/file identity changed".into());
        }
        for (id, saved) in &saved.files {
            let Component::Iff { state, .. } = self.owner.components.entries.get_mut(id).unwrap()
            else {
                unreachable!()
            };
            state.error = saved.error;
            state.unused = saved.unused;
            state.write_cursor = saved.write_cursor;
            state.modified = saved.modified;
        }
        for (id, error) in &saved.errors {
            self.owner.aggregates.get_mut(id).unwrap().error = *error;
        }
        Ok(())
    }
    pub fn filename(&mut self, id: u32) -> Result<Vec<u8>, String> {
        use resources::Host;
        self.ready()?;
        let result = self.owner.filename(id);
        self.completed(result)
    }
    fn with<T>(
        &mut self,
        id: u32,
        f: impl FnOnce(&mut Composite, &mut Components<'a>) -> Result<T, String>,
    ) -> Result<T, String> {
        self.ready()?;
        let resource = self
            .owner
            .aggregates
            .get_mut(&id)
            .ok_or("aggregate resource absent")?;
        let result = f(resource, &mut self.owner.components);
        self.completed(result)
    }
    pub fn count(&mut self, id: u32, tag: u32) -> Result<u32, String> {
        self.with(id, |r, c| r.count(tag, c).map(u32::from))
    }
    pub fn by_id(&mut self, id: u32, tag: u32, chunk: u32, callback: u32) -> Result<u32, String> {
        self.with(id, |r, c| r.by_id(tag, chunk, 0, callback, c))
    }
    pub fn ordinal(
        &mut self,
        id: u32,
        tag: u32,
        ordinal: u32,
        callback: u32,
    ) -> Result<u32, String> {
        self.with(id, |r, c| r.ordinal(tag, ordinal, callback, c))
    }
    pub fn name(&mut self, id: u32, handle: u32) -> Result<Vec<u8>, String> {
        self.with(id, |r, c| {
            let mut out = vec![];
            r.name(handle, &mut out, c)?;
            Ok(out)
        })
    }
    pub fn id(&mut self, id: u32, handle: u32) -> Result<u32, String> {
        self.with(id, |r, c| {
            let mut out = 0;
            r.id(handle, &mut out, c)?;
            Ok(out)
        })
    }
    pub fn error(&self, id: u32) -> Result<i32, String> {
        Ok(self
            .owner
            .aggregates
            .get(&id)
            .ok_or("aggregate resource absent")?
            .error)
    }
    pub fn release_chunk(&mut self, id: u32, handle: u32) -> Result<(), String> {
        self.with(id, |r, c| r.release_chunk(handle, c))?;
        if self.error(id)? == 0 {
            self.owner.components.buffers.remove(&handle);
        }
        Ok(())
    }
    pub fn allocate(&mut self, bytes: Vec<u8>) -> Result<u32, String> {
        self.ready()?;
        if bytes.len() as u64 > crate::MAX_RESOURCE {
            return Err("resource buffer exceeds 128 MiB".into());
        }
        let c = &mut self.owner.components;
        let handle = c.next_handle;
        c.next_handle = handle
            .checked_add(1)
            .ok_or("object resource handle space exhausted")?;
        c.buffers.insert(handle, bytes);
        Ok(handle)
    }
    pub fn add(&mut self, id: u32, chunk: composite::Chunk<'_>) -> Result<(), String> {
        self.with(id, |r, c| r.add(chunk, c))
    }
    pub fn delete(&mut self, id: u32, handle: u32) -> Result<(), String> {
        self.with(id, |r, c| r.delete(handle, c))
    }
    pub fn flush(&mut self, id: u32) -> Result<(), String> {
        self.with(id, |r, c| r.flush(c))
    }
    pub fn data(&self, handle: u32) -> Result<&[u8], String> {
        self.ready()?;
        self.owner.components.data(handle)
    }
    /// Exact mounted source of a loaded chunk. Aggregate filenames omit the
    /// extension and cannot distinguish a winning overlay from another IFF.
    pub fn chunk_source(&self, handle: u32) -> Result<&str, String> {
        self.data(handle)?;
        let &(component, _) = self
            .owner
            .components
            .handles
            .get(&handle)
            .ok_or("resource buffer is not attached to a chunk")?;
        let Some(Component::Iff { stream, .. }) = self.owner.components.entries.get(&component)
        else {
            return Err("resource chunk component absent".into());
        };
        let source = stream.source().ok_or("resource chunk source absent")?;
        self.owner
            .components
            .index
            .resources
            .get(source)
            .map(|r| r.locator.as_str())
            .ok_or_else(|| "resource chunk source not indexed".into())
    }
    pub fn data_mut(&mut self, handle: u32) -> Result<&mut Vec<u8>, String> {
        self.ready()?;
        let &(component, local) = self
            .owner
            .components
            .handles
            .get(&handle)
            .ok_or("resource chunk handle absent")?;
        let Some(Component::Iff { state, .. }) = self.owner.components.entries.get_mut(&component)
        else {
            return Err("resource chunk component absent".into());
        };
        state
            .payload_mut(local)
            .ok_or_else(|| "resource chunk payload absent".into())
    }
    pub fn component_opens(&self) -> &[(Vec<u8>, i32)] {
        &self.owner.components.opens
    }
    pub fn live_counts(&self) -> (usize, usize, usize) {
        (
            self.owner.aggregates.len(),
            self.owner.components.entries.len(),
            self.owner.components.handles.len(),
        )
    }
}
