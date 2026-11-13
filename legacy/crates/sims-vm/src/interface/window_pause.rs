//!4d31b7 window pause presentation. This does not write CPState+40 or the
//!simulator pause word. Timer and indicator calls preserve native ordering.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub paused: u8,
    pub timer_active: u8,
    pub interval: u32,
    pub indicator: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Unregister,
    Register(u32),
    Visible { indicator: u32, show: bool },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Clear,
    Write,
    Registered,
    Indicator,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    paused: u8,
    overlay: u8,
    stage: Stage,
    pending: Option<Request>,
}
impl Change {
    pub fn matches(&self, paused: u8, overlay: u8) -> bool {
        self.paused == paused && self.overlay == overlay
    }
    pub fn new(paused: u8, overlay: u8) -> Self {
        Self {
            paused,
            overlay,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (&self.pending, self.stage) {
            (None, _) => true,
            (Some(Request::Unregister), Stage::Clear) => true,
            (Some(Request::Register(_)), Stage::Registered) => {
                self.paused != 0 && self.overlay != 0
            }
            (Some(Request::Visible { indicator, .. }), Stage::Complete) => *indicator != 0,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("window pause lost its callback".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(r) {
            return Err("window pause reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        let r = loop {
            match self.stage {
                Stage::Entry => {
                    if fields.timer_active != 0 {
                        self.stage = Stage::Clear;
                        break Request::Unregister;
                    }
                    self.stage = Stage::Write;
                }
                Stage::Clear => {
                    fields.timer_active = 0;
                    self.stage = Stage::Write;
                }
                Stage::Write => {
                    fields.paused = self.paused;
                    self.stage = Stage::Indicator;
                    if self.paused != 0 && self.overlay != 0 {
                        self.stage = Stage::Registered;
                        break Request::Register(fields.interval);
                    }
                }
                Stage::Registered => {
                    fields.timer_active = 1;
                    self.stage = Stage::Indicator;
                }
                Stage::Indicator => {
                    self.stage = Stage::Complete;
                    if let Some(indicator) = fields.indicator {
                        if indicator == 0 {
                            return Err("window pause indicator must use None for null".into());
                        }
                        break Request::Visible {
                            indicator,
                            show: fields.paused != 0,
                        };
                    }
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
