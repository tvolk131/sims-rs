//!476a35 current-tool publication. This is separate from CPState4cefce.
//!Listeners use fresh next links AFTER both virtual calls, unlike Commander.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listeners {
    pub head: Option<u32>,
    pub next: BTreeMap<u32, Option<u32>>,
}
impl Listeners {
    ///476a9a construction, followed separately by476ad1 registration.
    pub fn construct(&mut self, id: u32) -> Result<(), String> {
        if id == 0 || self.next.contains_key(&id) {
            return Err("tool listener identity already exists or is null".into());
        }
        self.next.insert(id, None);
        Ok(())
    }
    pub fn register(&mut self, id: u32) -> Result<(), String> {
        if !self.next.contains_key(&id) {
            return Err("tool listener absent".into());
        }
        let mut n = self.head;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(current) = n {
            if !seen.insert(current) {
                return Err("cyclic tool listeners".into());
            }
            if current == id {
                return Ok(());
            }
            n = self.following(current)?;
        }
        self.next.insert(id, self.head);
        self.head = Some(id);
        Ok(())
    }
    ///476aa7 link writes: unlink, then clear self+4 even when detached.
    ///The caller owns vtable replacement and any later heap release.
    pub fn unlink(&mut self, id: u32) -> Result<(), String> {
        if !self.next.contains_key(&id) {
            return Err("tool listener absent".into());
        }
        let mut previous = None;
        let mut n = self.head;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(current) = n {
            if !seen.insert(current) {
                return Err("cyclic tool listeners".into());
            }
            let next = self.following(current)?;
            if current == id {
                if let Some(p) = previous {
                    self.next.insert(p, next);
                } else {
                    self.head = next;
                }
                break;
            }
            previous = n;
            n = next;
        }
        self.next.insert(id, None);
        Ok(())
    }
    pub fn following(&self, id: u32) -> Result<Option<u32>, String> {
        self.next
            .get(&id)
            .copied()
            .ok_or("tool listener released".into())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Reset(u32),
    Enter(u32),
    Old { listener: u32, tool: Option<u32> },
    New { listener: u32, tool: Option<u32> },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Publish,
    Head,
    Old,
    New,
    Next,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    target: Option<u32>,
    old: Option<u32>,
    listener: Option<u32>,
    stage: Stage,
    pending: Option<Request>,
}
impl Change {
    pub fn new(target: Option<u32>) -> Result<Self, String> {
        if target == Some(0) {
            return Err("null tool must use None".into());
        }
        Ok(Self {
            target,
            old: None,
            listener: None,
            stage: Stage::Entry,
            pending: None,
        })
    }
    pub fn target(&self) -> Option<u32> {
        self.target
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.target == Some(0) || self.old == Some(0) || self.listener == Some(0) {
            return Err("invalid current-tool identity".into());
        }
        let valid = match (self.stage, self.pending) {
            (Stage::Publish, Some(Request::Reset(id))) => self.old == Some(id),
            (Stage::Head, Some(Request::Enter(id))) => self.target == Some(id),
            (Stage::New, Some(Request::Old { listener, tool })) => {
                self.listener == Some(listener) && tool == self.old
            }
            (Stage::Next, Some(Request::New { listener, tool })) => {
                self.listener == Some(listener) && tool == self.target
            }
            (_, None) => true,
            _ => false,
        };
        if !valid || matches!(self.stage, Stage::New | Stage::Next) && self.listener.is_none() {
            return Err("current-tool callback lost its stage".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("current-tool reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        current: &mut Option<u32>,
        listeners: &Listeners,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Entry => {
                    if *current == Some(0) {
                        return Err("invalid current-tool global".into());
                    }
                    self.old = *current;
                    self.stage = Stage::Publish;
                    if let Some(id) = self.old {
                        break Request::Reset(id);
                    }
                }
                Stage::Publish => {
                    *current = self.target;
                    self.stage = Stage::Head;
                    if let Some(id) = self.target {
                        break Request::Enter(id);
                    }
                }
                Stage::Head => {
                    self.listener = listeners.head;
                    self.stage = Stage::Old;
                }
                Stage::Old => {
                    let Some(listener) = self.listener else {
                        self.stage = Stage::Complete;
                        continue;
                    };
                    listeners.following(listener)?; //receiver must still be resident
                    self.stage = Stage::New;
                    break Request::Old {
                        listener,
                        tool: self.old,
                    };
                }
                Stage::New => {
                    let listener = self.listener.ok_or("tool listener absent")?;
                    listeners.following(listener)?;
                    self.stage = Stage::Next;
                    break Request::New {
                        listener,
                        tool: self.target,
                    };
                }
                Stage::Next => {
                    self.listener =
                        listeners.following(self.listener.ok_or("tool listener absent")?)?;
                    self.stage = Stage::Old;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}
