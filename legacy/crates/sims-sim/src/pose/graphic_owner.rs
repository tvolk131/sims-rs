//! Ordinary object graphic ownership4376f6. Base post-construction431aaf calls
//! this before the factory date stamp and Reset/Init. Profile lookup precedes
//! allocation; the object receives its graphic before virtual owner Init.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub definition_flags: u8,
    pub resource: u64,
    pub graphic: Option<u64>,
    /// Object +180, not its selected/native flags word at +114.
    pub ready: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Profile { resource: u64 },
    Construct,
    Initialize { graphic: u64, owner: u64 },
}
pub enum Reply {
    Profile,
    Constructed(Option<u64>),
    /// The original ignores the virtual Init return value.
    Initialized(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Profile,
    Construct,
    Initialize,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    owner: u64,
    stage: Stage,
    constructed: Option<u64>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(owner: u64) -> Self {
        Self {
            owner,
            stage: Stage::Start,
            constructed: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn owner(&self) -> u64 {
        self.owner
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.owner == 0
            || self.constructed == Some(0)
            || self
                .pending
                .as_ref()
                .is_some_and(|r| match (self.stage, r) {
                    (Stage::Profile, Request::Profile { resource }) => *resource == 0,
                    (Stage::Construct, Request::Construct) => false,
                    (Stage::Initialize, Request::Initialize { graphic, owner }) => {
                        Some(*graphic) != self.constructed || *owner != self.owner
                    }
                    _ => true,
                })
        {
            return Err("invalid ordinary graphic owner continuation".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, state: &State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        if self.stage == Stage::Start {
            self.stage = if state.definition_flags & 1 == 0 {
                Stage::Done
            } else {
                Stage::Profile
            };
        }
        let request = match self.stage {
            Stage::Profile => {
                if state.resource == 0 {
                    return Err("ordinary graphic resource absent".into());
                }
                Request::Profile {
                    resource: state.resource,
                }
            }
            Stage::Construct => Request::Construct,
            Stage::Initialize => Request::Initialize {
                graphic: self
                    .constructed
                    .ok_or("native ordinary graphic allocation would dereference null")?,
                owner: self.owner,
            },
            Stage::Done => return Ok(None),
            Stage::Start => unreachable!(),
        };
        self.pending = Some(request);
        Ok(self.pending.clone())
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale ordinary graphic owner acknowledgment".into());
        }
        match (request, reply) {
            (Request::Profile { .. }, Reply::Profile) => self.stage = Stage::Construct,
            (Request::Construct, Reply::Constructed(graphic)) => {
                if graphic == Some(0) {
                    return Err("invalid constructed graphic identity".into());
                }
                self.constructed = graphic;
                state.graphic = graphic;
                self.stage = Stage::Initialize;
            }
            (Request::Initialize { .. }, Reply::Initialized(_)) => {
                state.ready = 1;
                self.stage = Stage::Done;
            }
            _ => return Err("mismatched ordinary graphic owner reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
