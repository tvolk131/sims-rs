use crate::Result;
use crate::reader::{Reader, western};
use std::ops::Range;

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
    pub chunks: Vec<Chunk>,
}
impl Iff {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let mut r = Reader::new(&bytes);
        let header = r.take(64)?;
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return r.fail("invalid Maxis IFF signature");
        }
        let mut chunks = Vec::new();
        while r.remaining() > 0 {
            let offset = r.pos;
            let tag = r.take(4)?.try_into().unwrap();
            let length = r.be32()? as usize;
            if length < 76 {
                return r.fail("IFF chunk shorter than its 76-byte header");
            }
            let id = r.be16()?;
            let flags = r.be16()?;
            let label_bytes = r.take(64)?;
            let end = label_bytes.iter().position(|b| *b == 0).unwrap_or(64);
            let label = western(&label_bytes[..end]);
            let start = r.pos;
            r.skip(length - 76)?;
            chunks.push(Chunk {
                tag,
                id,
                flags,
                label,
                offset,
                payload: start..r.pos,
            });
        }
        Ok(Self { bytes, chunks })
    }
    pub fn data(&self, chunk: &Chunk) -> &[u8] {
        &self.bytes[chunk.payload.clone()]
    }
    pub fn get(&self, tag: &[u8; 4], id: u16) -> Option<&Chunk> {
        self.chunks.iter().find(|c| &c.tag == tag && c.id == id)
    }
    pub fn all<'a>(&'a self, tag: &'a [u8; 4]) -> impl Iterator<Item = &'a Chunk> {
        self.chunks.iter().filter(move |c| &c.tag == tag)
    }
    /// Native IFFResFile3 enumeration. A version-0 resource map supplies its
    /// own per-type order (0x41be90/0x41bbc5); without a map, 0x41c252 appends
    /// live chunks in physical file order. Resource IDs are not sort keys.
    pub fn resource_order(&self, tag: &[u8; 4]) -> Result<Vec<&Chunk>> {
        let offset = u32::from_be_bytes(self.bytes[60..64].try_into().unwrap()) as usize;
        if offset == 0 {
            return Ok(self
                .chunks
                .iter()
                .filter(|c| c.tag == *tag && c.flags & 4 == 0)
                .collect());
        }
        let map = self
            .chunks
            .iter()
            .find(|c| c.offset == offset && c.tag == *b"rsmp" && c.flags & 4 == 0)
            .ok_or_else(|| crate::Error::new(offset, "IFF resource map not found"))?;
        let map = ResourceMap::parse(self.data(map))?;
        let mut result = vec![];
        for entry in map.types.get(tag).into_iter().flatten() {
            let chunk = self
                .chunks
                .iter()
                .find(|c| {
                    c.offset == entry.offset as usize
                        && c.tag == *tag
                        && c.id == entry.id
                        && c.flags == entry.flags
                })
                .ok_or_else(|| {
                    crate::Error::new(
                        entry.offset as usize,
                        "resource map entry disagrees with chunk header",
                    )
                })?;
            result.push(chunk);
        }
        Ok(result)
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
