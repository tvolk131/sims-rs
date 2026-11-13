//! FAR 1a: parse the directory independently of large payloads.
use crate::reader::{Reader, western};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub raw_name: Vec<u8>,
    pub offset: u32,
    pub stored_size: u32,
    pub unpacked_size: u32,
}
pub fn manifest_offset(header: &[u8], file_len: u64) -> Result<u32> {
    let mut r = Reader::new(header);
    if r.take(8)? != b"FAR!byAZ" {
        return r.fail("invalid FAR signature");
    }
    if r.u32()? != 1 {
        return r.fail("only FAR version 1a is supported");
    }
    let offset = r.u32()?;
    if offset < 16 || u64::from(offset) + 4 > file_len {
        return r.fail("FAR directory outside file");
    }
    Ok(offset)
}
pub fn directory(data: &[u8], file_len: u64) -> Result<Vec<Entry>> {
    let mut r = Reader::new(data);
    let n = r.u32()? as usize;
    r.count(n, 16)?;
    let mut entries = Vec::with_capacity(n);
    for _ in 0..n {
        let unpacked_size = r.u32()?;
        let stored_size = r.u32()?;
        let offset = r.u32()?;
        let name_len = r.u32()? as usize;
        if name_len > 32768 {
            return r.fail("FAR filename exceeds 32 KiB limit");
        }
        let raw_name = r.take(name_len)?.to_vec();
        if raw_name.contains(&0) {
            return r.fail("embedded NUL in FAR filename");
        }
        if u64::from(offset) + u64::from(stored_size) > file_len {
            return Err(Error::new(r.pos, "FAR entry outside file"));
        }
        entries.push(Entry {
            name: western(&raw_name),
            raw_name,
            offset,
            stored_size,
            unpacked_size,
        });
    }
    Ok(entries)
}
