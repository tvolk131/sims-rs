//!4b4525's lower spill and upper spill. Native bounds6c7420/6c741c are .5/1.
//! The corpus covers stored values under masked FP exceptions; this does not
//! emulate x87 exception status or unmasked traps.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightUpdate {
    value: u32,
    next: u8,
}
impl LightUpdate {
    pub fn new(value: u32) -> Self {
        Self { value, next: 0 }
    }
    pub fn complete(&self) -> bool {
        self.next == 2
    }
    pub fn input_bits(&self) -> u32 {
        self.value
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.next > 2 {
            Err("drawing light cursor differs".into())
        } else {
            Ok(())
        }
    }
    pub fn validate_words(&self, words: &[u32; 29]) -> Result<(), String> {
        self.validate()?;
        if self.next != 0 {
            let low = if f32::from_bits(self.value) >= 0.5 {
                self.value
            } else {
                0x3f000000
            };
            let expected = if self.next == 2 && !(f32::from_bits(low) <= 1.0) {
                0x3f800000
            } else {
                low
            };
            if words[0x48 / 4] != expected {
                return Err("drawing light cursor lost its prior value store".into());
            }
        }
        Ok(())
    }
    pub fn advance_words(&mut self, words: &mut [u32; 29]) -> Result<bool, String> {
        self.validate_words(words)?;
        match self.next {
            0 => {
                let value = f32::from_bits(self.value);
                words[0x48 / 4] = if value >= 0.5 { self.value } else { 0x3f000000 };
            }
            1 => {
                let value = f32::from_bits(words[0x48 / 4]);
                if !(value <= 1.0) {
                    words[0x48 / 4] = 0x3f800000;
                }
            }
            _ => return Ok(false),
        }
        self.next += 1;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4b4525 => self.next == 0,
            0x4b4544 => self.next == 1,
            0x4b455c => self.next == 2,
            _ => false,
        }
    }
}
