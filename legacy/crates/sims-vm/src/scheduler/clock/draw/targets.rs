//!4b017b color/depth target publication and plane lock/unlock operations. Virtual image calls remain
//! explicit children; AL lock results and partial failure state are retained.
use serde::{Deserialize, Serialize};
pub mod backend;
mod unlock;
pub use unlock::PlaneUnlock;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buffers {
    pub color: u32,
    pub color_pitch: u32,
    pub depth: u32,
    pub depth_pitch: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub active: u8,
    pub phase: i32,
    pub surface: u32,
    pub buffers: Buffers,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Lock { surface: u32, flags: u32 },
    Unlock { surface: u32, flags: u32 },
    Buffer { surface: u32 },
    Pitch { surface: u32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ColorLock,
    ColorLocked,
    DepthGate,
    DepthLock,
    DepthLocked,
    Cleanup,
    CleanupReturned,
    ColorGate,
    ColorBuffer,
    ColorBufferReturned,
    ColorPitch,
    ColorPitchReturned,
    DepthBufferGate,
    DepthBuffer,
    DepthBufferReturned,
    DepthPitch,
    DepthPitchReturned,
    Publish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Begin {
    surface: u32,
    stage: Stage,
    pending: Option<Request>,
    reply: Option<u32>,
    activated: bool,
    depth_reader: Option<u32>,
}
impl Begin {
    pub fn new(surface: u32) -> Result<Self, String> {
        if surface == 0 {
            return Err("target image owner absent".into());
        }
        Ok(Self {
            surface,
            stage: Stage::Entry,
            pending: None,
            reply: None,
            activated: false,
            depth_reader: None,
        })
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn activated(&self) -> bool {
        self.activated
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4b017b => self.stage == Stage::Entry,
            0x4b019f => self.stage == Stage::ColorLock,
            0x4b01a2 => self.stage == Stage::ColorLocked,
            0x4b01c3 => self.stage == Stage::DepthLock,
            0x4b01c6 => self.stage == Stage::DepthLocked,
            0x4b01e2 => self.stage == Stage::Cleanup,
            0x4b0202 => self.stage == Stage::ColorBufferReturned,
            0x4b0211 => self.stage == Stage::ColorPitchReturned,
            0x4b0243 => self.stage == Stage::DepthBufferReturned,
            0x4b0252 => self.stage == Stage::DepthPitchReturned,
            0x4b0265 => self.stage == Stage::Publish,
            0x4b0276 => self.complete(),
            _ => false,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.surface == 0 || self.activated && !self.complete() {
            return Err("target begin lifetime differs".into());
        }
        let returned = matches!(
            self.stage,
            Stage::ColorLocked
                | Stage::DepthLocked
                | Stage::CleanupReturned
                | Stage::ColorBufferReturned
                | Stage::ColorPitchReturned
                | Stage::DepthBufferReturned
                | Stage::DepthPitchReturned
        );
        if returned != self.reply.is_some() {
            return Err("target virtual return lifetime differs".into());
        }
        let calling = matches!(
            self.stage,
            Stage::ColorLock
                | Stage::DepthLock
                | Stage::Cleanup
                | Stage::ColorBuffer
                | Stage::ColorPitch
                | Stage::DepthBuffer
                | Stage::DepthPitch
        );
        if calling != self.pending.is_some() {
            return Err("target call lost its receiver".into());
        }
        let expected = match self.stage {
            Stage::ColorLock => Some(Request::Lock {
                surface: self.surface,
                flags: 64,
            }),
            Stage::DepthLock => self.pending.filter(|r| {
                matches!(
                    r,
                    Request::Lock {
                        surface: 1..,
                        flags: 64
                    }
                )
            }),
            Stage::Cleanup => Some(Request::Unlock {
                surface: self.surface,
                flags: 16,
            }),
            Stage::ColorBuffer => Some(Request::Buffer {
                surface: self.surface,
            }),
            Stage::ColorPitch => Some(Request::Pitch {
                surface: self.surface,
            }),
            Stage::DepthBuffer => self
                .pending
                .filter(|r| matches!(r, Request::Buffer { surface: 1.. })),
            Stage::DepthPitch => self
                .pending
                .filter(|r| matches!(r, Request::Pitch { surface: 1.. })),
            _ => None,
        };
        if self.pending != expected {
            return Err("target virtual child differs".into());
        }
        Ok(())
    }
    fn call(&mut self, stage: Stage, request: Request) {
        self.stage = stage;
        self.pending = Some(request);
    }
    pub fn acknowledge(&mut self, request: Request, result: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("target virtual return owner differs".into());
        }
        self.stage = match self.stage {
            Stage::ColorLock => Stage::ColorLocked,
            Stage::DepthLock => Stage::DepthLocked,
            Stage::Cleanup => Stage::CleanupReturned,
            Stage::ColorBuffer => Stage::ColorBufferReturned,
            Stage::ColorPitch => Stage::ColorPitchReturned,
            Stage::DepthBuffer => Stage::DepthBufferReturned,
            Stage::DepthPitch => Stage::DepthPitchReturned,
            _ => return Err("target virtual return is not pending".into()),
        };
        self.pending = None;
        self.reply = Some(result);
        self.validate()
    }
    /// Flags and the optional depth link are current borrowed inputs, reread at
    /// their native gates. A virtual child retains its already selected receiver.
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        flags: u32,
        depth: Option<u32>,
    ) -> Result<bool, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(false);
        }
        match self.stage {
            Stage::Entry => {
                if flags & 1 != 0 {
                    self.call(
                        Stage::ColorLock,
                        Request::Lock {
                            surface: self.surface,
                            flags: 64,
                        },
                    )
                } else {
                    self.stage = Stage::DepthGate
                }
            }
            Stage::ColorLocked => {
                let ok = self.reply.take().unwrap() as u8 != 0;
                self.stage = if ok {
                    Stage::DepthGate
                } else {
                    Stage::Complete
                };
            }
            Stage::DepthGate => {
                if flags & 2 != 0 && depth.is_some() {
                    self.call(
                        Stage::DepthLock,
                        Request::Lock {
                            surface: depth.unwrap(),
                            flags: 64,
                        },
                    )
                } else {
                    self.stage = Stage::ColorGate
                }
            }
            Stage::DepthLocked => {
                let ok = self.reply.take().unwrap() as u8 != 0;
                if ok {
                    self.stage = Stage::ColorGate
                } else if flags & 1 != 0 {
                    self.call(
                        Stage::Cleanup,
                        Request::Unlock {
                            surface: self.surface,
                            flags: 16,
                        },
                    )
                } else {
                    self.stage = Stage::Complete
                }
            }
            Stage::CleanupReturned => {
                self.reply = None;
                self.stage = Stage::Complete;
            }
            Stage::ColorGate => {
                if flags & 1 != 0 {
                    self.call(
                        Stage::ColorBuffer,
                        Request::Buffer {
                            surface: self.surface,
                        },
                    )
                } else {
                    fields.buffers.color = 0;
                    fields.buffers.color_pitch = 0;
                    self.stage = Stage::DepthBufferGate
                }
            }
            Stage::ColorBufferReturned => {
                fields.buffers.color = self.reply.take().unwrap();
                self.call(
                    Stage::ColorPitch,
                    Request::Pitch {
                        surface: self.surface,
                    },
                );
            }
            Stage::ColorPitchReturned => {
                fields.buffers.color_pitch = self.reply.take().unwrap();
                self.stage = Stage::DepthBufferGate;
            }
            Stage::DepthBufferGate => {
                if flags & 2 != 0 && depth.is_some() {
                    self.depth_reader = depth;
                    self.call(
                        Stage::DepthBuffer,
                        Request::Buffer {
                            surface: depth.unwrap(),
                        },
                    )
                } else {
                    fields.buffers.depth = 0;
                    fields.buffers.depth_pitch = 0;
                    self.stage = Stage::Publish
                }
            }
            Stage::DepthBufferReturned => {
                fields.buffers.depth = self.reply.take().unwrap();
                self.call(
                    Stage::DepthPitch,
                    Request::Pitch {
                        surface: self
                            .depth_reader
                            .ok_or("target depth getter receiver absent")?,
                    },
                );
            }
            Stage::DepthPitchReturned => {
                fields.buffers.depth_pitch = self.reply.take().unwrap();
                self.stage = Stage::Publish;
            }
            Stage::Publish => {
                fields.active = 1;
                fields.phase = 3;
                fields.surface = self.surface;
                self.activated = true;
                self.stage = Stage::Complete;
            }
            Stage::Complete => return Ok(false),
            _ => return Err("target call stage lost its child".into()),
        }
        self.validate()?;
        Ok(true)
    }
}

