//! Window local position5dd43d, surface-relative position5dd711, and clipping
//! traversal5de213. Parent/virtual calls remain class-dispatched children.
//! Rectangles and child lists are read from shared state at native read points.
use super::{layout, visibility::Call};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub local: [i32; 4],    //window+1c
    pub relative: [i32; 4], //window+8c
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Region {
    Window(u32),   //reference to window+0c, not a copy
    External(u32), //reference to an independently owned rectangle
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub layout: layout::State,
    pub geometry: BTreeMap<u32, Fields>,
    pub regions: BTreeMap<u32, [i32; 4]>,
}
impl State {
    fn geometry_mut(&mut self, owner: u32) -> Result<&mut Fields, String> {
        self.geometry
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or_else(|| "window surface geometry absent".into())
    }
    pub fn region(&self, region: Region) -> Result<[i32; 4], String> {
        match region {
            Region::Window(owner) => Ok(self.layout.window(owner)?.absolute),
            Region::External(id) => self
                .regions
                .get(&id)
                .copied()
                .filter(|_| id != 0)
                .ok_or_else(|| "window clip region absent".into()),
        }
    }
}
///603390 returns the full pointer word. It is not a boolean or low-byte test.
pub fn surface(fields: &layout::Fields) -> u32 {
    fields.surface
}
///5de95f uses signed overlap tests and returns a zero rectangle if disjoint.
/// This also preserves the original behavior for inverted/empty input bounds.
pub fn intersection(a: [i32; 4], b: [i32; 4]) -> ([i32; 4], u8) {
    if a[2] <= b[0] || a[0] >= b[2] || a[3] <= b[1] || a[1] >= b[3] {
        ([0; 4], 0)
    } else {
        (
            [
                a[0].max(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].min(b[3]),
            ],
            1,
        )
    }
}
fn offset(r: &mut [i32; 4], x: i32, y: i32) {
    r[0] = r[0].wrapping_add(x);
    r[2] = r[2].wrapping_add(x);
    r[1] = r[1].wrapping_add(y);
    r[3] = r[3].wrapping_add(y);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Local,
    Relative,
    Clip { region: Region },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Virtual(Call),
    Relative { window: u32 },             //direct5dd711, AL result
    Clip { window: u32, region: Region }, //child virtual170, same rectangle reference
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Word(u32),
    Relative(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    LocalFirstParent,
    LocalParentSurface,
    LocalNextParent,
    LocalCommit,
    RelativeSelfSurface,
    RelativeFirstParent,
    RelativeParentSurface,
    RelativeNextParent,
    ClipFlags,
    ClipFirst,
    ClipNext,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    stage: Stage,
    parent: u32,
    node: u32,
    rectangle: [i32; 4],
    answer: Reply,
    result: Option<u8>,
    pending: Option<Request>,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            stage: Stage::Entry,
            parent: 0,
            node: 0,
            rectangle: [0; 4],
            answer: Reply::Done,
            result: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn result(&self) -> Option<u8> {
        self.result
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        use Function as F;
        use Stage as S;
        let valid_stage = match self.function {
            F::Local => matches!(
                self.stage,
                S::Entry
                    | S::LocalFirstParent
                    | S::LocalParentSurface
                    | S::LocalNextParent
                    | S::LocalCommit
                    | S::Complete
            ),
            F::Relative => matches!(
                self.stage,
                S::Entry
                    | S::RelativeSelfSurface
                    | S::RelativeFirstParent
                    | S::RelativeParentSurface
                    | S::RelativeNextParent
                    | S::Complete
            ),
            F::Clip { .. } => matches!(
                self.stage,
                S::Entry | S::ClipFlags | S::ClipFirst | S::ClipNext | S::Complete
            ),
        };
        let valid_result = if self.function == F::Relative && self.stage == S::Complete {
            matches!(self.result, Some(0 | 1))
        } else {
            self.result.is_none()
        };
        let valid_child = match &self.pending {
            None => {
                !matches!(
                    self.stage,
                    S::LocalFirstParent
                        | S::LocalParentSurface
                        | S::LocalNextParent
                        | S::RelativeSelfSurface
                        | S::RelativeFirstParent
                        | S::RelativeParentSurface
                        | S::RelativeNextParent
                        | S::ClipFlags
                ) || matches!(self.answer, Reply::Word(_))
            }
            Some(r) => {
                let call = |owner, slot, args: &[u32]| {
                    *r == Request::Virtual(Call::new(owner, slot, args))
                };
                match (self.function, self.stage, r) {
                    (F::Local, S::LocalFirstParent, _)
                    | (F::Relative, S::RelativeFirstParent, _) => call(self.owner, 0x18, &[]),
                    (F::Local, S::LocalParentSurface, _)
                    | (F::Relative, S::RelativeParentSurface, _) => {
                        self.parent != 0 && call(self.parent, 0x184, &[])
                    }
                    (F::Local, S::LocalNextParent, _) | (F::Relative, S::RelativeNextParent, _) => {
                        self.parent != 0 && call(self.parent, 0x18, &[])
                    }
                    (F::Relative, S::RelativeSelfSurface, _) => call(self.owner, 0x184, &[]),
                    (F::Local, S::Complete, Request::Relative { window }) => *window == self.owner,
                    (F::Clip { .. }, S::ClipFlags, _) => call(self.owner, 0x8c, &[1]),
                    (F::Clip { .. }, S::ClipFirst, _) => call(self.owner, 0x168, &[]),
                    (F::Clip { region: a }, S::ClipNext, Request::Clip { region: b, .. }) => {
                        self.node != 0 && a == *b
                    }
                    _ => false,
                }
            }
        };
        if valid_stage && valid_result && valid_child {
            Ok(())
        } else {
            Err("window surface geometry lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let kind = match r {
            Request::Relative { .. } => matches!(reply, Reply::Relative(0 | 1)),
            Request::Virtual(c) if matches!(c.slot, 0x18 | 0x184 | 0x8c) => {
                matches!(reply, Reply::Word(_))
            }
            _ => reply == Reply::Done,
        };
        if self.pending.as_ref() != Some(r) || !kind {
            return Err("window surface geometry reply differs".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    pub fn advance(&mut self, s: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        use Function as F;
        use Stage as S;
        let r = loop {
            match self.stage {
                S::Entry => match self.function {
                    F::Local => {
                        let f = s.layout.window(self.owner)?;
                        let r = f.rectangle;
                        if f.surface != 0 {
                            s.geometry_mut(self.owner)?.local =
                                [0, 0, r[2].wrapping_sub(r[0]), r[3].wrapping_sub(r[1])];
                            self.stage = S::Complete;
                            break Request::Relative { window: self.owner };
                        }
                        s.geometry_mut(self.owner)?.local = r;
                        self.rectangle = r;
                        self.stage = S::LocalFirstParent;
                        break Request::Virtual(Call::new(self.owner, 0x18, &[]));
                    }
                    F::Relative => {
                        self.stage = S::RelativeSelfSurface;
                        break Request::Virtual(Call::new(self.owner, 0x184, &[]));
                    }
                    F::Clip { .. } => {
                        self.stage = S::ClipFlags;
                        break Request::Virtual(Call::new(self.owner, 0x8c, &[1]));
                    }
                },
                S::LocalFirstParent | S::LocalNextParent => {
                    let Reply::Word(parent) = self.answer else {
                        unreachable!()
                    };
                    if parent == 0 {
                        self.stage = if self.stage == S::LocalFirstParent {
                            S::Complete
                        } else {
                            S::LocalCommit
                        };
                        continue;
                    }
                    self.parent = parent;
                    self.stage = S::LocalParentSurface;
                    break Request::Virtual(Call::new(parent, 0x184, &[]));
                }
                S::LocalParentSurface => {
                    let Reply::Word(surface) = self.answer else {
                        unreachable!()
                    };
                    if surface != 0 {
                        self.stage = S::LocalCommit;
                        continue;
                    }
                    let r = s.layout.window(self.parent)?.rectangle;
                    offset(&mut self.rectangle, r[0], r[1]);
                    self.stage = S::LocalNextParent;
                    break Request::Virtual(Call::new(self.parent, 0x18, &[]));
                }
                S::LocalCommit => {
                    s.geometry_mut(self.owner)?.local = self.rectangle;
                    self.stage = S::Complete;
                }
                S::RelativeSelfSurface => {
                    let Reply::Word(surface) = self.answer else {
                        unreachable!()
                    };
                    if surface == 0 {
                        self.result = Some(0);
                        self.stage = S::Complete;
                        continue;
                    }
                    self.stage = S::RelativeFirstParent;
                    break Request::Virtual(Call::new(self.owner, 0x18, &[]));
                }
                S::RelativeFirstParent | S::RelativeNextParent => {
                    let Reply::Word(parent) = self.answer else {
                        unreachable!()
                    };
                    if self.stage == S::RelativeFirstParent {
                        let r = s.layout.window(self.owner)?.rectangle;
                        s.geometry_mut(self.owner)?.relative = r;
                    }
                    if parent == 0 {
                        self.result = Some(0);
                        self.stage = S::Complete;
                        continue;
                    }
                    self.parent = parent;
                    self.stage = S::RelativeParentSurface;
                    break Request::Virtual(Call::new(parent, 0x184, &[]));
                }
                S::RelativeParentSurface => {
                    let Reply::Word(surface) = self.answer else {
                        unreachable!()
                    };
                    if surface != 0 {
                        self.result = Some(1);
                        self.stage = S::Complete;
                        continue;
                    }
                    let r = s.layout.window(self.parent)?.rectangle;
                    offset(&mut s.geometry_mut(self.owner)?.relative, r[0], r[1]);
                    self.stage = S::RelativeNextParent;
                    break Request::Virtual(Call::new(self.parent, 0x18, &[]));
                }
                S::ClipFlags => {
                    let Reply::Word(flags) = self.answer else {
                        unreachable!()
                    };
                    let F::Clip { region } = self.function else {
                        unreachable!()
                    };
                    if flags as u8 == 0 {
                        self.stage = S::Complete;
                        continue;
                    }
                    let original = s.layout.window(self.owner)?.absolute;
                    if intersection(original, s.region(region)?).0 == original {
                        self.stage = S::Complete;
                        continue;
                    }
                    self.stage = S::ClipFirst;
                    break Request::Virtual(Call::new(self.owner, 0x168, &[]));
                }
                S::ClipFirst | S::ClipNext => {
                    let sentinel = s.layout.window(self.owner)?.sentinel;
                    let node = s
                        .layout
                        .link(if self.stage == S::ClipFirst {
                            sentinel
                        } else {
                            self.node
                        })?
                        .next;
                    if node == sentinel {
                        self.stage = S::Complete;
                        continue;
                    }
                    let window = s.layout.link(node)?.window;
                    self.node = node;
                    self.stage = S::ClipNext;
                    let F::Clip { region } = self.function else {
                        unreachable!()
                    };
                    break Request::Clip { window, region };
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
