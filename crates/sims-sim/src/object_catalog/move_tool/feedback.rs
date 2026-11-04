//!492a72/492ad4/492b05. Unsigned strict400ms gate; reread the application
//!clock after the synchronous sound child, then store that value. No VM tick.
use super::sell::Feedback;
use serde::{Deserialize, Serialize};

impl Feedback {
    pub fn sound(self) -> &'static str {
        match self {
            Self::Blocked => "UI_error",
            Self::MoneyBack => "UI_moneyback",
            Self::Delete => "UI_archtool_delete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Sound,
    Store,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    kind: Feedback,
    phase: Phase,
    pending: Option<Feedback>,
}
impl Call {
    pub fn new(kind: Feedback) -> Self {
        Self {
            kind,
            phase: Phase::Entry,
            pending: None,
        }
    }
    pub fn kind(&self) -> Feedback {
        self.kind
    }
    pub fn request(&self) -> Option<Feedback> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if (self.phase == Phase::Sound) != self.pending.is_some()
            || self.pending.is_some_and(|k| k != self.kind)
        {
            Err("invalid tool feedback continuation".into())
        } else {
            Ok(())
        }
    }
    ///57a54c(name,-4) has returned; it does not own the subsequent time store.
    pub fn acknowledge(&mut self, kind: Feedback) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(kind) {
            return Err("tool feedback sound caller differs".into());
        }
        self.pending = None;
        self.phase = Phase::Store;
        Ok(())
    }
    pub fn advance(&mut self, last: &mut u32, now: u32) -> Result<Option<Feedback>, String> {
        self.validate()?;
        match self.phase {
            Phase::Entry if last.wrapping_add(400) < now => {
                self.phase = Phase::Sound;
                self.pending = Some(self.kind);
            }
            Phase::Entry => self.phase = Phase::Complete,
            Phase::Store => {
                *last = now;
                self.phase = Phase::Complete;
            }
            Phase::Sound | Phase::Complete => (),
        }
        Ok(self.pending)
    }
}
