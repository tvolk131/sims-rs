//! Adds or replacements on an IFFResFile2, followed by close: one on an
//! existing file, or a sequence into a newly created one.

use crate::iff::{DELETED, Iff};
use crate::reader::Reader;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::ops::Range;

const FILE_HEADER: usize = 64;
const CHUNK_HEADER: usize = 76;
const SIGNATURE: &[u8; 60] = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1";
const MAP: [u8; 4] = *b"rsmp";
const FREE: [u8; 4] = *b"XXXX";
const CONVERTED: u16 = 0x10;

#[derive(Debug, Clone, Copy)]
pub struct ChunkWrite<'a> {
    pub tag: [u8; 4],
    pub id: u16,
    /// The variant in the high byte and the converted bit (0x10).
    pub flags: u16,
    /// Original encoded bytes of the C string. The native buffer holds 63
    /// bytes plus the terminator.
    pub label: &'a [u8],
    pub payload: &'a [u8],
}

/// Adds or replaces one resource and saves the file as on IFFResFile2 close.
/// Missing native stack or heap bytes are reported with the write address.
pub fn write_chunk(bytes: &[u8], chunk: ChunkWrite<'_>) -> Result<Vec<u8>> {
    write_chunk_with_residue(bytes, chunk, |_| None)
}

/// Replays the same write with captured bytes for positions the original
/// leaves uninitialized. The callback supplies exactly the requested range
/// at its file offset when written. A later write may reuse that location.
pub fn write_chunk_with_residue<'a>(
    bytes: &[u8],
    chunk: ChunkWrite<'_>,
    mut residue: impl FnMut(Range<usize>) -> Option<&'a [u8]>,
) -> Result<Vec<u8>> {
    let mut file = OpenFile::parse(bytes)?;
    file.add(chunk, &mut residue)?;
    file.finish(&mut residue)
}

/// Writes a new file the way a save into a freshly created IFFResFile2
/// does (0x465949 creates it, each resource is added in turn, and the close
/// at 0x46b6d2 writes the resource map). The file starts empty, so every
/// resource lands at the end in the order given, and the map follows them.
/// `residue` supplies the bytes the original leaves uninitialized, as for
/// [`write_chunk_with_residue`]; a fresh file's only ones are the map's
/// alignment bytes.
///
/// The map lists the types in `type_order` first, in that order, then any
/// others in the order they were first added. The original's order for a
/// new file is not insertion order (see
/// [`crate::neighborhood_file`]), and its rule is untraced, so a caller
/// that knows the order passes it.
pub fn write_new_file<'a>(
    chunks: &[ChunkWrite<'_>],
    type_order: &[[u8; 4]],
    mut residue: impl FnMut(Range<usize>) -> Option<&'a [u8]>,
) -> Result<Vec<u8>> {
    let mut bytes = SIGNATURE.to_vec();
    bytes.resize(FILE_HEADER, 0);
    let mut file = OpenFile {
        bytes,
        types: Vec::new(),
        map_offset: 0,
        cursor: FILE_HEADER,
        unused: 0,
    };
    let mut keys = std::collections::BTreeSet::new();
    for &chunk in chunks {
        // A second write of a key would replace the first and leave a
        // hole, whose bytes are not the map's alignment bytes.
        if !keys.insert((chunk.tag, chunk.id)) {
            return Err(Error::new(
                0,
                format!(
                    "{} #{} written twice to a new file",
                    String::from_utf8_lossy(&chunk.tag),
                    chunk.id
                ),
            ));
        }
        file.add(chunk, &mut residue)?;
    }
    file.types.sort_by_key(|group| {
        type_order
            .iter()
            .position(|&tag| tag == group.tag)
            .unwrap_or(type_order.len())
    });
    file.finish(&mut residue)
}

struct Entry {
    offset: usize,
    id: u16,
    flags: u16,
    label: Vec<u8>,
}

struct ResourceType {
    tag: [u8; 4],
    entries: Vec<Entry>,
}

struct OpenFile {
    bytes: Vec<u8>,
    types: Vec<ResourceType>,
    map_offset: usize,
    cursor: usize,
    unused: u32,
}

