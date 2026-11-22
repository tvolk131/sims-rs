use crate::Result;
use crate::reader::{Reader, western};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// A chunk flag: the chunk is deleted, free space to be reused.
pub const DELETED: u16 = 4;

const FILE_HEADER_SIZE: usize = 64;
const CHUNK_HEADER_SIZE: usize = 76;
const SIGNATURE_25: &[u8; 60] = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1";

/// A chunk's header fields and payload, independent of an IFF's layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    pub tag: [u8; 4],
    pub id: u16,
    pub flags: u16,
    pub label: String,
    pub data: Vec<u8>,
}

/// Writes a fresh Maxis IFF 2.5 in the supplied chunk order, without a
/// resource map. Labels use the same Windows-1252 encoding as [`Iff::parse`].
pub fn write(chunks: &[ChunkData]) -> Result<Vec<u8>> {
    let mut bytes = SIGNATURE_25.to_vec();
    bytes.resize(FILE_HEADER_SIZE, 0);
    for chunk in chunks {
        let offset = bytes.len();
        let size = u32::try_from(chunk.data.len())
            .ok()
            .and_then(|size| size.checked_add(CHUNK_HEADER_SIZE as u32))
            .ok_or_else(|| crate::Error::new(offset, "IFF chunk size exceeds 32 bits"))?;
        let label = crate::far::encode_name(&chunk.label)
            .filter(|label| label.len() <= 64 && !label.contains(&0))
            .ok_or_else(|| crate::Error::new(offset, "invalid IFF chunk label"))?;
        bytes.extend(chunk.tag);
        bytes.extend(size.to_be_bytes());
        bytes.extend(chunk.id.to_be_bytes());
        bytes.extend(chunk.flags.to_be_bytes());
        bytes.extend(label);
        bytes.resize(offset + CHUNK_HEADER_SIZE, 0);
        bytes.extend(&chunk.data);
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub tag: [u8; 4],
    pub id: u16,
    pub flags: u16,
    pub label: String,
    pub offset: usize,
    pub payload: Range<usize>,
}
impl Chunk {
    pub fn kind(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}
#[derive(Debug, Clone)]
pub struct Iff {
    pub bytes: Vec<u8>,
    /// Every chunk in file order, deleted and replaced ones included. The
    /// game's resources are [`Iff::get`], [`Iff::all`] and
    /// [`Iff::resource_order`].
    pub chunks: Vec<Chunk>,
    resources: Resources,
}

/// The resources an IFF file holds, as the original's IFFResFile3 table
/// keeps them (`legacy/crates/sims-sim/src/object_catalog/iff.rs`, from
/// 0x41c252 and 0x41be90). A resource's key is its type, its ID and the
/// high byte of its flags.
///
/// - Without a resource map, the live chunks in file order. Deleted chunks
///   are skipped. A chunk whose key an earlier chunk has replaces it and
///   takes its place in the order.
/// - With a version-0 resource map, the map's entries in its order. Of two
///   entries with one key, the first is kept. A live chunk the map does
///   not list is not a resource. A map that lists a resource map with ID 0
///   is ignored, and the chunks are read as if there were none.
///
/// **Unverified** against the original: deleted chunks with a live twin,
/// replaced chunks and maps listing a map. These rules follow the frozen
/// prototype's reader. The game's own files hold no deleted chunk sharing
/// a key with a live one, and their only chunks sharing a type and ID are
/// `BMP_` chunks told apart by the flags' high byte.
#[derive(Debug, Clone, Default)]
struct Resources {
    /// Indices into [`Iff::chunks`] by type, in resource order.
    order: BTreeMap<[u8; 4], Vec<usize>>,
    /// Indices into [`Iff::chunks`] by key.
    by_key: BTreeMap<([u8; 4], u16, u8), usize>,
    /// The flags the resource map lists for a resource, by index into
    /// [`Iff::chunks`]; see [`Iff::resource_flags`].
    map_flags: BTreeMap<usize, u16>,
    /// Why the resource map could not be used. The table is then the
    /// chunks read as if there were no map, so lookups still work, but
    /// [`Iff::resource_order`] fails.
    map_error: Option<crate::Error>,
}

/// Compact header metadata for a single chunk used by both [`Iff::parse`] and
/// the streaming [`Iff::scan_types`] / [`Iff::scan_types_from_reader`] scanners.
#[derive(Debug, Clone, Copy)]
struct ChunkStub {
    tag: [u8; 4],
    id: u16,
    flags: u16,
    offset: usize,
    payload_start: usize,
    payload_end: usize,
}

#[derive(Debug, Clone, Copy)]
struct RawRsmpEntry {
    offset: u32,
    id: u16,
    flags: u16,
}

struct RawRsmp {
    types: Vec<([u8; 4], Range<usize>)>,
    entries: Vec<RawRsmpEntry>,
}

/// Parses a version-0 `rsmp` chunk without allocating a `BTreeMap` or decoding
/// entry label `String`s, enforcing the exact same validation and error
/// offsets as [`ResourceMap::parse`].
///
/// Returns `Ok(None)` when the map lists an `rsmp` entry with ID 0 and high
/// flag byte 0 (causing the map to be ignored).
fn parse_rsmp_raw(data: &[u8]) -> Result<Option<RawRsmp>> {
    let mut r = Reader::new(data);
    if r.take(8)? != [0; 8] || r.take(4)? != b"pmsr" {
        return r.fail("unsupported resource-map stream header");
    }
    let _unused_bytes = r.u32()?;
    let count = r.u32()? as usize;
    r.count(count, 8)?;
    let mut types: Vec<([u8; 4], Range<usize>)> = Vec::with_capacity(count);
    let mut seen_tags: BTreeSet<[u8; 4]> = BTreeSet::new();
    let mut entries: Vec<RawRsmpEntry> = Vec::new();
    for _ in 0..count {
        let tag = r.u32()?.to_be_bytes();
        let n = r.u32()? as usize;
        r.count(n, 10)?;
        let start = entries.len();
        entries.reserve(n);
        for _ in 0..n {
            let offset = r.u32()?;
            let id = r.u16()?;
            let flags = r.u16()?;
            let _label = r.cstr()?;
            if r.pos & 1 != 0 {
                r.skip(1)?;
            }
            entries.push(RawRsmpEntry { offset, id, flags });
        }
        if !seen_tags.insert(tag) {
            return r.fail("duplicate resource-map type");
        }
        types.push((tag, start..entries.len()));
    }
    if r.remaining() != 0 {
        return r.fail("unconsumed resource-map bytes");
    }
    // Sort by tag to match `ResourceMap`'s `BTreeMap<[u8; 4], _>` iteration order.
    types.sort_unstable_by_key(|(tag, _)| *tag);
    if let Ok(pos) = types.binary_search_by_key(b"rsmp", |(tag, _)| *tag) {
        let range = types[pos].1.clone();
        if entries[range]
            .iter()
            .any(|e| e.id == 0 && e.flags >> 8 == 0)
        {
            return Ok(None);
        }
    }
    Ok(Some(RawRsmp { types, entries }))
}

fn decode_label_fast(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        String::new()
    } else if bytes.is_ascii() {
        // SAFETY-FREE: every ASCII byte is valid UTF-8.
        std::str::from_utf8(bytes)
            .expect("ASCII is valid UTF-8")
            .to_owned()
    } else {
        western(bytes)
    }
}

impl Resources {
    /// Adds a resource, or replaces the one with its key where it stands.
    /// With `first_wins`, a resource whose key is taken is dropped instead.
    fn add_stub(&mut self, tag: [u8; 4], id: u16, flags: u16, index: usize, first_wins: bool) {
        let key = (tag, id, (flags >> 8) as u8);
        let order = self.order.entry(tag).or_default();
        match self.by_key.insert(key, index) {
            None => order.push(index),
            Some(old) if first_wins => {
                self.by_key.insert(key, old);
            }
            Some(old) => {
                let at = order.iter().position(|&i| i == old).expect("listed");
                order[at] = index;
            }
        }
    }

    fn physical(chunks: &[Chunk]) -> Self {
        let mut resources = Self::default();
        for (index, chunk) in chunks.iter().enumerate() {
            if chunk.flags & DELETED == 0 {
                resources.add_stub(chunk.tag, chunk.id, chunk.flags, index, false);
            }
        }
        resources
    }

    /// The table for `bytes`: from its resource map if it has one. A
    /// resource map is never a resource itself: the open removes its type.
    fn build(bytes: &[u8], chunks: &[Chunk]) -> Self {
        let mut resources = Self::with_map(bytes, chunks);
        resources.order.remove(b"rsmp");
        resources.by_key.retain(|key, _| key.0 != *b"rsmp");
        resources
    }

    fn with_map(bytes: &[u8], chunks: &[Chunk]) -> Self {
        // Only 2.5 headers carry a map offset, in bytes 60..64. In 2.0
        // headers those bytes are the end of the signature text
        // ("...MAXIS 1996\0").
        let offset = if bytes.starts_with(b"IFF FILE 2.5:") {
            u32::from_be_bytes(bytes[60..64].try_into().unwrap()) as usize
        } else {
            0
        };
        if offset == 0 {
            return Self::physical(chunks);
        }
        match Self::mapped(bytes, chunks, offset) {
            Ok(Some(resources)) => resources,
            Ok(None) => Self::physical(chunks),
            Err(error) => Self {
                map_error: Some(error),
                ..Self::physical(chunks)
            },
        }
    }

    /// The table from the resource map at `offset`, or None when the map
    /// lists a map with ID 0.
    fn mapped(bytes: &[u8], chunks: &[Chunk], offset: usize) -> Result<Option<Self>> {
        // Chunks are in file order, so by offset.
        let at = |offset: usize| {
            chunks
                .binary_search_by_key(&offset, |c| c.offset)
                .ok()
                .map(|i| (i, &chunks[i]))
        };
        let map = at(offset)
            .filter(|(_, c)| c.tag == *b"rsmp" && c.flags & DELETED == 0)
            .ok_or_else(|| crate::Error::new(offset, "IFF resource map not found"))?
            .1;
        let Some(RawRsmp { types, entries }) = parse_rsmp_raw(&bytes[map.payload.clone()])? else {
            return Ok(None);
        };
        let mut resources = Self::default();
        for (tag, range) in types {
            for entry in &entries[range] {
                // A later entry with a key already listed is dropped
                // unread, as the original's table insert drops it.
                let key = (tag, entry.id, (entry.flags >> 8) as u8);
                if resources.by_key.contains_key(&key) {
                    continue;
                }
                // The entry's chunk must be there with the entry's key. Its
                // low flag bits are the map's to state, as in the
                // prototype, which reads the payload wherever the map
                // points and checks only the type there. **Unverified:**
                // the original looks the resource up by the map's ID and
                // flags' high byte (0x41bab1, 0x41d92b) and checks only
                // the chunk's type (0x41d33b), so a chunk whose ID or high
                // flag byte differs would be read there under the map's;
                // here the map is refused. No file of the install has one.
                let index = at(entry.offset as usize)
                    .filter(|(_, c)| {
                        c.tag == tag && c.id == entry.id && c.flags >> 8 == entry.flags >> 8
                    })
                    .ok_or_else(|| {
                        crate::Error::new(
                            entry.offset as usize,
                            "resource map entry disagrees with chunk header",
                        )
                    })?
                    .0;
                resources.add_stub(tag, entry.id, chunks[index].flags, index, true);
                resources.map_flags.insert(index, entry.flags);
            }
        }
        Ok(Some(resources))
    }

    fn physical_stubs(stubs: &[ChunkStub], tags: &[[u8; 4]]) -> Self {
        let mut resources = Self::default();
        for (index, stub) in stubs.iter().enumerate() {
            if stub.flags & DELETED == 0 && tags.contains(&stub.tag) {
                resources.add_stub(stub.tag, stub.id, stub.flags, index, false);
            }
        }
        resources
    }

    fn mapped_stubs(
        rsmp_payload: &[u8],
        stubs: &[ChunkStub],
        tags: &[[u8; 4]],
    ) -> Result<Option<Self>> {
        let at = |offset: usize| {
            stubs
                .binary_search_by_key(&offset, |c| c.offset)
                .ok()
                .map(|i| (i, &stubs[i]))
        };
        let Some(RawRsmp { types, entries }) = parse_rsmp_raw(rsmp_payload)? else {
            return Ok(None);
        };
        let mut resources = Self::default();
        for (tag, range) in types {
            let keep = tags.contains(&tag);
            // Track seen keys for this tag even if not kept so duplicate-key
            // entries are skipped unread exactly as in `Resources::mapped`.
            let mut skipped_keys: BTreeSet<(u16, u8)> = BTreeSet::new();
            for entry in &entries[range] {
                let flags_hi = (entry.flags >> 8) as u8;
                if keep {
                    let key = (tag, entry.id, flags_hi);
                    if resources.by_key.contains_key(&key) {
                        continue;
                    }
                } else if !skipped_keys.insert((entry.id, flags_hi)) {
                    continue;
                }
                let index = at(entry.offset as usize)
                    .filter(|(_, c)| {
                        c.tag == tag && c.id == entry.id && c.flags >> 8 == entry.flags >> 8
                    })
                    .ok_or_else(|| {
                        crate::Error::new(
                            entry.offset as usize,
                            "resource map entry disagrees with chunk header",
                        )
                    })?
                    .0;
                if keep {
                    resources.add_stub(tag, entry.id, stubs[index].flags, index, true);
                    resources.map_flags.insert(index, entry.flags);
                }
            }
        }
        Ok(Some(resources))
    }
}
impl Iff {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let mut r = Reader::new(&bytes);
        let header = r.take(FILE_HEADER_SIZE)?;
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return r.fail("invalid Maxis IFF signature");
        }
        let mut chunks = Vec::new();
        while r.remaining() > 0 {
            let offset = r.pos;
            let tag = r.take(4)?.try_into().unwrap();
            let length = r.be32()? as usize;
            if length < CHUNK_HEADER_SIZE {
                return r.fail("IFF chunk shorter than its 76-byte header");
            }
            let id = r.be16()?;
            let flags = r.be16()?;
            let label_bytes = r.take(64)?;
            let end = label_bytes.iter().position(|b| *b == 0).unwrap_or(64);
            let label = decode_label_fast(&label_bytes[..end]);
            let start = r.pos;
            r.skip(length - CHUNK_HEADER_SIZE)?;
            chunks.push(Chunk {
                tag,
                id,
                flags,
                label,
                offset,
                payload: start..r.pos,
            });
        }
        let resources = Resources::build(&bytes, &chunks);
        Ok(Self {
            bytes,
            chunks,
            resources,
        })
    }
    /// The flags the original reads a resource's byte order from
    /// (0x41d3a7): its resource map entry's when the file's map is used,
    /// else the chunk header's.
    pub fn resource_flags(&self, chunk: &Chunk) -> u16 {
        self.chunks
            .binary_search_by_key(&chunk.offset, |c| c.offset)
            .ok()
            .and_then(|index| self.resources.map_flags.get(&index))
            .copied()
            .unwrap_or(chunk.flags)
    }
    pub fn data(&self, chunk: &Chunk) -> &[u8] {
        &self.bytes[chunk.payload.clone()]
    }
    /// Raw NUL-terminated Windows-1252 label bytes of `chunk` sliced directly
    /// from [`Iff::bytes`] without allocating.
    pub fn label_bytes(&self, chunk: &Chunk) -> &[u8] {
        let raw = &self.bytes[chunk.offset + 12..chunk.offset + CHUNK_HEADER_SIZE];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        &raw[..end]
    }
    /// Decodes `chunk`'s label lazily from [`Iff::bytes`], borrowing directly
    /// when pure ASCII.
    pub fn chunk_label(&self, chunk: &Chunk) -> Cow<'_, str> {
        let raw = self.label_bytes(chunk);
        if raw.is_ascii() {
            Cow::Borrowed(std::str::from_utf8(raw).expect("ASCII is valid UTF-8"))
        } else {
            Cow::Owned(western(raw))
        }
    }
    /// Walks chunk headers identically to [`Iff::parse`] (validating the
    /// complete file framing without allocating chunk labels or parsing `rsmp`)
    /// and returns the payload of the first physical chunk matching `tag`.
    /// Matches `Iff::parse(bytes)?.chunks.iter().find(|c| &c.tag == tag)`.
    pub fn find_first_physical(bytes: &[u8], target: &[u8; 4]) -> Result<Option<Vec<u8>>> {
        let mut r = Reader::new(bytes);
        let header = r.take(FILE_HEADER_SIZE)?;
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return r.fail("invalid Maxis IFF signature");
        }
        let mut found: Option<Range<usize>> = None;
        while r.remaining() > 0 {
            let tag: [u8; 4] = r.take(4)?.try_into().unwrap();
            let length = r.be32()? as usize;
            if length < CHUNK_HEADER_SIZE {
                return r.fail("IFF chunk shorter than its 76-byte header");
            }
            let _id = r.be16()?;
            let _flags = r.be16()?;
            let _label = r.take(64)?;
            let start = r.pos;
            r.skip(length - CHUNK_HEADER_SIZE)?;
            if found.is_none() && &tag == target {
                found = Some(start..r.pos);
            }
        }
        Ok(found.map(|range| bytes[range].to_vec()))
    }
    /// Scans only the requested chunk `tags` from an in-memory IFF buffer,
    /// reading headers and `rsmp` zero-copy directly from `bytes` (zero chunk
    /// label allocations, zero `ResourceMap` `BTreeMap`s, zero non-target
    /// payload copies) while preserving the exact validation, resource
    /// ordering, and error diagnostics of [`Iff::parse`].
    pub fn scan_types(bytes: &[u8], tags: &[[u8; 4]]) -> Result<ScannedIff> {
        let file_len = bytes.len();
        if file_len < FILE_HEADER_SIZE {
            return Err(crate::Error::new(
                0,
                format!("need {FILE_HEADER_SIZE} bytes; only {file_len} remain"),
            ));
        }
        let header = &bytes[..FILE_HEADER_SIZE];
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return Err(crate::Error::new(
                FILE_HEADER_SIZE,
                "invalid Maxis IFF signature",
            ));
        }
        let rsmp_offset = if header.starts_with(b"IFF FILE 2.5:") {
            u32::from_be_bytes(header[60..64].try_into().unwrap()) as usize
        } else {
            0
        };

        let mut stubs = Vec::new();
        let mut pos = FILE_HEADER_SIZE;
        while pos < file_len {
            let remaining = file_len - pos;
            if remaining < 4 {
                return Err(crate::Error::new(
                    pos,
                    format!("need 4 bytes; only {remaining} remain"),
                ));
            }
            if remaining < 8 {
                return Err(crate::Error::new(
                    pos + 4,
                    format!("need 4 bytes; only {} remain", remaining - 4),
                ));
            }
            let tag: [u8; 4] = bytes[pos..pos + 4].try_into().unwrap();
            let length = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            if length < CHUNK_HEADER_SIZE {
                return Err(crate::Error::new(
                    pos + 8,
                    "IFF chunk shorter than its 76-byte header",
                ));
            }
            if remaining < 10 {
                return Err(crate::Error::new(
                    pos + 8,
                    format!("need 2 bytes; only {} remain", remaining - 8),
                ));
            }
            if remaining < 12 {
                return Err(crate::Error::new(
                    pos + 10,
                    format!("need 2 bytes; only {} remain", remaining - 10),
                ));
            }
            if remaining < CHUNK_HEADER_SIZE {
                return Err(crate::Error::new(
                    pos + 12,
                    format!("need 64 bytes; only {} remain", remaining - 12),
                ));
            }
            let id = u16::from_be_bytes(bytes[pos + 8..pos + 10].try_into().unwrap());
            let flags = u16::from_be_bytes(bytes[pos + 10..pos + 12].try_into().unwrap());
            let payload_start = pos + CHUNK_HEADER_SIZE;
            let payload_len = length - CHUNK_HEADER_SIZE;
            let payload_end = payload_start
                .checked_add(payload_len)
                .ok_or_else(|| crate::Error::new(payload_start, "size overflow"))?;
            if payload_end > file_len {
                return Err(crate::Error::new(
                    payload_start,
                    format!(
                        "need {payload_len} bytes; only {} remain",
                        file_len - payload_start
                    ),
                ));
            }
            stubs.push(ChunkStub {
                tag,
                id,
                flags,
                offset: pos,
                payload_start,
                payload_end,
            });
            pos = payload_end;
        }

        let mut resources = if rsmp_offset == 0 {
            Resources::physical_stubs(&stubs, tags)
        } else {
            let map_stub = stubs
                .binary_search_by_key(&rsmp_offset, |c| c.offset)
                .ok()
                .map(|i| stubs[i])
                .filter(|c| c.tag == *b"rsmp" && c.flags & DELETED == 0);
            match map_stub {
                None => Resources {
                    map_error: Some(crate::Error::new(rsmp_offset, "IFF resource map not found")),
                    ..Resources::physical_stubs(&stubs, tags)
                },
                Some(stub) => match Resources::mapped_stubs(
                    &bytes[stub.payload_start..stub.payload_end],
                    &stubs,
                    tags,
                ) {
                    Ok(Some(res)) => res,
                    Ok(None) => Resources::physical_stubs(&stubs, tags),
                    Err(error) => Resources {
                        map_error: Some(error),
                        ..Resources::physical_stubs(&stubs, tags)
                    },
                },
            }
        };
        resources.order.remove(b"rsmp");
        resources.by_key.retain(|key, _| key.0 != *b"rsmp");

        let mut chunks = Vec::new();
        let mut order: BTreeMap<[u8; 4], Vec<usize>> = BTreeMap::new();
        for (tag, indices) in resources.order {
            let mut mapped_indices = Vec::with_capacity(indices.len());
            for stub_idx in indices {
                let stub = stubs[stub_idx];
                let out_idx = chunks.len();
                chunks.push(ScannedChunk {
                    tag: stub.tag,
                    id: stub.id,
                    flags: stub.flags,
                    offset: stub.offset,
                    data: bytes[stub.payload_start..stub.payload_end].to_vec(),
                });
                mapped_indices.push(out_idx);
            }
            order.insert(tag, mapped_indices);
        }
        Ok(ScannedIff {
            chunks,
            order,
            map_error: resources.map_error,
        })
    }
    /// Streaming header-only IFF scanner over a random-access byte source
    /// (`read_at(offset, buf)`). Walks chunk headers via a 4 KiB stack sliding
    /// window, skips megabytes of non-target payloads (such as `SPR2`), and
    /// reads only `rsmp` (if present) plus the resolved payloads of `tags`,
    /// preserving exact [`Iff::parse`] validation, resource ordering, and
    /// diagnostics.
    pub fn scan_types_from_reader<E, F>(
        file_len: usize,
        mut read_at: F,
        tags: &[[u8; 4]],
    ) -> std::result::Result<ScannedIff, ScanError<E>>
    where
        F: FnMut(usize, &mut [u8]) -> std::result::Result<(), E>,
    {
        const WINDOW: usize = 4_096;
        let mut window = [0u8; WINDOW];
        let mut win_start = 0usize;
        let mut win_len = 0usize;

        let mut read_exact_windowed =
            |offset: usize, dst: &mut [u8], refill: bool| -> std::result::Result<(), E> {
                if dst.is_empty() {
                    return Ok(());
                }
                if offset >= win_start && offset + dst.len() <= win_start + win_len {
                    let rel = offset - win_start;
                    dst.copy_from_slice(&window[rel..rel + dst.len()]);
                    return Ok(());
                }
                if !refill || dst.len() > WINDOW {
                    return read_at(offset, dst);
                }
                let fill = WINDOW.min(file_len - offset);
                read_at(offset, &mut window[..fill])?;
                win_start = offset;
                win_len = fill;
                dst.copy_from_slice(&window[..dst.len()]);
                Ok(())
            };

        if file_len < FILE_HEADER_SIZE {
            return Err(ScanError::Format(crate::Error::new(
                0,
                format!("need {FILE_HEADER_SIZE} bytes; only {file_len} remain"),
            )));
        }
        let mut header = [0u8; FILE_HEADER_SIZE];
        read_exact_windowed(0, &mut header, true).map_err(ScanError::Io)?;
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return Err(ScanError::Format(crate::Error::new(
                FILE_HEADER_SIZE,
                "invalid Maxis IFF signature",
            )));
        }
        let rsmp_offset = if header.starts_with(b"IFF FILE 2.5:") {
            u32::from_be_bytes(header[60..64].try_into().unwrap()) as usize
        } else {
            0
        };

        let mut stubs = Vec::new();
        let mut pos = FILE_HEADER_SIZE;
        let mut head = [0u8; 12];
        while pos < file_len {
            let remaining = file_len - pos;
            if remaining < 4 {
                return Err(ScanError::Format(crate::Error::new(
                    pos,
                    format!("need 4 bytes; only {remaining} remain"),
                )));
            }
            if remaining < 8 {
                return Err(ScanError::Format(crate::Error::new(
                    pos + 4,
                    format!("need 4 bytes; only {} remain", remaining - 4),
                )));
            }
            let head_len = remaining.min(12);
            read_exact_windowed(pos, &mut head[..head_len], true).map_err(ScanError::Io)?;
            let tag: [u8; 4] = head[0..4].try_into().unwrap();
            let length = u32::from_be_bytes(head[4..8].try_into().unwrap()) as usize;
            if length < CHUNK_HEADER_SIZE {
                return Err(ScanError::Format(crate::Error::new(
                    pos + 8,
                    "IFF chunk shorter than its 76-byte header",
                )));
            }
            if remaining < 10 {
                return Err(ScanError::Format(crate::Error::new(
                    pos + 8,
                    format!("need 2 bytes; only {} remain", remaining - 8),
                )));
            }
            if remaining < 12 {
                return Err(ScanError::Format(crate::Error::new(
                    pos + 10,
                    format!("need 2 bytes; only {} remain", remaining - 10),
                )));
            }
            if remaining < CHUNK_HEADER_SIZE {
                return Err(ScanError::Format(crate::Error::new(
                    pos + 12,
                    format!("need 64 bytes; only {} remain", remaining - 12),
                )));
            }
            let id = u16::from_be_bytes(head[8..10].try_into().unwrap());
            let flags = u16::from_be_bytes(head[10..12].try_into().unwrap());
            let payload_start = pos + CHUNK_HEADER_SIZE;
            let payload_len = length - CHUNK_HEADER_SIZE;
            let payload_end = payload_start.checked_add(payload_len).ok_or_else(|| {
                ScanError::Format(crate::Error::new(payload_start, "size overflow"))
            })?;
            if payload_end > file_len {
                return Err(ScanError::Format(crate::Error::new(
                    payload_start,
                    format!(
                        "need {payload_len} bytes; only {} remain",
                        file_len - payload_start
                    ),
                )));
            }
            stubs.push(ChunkStub {
                tag,
                id,
                flags,
                offset: pos,
                payload_start,
                payload_end,
            });
            pos = payload_end;
        }

        let mut resources = if rsmp_offset == 0 {
            Resources::physical_stubs(&stubs, tags)
        } else {
            let map_stub = stubs
                .binary_search_by_key(&rsmp_offset, |c| c.offset)
                .ok()
                .map(|i| stubs[i])
                .filter(|c| c.tag == *b"rsmp" && c.flags & DELETED == 0);
            match map_stub {
                None => Resources {
                    map_error: Some(crate::Error::new(rsmp_offset, "IFF resource map not found")),
                    ..Resources::physical_stubs(&stubs, tags)
                },
                Some(stub) => {
                    let mut rsmp_buf = vec![0u8; stub.payload_end - stub.payload_start];
                    read_exact_windowed(stub.payload_start, &mut rsmp_buf, false)
                        .map_err(ScanError::Io)?;
                    match Resources::mapped_stubs(&rsmp_buf, &stubs, tags) {
                        Ok(Some(res)) => res,
                        Ok(None) => Resources::physical_stubs(&stubs, tags),
                        Err(error) => Resources {
                            map_error: Some(error),
                            ..Resources::physical_stubs(&stubs, tags)
                        },
                    }
                }
            }
        };
        resources.order.remove(b"rsmp");
        resources.by_key.retain(|key, _| key.0 != *b"rsmp");

        let mut chunks = Vec::new();
        let mut order: BTreeMap<[u8; 4], Vec<usize>> = BTreeMap::new();
        for (tag, indices) in resources.order {
            let mut mapped_indices = Vec::with_capacity(indices.len());
            for stub_idx in indices {
                let stub = stubs[stub_idx];
                let mut data = vec![0u8; stub.payload_end - stub.payload_start];
                read_exact_windowed(stub.payload_start, &mut data, false).map_err(ScanError::Io)?;
                let out_idx = chunks.len();
                chunks.push(ScannedChunk {
                    tag: stub.tag,
                    id: stub.id,
                    flags: stub.flags,
                    offset: stub.offset,
                    data,
                });
                mapped_indices.push(out_idx);
            }
            order.insert(tag, mapped_indices);
        }
        Ok(ScannedIff {
            chunks,
            order,
            map_error: resources.map_error,
        })
    }
    /// The resource of type `tag` with ID `id` (and flags whose high byte
    /// is 0), as the original looks one up (see [`Resources`]).
    pub fn get(&self, tag: &[u8; 4], id: u16) -> Option<&Chunk> {
        self.resources
            .by_key
            .get(&(*tag, id, 0))
            .map(|&i| &self.chunks[i])
    }
    /// Why the file's resource map could not be used, if it could not. The
    /// original reads the map as it is (0x41bfdd to 0x41c032), so the
    /// resources [`Iff::get`] and [`Iff::all`] give are then not the
    /// original's.
    pub fn map_error(&self) -> Option<&crate::Error> {
        self.resources.map_error.as_ref()
    }
    /// The resources of type `tag` in the original's order. When the
    /// resource map cannot be used, the chunks as if there were no map.
    pub fn all<'a>(&'a self, tag: &'a [u8; 4]) -> impl Iterator<Item = &'a Chunk> {
        self.resources
            .order
            .get(tag)
            .into_iter()
            .flatten()
            .map(|&i| &self.chunks[i])
    }
    /// Native IFFResFile3 enumeration. A version-0 resource map supplies its
    /// own per-type order (0x41be90/0x41bbc5); without a map, 0x41c252 appends
    /// live chunks in physical file order. Resource IDs are not sort keys.
    /// Fails when the file names a resource map that cannot be used.
    pub fn resource_order(&self, tag: &[u8; 4]) -> Result<Vec<&Chunk>> {
        match &self.resources.map_error {
            Some(error) => Err(error.clone()),
            None => Ok(self
                .resources
                .order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|&i| &self.chunks[i])
                .collect()),
        }
    }
}

