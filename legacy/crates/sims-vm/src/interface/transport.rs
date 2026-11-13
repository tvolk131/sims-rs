//! CPState4cee4a pause setter. This runs BEFORE the later4ce44f dirty pass
//! publishes pause to the lot.4b1bda refreshes people, not audio.
use super::state::State;
use serde::{Deserialize, Serialize};

pub const DIRTY: u32 = 0x800040;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Full call argument is retained;4d31b7 reads its low byte.
    WindowPause(u32),
    RefreshPeople,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Refresh,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPause {
    value: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl SetPause {
    pub fn new(value: u32) -> Self {
        Self {
            value,
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
            (Stage::Refresh, Some(Request::WindowPause(v))) => v == self.value,
            (Stage::Complete, Some(Request::RefreshPeople)) => self.value as u8 != 0,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("pause setter lost its callback".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("pause display reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, state: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        state.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            Stage::Entry => {
                state.mark_dirty(DIRTY);
                state.paused = self.value as u8;
                if state.mode == 2 {
                    self.stage = Stage::Refresh;
                    self.pending = Some(Request::WindowPause(self.value));
                } else {
                    self.stage = Stage::Complete;
                }
            }
            Stage::Refresh => {
                self.stage = Stage::Complete;
                // Tests the retained argument, not CPState fields possibly
                // changed by the window callback, and does not retest mode.
                if self.value as u8 != 0 {
                    self.pending = Some(Request::RefreshPeople);
                }
            }
            Stage::Complete => (),
        }
        Ok(self.pending)
    }
}

///4ce44f's mask800040 branches, with the original entry gates. Other dirty
/// branches and4ce390's enclosing publication are separate callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub menu_present: bool,
    pub menu_active: u8,
    pub closing: u8,
    pub lot_speed: i16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateRequest {
    SetSpeed(i32),
    Pause,
    Resume,
    PauseAudio,
    ResumeAudio,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum UpdateStage {
    Entry,
    Speed,
    Transport,
    PauseAudio,
    ResumeAudio,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    mask: u32,
    stage: UpdateStage,
    pending: Option<UpdateRequest>,
}
impl Update {
    pub fn new(mask: u32) -> Result<Self, String> {
        if mask & !DIRTY != 0 {
            return Err("unsupported transport publication mask".into());
        }
        Ok(Self {
            mask,
            stage: UpdateStage::Entry,
            pending: None,
        })
    }
    pub fn complete(&self) -> bool {
        self.stage == UpdateStage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<UpdateRequest> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        use UpdateRequest as R;
        use UpdateStage as S;
        let valid = match (self.stage, self.pending) {
            (_, None) => true,
            (S::Transport, Some(R::SetSpeed(_))) => self.mask & 0x40 != 0,
            (S::PauseAudio, Some(R::Pause))
            | (S::ResumeAudio, Some(R::Resume))
            | (S::Complete, Some(R::PauseAudio | R::ResumeAudio)) => self.mask & 0x800000 != 0,
            _ => false,
        };
        if self.mask & !DIRTY == 0 && valid {
            Ok(())
        } else {
            Err("transport publication lost its callback".into())
        }
    }
    pub fn acknowledge(&mut self, request: UpdateRequest) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("transport publication reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &State,
        lot_mode: &mut i16,
        input: Inputs,
    ) -> Result<Option<UpdateRequest>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use UpdateRequest as R;
        use UpdateStage as S;
        let request = loop {
            match self.stage {
                S::Entry => {
                    self.stage = S::Complete;
                    if input.menu_present && input.menu_active == 0 || input.closing != 0 {
                        return Ok(None);
                    }
                    if self.mask & 0x40 != 0 {
                        match state.mode {
                            0 => *lot_mode = 1,
                            1 => *lot_mode = 2,
                            2 => *lot_mode = 0,
                            3 => *lot_mode = 3,
                            4 => *lot_mode = 4,
                            _ => (),
                        }
                    }
                    self.stage = S::Speed;
                }
                S::Speed => {
                    self.stage = S::Transport;
                    if self.mask & 0x40 != 0 && state.speed != i32::from(input.lot_speed) {
                        break R::SetSpeed(state.speed);
                    }
                }
                S::Transport => {
                    self.stage = S::Complete;
                    if self.mask & 0x800000 != 0 && state.mode == 2 {
                        if state.paused == 0 {
                            self.stage = S::ResumeAudio;
                            break R::Resume;
                        }
                        self.stage = S::PauseAudio;
                        break R::Pause;
                    }
                }
                S::PauseAudio => {
                    self.stage = S::Complete;
                    break R::PauseAudio;
                }
                S::ResumeAudio => {
                    self.stage = S::Complete;
                    break R::ResumeAudio;
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
