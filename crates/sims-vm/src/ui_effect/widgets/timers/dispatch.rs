//! Original5da820 timer pass. Pointer publication, sorting, one timeGetTime
//! sample, deadline writes, membership queries and event1A retain native order.
//! The manager's list/indices/flags are borrowed shared state across children.
use super::{Registry, sort_entries};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub running: u8,         // manager+1e0; not an entry guard in this routine
    pub pointer_pending: u8, // manager+1e1
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    PointerPosition, //5d9a2f, two output words
    PublishPointer {
        message: u32,
        argument: u32,
        packed: u32,
    }, //5dad37
    Clock,           //timeGetTime
    Registered(u32), //5da49f
    Dispatch {
        owner: u32,
        message: u32,
        argument: u32,
    }, //owner virtual0
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Point([u32; 2]),
    Clock(u32),
    Registered(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    PointerResult,
    ClearPointer,
    Start,
    ClockResult,
    Next,
    Presence,
    Increment,
    Finish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pass {
    stage: Stage,
    pending: Option<Request>,
    answer: Reply,
    now: u32,
    /// Native EDI retains the chosen vector slot across5da49f. The backing
    /// storage must remain valid during that child, as in the original call.
    slot: Option<usize>,
}
impl Default for Pass {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
            answer: Reply::Done,
            now: 0,
            slot: None,
        }
    }
}
impl Pass {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        use Stage as S;
        let valid = match (self.stage, self.pending) {
            (_, None) => match self.stage {
                S::PointerResult => matches!(self.answer, Reply::Point(_)),
                S::ClockResult => matches!(self.answer, Reply::Clock(_)),
                S::Presence => matches!(self.answer, Reply::Registered(_)) && self.slot.is_some(),
                _ => true,
            },
            (S::PointerResult, Some(Request::PointerPosition))
            | (
                S::ClearPointer,
                Some(Request::PublishPointer {
                    message: 13,
                    argument: 0,
                    ..
                }),
            )
            | (S::ClockResult, Some(Request::Clock)) => true,
            (S::Presence, Some(Request::Registered(_)))
            | (S::Increment, Some(Request::Dispatch { message: 26, .. })) => self.slot.is_some(),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("UI timer pass lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let kind = matches!(
            (r, reply),
            (Request::PointerPosition, Reply::Point(_))
                | (
                    Request::PublishPointer { .. } | Request::Dispatch { .. },
                    Reply::Done
                )
                | (Request::Clock, Reply::Clock(_))
                | (Request::Registered(_), Reply::Registered(_))
        );
        if self.pending != Some(r) || !kind {
            return Err("UI timer pass reply differs".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    pub fn advance(
        &mut self,
        reg: &mut Registry,
        flags: &mut Flags,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        if reg.entries.len() > i32::MAX as usize {
            return Err("UI timer pass vector exceeds signed indices".into());
        }
        use Stage as S;
        let r = loop {
            match self.stage {
                S::Entry => {
                    self.stage = S::Start;
                    if flags.pointer_pending != 0 {
                        self.stage = S::PointerResult;
                        break Request::PointerPosition;
                    }
                }
                S::PointerResult => {
                    let Reply::Point([x, y]) = self.answer else {
                        unreachable!()
                    };
                    self.stage = S::ClearPointer;
                    break Request::PublishPointer {
                        message: 13,
                        argument: 0,
                        packed: x | y.wrapping_shl(16),
                    };
                }
                S::ClearPointer => {
                    flags.pointer_pending = 0;
                    self.stage = S::Start;
                }
                S::Start => {
                    flags.running = 1;
                    if reg.first < 0 && !reg.entries.is_empty() {
                        reg.first = 0;
                        reg.second = reg.entries.len() as i32;
                        if reg.entries.len() > 1 {
                            sort_entries(&mut reg.entries);
                        }
                    }
                    self.stage = S::ClockResult;
                    break Request::Clock;
                }
                S::ClockResult => {
                    let Reply::Clock(now) = self.answer else {
                        unreachable!()
                    };
                    self.now = now;
                    self.stage = S::Next;
                }
                S::Next => {
                    if reg.first < 0 || reg.first >= reg.second {
                        self.stage = S::Finish;
                        continue;
                    }
                    let slot = reg.first as usize;
                    let e = reg
                        .entries
                        .get_mut(slot)
                        .ok_or("UI timer traversal points outside its vector")?;
                    if (e.deadline as i32) > (self.now as i32) {
                        reg.first = -1;
                        reg.second = -1;
                        self.stage = S::Finish;
                        continue;
                    }
                    e.deadline = e.deadline.wrapping_add(e.interval);
                    self.slot = Some(slot);
                    self.stage = S::Presence;
                    break Request::Registered(e.owner);
                }
                S::Presence => {
                    let Reply::Registered(present) = self.answer else {
                        unreachable!()
                    };
                    let r = if present {
                        let e = reg
                            .entries
                            .get(self.slot.unwrap())
                            .ok_or("UI timer captured slot disappeared during membership lookup")?;
                        Some(Request::Dispatch {
                            owner: e.owner,
                            message: 26,
                            argument: e.argument,
                        })
                    } else {
                        None
                    };
                    self.stage = S::Increment;
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::Increment => {
                    reg.first = reg.first.wrapping_add(1);
                    self.slot = None;
                    self.stage = S::Next;
                }
                S::Finish => {
                    if reg.first >= reg.second {
                        reg.first = -1;
                        reg.second = -1;
                    }
                    flags.running = 0;
                    self.stage = S::Complete;
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
