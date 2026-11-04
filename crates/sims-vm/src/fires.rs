//! Registry +0x38: explicit fire membership, independent of resource GUID.
//! 443b6d appends; 443bf7 erases the first match. 44383c repeatedly erases
//! matching entries during registry unlink. Duplicates are intentional.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub registered: Vec<ObjectId>,
    /// Broadcast 0x40760d(0x108, 0) occurs only on an empty/nonempty transition.
    /// Retain the activity at each broadcast for a presentation subscriber;
    /// the original Windows UI subscriber bodies are not executed here.
    pub activity_changes: Vec<bool>,
}

impl State {
    pub fn register(&mut self, object: ObjectId) {
        let was_empty = self.registered.is_empty();
        self.registered.push(object);
        if was_empty {
            self.activity_changes.push(true);
        }
    }

    pub fn unregister(&mut self, object: ObjectId) {
        if let Some(index) = self.registered.iter().position(|&id| id == object) {
            self.registered.remove(index);
            if self.registered.is_empty() {
                self.activity_changes.push(false);
            }
        }
    }

    /// Called only after registry ownership has been confirmed, as in 44383c.
    pub fn remove_object(&mut self, object: ObjectId) {
        while self.registered.contains(&object) {
            self.unregister(object);
        }
    }
}
