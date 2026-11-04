//! Original407577/4075df/4075a1 registration and40760d synchronous broadcast.
//! Stable host IDs replace pointers. A broadcast retains its captured next link
//! across callbacks, including nested broadcasts and registration mutations.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub type Id = u32;
pub mod house;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node<H> {
    pub next: Option<Id>,
    pub auxiliary: u32,
    pub serial: u32,
    pub handler: H,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry<H> {
    pub head: Option<Id>,
    pub generation: u32,
    pub nodes: BTreeMap<Id, Node<H>>,
}
impl<H> Default for Registry<H> {
    fn default() -> Self {
        Self {
            head: None,
            generation: 0,
            nodes: BTreeMap::new(),
        }
    }
}
// H::default() denotes the original base handler407607, which returns zero.
impl<H: Default> Registry<H> {
    pub fn validate(&self) -> Result<(), String> {
        let mut next = self.head;
        let mut seen = BTreeSet::new();
        if self.nodes.contains_key(&0) {
            return Err("null Commander identity".into());
        }
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err("cyclic Commander registry".into());
            }
            next = self
                .nodes
                .get(&id)
                .ok_or("Commander registry link absent")?
                .next;
        }
        // Detached objects retain their old next pointer; it is not traversed
        // by registry validation. A suspended broadcast may still own it.
        Ok(())
    }
    pub fn construct(&mut self, id: Id) -> Result<(), String> {
        self.validate()?;
        if id == 0 {
            return Err("null Commander identity".into());
        }
        let mut next = self.head;
        while let Some(n) = next {
            if n == id {
                return Err("Commander already registered".into());
            }
            next = self.nodes[&n].next;
        }
        self.nodes.insert(
            id,
            Node {
                next: self.head,
                auxiliary: 0,
                serial: self.generation,
                handler: H::default(),
            },
        );
        self.generation = self.generation.wrapping_add(1);
        self.head = Some(id);
        Ok(())
    }
    /// Derived construction's vtable assignment, separate from407577.
    pub fn bind(&mut self, id: Id, handler: H) -> Result<(), String> {
        self.nodes
            .get_mut(&id)
            .ok_or("Commander object absent")?
            .handler = handler;
        Ok(())
    }
    pub fn unlink(&mut self, id: Id) -> Result<(), String> {
        self.validate()?;
        self.bind(id, H::default())?;
        let mut previous = None;
        let mut next = self.head;
        while let Some(n) = next {
            let following = self.nodes[&n].next;
            if n == id {
                if let Some(p) = previous {
                    self.nodes.get_mut(&p).unwrap().next = following;
                } else {
                    self.head = following;
                }
                break;
            }
            previous = Some(n);
            next = following;
        }
        Ok(())
    }
    /// Native byte flag bit0 selects heap release. A callback may destroy
    /// itself; its dispatcher already owns the next link and return slot.
    pub fn destroy(&mut self, id: Id, flags: u8) -> Result<(), String> {
        self.unlink(id)?;
        if flags & 1 != 0 {
            self.nodes.remove(&id);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation<H> {
    pub recipient: Id,
    pub handler: H,
    pub event: u32,
    pub value: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Broadcast<H> {
    event: u32,
    value: u32,
    next: Option<Id>,
    pending: Option<Invocation<H>>,
    handled: bool,
}
impl<H: Default + Clone + PartialEq> Broadcast<H> {
    pub fn begin(registry: &Registry<H>, event: u32, value: u32) -> Result<Self, String> {
        registry.validate()?;
        Ok(Self {
            event,
            value,
            next: registry.head,
            pending: None,
            handled: false,
        })
    }
    pub fn validate(&self, registry: &Registry<H>) -> Result<(), String> {
        registry.validate()?;
        if self
            .next
            .is_some_and(|id| !registry.nodes.contains_key(&id))
        {
            return Err("broadcast captured a released Commander".into());
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.recipient == 0 || p.event != self.event || p.value != self.value)
        {
            return Err("invalid Commander callback continuation".into());
        }
        Ok(())
    }
    pub fn pending(&self) -> Option<&Invocation<H>> {
        self.pending.as_ref()
    }
    pub fn message(&self) -> (u32, u32) {
        (self.event, self.value)
    }
    pub fn complete(&self) -> bool {
        self.pending.is_none() && self.next.is_none()
    }
    pub fn returned(&self) -> Option<u16> {
        self.complete().then_some(u16::from(self.handled))
    }
    pub fn advance(&mut self, registry: &Registry<H>) -> Result<Option<&Invocation<H>>, String> {
        self.validate(registry)?;
        if self.pending.is_none() {
            if let Some(id) = self.next {
                let node = &registry.nodes[&id];
                // Both the vtable and next link are captured before dispatch.
                self.next = node.next;
                self.pending = Some(Invocation {
                    recipient: id,
                    handler: node.handler.clone(),
                    event: self.event,
                    value: self.value,
                });
            }
        }
        Ok(self.pending.as_ref())
    }
    pub fn acknowledge(&mut self, request: &Invocation<H>, returned: u32) -> Result<(), String> {
        if self.pending.as_ref() != Some(request) {
            return Err("Commander acknowledgement mismatch".into());
        }
        self.handled |= returned as u16 != 0;
        self.pending = None;
        Ok(())
    }
}
