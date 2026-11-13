//! Original 46d553 object-registry pass. A completed paused pass still returns
//! false to 46d345, which performs a partial world update without advancing the
//! lot clock. Diagnostic Rust budgets suspend this call; they are not native
//! returns and must not restart its captured mode/registry-length reads.
//!
//! This kernel does not execute a VM thread, advance time, or handle object
//! errors itself. Its host must service the original object virtual+14 and
//! 439a6f boundaries. In particular, 4745b2 reads Object+28's error word; it is
//! not a wall-clock or instruction-budget test.
use serde::{Deserialize, Serialize};
pub mod clock;
pub mod registry;
pub mod storage;
pub mod transport;
pub mod visit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub paused: i16,
    pub mode: i16,
    pub lot_number: i16,
    pub tick: u32,
}
impl Inputs {
    /// Global words 17,19,10, tested as whole words for nonzero.
    pub fn partial(self) -> bool {
        self.paused != 0 || (self.mode != 0 && self.lot_number != 0)
    }
}

/// Borrow the registry; no world copy or snapshot of its entries is needed.
/// Indices are native one-based registry slots. Object handles are opaque and
/// must remain resolvable across their synchronous Tick/error callbacks.
pub trait Registry {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn object(&self, index: i32) -> Result<Option<u32>, String>;
    fn word(&self, index: i32) -> Result<u32, String>;
    fn set_word(&mut self, index: i32, word: u32) -> Result<(), String>;
    /// Unchanged 4745b2 getter, including after an object's Tick returns true.
    fn error(&self, object: u32) -> Result<i16, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Tick { object: u32, tick: u32 },
    ObjectError { object: u32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Only the low byte of the original Tick result controls the branch.
    Tick(u8),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Read,
    AfterTick,
    Increment,
    ReturnFalse,
    Complete(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pass {
    partial: bool,
    end: i32,
    object: Option<u32>,
    stage: Stage,
    pending: Option<Request>,
}
impl Pass {
    /// The cursor belongs to the lot (native+1a8), not this invocation. Native
    /// false returns retain it; exhausting the captured registry sets it to 0.
    pub fn begin(
        input: Inputs,
        registry: &impl Registry,
        cursor: &mut i32,
    ) -> Result<Self, String> {
        let end = i32::try_from(registry.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("scheduler registry is too large")?;
        if *cursor < 0 {
            return Err("negative scheduler cursor would read before registry".into());
        }
        if *cursor == 0 {
            *cursor = 1;
        }
        Ok(Self {
            partial: input.partial(),
            end,
            object: None,
            stage: Stage::Read,
            pending: None,
        })
    }
    pub fn result(&self) -> Option<bool> {
        match (self.stage, &self.pending) {
            (Stage::Complete(value), None) => Some(value),
            _ => None,
        }
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.end < 1
            || self.object == Some(0)
            || (self.partial && self.stage == Stage::Complete(true))
        {
            return Err("invalid scheduler extent or object handle".into());
        }
        let valid = match (&self.pending, self.stage) {
            (Some(Request::Tick { object, .. }), Stage::AfterTick)
            | (Some(Request::ObjectError { object }), Stage::ReturnFalse) => {
                self.object == Some(*object)
            }
            (None, Stage::AfterTick) => self.object.is_some(),
            (None, Stage::Read) => self.object.is_none(),
            (None, Stage::Increment | Stage::ReturnFalse | Stage::Complete(_)) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("scheduler lost its synchronous callback".into())
        }
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("scheduler reply differs from pending callback".into());
        }
        match (request, reply) {
            (Request::Tick { .. }, Reply::Tick(0)) => self.stage = Stage::ReturnFalse,
            (Request::Tick { .. }, Reply::Tick(_)) | (Request::ObjectError { .. }, Reply::Done) => {
                ()
            }
            _ => return Err("scheduler reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        registry: &mut impl Registry,
        cursor: &mut i32,
        input: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        let request = loop {
            match self.stage {
                Stage::Read => {
                    if *cursor >= self.end {
                        *cursor = 0;
                        self.stage = Stage::Complete(!self.partial);
                        return Ok(None);
                    }
                    if *cursor < 1 {
                        return Err("scheduler cursor would read before registry".into());
                    }
                    let Some(object) = registry.object(*cursor)? else {
                        self.stage = Stage::Increment;
                        continue;
                    };
                    if object == 0 {
                        return Err("null scheduler object must use None".into());
                    }
                    if registry.word(*cursor)? & (1 << 18) != 0 {
                        if registry.error(object)? != 0 {
                            self.object = Some(object);
                            self.stage = Stage::ReturnFalse;
                            break Request::ObjectError { object };
                        }
                        //4457ef(slot,4,0), before rereading enable/paused bits.
                        let word = registry.word(*cursor)? & !(1 << 18);
                        registry.set_word(*cursor, word)?;
                    }
                    let word = registry.word(*cursor)?;
                    if word & (1 << 16) == 0 || (self.partial && word & (1 << 17) == 0) {
                        self.stage = Stage::Increment;
                        continue;
                    }
                    let countdown = word as i16;
                    if countdown > 0 {
                        //431dc9 replaces the low half, preserving all flags.
                        registry.set_word(*cursor, (word & 0xffff0000) | (countdown - 1) as u32)?;
                        self.stage = Stage::Increment;
                        continue;
                    }
                    self.object = Some(object);
                    self.stage = Stage::AfterTick;
                    break Request::Tick {
                        object,
                        tick: input.tick,
                    };
                }
                Stage::AfterTick => {
                    //Use the retained object even if its callback changed the
                    //registry slot. Do not inspect an error after a false Tick.
                    let object = self.object.expect("validated scheduler object");
                    if registry.error(object)? != 0 {
                        self.stage = Stage::ReturnFalse;
                        break Request::ObjectError { object };
                    }
                    self.stage = Stage::Increment;
                }
                Stage::Increment => {
                    *cursor = cursor.checked_add(1).ok_or("scheduler cursor overflow")?;
                    self.object = None;
                    self.stage = Stage::Read;
                }
                Stage::ReturnFalse => {
                    self.stage = Stage::Complete(false);
                    return Ok(None);
                }
                Stage::Complete(_) => return Ok(None),
            }
        };
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}
