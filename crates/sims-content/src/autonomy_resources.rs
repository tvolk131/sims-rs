//! Session-owned Global FCNS2 services for the explicitly observed CC1.91
//! initial process profile. Installed files stay immutable. This focused owner
//! models the observed readonly ObjResFile/IFFResFile3 chain; other classes,
//! writable providers and unconverted payload callbacks remain explicit errors.
use crate::Index;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_sim::object_catalog::{
    composite::{self, Composite, Reply as FileReply},
    iff::{Iff, Stream, WriteStream},
};
use sims_vm::autonomy_constants::{self as constants, Binding, Owner, Reply, Request, Table};
use std::collections::{BTreeMap, BTreeSet};
mod retained;
pub use retained::Retained;
mod catalog;
pub use catalog::{CatalogObservation, CatalogProvider};
const TAG: u32 = u32::from_be_bytes(*b"FCNS");
const CALLBACK: u32 = 0x464f49;
pub const ARCHIVE: &str = "25fe73945f89f82b1f49d2c2f53eb7c4bc31db1bc1dd575b1778a8cf72aaca59";
pub const MODULE: &str = "bdfd8456abceaa3e44f0a9754815248ea1e80c7b0df4fbaff53843fae3731478";
pub const PAYLOAD: &str = "fd3a44e2068aa43ba01d17d79bafb2cd8882beb44a2f53376d34f6172c637a2e";
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    locator: String,
    archive_sha256: String,
    module_sha256: String,
    offset: u64,
    size: u64,
    chunk_offset: u32,
    disk_flags: u16,
    payload_sha256: String,
}
impl Source {
    fn observed(locator: &str) -> Self {
        Self {
            locator: locator.into(),
            archive_sha256: ARCHIVE.into(),
            module_sha256: MODULE.into(),
            offset: 308710,
            size: 148833,
            chunk_offset: 135839,
            disk_flags: 16,
            payload_sha256: PAYLOAD.into(),
        }
    }
    fn bytes(&self, index: &Index) -> Result<Vec<u8>> {
        ensure!(
            self.archive_sha256 == ARCHIVE
                && self.module_sha256 == MODULE
                && self.payload_sha256 == PAYLOAD
                && self.chunk_offset == 135839
                && self.disk_flags == 16
                && self.offset == 308710
                && self.size == 148833,
            "Global FCNS observed source profile changed"
        );
        let resource = &index.resources[index.find(&self.locator)?];
        ensure!(
            resource.locator == self.locator
                && resource.archived
                && resource.owned.is_none()
                && resource.offset == self.offset
                && resource.size == self.size
                && resource.name == "Global.iff",
            "Global FCNS immutable provider identity changed"
        );
        ensure!(
            hash(&std::fs::read(&resource.source)?) == self.archive_sha256,
            "Global FCNS immutable archive changed"
        );
        let bytes = resource.read()?;
        ensure!(
            hash(&bytes) == self.module_sha256,
            "Global FCNS immutable module changed"
        );
        let at = self.chunk_offset as usize;
        ensure!(
            bytes.get(at..at + 4) == Some(b"FCNS")
                && bytes.get(at + 8..at + 10) == Some(&2u16.to_be_bytes())
                && bytes.get(at + 10..at + 12) == Some(&16u16.to_be_bytes()),
            "Global FCNS chunk header changed"
        );
        let size = u32::from_be_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        ensure!(
            size == 472
                && hash(
                    bytes
                        .get(at + 76..at + size)
                        .context("Global FCNS outside module")?
                ) == self.payload_sha256,
            "Global FCNS payload changed"
        );
        Ok(bytes)
    }
    fn binding(&self) -> Binding {
        Binding {
            module: "Global.iff".into(),
            module_sha256: self.module_sha256.clone(),
            resource_id: 2,
            resource_sha256: self.payload_sha256.clone(),
        }
    }
}

