//! Exported households and their lot resources.

use crate::iff::{Chunk, DELETED, Iff, ResourceMap};
use crate::lot::header;
use crate::neighborhood::{Family, Neighbors};
use crate::reader::Reader;
use crate::strings::StringTable;
use crate::{Error, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct FamilyFile {
    pub iff: Iff,
    pub family: Family,
    /// The family name is the first string in `FAMs` for the chosen language.
    pub name: StringTable,
    /// Optional `FAMh` payload. The original reads a compressed record vector
    /// (0x4109a9, 0x4113d8, 0x41180a). The meanings of its fields are unverified.
    pub history: Option<Vec<u8>>,
    pub export_info: ExportInfo,
    pub neighbors: Neighbors,
    /// Character resources keyed by their `uChr` ID, using the IFF lookup's default variant.
    pub members: BTreeMap<u16, FamilyMember>,
}

#[derive(Debug, Clone)]
pub struct ExportInfo {
    pub version: u32,
    pub family_id: u16,
    /// Exported neighbor IDs in `EXPi` order. The importer stops at the first zero.
    pub member_ids: [u16; 8],
    /// The export-time language byte added in version 0x3f (0x4243bc, 0x428a90).
    pub language: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct FamilyMember {
    /// The `uChr` body string table. The writer at 0x46fc7a allocates one
    /// extra byte per entry and leaves that space after the table. The
    /// reader at 0x46ffcc consumes only the declared entries. The full
    /// payload remains available in the IFF.
    pub body_strings: StringTable,
    /// Same-ID `CTSS` for the member's name and description.
    pub ctss: Option<Vec<u8>>,
    /// The `CTSS` at ID + 2000, tried before the same-ID table by the
    /// original importer (0x42945d, 0x429516).
    pub localized_ctss: Option<Vec<u8>>,
    pub portrait: Option<Vec<u8>>,
}

impl FamilyFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let iff = Iff::parse(bytes.to_vec()).map_err(|error| container_error(bytes, error))?;
        let characters = iff
            .resource_order(b"uChr")
            .map_err(|error| resource_map_error(&iff, error))?;
        let export_info = parse_chunk(&iff, required(&iff, b"EXPi", 0)?, ExportInfo::parse)?;
        let id = export_info.family_id;
        let family = parse_chunk(&iff, required(&iff, b"FAMI", id)?, Family::parse)?;
        let name = parse_chunk(&iff, required(&iff, b"FAMs", id)?, StringTable::parse)?;
        let history = payload(&iff, b"FAMh", id);
        let neighbors = parse_chunk(&iff, required(&iff, b"NBRS", 1)?, Neighbors::parse)?;

        let mut members = BTreeMap::new();
        for chunk in characters.into_iter().filter(|chunk| chunk.flags >> 8 == 0) {
            let id = chunk.id;
            let body_strings = parse_chunk(&iff, chunk, |data| {
                // The native writer always stores a version and a count.
                Reader::new(data).take(4)?;
                StringTable::parse(data)
            })?;
            members.insert(
                id,
                FamilyMember {
                    body_strings,
                    ctss: payload(&iff, b"CTSS", id),
                    localized_ctss: payload(&iff, b"CTSS", id.wrapping_add(2000)),
                    portrait: payload(&iff, b"BMP_", id),
                },
            );
        }

        Ok(Self {
            iff,
            family,
            name,
            history,
            export_info,
            neighbors,
            members,
        })
    }

    /// Finds the exported chunk ID for a `FAMI` member's GUID.
    ///
    /// Export walks the family GUID vector at 0x428abc, finds the first
    /// neighborhood record for each GUID at 0x42571f, and writes its ID
    /// into `EXPi` at 0x428ad2. Import reads those eight words into
    /// `[ebp-0x9c]` (0x428f86, 0x4243bc), uses each word to look up
    /// `uChr` (0x429271–0x4292bb), then uses it as a one-based index in
    /// the exported `NBRS` vector (0x42965d–0x429695). The source record's
    /// GUID is read at 0x42973b. The 0x42886e routine reads lot coordinates
    /// for its callers at 0x427fd9, 0x427fed, and 0x428001.
    ///
    /// Returns `None` for a GUID without a corresponding exported member.
    pub fn member_chunk_id(&self, guid: u32) -> Option<u16> {
        if !self.family.members.contains(&guid) {
            return None;
        }
        self.export_info
            .member_ids
            .into_iter()
            .take_while(|&id| id != 0)
            .filter(|&id| id <= i16::MAX as u16)
            .find(|&id| {
                self.members.contains_key(&id)
                    && self
                        .neighbors
                        .entries
                        .get(usize::from(id) - 1)
                        .and_then(Option::as_ref)
                        .is_some_and(|neighbor| neighbor.guid == guid)
            })
    }
}

