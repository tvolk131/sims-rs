//! IFFResFile3 reading and resident/disk writeback. Stream access is explicit.
//! Modified-file close/map regeneration is still a separate boundary. Maps supply
//! their own metadata; payload headers are checked only when first requested.
use crate::pose::clothing::name_map::PRIMES;
mod write;
pub use write::{Add, WriteStream};

/// Exact-count stream services. A short read returns -48, a closed stream -49.
/// Original files stay read-only. A host may grant writable access to an owned
/// copy; WriteStream supplies the additional native writeback/flush boundaries.
pub trait Stream {
    fn open(&mut self, path: &[u8], writable: u32) -> Result<i32, String>;
    fn seek(&mut self, offset: u32) -> Result<i32, String>;
    fn read(&mut self, output: &mut [u8]) -> Result<i32, String>;
    fn close(&mut self) -> Result<i32, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Table<T> {
    buckets: Vec<Vec<(u32, T)>>,
    len: usize,
    short: bool,
}
impl<T> Table<T> {
    fn new(short: bool) -> Self {
        Self {
            buckets: (0..193).map(|_| vec![]).collect(),
            len: 0,
            short,
        }
    }
    fn hash(&self, key: u32) -> u32 {
        if self.short {
            key as i16 as i32 as u32
        } else {
            key
        }
    }
    fn get(&self, key: u32) -> Option<&T> {
        self.buckets[self.hash(key) as usize % self.buckets.len()]
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
    fn get_mut(&mut self, key: u32) -> Option<&mut T> {
        let bucket = self.hash(key) as usize % self.buckets.len();
        self.buckets[bucket]
            .iter_mut()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }
    fn insert(&mut self, key: u32, value: T) -> Result<bool, String> {
        if self.len + 1 > self.buckets.len() {
            let n = *PRIMES
                .iter()
                .find(|&&n| n as usize > self.len)
                .ok_or("IFF resource table exceeds native bucket sizes")?
                as usize;
            let old = std::mem::replace(&mut self.buckets, (0..n).map(|_| vec![]).collect());
            for entry in old.into_iter().flatten() {
                let bucket = self.hash(entry.0) as usize % n;
                self.buckets[bucket].insert(0, entry);
            }
        }
        let bucket = self.hash(key) as usize % self.buckets.len();
        if self.buckets[bucket].iter().any(|(k, _)| *k == key) {
            return Ok(false);
        }
        self.buckets[bucket].insert(0, (key, value));
        self.len += 1;
        Ok(true)
    }
    fn remove(&mut self, key: u32) {
        let bucket = self.hash(key) as usize % self.buckets.len();
        if let Some(i) = self.buckets[bucket].iter().position(|(k, _)| *k == key) {
            self.buckets[bucket].remove(i);
            self.len -= 1;
        }
    }
    fn clear(&mut self) {
        for bucket in &mut self.buckets {
            bucket.clear();
        }
        self.len = 0;
    }
    fn iter(&self) -> impl Iterator<Item = &(u32, T)> {
        self.buckets.iter().flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Payload {
    /// Local identity; a composite host maps it into its shared handle space.
    pub handle: u32,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub tag: u32,
    pub id: u16,
    pub flags: u16,
    pub disk_flags: u16,
    pub ordinal: u16,
    pub offset: u32,
    pub name: Vec<u8>,
    pub payload: Option<Payload>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Iff {
    pub valid: bool,
    pub error: i32,
    pub map_offset: u32,
    pub unused: u32,
    pub write_cursor: u32,
    pub modified: bool,
    table: Table<Table<usize>>,
    storage: Vec<Record>,
    next_handle: u32,
}

/// Explicit bucket topology for a retained owner; importing never reinserts or
/// rehashes these chains. This data type exposes no mutable live table methods.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedTable<T> {
    pub buckets: Vec<Vec<(u32, T)>>,
    pub len: usize,
    pub short: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedHandleEpoch {
    NewUnpublished { next: u32 },
}

/// All modeled IFF fields are explicit. The caller translates native pointers
/// into a declared adapter identity domain before handing this owner over.
pub struct RetainedParts {
    pub valid: bool,
    pub error: i32,
    pub map_offset: u32,
    pub unused: u32,
    pub write_cursor: u32,
    pub modified: bool,
    pub table: RetainedTable<RetainedTable<usize>>,
    pub storage: Vec<Record>,
    pub handle_epoch: RetainedHandleEpoch,
}
impl Default for Iff {
    fn default() -> Self {
        Self {
            valid: false,
            error: 0,
            map_offset: 0,
            unused: 0,
            write_cursor: u32::MAX,
            modified: false,
            table: Table::new(false),
            storage: vec![],
            next_handle: 1,
        }
    }
}
const RSMP: u32 = u32::from_be_bytes(*b"rsmp");
const HEADER: &[u8; 64] = b"IFF FILE *.*:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1****";
impl Iff {
    pub fn from_retained_parts(parts: RetainedParts) -> Result<Self, String> {
        let RetainedHandleEpoch::NewUnpublished { next } = parts.handle_epoch;
        let table = parts.table;
        let state = Self {
            valid: parts.valid,
            error: parts.error,
            map_offset: parts.map_offset,
            unused: parts.unused,
            write_cursor: parts.write_cursor,
            modified: parts.modified,
            table: Table {
                buckets: table
                    .buckets
                    .into_iter()
                    .map(|bucket| {
                        bucket
                            .into_iter()
                            .map(|(key, inner)| {
                                (
                                    key,
                                    Table {
                                        buckets: inner.buckets,
                                        len: inner.len,
                                        short: inner.short,
                                    },
                                )
                            })
                            .collect()
                    })
                    .collect(),
                len: table.len,
                short: table.short,
            },
            storage: parts.storage,
            next_handle: next,
        };
        state.validate_retained()?;
        Ok(state)
    }

    /// Validate retained hash topology and resident identities without rebuilding
    /// buckets/order or discarding detached/deleted storage. Observer limits are
    /// host safety bounds, not claims about native capacities.
    pub fn validate_retained(&self) -> Result<(), String> {
        use std::collections::BTreeSet;
        fn table<T>(t: &Table<T>, short: bool) -> Result<(), String> {
            if t.short != short
                || t.buckets.is_empty()
                || t.buckets.len() > 1_000_000
                || t.buckets.len() != 193 && !PRIMES.iter().any(|&p| p as usize == t.buckets.len())
            {
                return Err("invalid retained IFF bucket owner".into());
            }
            let mut keys = BTreeSet::new();
            for (b, entries) in t.buckets.iter().enumerate() {
                for (k, _) in entries {
                    if t.hash(*k) as usize % t.buckets.len() != b || !keys.insert(*k) {
                        return Err("invalid retained IFF bucket topology".into());
                    }
                }
            }
            if keys.len() != t.len {
                return Err("invalid retained IFF table length".into());
            }
            Ok(())
        }
        table(&self.table, false)?;
        if self.storage.len() > 1_000_000 || self.next_handle == 0 {
            return Err("invalid retained IFF storage owner".into());
        }
        let mut indices = BTreeSet::new();
        let mut handles = BTreeSet::new();
        for (tag, inner) in self.table.iter() {
            table(inner, true)?;
            for (key, index) in inner.iter() {
                let record = self
                    .storage
                    .get(*index)
                    .ok_or("IFF retained record outside storage")?;
                if !indices.insert(*index)
                    || record.tag != *tag
                    || *key != u32::from(record.id) | (u32::from(record.flags >> 8) << 16)
                {
                    return Err("invalid retained IFF record binding".into());
                }
            }
        }
        for record in &self.storage {
            if record.name.len() > 63 || record.name.contains(&0) {
                return Err("invalid retained IFF name".into());
            }
            if let Some(payload) = &record.payload {
                if payload.handle == 0
                    || payload.handle >= self.next_handle
                    || payload.bytes.len() > 128 * 1024 * 1024
                    || !handles.insert(payload.handle)
                {
                    return Err("invalid retained IFF payload identity".into());
                }
            }
        }
        Ok(())
    }
    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.table
            .iter()
            .flat_map(|(_, t)| t.iter().map(|(_, i)| &self.storage[*i]))
    }
    /// Includes allocations in retained storage that no longer have a live map
    /// key. A resource host must account for those identities as well.
    pub fn resident_payloads(&self) -> impl Iterator<Item = &Payload> {
        self.storage
            .iter()
            .filter_map(|record| record.payload.as_ref())
    }
    fn clear(&mut self) {
        self.table.clear();
        self.storage.clear();
    }
    fn read_at(stream: &mut impl Stream, offset: u32, out: &mut [u8]) -> Result<i32, String> {
        let code = stream.seek(offset)?;
        if code != 0 {
            return Ok(code);
        }
        stream.read(out)
    }
    fn chunk(stream: &mut impl Stream, offset: u32) -> Result<Result<Header, i32>, String> {
        let mut bytes = [0; 76];
        let code = Self::read_at(stream, offset, &mut bytes)?;
        Ok(if code != 0 {
            Err(code)
        } else {
            Ok(Header::new(&bytes))
        })
    }
    fn physical(&mut self, stream: &mut impl Stream) -> Result<(), String> {
        let mut offset = 64_u32;
        loop {
            let Ok(chunk) = Self::chunk(stream, offset)? else {
                break;
            };
            if chunk.size < 76 || chunk.size as i32 > 0x4000_0000 {
                break;
            }
            if chunk.flags & 4 != 0 {
                self.unused = self.unused.wrapping_add(chunk.size);
            } else {
                let key = u32::from(chunk.id) | (u32::from(chunk.flags >> 8) << 16);
                if self.table.get(chunk.tag).is_none() {
                    self.table.insert(chunk.tag, Table::new(true))?;
                }
                let inner = self.table.get_mut(chunk.tag).unwrap();
                let i = if let Some(&i) = inner.get(key) {
                    i
                } else {
                    let ordinal = inner
                        .iter()
                        .map(|(_, i)| self.storage[*i].ordinal as i16)
                        .fold(0_i16, i16::max)
                        .wrapping_add(1) as u16;
                    let i = self.storage.len();
                    self.storage.push(Record {
                        tag: chunk.tag,
                        id: chunk.id,
                        flags: chunk.flags & 0xff00,
                        disk_flags: 0xffff,
                        ordinal,
                        offset,
                        name: vec![],
                        payload: None,
                    });
                    inner.insert(key, i)?;
                    i
                };
                let record = &mut self.storage[i];
                record.flags = (record.flags & 0xff00) | (chunk.flags & 255);
                record.disk_flags = chunk.flags;
                record.offset = offset;
                record.name = chunk.name;
                record.payload = None;
            }
            let next = offset.wrapping_add(chunk.size);
            if next == offset {
                return Err("IFF physical scan did not advance".into());
            }
            offset = next;
        }
        Ok(())
    }
    /// Open preserves the resource error word. Header/read errors close the
    /// stream; an invalid map header leaves an open stream and an empty table.
    pub fn open(
        &mut self,
        path: &[u8],
        writable: u32,
        stream: &mut impl Stream,
    ) -> Result<i32, String> {
        if self.valid {
            return Ok(-47);
        }
        let code = stream.open(path, writable)?;
        if code != 0 {
            return Ok(code);
        }
        self.valid = true;
        let mut header = [0; 64];
        let mut code = Self::read_at(stream, 0, &mut header)?;
        if code == 0
            && (HEADER.iter().zip(header).any(|(&a, b)| a != b'*' && a != b)
                || !matches!((header[9], header[11]), (b'2', b'0' | b'5')))
        {
            code = -93;
        }
        if code != 0 {
            stream.close()?;
            self.valid = false;
            return Ok(code);
        }
        let map = if header[11] == b'5' {
            self.map_offset = u32::from_be_bytes(header[60..64].try_into().unwrap());
            self.map_offset
        } else {
            0
        };
        self.clear();
        self.table = Table::new(false);
        self.unused = 0;
        let mut fallback = map == 0;
        if map != 0 {
            let Ok(chunk) = Self::chunk(stream, map)? else {
                return Ok(-47);
            };
            if chunk.flags & 4 != 0 || chunk.tag != RSMP {
                return Ok(-47);
            }
            self.table.insert(RSMP, Table::new(true))?;
            self.storage.push(Record {
                tag: RSMP,
                id: chunk.id,
                flags: chunk.flags,
                disk_flags: 0xffff,
                ordinal: 1,
                offset: map,
                name: chunk.name,
                payload: None,
            });
            self.table
                .get_mut(RSMP)
                .unwrap()
                .insert(u32::from(chunk.id) | (u32::from(chunk.flags >> 8) << 16), 0)?;
            let (code, handle) = self.load(0, false, stream)?;
            if code == 0 && handle != 0 {
                let bytes = self.storage[0].payload.take().unwrap().bytes;
                self.read_map(&bytes)?;
                fallback = self.table.get(RSMP).and_then(|t| t.get(0)).is_some();
                if fallback {
                    self.clear();
                }
            } else {
                fallback = true;
            }
        }
        if fallback {
            self.physical(stream)?;
        }
        self.table.remove(RSMP);
        // 41c5fd: only the successful Open tail resets this open epoch.
        self.modified = false;
        Ok(0)
    }
    fn read_map(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut r = MapReader { bytes, pos: 0 };
        let header = r.take(12)?;
        if header[0] != 0 {
            return Err("compressed resource-map stream is not implemented".into());
        }
        self.unused = r.u32()?;
        let types = r.u32()? as i32;
        self.clear();
        for _ in 0..types {
            let tag = r.u32()?;
            let count = r.u32()? as i32;
            if count <= 0 {
                continue;
            }
            if self.table.get(tag).is_some() {
                return Err(
                    "duplicate resource-map type would dereference an invalid native table".into(),
                );
            }
            self.table.insert(tag, Table::new(true))?;
            for ordinal in 1..=count {
                let offset = r.u32()?;
                let id = r.u16()?;
                let flags = r.u16()?;
                let name = r.name()?;
                let key = u32::from(id) | (u32::from(flags >> 8) << 16);
                let i = self.storage.len();
                if self.table.get_mut(tag).unwrap().insert(key, i)? {
                    self.storage.push(Record {
                        tag,
                        id,
                        flags,
                        disk_flags: flags,
                        ordinal: ordinal as u16,
                        offset,
                        name,
                        payload: None,
                    });
                }
            }
        }
        Ok(())
    }
    fn guard(&mut self) -> bool {
        self.error = if self.valid { 0 } else { -49 };
        self.valid
    }
    pub fn count(&mut self, tag: u32) -> u16 {
        self.count_raw(tag) as u16
    }
    /// The implementation returns the table's whole word; callers whose ABI
    /// exposes a short consume its low half. Composite subtraction uses it too.
    pub fn count_raw(&mut self, tag: u32) -> u32 {
        if !self.guard() {
            return 0;
        }
        self.table.get(tag).map_or(0, |t| t.len as u32)
    }
    fn load(
        &mut self,
        index: usize,
        callback: bool,
        stream: &mut impl Stream,
    ) -> Result<(i32, u32), String> {
        let record = &mut self.storage[index];
        if let Some(payload) = &record.payload {
            return Ok((0, payload.handle));
        }
        let chunk = match Self::chunk(stream, record.offset)? {
            Ok(chunk) => chunk,
            Err(code) => return Ok((code, 0)),
        };
        let size = chunk.size.wrapping_sub(76) as i32;
        if !(0..=0x4000_0000).contains(&size) || chunk.tag != record.tag {
            return Ok((-1, 0));
        }
        if size > 128 * 1024 * 1024 {
            return Err("IFF payload exceeds the read-only host's 128 MiB resource limit".into());
        }
        // Native heap contents on a failed post-allocation seek are undefined.
        // Safe Rust storage starts zeroed; no native-byte claim is made for it.
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size as usize)
            .map_err(|_| "IFF payload allocation failed")?;
        bytes.resize(size as usize, 0);
        let handle = self.next_handle;
        self.next_handle = handle.checked_add(1).ok_or("IFF handle space exhausted")?;
        record.payload = Some(Payload { handle, bytes });
        let code = stream.seek(record.offset.wrapping_add(76))?;
        if code != 0 {
            return Ok((code, handle));
        }
        let code = stream.read(&mut record.payload.as_mut().unwrap().bytes)?;
        if code != 0 {
            record.payload = None;
            return Ok((code, 0));
        }
        // The catalog supplies 46fb64, the original no-op callback. The
        // callback bit still changes, and a cached payload is never called again.
        if record.disk_flags & 16 == 0 && callback {
            record.flags |= 16;
        }
        Ok((0, handle))
    }
    pub fn by_id(
        &mut self,
        tag: u32,
        id: u32,
        callback: bool,
        stream: &mut impl Stream,
    ) -> Result<u32, String> {
        if !self.guard() {
            return Ok(0);
        }
        self.query(
            self.table.get(tag).and_then(|t| t.get(id & 65535)).copied(),
            callback,
            stream,
        )
    }
    pub fn ordinal(
        &mut self,
        tag: u32,
        ordinal: u32,
        callback: bool,
        stream: &mut impl Stream,
    ) -> Result<u32, String> {
        if !self.guard() {
            return Ok(0);
        }
        let index = self.table.get(tag).and_then(|t| {
            t.iter()
                .find(|(_, i)| self.storage[*i].ordinal == ordinal as u16)
                .map(|(_, i)| *i)
        });
        self.query(index, callback, stream)
    }
    fn query(
        &mut self,
        index: Option<usize>,
        callback: bool,
        stream: &mut impl Stream,
    ) -> Result<u32, String> {
        let Some(i) = index else {
            self.error = -98;
            return Ok(0);
        };
        let (error, handle) = self.load(i, callback, stream)?;
        self.error = error;
        Ok(handle)
    }
    pub fn payload(&self, handle: u32) -> Option<&[u8]> {
        self.records()
            .filter_map(|r| r.payload.as_ref())
            .find(|p| p.handle == handle)
            .map(|p| p.bytes.as_slice())
    }
    /// Resident resource storage, including native OBJD normalization writes.
    pub fn payload_mut(&mut self, handle: u32) -> Option<&mut Vec<u8>> {
        let index = self
            .table
            .iter()
            .flat_map(|(_, t)| t.iter())
            .find_map(|(_, i)| {
                self.storage[*i]
                    .payload
                    .as_ref()
                    .is_some_and(|p| p.handle == handle)
                    .then_some(*i)
            })?;
        self.storage[index].payload.as_mut().map(|p| &mut p.bytes)
    }
    /// 41ce00 followed by 465918: discard the resident payload and restore
    /// the low flag byte from its disk metadata. A later query reloads bytes.
    /// The caller owns freeing the detached native handle; Rust drops it here.
    pub fn release_chunk(&mut self, handle: u32) -> Result<(), String> {
        if !self.guard() {
            return Ok(());
        }
        let index = self.table.iter().find_map(|(_, t)| {
            t.iter().find_map(|(_, i)| {
                (self.storage[*i].payload.as_ref().map_or(0, |p| p.handle) == handle).then_some(*i)
            })
        });
        let Some(index) = index else {
            self.error = -99;
            return Ok(());
        };
        let record = &mut self.storage[index];
        if record.offset == u32::MAX {
            return Err("release of an unsaved IFF resource is not implemented".into());
        }
        record.payload = None;
        record.flags = (record.flags & 0xff00) | (record.disk_flags & 255);
        Ok(())
    }
    pub fn name(&mut self, handle: u32, output: &mut Vec<u8>) {
        if !self.guard() {
            return;
        }
        if let Some(record) = self
            .records()
            .find(|r| r.payload.as_ref().map_or(0, |p| p.handle) == handle)
        {
            *output = record.name.clone();
        } else {
            output.clear();
            self.error = -98;
        }
    }
    /// The original writes only the low 16 bits of the supplied output word.
    pub fn id(&mut self, handle: u32, output: &mut u32) {
        if !self.guard() {
            return;
        }
        if let Some(record) = self
            .records()
            .find(|r| r.payload.as_ref().map_or(0, |p| p.handle) == handle)
        {
            *output = (*output & 0xffff_0000) | u32::from(record.id);
        } else {
            self.error = -98;
        }
    }
    pub fn close(&mut self, stream: &mut impl Stream) -> Result<i32, String> {
        if !self.guard() {
            return Ok(self.error);
        }
        if self.modified {
            return Err(
                "modified IFF close requires native resource-map rebuild/compaction".into(),
            );
        }
        self.clear();
        self.unused = 0;
        self.error = stream.close()?;
        self.valid = false;
        Ok(self.error)
    }
}

#[cfg(test)]
mod open_epoch_tests {
    use super::*;
    struct Input {
        bytes: Vec<u8>,
        position: usize,
        opened: bool,
        open_error: i32,
        seek_error: bool,
        opens: usize,
        closes: usize,
    }
    impl Input {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                position: 0,
                opened: false,
                open_error: 0,
                seek_error: false,
                opens: 0,
                closes: 0,
            }
        }
    }
    impl Stream for Input {
        fn open(&mut self, _: &[u8], _: u32) -> Result<i32, String> {
            self.opens += 1;
            self.opened = self.open_error == 0;
            Ok(self.open_error)
        }
        fn seek(&mut self, offset: u32) -> Result<i32, String> {
            self.position = offset as usize;
            Ok(if self.seek_error { -48 } else { 0 })
        }
        fn read(&mut self, out: &mut [u8]) -> Result<i32, String> {
            let input = self.bytes.get(self.position..).unwrap_or_default();
            let n = input.len().min(out.len());
            out[..n].copy_from_slice(&input[..n]);
            self.position += n;
            Ok(if n == out.len() { 0 } else { -48 })
        }
        fn close(&mut self) -> Result<i32, String> {
            self.closes += 1;
            self.opened = false;
            Ok(0)
        }
    }
    fn header(version: u8, map: u32) -> Vec<u8> {
        let mut bytes = HEADER.to_vec();
        bytes[9] = b'2';
        bytes[11] = version;
        bytes[60..64].copy_from_slice(&map.to_be_bytes());
        bytes
    }
    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 76];
        bytes[..4].copy_from_slice(tag);
        bytes[4..8].copy_from_slice(&(76u32 + payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }
    fn closed(modified: bool) -> Iff {
        // Explicit closed input; modified-file Close/map regeneration remains
        // a separate unsupported boundary, not a fake successful close here.
        Iff {
            error: -7,
            write_cursor: 404,
            modified,
            ..Iff::default()
        }
    }
    #[test]
    fn successful_scan_and_map_opens_reset_only_modified_at_the_tail() {
        let mut mapped = header(b'5', 64);
        let mut map = [0; 20];
        map[8..12].copy_from_slice(b"pmsr");
        mapped.extend(chunk(b"rsmp", &map));
        for bytes in [header(b'0', 0), header(b'5', 0), mapped] {
            let mut expected = closed(false);
            let mut actual = closed(true);
            assert_eq!(
                expected.open(b"test.iff", 1, &mut Input::new(bytes.clone())),
                Ok(0)
            );
            assert_eq!(actual.open(b"test.iff", 1, &mut Input::new(bytes)), Ok(0));
            assert_eq!(actual, expected); // all other fields/counters/errors
            assert!(actual.valid && !actual.modified);
            assert_eq!(actual.error, -7);
            assert_eq!(actual.write_cursor, 404);
        }
    }
    #[test]
    fn failed_opens_preserve_modified_until_a_later_successful_reopen() {
        let mut already_open = closed(true);
        already_open.valid = true;
        let before = already_open.clone();
        let mut stream = Input::new(header(b'0', 0));
        assert_eq!(already_open.open(b"test.iff", 1, &mut stream), Ok(-47));
        assert_eq!(already_open, before);
        assert_eq!(stream.opens, 0);
        for kind in 0..5 {
            let mut state = closed(true);
            let mut stream = Input::new(header(b'0', 0));
            let code = match kind {
                0 => {
                    stream.open_error = -50;
                    -50
                }
                1 => {
                    stream.seek_error = true;
                    -48
                }
                2 => {
                    stream.bytes.truncate(12);
                    -48
                }
                3 => {
                    stream.bytes[0] = b'X';
                    -93
                }
                _ => {
                    stream.bytes = header(b'5', 64);
                    stream.bytes.extend(chunk(b"NOPE", &[]));
                    -47
                }
            };
            assert_eq!(state.open(b"test.iff", 1, &mut stream), Ok(code));
            assert!(state.modified, "failure {kind} reset the epoch");
            assert_eq!(state.error, -7);
            assert_eq!(state.write_cursor, 404);
            if kind == 4 {
                // A bad map leaves the stream open; an unmodified diagnostic
                // flag is not invented to make modified Close succeed.
                assert!(state.valid && stream.opened);
                assert_eq!(stream.closes, 0);
                continue;
            }
            assert!(!state.valid);
            assert_eq!(stream.closes, usize::from(kind != 0));
            stream.bytes = header(b'0', 0);
            stream.open_error = 0;
            stream.seek_error = false;
            assert_eq!(state.open(b"test.iff", 1, &mut stream), Ok(0));
            assert!(!state.modified && state.valid);
            assert_eq!(state.error, -7);
        }
    }
}

struct Header {
    tag: u32,
    size: u32,
    id: u16,
    flags: u16,
    name: Vec<u8>,
}
impl Header {
    fn new(b: &[u8; 76]) -> Self {
        Self {
            tag: u32::from_be_bytes(b[..4].try_into().unwrap()),
            size: u32::from_be_bytes(b[4..8].try_into().unwrap()),
            id: u16::from_be_bytes(b[8..10].try_into().unwrap()),
            flags: u16::from_be_bytes(b[10..12].try_into().unwrap()),
            name: super::cstring(&b[12..])[..super::cstring(&b[12..]).len().min(63)].to_vec(),
        }
    }
}
struct MapReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> MapReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or("resource-map cursor overflow")?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or("truncated resource-map stream")?;
        self.pos = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn name(&mut self) -> Result<Vec<u8>, String> {
        let available = &self.bytes[self.pos..];
        let n = available[..available.len().min(63)]
            .iter()
            .position(|&b| b == 0)
            .or_else(|| (available.len() >= 63).then_some(63))
            .ok_or("truncated resource-map name")?;
        let name = self.take(n)?.to_vec();
        // 464c3e advances by the destination CFString64 length, including
        // truncation, then adds one and aligns. Do not scan to a later NUL.
        self.take(1)?;
        if self.pos & 1 != 0 {
            self.take(1)?;
        }
        Ok(name)
    }
}

