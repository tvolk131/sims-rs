//! Named floating-point tuning (FCNS). Strings are NUL terminated and aligned
//! to even stream offsets, as read by native 0x00464c3e / 0x004131b3.
use crate::Result;
use crate::reader::{Reader, western};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Constant {
    pub name: String,
    pub value: f32,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Constants {
    pub version: u32,
    pub entries: Vec<Constant>,
}

fn string(r: &mut Reader<'_>) -> Result<String> {
    let value = western(r.cstr()?);
    if r.pos & 1 != 0 {
        r.skip(1)?;
    }
    Ok(value)
}

impl Constants {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        r.u32()?;
        let version = r.u32()?;
        if version > 1 {
            return r.fail(format!("unsupported FCNS version {version}"));
        }
        if r.take(4)? != b"SNCF" {
            return r.fail("invalid FCNS signature");
        }
        let count = r.u32()? as usize;
        r.count(count, if version > 0 { 8 } else { 6 })?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let name = string(&mut r)?;
            let value = r.f32()?;
            let comment = if version > 0 {
                string(&mut r)?
            } else {
                String::new()
            };
            entries.push(Constant {
                name,
                value,
                comment,
            });
        }
        Ok(Self { version, entries })
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.entries
            .iter()
            .find(|v| v.name == name)
            .map(|v| v.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strings_align_by_stream_position_and_truncation_is_checked() {
        let mut data = Vec::new();
        data.extend(0u32.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.extend(b"SNCF");
        data.extend(2u32.to_le_bytes());
        data.extend(b"ab\0\x99");
        data.extend(1.25f32.to_le_bytes());
        data.extend(b"note\0\x99");
        data.extend(b"abc\0");
        data.extend((-8.5f32).to_le_bytes());
        data.extend(b"\0\x99");
        let parsed = Constants::parse(&data).unwrap();
        assert_eq!(parsed.get("ab"), Some(1.25));
        assert_eq!(parsed.get("abc"), Some(-8.5));
        assert_eq!(parsed.entries[0].comment, "note");
        for len in 0..data.len() {
            assert!(Constants::parse(&data[..len]).is_err(), "length {len}");
        }
    }
}
