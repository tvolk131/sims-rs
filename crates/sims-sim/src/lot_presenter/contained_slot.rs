//!437fdd's no-child branch, including40743b's nonpositive-ID fast return.
//! Unknown output words remain unknown until their original MOVSD stores.
use serde::{Deserialize, Serialize};
pub const EMPTY: [i32; 4] = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    LookupCall,
    LookupEntry,
    LookedUp,
    Output,
    Copy(u8),
    Return,
    Returned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {
    child: i16,
    stage: Stage,
    output: [Option<i32>; 4],
}
impl Empty {
    pub fn new(child: i16) -> Result<Self, String> {
        if child > 0 {
            return Err("contained drawing requires its positive-ID lookup/child".into());
        }
        Ok(Self {
            child,
            stage: Stage::Entry,
            output: [None; 4],
        })
    }
    pub fn child(&self) -> i16 {
        self.child
    }
    pub fn pc(&self) -> u32 {
        match self.stage {
            Stage::Entry => 0x437fdd,
            Stage::LookupCall => 0x438008,
            Stage::LookupEntry => 0x40743b,
            Stage::LookedUp => 0x43800d,
            Stage::Output => 0x4380e1,
            Stage::Copy(n) => 0x4380e9 + u32::from(n),
            Stage::Return => 0x4380f0,
            Stage::Returned => 0x437cc2,
        }
    }
    pub fn initialized_locals(&self) -> bool {
        self.stage != Stage::Entry
    }
    pub fn lookup_returned(&self) -> bool {
        self.stage == Stage::LookedUp
    }
    pub fn output_words(&self) -> [Option<i32>; 4] {
        self.output
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn result(&self) -> Option<[i32; 4]> {
        self.complete().then_some(EMPTY)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.child > 0 || matches!(self.stage,Stage::Copy(n)if n>3) {
            return Err("invalid empty contained-slot state".into());
        }
        let count = match self.stage {
            Stage::Copy(n) => n as usize,
            Stage::Return | Stage::Returned => 4,
            _ => 0,
        };
        for (n, word) in self.output.iter().enumerate() {
            if *word != (n < count).then_some(EMPTY[n]) {
                return Err("contained-slot output lifetime differs".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self) -> Result<bool, String> {
        self.validate()?;
        self.stage = match self.stage {
            Stage::Entry => Stage::LookupCall,
            Stage::LookupCall => Stage::LookupEntry,
            Stage::LookupEntry => Stage::LookedUp,
            Stage::LookedUp => Stage::Output,
            Stage::Output => Stage::Copy(0),
            Stage::Copy(n) => {
                self.output[n as usize] = Some(EMPTY[n as usize]);
                if n == 3 {
                    Stage::Return
                } else {
                    Stage::Copy(n + 1)
                }
            }
            Stage::Return => Stage::Returned,
            Stage::Returned => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
}
