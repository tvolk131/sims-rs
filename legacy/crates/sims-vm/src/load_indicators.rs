//! Native 0x445e4a. Indicator factories run synchronously before the hidden
//! flag and registry cache are written. Attachment to selected/star Sims is
//! a later service, not part of this initializer.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

pub const GUIDS: [u32; 2] = [0x2f726fff, 0x0baa71c5];
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Indicators {
    pub primary: Option<ObjectId>,
    pub secondary: Option<ObjectId>,
}
pub trait Host {
    fn lookup(&mut self, guid: u32) -> Result<bool, String>;
    /// None includes allocation failure and a returned ID absent from registry.
    fn create(&mut self, guid: u32) -> Result<Option<ObjectId>, String>;
    /// OR object data word 8 with 0x4000, preserving every other bit.
    fn hide(&mut self, object: ObjectId) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Lookup(usize),
    Create(usize),
    Hide(ObjectId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Available(bool),
    Created(Option<ObjectId>),
    Done,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    #[default]
    Start,
    Lookup(usize),
    Create(usize),
    Hide(usize, ObjectId),
    Complete,
}
/// Saved445e4a caller. Lookup results are retained before either factory runs;
/// a callback changing the primary cache does not repeat the entry guard.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Initialization {
    stage: Stage,
    available: [Option<bool>; 2],
    pending: Option<Request>,
}
impl Initialization {
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn available(&self) -> [Option<bool>; 2] {
        self.available
    }
    pub fn validate(&self) -> Result<(), String> {
        let expected = match self.stage {
            Stage::Start | Stage::Complete => None,
            Stage::Lookup(i) if i < 2 => Some(Request::Lookup(i)),
            Stage::Create(i) if i < 2 => Some(Request::Create(i)),
            Stage::Hide(i, id) if i < 2 && id > 0 => Some(Request::Hide(id)),
            _ => return Err("invalid indicator initialization stage".into()),
        };
        if self.pending.is_some() && self.pending != expected {
            return Err("indicator initialization lost its callback".into());
        }
        let valid = match self.stage {
            Stage::Start | Stage::Lookup(0) => self.available == [None, None],
            Stage::Lookup(1) => self.available[0].is_some() && self.available[1].is_none(),
            Stage::Create(i) => {
                self.available.iter().all(Option::is_some)
                    && (self.pending.is_none() || self.available[i] == Some(true))
            }
            Stage::Hide(i, _) => {
                self.available.iter().all(Option::is_some) && self.available[i] == Some(true)
            }
            Stage::Complete => {
                self.available == [None, None] || self.available.iter().all(Option::is_some)
            }
            _ => false,
        };
        if !valid {
            return Err("indicator initialization resource order differs".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, indicators: &Indicators) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            self.stage = match self.stage {
                Stage::Start if indicators.primary.is_some() => Stage::Complete,
                Stage::Start => Stage::Lookup(0),
                Stage::Create(0) if self.available[0] == Some(false) => Stage::Create(1),
                Stage::Create(1) if self.available[1] == Some(false) => Stage::Complete,
                _ => break,
            };
        }
        self.pending = match self.stage {
            Stage::Lookup(i) => Some(Request::Lookup(i)),
            Stage::Create(i) => Some(Request::Create(i)),
            Stage::Hide(_, id) => Some(Request::Hide(id)),
            Stage::Complete => None,
            Stage::Start => unreachable!(),
        };
        Ok(self.pending)
    }
    pub fn acknowledge(
        &mut self,
        indicators: &mut Indicators,
        request: Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale indicator initialization reply".into());
        }
        match (self.stage, request, reply) {
            (Stage::Lookup(i), Request::Lookup(j), Reply::Available(value)) if i == j => {
                self.available[i] = Some(value);
                self.stage = if i == 0 {
                    Stage::Lookup(1)
                } else {
                    Stage::Create(0)
                };
            }
            (Stage::Create(i), Request::Create(j), Reply::Created(id))
                if i == j && id.is_none_or(|n| n > 0) =>
            {
                self.stage = if let Some(id) = id {
                    Stage::Hide(i, id)
                } else if i == 0 {
                    Stage::Create(1)
                } else {
                    Stage::Complete
                };
            }
            (Stage::Hide(i, id), Request::Hide(j), Reply::Done) if id == j => {
                if i == 0 {
                    indicators.primary = Some(id);
                } else {
                    indicators.secondary = Some(id);
                }
                self.stage = if i == 0 {
                    Stage::Create(1)
                } else {
                    Stage::Complete
                };
            }
            _ => return Err("indicator initialization reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
}
impl Indicators {
    pub fn initialize(&mut self, host: &mut impl Host) -> Result<(), String> {
        let mut call = Initialization::default();
        while let Some(request) = call.advance(self)? {
            let reply = match request {
                Request::Lookup(i) => Reply::Available(host.lookup(GUIDS[i])?),
                Request::Create(i) => Reply::Created(host.create(GUIDS[i])?),
                Request::Hide(id) => {
                    host.hide(id)?;
                    Reply::Done
                }
            };
            call.acknowledge(self, request, reply)?;
        }
        Ok(())
    }
}
