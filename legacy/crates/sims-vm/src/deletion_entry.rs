//! 440eeb's single-object entry, ending immediately before virtual Reset.
//! Each service finishes before the next live read. Multipart traversal and
//! destruction after Reset belong to the enclosing deletion continuation.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

pub trait View {
    fn flags(&self, object: ObjectId) -> Result<u32, String>;
    fn write_flags(&mut self, object: ObjectId, flags: u32) -> Result<(), String>;
    fn ui_owner(&self) -> Option<ObjectId>;
    fn selected(&self) -> Result<Option<ObjectId>, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Signal,
    ReleaseUi,
    ClearSelection,
    ReleaseIndicator,
    Invalidate,
    References,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Signal,
    Mark,
    Ui,
    Selection,
    Indicator,
    Invalidate,
    References,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    object: ObjectId,
    stage: Stage,
    pending: Option<Request>,
}
impl Entry {
    pub fn new(object: ObjectId) -> Self {
        Self {
            object,
            stage: Stage::Signal,
            pending: None,
        }
    }
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.object <= 0
            || self.pending.is_some_and(|r| {
                !matches!(
                    (self.stage, r),
                    (Stage::Signal, Request::Signal)
                        | (Stage::Ui, Request::ReleaseUi)
                        | (Stage::Selection, Request::ClearSelection)
                        | (Stage::Indicator, Request::ReleaseIndicator)
                        | (Stage::Invalidate, Request::Invalidate)
                        | (Stage::References, Request::References)
                )
            })
        {
            return Err("invalid deletion entry continuation".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, view: &mut impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                Stage::Signal => Request::Signal,
                Stage::Mark => {
                    view.write_flags(self.object, view.flags(self.object)? | 0x40)?;
                    self.stage = Stage::Ui;
                    continue;
                }
                Stage::Ui if view.ui_owner() == Some(self.object) => Request::ReleaseUi,
                Stage::Ui => {
                    self.stage = Stage::Selection;
                    continue;
                }
                Stage::Selection if view.selected()? == Some(self.object) => {
                    Request::ClearSelection
                }
                Stage::Selection => {
                    self.stage = Stage::Indicator;
                    continue;
                }
                Stage::Indicator => Request::ReleaseIndicator,
                Stage::Invalidate => Request::Invalidate,
                Stage::References => Request::References,
                Stage::Complete => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale deletion entry acknowledgment".into());
        }
        self.stage = match request {
            Request::Signal => Stage::Mark,
            Request::ReleaseUi => Stage::Selection,
            Request::ClearSelection => Stage::Indicator,
            Request::ReleaseIndicator => Stage::Invalidate,
            Request::Invalidate => Stage::References,
            Request::References => Stage::Complete,
        };
        self.pending = None;
        Ok(())
    }
}
