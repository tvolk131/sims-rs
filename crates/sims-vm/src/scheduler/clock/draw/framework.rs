//!5db1ed registered-window notifications. Native visible/ancestry helpers keep
//! their repeated virtual reads; callbacks and lock ownership remain explicit.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub owner: u32,
    pub root: u32,
    pub registered: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Enter { critical: u32 },
    Flags { window: u32, mask: u32 },
    Visible { window: u32 },
    Descendant { window: u32, ancestor: u32 },
    Parent { window: u32 },
    Notify { window: u32, force: u32 },
    Leave { critical: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Value(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Scan,
    Flags,
    Visible,
    Descendant,
    Parent,
    ParentAgain,
    ParentFlags,
    Notify,
    Next,
    Leave,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    owner: u32,
    index: usize,
    window: u32,
    parent: u32,
    stage: Stage,
    pending: Option<Request>,
}
impl Frame {
    pub fn new(owner: u32) -> Self {
        Self {
            owner,
            index: 0,
            window: 0,
            parent: 0,
            stage: Stage::Entry,
            pending: None,
        }
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Entry && self.pending.is_none()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self, s: &Fields) -> Result<(), String> {
        if self.owner == 0
            || s.owner != self.owner
            || s.registered.len() > 4096
            || self.index > s.registered.len()
            || self.complete() && self.pending.is_some()
        {
            return Err("framework owner/cursor differs".into());
        }
        if let Some(r) = &self.pending {
            let expected = match self.stage {
                Stage::Entry => Request::Enter {
                    critical: self.owner.wrapping_add(0x200),
                },
                Stage::Flags => Request::Flags {
                    window: self.window,
                    mask: 0x4000,
                },
                Stage::Visible => Request::Visible {
                    window: self.window,
                },
                Stage::Descendant => Request::Descendant {
                    window: self.window,
                    ancestor: s.root,
                },
                Stage::Parent | Stage::ParentAgain => Request::Parent {
                    window: self.window,
                },
                Stage::ParentFlags => Request::Flags {
                    window: self.parent,
                    mask: 0x4000,
                },
                Stage::Notify => Request::Notify {
                    window: self.window,
                    force: 1,
                },
                Stage::Leave => Request::Leave {
                    critical: self.owner.wrapping_add(0x200),
                },
                _ => return Err("framework child outside a caller stage".into()),
            };
            if *r != expected {
                return Err("framework child arguments differ".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, s: &Fields) -> Result<Option<Request>, String> {
        self.validate(s)?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        for _ in 0..8194 {
            let request = match self.stage {
                Stage::Entry => Request::Enter {
                    critical: self.owner.wrapping_add(0x200),
                },
                Stage::Scan => {
                    if let Some(&window) = s.registered.get(self.index) {
                        self.window = window;
                        if window == 0 {
                            self.stage = Stage::Next;
                        } else {
                            self.stage = Stage::Flags;
                        }
                    } else {
                        self.stage = Stage::Leave;
                    }
                    continue;
                }
                Stage::Flags => Request::Flags {
                    window: self.window,
                    mask: 0x4000,
                },
                Stage::Visible => Request::Visible {
                    window: self.window,
                },
                Stage::Descendant => Request::Descendant {
                    window: self.window,
                    ancestor: s.root,
                },
                Stage::Parent | Stage::ParentAgain => Request::Parent {
                    window: self.window,
                },
                Stage::ParentFlags => {
                    if self.parent == 0 {
                        return Err("framework repeated parent query returned null".into());
                    }
                    Request::Flags {
                        window: self.parent,
                        mask: 0x4000,
                    }
                }
                Stage::Notify => Request::Notify {
                    window: self.window,
                    force: 1,
                },
                Stage::Next => {
                    self.index += 1;
                    self.stage = Stage::Scan;
                    continue;
                }
                Stage::Leave => Request::Leave {
                    critical: self.owner.wrapping_add(0x200),
                },
                Stage::Complete => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
        Err("framework scan exceeded bounded roster".into())
    }
    pub fn acknowledge(&mut self, s: &Fields, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate(s)?;
        if self.pending.as_ref() != Some(r) {
            return Err("framework reply is not pending".into());
        }
        let scalar = matches!(
            r,
            Request::Flags { .. }
                | Request::Visible { .. }
                | Request::Descendant { .. }
                | Request::Parent { .. }
        );
        if scalar != matches!(reply, Reply::Value(_)) {
            return Err("framework reply type differs".into());
        }
        let v = if let Reply::Value(v) = reply { v } else { 0 };
        self.stage = match self.stage {
            Stage::Entry => Stage::Scan,
            Stage::Flags => {
                if v & 255 != 0 {
                    Stage::Visible
                } else {
                    Stage::Next
                }
            }
            Stage::Visible => {
                if v & 255 != 0 {
                    Stage::Descendant
                } else {
                    Stage::Next
                }
            }
            Stage::Descendant => {
                if v & 255 != 0 {
                    Stage::Parent
                } else {
                    Stage::Next
                }
            }
            Stage::Parent => {
                if v != 0 {
                    Stage::ParentAgain
                } else {
                    Stage::Next
                }
            }
            Stage::ParentAgain => {
                self.parent = v;
                Stage::ParentFlags
            }
            Stage::ParentFlags => {
                if v & 255 == 0 {
                    Stage::Notify
                } else {
                    Stage::Next
                }
            }
            Stage::Notify => Stage::Next,
            Stage::Leave => Stage::Complete,
            _ => return Err("framework reply outside child".into()),
        };
        self.pending = None;
        self.validate(s)
    }
}
pub trait Query {
    fn flags(&mut self, window: u32, mask: u32) -> Result<u32, String>;
    fn parent(&mut self, window: u32) -> Result<u32, String>;
}
///5de106: root flags are deliberately read twice, as is each nonnull parent.
pub fn visible(mut window: u32, q: &mut impl Query) -> Result<u32, String> {
    for _ in 0..4096 {
        if q.flags(window, 1)? & 255 == 0 {
            return Ok(0);
        }
        if q.parent(window)? == 0 {
            return Ok(q.flags(window, 1)?);
        }
        window = q.parent(window)?;
        if window == 0 {
            return Err("visible helper repeated parent became null".into());
        }
    }
    Err("visible helper parent cycle/budget".into())
}
///5dc8bd starts at the parent, so a window is not its own descendant.
pub fn descendant(window: u32, ancestor: u32, q: &mut impl Query) -> Result<u32, String> {
    let mut current = q.parent(window)?;
    for _ in 0..4096 {
        if current == 0 {
            return Ok(0);
        }
        if current == ancestor {
            return Ok(1);
        }
        current = q.parent(current)?;
    }
    Err("descendant helper parent cycle/budget".into())
}