impl OpenFile {
    /// One add or replacement (0x419ed8), without the close.
    fn add<'a>(
        &mut self,
        chunk: ChunkWrite<'_>,
        residue: &mut impl FnMut(Range<usize>) -> Option<&'a [u8]>,
    ) -> Result<()> {
        if chunk.tag == MAP || chunk.flags & !(0xff00 | CONVERTED) != 0 {
            return Err(Error::new(
                0,
                "unsupported resource tag or flags at 0x419ed8",
            ));
        }
        let label_end = chunk.label.iter().take(63).position(|&byte| byte == 0);
        let label = &chunk.label[..label_end.unwrap_or(chunk.label.len().min(63))];

        // 0x419ed8 searches the requested variant, then variant zero. It
        // discards even a same-size old resource and keeps its slot in the table.
        let group = self.types.iter().position(|group| group.tag == chunk.tag);
        let slot = group.and_then(|group| {
            let entries = &self.types[group].entries;
            let find = |variant| {
                entries
                    .iter()
                    .position(|entry| entry.id == chunk.id && entry.flags >> 8 == variant)
            };
            find(chunk.flags >> 8).or_else(|| find(0))
        });
        if let (Some(group), Some(slot)) = (group, slot) {
            self.erase(self.types[group].entries[slot].offset)?;
        }
        if self.map_offset != 0 {
            self.erase(self.map_offset)?;
            self.map_offset = 0;
        }
        let offset = self.reserve(chunk.payload.len(), residue)?;
        self.write(offset, ChunkWrite { label, ..chunk });

        let entry = Entry {
            offset,
            id: chunk.id,
            flags: chunk.flags,
            label: label.to_vec(),
        };
        match (group, slot) {
            (Some(group), Some(slot)) => self.types[group].entries[slot] = entry,
            (Some(group), None) => self.types[group].entries.push(entry),
            _ => self.types.push(ResourceType {
                tag: chunk.tag,
                entries: vec![entry],
            }),
        }
        Ok(())
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let iff = Iff::parse(bytes.to_vec())?;
        let mut signature: [u8; 60] = iff.bytes[..60].try_into().unwrap();
        let version = signature[11];
        signature[11] = b'5';
        if &signature != SIGNATURE || iff.bytes.len() > i32::MAX as usize {
            return Err(Error::new(
                0,
                "unsupported IFF header or length at 0x4191db",
            ));
        }
        let map_offset = if version == b'5' {
            u32::from_be_bytes(iff.bytes[60..64].try_into().unwrap()) as usize
        } else {
            0
        };
        let mut types = Vec::new();
        let mut unused = 0;
        let mut scan = map_offset == 0;
        if map_offset != 0 {
            let map = iff
                .chunks
                .iter()
                .find(|chunk| {
                    chunk.offset == map_offset && chunk.tag == MAP && chunk.flags & DELETED == 0
                })
                .ok_or_else(|| Error::new(map_offset, "missing live rsmp at 0x419330"))?;
            (types, unused) = read_map(iff.data(map)).map_err(|error| {
                Error::new(
                    map.payload.start + error.offset,
                    format!("rsmp at 0x41891d: {}", error.message),
                )
            })?;
            // A map listing rsmp #0 itself triggers the physical scan.
            scan = types
                .iter()
                .find(|group| group.tag == MAP)
                .is_some_and(|group| {
                    group
                        .entries
                        .iter()
                        .any(|entry| entry.id == 0 && entry.flags >> 8 == 0)
                });
            if scan {
                types.clear();
            }
        }
        if scan {
            for chunk in &iff.chunks {
                let size = chunk.payload.end - chunk.offset;
                if size > 0x4000_0000 {
                    return Err(Error::new(chunk.offset, "oversized chunk at 0x41948b"));
                }
                if chunk.flags & DELETED != 0 {
                    unused = unused.wrapping_add(size as u32);
                    continue;
                }
                let raw = &iff.bytes[chunk.offset + 12..chunk.payload.start];
                let end = raw.iter().position(|&byte| byte == 0).ok_or_else(|| {
                    Error::new(chunk.offset + 12, "unterminated disk label at 0x4194fe")
                })?;
                let entry = Entry {
                    offset: chunk.offset,
                    id: chunk.id,
                    flags: chunk.flags,
                    label: raw[..end].to_vec(),
                };
                if let Some(group) = types.iter_mut().find(|group| group.tag == chunk.tag) {
                    group.entries.push(entry);
                } else {
                    types.push(ResourceType {
                        tag: chunk.tag,
                        entries: vec![entry],
                    });
                }
            }
        }
        remove_map_type(&mut types);
        Ok(Self {
            bytes: iff.bytes,
            types,
            map_offset,
            cursor: FILE_HEADER,
            unused,
        })
    }

    fn erase(&mut self, offset: usize) -> Result<()> {
        // 0x41a3ac reads the header, changes the tag and sets the deleted
        // bit. Its size, ID, label and payload are left in the file.
        let (size, flags) = self.chunk_fields(offset)?;
        self.bytes[offset..offset + 4].copy_from_slice(&FREE);
        self.bytes[offset + 10..offset + 12].copy_from_slice(&(flags | DELETED).to_be_bytes());
        self.unused = self.unused.wrapping_add(size as u32);
        Ok(())
    }

    fn chunk_fields(&self, offset: usize) -> Result<(usize, u16)> {
        let header = self
            .bytes
            .get(offset..)
            .and_then(|remaining| remaining.get(..CHUNK_HEADER))
            .ok_or_else(|| Error::new(offset, "missing chunk header at 0x41d618"))?;
        let size = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
        if size < CHUNK_HEADER || size > self.bytes.len() - offset {
            return Err(Error::new(offset, "invalid chunk size at 0x41d618"));
        }
        let flags = u16::from_be_bytes(header[10..12].try_into().unwrap());
        Ok((size, flags))
    }

    fn reserve<'a>(
        &mut self,
        payload_len: usize,
        residue: &mut impl FnMut(Range<usize>) -> Option<&'a [u8]>,
    ) -> Result<usize> {
        let mut offset = self.cursor;
        let length = payload_len
            .checked_add(CHUNK_HEADER)
            .filter(|&length| length <= i32::MAX as usize)
            .ok_or_else(|| Error::new(offset, "chunk length overflow at 0x41a427"))?;
        let mut lookahead = None;
        while offset < self.bytes.len() {
            let (size, flags) = self.chunk_fields(offset)?;
            let mut end = offset + size;
            if flags & DELETED == 0 {
                offset = end;
                continue;
            }

            // 0x41a3ec merges consecutive deleted headers as it scans.
            // The last successful lookahead stays in scratch on a failed EOF read.
            while end < self.bytes.len() {
                let (next_size, next_flags) = self.chunk_fields(end)?;
                lookahead = Some(end);
                if next_flags & DELETED == 0 {
                    break;
                }
                end += next_size;
            }
            let available = end - offset;
            if available != size {
                self.set_size(offset, available);
            }
            if available == length || available >= length + CHUNK_HEADER {
                if available != length {
                    let split = offset + length;
                    if let Some(next) = lookahead {
                        self.bytes.copy_within(next..next + CHUNK_HEADER, split);
                    } else {
                        for range in [split + 8..split + 10, split + 12..split + CHUNK_HEADER] {
                            fill_residue(
                                &mut self.bytes,
                                range,
                                residue,
                                "EOF split header at 0x41a4da",
                            )?;
                        }
                    }
                    self.bytes[split..split + 4].copy_from_slice(&FREE);
                    self.set_size(split, available - length);
                    self.bytes[split + 10..split + 12].copy_from_slice(&DELETED.to_be_bytes());
                }
                self.unused = self.unused.wrapping_sub(length as u32);
                self.cursor = offset;
                return Ok(offset);
            }
            offset = end;
        }

        // EOF gets a fresh active chunk after the reservation at 0x41a52c.
        let end = offset
            .checked_add(length)
            .filter(|&end| end <= i32::MAX as usize)
            .ok_or_else(|| Error::new(offset, "IFF length overflow at 0x41a52c"))?;
        self.bytes.resize(end, 0);
        self.cursor = offset;
        Ok(offset)
    }

    fn set_size(&mut self, offset: usize, size: usize) {
        self.bytes[offset + 4..offset + 8].copy_from_slice(&(size as u32).to_be_bytes());
    }

    fn write(&mut self, offset: usize, chunk: ChunkWrite<'_>) {
        self.bytes[offset..offset + 4].copy_from_slice(&chunk.tag);
        self.set_size(offset, CHUNK_HEADER + chunk.payload.len());
        self.bytes[offset + 8..offset + 10].copy_from_slice(&chunk.id.to_be_bytes());
        self.bytes[offset + 10..offset + 12].copy_from_slice(&chunk.flags.to_be_bytes());
        let label = &mut self.bytes[offset + 12..offset + CHUNK_HEADER];
        label.fill(0);
        label[..chunk.label.len()].copy_from_slice(chunk.label);
        let start = offset + CHUNK_HEADER;
        self.bytes[start..start + chunk.payload.len()].copy_from_slice(chunk.payload);
    }

    fn compact(&mut self) -> Result<()> {
        let mut offsets = BTreeMap::new();
        let mut source = FILE_HEADER;
        let mut destination = FILE_HEADER;
        while source < self.bytes.len() {
            let (size, flags) = self.chunk_fields(source)?;
            if flags & DELETED == 0 && self.bytes[source..source + 4] != MAP {
                self.bytes.copy_within(source..source + size, destination);
                offsets.insert(source, destination);
                destination += size;
            }
            source += size;
        }
        for entry in self.types.iter_mut().flat_map(|group| &mut group.entries) {
            if let Some(offset) = offsets.remove(&entry.offset) {
                entry.offset = offset;
            }
        }
        self.bytes.truncate(destination);
        self.cursor = FILE_HEADER;
        self.unused = 0;
        Ok(())
    }

    fn finish<'a>(
        mut self,
        residue: &mut impl FnMut(Range<usize>) -> Option<&'a [u8]>,
    ) -> Result<Vec<u8>> {
        // 0x4196c2 compacts once the signed unused counter reaches 10 KiB.
        if self.unused as i32 >= 0x2800 {
            self.compact()?;
        }
        remove_map_type(&mut self.types);

        // 0x41ac25 writes the two vectors in their current order. The
        // unused count is sampled before the map's own allocation.
        let mut map = Vec::from(*b"\0\0\0\0\0\0\0\0pmsr");
        map.extend(self.unused.to_le_bytes());
        map.extend((self.types.len() as u32).to_le_bytes());
        let mut padding = Vec::new();
        for group in &self.types {
            map.extend(u32::from_be_bytes(group.tag).to_le_bytes());
            map.extend((group.entries.len() as u32).to_le_bytes());
            for entry in &group.entries {
                map.extend((entry.offset as u32).to_le_bytes());
                map.extend(entry.id.to_le_bytes());
                map.extend(entry.flags.to_le_bytes());
                map.extend(&entry.label);
                map.push(0);
                if map.len() & 1 != 0 {
                    padding.push(map.len());
                    map.push(0);
                }
            }
        }
        let offset = self.reserve(map.len(), residue)?;
        self.write(
            offset,
            ChunkWrite {
                tag: MAP,
                id: 0,
                flags: CONVERTED,
                label: b"",
                payload: &map,
            },
        );
        for pad in padding {
            let at = offset + CHUNK_HEADER + pad;
            fill_residue(
                &mut self.bytes,
                at..at + 1,
                residue,
                "rsmp alignment byte at 0x464cd7",
            )?;
        }
        self.bytes[..60].copy_from_slice(SIGNATURE);
        self.bytes[60..FILE_HEADER].copy_from_slice(&(offset as u32).to_be_bytes());
        Ok(self.bytes)
    }
}

