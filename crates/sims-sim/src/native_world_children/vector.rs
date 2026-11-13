use super::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

pub const ELEMENTS: u32 = 4096;
pub const ELEMENT_BYTES: u32 = 4;
const BYTES: usize = (ELEMENTS * ELEMENT_BYTES) as usize;

pub trait Host {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub owner: Owner,
    pub backing: Option<Owner>,
}

impl State {
    fn validate(&self) -> Result<(), Stop> {
        self.owner.validate(12)?;
        if let Some(backing) = &self.backing {
            backing.validate(BYTES)?;
            if backing.token + BYTES as u64 > u64::from(u32::MAX) || self.owner.overlaps(backing) {
                return Err(Stop::Unsupported(
                    "element-vector allocation aliases its owner or exceeds native end-pointer width".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Initialize,
    Allocate,
    Accept,
    Fill,
    Publish,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    pub state: State,
    phase: Phase,
    pending: Option<Allocated>,
}

impl Construction {
    pub(crate) fn pending_allocation(&self) -> Option<&Allocated> {
        self.pending.as_ref()
    }

    pub fn new(owner: Owner) -> Result<Self, Stop> {
        owner.validate(12)?;
        Ok(Self {
            state: State {
                owner,
                backing: None,
            },
            phase: Phase::Initialize,
            pending: None,
        })
    }

    fn validate(&self) -> Result<(), Stop> {
        self.state.validate()?;
        let needs_backing = matches!(self.phase, Phase::Fill | Phase::Publish | Phase::Complete);
        if needs_backing != self.state.backing.is_some()
            || (self.phase == Phase::Accept) != self.pending.is_some()
        {
            return Err(Stop::Unsupported(
                "invalid retained element-vector stage ownership".into(),
            ));
        }
        if self.phase != Phase::Initialize
            && self.phase != Phase::Complete
            && self.state.owner.data != [0; 12]
        {
            return Err(Stop::Unsupported(
                "fresh element-vector header changed before publication".into(),
            ));
        }
        if matches!(self.phase, Phase::Publish | Phase::Complete)
            && self
                .state
                .backing
                .as_ref()
                .unwrap()
                .data
                .iter()
                .any(|byte| *byte != 0x7f)
        {
            return Err(Stop::Unsupported(
                "element-vector fill is incomplete before publication".into(),
            ));
        }
        if self.phase == Phase::Complete {
            let backing = self.state.backing.as_ref().unwrap();
            let beginning = backing.token as u32;
            let end = beginning + BYTES as u32;
            for (offset, expected) in [(0, beginning), (4, end), (8, end)] {
                if self.state.owner.data[offset..offset + 4] != expected.to_le_bytes() {
                    return Err(Stop::Unsupported(
                        "published element-vector bounds differ from its owned allocation".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Initialize => {
                for offset in [0, 4, 8] {
                    self.state.owner.write_word(offset, 0);
                }
                self.phase = Phase::Allocate;
            }
            Phase::Allocate => {
                self.pending = Some(host.allocate_vector_elements(BYTES as u32)?);
                self.phase = Phase::Accept;
            }
            Phase::Accept => {
                let backing =
                    match self.pending.as_ref().unwrap() {
                        Allocated::Null => return Err(Stop::Unavailable(
                            "native positive-size element-vector fill has no writable allocation"
                                .into(),
                        )),
                        Allocated::Bytes { token, data } => Owner {
                            token: *token,
                            data: data.clone(),
                        },
                    };
                let state = State {
                    owner: self.state.owner.clone(),
                    backing: Some(backing),
                };
                state.validate()?;
                self.state = state;
                self.pending = None;
                self.phase = Phase::Fill;
            }
            Phase::Fill => {
                self.state.backing.as_mut().unwrap().data.fill(0x7f);
                self.phase = Phase::Publish;
            }
            Phase::Publish => {
                let beginning = self.state.backing.as_ref().unwrap().token as u32;
                let end = beginning + BYTES as u32;
                self.state.owner.write_word(0, beginning);
                self.state.owner.write_word(4, end);
                self.state.owner.write_word(8, end);
                self.phase = Phase::Complete;
            }
            Phase::Complete => {}
        }
        Ok(self.phase == Phase::Complete)
    }
}

#[cfg(test)]
mod tests;