#[cfg(test)]
mod retained_parts_tests {
    use super::*;

    fn parts() -> RetainedParts {
        RetainedParts {
            valid: true,
            error: -45,
            map_offset: 128,
            unused: 76,
            write_cursor: 404,
            modified: true,
            table: RetainedTable {
                buckets: vec![vec![]; 193],
                len: 0,
                short: false,
            },
            // An allocation not currently reachable through a map key must
            // still survive, with its declared local identity.
            storage: vec![Record {
                tag: 9,
                id: 1,
                flags: 16,
                disk_flags: 0,
                ordinal: 1,
                offset: u32::MAX,
                name: vec![65],
                payload: Some(Payload {
                    handle: 3,
                    bytes: vec![7, 8],
                }),
            }],
            handle_epoch: RetainedHandleEpoch::NewUnpublished { next: 4 },
        }
    }
    #[test]
    fn complete_retained_parts_keep_detached_storage_and_every_scalar() {
        let state = Iff::from_retained_parts(parts()).unwrap();
        assert!(state.valid && state.modified);
        assert_eq!(
            (
                state.error,
                state.map_offset,
                state.unused,
                state.write_cursor
            ),
            (-45, 128, 76, 404)
        );
        assert_eq!(state.next_handle, 4);
        // Existing payload lookup deliberately follows live map keys only.
        assert_eq!(state.payload(3), None);
        assert_eq!(state.resident_payloads().next().unwrap().bytes, [7, 8]);
        assert_eq!(state.resident_payloads().count(), 1);
        assert_eq!(state.records().count(), 0);
        let restored: Iff = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, restored);
    }
    #[test]
    fn retained_parts_reject_bad_table_length_and_handle_aliases() {
        let mut bad = parts();
        bad.table.len = 1;
        assert!(Iff::from_retained_parts(bad).is_err());
        let mut bad = parts();
        bad.handle_epoch = RetainedHandleEpoch::NewUnpublished { next: 3 };
        assert!(Iff::from_retained_parts(bad).is_err());
        let mut bad = parts();
        bad.storage.push(bad.storage[0].clone());
        assert!(Iff::from_retained_parts(bad).is_err());
        let mut bad = parts();
        bad.table.buckets[0].push((
            1,
            RetainedTable {
                buckets: vec![vec![]; 193],
                len: 0,
                short: true,
            },
        ));
        bad.table.len = 1;
        assert!(Iff::from_retained_parts(bad).is_err());
    }
}
