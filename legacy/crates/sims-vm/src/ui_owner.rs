//! UI-event owner from 0x44d906/0x444bc8. This is a research label: the
//! original UI queries this object's "query wait event" tree (0x4764fe).
//! Live dialog widgets and signal subscribers remain a presentation boundary.
pub mod wait;
use crate::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct State {
    pub owner: Option<ObjectId>,
    /// Owners whose live dialogs must be closed, in original registry order.
    /// Mirrors private-dialog registry order when that host is active.
    pub dialog_owners: Vec<ObjectId>,
    pub effects: Vec<Effect>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Effect {
    CloseDialog(usize),
    /// Original signal 0x107, payload zero.
    Changed,
}
impl State {
    pub fn primitive(&mut self, caller: ObjectId, mode: u8) -> Result<bool, String> {
        match mode {
            0 if self.owner.is_none() => self.replace(Some(caller)),
            0 => Ok(false), // Even acquiring again by the same object fails.
            1 if self.owner == Some(caller) => self.replace(None),
            1 => Ok(false),
            _ => Err("invalid UI-event ownership mode (native error 0x35)".into()),
        }
    }
    /// Native reset/deletion also clears the owner through 0x444bc8.
    pub fn release_if_owner(&mut self, object: ObjectId) -> Result<(), String> {
        if self.owner == Some(object) {
            self.replace(None)?;
        }
        Ok(())
    }
    fn replace(&mut self, new_owner: Option<ObjectId>) -> Result<bool, String> {
        let closing: Vec<_> = self
            .dialog_owners
            .iter()
            .enumerate()
            .filter_map(|(i, &id)| (self.owner == Some(id)).then_some(Effect::CloseDialog(i)))
            .collect();
        if self.effects.len() + closing.len() + 1 > 4096 {
            return Err("UI-event ownership effects require consumption".into());
        }
        self.effects.extend(closing);
        self.owner = new_owner;
        self.effects.push(Effect::Changed);
        Ok(true)
    }
}
