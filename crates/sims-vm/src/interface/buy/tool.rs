//! 4cf360 selects the Buy tool. The selected definition's GUID getter and
//! object-tool activation remain owned children; a selected catalog row is
//! not an object instance or a GUID.
use crate::interface::state::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Clear,
    Select { index: i32, argument: u32 },
    Guid(u32),
    SelectDefinition(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Guid(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Guid,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    value: i32,
    stage: Stage,
    pending: Option<Request>,
    guid: Option<u32>,
}
impl Selection {
    pub fn selects(&self, value: i32) -> bool {
        self.value == value
    }
    pub fn new(value: i32) -> Self {
        Self {
            value,
            stage: Stage::Entry,
            pending: None,
            guid: None,
        }
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (self.stage, self.pending) {
            (Stage::Entry, None) => self.guid.is_none(),
            (Stage::Guid, Some(Request::Guid(id))) => {
                self.value == 3 && id != 0 && self.guid.is_none()
            }
            (Stage::Guid, None) => self.value == 3 && self.guid.is_some(),
            (Stage::Complete, Some(Request::Clear)) => {
                matches!(self.value, 1 | 2) && self.guid.is_none()
            }
            (
                Stage::Complete,
                Some(Request::Select {
                    index: 7,
                    argument: u32::MAX,
                }),
            ) => matches!(self.value, 0 | 3) && self.guid.is_none(),
            (Stage::Complete, Some(Request::SelectDefinition(guid))) => {
                self.value == 3 && self.guid == Some(guid)
            }
            (Stage::Complete, None) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("Buy tool selection lost its child".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Buy tool reply differs".into());
        }
        match (request, reply) {
            (Request::Guid(_), Reply::Guid(guid)) => self.guid = Some(guid),
            (
                Request::Clear | Request::Select { .. } | Request::SelectDefinition(_),
                Reply::Done,
            ) => (),
            _ => return Err("Buy tool reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, state: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        self.pending = match self.stage {
            Stage::Entry => {
                // Unconditional, including unknown selectors and repeated input.
                state.buy_tool = self.value;
                self.stage = Stage::Complete;
                match self.value {
                    1 | 2 => Some(Request::Clear),
                    3 if state.buy.selected_object != 0 => {
                        self.stage = Stage::Guid;
                        Some(Request::Guid(state.buy.selected_object))
                    }
                    0 | 3 => Some(Request::Select {
                        index: 7,
                        argument: u32::MAX,
                    }),
                    _ => None,
                }
            }
            Stage::Guid => {
                self.stage = Stage::Complete;
                Some(Request::SelectDefinition(
                    self.guid.expect("validated GUID reply"),
                ))
            }
            Stage::Complete => None,
        };
        Ok(self.pending)
    }
}