fn read_map(data: &[u8]) -> Result<(Vec<ResourceType>, u32)> {
    let mut reader = Reader::new(data);
    if reader.take(12)? != b"\0\0\0\0\0\0\0\0pmsr" {
        return reader.fail("unsupported resource-map stream header");
    }
    let unused = reader.u32()?;
    let count = reader.u32()? as usize;
    reader.count(count, 8)?;
    let mut types = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = reader.u32()?.to_be_bytes();
        let count = reader.u32()? as usize;
        reader.count(count, 10)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = reader.u32()? as usize;
            let id = reader.u16()?;
            let flags = reader.u16()?;
            let label = reader.cstr()?;
            if label.len() > 63 {
                return reader.fail("resource-map label exceeds the native buffer");
            }
            if reader.pos & 1 != 0 {
                reader.skip(1)?;
            }
            entries.push(Entry {
                offset,
                id,
                flags,
                label: label.to_vec(),
            });
        }
        types.push(ResourceType { tag, entries });
    }
    if reader.remaining() != 0 {
        return reader.fail("unconsumed resource-map bytes");
    }
    Ok((types, unused))
}

fn remove_map_type(types: &mut Vec<ResourceType>) {
    if let Some(index) = types.iter().position(|group| group.tag == MAP) {
        types.remove(index);
    }
}

fn fill_residue<'a>(
    bytes: &mut [u8],
    range: Range<usize>,
    residue: &mut impl FnMut(Range<usize>) -> Option<&'a [u8]>,
    source: &str,
) -> Result<()> {
    let data = residue(range.clone())
        .filter(|data| data.len() == range.len())
        .ok_or_else(|| Error::new(range.start, format!("missing native bytes for {source}")))?;
    bytes[range].copy_from_slice(data);
    Ok(())
}
