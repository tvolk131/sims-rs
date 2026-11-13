//! Original4ea31c action-image collection caller. It uses one temporary record
//! for the active action and all queued actions, then destroys string/image in
//! order. Record construction, builder, copy and resource children are explicit.
use super::Record;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Person {
        index: u32,
    },
    Construct,
    Build {
        index: u32,
        person: u32,
        node: u32,
        current: u32,
        record: Record,
    },
    Append {
        record: Record,
    },
    QueueCount {
        person: u32,
    },
    QueueAction {
        person: u32,
        index: u32,
    },
    DestroyString {
        record: Record,
    },
    DestroyImage {
        record: Record,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Person(Option<u32>),
    Record(Record),
    Built { record: Record, success: bool },
    Count(i32),
    Action(u32),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Person,
    Construct,
    Active,
    AppendActive,
    Count,
    QueueAction,
    Queued,
    AppendQueued,
    String,
    Image,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    index: u32,
    person: Option<u32>,
    phase: Phase,
    temp: Option<Record>,
    count: i32,
    queued: u32,
    node: u32,
    pending: Option<Request>,
}
impl Frame {
    pub fn at_entry(index: u32) -> Self {
        Self {
            index,
            person: None,
            phase: Phase::Person,
            temp: None,
            count: 0,
            queued: 0,
            node: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn selected_index(&self)->u32 {self.index}
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn temporary(&self) -> Option<&Record> {
        self.temp.as_ref()
    }
    fn request(&self) -> Result<Option<Request>, String> {
        let person = || {
            self.person
                .filter(|p| *p != 0)
                .ok_or("action-image collection person absent")
        };
        let temp = || self.temp.ok_or("action-image collection temporary absent");
        Ok(Some(match self.phase {
            Phase::Person => Request::Person { index: self.index },
            Phase::Construct => Request::Construct,
            Phase::Active => Request::Build {
                index: self.index,
                person: person()?,
                node: person()?.wrapping_add(0xa48),
                current: 1,
                record: temp()?,
            },
            Phase::AppendActive | Phase::AppendQueued => Request::Append { record: temp()? },
            Phase::Count => Request::QueueCount { person: person()? },
            Phase::QueueAction => Request::QueueAction {
                person: person()?,
                index: self.queued,
            },
            Phase::Queued => Request::Build {
                index: self.index,
                person: person()?,
                node: self.node,
                current: 0,
                record: temp()?,
            },
            Phase::String => Request::DestroyString { record: temp()? },
            Phase::Image => Request::DestroyImage { record: temp()? },
            Phase::Complete => return Ok(None),
        }))
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.count > 4096 || self.queued > 4096 || self.person == Some(0) {
            return Err("action-image collection source extent differs".into());
        }
        if let Some(r) = &self.pending {
            if self.request()?.as_ref() != Some(r) {
                return Err("action-image collection child differs from cursor".into());
            }
        }
        if matches!(self.phase, Phase::Construct) && self.person.is_none() {
            return Err("collection constructor lost its person".into());
        }
        if matches!(
            self.phase,
            Phase::Active
                | Phase::AppendActive
                | Phase::Count
                | Phase::QueueAction
                | Phase::Queued
                | Phase::AppendQueued
                | Phase::String
                | Phase::Image
        ) {
            if self.person.is_none() || self.temp.is_none() {
                return Err("collection caller lost its live temporary/person".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_none() {
            self.pending = self.request()?;
        }
        Ok(self.pending.clone())
    }
    fn next_queued(&mut self) {
        self.queued = self.queued.wrapping_add(1);
        self.phase = if self.queued < (self.count as u32) {
            Phase::QueueAction
        } else {
            Phase::String
        };
    }
    pub fn acknowledge(
        &mut self,
        output: &mut Vec<Record>,
        request: Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(&request) {
            return Err("collection reply is not its pending child".into());
        }
        let valid = match (&request, &reply) {
            (Request::Person { .. }, Reply::Person(p)) => *p != Some(0),
            (
                Request::Construct | Request::Append { .. } | Request::DestroyImage { .. },
                Reply::Record(_),
            ) => true,
            (Request::Build { .. }, Reply::Built { .. }) => true,
            (Request::QueueCount { .. }, Reply::Count(n)) => *n <= 4096,
            (Request::QueueAction { .. }, Reply::Action(_))
            | (Request::DestroyString { .. }, Reply::Done) => true,
            _ => false,
        };
        if !valid {
            return Err("collection child reply type/extent differs".into());
        }
        if matches!(request, Request::Append { .. }) && output.len() >= 4096 {
            return Err("action-image output vector exceeds bounded profile".into());
        }
        match reply {
            Reply::Person(p) => {
                self.person = p;
                self.phase = if p.is_some() {
                    Phase::Construct
                } else {
                    Phase::Complete
                };
            }
            Reply::Record(r) => match self.phase {
                Phase::Construct => {
                    self.temp = Some(r);
                    self.phase = Phase::Active;
                }
                Phase::AppendActive => {
                    output.push(r);
                    self.phase = Phase::Count;
                }
                Phase::AppendQueued => {
                    output.push(r);
                    self.next_queued();
                }
                Phase::Image => {
                    self.temp = Some(r);
                    self.phase = Phase::Complete;
                }
                _ => unreachable!(),
            },
            Reply::Built { record, success } => {
                self.temp = Some(record);
                if self.phase == Phase::Active {
                    self.phase = if success {
                        Phase::AppendActive
                    } else {
                        Phase::Count
                    };
                } else if success {
                    self.temp.as_mut().unwrap()[9] = 0;
                    self.phase = Phase::AppendQueued;
                } else {
                    self.next_queued();
                }
            }
            Reply::Count(n) => {
                self.count = n;
                self.phase = if n > 0 {
                    Phase::QueueAction
                } else {
                    Phase::String
                };
            }
            Reply::Action(node) => {
                self.node = node;
                self.phase = Phase::Queued;
            }
            Reply::Done => self.phase = Phase::Image,
        }
        self.pending = None;
        self.validate()
    }
}