/// Complete modeled stream state, not a path that will reopen a starting save.
/// The capability is independently retained; having owned bytes grants no write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OwnedStream {
    bytes: Vec<u8>,
    position: u32,
    open: bool,
    writable: bool,
}
impl Stream for OwnedStream {
    fn open(&mut self, _path: &[u8], writable: u32) -> Result<i32, String> {
        if writable != 0 {
            return Err("unobserved writable Global provider open".into());
        }
        self.position = 0;
        self.open = true;
        Ok(0)
    }
    fn seek(&mut self, offset: u32) -> Result<i32, String> {
        if !self.open {
            return Ok(-49);
        }
        self.position = offset;
        Ok(0)
    }
    fn read(&mut self, out: &mut [u8]) -> Result<i32, String> {
        if !self.open {
            return Ok(-49);
        }
        let at = self.position as usize;
        let available = self.bytes.len().saturating_sub(at).min(out.len());
        if available != 0 {
            out[..available].copy_from_slice(&self.bytes[at..at + available]);
        }
        self.position = self.position.wrapping_add(available as u32);
        Ok(if available == out.len() { 0 } else { -48 })
    }
    fn close(&mut self) -> Result<i32, String> {
        self.open = false;
        Ok(0)
    }
}
impl WriteStream for OwnedStream {
    fn writable(&self) -> bool {
        self.writable
    }
    fn write(&mut self, _: &[u8]) -> Result<i32, String> {
        Err("unobserved Global child write".into())
    }
    fn position(&mut self) -> Result<Result<u32, i32>, String> {
        Ok(if self.open {
            Ok(self.position)
        } else {
            Err(-49)
        })
    }
    fn flush(&mut self) -> Result<i32, String> {
        Err("unobserved Global child stream flush".into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Component {
    iff: Iff,
    stream: OwnedStream,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Components {
    entries: BTreeMap<u32, Component>,
    /// External identity -> (component, local IFF allocation). Detached buffers
    /// retain both identities until their actual free, preserving alias lookup.
    handles: BTreeMap<u32, (u32, u32)>,
    next_handle: u32,
    buffers: BTreeMap<u32, Vec<u8>>,
}
impl Components {
    fn promote(&mut self, component: u32, local: u32) -> Result<u32, String> {
        if local == 0 {
            return Ok(0);
        }
        if let Some((&h, _)) = self
            .handles
            .iter()
            .find(|(_, pair)| **pair == (component, local))
        {
            return Ok(h);
        }
        let h = self.allocate_id()?;
        self.handles.insert(h, (component, local));
        Ok(h)
    }
    fn allocate_id(&mut self) -> Result<u32, String> {
        let h = self.next_handle;
        self.next_handle = h.checked_add(1).ok_or("Global handle space exhausted")?;
        Ok(h)
    }
    fn allocate(&mut self, bytes: Vec<u8>) -> Result<u32, String> {
        let h = self.allocate_id()?;
        self.buffers.insert(h, bytes);
        Ok(h)
    }
    fn local(&self, c: u32, h: u32) -> u32 {
        self.handles
            .get(&h)
            .filter(|(owner, _)| *owner == c)
            .map_or(u32::MAX, |(_, l)| *l)
    }
    fn data(&self, h: u32) -> Result<&[u8], String> {
        if let Some(b) = self.buffers.get(&h) {
            return Ok(b);
        }
        let (c, l) = self.handles.get(&h).ok_or("Global handle absent")?;
        self.entries
            .get(c)
            .and_then(|c| c.iff.payload(*l))
            .ok_or_else(|| "Global resident allocation absent".into())
    }
    fn free(&mut self, h: u32) {
        self.buffers.remove(&h);
        self.handles.remove(&h);
    }
    fn component(&mut self, c: u32) -> Result<&mut Component, String> {
        self.entries
            .get_mut(&c)
            .ok_or_else(|| "Global component absent".into())
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.next_handle > 0 && self.entries.len() == 1 && self.entries.contains_key(&1),
            "Global provider component topology changed"
        );
        // Handle lookups traverse IFF record indices. Validate those indices
        // before following any caller-supplied retained handle binding.
        for component in self.entries.values() {
            component
                .iff
                .validate_retained()
                .map_err(anyhow::Error::msg)?;
        }
        let mut pairs = BTreeSet::new();
        for (&h, &pair) in &self.handles {
            ensure!(
                h > 0 && h < self.next_handle && pair.1 > 0 && pairs.insert(pair),
                "Global resident handle topology invalid"
            );
            let component = self
                .entries
                .get(&pair.0)
                .context("Global handle component absent")?;
            ensure!(
                component.iff.payload(pair.1).is_some() != self.buffers.contains_key(&h),
                "Global resident/detached handle ownership invalid"
            );
        }
        for (&h, bytes) in &self.buffers {
            ensure!(
                h > 0 && h < self.next_handle && bytes.len() <= 128 * 1024 * 1024,
                "Global buffer owner invalid"
            );
        }
        for (&id, c) in &self.entries {
            for record in c.iff.records() {
                if let Some(p) = &record.payload {
                    ensure!(
                        pairs.contains(&(id, p.handle)),
                        "Global resident allocation lost external owner"
                    );
                }
            }
        }
        Ok(())
    }
}
impl composite::Host for Components {
    fn construct(&mut self, _: u8) -> Result<u32, String> {
        Err("Global component construction is explicit profile input".into())
    }
    fn open(&mut self, _: u32, _: &[u8], _: u32) -> Result<i32, String> {
        Err("Global reopen not supported".into())
    }
    fn valid(&mut self, c: u32) -> Result<bool, String> {
        Ok(self.component(c)?.iff.valid)
    }
    fn writable(&mut self, c: u32) -> Result<bool, String> {
        Ok(self.component(c)?.stream.writable)
    }
    fn filename(&mut self, _: u32, _: &mut Vec<u8>) -> Result<(), String> {
        Err("Global filename query outside observed service".into())
    }
    fn count(&mut self, _: u32, _: u32) -> Result<FileReply, String> {
        Err("Global count query outside observed service".into())
    }
    fn by_id(
        &mut self,
        c: u32,
        tag: u32,
        id: u32,
        _options: u32,
        callback: u32,
    ) -> Result<FileReply, String> {
        if tag != TAG || id != 2 || !matches!(callback, 0 | CALLBACK) {
            return Err("Global lookup outside observed FCNS service".into());
        }
        let child = self.component(c)?;
        // Actual source has conversion bit16. Do not pass the FCNS callback
        // through Iff's historical catalog-noop bool path for unconverted data.
        if callback != 0
            && child
                .iff
                .records()
                .find(|r| r.tag == tag && r.id == id as u16)
                .is_some_and(|r| r.payload.is_none() && r.disk_flags & 16 == 0)
        {
            return Err(
                "unconverted FCNS callback requires original transformation service".into(),
            );
        }
        let local = child.iff.by_id(tag, id, false, &mut child.stream)?;
        let error = child.iff.error;
        let value = self.promote(c, local)?;
        Ok(FileReply { value, error })
    }
    fn ordinal(&mut self, _: u32, _: u32, _: u32, _: u32) -> Result<FileReply, String> {
        Err("Global ordinal query outside observed service".into())
    }
    fn name(&mut self, _: u32, _: u32, _: &mut Vec<u8>) -> Result<i32, String> {
        Err("Global name query outside observed service".into())
    }
    fn id(&mut self, _: u32, _: u32, _: &mut u32) -> Result<i32, String> {
        Err("Global id query outside observed service".into())
    }
    fn release_chunk(&mut self, c: u32, h: u32) -> Result<i32, String> {
        let local = self.local(c, h);
        let child = self.component(c)?;
        let payload = child.iff.detach_payload(local, &mut child.stream)?;
        let error = child.iff.error;
        if let Some(payload) = payload {
            self.buffers.insert(h, payload.bytes);
        }
        Ok(error)
    }
    fn close(&mut self, _: u32) -> Result<i32, String> {
        Err("Global close outside observed service".into())
    }
    fn destroy(&mut self, _: u32) -> Result<(), String> {
        Err("Global destroy outside observed service".into())
    }
    // Native Composite skips these methods on a readonly component. A changed
    // capability must stop explicitly rather than return fabricated success.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub operation: String,
    pub handle: u32,
    pub return_value: i32,
    pub aggregate_error: i32,
    pub child_error: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    source: Source,
    aggregate: Composite,
    components: Components,
    pub events: Vec<Event>,
}
impl Provider {
    /// Construct a newly opened installed-profile provider. This does not
    /// import a hot native provider; use `from_retained` for that boundary.
    pub fn observed_readonly(index: &Index, locator: &str) -> Result<Self> {
        let source = Source::observed(locator);
        let bytes = source.bytes(index)?;
        let mut stream = OwnedStream {
            bytes,
            position: 0,
            open: false,
            writable: false,
        };
        let mut iff = Iff::default();
        ensure!(
            iff.open(b"Global.iff", 0, &mut stream)
                .map_err(anyhow::Error::msg)?
                == 0,
            "Global IFF open failed"
        );
        let mut aggregate = Composite::default();
        aggregate.slots[0] = Some(1);
        let result = Self {
            source,
            aggregate,
            components: Components {
                entries: BTreeMap::from([(1, Component { iff, stream })]),
                handles: BTreeMap::new(),
                next_handle: 1,
                buffers: BTreeMap::new(),
            },
            events: vec![],
        };
        result.validate(index)?;
        Ok(result)
    }
    pub fn binding(&self) -> Binding {
        self.source.binding()
    }
    pub fn validate(&self, index: &Index) -> Result<()> {
        let bytes = self.source.bytes(index)?;
        self.components.validate()?;
        let c = &self.components.entries[&1];
        ensure!(
            c.stream.bytes == bytes && !c.stream.writable && c.stream.open && c.iff.valid,
            "Global provider retained stream/capability changed"
        );
        ensure!(
            self.aggregate.configurations == Composite::default().configurations
                && self.aggregate.slots == [Some(1), None, None, None, None, None, None, None]
                && self.aggregate.filters == Composite::default().filters,
            "Global aggregate profile topology changed"
        );
        ensure!(
            self.events.len() <= 4096,
            "Global resource diagnostic event bound exceeded"
        );
        Ok(())
    }
    fn event(&mut self, op: &str, h: u32, result: i32) {
        self.events.push(Event {
            operation: op.into(),
            handle: h,
            return_value: result,
            aggregate_error: self.aggregate.error,
            child_error: self.components.entries[&1].iff.error,
        });
    }
    fn load(&mut self) -> Result<Table> {
        let h = self
            .aggregate
            .by_id(TAG, 2, 0, CALLBACK, &mut self.components)
            .map_err(anyhow::Error::msg)?;
        self.event(
            "Load.Lookup",
            h,
            if h == 0 { self.aggregate.error } else { 0 },
        );
        if h == 0 {
            return Ok(Table::default());
        }
        let table = decode(self.components.data(h).map_err(anyhow::Error::msg)?)?;
        self.aggregate
            .release_chunk(h, &mut self.components)
            .map_err(anyhow::Error::msg)?;
        self.event("Load.Detach", h, self.aggregate.error);
        if self.aggregate.error == 0 {
            self.components.free(h)
        }
        Ok(table)
    }
    fn flush(&mut self, table: &Table) -> Result<i32> {
        let bytes = encode(table)?;
        let h = self
            .components
            .allocate(bytes)
            .map_err(anyhow::Error::msg)?;
        self.event("Flush.Serialize", h, 0);
        self.aggregate
            .add(
                composite::Chunk {
                    handle: h,
                    tag: TAG,
                    id: 2,
                    name: b"",
                    variant: 0,
                    converted: 1,
                },
                &mut self.components,
            )
            .map_err(anyhow::Error::msg)?;
        let add_error = self.aggregate.error;
        self.event("Flush.Add", h, add_error);
        if add_error != 0 {
            self.components.free(h);
            return Ok(add_error);
        }
        // These paths are preserved in order but can only run once a real
        // writable child is implemented/proved; the present profile is readonly.
        self.aggregate
            .write(h, &mut self.components)
            .map_err(anyhow::Error::msg)?;
        let error = self.aggregate.error;
        self.event("Flush.Write", h, error);
        if error != 0 {
            self.aggregate
                .delete(h, &mut self.components)
                .map_err(anyhow::Error::msg)?;
            self.event("Flush.Delete", h, self.aggregate.error);
            return Ok(error);
        }
        self.aggregate
            .release_chunk(h, &mut self.components)
            .map_err(anyhow::Error::msg)?;
        self.event("Flush.Detach", h, self.aggregate.error);
        self.components.free(h);
        Ok(0)
    }
    pub fn service(&mut self, request: &Request, table: Option<&Table>) -> Result<Reply> {
        let binding = match request {
            Request::Load(b) | Request::Flush(b) => b,
        };
        ensure!(
            *binding == self.binding(),
            "Global FCNS service source differs"
        );
        match request {
            Request::Load(_) => Ok(Reply::Loaded {
                binding: binding.clone(),
                table: self.load()?,
            }),
            Request::Flush(_) => Ok(Reply::Flushed {
                binding: binding.clone(),
                return_value: self.flush(table.context("Global Flush table absent")?)?,
            }),
        }
    }
}

/// Raw codec for the version1 payload observed at464ec1/4131b3. Unsupported
/// headers stop explicitly; no Unicode conversion or float roundtrip occurs.
fn decode(bytes: &[u8]) -> Result<Table> {
    ensure!(
        bytes.len() >= 16
            && bytes[..4] == [0, 0, 0, 0]
            && bytes[4..8] == 1u32.to_le_bytes()
            && &bytes[8..12] == b"SNCF",
        "unsupported FCNS raw header/version"
    );
    let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    ensure!(count <= bytes.len() / 8, "FCNS count exceeds payload");
    let mut at = 16;
    fn string(bytes: &[u8], at: &mut usize, cap: usize) -> Result<Vec<u8>> {
        let tail = bytes.get(*at..).context("FCNS string outside payload")?;
        let n = tail
            .iter()
            .position(|b| *b == 0)
            .context("FCNS string terminator absent")?;
        ensure!(
            n < cap,
            "FCNS fixed string exceeds supported native capacity"
        );
        let result = tail[..n].to_vec();
        *at += n + 1;
        if *at % 2 != 0 {
            ensure!(*at < bytes.len(), "FCNS padding absent");
            *at += 1;
        }
        Ok(result)
    }
    let mut rows = Vec::new();
    for _ in 0..count {
        let name = string(bytes, &mut at, 64)?;
        let raw = bytes.get(at..at + 4).context("FCNS float truncated")?;
        at += 4;
        let value_bits = u32::from_le_bytes(raw.try_into().unwrap());
        let comment = string(bytes, &mut at, 256)?;
        rows.push(constants::Row {
            name,
            value_bits,
            comment,
        });
    }
    ensure!(
        at == bytes.len(),
        "FCNS trailing bytes outside observed codec"
    );
    Ok(Table { rows })
}
fn encode(table: &Table) -> Result<Vec<u8>> {
    table.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        table.rows.len() <= u32::MAX as usize,
        "FCNS row count exceeds DWORD"
    );
    let mut bytes = vec![0; 4];
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(b"SNCF");
    bytes.extend((table.rows.len() as u32).to_le_bytes());
    fn string(bytes: &mut Vec<u8>, s: &[u8]) {
        bytes.extend(s);
        bytes.push(0);
        if bytes.len() % 2 != 0 {
            bytes.push(0)
        }
    }
    for row in &table.rows {
        string(&mut bytes, &row.name);
        bytes.extend(row.value_bits.to_le_bytes());
        string(&mut bytes, &row.comment)
    }
    Ok(bytes)
}

/// Input rather than a universal default. One caller consumes this initial
/// owner at the saved-import seam; all subsequent people share World::Owner.
#[derive(Debug, Clone)]
pub struct FreshInput {
    pub owner: Owner,
    pub provider: Provider,
}
impl FreshInput {
    pub fn observed_initial(index: &Index, locator: &str) -> Result<Self> {
        let provider = Provider::observed_readonly(index, locator)?;
        let owner: Owner =
            serde_json::from_str(include_str!("autonomy_resources/initial-input.json"))?;
        owner.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            owner.configuration.source.as_ref() == Some(&provider.binding()),
            "observed initial provider/configuration differ"
        );
        Ok(Self { owner, provider })
    }
}
#[cfg(test)]
mod tests;

/// Named process input profile, never inferred from a later loaded save. The
/// newly constructed application's fresh-process options choose this profile;
/// missing historical options do not. Immutable bytes are checked before use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialProfile {
    #[serde(rename = "cc191-observed-initial-20261024")]
    CompleteCollection191ObservedInitial20261024,
}
impl InitialProfile {
    pub fn prepare(self, index: &Index) -> Result<FreshInput> {
        let candidates: Vec<_> = index
            .resources
            .iter()
            .filter(|r| {
                r.name.eq_ignore_ascii_case("Global.iff")
                    && r.source.file_name().is_some_and(|name| {
                        name.to_string_lossy().eq_ignore_ascii_case("Global.far")
                    })
            })
            .collect();
        ensure!(
            candidates.len() == 1,
            "observed Global profile provider missing or ambiguous"
        );
        FreshInput::observed_initial(index, &candidates[0].locator)
    }
}
