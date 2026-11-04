//! Buy-mode RemoveFromLot wrappers:435751 and422ff5. These run function12
//! before virtual physical removal(+50), without deleting the object. The
//! multipart wrapper optionally splits dynamic groups before reading the head.
use serde::{Deserialize, Serialize};

pub mod live;
#[cfg(test)]
mod tests;

/// All identities are retained registry lifetime handles, not reusable IDs.
pub trait View {
    fn multipart(&self, object: u32) -> Result<bool, String>;
    fn dynamic(&self, object: u32) -> Result<bool, String>;
    fn grouped_definition(&self, object: u32) -> Result<bool, String>;
    fn head(&self, object: u32) -> Result<Option<u32>, String>;
    fn next(&self, object: u32) -> Result<Option<u32>, String>;
    fn slots(&self, object: u32) -> Result<usize, String>;
    fn child(&self, object: u32, slot: usize) -> Result<Option<u32>, String>;
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Split(u32),
    Function12(u32),
    /// Always virtual+50(1), even when RemoveFromLot's own argument is zero.
    Physical(u32),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Entry,
    Head,
    Part,
    SlotsStart,
    Slots,
    Next,
    Base,
    Physical,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    object: u32,
    split: bool,
    stage: Stage,
    current: Option<u32>,
    group_pass: bool,
    slot: usize,
    limit: usize,
    pending: Option<Request>,
}
impl Call {
    pub fn new(object: u32, split: bool) -> Self {
        Self {
            object,
            split,
            stage: Stage::Entry,
            current: None,
            group_pass: false,
            slot: 0,
            limit: 0,
            pending: None,
        }
    }
    pub fn object(&self) -> u32 {
        self.object
    }
    pub fn split(&self) -> bool {
        self.split
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self.pending {
            Some(Request::Split(id)) => {
                id == self.object && self.stage == Stage::Head && self.split
            }
            Some(Request::Function12(id)) => {
                id != 0
                    && (self.stage == Stage::SlotsStart && self.current == Some(id)
                        || self.stage == Stage::Slots)
            }
            Some(Request::Physical(id)) => id == self.object && self.stage == Stage::Complete,
            None => true,
        };
        if self.object == 0 || self.current == Some(0) || self.slot > self.limit || !valid {
            return Err("invalid pickup wrapper continuation".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("pickup reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, view: &impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Entry => {
                    self.stage = if view.multipart(self.object)? {
                        Stage::Head
                    } else {
                        Stage::Base
                    };
                    if self.stage == Stage::Head && self.split && view.dynamic(self.object)? {
                        break Request::Split(self.object);
                    }
                }
                Stage::Head => {
                    //The dynamic split callback can change the head.
                    self.current = Some(view.head(self.object)?.unwrap_or(self.object));
                    self.group_pass = true;
                    self.stage = Stage::Part;
                }
                Stage::Part => {
                    if let Some(id) = self.current {
                        self.stage = Stage::SlotsStart;
                        break Request::Function12(id);
                    }
                    self.stage = Stage::Base;
                }
                Stage::SlotsStart => {
                    //Original captures end after the receiver callback, then
                    //reads each slot's current contents after child callbacks.
                    self.limit = view.slots(self.current.ok_or("pickup receiver absent")?)?;
                    self.slot = 0;
                    self.stage = Stage::Slots;
                }
                Stage::Slots => {
                    let current = self.current.ok_or("pickup slot receiver absent")?;
                    if self.slot < self.limit {
                        let child = view.child(current, self.slot)?;
                        self.slot += 1;
                        if let Some(child) = child {
                            break Request::Function12(child);
                        }
                    } else {
                        self.stage = Stage::Next;
                    }
                }
                Stage::Next => {
                    if self.group_pass {
                        self.current = view.next(self.current.ok_or("pickup part absent")?)?;
                        self.stage = Stage::Part;
                    } else {
                        self.stage = Stage::Physical;
                    }
                }
                Stage::Base => {
                    self.group_pass = false;
                    if !view.grouped_definition(self.object)? {
                        self.current = Some(self.object);
                        self.stage = Stage::Part;
                    } else {
                        self.stage = Stage::Physical;
                    }
                }
                Stage::Physical => {
                    self.stage = Stage::Complete;
                    break Request::Physical(self.object);
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
