//! 413083/413105 stream access. Filesystem operations are host boundaries;
//! the writable byte is independent of both requested access and open state.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub opened: bool,
    pub writable: u8,
}

pub trait Host {
    fn open(&mut self, writable: bool) -> Result<bool, String>;
    fn buffer(&mut self, read: u32, write: u32) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
}

impl Access {
    pub fn open(&mut self, request: u32, host: &mut impl Host) -> Result<i32, String> {
        if self.opened {
            return Ok(-47);
        }
        if request as u8 != 0 {
            self.opened = host.open(true)?;
            if self.opened {
                host.buffer(128, 128)?;
                self.writable = 1;
            }
        }
        if !self.opened {
            self.writable = 0;
            self.opened = host.open(false)?;
            if self.opened {
                host.buffer(128, 0)?;
            }
        }
        Ok(if self.opened { 0 } else { -50 })
    }

    pub fn close(&mut self, host: &mut impl Host) -> Result<i32, String> {
        if !self.opened {
            return Ok(-49);
        }
        host.close()?;
        self.opened = false;
        // Native deliberately retains the last writable byte after closing.
        Ok(0)
    }
}
