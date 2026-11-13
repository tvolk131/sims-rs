//!4226bc: request a facing through the cursor's actual virtual callbacks.
//!The point is captured before detach; the level and represented object are
//!reread after callbacks. Eight attempts is a native bound, not an error.
use serde::{Deserialize, Serialize};

///422776 uses camera quarters, not a rotation of the object's current facing.
pub fn camera_facing(quarter: i32) -> i32 {
    match quarter {
        0 => 2,
        1 => 0,
        2 => 6,
        3 => 4,
        _ => 0,
    }
}
pub trait View {
    fn has_furniture(&self) -> Result<bool, String>;
    fn facing(&self, furniture: bool) -> Result<i16, String>;
    fn position(&self) -> Result<[i32; 2], String>;
    fn level(&self) -> Result<i32, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Detach,
    Rotate,
    Check { position: [i32; 2], level: i32 },
    Place { position: [i32; 2], level: i32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Checked { accepted: bool, position: [i32; 2] },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Detach,
    Turn,
    Check,
    Place,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    direction: i32,
    phase: Phase,
    furniture: bool,
    position: Option<[i32; 2]>,
    attempts: u8,
    pending: Option<Request>,
}
impl Call {
    pub fn new(direction: i32) -> Self {
        Self {
            direction,
            phase: Phase::Entry,
            furniture: false,
            position: None,
            attempts: 0,
            pending: None,
        }
    }
    pub fn direction(&self) -> i32 {
        self.direction
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.attempts > 8
            || self.position.is_some() && !self.furniture
            || matches!(self.phase, Phase::Detach | Phase::Check | Phase::Place)
                && (!self.furniture || self.position.is_none())
            || self.pending.is_some_and(|r| {
                !matches!(
                    (self.phase, r),
                    (Phase::Detach, Request::Detach)
                        | (Phase::Turn, Request::Rotate)
                        | (Phase::Check, Request::Check { .. })
                        | (Phase::Place, Request::Place { .. })
                )
            })
            || self.pending.is_some_and(|r| match r {
                Request::Check { position, .. } | Request::Place { position, .. } => {
                    Some(position) != self.position
                }
                Request::Rotate => self.attempts == 8,
                Request::Detach => false,
            })
        {
            return Err("invalid cursor orientation continuation".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("cursor orientation reply has no caller".into());
        }
        let phase = match (request, reply) {
            (Request::Detach, Reply::Done) => Phase::Turn,
            (Request::Rotate, Reply::Done) => {
                self.attempts += 1;
                Phase::Turn
            }
            (Request::Check { .. }, Reply::Checked { accepted, position }) => {
                self.position = Some(position);
                if accepted {
                    Phase::Place
                } else {
                    Phase::Complete
                }
            }
            (Request::Place { .. }, Reply::Done) => Phase::Complete,
            _ => return Err("cursor orientation reply type differs".into()),
        };
        self.pending = None;
        self.phase = phase;
        Ok(())
    }
    pub fn advance(&mut self, view: &impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.phase {
                Phase::Entry => {
                    self.furniture = view.has_furniture()?;
                    if i32::from(view.facing(self.furniture)?) == self.direction {
                        self.phase = Phase::Complete;
                    } else if self.furniture {
                        self.position = Some(view.position()?);
                        self.phase = Phase::Detach;
                    } else {
                        self.phase = Phase::Turn;
                    }
                    continue;
                }
                Phase::Detach => Request::Detach,
                Phase::Turn => {
                    //After the eighth callback native code proceeds without
                    //another facing read, including an unreachable direction.
                    if self.attempts == 8
                        || i32::from(view.facing(self.furniture)?) == self.direction
                    {
                        self.phase = if self.furniture {
                            Phase::Check
                        } else {
                            Phase::Complete
                        };
                        continue;
                    }
                    Request::Rotate
                }
                Phase::Check => Request::Check {
                    position: self.position.ok_or("orientation point absent")?,
                    level: view.level()?,
                },
                Phase::Place => Request::Place {
                    position: self.position.ok_or("orientation point absent")?,
                    level: view.level()?,
                },
                Phase::Complete => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(Some(request));
        }
    }
}
