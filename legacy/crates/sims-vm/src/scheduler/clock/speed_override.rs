//! Original46d790/46d7db. Both execute46d697 synchronously, then overwrite
//! global18 and broadcast109. The saved prior speed is a caller local, not a
//! value reloaded after a C2 listener changes global16.
use super::{Fields, Speed};
use crate::scheduler::transport::Timing;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Boost(i32),
    Restore,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub event: u32,
    pub argument: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Speed,
    AfterSpeed,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    command: Command,
    stage: Stage,
    previous: i16,
    speed: Option<Speed>,
    pending: Option<Notice>,
}
impl Change {
    pub fn new(command: Command) -> Self {
        Self {
            command,
            stage: Stage::Entry,
            previous: 0,
            speed: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn command(&self) -> Command {
        self.command
    }
    pub fn request(&self) -> Option<Notice> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(speed) = &self.speed {
            speed.validate()?;
        }
        if (self.stage == Stage::Speed) != self.speed.is_some() {
            return Err("speed override lost child".into());
        }
        if self.speed.as_ref().is_some_and(|speed| speed.pending)
            != (self.pending
                == Some(Notice {
                    event: 0xc2,
                    argument: 0,
                }))
        {
            return Err("speed override lost its nested notification".into());
        }
        if let Some(n) = self.pending {
            let valid = if self.stage == Stage::Speed {
                n == (Notice {
                    event: 0xc2,
                    argument: 0,
                }) && self.speed.as_ref().unwrap().pending
            } else {
                self.stage == Stage::Complete
                    && n == (Notice {
                        event: 0x109,
                        argument: u32::from(matches!(self.command, Command::Boost(_))),
                    })
            };
            if !valid {
                return Err("speed override notification lost caller".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, notice: Notice) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(notice) {
            return Err("speed override reply differs".into());
        }
        if self.stage == Stage::Speed {
            self.speed.as_mut().unwrap().acknowledge()?;
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        speed: &mut i16,
        saved_speed: &mut i16,
    ) -> Result<Option<Notice>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                Stage::Entry => {
                    let argument = match self.command {
                        Command::Boost(value) if *saved_speed < 0 => {
                            self.previous = *speed;
                            Some(value)
                        }
                        Command::Restore if *saved_speed >= 0 => Some(i32::from(*saved_speed)),
                        _ => None,
                    };
                    if let Some(value) = argument {
                        fields.timing = Timing::default();
                        self.speed = Some(Speed::new(value));
                        self.stage = Stage::Speed;
                    } else {
                        self.stage = Stage::Complete;
                    }
                }
                Stage::Speed => {
                    if self
                        .speed
                        .as_mut()
                        .unwrap()
                        .advance(fields, speed, saved_speed)?
                    {
                        self.pending = Some(Notice {
                            event: 0xc2,
                            argument: 0,
                        });
                    } else {
                        self.speed = None;
                        self.stage = Stage::AfterSpeed;
                    }
                }
                Stage::AfterSpeed => {
                    *saved_speed = if matches!(self.command, Command::Boost(_)) {
                        self.previous
                    } else {
                        -1
                    };
                    self.pending = Some(Notice {
                        event: 0x109,
                        argument: u32::from(matches!(self.command, Command::Boost(_))),
                    });
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
            if self.pending.is_some() {
                return Ok(self.pending);
            }
        }
    }
}
