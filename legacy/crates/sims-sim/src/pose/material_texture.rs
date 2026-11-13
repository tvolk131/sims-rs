//! DDDMaterial::SetTexture (005d47f9), with its virtual calls exposed as
//! resumable services. The recovered assembly returns the publication HRESULT
//! even though the decompiler inferred void. Device creation, texture lookup
//! and destruction are separate operations; no replacement texture is invented.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// D3DMATERIAL at +04; word 18 is the texture handle at +4c.
    pub words: [u32; 20],
    /// Texture owner at +8c, distinct from its device handle.
    pub texture: Option<u64>,
    /// Material interface at +58. This operation requires a live interface.
    pub device: Option<u64>,
    /// Live texture +0c fields, owned by the backend. Callbacks can change them.
    pub handles: BTreeMap<u64, u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Acquire { texture: u64 },
    Release { texture: u64 },
    Publish { device: u64, words: [u32; 20] },
}

#[derive(Debug, Clone, Copy)]
pub enum Reply {
    /// AddRef/Release return values are not used by this routine.
    Done,
    Published(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Acquire,
    Release,
    Clear,
    Install,
    Publish,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attach {
    /// Captured argument in EBX, never reread from the owner's texture field.
    replacement: Option<u64>,
    stage: Stage,
    pending: Option<Request>,
    result: Option<u32>,
}

impl Attach {
    pub fn new(replacement: Option<u64>) -> Self {
        Self {
            replacement,
            stage: Stage::Acquire,
            pending: None,
            result: None,
        }
    }

    pub fn result(&self) -> Option<u32> {
        self.result
    }

    pub fn validate(&self) -> Result<(), String> {
        if (self.stage == Stage::Done) != self.result.is_some()
            || self.pending.as_ref().is_some_and(|request| match request {
                Request::Acquire { texture } => {
                    self.stage != Stage::Acquire || self.replacement != Some(*texture)
                }
                Request::Release { .. } => self.stage != Stage::Release,
                Request::Publish { .. } => self.stage != Stage::Publish,
            })
        {
            return Err("invalid texture attachment continuation".into());
        }
        Ok(())
    }

    pub fn advance(&mut self, state: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Acquire => match self.replacement {
                    Some(texture) => Request::Acquire { texture },
                    None => {
                        self.stage = Stage::Release;
                        continue;
                    }
                },
                // 5d480c reads the old owner AFTER the new texture's AddRef.
                Stage::Release => match state.texture {
                    Some(texture) => Request::Release { texture },
                    None => {
                        self.stage = Stage::Install;
                        continue;
                    }
                },
                Stage::Clear => {
                    // Release may have replaced this slot; the original still
                    // clears it and installs the captured replacement argument.
                    state.texture = None;
                    self.stage = Stage::Install;
                    continue;
                }
                Stage::Install => {
                    state.texture = self.replacement;
                    state.words[18] = match self.replacement {
                        Some(id) => *state
                            .handles
                            .get(&id)
                            .ok_or("attached texture has no live device handle")?,
                        None => 0,
                    };
                    self.stage = Stage::Publish;
                    continue;
                }
                Stage::Publish => Request::Publish {
                    device: state.device.ok_or("texture material interface absent")?,
                    words: state.words,
                },
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }

    pub fn acknowledge(&mut self, expected: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("stale texture attachment acknowledgment".into());
        }
        match (expected, reply) {
            (Request::Acquire { .. }, Reply::Done) => self.stage = Stage::Release,
            (Request::Release { .. }, Reply::Done) => self.stage = Stage::Clear,
            (Request::Publish { .. }, Reply::Published(result)) => {
                // No rollback, including when the publication reports failure.
                self.result = Some(result);
                self.stage = Stage::Done;
            }
            _ => return Err("mismatched texture attachment reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
