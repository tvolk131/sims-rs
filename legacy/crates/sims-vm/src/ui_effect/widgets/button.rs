//! cTSWinBtn5e480a changes the state at+e0, not the enable byte at+168.
//! Callback, self invalidation and both parent lookups retain native order.
use serde::{Deserialize, Serialize};

/// 5e443a's distinct field+e4 setter. It invalidates while the old value is
/// visible, then unconditionally writes the argument after the callback.
/// This is not5e480a's field+e0 setter and does not emit Changed/parent calls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuxiliarySetter {
    value: i32,
    stage: AuxiliaryStage,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum AuxiliaryStage {
    Entry,
    Invalidate,
    Write,
    Complete,
}
impl AuxiliarySetter {
    pub fn matches(&self, value: i32) -> bool {
        self.value == value
    }
    pub fn new(value: i32) -> Self {
        Self {
            value,
            stage: AuxiliaryStage::Entry,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == AuxiliaryStage::Complete
    }
    /// Some(Invalidate) requires the real widget invalidation before returning.
    pub fn advance(&mut self, state: &mut i32) -> Option<Request> {
        if self.stage == AuxiliaryStage::Entry {
            self.stage = if *state == self.value {
                AuxiliaryStage::Write
            } else {
                AuxiliaryStage::Invalidate
            };
        }
        if self.stage == AuxiliaryStage::Invalidate {
            return Some(Request::Invalidate);
        }
        if self.stage == AuxiliaryStage::Write {
            *state = self.value;
            self.stage = AuxiliaryStage::Complete;
        }
        None
    }
    pub fn acknowledge(&mut self, r: &Request) -> Result<(), String> {
        if self.stage != AuxiliaryStage::Invalidate || *r != Request::Invalidate {
            return Err("button auxiliary-state reply differs".into());
        }
        self.stage = AuxiliaryStage::Write;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Changed,
    Invalidate,
    Parent,
    InvalidateParent(u32),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Parent(Option<u32>),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Entry,
    Invalidate,
    ParentFirst,
    ParentSecond,
    ParentInvalidate,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Setter {
    value: i32,
    stage: Stage,
    pending: Option<Request>,
    parent: Option<u32>,
}
impl Setter {
    pub fn matches(&self, value: i32) -> bool {
        self.value == value
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn new(value: i32) -> Self {
        Self {
            value,
            stage: Stage::Entry,
            pending: None,
            parent: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = self.parent != Some(0)
            && match (&self.pending, self.stage) {
                (None, _) => true,
                (Some(Request::Changed), Stage::Invalidate)
                | (Some(Request::Invalidate), Stage::ParentFirst)
                | (Some(Request::Parent), Stage::ParentSecond | Stage::ParentInvalidate) => true,
                (Some(Request::InvalidateParent(id)), Stage::Complete) => Some(*id) == self.parent,
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err("button state setter lost its service".into())
        }
    }
    pub fn acknowledge(&mut self, expected: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("button state reply differs".into());
        }
        match (expected, reply) {
            (Request::Parent, Reply::Parent(parent)) => {
                if parent == Some(0) {
                    return Err("invalid parent handle".into());
                }
                self.parent = parent;
            }
            (
                Request::Changed | Request::Invalidate | Request::InvalidateParent(_),
                Reply::Done,
            ) => (),
            _ => return Err("button state reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, state: &mut i32) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Entry => {
                    if *state == self.value {
                        self.stage = Stage::Complete;
                        None
                    } else {
                        *state = self.value;
                        self.stage = Stage::Invalidate;
                        Some(Request::Changed)
                    }
                }
                Stage::Invalidate => {
                    self.stage = Stage::ParentFirst;
                    Some(Request::Invalidate)
                }
                Stage::ParentFirst => {
                    self.stage = Stage::ParentSecond;
                    Some(Request::Parent)
                }
                Stage::ParentSecond => {
                    if self.parent.is_some() {
                        self.stage = Stage::ParentInvalidate;
                        Some(Request::Parent)
                    } else {
                        self.stage = Stage::Complete;
                        None
                    }
                }
                Stage::ParentInvalidate => {
                    let parent = self.parent.ok_or(
                        "native button setter would dereference the missing second parent",
                    )?;
                    self.stage = Stage::Complete;
                    Some(Request::InvalidateParent(parent))
                }
                Stage::Complete => return Ok(None),
            };
            if let Some(r) = request {
                self.pending = Some(r.clone());
                return Ok(Some(r));
            }
        }
    }
}
