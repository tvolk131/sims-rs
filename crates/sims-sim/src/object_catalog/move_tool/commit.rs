//!486ada: release the held furniture, append its undo command, release the
//!tool's command reference, and clear catalog selection. Accounting and cursor
//!destruction belong to the mouse-up caller, not this child.
use super::cleanup;
use serde::{Deserialize, Serialize};

pub trait View {
    fn underlying(&self, cursor: u32) -> Result<Option<u32>, String>;
    fn facing(&self, furniture: u32) -> Result<i16, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ReleaseCursor(u32),
    Shared(cleanup::Request),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Released(bool),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    Released,
    DropReference,
    ClearPayload,
    Selection,
    Selected,
    Tail,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    phase: Phase,
    pending: Option<Request>,
    released: bool,
    result: bool,
}
impl Default for Call {
    fn default() -> Self {
        Self {
            phase: Phase::Start,
            pending: None,
            released: false,
            result: false,
        }
    }
}
impl Call {
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn outcome(&self) -> Option<bool> {
        (self.phase == Phase::Complete && self.pending.is_none()).then_some(self.result)
    }
    pub fn awaiting_payload_clear(&self) -> bool {
        self.phase == Phase::ClearPayload
    }
    pub fn validate(&self) -> Result<(), String> {
        use cleanup::Request as R;
        let valid = match self.pending {
            None => true,
            Some(Request::ReleaseCursor(h)) => self.phase == Phase::Released && h != 0,
            Some(Request::Shared(R::Append { clipboard, payload })) => {
                self.phase == Phase::DropReference && clipboard != 0 && payload != 0
            }
            Some(Request::Shared(R::ReleasePayload(h))) => {
                self.phase == Phase::ClearPayload && h != 0
            }
            Some(Request::Shared(R::ClearCatalogSelection)) => self.phase == Phase::Selected,
            Some(Request::Shared(R::Cursor)) => self.phase == Phase::Complete,
            _ => false,
        };
        if !valid
            || self.result
                && (!self.released || !matches!(self.phase, Phase::Tail | Phase::Complete))
            || matches!(
                self.phase,
                Phase::DropReference | Phase::ClearPayload | Phase::Selection | Phase::Selected
            ) && !self.released
        {
            return Err("invalid Move commit continuation".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Move commit reply lost its caller".into());
        }
        match (request, reply) {
            (Request::ReleaseCursor(_), Reply::Released(value)) => self.released = value,
            (Request::Shared(_), Reply::Done) => (),
            _ => return Err("Move commit reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &mut cleanup::State,
        direction: &mut i32,
        view: &impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        state.validate()?;
        if self.pending.is_some() || self.outcome().is_some() {
            return Ok(self.pending);
        }
        use cleanup::Request as R;
        let request = loop {
            match self.phase {
                Phase::Start => {
                    let held = state.held;
                    let underlying = held.map(|h| view.underlying(h)).transpose()?.flatten();
                    self.phase = Phase::Tail;
                    if let Some(h) = underlying {
                        *direction = i32::from(view.facing(h)?);
                        self.phase = Phase::Released;
                        break Request::ReleaseCursor(held.unwrap());
                    }
                }
                Phase::Released => {
                    self.phase = Phase::Tail;
                    if self.released {
                        self.phase = Phase::Selection;
                        if let Some(payload) = state.payload {
                            self.phase = Phase::DropReference;
                            break Request::Shared(R::Append {
                                clipboard: state.clipboard,
                                payload,
                            });
                        }
                    }
                }
                Phase::DropReference => {
                    //485a19 rereads +30 after493259 returns.
                    let payload = state
                        .payload
                        .ok_or("Move commit payload vanished before reference release")?;
                    self.phase = Phase::ClearPayload;
                    break Request::Shared(R::ReleasePayload(payload));
                }
                Phase::ClearPayload => {
                    state.payload = None;
                    self.phase = Phase::Selection;
                }
                Phase::Selection => {
                    self.phase = Phase::Selected;
                    break Request::Shared(R::ClearCatalogSelection);
                }
                Phase::Selected => {
                    self.result = true;
                    self.phase = Phase::Tail;
                }
                Phase::Tail => {
                    self.phase = Phase::Complete;
                    break Request::Shared(R::Cursor);
                }
                Phase::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
