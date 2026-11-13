//! Persistent operands in the recovered Application324 caller frame.
//! The incoming temporary is [ebp-2c]; deriving these views supplies no real
//! stack allocation, SEH prologue, register state or native process epoch.
use super::{Owner, Stop, unsupported};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Frame {
    temporary: u64,
    ebp: u64,
    end: u64,
}

impl Frame {
    pub fn new(temporary: u64) -> Result<Self, Stop> {
        let ebp = temporary
            .checked_add(0x2c)
            .ok_or_else(|| unsupported("application frame pointer overflows"))?;
        let end = ebp
            .checked_add(12)
            .filter(|end| *end <= 1_u64 << 32)
            .ok_or_else(|| unsupported("application frame exceeds native width"))?;
        if temporary == 0 {
            return Err(unsupported("application frame has no temporary view"));
        }
        Ok(Self {
            temporary,
            ebp,
            end,
        })
    }

    pub fn containing<'a>(&self, retained: &'a [Owner]) -> Result<&'a Owner, Stop> {
        retained
            .iter()
            .find(|owner| {
                self.temporary >= owner.token
                    && owner
                        .token
                        .checked_add(owner.data.len() as u64)
                        .is_some_and(|end| self.end <= end)
            })
            .ok_or_else(|| {
                unsupported("application frame operands need one complete retained caller owner")
            })
    }

    pub fn address(&self, relative: i32, bytes: usize) -> Result<u64, Stop> {
        let address = self
            .ebp
            .checked_add_signed(i64::from(relative))
            .filter(|address| *address >= self.temporary)
            .ok_or_else(|| unsupported("application frame operand starts outside its view"))?;
        if bytes == 0
            || address
                .checked_add(bytes as u64)
                .is_none_or(|end| end > self.end)
        {
            return Err(unsupported(
                "application frame operand ends outside its view",
            ));
        }
        Ok(address)
    }
}
