//! Full-range4d4032 record erasure:4e8835 destroys string storage before its
//! image wrapper.4016f8,5a5f3a and5e369b ordering remains explicit at children.
use super::Record;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
fn word(r: &Record, off: usize) -> u32 {
    u32::from_le_bytes(r[off..off + 4].try_into().unwrap())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    FreeString {
        record: usize,
        pointer: u32,
        bytes: u32,
    },
    DestroyImage {
        record: usize,
        image: u32,
        flags: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    String,
    Image,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    index: usize,
    phase: Phase,
    pending: Option<Request>,
}
impl Frame {
    pub fn at_entry() -> Self {
        Self {
            index: 0,
            phase: Phase::String,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self, records: &[Record], images: &BTreeMap<u32, u32>) -> Result<(), String> {
        if records.len() > 4096 || self.index > records.len() {
            return Err("record destruction iterator extent differs".into());
        }
        if self.complete() && (self.index != records.len() || self.pending.is_some()) {
            return Err("completed record destruction retains a child".into());
        }
        if let Some(request) = self.pending {
            let r = records
                .get(self.index)
                .ok_or("record destruction lost its owner")?;
            let valid = match (self.phase, request) {
                (
                    Phase::String,
                    Request::FreeString {
                        record,
                        pointer,
                        bytes,
                    },
                ) => {
                    record == self.index
                        && pointer == word(r, 16)
                        && pointer != 0
                        && bytes != 0
                        && bytes == word(r, 24).wrapping_sub(pointer)
                }
                (
                    Phase::Image,
                    Request::DestroyImage {
                        record,
                        image,
                        flags: 1,
                    },
                ) => {
                    record == self.index
                        && image != 0
                        && image == word(r, 4)
                        && images.get(&image) == Some(&0)
                }
                _ => false,
            };
            if !valid {
                return Err("record destruction child differs from its saved owner".into());
            }
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        records: &mut [Record],
        images: &mut BTreeMap<u32, u32>,
    ) -> Result<Option<Request>, String> {
        self.validate(records, images)?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        for _ in 0..8194 {
            if self.index == records.len() {
                self.phase = Phase::Complete;
                return Ok(None);
            }
            let r = &mut records[self.index];
            let request = match self.phase {
                Phase::String => {
                    let pointer = word(r, 16);
                    let bytes = word(r, 24).wrapping_sub(pointer);
                    if pointer == 0 || bytes == 0 {
                        self.phase = Phase::Image;
                        continue;
                    }
                    Request::FreeString {
                        record: self.index,
                        pointer,
                        bytes,
                    }
                }
                Phase::Image => {
                    let image = word(r, 4);
                    if image == 0 {
                        self.index += 1;
                        self.phase = Phase::String;
                        continue;
                    }
                    let count = images
                        .get_mut(&image)
                        .ok_or("record image reference owner absent")?;
                    *count = count.wrapping_sub(1);
                    if *count == 0 {
                        Request::DestroyImage {
                            record: self.index,
                            image,
                            flags: 1,
                        }
                    } else {
                        r[4..8].fill(0);
                        self.index += 1;
                        self.phase = Phase::String;
                        continue;
                    }
                }
                Phase::Complete => {
                    return Err("record destruction completed before its iterator".into());
                }
            };
            self.pending = Some(request);
            return Ok(Some(request));
        }
        Err("record destruction work bound exceeded".into())
    }
    /// Call only after the concrete resource owner has freed string storage or
    /// executed the image's deleting destructor. Counter decrement precedes a
    /// deleting-destructor child; the wrapper clears only after that returns.
    pub fn acknowledge(
        &mut self,
        records: &mut [Record],
        images: &BTreeMap<u32, u32>,
        request: Request,
    ) -> Result<(), String> {
        self.validate(records, images)?;
        if self.pending != Some(request) {
            return Err("record destruction reply is not its pending child".into());
        }
        match request {
            Request::FreeString { .. } => self.phase = Phase::Image,
            Request::DestroyImage { .. } => {
                records[self.index][4..8].fill(0);
                self.index += 1;
                self.phase = Phase::String;
            }
        }
        self.pending = None;
        self.validate(records, images)
    }
}
