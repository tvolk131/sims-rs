//!43fc94 after its ordered object visits. Keep the initializer, enable write,
//! selected-indicator call and live person-vector star pass in native order.
use crate::{ObjectId, selection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Initialize,
    Selection(Option<ObjectId>),
    Star(ObjectId),
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    #[default]
    Initialize,
    Enable,
    Selection(Option<ObjectId>),
    Stars(usize),
    Complete,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tail {
    stage: Stage,
    pending: Option<Request>,
}
impl Tail {
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (self.pending, &self.stage) {
            (None, _) => true,
            (Some(Request::Initialize), Stage::Initialize) => true,
            (Some(Request::Selection(id)), Stage::Selection(saved)) => {
                id == *saved && id.is_none_or(|n| n > 0)
            }
            (Some(Request::Star(id)), Stage::Stars(_)) => id > 0,
            _ => false,
        };
        if !valid {
            return Err("post-load tail lost its callback".into());
        }
        if matches!(self.stage,Stage::Selection(Some(n)) if n<=0) {
            return Err("invalid post-load selected identity".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        enabled: &mut u8,
        view: &impl selection::View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        if self.stage == Stage::Enable {
            *enabled = 1;
            self.stage = Stage::Selection(selection::selected(view)?);
        }
        self.pending = match self.stage {
            Stage::Initialize => Some(Request::Initialize),
            Stage::Selection(id) => Some(Request::Selection(id)),
            Stage::Stars(index) => {
                let people = view.person_order();
                if index > people.len() {
                    return Err("post-load person vector shrank behind its iterator".into());
                }
                if let Some(&id) = people.get(index) {
                    Some(Request::Star(id))
                } else {
                    self.stage = Stage::Complete;
                    None
                }
            }
            Stage::Complete => None,
            Stage::Enable => unreachable!(),
        };
        self.validate()?;
        Ok(self.pending)
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale post-load tail reply".into());
        }
        self.stage = match self.stage {
            Stage::Initialize => Stage::Enable,
            Stage::Selection(_) => Stage::Stars(0),
            Stage::Stars(index) => {
                Stage::Stars(index.checked_add(1).ok_or("post-load iterator overflow")?)
            }
            _ => return Err("invalid post-load tail reply".into()),
        };
        self.pending = None;
        Ok(())
    }
}