/// Error returned by [`Iff::scan_types_from_reader`], distinguishing I/O
/// callback failures from IFF format errors.
#[derive(Debug)]
pub enum ScanError<E> {
    Io(E),
    Format(crate::Error),
}

/// One resolved chunk payload extracted by [`Iff::scan_types`] or
/// [`Iff::scan_types_from_reader`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedChunk {
    pub tag: [u8; 4],
    pub id: u16,
    pub flags: u16,
    pub offset: usize,
    pub data: Vec<u8>,
}

/// Subset of an IFF file's resolved resources returned by [`Iff::scan_types`]
/// and [`Iff::scan_types_from_reader`].
#[derive(Debug, Clone)]
pub struct ScannedIff {
    chunks: Vec<ScannedChunk>,
    order: BTreeMap<[u8; 4], Vec<usize>>,
    map_error: Option<crate::Error>,
}

impl ScannedIff {
    pub fn map_error(&self) -> Option<&crate::Error> {
        self.map_error.as_ref()
    }

    pub fn all<'a>(&'a self, tag: &'a [u8; 4]) -> impl Iterator<Item = &'a ScannedChunk> {
        self.order
            .get(tag)
            .into_iter()
            .flatten()
            .map(|&i| &self.chunks[i])
    }

    pub fn resource_order(&self, tag: &[u8; 4]) -> Result<Vec<&ScannedChunk>> {
        match &self.map_error {
            Some(error) => Err(error.clone()),
            None => Ok(self
                .order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|&i| &self.chunks[i])
                .collect()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResourceMapEntry {
    pub offset: u32,
    pub id: u16,
    pub flags: u16,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMap {
    pub unused_bytes: u32,
    pub types: std::collections::BTreeMap<[u8; 4], Vec<ResourceMapEntry>>,
}
impl ResourceMap {
    /// Raw version-0 rsmp stream, 0x41be90 and 0x41b7cb. String alignment is
    /// relative to the enclosing payload, including its twelve-byte header.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.take(8)? != [0; 8] || r.take(4)? != b"pmsr" {
            return r.fail("unsupported resource-map stream header");
        }
        let unused_bytes = r.u32()?;
        let count = r.u32()? as usize;
        r.count(count, 8)?;
        let mut types = std::collections::BTreeMap::new();
        for _ in 0..count {
            let tag = r.u32()?.to_be_bytes();
            let n = r.u32()? as usize;
            r.count(n, 10)?;
            let mut entries = vec![];
            for _ in 0..n {
                let offset = r.u32()?;
                let id = r.u16()?;
                let flags = r.u16()?;
                let label = western(r.cstr()?);
                if r.pos & 1 != 0 {
                    r.skip(1)?;
                }
                entries.push(ResourceMapEntry {
                    offset,
                    id,
                    flags,
                    label,
                });
            }
            if types.insert(tag, entries).is_some() {
                return r.fail("duplicate resource-map type");
            }
        }
        if r.remaining() != 0 {
            return r.fail("unconsumed resource-map bytes");
        }
        Ok(Self {
            unused_bytes,
            types,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_iff_round_trips_headers_payloads_and_resource_order() {
        let chunks = [
            ChunkData {
                tag: *b"BMP_",
                id: 0x1234,
                flags: 0x0210,
                label: "Café".into(),
                data: vec![0, 1, 2],
            },
            ChunkData {
                tag: *b"BMP_",
                id: 0x1234,
                flags: 0,
                label: "X".repeat(64),
                data: Vec::new(),
            },
            ChunkData {
                tag: *b"XXXX",
                id: 17,
                flags: DELETED,
                label: String::new(),
                data: vec![255],
            },
        ];
        let bytes = write(&chunks).unwrap();
        assert_eq!(&bytes[..60], SIGNATURE_25);
        assert_eq!(&bytes[60..64], &[0; 4]);
        let iff = Iff::parse(bytes).unwrap();
        assert_eq!(iff.chunks.len(), chunks.len());
        for (actual, expected) in iff.chunks.iter().zip(&chunks) {
            assert_eq!(actual.tag, expected.tag);
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.flags, expected.flags);
            assert_eq!(actual.label, expected.label);
            assert_eq!(iff.data(actual), expected.data);
        }
        let bitmaps = iff.resource_order(b"BMP_").unwrap();
        assert_eq!(bitmaps[0].flags, 0x0210);
        assert_eq!(bitmaps[1].flags, 0);
        assert_eq!(iff.get(b"BMP_", 0x1234).unwrap().flags, 0);
        assert!(iff.resource_order(b"XXXX").unwrap().is_empty());

        // Verify zero-alloc label helpers & streaming scanner parity.
        for (actual, expected) in iff.chunks.iter().zip(&chunks) {
            assert_eq!(iff.chunk_label(actual).as_ref(), expected.label);
        }
        let scanned = Iff::scan_types(&iff.bytes, &[*b"BMP_"]).unwrap();
        let scanned_bmps = scanned.resource_order(b"BMP_").unwrap();
        assert_eq!(scanned_bmps.len(), bitmaps.len());
        for (a, b) in scanned_bmps.iter().zip(&bitmaps) {
            assert_eq!(a.tag, b.tag);
            assert_eq!(a.id, b.id);
            assert_eq!(a.flags, b.flags);
            assert_eq!(a.offset, b.offset);
            assert_eq!(a.data.as_slice(), iff.data(b));
        }
    }

    #[test]
    fn scan_types_from_reader_skips_non_target_payloads_and_matches_full_parse() {
        let big_spr2 = vec![0xABu8; 32_768];
        let chunks = [
            ChunkData {
                tag: *b"SPR2",
                id: 100,
                flags: 0,
                label: "big sprite".into(),
                data: big_spr2,
            },
            ChunkData {
                tag: *b"OBJD",
                id: 1,
                flags: 0,
                label: "chair".into(),
                data: vec![1, 2, 3, 4],
            },
            ChunkData {
                tag: *b"SPR2",
                id: 101,
                flags: 0,
                label: "big sprite 2".into(),
                data: vec![0xCDu8; 32_768],
            },
            ChunkData {
                tag: *b"GLOB",
                id: 256,
                flags: 0,
                label: "semi".into(),
                data: b"ChairGlobals".to_vec(),
            },
        ];
        let bytes = write(&chunks).unwrap();
        let full = Iff::parse(bytes.clone()).unwrap();
        let mut bytes_read = 0usize;
        let scanned = Iff::scan_types_from_reader(
            bytes.len(),
            |offset, dst| {
                bytes_read += dst.len();
                dst.copy_from_slice(&bytes[offset..offset + dst.len()]);
                Ok::<(), std::convert::Infallible>(())
            },
            &[*b"OBJD", *b"GLOB"],
        )
        .unwrap();
        assert!(
            bytes_read < bytes.len() / 2,
            "streaming scanner must skip large SPR2 payloads: read {bytes_read} of {} bytes",
            bytes.len()
        );
        let objds = scanned.resource_order(b"OBJD").unwrap();
        let full_objds = full.resource_order(b"OBJD").unwrap();
        assert_eq!(objds.len(), full_objds.len());
        assert_eq!(objds[0].data.as_slice(), full.data(full_objds[0]));
        let globs: Vec<_> = scanned.all(b"GLOB").collect();
        let full_globs: Vec<_> = full.all(b"GLOB").collect();
        assert_eq!(globs.len(), full_globs.len());
        assert_eq!(globs[0].data.as_slice(), full.data(full_globs[0]));
    }
}
