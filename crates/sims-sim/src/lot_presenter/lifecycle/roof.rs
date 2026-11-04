//!4b1459: roof byte, invalidate, dirty byte, then the live optional CPState.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    InvalidateView,
    UiFlags(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Dirty,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    value: u8,
    force: u8,
    stage: Stage,
    pending: Option<Request>,
}
impl Cursor {
    pub fn new(value: u32, force: u32) -> Self {
        Self {
            value: value as u8,
            force: force as u8,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            (self.stage, self.pending),
            (_, None)
                | (Stage::Dirty, Some(Request::InvalidateView))
                | (Stage::Complete, Some(Request::UiFlags(0x80)))
        ) {
            return Err("roof callback lost its native stage".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        roof: &mut u8,
        dirty: &mut u8,
        controls_present: bool,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            Stage::Entry if *roof == self.value && self.force == 0 => self.stage = Stage::Complete,
            Stage::Entry => {
                *roof = self.value;
                self.stage = Stage::Dirty;
                self.pending = Some(Request::InvalidateView);
            }
            Stage::Dirty => {
                *dirty = 1;
                self.stage = Stage::Complete;
                if controls_present {
                    self.pending = Some(Request::UiFlags(0x80));
                }
            }
            Stage::Complete => (),
        }
        Ok(self.pending)
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("roof callback reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
}
