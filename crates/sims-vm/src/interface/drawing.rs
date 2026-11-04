//!4c4222: publish the low byte before synchronous40760d(83,0).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    value: u8,
    entered: bool,
    notify: bool,
}
impl Change {
    pub fn new(value: u32) -> Self {
        Self {
            value: value as u8,
            entered: false,
            notify: false,
        }
    }
    pub fn value(&self) -> u8 {
        self.value
    }
    pub fn written(&self) -> Option<u8> {
        self.entered.then_some(self.value)
    }
    pub fn complete(&self) -> bool {
        self.entered && !self.notify
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.notify && !self.entered {
            return Err("drawing notification precedes its byte write".into());
        }
        Ok(())
    }
    ///Returns true while the actual83 broadcast must run/return.
    pub fn advance(&mut self, drawing: &mut u8) -> Result<bool, String> {
        self.validate()?;
        if !self.entered {
            self.entered = true;
            self.notify = *drawing != self.value;
            if self.notify {
                *drawing = self.value;
            }
        }
        Ok(self.notify)
    }
    pub fn acknowledge(&mut self) -> Result<(), String> {
        self.validate()?;
        if !self.notify {
            return Err("drawing has no pending broadcast".into());
        }
        self.notify = false;
        Ok(())
    }
}
