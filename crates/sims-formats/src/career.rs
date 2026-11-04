//! CARR, decoded by native 0x4035e7/0x403433. Numeric fields are kept in
//! storage order; naming or scaling them does not belong to the file reader.
use crate::{
    Error, Result,
    field::Fields,
    reader::{Reader, western},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Level {
    pub values: [i32; 21],
    /// Native offsets 0x54, 0x284, 0x2d0, 0x31c and 0x368 respectively.
    pub strings: [String; 5],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Career {
    pub version: u32,
    pub packed: bool,
    pub name: String,
    pub levels: Vec<Level>,
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
    fn string(&mut self) -> Result<String> {
        match self {
            Self::Packed(f) => f.string(),
            Self::Raw(r) => {
                let value = western(r.cstr()?);
                if r.pos & 1 != 0 {
                    r.skip(1)?;
                }
                Ok(value)
            }
        }
    }
    fn position(&self) -> usize {
        match self {
            Self::Packed(f) => 13 + f.byte_position(),
            Self::Raw(r) => r.pos,
        }
    }
}
impl Career {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.u32()? != 0 {
            return r.fail("unsupported CARR outer stream mode");
        }
        let version = r.u32()?;
        if r.take(4)? != b"RRAC" {
            return r.fail("invalid CARR signature");
        }
        let packed = r.u8()? != 0;
        let mut stream = if packed {
            Stream::Packed(Fields::new(&bytes[13..], 13))
        } else {
            Stream::Raw(r)
        };
        let name = stream.string()?;
        let count = stream.int()?;
        if count < 0 || count as usize > bytes.len() {
            return Err(Error::new(stream.position(), "invalid CARR level count"));
        }
        let mut levels = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let mut values = [0; 21];
            for value in &mut values {
                *value = stream.int()?;
            }
            let mut strings = std::array::from_fn(|_| String::new());
            for value in &mut strings {
                *value = stream.string()?;
            }
            levels.push(Level { values, strings });
        }
        Ok(Self {
            version,
            packed,
            name,
            levels,
            consumed: stream.position(),
        })
    }
}