///5dece2/5ded38 publish plane fields only after their backend succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plane {
    pub buffer: u32,
    pub half_pitch: u32,
    pub locks: u32,
    pub flags: u32,
}
impl Plane {
    pub fn needs_backend(flags: u32) -> bool {
        flags != 16 && flags & 0x7cc != 0
    }
    pub fn lock(&mut self, flags: u32, backend_result: u32, buffer: u32, pitch: u32) -> u8 {
        if self.publish_lock(flags,backend_result,buffer,pitch)==0{return 0;}
        self.finish_lock();1
    }
    /// Plane publication precedes the native5ded2e count instruction.
    pub fn publish_lock(&mut self, flags:u32, backend_result:u32, buffer:u32, pitch:u32)->u8{
        if Self::needs_backend(flags) {
            if backend_result as u8 == 0 {
                return 0;
            }
            self.buffer = buffer;
            self.half_pitch = pitch >> 1;
        } else if flags != 16 {
            self.buffer = 0;
            self.half_pitch = 0;
        }
        self.flags |= flags;
        1
    }
    pub fn finish_lock(&mut self){self.locks=self.locks.wrapping_add(1);}
    pub fn unlock(&mut self, flags: u32, backend_result: u32) -> u8 {
        if Self::needs_backend(flags) {
            if backend_result as u8 == 0 {
                return 0;
            }
            self.buffer = 0;
            self.half_pitch = 0;
        }
        self.flags &= !flags;
        self.locks = self.locks.wrapping_sub(1);
        1
    }
}
