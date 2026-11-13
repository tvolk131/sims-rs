//! Original46d813/46d83a. The pause word belongs to the lot's global17;
//! elapsed counters belong to the application's scheduling owner. Notification
//! completion precedes the Resume timer reset, even if a listener repauses.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    /// Lot+b8,+bc. Preserve bit patterns; the enclosing clock uses signed tests.
    pub last: u32,
    pub elapsed: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Pause,
    Resume,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Broadcast,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Reset,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    command: Command,
    stage: Stage,
    pending: Option<Request>,
}
impl Change {
    pub fn new(command: Command) -> Self {
        Self {
            command,
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
        if (self.stage == Stage::Reset && self.command != Command::Resume)
            || (self.pending.is_some()
                && self.stage
                    != match self.command {
                        Command::Pause => Stage::Complete,
                        Command::Resume => Stage::Reset,
                    })
        {
            return Err("transport callback lost its owner".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("transport reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        paused: &mut i16,
        timing: &mut Timing,
        application_gate: i32,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            Stage::Entry => {
                let changed = match self.command {
                    Command::Pause => application_gate == 0 && *paused == 0,
                    Command::Resume => *paused != 0,
                };
                self.stage = Stage::Complete;
                if changed {
                    *paused = i16::from(self.command == Command::Pause);
                    if self.command == Command::Resume {
                        self.stage = Stage::Reset;
                    }
                    self.pending = Some(Request::Broadcast);
                }
            }
            Stage::Reset => {
                timing.elapsed = 0;
                timing.last = 0;
                self.stage = Stage::Complete;
            }
            Stage::Complete => (),
        }
        Ok(self.pending)
    }
}
