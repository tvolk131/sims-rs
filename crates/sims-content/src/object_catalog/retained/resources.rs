//! Import already open IFF owners. No native Open, map rebuild, registration,
//! lazy payload load, callback, seek, or retain operation is replayed.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_sim::object_catalog::access::Access;
use std::collections::BTreeSet;

/// These counters are declared by a new adapter domain, not inferred native
/// allocation history. Existing managed-save identities need a separate input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RetainedIdentities {
    NewUnpublished {
        next_aggregate: u32,
        next_component: u32,
        next_handle: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedStream {
    pub locator: String,
    /// Hash of the indexed installation baseline. `bytes` below is the explicit
    /// current logical file, including any already completed writable changes.
    pub source_sha256: [u8; 32],
    pub bytes: Vec<u8>,
    pub filename: Vec<u8>,
    pub access: Access,
    pub position: u32,
    /// This profile cannot translate pending native backend writes. Zero must
    /// be observed; omission must not silently stand for a drained buffer.
    pub pending_backend_write_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedComponent {
    #[serde(deserialize_with = "crate::object_catalog::retained::strict::iff")]
    pub iff: Iff,
    pub stream: RetainedStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedResources {
    pub cache_buckets: Vec<Vec<resources::Entry<u32>>>,
    pub cache_len: usize,
    pub aggregates: BTreeMap<u32, Composite>,
    pub components: BTreeMap<u32, RetainedComponent>,
    pub handles: BTreeMap<u32, (u32, u32)>,
    pub buffers: BTreeMap<u32, Vec<u8>>,
    pub identities: RetainedIdentities,
}

impl<'a> Resources<'a> {
    /// Import a complete supplied resource owner. Provenance/coverage of the
    /// original capture is the caller's responsibility; validation here does
    /// not convert a component fixture into a full original starting state.
    /// Only already open IFF components with no pending backend writes are
    /// supported. DIR/closed components must not be substituted with defaults.
    pub fn from_retained(
        index: &'a Index,
        directories: Directories,
        search: DirectorySearch,
        retained: RetainedResources,
    ) -> Result<Self, String> {
        let RetainedIdentities::NewUnpublished {
            next_aggregate,
            next_component,
            next_handle,
        } = retained.identities;
        let valid_ids =
            |next: u32, ids: Vec<u32>| next != 0 && ids.into_iter().all(|id| id != 0 && id < next);
        if !valid_ids(
            next_aggregate,
            retained.aggregates.keys().copied().collect(),
        ) || !valid_ids(
            next_component,
            retained.components.keys().copied().collect(),
        ) || !valid_ids(
            next_handle,
            retained
                .handles
                .keys()
                .chain(retained.buffers.keys())
                .copied()
                .collect(),
        ) || retained
            .buffers
            .values()
            .any(|b| b.len() as u64 > crate::MAX_RESOURCE)
        {
            return Err("invalid retained resource identity epoch".into());
        }
        let cache = resources::Cache::from_retained(retained.cache_buckets, retained.cache_len)?;
        if cache
            .entries()
            .any(|entry| !retained.aggregates.contains_key(&entry.resource))
        {
            return Err("retained resource cache aggregate absent".into());
        }
        let mut reachable = BTreeSet::new();
        for aggregate in retained.aggregates.values() {
            if aggregate.configurations.len() > 16
                || aggregate.configurations.iter().any(|c| {
                    !(1..=3).contains(&c.kind) || c.extension.len() > 7 || c.extension.contains(&0)
                })
            {
                return Err("unsupported retained aggregate configuration".into());
            }
            for id in aggregate.slots.into_iter().flatten() {
                if !retained.components.contains_key(&id) {
                    return Err("retained aggregate component absent".into());
                }
                reachable.insert(id);
            }
        }
        if reachable.len() != retained.components.len() {
            return Err("retained component has no aggregate owner".into());
        }
        let mut sources = BTreeMap::new();
        let mut writable_sources = BTreeSet::new();
        let mut source_current = BTreeMap::<usize, &Vec<u8>>::new();
        for (&id, component) in &retained.components {
            component.iff.validate_retained()?;
            let stream = &component.stream;
            if !stream.access.opened
                || stream.access.writable > 1
                || stream.pending_backend_write_bytes != 0
                || stream.bytes.len() as u64 > crate::MAX_RESOURCE
                || stream.filename.len() > 32767
                || stream.filename.contains(&0)
            {
                return Err("unsupported retained logical stream owner".into());
            }
            let mut matches = index
                .resources
                .iter()
                .enumerate()
                .filter(|(_, r)| r.locator == stream.locator);
            let (source, resource) = matches.next().ok_or("retained stream source absent")?;
            if matches.next().is_some() {
                return Err("ambiguous retained stream source".into());
            }
            let baseline = resource.read().map_err(|e| e.to_string())?;
            let digest: [u8; 32] = Sha256::digest(&baseline).into();
            if digest != stream.source_sha256 {
                return Err(format!(
                    "retained logical stream baseline changed: {}",
                    stream.locator
                ));
            }
            if stream.access.writable == 0 && baseline != stream.bytes {
                return Err("readonly retained stream differs from bound immutable source".into());
            }
            if let Some(other) = source_current.insert(source, &stream.bytes)
                && other != &stream.bytes
            {
                return Err("aliased retained file owners disagree on logical bytes".into());
            }
            if stream.access.writable != 0 {
                writable_sources.insert(source);
            }
            sources.insert(id, source);
        }
        let mut reverse = BTreeMap::new();
        for (&external, &(component, local)) in &retained.handles {
            if retained.buffers.contains_key(&external)
                || reverse.insert((component, local), external).is_some()
                || !retained
                    .components
                    .get(&component)
                    .is_some_and(|c| c.iff.payload(local).is_some())
            {
                return Err("lost or aliased retained resource handle owner".into());
            }
        }
        for (&id, component) in &retained.components {
            for payload in component.iff.resident_payloads() {
                if !reverse.contains_key(&(id, payload.handle)) {
                    return Err("retained resident payload lacks external identity".into());
                }
            }
        }
        let mut overlay = Overlay::copy(index, writable_sources.iter().copied())?;
        for source in writable_sources {
            overlay.commit(&index.resources[source].locator, source_current[&source])?;
        }
        let overlay = Arc::new(std::sync::Mutex::new(overlay));
        let scan: SharedScan = Arc::new(Default::default());
        let directories = Arc::new(directories);
        let mut entries = BTreeMap::new();
        for (id, component) in retained.components {
            let state = component.iff;
            let stream = component.stream;
            entries.insert(
                id,
                Component::Iff {
                    state,
                    stream: MountedStream {
                        access: stream.access,
                        file: MountedFile {
                            index: IndexView::Borrowed(index),
                            directories: DirectoryView::Shared(directories.clone()),
                            bytes: Some(stream.bytes),
                            position: stream.position as usize,
                            source: Some(sources[&id]),
                            filename: stream.filename,
                            overlay: overlay.clone(),
                            scan: scan.clone(),
                        },
                    },
                },
            );
        }
        Ok(Self {
            cache,
            retained_identity: Arc::new(()),
            owner: Owner {
                aggregates: retained.aggregates,
                next: next_aggregate,
                components: Components {
                    index: IndexView::Borrowed(index),
                    directories,
                    search,
                    entries,
                    next_component,
                    handles: retained.handles,
                    reverse_handles: reverse,
                    next_handle,
                    opens: vec![],
                    overlay,
                    scan,
                    buffers: retained.buffers,
                },
            },
            fault: None,
        })
    }

    pub(crate) fn retained_has_aggregate(&self, id: u32) -> bool {
        self.owner.aggregates.contains_key(&id)
    }

    pub(crate) fn retained_identity(&self) -> std::sync::Weak<()> {
        Arc::downgrade(&self.retained_identity)
    }

    pub(crate) fn retained_global_receiver(&self, id: u32) -> Result<GlobalReceiver<'_>, String> {
        self.ready()?;
        let aggregate = self
            .owner
            .aggregates
            .get(&id)
            .ok_or("Global aggregate absent")?;
        let component = aggregate.slots[0].ok_or("Global IFF component absent")?;
        if aggregate.slots[1..].iter().any(Option::is_some) {
            return Err("borrowed Global requires the observed single component".into());
        }
        let Some(Component::Iff { state, stream }) = self.owner.components.entries.get(&component)
        else {
            return Err("borrowed Global component class unsupported".into());
        };
        let source = stream
            .file
            .source
            .and_then(|s| self.owner.components.index.resources.get(s))
            .ok_or("Global stream source absent")?;
        Ok(GlobalReceiver {
            aggregate,
            iff: state,
            source,
            bytes: stream
                .file
                .bytes
                .as_deref()
                .ok_or("Global stream bytes absent")?,
            access: stream.access,
            position: stream
                .file
                .position
                .try_into()
                .map_err(|_| "Global cursor exceeds native width")?,
            component,
            handles: &self.owner.components.handles,
            next_handle: self.owner.components.next_handle,
        })
    }

    /// 464f49 is supported only for the actual already-converted FCNS2 source.
    /// Do not send it through the generic catalog no-op transformation path.
    pub(crate) fn retained_global_fcns_lookup(&mut self, id: u32) -> Result<u32, String> {
        let receiver = self.retained_global_receiver(id)?;
        if receiver
            .iff
            .records()
            .find(|r| r.tag == u32::from_be_bytes(*b"FCNS") && r.id == 2)
            .is_some_and(|r| r.payload.is_none() && r.disk_flags & 16 == 0)
        {
            return Err(
                "unconverted FCNS callback requires original transformation service".into(),
            );
        }
        self.by_id(id, u32::from_be_bytes(*b"FCNS"), 2, 0)
    }

    pub(crate) fn retained_free_buffer(&mut self, handle: u32) -> Result<(), String> {
        if self.owner.components.handles.contains_key(&handle) {
            return Err("cannot free a still attached Global allocation".into());
        }
        self.owner.components.buffers.remove(&handle);
        Ok(())
    }

    /// Source identity of an existing variant-zero map record, without issuing
    /// the original resource lookup or modifying errors/cursors/residency.
    pub(crate) fn retained_record_source(
        &self,
        id: u32,
        tag: u32,
        chunk: u16,
    ) -> Result<&str, String> {
        self.ready()?;
        let aggregate = self
            .owner
            .aggregates
            .get(&id)
            .ok_or("retained definition aggregate absent")?;
        for component in aggregate.slots {
            let Some(component) = component else { break };
            let Some(Component::Iff { state, stream }) =
                self.owner.components.entries.get(&component)
            else {
                return Err("retained definition source component class unsupported".into());
            };
            if state.valid
                && state
                    .records()
                    .any(|r| r.tag == tag && r.id == chunk && r.flags >> 8 == 0)
            {
                let source = stream
                    .file
                    .source
                    .and_then(|s| self.owner.components.index.resources.get(s))
                    .ok_or("retained definition source identity absent")?;
                return Ok(&source.locator);
            }
        }
        Err("retained definition map record absent".into())
    }
}

pub(crate) struct GlobalReceiver<'a> {
    pub aggregate: &'a Composite,
    pub iff: &'a Iff,
    pub source: &'a crate::Resource,
    pub bytes: &'a [u8],
    pub access: Access,
    pub position: u32,
    pub component: u32,
    pub handles: &'a BTreeMap<u32, (u32, u32)>,
    pub next_handle: u32,
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
