//! 59fc48's NDX record stream, using 5b91d4 and 5b9201. These are lazy catalog
//! references, not meshes or animations. Names retain the original byte case.
use crate::{Result, reader::Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Skeleton,
    Appearance,
    Animation,
}
impl Kind {
    pub fn index(self) -> usize {
        match self {
            Self::Skeleton => 0,
            Self::Appearance => 1,
            Self::Animation => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub name: Vec<u8>,
    /// Original catalog filename before 59fc48 appends the literal `.bcf`.
    pub catalog: Vec<u8>,
}

/// Streaming so a caller can preserve successful registrations before a later
/// truncated record. Negative group counts skip that group in the original.
/// Trailing bytes remain unread. Native unsafe oversized strings are rejected.
pub struct IndexReader<'a> {
    reader: Reader<'a>,
    group: usize,
    remaining: usize,
    failed: bool,
    short_read: bool,
}
impl<'a> IndexReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
            group: 0,
            remaining: 0,
            failed: false,
            short_read: false,
        }
    }
    pub fn bytes_consumed(&self) -> usize {
        self.reader.pos
    }
    /// Distinguishes an ordinary failed stream read from a Rust safety/support
    /// limit (for example the original's unbounded string copy).
    pub fn failed_short_read(&self) -> bool {
        self.short_read
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let result = self.reader.take(size);
        if result.is_err() {
            self.short_read = true;
            // A short native byte-stream read consumes every available byte.
            // The error still points to the start of the failed read.
            self.reader.pos = self.reader.data.len();
        }
        result
    }
    fn word(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<Vec<u8>> {
        let prefix = self.take(1)?[0];
        let size = if prefix == 255 {
            self.word()? as usize
        } else {
            prefix as usize
        };
        // The native helper ignores its capacity argument. Keep the verified
        // 1024-byte caller buffer bounded, including its terminal NUL.
        if size >= 1024 {
            return self
                .reader
                .fail("NDX string exceeds native buffer capacity");
        }
        let bytes = self.take(size)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(bytes[..end].to_vec())
    }
    fn record(&mut self) -> Result<Option<Entry>> {
        while self.remaining == 0 {
            if self.group == 3 {
                return Ok(None);
            }
            self.remaining = (self.word()? as i32).max(0) as usize;
            self.group += 1;
        }
        let name = self.string()?;
        let catalog = self.string()?;
        self.remaining -= 1;
        Ok(Some(Entry {
            kind: [Kind::Skeleton, Kind::Appearance, Kind::Animation][self.group - 1],
            name,
            catalog,
        }))
    }
}
impl Iterator for IndexReader<'_> {
    type Item = Result<Entry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.record() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => None,
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}
