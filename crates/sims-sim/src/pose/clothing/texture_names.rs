//! 40895a: lazily copy BMP then TGA resource names into the clothing cache.
//! An empty result is retried on the next request; there is no loaded flag.
use super::generator::fixed;
use serde::{Deserialize, Serialize};

pub const BMP: u32 = u32::from_be_bytes(*b"BMP\0");
pub const TGA: u32 = u32::from_be_bytes(*b"TGA\0");

pub trait Host {
    /// Append to the same resource list. Nonzero is a native enumeration
    /// failure; Err denotes an unresolved host operation instead.
    fn enumerate(&mut self, kind: u32, names: &mut Vec<Vec<u8>>) -> Result<i32, String>;
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub names: Vec<Vec<u8>>,
}
impl Cache {
    pub fn load(&mut self, host: &mut impl Host) -> Result<(), String> {
        if !self.names.is_empty() {
            return Ok(());
        }
        let mut names = vec![];
        if host.enumerate(BMP, &mut names)? == 0 && host.enumerate(TGA, &mut names)? == 0 {
            self.names
                .extend(names.iter().map(|name| fixed(name, 63).to_vec()));
        }
        Ok(())
    }
}
