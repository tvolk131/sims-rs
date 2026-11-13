//! Base window layout callers: Move4cb0bb, rectangle5ddb0c, recursive flag
//! 4000/5de0d0, surface-position refresh5dd4f3 and absolute rectangle5dc153.
//! Children are explicit calls to the actual class. Circular-list cursors retain
//! node identity across callbacks, then reread links and the current sentinel.
use super::visibility::Call;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub rectangle: [i32; 4], //+7c
    pub absolute: [i32; 4],  //+0c
    pub sentinel: u32,       //+38
    pub updating: u8,        //+3c
    pub surface: u32,        //+58
    pub owned: u32,          //+d0
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub next: u32,   //+0; sentinel nodes also have a next pointer
    pub window: u32, //+8; not read for the sentinel
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub windows: BTreeMap<u32, Fields>,
    pub links: BTreeMap<u32, Link>,
    pub global_manager: u32,       //6fe944
    pub roots: BTreeMap<u32, u32>, //manager+2c
}
impl State {
    pub fn window(&self, owner: u32) -> Result<&Fields, String> {
        self.windows
            .get(&owner)
            .filter(|_| owner != 0)
            .ok_or_else(|| "layout window absent".into())
    }
    fn window_mut(&mut self, owner: u32) -> Result<&mut Fields, String> {
        self.windows
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or_else(|| "layout window absent".into())
    }
    pub(super) fn link(&self, node: u32) -> Result<Link, String> {
        self.links
            .get(&node)
            .copied()
            .filter(|_| node != 0)
            .ok_or_else(|| "layout list node absent".into())
    }
    fn root(&self) -> Result<u32, String> {
        self.roots
            .get(&self.global_manager)
            .copied()
            .filter(|_| self.global_manager != 0)
            .ok_or_else(|| "layout manager absent".into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Move { x: i32, y: i32 },
    Rectangle { rectangle: [i32; 4] },
    Flag4000 { value: u32 },
    Refresh,
    Absolute,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Virtual(Call),
    Flag4000 {
        window: u32,
        value: u32,
    },
    Dispose {
        resource: u32,
    }, //owned resource virtual0(1)
    /// root.virtual170 receives a reference to window+0c, not a rectangle copy.
    RootRegion {
        root: u32,
        window: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Word(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Move,
    Surface,
    Refresh,
    Absolute,
    Owned,
    ClearOwned,
    Flags,
    Root,
    First,
    Next,
    Parent,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    stage: Stage,
    resized: bool,
    node: u32,
    parent: u32,
    rectangle: [i32; 4],
    answer: Reply,
    pending: Option<Request>,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            stage: Stage::Entry,
            resized: false,
            node: 0,
            parent: 0,
            rectangle: [0; 4],
            answer: Reply::Done,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        use Function as F;
        use Stage as S;
        let valid_stage = match self.function {
            F::Move { .. } => matches!(self.stage, S::Entry | S::Move | S::Complete),
            F::Rectangle { .. } => matches!(
                self.stage,
                S::Entry
                    | S::Surface
                    | S::Refresh
                    | S::Absolute
                    | S::Owned
                    | S::ClearOwned
                    | S::Flags
                    | S::Root
                    | S::Complete
            ),
            F::Flag4000 { .. } | F::Refresh => {
                matches!(self.stage, S::Entry | S::First | S::Next | S::Complete)
            }
            F::Absolute => matches!(
                self.stage,
                S::Entry | S::Parent | S::First | S::Next | S::Complete
            ),
        };
        let valid_child = match &self.pending {
            None => {
                !matches!(self.stage, S::Root | S::Parent) || matches!(self.answer, Reply::Word(_))
            }
            Some(r) => {
                let call =
                    |slot, args: &[u32]| *r == Request::Virtual(Call::new(self.owner, slot, args));
                match (self.function, self.stage, r) {
                    (F::Move { .. }, S::Move, _) => call(0x60, &self.rectangle.map(|x| x as u32)),
                    (F::Rectangle { .. }, S::Refresh, _) => self.resized && call(0x174, &[1, 0]),
                    (F::Rectangle { .. }, S::Absolute, _) => call(0x198, &[]),
                    (F::Rectangle { .. }, S::Owned, _) => call(0x88, &[]),
                    (F::Rectangle { .. }, S::ClearOwned, Request::Dispose { resource }) => {
                        self.resized && *resource != 0
                    }
                    (F::Rectangle { .. }, S::Root, _) => call(0x8c, &[8]),
                    (F::Rectangle { .. }, S::Complete, Request::RootRegion { window, .. }) => {
                        *window == self.owner
                    }
                    (F::Flag4000 { value }, S::First, _) => call(0x90, &[0x4000, value]),
                    (F::Flag4000 { value }, S::Next, Request::Flag4000 { value: v, .. }) => {
                        *v == value && self.node != 0
                    }
                    (F::Refresh, S::First, _) => call(0x194, &[]),
                    (F::Refresh, S::Next, Request::Virtual(c)) => {
                        self.node != 0 && c.slot == 0x198 && c.arguments.is_empty()
                    }
                    (F::Absolute, S::Parent, Request::Virtual(c)) => {
                        *c == Call::new(self.parent, 0x18, &[])
                    }
                    (F::Absolute, S::Next, Request::Virtual(c)) => {
                        self.node != 0 && c.slot == 0x88 && c.arguments.is_empty()
                    }
                    _ => false,
                }
            }
        };
        if valid_stage && valid_child {
            Ok(())
        } else {
            Err("window layout lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let word = matches!(r,Request::Virtual(c) if matches!(c.slot,0x18|0x8c|0x90));
        if self.pending.as_ref() != Some(r) || word != matches!(reply, Reply::Word(_)) {
            return Err("window layout reply differs".into());
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
                    F::Move { x, y } => {
                        let [l, t, r, b] = s.window(self.owner)?.rectangle;
                        self.rectangle = [
                            x,
                            y,
                            r.wrapping_sub(l).wrapping_add(x),
                            b.wrapping_sub(t).wrapping_add(y),
                        ];
                        self.stage = S::Move;
                        break Request::Virtual(Call::new(
                            self.owner,
                            0x60,
                            &self.rectangle.map(|x| x as u32),
                        ));
                    }
                    F::Rectangle { rectangle: r } => {
                        let f = s.window_mut(self.owner)?;
                        let old = f.rectangle;
                        self.resized = r[2].wrapping_sub(r[0]) != old[2].wrapping_sub(old[0])
                            || r[3].wrapping_sub(r[1]) != old[3].wrapping_sub(old[1]);
                        f.rectangle = r;
                        self.stage = S::Surface;
                    }
                    F::Flag4000 { value } => {
                        self.stage = S::First;
                        break Request::Virtual(Call::new(self.owner, 0x90, &[0x4000, value]));
                    }
                    F::Refresh => {
                        self.stage = S::First;
                        break Request::Virtual(Call::new(self.owner, 0x194, &[]));
                    }
                    F::Absolute => {
                        self.rectangle = s.window(self.owner)?.rectangle;
                        self.parent = self.owner;
                        self.stage = S::Parent;
                        break Request::Virtual(Call::new(self.parent, 0x18, &[]));
                    }
                },
                S::Move => {
                    self.stage = S::Complete;
                }
                S::Surface => {
                    self.stage = S::Refresh;
                    if s.window(self.owner)?.surface != 0 && self.resized {
                        break Request::Virtual(Call::new(self.owner, 0x174, &[1, 0]));
                    }
                }
                S::Refresh => {
                    self.stage = S::Absolute;
                    break Request::Virtual(Call::new(self.owner, 0x198, &[]));
                }
                S::Absolute => {
                    self.stage = S::Owned;
                    break Request::Virtual(Call::new(self.owner, 0x88, &[]));
                }
                S::Owned => {
                    if self.resized {
                        let resource = s.window(self.owner)?.owned;
                        self.stage = S::ClearOwned;
                        if resource != 0 {
                            break Request::Dispose { resource };
                        }
                    } else {
                        self.stage = S::Flags;
                    }
                }
                S::ClearOwned => {
                    s.window_mut(self.owner)?.owned = 0;
                    self.stage = S::Flags;
                }
                S::Flags => {
                    self.stage = S::Root;
                    break Request::Virtual(Call::new(self.owner, 0x8c, &[8]));
                }
                S::Root => {
                    let Reply::Word(flags) = self.answer else {
                        unreachable!()
                    };
                    self.stage = S::Complete;
                    if flags as u8 != 0 {
                        break Request::RootRegion {
                            root: s.root()?,
                            window: self.owner,
                        };
                    }
                }
                S::Parent => {
                    let Reply::Word(parent) = self.answer else {
                        unreachable!()
                    };
                    if parent != 0 {
                        let r = s.window(parent)?.rectangle;
                        self.rectangle[0] = self.rectangle[0].wrapping_add(r[0]);
                        self.rectangle[2] = self.rectangle[2].wrapping_add(r[0]);
                        self.rectangle[1] = self.rectangle[1].wrapping_add(r[1]);
                        self.rectangle[3] = self.rectangle[3].wrapping_add(r[1]);
                        self.parent = parent;
                        break Request::Virtual(Call::new(parent, 0x18, &[]));
                    }
                    let f = s.window_mut(self.owner)?;
                    f.absolute = self.rectangle;
                    if f.updating == 0 {
                        f.updating = 1;
                    }
                    self.stage = S::First;
                }
                S::First | S::Next => {
                    let sentinel = s.window(self.owner)?.sentinel;
                    let node = s
                        .link(if self.stage == S::First {
                            sentinel
                        } else {
                            self.node
                        })?
                        .next;
                    if node == sentinel {
                        if self.function == F::Absolute {
                            s.window_mut(self.owner)?.updating = 0;
                        }
                        self.stage = S::Complete;
                        continue;
                    }
                    let window = s.link(node)?.window;
                    self.node = node;
                    self.stage = S::Next;
                    break match self.function {
                        F::Flag4000 { value } => Request::Flag4000 { window, value },
                        F::Refresh => Request::Virtual(Call::new(window, 0x198, &[])),
                        F::Absolute => Request::Virtual(Call::new(window, 0x88, &[])),
                        _ => unreachable!(),
                    };
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
