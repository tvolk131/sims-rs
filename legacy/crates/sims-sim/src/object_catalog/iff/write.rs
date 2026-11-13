//! Native resident/disk writeback. Stream writes target owned host storage.
//! Inactive block headers contain unspecified native stack bytes; Rust uses
//! initialized storage for those bytes. Active chunk bytes and ordering match.
use super::{Iff, Payload, Record, Stream, Table};

pub trait WriteStream: Stream {
    fn writable(&self) -> bool;
    fn write(&mut self, bytes: &[u8]) -> Result<i32, String>;
    fn position(&mut self) -> Result<Result<u32, i32>, String>;
    fn flush(&mut self) -> Result<i32, String>;
}
pub struct Add<'a> {
    pub payload: Option<Payload>,
    pub tag: u32,
    pub id: u32,
    pub variant: u32,
    pub name: &'a [u8],
    pub converted: u32,
}

#[derive(Clone)]
struct Header([u8; 76]);
impl Header {
    fn new() -> Self {
        Self([0; 76])
    }
    fn size(&self) -> u32 {
        u32::from_be_bytes(self.0[4..8].try_into().unwrap())
    }
    fn set_size(&mut self, n: u32) {
        self.0[4..8].copy_from_slice(&n.to_be_bytes());
    }
    fn flags(&self) -> u16 {
        u16::from_be_bytes(self.0[10..12].try_into().unwrap())
    }
    fn set_flags(&mut self, n: u16) {
        self.0[10..12].copy_from_slice(&n.to_be_bytes());
    }
    fn read(&mut self, offset: u32, stream: &mut impl WriteStream) -> Result<i32, String> {
        Iff::read_at(stream, offset, &mut self.0)
    }
}
impl Iff {
    pub fn allocate_payload(&mut self, bytes: Vec<u8>) -> Result<Payload, String> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("IFF payload exceeds 128 MiB".into());
        }
        let handle = self.next_handle;
        self.next_handle = handle.checked_add(1).ok_or("IFF handle space exhausted")?;
        Ok(Payload { handle, bytes })
    }
    fn handle_index(&self, handle: u32) -> Option<usize> {
        self.table
            .iter()
            .flat_map(|(_, t)| t.iter())
            .find_map(|(_, i)| {
                (self.storage[*i].payload.as_ref().map_or(0, |p| p.handle) == handle).then_some(*i)
            })
    }
    fn write_guard(&mut self, stream: &impl WriteStream) -> bool {
        if !self.guard() {
            return false;
        }
        if !stream.writable() {
            self.error = -45;
            return false;
        }
        true
    }
    /// 41cf32 deliberately looks for variant zero before inserting the supplied
    /// variant; this can replace variant zero even when another was requested.
    pub fn add(&mut self, input: Add<'_>, stream: &mut impl WriteStream) -> Result<(), String> {
        if !self.write_guard(stream) {
            return Ok(());
        }
        let index = self
            .table
            .get(input.tag)
            .and_then(|t| t.get(input.id & 65535))
            .copied();
        let index = if let Some(index) = index {
            if self.storage[index].payload.as_ref().map(|p| p.handle)
                == input.payload.as_ref().map(|p| p.handle)
            {
                self.storage[index].payload = None;
            }
            let code = self.discard(index, stream)?;
            if code != 0 {
                self.error = code;
                return Ok(());
            }
            let _ = stream.flush()?;
            index
        } else {
            if self.table.get(input.tag).is_none() {
                self.table.insert(input.tag, Table::new(true))?;
            }
            let table = self.table.get_mut(input.tag).unwrap();
            let key = input.id & 65535 | ((input.variant & 255) << 16);
            if let Some(&index) = table.get(key) {
                index
            } else {
                let ordinal = table
                    .iter()
                    .map(|(_, i)| self.storage[*i].ordinal as i16)
                    .fold(0_i16, i16::max)
                    .wrapping_add(1) as u16;
                let index = self.storage.len();
                self.storage.push(Record {
                    tag: input.tag,
                    id: input.id as u16,
                    flags: (input.variant as u16) << 8,
                    disk_flags: 0,
                    ordinal,
                    offset: u32::MAX,
                    name: vec![],
                    payload: None,
                });
                table.insert(key, index)?;
                index
            }
        };
        let record = &mut self.storage[index];
        record.flags = (record.flags & 0xff00) | if input.converted as u8 != 0 { 16 } else { 0 };
        record.disk_flags = 0;
        record.payload = input.payload;
        record.name = super::super::cstring(input.name)
            .iter()
            .take(63)
            .copied()
            .collect();
        record.offset = u32::MAX;
        Ok(())
    }
    fn invalidate_map(&mut self, stream: &mut impl WriteStream) -> Result<i32, String> {
        let offset = self.map_offset;
        if offset == 0 {
            return Ok(0);
        }
        // 41c698 temporarily clears the map offset to avoid recursive invalidation.
        self.map_offset = 0;
        let result = self.erase_disk(offset, stream);
        self.map_offset = offset;
        let code = result?;
        if code != 0 {
            return Ok(code);
        }
        let mut header = *super::HEADER;
        header[9] = b'2';
        header[11] = b'5';
        header[60..].fill(0);
        let mut code = stream.seek(0)?;
        if code == 0 {
            code = stream.write(&header)?;
        }
        if code == 0 {
            let _ = stream.flush()?;
            self.map_offset = 0;
            self.modified = true;
        }
        Ok(code)
    }
    fn write_header(
        &mut self,
        header: &Header,
        offset: u32,
        stream: &mut impl WriteStream,
    ) -> Result<i32, String> {
        self.modified = true;
        let code = self.invalidate_map(stream)?;
        if code != 0 {
            return Ok(code);
        }
        let code = stream.seek(offset)?;
        if code != 0 {
            return Ok(code);
        }
        stream.write(&header.0)
    }
    fn erase_disk(&mut self, offset: u32, stream: &mut impl WriteStream) -> Result<i32, String> {
        let mut header = Header::new();
        let code = header.read(offset, stream)?;
        if code != 0 {
            return Ok(code);
        }
        header.0[..4].copy_from_slice(b"XXXX");
        header.set_flags(header.flags() | 4);
        self.unused = self.unused.wrapping_add(header.size());
        self.write_header(&header, offset, stream)
    }
    fn discard(&mut self, index: usize, stream: &mut impl WriteStream) -> Result<i32, String> {
        let offset = self.storage[index].offset;
        if offset != u32::MAX {
            let code = self.erase_disk(offset, stream)?;
            if code != 0 {
                return Ok(code);
            }
        }
        self.storage[index].payload = None;
        Ok(0)
    }
    /// 41d192/41bc7b. Removal searches the outer tables by the record's key,
    /// rather than retaining the type containing the matched payload handle.
    pub fn delete(&mut self, handle: u32, stream: &mut impl WriteStream) -> Result<(), String> {
        if !self.write_guard(stream) {
            return Ok(());
        }
        if let Some(i) = self.handle_index(handle) {
            let code = self.discard(i, stream)?;
            if code == 0 {
                let r = &self.storage[i];
                let key = u32::from(r.id) | (u32::from(r.flags >> 8) << 16);
                let tag = self
                    .table
                    .iter()
                    .find_map(|(tag, t)| t.get(key).map(|_| *tag));
                if let Some(tag) = tag {
                    self.table.get_mut(tag).unwrap().remove(key);
                }
            } else {
                self.error = code;
            }
        } else {
            self.error = -99;
        }
        let _ = stream.flush()?;
        Ok(())
    }
    fn reserve_disk(
        &mut self,
        header: &mut Header,
        size: u32,
        offset: &mut u32,
        stream: &mut impl WriteStream,
    ) -> Result<i32, String> {
        self.modified = true;
        let code = self.invalidate_map(stream)?;
        if code != 0 {
            return Ok(code);
        }
        let total = size.checked_add(76).ok_or("IFF chunk size overflow")?;
        *offset = if self.write_cursor as i32 > 0 {
            self.write_cursor
        } else {
            64
        };
        for _ in 0..1_000_000 {
            let code = header.read(*offset, stream)?;
            if code != 0 {
                // Native only queries position to check stream access; it keeps
                // the attempted chunk offset even after a partial header read.
                if let Err(code) = stream.position()? {
                    return Ok(code);
                }
                self.write_cursor = *offset;
                header.set_size(total);
                header.0[..4].copy_from_slice(b"XXXX");
                header.set_flags(4);
                header.0[12..].fill(0);
                return self.write_header(header, *offset, stream);
            }
            self.write_cursor = *offset;
            if header.flags() & 4 != 0 {
                let mut next = Header::new();
                let mut merged = false;
                if next.read(offset.wrapping_add(header.size()), stream)? == 0 {
                    loop {
                        if next.flags() & 4 == 0 {
                            break;
                        }
                        if next.size() < 76 {
                            return Err("native free-block scan would not advance".into());
                        }
                        header.set_size(header.size().wrapping_add(next.size()));
                        merged = true;
                        if next.read(offset.wrapping_add(header.size()), stream)? != 0 {
                            break;
                        }
                    }
                    if merged {
                        let code = self.write_header(header, *offset, stream)?;
                        if code != 0 {
                            return Ok(code);
                        }
                    }
                }
                if header.size() == total {
                    self.unused = self.unused.wrapping_sub(total);
                    return Ok(0);
                }
                if header.size() as i32 >= total.wrapping_add(76) as i32 {
                    next.0[..4].copy_from_slice(b"XXXX");
                    next.set_size(header.size() - total);
                    next.set_flags(4);
                    let code = self.write_header(&next, offset.wrapping_add(total), stream)?;
                    if code != 0 {
                        return Ok(code);
                    }
                    header.set_size(total);
                    let code = self.write_header(header, *offset, stream)?;
                    if code != 0 {
                        return Ok(code);
                    }
                    self.unused = self.unused.wrapping_sub(total);
                    return Ok(0);
                }
            }
            if header.size() < 76 {
                return Err("native allocation scan would not advance".into());
            }
            *offset = offset
                .checked_add(header.size())
                .ok_or("IFF allocation offset overflow")?;
        }
        Err("IFF allocation scan exceeds host capacity".into())
    }
    pub fn write_chunk(
        &mut self,
        handle: u32,
        stream: &mut impl WriteStream,
    ) -> Result<(), String> {
        if !self.write_guard(stream) {
            return Ok(());
        }
        let Some(i) = self.handle_index(handle) else {
            self.error = -99;
            return Ok(());
        };
        let bytes = self.storage[i]
            .payload
            .as_ref()
            .map(|p| p.bytes.clone())
            .unwrap_or_default();
        let mut offset = self.storage[i].offset;
        let mut header = Header::new();
        if offset != u32::MAX {
            let _ = header.read(offset, stream)?;
            if header.size() != bytes.len() as u32 + 76 {
                let _ = self.erase_disk(offset, stream)?;
                let _ = self.reserve_disk(&mut header, bytes.len() as u32, &mut offset, stream)?;
            }
        } else {
            let _ = self.reserve_disk(&mut header, bytes.len() as u32, &mut offset, stream)?;
        }
        self.storage[i].offset = offset;
        let r = &self.storage[i];
        header.0[..4].copy_from_slice(&r.tag.to_be_bytes());
        header.0[8..10].copy_from_slice(&r.id.to_be_bytes());
        header.set_flags(r.flags);
        header.0[12..].fill(0);
        header.0[12..12 + r.name.len()].copy_from_slice(&r.name);
        let mut code = self.write_header(&header, offset, stream)?;
        if code == 0 {
            code = stream.seek(offset.wrapping_add(76))?;
        }
        if code == 0 {
            code = stream.write(&bytes)?;
            if code == 0 {
                self.storage[i].disk_flags = self.storage[i].flags;
                let _ = stream.flush()?;
                return Ok(());
            }
        }
        self.error = code;
        Ok(())
    }
    /// 41ce00 clears the payload and resets flags even when writing fails. Its
    /// final zero error overwrites the write error; preserve this native quirk.
    pub fn detach(&mut self, handle: u32, stream: &mut impl WriteStream) -> Result<(), String> {
        self.detach_payload(handle, stream).map(|_| ())
    }
    /// Returns the detached allocation to its caller without copying it.
    pub fn detach_payload(
        &mut self,
        handle: u32,
        stream: &mut impl WriteStream,
    ) -> Result<Option<Payload>, String> {
        if !self.guard() {
            return Ok(None);
        }
        let Some(i) = self.handle_index(handle) else {
            self.error = -99;
            return Ok(None);
        };
        if self.storage[i].offset == u32::MAX {
            self.write_chunk(handle, stream)?;
        }
        let r = &mut self.storage[i];
        let payload = r.payload.take();
        r.flags = (r.flags & 0xff00) | (r.disk_flags & 255);
        self.error = 0;
        Ok(payload)
    }
    pub fn flush(&mut self, stream: &mut impl WriteStream) -> Result<(), String> {
        if !self.guard() {
            return Ok(());
        }
        let indexes = self
            .table
            .iter()
            .flat_map(|(_, t)| t.iter().map(|(_, i)| *i))
            .collect::<Vec<_>>();
        let mut first = 0;
        for i in indexes.into_iter().rev() {
            if self.storage[i].offset == u32::MAX {
                let handle = self.storage[i].payload.as_ref().map_or(0, |p| p.handle);
                self.write_chunk(handle, stream)?;
                if first == 0 {
                    first = self.error;
                }
            }
        }
        if first == 0 {
            first = stream.flush()?;
        }
        self.error = first;
        Ok(())
    }
}
