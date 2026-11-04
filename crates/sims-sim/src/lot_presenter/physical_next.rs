//!432656 ->432823(record0) ->40743b. The registry slot is borrowed, never
//!fabricated from a missing object. No memory writes occur outside the stack.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    IndexCall,
    Indexed,
    IdLoaded,
    LookupEntry,
    Cleared,
    Count,
    Begin,
    Read,
    Result,
    IndexRet,
    WrapperRet,
    CallerRet,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Next {
    child: i16,
    table: Option<[u32; 2]>,
    stage: Stage,
    eax: Option<u32>,
    result: Option<u32>,
}
impl Next {
    pub fn new(child: i16, table: Option<[u32; 2]>) -> Result<Self, String> {
        let s = Self {
            child,
            table,
            stage: Stage::Entry,
            eax: None,
            result: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn child(&self) -> i16 {
        self.child
    }
    pub fn pc(&self) -> u32 {
        match self.stage {
            Stage::Entry => 0x432656,
            Stage::IndexCall => 0x432658,
            Stage::Indexed => 0x432823,
            Stage::IdLoaded => 0x43283b,
            Stage::LookupEntry => 0x40743b,
            Stage::Cleared => 0x407441,
            Stage::Count => 0x407450,
            Stage::Begin => 0x407455,
            Stage::Read => 0x407458,
            Stage::Result => 0x40745c,
            Stage::IndexRet => 0x432841,
            Stage::WrapperRet => 0x43265d,
            Stage::CallerRet => 0x4aebe3,
        }
    }
    pub fn eax(&self) -> Option<u32> {
        self.eax
    }
    pub fn registry_scope(&self) -> bool {
        !matches!(self.stage, Stage::Entry | Stage::IndexCall | Stage::Indexed)
    }
    pub fn esp_delta(&self) -> i32 {
        match self.stage {
            Stage::Entry | Stage::WrapperRet => 0,
            Stage::IndexCall => -4,
            Stage::Indexed | Stage::IdLoaded | Stage::IndexRet => -8,
            Stage::Count => -20,
            Stage::LookupEntry | Stage::Cleared | Stage::Begin | Stage::Read | Stage::Result => -16,
            Stage::CallerRet => 4,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::CallerRet
    }
    pub fn result(&self) -> Option<u32> {
        self.complete().then_some(self.result).flatten()
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some([b, e]) = self.table {
            if b > e || (e - b) % 4 != 0 || e - b > 65536 {
                return Err("physical-next registry extent differs".into());
            }
        }
        let expected = match self.stage {
            Stage::Entry | Stage::IndexCall | Stage::Indexed => None,
            Stage::IdLoaded | Stage::LookupEntry => Some(self.child as i32 as u32),
            Stage::Cleared | Stage::Count | Stage::Begin => Some(0),
            Stage::Read => self.table.map(|t| t[0]),
            _ => self.result,
        };
        if self.eax != expected
            || matches!(
                self.stage,
                Stage::Result | Stage::IndexRet | Stage::WrapperRet | Stage::CallerRet
            ) && self.result.is_none()
        {
            return Err("physical-next result lifetime differs".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        lookup: impl FnOnce(i16) -> Result<Option<u32>, String>,
    ) -> Result<bool, String> {
        self.validate()?;
        self.stage = match self.stage {
            Stage::Entry => Stage::IndexCall,
            Stage::IndexCall => Stage::Indexed,
            Stage::Indexed => {
                self.eax = Some(self.child as i32 as u32);
                Stage::IdLoaded
            }
            Stage::IdLoaded => Stage::LookupEntry,
            Stage::LookupEntry => {
                self.eax = Some(0);
                Stage::Cleared
            }
            Stage::Cleared => {
                if self.child <= 0 {
                    self.result = Some(0);
                    Stage::Result
                } else {
                    if self.table.is_none() {
                        return Err("positive physical-next registry is unobserved".into());
                    }
                    Stage::Count
                }
            }
            Stage::Count => {
                let [b, e] = self.table.ok_or("physical-next registry absent")?;
                if self.child as u32 >= 1 + (e - b) / 4 {
                    self.result = Some(0);
                    Stage::Result
                } else {
                    Stage::Begin
                }
            }
            Stage::Begin => {
                self.eax = self.table.map(|t| t[0]);
                Stage::Read
            }
            Stage::Read => {
                let Some(p) = lookup(self.child)? else {
                    return Ok(false);
                };
                self.eax = Some(p);
                self.result = Some(p);
                Stage::Result
            }
            Stage::Result => Stage::IndexRet,
            Stage::IndexRet => Stage::WrapperRet,
            Stage::WrapperRet => Stage::CallerRet,
            Stage::CallerRet => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
}
