//! Neighborhood TATT: GUID-keyed resource-global words (native 0x43e0f0).
use crate::{Error, Result, field::Fields, reader::Reader};
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
}