impl ExportInfo {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let version = header(&mut reader, b"iPXE")?;
        let family_id = reader.u16()?;
        let mut member_ids = [0; 8];
        for id in &mut member_ids {
            *id = reader.u16()?;
        }
        let language = (version as i32 >= 0x3f).then(|| reader.u8()).transpose()?;
        Ok(Self {
            version,
            family_id,
            member_ids,
            language,
        })
    }
}

fn required<'a>(iff: &'a Iff, tag: &[u8; 4], id: u16) -> Result<&'a Chunk> {
    iff.get(tag, id).ok_or_else(|| {
        Error::new(
            iff.bytes.len(),
            format!("missing {} chunk {id}", String::from_utf8_lossy(tag)),
        )
    })
}

fn payload(iff: &Iff, tag: &[u8; 4], id: u16) -> Option<Vec<u8>> {
    iff.get(tag, id).map(|chunk| iff.data(chunk).to_vec())
}

fn parse_chunk<T>(iff: &Iff, chunk: &Chunk, parse: impl FnOnce(&[u8]) -> Result<T>) -> Result<T> {
    parse(iff.data(chunk)).map_err(|error| {
        Error::new(
            chunk.payload.start + error.offset,
            format!("{} chunk {}: {}", chunk.kind(), chunk.id, error.message),
        )
    })
}

fn resource_map_error(iff: &Iff, error: Error) -> Error {
    let offset = Reader::new(&iff.bytes[60..]).be32().ok();
    // Iff reports map decoding errors relative to its payload and bad
    // resource references relative to the file. Check which happened.
    let offset = iff
        .chunks
        .iter()
        .find(|chunk| {
            offset.is_some_and(|offset| chunk.offset == offset as usize)
                && chunk.tag == *b"rsmp"
                && chunk.flags & DELETED == 0
        })
        .filter(|chunk| ResourceMap::parse(iff.data(chunk)).is_err())
        .map_or(error.offset, |chunk| chunk.payload.start + error.offset);
    Error::new(offset, format!("rsmp chunk: {}", error.message))
}

fn container_error(bytes: &[u8], error: Error) -> Error {
    // Recover the tag for an IFF framing error. The IFF reader supplies
    // the failing offset, so only the preceding tag and size words matter.
    let mut start = 64;
    while start <= error.offset {
        let Some(chunk) = bytes.get(start..) else {
            break;
        };
        let mut reader = Reader::new(chunk);
        let Ok(tag) = reader.take(4) else { break };
        let next = reader
            .be32()
            .ok()
            .filter(|&length| length >= 76)
            .and_then(|length| start.checked_add(length as usize));
        match next {
            Some(next) if next <= error.offset => start = next,
            _ => {
                return Error::new(
                    error.offset,
                    format!(
                        "{} IFF chunk: {}",
                        String::from_utf8_lossy(tag),
                        error.message
                    ),
                );
            }
        }
    }
    Error::new(error.offset, format!("IFF container: {}", error.message))
}
