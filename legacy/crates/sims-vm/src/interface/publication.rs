//! Original4ce390: poll the shared timers, publish world changes, then panels,
//! then replace the dirty mask. The two publications are owned child calls;
//! this kernel does not supply their implementations or bind a frontend frame.
use super::state::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// CPState+10, copied to+0c after both children. Not an accumulated mask.
    pub after_dirty: u32,
    /// CPState+14, written only after a nonempty publication returns.
    pub last: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    pub now: u32,
    /// Stable host identity corresponding to CPState+54. Read on entry and
    /// again after4ce44f. A callback may replace it or clear it.
    pub panel_owner: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///4ce44f receives the captured dirty mask, not a later callback's mask.
    World(u32),
    ///50a8ec reads the live shared CPState. None preserves a null receiver
    /// after a callback; the host must report that unsupported/invalid call,
    /// not silently skip it or acknowledge it as successful.
    Panels(Option<u64>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Panels,
    Finish,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    force: u32,
    stage: Stage,
    pending: Option<Request>,
}

impl Frame {
    pub fn new(force: u32) -> Self {
        Self {
            force,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (self.stage, self.pending) {
            (_, None) => true,
            (Stage::Panels, Some(Request::World(mask))) => mask != 0,
            (Stage::Finish, Some(Request::Panels(_))) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("interface publication lost its child".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("interface publication reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, state: &mut State, input: Inputs) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            Stage::Entry => {
                // Original early return precedes both timers and every write.
                if input.panel_owner.is_none() {
                    self.stage = Stage::Complete;
                    return Ok(None);
                }
                state.validate()?;
                if state.publication.is_none() || state.speed_timers.is_none() {
                    return Err("interface publication fields/timers are not bound".into());
                }
                let timers = state.speed_timers.as_mut().unwrap();
                timers.first.update(input.now);
                timers.second.update(input.now);
                if self.force as u8 != 0 {
                    // Forced frames update credit but do not consume it.
                    state.buy.dirty = u32::MAX;
                } else {
                    if timers.first.consume() {
                        state.buy.dirty |= 1;
                    }
                    if timers.second.consume() {
                        state.buy.dirty |= 0x6078000;
                    }
                }
                if state.buy.dirty == 0 {
                    self.stage = Stage::Complete;
                    return Ok(None);
                }
                self.stage = Stage::Panels;
                self.pending = Some(Request::World(state.buy.dirty));
            }
            Stage::Panels => {
                self.stage = Stage::Finish;
                // Do not repeat the entry gate, poll timers, or capture flags.
                self.pending = Some(Request::Panels(input.panel_owner));
            }
            Stage::Finish => {
                state.validate()?;
                let fields = state
                    .publication
                    .as_mut()
                    .ok_or("interface publication fields are not bound")?;
                state.buy.dirty = fields.after_dirty;
                fields.last = input.now;
                self.stage = Stage::Complete;
            }
            Stage::Complete => (),
        }
        Ok(self.pending)
    }
}
