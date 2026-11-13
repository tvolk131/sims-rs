//! Ordinary definition StringSet cache457eec and ObjectStrings load455e97.
//! The cache belongs to a definition resource, not an individual Animator.
//! Person BodyStrings are a separate loader with different padding rules.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub kind: i16,
    pub id: u16,
    pub file: u64,
    pub cached: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Construct,
    Load { profile: u64, file: u64, id: u16 },
}
pub enum Reply {
    Constructed(Option<u64>),
    /// The original cache ignores the load's return value.
    Loaded(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Construct,
    Load,
    Finish,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    stage: Stage,
    constructed: Option<u64>,
    result: Option<u64>,
    pending: Option<Request>,
}
impl Default for Update {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            constructed: None,
            result: None,
            pending: None,
        }
    }
}
impl Update {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn result(&self) -> Result<Option<u64>, String> {
        if !self.complete() {
            return Err("ordinary profile lookup incomplete".into());
        }
        Ok(self.result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.constructed == Some(0)
            || self.result == Some(0)
            || self
                .pending
                .as_ref()
                .is_some_and(|r| match (self.stage, r) {
                    (Stage::Construct, Request::Construct) => false,
                    (Stage::Load, Request::Load { profile, .. }) => {
                        Some(*profile) != self.constructed
                    }
                    _ => true,
                })
        {
            return Err("invalid ordinary profile continuation".into());
        }
        Ok(())
    }
    /// The native unused argument is deliberately absent. A person returns
    /// null even if the definition has a stale cached profile pointer.
    pub fn advance(&mut self, state: &State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        if self.stage == Stage::Start {
            if state.kind == 2 {
                self.stage = Stage::Done;
            } else if state.cached.is_some() {
                if state.cached == Some(0) {
                    return Err("invalid cached ordinary profile identity".into());
                }
                self.result = state.cached;
                self.stage = Stage::Done;
            } else {
                self.stage = Stage::Construct;
            }
        }
        if self.stage == Stage::Finish {
            self.result = state.cached;
            self.stage = Stage::Done;
        }
        let request = match self.stage {
            Stage::Construct => Request::Construct,
            Stage::Load => Request::Load {
                profile: self
                    .constructed
                    .ok_or("native ordinary profile allocation would dereference null")?,
                file: state.file,
                id: state.id,
            },
            Stage::Done => return Ok(None),
            Stage::Start | Stage::Finish => unreachable!(),
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
            return Err("stale ordinary profile acknowledgment".into());
        }
        match (request, reply) {
            (Request::Construct, Reply::Constructed(id)) => {
                if id == Some(0) {
                    return Err("invalid constructed profile identity".into());
                }
                self.constructed = id;
                // Native publishes the pointer before virtual Load. Its null
                // allocation path stores null then dereferences it; diagnose.
                state.cached = id;
                self.stage = Stage::Load;
            }
            (Request::Load { .. }, Reply::Loaded(_)) => self.stage = Stage::Finish,
            _ => return Err("mismatched ordinary profile reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub file: u64,
    pub id: u16,
    pub tag: u32,
    pub flags: u8,
    pub language: u8,
}
pub trait Strings {
    /// 46feac(0,1), after 470256's configuration. Loading failure is a
    /// returned value; unavailable host services use Err and remain explicit.
    fn read(&mut self, metadata: &Metadata) -> Result<u32, String>;
    fn count(&mut self) -> Result<i32, String>;
    /// 470482(count+1,"",-1). Appends through the active language.
    fn append_empty(&mut self, index: i32) -> Result<(), String>;
    fn write(&mut self, file: u64, id: u16) -> Result<u32, String>;
}
/// Synchronous StringSet load protocol inside the outer cache Load service.
/// The resource backend owns string decoding/encoding and writes. A short
/// table requests writeback even if read returned failure; write status is
/// ignored. This must not mutate original assets or skip the write request.
pub fn load(
    metadata: &mut Metadata,
    file: u64,
    id: u16,
    host: &mut impl Strings,
) -> Result<u32, String> {
    *metadata = Metadata {
        file,
        id,
        tag: u32::from_be_bytes(*b"STR#"),
        flags: 0,
        language: 0,
    };
    let result = host.read(metadata)?;
    if host.count()? < 2 {
        loop {
            let next = host.count()?.wrapping_add(1);
            host.append_empty(next)?;
            if host.count()? >= 2 {
                break;
            }
        }
        host.write(file, id)?;
    }
    Ok(result)
}
