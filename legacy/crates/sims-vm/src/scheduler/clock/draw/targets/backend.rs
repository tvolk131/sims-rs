//!604e5e surface acquisition. DirectDraw writes and HRESULTs are external
//! replies; nested lock counts and descriptor publication execute here.
use serde::{Deserialize, Serialize};
mod unlock;
pub use unlock::Unlock;
mod blit;
pub use blit::Blit;
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn set(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub raw: Vec<u8>,
}
impl Fields {
    pub fn validate(&self) -> Result<(), String> {
        if self.raw.len() != 0x188 {
            Err("surface backend fields absent".into())
        } else {
            Ok(())
        }
    }
    pub fn buffer(&self) -> u32 {
        word(&self.raw, 0x178)
    }
    pub fn pitch(&self) -> u32 {
        word(&self.raw, 0x17c)
    }
    pub fn locks(&self) -> u32 {
        word(&self.raw, 0x160)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    DirectDrawLock {
        interface: u32,
        descriptor_offset: usize,
        descriptor: Vec<u8>,
        flags: u32,
        event: u32,
    },
    Restore,
    Error {
        hresult: u32,
    },
    DirectDrawUnlock { interface: u32, argument: u32 },
    DirectDrawBlit {
        interface: u32,
        destination: [i32; 4],
        source_interface: u32,
        source: [i32; 4],
        flags: u32,
        effects_address: u32,
        effects: Vec<u8>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Call,
    Returned,
    Publish,
    Success,
    Restore,
    Restored,
    Error,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    stage: Stage,
    pending: Option<Request>,
    hresult: Option<u32>,
    attempts: u8,
    result: Option<u8>,
    restored: Option<u32>,
}
impl Default for Lock {
    fn default() -> Self {
        Self::new()
    }
}
impl Lock {
    pub fn new() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
            hresult: None,
            attempts: 0,
            result: None,
            restored: None,
        }
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn result(&self) -> Option<u8> {
        self.result
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x604e5e => self.stage == Stage::Entry,
            0x604eaa => self.stage == Stage::Call,
            0x604ead => self.stage == Stage::Returned,
            0x604efc => self.stage == Stage::Publish,
            0x604f2d => self.stage == Stage::Success,
            _ => false,
        }
    }
    pub fn validate(&self, fields: &Fields) -> Result<(), String> {
        fields.validate()?;
        if self.attempts > 2 || self.complete() != self.result.is_some() {
            return Err("backend lock lifetime differs".into());
        }
        if matches!(self.stage, Stage::Call | Stage::Restore | Stage::Error)
            != self.pending.is_some()
        {
            return Err("backend child lifetime differs".into());
        }
        match (&self.stage, &self.pending) {
            (
                Stage::Call,
                Some(Request::DirectDrawLock {
                    interface,
                    descriptor_offset,
                    descriptor,
                    flags: 1,
                    event: 0,
                }),
            ) => {
                let has_second = word(&fields.raw, 8) != 0;
                let off = if has_second { 0x78 } else { 0xc };
                if !matches!(descriptor.len(), 108 | 124)
                    || *interface != word(&fields.raw, if has_second { 8 } else { 4 })
                    || *interface == 0
                    || *descriptor_offset != off
                    || descriptor != &fields.raw[off..off + descriptor.len()]
                {
                    return Err("backend DirectDraw call alias differs".into());
                }
            }
            (Stage::Restore, Some(Request::Restore))
            | (Stage::Error, Some(Request::Error { .. })) => (),
            (_, None) => (),
            _ => return Err("backend child kind differs".into()),
        }
        Ok(())
    }
    fn begin_call(&mut self, fields: &Fields) -> Result<(), String> {
        let second = word(&fields.raw, 8) != 0;
        let offset = if second { 0x78 } else { 0xc };
        let size = word(&fields.raw, offset) as usize;
        if !matches!(size, 108 | 124) {
            return Err("backend descriptor type requires its retained owner".into());
        }
        self.stage = Stage::Call;
        self.pending = Some(Request::DirectDrawLock {
            interface: word(&fields.raw, if second { 8 } else { 4 }),
            descriptor_offset: offset,
            descriptor: fields.raw[offset..offset + size].to_vec(),
            flags: 1,
            event: 0,
        });
        Ok(())
    }
    /// Descriptor writes happen inside the platform call, before HRESULT testing.
    /// They are retained even on a failed call.
    pub fn directdraw_return(
        &mut self,
        fields: &mut Fields,
        hresult: u32,
        descriptor: &[u8],
    ) -> Result<(), String> {
        self.validate(fields)?;
        let Some(Request::DirectDrawLock {
            descriptor_offset,
            descriptor: before,
            ..
        }) = &self.pending
        else {
            return Err("backend DirectDraw lock is not pending".into());
        };
        if descriptor.len() != before.len() {
            return Err("backend DirectDraw descriptor extent differs".into());
        }
        let offset = *descriptor_offset;
        fields.raw[offset..offset + descriptor.len()].copy_from_slice(descriptor);
        self.hresult = Some(hresult);
        self.pending = None;
        self.stage = Stage::Returned;
        self.validate(fields)
    }
    pub fn restore_return(&mut self, fields: &Fields, result: u32) -> Result<(), String> {
        self.validate(fields)?;
        if self.pending != Some(Request::Restore) {
            return Err("backend restore is not pending".into());
        }
        self.pending = None;
        self.restored = Some(result);
        self.stage = Stage::Restored;
        self.validate(fields)
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<bool, String> {
        self.validate(fields)?;
        if self.pending.is_some() {
            return Ok(false);
        }
        match self.stage {
            Stage::Entry => {
                if fields.locks() != 0 {
                    let count = fields.locks().wrapping_add(1);
                    set(&mut fields.raw, 0x160, count);
                    self.stage = Stage::Success;
                } else {
                    self.begin_call(fields)?;
                }
            }
            Stage::Returned => {
                if self.hresult == Some(0) {
                    self.stage = Stage::Publish;
                } else if self.hresult == Some(0x887601c2) {
                    self.stage = Stage::Restore;
                    self.pending = Some(Request::Restore);
                } else {
                    self.stage = Stage::Error;
                    self.pending = Some(Request::Error {
                        hresult: self.hresult.unwrap(),
                    });
                }
            }
            Stage::Publish => {
                let second = word(&fields.raw, 8) != 0;
                let buffer = word(&fields.raw, if second { 0x9c } else { 0x30 });
                let pitch = word(&fields.raw, if second { 0x88 } else { 0x1c });
                set(&mut fields.raw, 0x178, buffer);
                set(&mut fields.raw, 0x17c, pitch);
                let count = fields.locks().wrapping_add(1);
                set(&mut fields.raw, 0x160, count);
                self.stage = Stage::Success;
            }
            Stage::Success => {
                self.result = Some(1);
                self.stage = Stage::Complete;
            }
            Stage::Restored => {
                if self.restored.take().unwrap() as u8 == 0 {
                    self.result = Some(0);
                    self.stage = Stage::Complete;
                } else {
                    self.attempts += 1;
                    if self.attempts < 2 {
                        self.begin_call(fields)?;
                    } else {
                        self.stage = Stage::Error;
                        self.pending = Some(Request::Error {
                            hresult: self.hresult.unwrap(),
                        });
                    }
                }
            }
            Stage::Complete => return Ok(false),
            _ => return Err("backend lock lost its child".into()),
        }
        self.validate(fields)?;
        Ok(true)
    }
}
