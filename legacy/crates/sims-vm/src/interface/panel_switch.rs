//! 50a7ec. Always publish the selector and visit all six children in original
//! order. Child50a7a2 owns null handling; the selected pointer is captured for
//! final invalidation even if callbacks replace that panel's shared identity.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub selected: i32, // +d8
    /// +e4,+e0,+e8,+ec,+f0,+f4 in selector order0..5.
    pub panels: [u32; 6],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Visibility { panel: u32, show: u32 }, //50a7a2
    Invalidate(u32),                      //virtual168
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Children,
    Invalidate,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    selector: i32,
    stage: Stage,
    next: usize,
    chosen: u32,
    pending: Option<Request>,
}
impl Selection {
    pub fn matches(&self, selector: i32) -> bool {
        self.selector == selector
    }
    pub fn new(selector: i32) -> Self {
        Self {
            selector,
            stage: Stage::Entry,
            next: 0,
            chosen: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = self.next <= 6
            && match (self.stage, self.pending) {
                (Stage::Entry, None) => self.next == 0,
                (_, None) => true,
                (Stage::Children, Some(Request::Visibility { show, .. })) => {
                    self.next > 0 && show == u32::from(self.selector == self.next as i32 - 1)
                }
                (Stage::Complete, Some(Request::Invalidate(p))) => {
                    self.next == 6 && p == self.chosen && p != 0
                }
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err("panel selection lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("panel selection reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                Stage::Entry => {
                    fields.selected = self.selector;
                    self.stage = Stage::Children;
                }
                Stage::Children => {
                    if self.next == 6 {
                        self.stage = Stage::Invalidate;
                        continue;
                    }
                    let panel = fields.panels[self.next];
                    let show = u32::from(self.selector == self.next as i32);
                    if show != 0 {
                        self.chosen = panel;
                    }
                    self.next += 1;
                    self.pending = Some(Request::Visibility { panel, show });
                    return Ok(self.pending);
                }
                Stage::Invalidate => {
                    self.stage = Stage::Complete;
                    if self.chosen != 0 {
                        self.pending = Some(Request::Invalidate(self.chosen));
                        return Ok(self.pending);
                    }
                }
                Stage::Complete => return Ok(None),
            }
        }
    }
}
