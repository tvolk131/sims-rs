//! FAR 1a: parse the directory independently of large payloads.
use crate::reader::{Reader, western};
use crate::{Error, Result};

/// The largest FAR directory a reader loads, as the frozen prototype's
/// reader capped it. The game's own archives hold directories of a few
/// megabytes at most.
pub const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;
/// The largest archive member a reader loads, as the prototype capped it.
pub const MAX_MEMBER: u64 = 128 * 1024 * 1024;

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
/// The length of the directory at `offset`: the rest of the file, which
/// must not exceed [`MAX_DIRECTORY`]. Read this many bytes, not the rest
/// of the file, so a sparse file claiming gigabytes is never loaded.
pub fn directory_len(offset: u32, file_len: u64) -> Result<usize> {
    let len = file_len.saturating_sub(u64::from(offset));
    if len > MAX_DIRECTORY {
        return Err(Error::new(
            offset as usize,
            "FAR directory exceeds the 64 MiB limit",
        ));
    }
    Ok(len as usize)
}
impl Entry {
    /// Whether the member's bytes lie after the header and before the
    /// directory at `manifest_offset`. A member that covers the header
    /// could be the archive itself, so a reader that opens archives inside
    /// archives must reject it.
    pub fn within_payload(&self, manifest_offset: u32) -> bool {
        self.offset >= 16
            && u64::from(self.offset) + u64::from(self.stored_size) <= u64::from(manifest_offset)
    }
}
/// A file name as the original sees it, in the Windows-1252 code page that
/// archive member names are in, or None when the name has a character that
/// code page lacks. On Windows the game gets disk names in the system code
/// page; for a Western install that is this one.
pub fn encode_name(name: &str) -> Option<Vec<u8>> {
    let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(name);
    (!unmappable).then(|| bytes.into_owned())
}
/// The reverse of [`encode_name`]: a name the original holds, read in the
/// Windows-1252 code page. Every byte is read as itself: a leading
/// byte-order mark is three characters of the name, as the original's
/// byte comparison sees it, not a sign of another encoding.
pub fn decode_name(name: &[u8]) -> String {
    western(name)
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_member_name_keeps_a_leading_byte_order_mark() {
        let name = b"\xef\xbb\xbfplain.flr";
        let mut b = Vec::new();
        b.extend(1u32.to_le_bytes());
        for word in [0u32, 0, 0, name.len() as u32] {
            b.extend(word.to_le_bytes());
        }
        b.extend(name);
        let entries = super::directory(&b, 0).unwrap();
        assert_eq!(entries[0].name, "\u{ef}\u{bb}\u{bf}plain.flr");
        assert_eq!(entries[0].raw_name, name);
    }
}
