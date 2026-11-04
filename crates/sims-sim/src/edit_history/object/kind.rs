//!48bcc2: pre-order recursive kind stores using captured cursors and live ends.
use super::Objects;
use crate::{
    edit_history::{List, Slot, Store},
    object_catalog::move_tool::{cleanup, dispatch},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Frame {
    owner: u32,
    cursor: Option<Slot>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    owner: u32,
    value: u32,
    stack: Vec<Frame>,
}
impl Call {
    pub fn binding(&self) -> (u32, u32) {
        (self.owner, self.value)
    }
    pub fn validate(&self, store: &Store) -> Result<(), String> {
        if self.owner == 0 || self.stack.first().is_some_and(|f| f.owner != self.owner) {
            return Err("kind setter root differs".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for (n, frame) in self.stack.iter().enumerate() {
            if frame.owner == 0 || !seen.insert(frame.owner) {
                return Err("kind setter has a null or cyclic frame".into());
            }
            let end = store.end(List::Objects(frame.owner))?;
            if frame
                .cursor
                .is_some_and(|c| c.allocation != end.allocation || c.index > end.index)
            {
                return Err("kind setter lost its allocation cursor".into());
            }
            if n > 0
                && store.read(
                    self.stack[n - 1]
                        .cursor
                        .ok_or("kind parent cursor absent")?,
                )? != frame.owner
            {
                return Err("kind setter child differs from parent".into());
            }
        }
        Ok(())
    }
    pub fn new(owner: u32, value: u32) -> Self {
        Self {
            owner,
            value,
            stack: vec![Frame {
                owner,
                cursor: None,
            }],
        }
    }
    pub fn complete(&self) -> bool {
        self.stack.is_empty()
    }
    ///A diagnostic instruction budget permits saving deep trees without using
    ///the Rust call stack. It does not yield a gameplay tick or a native callback.
    pub fn advance(
        &mut self,
        store: &Store,
        objects: &mut Objects,
        budget: usize,
    ) -> Result<bool, String> {
        for _ in 0..budget {
            let Some(frame) = self.stack.last_mut() else {
                return Ok(true);
            };
            if frame.cursor.is_none() {
                let cursor = store.begin(List::Objects(frame.owner))?;
                objects.get_mut(frame.owner)?.kind = self.value as i32;
                frame.cursor = Some(cursor);
            }
            let cursor = frame.cursor.unwrap();
            if cursor == store.end(List::Objects(frame.owner))? {
                self.stack.pop();
                if let Some(parent) = self.stack.last_mut() {
                    parent.cursor = Some(parent.cursor.ok_or("kind parent has no cursor")?.next()?);
                }
            } else {
                let child = store.read(cursor)?;
                //The original dereferences null; it is not a destructor skip.
                if child == 0 {
                    return Err("recursive kind setter dereferenced a null child".into());
                }
                if self.stack.iter().any(|f| f.owner == child) {
                    return Err("cyclic recursive kind setter".into());
                }
                self.stack.push(Frame {
                    owner: child,
                    cursor: None,
                });
            }
        }
        Ok(self.complete())
    }
    pub fn finish(&self, parent: &mut dispatch::Dispatch) -> Result<(), String> {
        let service = dispatch::Service::External(cleanup::Request::PayloadKind {
            payload: self.owner,
            value: self.value,
        });
        if !self.complete() || parent.service() != Some(service) {
            return Err("kind setter cannot acknowledge unfinished or different caller".into());
        }
        parent.acknowledge(service, cleanup::Reply::Done)
    }
}
