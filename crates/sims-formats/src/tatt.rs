//! Neighborhood TATT: GUID-keyed resource-global words (native 0x43e0f0).
use crate::field::{FieldWriter, Fields};
use crate::{Error, Result, reader::Reader};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub guid: u32,
    pub values: Vec<i16>,
    /// Native may return at the count if the destination is too small.
    pub count_end: usize,
    pub end: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tatt {
    pub version: u32,
    pub packed: bool,
    pub entries: Vec<Entry>,
    pub count_end: usize,
    pub consumed: usize,
}
enum Stream<'a> {
    Packed(Fields<'a>),
    Raw(Reader<'a>),
}
impl Stream<'_> {
    fn int(&mut self) -> Result<i32> {
        match self {
            Self::Packed(f) => f.i32(),
            Self::Raw(r) => r.i32(),
        }
    }
    fn short(&mut self) -> Result<i16> {
        match self {
            Self::Packed(f) => f.i16(),
            Self::Raw(r) => r.i16(),
        }
    }
    fn position(&self) -> usize {
        match self {
            Self::Packed(f) => 13 + f.byte_position(),
            Self::Raw(r) => r.pos,
        }
    }
}
impl Tatt {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.u32()? != 0 {
            return r.fail("unsupported TATT outer stream mode");
        }
        let version = r.u32()?;
        if r.take(4)? != b"TTAT" {
            return r.fail("invalid TATT signature");
        }
        let packed = r.u8()? != 0;
        let mut stream = if packed {
            Stream::Packed(Fields::new(&bytes[13..], 13))
        } else {
            Stream::Raw(r)
        };
        let count = stream.int()?;
        if count < 0 || count as usize > bytes.len() * 4 {
            return Err(Error::new(stream.position(), "invalid TATT record count"));
        }
        let count_end = stream.position();
        let mut entries = Vec::new();
        for _ in 0..count {
            let guid = stream.int()? as u32;
            let count = stream.int()?;
            if count < 0 || count as usize > bytes.len() * 8 {
                return Err(Error::new(stream.position(), "invalid TATT word count"));
            }
            let count_end = stream.position();
            let values = (0..count)
                .map(|_| stream.short())
                .collect::<Result<Vec<_>>>()?;
            entries.push(Entry {
                guid,
                values,
                count_end,
                end: stream.position(),
            });
        }
        Ok(Self {
            version,
            packed,
            entries,
            count_end,
            consumed: stream.position(),
        })
    }

    /// TATT #1 as the neighborhood save writes it (0x43e0be → 0x43f114,
    /// version 0, serializer 0x43e0f0): the packed flag (always 1, set at
    /// 0x43e105), then in packed fields the entry count, and for each
    /// entry its GUID, its word count and its words. `version`, `packed`
    /// and the reader's positions are not written.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = vec![0; 8];
        bytes.extend(b"TTAT");
        bytes.push(1);
        let mut fields = FieldWriter::new();
        fields.i32(count(self.entries.len())?);
        for entry in &self.entries {
            fields.u32(entry.guid);
            fields.i32(count(entry.values.len())?);
            for &value in &entry.values {
                fields.i16(value);
            }
        }
        bytes.extend(fields.finish());
        Ok(bytes)
    }
}

fn count(len: usize) -> Result<i32> {
    i32::try_from(len).map_err(|_| Error::new(0, "TATT count exceeds 31 bits"))
}
