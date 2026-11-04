//! Original 5d532f record constructor and 4b3e5b in-capacity append.
//! The timestamp is an explicit RDTSC input, never the host wall clock.
//! This is one process-global vector, not the whole profiler or outer frame.
use serde::{Deserialize, Serialize};
pub mod tail;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub timestamp: u64,
    /// Opaque native label identity; no string dereference occurs here.
    pub name: u32,
    pub argument: u32,
}
impl Record {
    pub fn from_words(words: [u32; 4]) -> Self {
        Self {
            timestamp: u64::from(words[0]) | (u64::from(words[1]) << 32),
            name: words[2],
            argument: words[3],
        }
    }
    pub fn words(self) -> [u32; 4] {
        [
            self.timestamp as u32,
            (self.timestamp >> 32) as u32,
            self.name,
            self.argument,
        ]
    }
}

/// Logical native capacity is saved explicitly. Rust's allocation capacity
/// cannot replace it: reaching the original end invokes a different child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buffer<T = Record> {
    records: Vec<T>,
    capacity: u32,
}
impl<T> Buffer<T> {
    pub fn from_captured(records: Vec<T>, capacity: u32) -> Result<Self, String> {
        let result = Self { records, capacity };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.records.len() as u64 > u64::from(self.capacity) {
            return Err("profiler records exceed original capacity".into());
        }
        Ok(())
    }
    pub fn records(&self) -> &[T] {
        &self.records
    }
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
    pub fn append(&mut self, record: T) -> Result<(), String> {
        self.validate()?;
        if self.records.len() as u64 == u64::from(self.capacity) {
            return Err("profiler append requires original allocation child 4b411c".into());
        }
        self.records.push(record);
        Ok(())
    }
    fn clear(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Native {
        before: Vec<[u32; 4]>,
        capacity: u32,
        sampled_clock_words: [u32; 2],
        name_pointer: u32,
        argument: u32,
        after: Vec<[u32; 4]>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_rdtsc_constructor_append_and_cold_capacity()
    -> Result<(), Box<dyn std::error::Error>> {
        let native: Native = serde_json::from_str(include_str!(
            "../../../tests/fixtures/native-profiler-append.json"
        ))?;
        let mut buffer = Buffer::from_captured(
            native.before.into_iter().map(Record::from_words).collect(),
            native.capacity,
        )?;
        // Cold storage must preserve original spare slots, regardless of Vec capacity.
        buffer = serde_json::from_slice(&serde_json::to_vec(&buffer)?)?;
        buffer.append(Record::from_words([
            native.sampled_clock_words[0],
            native.sampled_clock_words[1],
            native.name_pointer,
            native.argument,
        ]))?;
        assert_eq!(
            buffer
                .records()
                .iter()
                .copied()
                .map(Record::words)
                .collect::<Vec<_>>(),
            native.after
        );
        assert_eq!(buffer.capacity(), 32);
        let mut full = Buffer::from_captured(
            buffer.records().to_vec(),
            buffer.records().len().try_into()?,
        )?;
        let before = serde_json::to_vec(&full)?;
        assert!(
            full.append(Record::from_words([0; 4]))
                .unwrap_err()
                .contains("4b411c")
        );
        assert_eq!(serde_json::to_vec(&full)?, before);
        let mut malformed = serde_json::to_value(&full)?;
        malformed["capacity"] = 0.into();
        let mut malformed: Buffer = serde_json::from_value(malformed)?;
        let before = serde_json::to_vec(&malformed)?;
        assert!(malformed.append(Record::from_words([0; 4])).is_err());
        assert_eq!(serde_json::to_vec(&malformed)?, before);
        Ok(())
    }
}
