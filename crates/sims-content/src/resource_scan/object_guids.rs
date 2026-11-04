//! Discovery-only projection of Iff::parse + all(OBJD). Validate the whole IFF
//! before returning, including unrelated chunks, with the same error order.
//! Chunk IDs, flags and 64-byte labels are consumed even though this caller only
//! needs payload GUIDs. CP1252 label decoding is infallible; its unused Strings
//! and the complete Chunk vector need not be allocated. This does not read
//! resource maps or reject unsupported OBJD versions: neither did this lookup.
use sims_formats::{Error, Result};

pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<u32>> {
    let mut reader = Cursor { bytes, position: 0 };
    let header = reader.take(64)?;
    if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
        return reader.fail("invalid Maxis IFF signature");
    }
    let mut guids = Vec::new();
    while reader.remaining() > 0 {
        let tag = reader.take(4)?;
        let length = u32::from_be_bytes(reader.take(4)?.try_into().unwrap()) as usize;
        if length < 76 {
            return reader.fail("IFF chunk shorter than its 76-byte header");
        }
        // Keep separate reads: truncated id, flags and label have distinct
        // Reader error offsets/lengths in the full parser.
        reader.take(2)?;
        reader.take(2)?;
        reader.take(64)?;
        let payload = reader.take(length - 76)?;
        // Short and unsupported historical OBJD records remain unrelated until
        // selected; selected definitions still pass through the strict loader.
        if tag == b"OBJD" && payload.len() >= 32 {
            guids.push(u32::from_le_bytes(payload[28..32].try_into().unwrap()));
        }
    }
    Ok(guids)
}

// Reader is private to sims-formats. Keep this narrow checked-slice cursor
// equivalent to its take/remaining/fail implementation; differential tests bind
// complete successful projections and exact Error offset/message on failures.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    fn fail<T>(&self, message: &str) -> Result<T> {
        Err(Error::new(self.position, message))
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| Error::new(self.position, "size overflow"))?;
        let result = self.bytes.get(self.position..end).ok_or_else(|| {
            Error::new(
                self.position,
                format!("need {length} bytes; only {} remain", self.remaining()),
            )
        })?;
        self.position = end;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
