//! 5b9dbc's incremental manifest reads. This controller reads but does not
//! validate magic/version. The ordinary FAR parser separately validates source
//! archives; this reader preserves partial native mounting behavior.
use crate::{Error, Result};
pub struct Reader<'a> {
    data: &'a [u8],
    directory: Option<&'a [u8]>,
    pub position: usize,
    pub manifest_offset: Option<u32>,
}
#[derive(Debug)]
pub struct Entry {
    pub name: Vec<u8>,
    pub offset: u32,
    pub size: u32,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            directory: None,
            position: 0,
            manifest_offset: None,
        }
    }
    pub fn with_directory(header: &'a [u8], directory: &'a [u8]) -> Self {
        Self {
            directory: Some(directory),
            ..Self::new(header)
        }
    }
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (data, start) = match (self.directory, self.manifest_offset) {
            (Some(directory), Some(offset)) => {
                (directory, self.position.checked_sub(offset as usize)?)
            }
            _ => (self.data, self.position),
        };
        let available = data.len().saturating_sub(start).min(count);
        self.position += available;
        if available == count {
            data.get(start..start + available)
        } else {
            None
        }
    }
    fn word(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn count(&mut self) -> Option<i32> {
        self.take(8)?;
        self.word()?;
        let offset = self.word()?;
        self.manifest_offset = Some(offset);
        self.position = offset as usize;
        Some(self.word()? as i32)
    }
    pub fn entry(&mut self) -> Result<Option<Entry>> {
        let Some(size) = self.word() else {
            return Ok(None);
        };
        if self.word().is_none() {
            return Ok(None);
        }
        let Some(offset) = self.word() else {
            return Ok(None);
        };
        let Some(length) = self.word() else {
            return Ok(None);
        };
        if length >= 2048 {
            return Err(Error::new(
                self.position,
                "FAR filename exceeds native mount buffer",
            ));
        }
        let Some(name) = self.take(length as usize) else {
            return Ok(None);
        };
        let name = name.split(|&c| c == 0).next().unwrap().to_vec();
        Ok(Some(Entry { name, offset, size }))
    }
}
