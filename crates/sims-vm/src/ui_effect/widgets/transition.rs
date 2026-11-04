//! Animated visibility5de382 and cancellation5de7ef. Cleanup order differs
//! between expiry and explicit cancellation. Parent message27 goes through the
//! manager's queued notification wrapper5da74b; it is not immediate delivery.
use super::{surface_resource, visibility::Call};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod curve;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub direction: i32,  //+b4
    pub curve: [u32; 4], //+bc..c8
    pub had_surface: u8, //+cc
    pub option: u8,      //+cd
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub resources: surface_resource::State,
    pub transitions: BTreeMap<u32, Fields>,
    pub now: u32, //6e54e0
}
impl State {
    fn fields(&self, owner: u32) -> Result<&Fields, String> {
        self.transitions
            .get(&owner)
            .filter(|_| owner != 0)
            .ok_or_else(|| "window transition fields absent".into())
    }
    fn fields_mut(&mut self, owner: u32) -> Result<&mut Fields, String> {
        self.transitions
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or_else(|| "window transition fields absent".into())
    }
    fn clear_owned(&mut self, owner: u32) -> Result<(), String> {
        self.resources
            .geometry
            .layout
            .windows
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or("transition window absent")?
            .owned = 0;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Visibility {
        show: u32,
        option: u32,
        duration: i32,
    },
    Cancel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Window(Call),
    Dispose {
        resource: u32,
    },
    ///5da74b packs [recipient,message,argument1,argument2,argument3] and
    /// appends it via5db3f9 to manager+154. Allocation/queue service stays explicit.
    Notify {
        manager: u32,
        record: [u32; 5],
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
    CancelRelease,
    NotifyParent,
    ParentOne,
    ParentTwo,
    ClearOwned,
    AfterDispose,
    Query,
    QueryResult,
    FirstChild,
    NextChild,
    SetSurface,
    Configure,
    InvalidateParent,
    InvalidateOne,
    InvalidateTwo,
    InvalidateSelf,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    stage: Stage,
    answer: Reply,
    pending: Option<Request>,
    direction: i32,
    parent: u32,
    owned: u32,
    node: u32,
    duration: i32,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            stage: Stage::Entry,
            answer: Reply::Done,
            pending: None,
            direction: 0,
            parent: 0,
            owned: 0,
            node: 0,
            duration: 0,
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
        let phase = match self.function {
            F::Cancel => !matches!(
                self.stage,
                S::Query
                    | S::QueryResult
                    | S::SetSurface
                    | S::Configure
                    | S::InvalidateParent
                    | S::InvalidateOne
                    | S::InvalidateTwo
                    | S::InvalidateSelf
            ),
            F::Visibility { .. } => self.stage != S::CancelRelease,
        };
        let child = match &self.pending {
            None => {
                !matches!(
                    self.stage,
                    S::ParentOne
                        | S::ParentTwo
                        | S::QueryResult
                        | S::InvalidateOne
                        | S::InvalidateTwo
                ) || matches!(self.answer, Reply::Word(_))
            }
            Some(r) => {
                let window =
                    |owner, slot, args: &[u32]| *r == Request::Window(Call::new(owner, slot, args));
                match (self.function, self.stage, r) {
                    (F::Cancel, S::CancelRelease, _) => window(self.owner, 0x98, &[]),
                    (F::Visibility { .. }, S::NotifyParent, _) => window(self.owner, 0x98, &[]),
                    (F::Cancel, S::NotifyParent, _) => window(self.owner, 0x174, &[0, 0]),
                    (_, S::ParentOne | S::ParentTwo, _) => window(self.owner, 0x18, &[]),
                    (_, S::ClearOwned, Request::Notify { record, .. }) => {
                        *record == [self.parent, 27, self.owner, self.direction as u32, 0]
                            && self.parent != 0
                    }
                    (_, S::AfterDispose, Request::Dispose { resource }) => {
                        *resource == self.owned && *resource != 0
                    }
                    (F::Visibility { .. }, S::Query, _) => window(self.owner, 0x174, &[0, 0]),
                    (F::Visibility { .. }, S::QueryResult, _) => window(self.owner, 0x8c, &[1]),
                    (_, S::NextChild, Request::Window(c)) => {
                        self.node != 0 && c.slot == 0xac && c.arguments.is_empty()
                    }
                    (F::Visibility { .. }, S::Configure, _) => window(self.owner, 0x174, &[1, 0]),
                    (F::Visibility { show, .. }, S::InvalidateParent, _) => {
                        show as u8 != 0 && window(self.owner, 0x94, &[])
                    }
                    (F::Visibility { .. }, S::InvalidateOne | S::InvalidateTwo, _) => {
                        window(self.owner, 0x18, &[])
                    }
                    (F::Visibility { .. }, S::InvalidateSelf, _) => window(self.parent, 0x168, &[]),
                    (F::Visibility { .. }, S::Complete, _) => window(self.owner, 0x168, &[]),
                    _ => false,
                }
            }
        };
        if phase && child {
            Ok(())
        } else {
            Err("window transition lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let word = matches!(r,Request::Window(c) if matches!(c.slot,0x18|0x8c));
        if self.pending.as_ref() != Some(r) || word != matches!(reply, Reply::Word(_)) {
            return Err("window transition reply differs".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    fn word(&self) -> u32 {
        let Reply::Word(v) = self.answer else {
            unreachable!()
        };
        v
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
                S::Entry => {
                    let f = s.fields(self.owner)?;
                    let direction = f.direction;
                    let active =
                        direction != 0 && (self.function == F::Cancel || s.now >= f.curve[3]);
                    if active {
                        self.stage = if self.function == F::Cancel {
                            S::CancelRelease
                        } else {
                            S::NotifyParent
                        };
                        if direction < 0 {
                            break Request::Window(Call::new(self.owner, 0x98, &[]));
                        }
                    } else {
                        self.stage = if self.function == F::Cancel {
                            S::FirstChild
                        } else {
                            S::Query
                        };
                    }
                }
                S::CancelRelease => {
                    self.stage = S::NotifyParent;
                    if s.fields(self.owner)?.had_surface == 0 {
                        break Request::Window(Call::new(self.owner, 0x174, &[0, 0]));
                    }
                }
                S::NotifyParent => {
                    self.stage = S::ParentOne;
                    break Request::Window(Call::new(self.owner, 0x18, &[]));
                }
                S::ParentOne => {
                    if self.word() == 0 {
                        self.stage = S::ClearOwned;
                        continue;
                    }
                    self.direction = s.fields(self.owner)?.direction;
                    self.stage = S::ParentTwo;
                    break Request::Window(Call::new(self.owner, 0x18, &[]));
                }
                S::ParentTwo => {
                    self.parent = self.word();
                    self.stage = S::ClearOwned;
                    if self.parent != 0 {
                        break Request::Notify {
                            manager: s.resources.geometry.layout.global_manager,
                            record: [self.parent, 27, self.owner, self.direction as u32, 0],
                        };
                    }
                }
                S::ClearOwned => {
                    self.owned = s.resources.geometry.layout.window(self.owner)?.owned;
                    s.fields_mut(self.owner)?.direction = 0;
                    self.stage = S::AfterDispose;
                    if self.owned != 0 {
                        break Request::Dispose {
                            resource: self.owned,
                        };
                    }
                }
                S::AfterDispose => {
                    s.clear_owned(self.owner)?;
                    if self.function == F::Cancel {
                        self.stage = S::FirstChild;
                    } else {
                        self.stage = S::Query;
                        if s.fields(self.owner)?.had_surface == 0 {
                            break Request::Window(Call::new(self.owner, 0x174, &[0, 0]));
                        }
                    }
                }
                S::Query => {
                    self.stage = S::QueryResult;
                    break Request::Window(Call::new(self.owner, 0x8c, &[1]));
                }
                S::QueryResult => {
                    let F::Visibility { show, duration, .. } = self.function else {
                        unreachable!()
                    };
                    let show = show as u8;
                    let f = s.fields(self.owner)?;
                    let direction = f.direction;
                    if (show == self.word() as u8 && (show == 0 || direction >= 0))
                        || (show == 0 && direction < 0)
                        || (show != 0 && direction > 0)
                    {
                        self.stage = S::Complete;
                        continue;
                    }
                    self.duration = if duration == 0 { 333 } else { duration };
                    if direction != 0 {
                        let curve = curve::reverse(f.curve, direction, self.duration, s.now)?;
                        let f = s.fields_mut(self.owner)?;
                        f.curve = curve;
                        f.direction = if direction < 0 { 1 } else { -1 };
                        self.stage = S::Complete;
                    } else {
                        self.stage = S::FirstChild;
                    }
                }
                S::FirstChild | S::NextChild => {
                    let layout = &s.resources.geometry.layout;
                    let sentinel = layout.window(self.owner)?.sentinel;
                    let node = layout
                        .link(if self.stage == S::FirstChild {
                            sentinel
                        } else {
                            self.node
                        })?
                        .next;
                    if node == sentinel {
                        self.stage = if self.function == F::Cancel {
                            S::Complete
                        } else {
                            S::SetSurface
                        };
                        continue;
                    }
                    let window = layout.link(node)?.window;
                    self.node = node;
                    self.stage = S::NextChild;
                    break Request::Window(Call::new(window, 0xac, &[]));
                }
                S::SetSurface => {
                    let F::Visibility { option, .. } = self.function else {
                        unreachable!()
                    };
                    let has_surface = s.resources.geometry.layout.window(self.owner)?.surface != 0;
                    let f = s.fields_mut(self.owner)?;
                    f.option = option as u8;
                    f.had_surface = u8::from(has_surface);
                    self.stage = S::Configure;
                    if !has_surface {
                        break Request::Window(Call::new(self.owner, 0x174, &[1, 0]));
                    }
                }
                S::Configure => {
                    let F::Visibility { show, .. } = self.function else {
                        unreachable!()
                    };
                    let show = show as u8 != 0;
                    let now = s.now;
                    let f = s.fields_mut(self.owner)?;
                    f.direction = if show { 1 } else { -1 };
                    f.curve = curve::configure(
                        if show { 0 } else { 0x3f800000 },
                        show,
                        self.duration,
                        now,
                    );
                    self.stage = S::InvalidateParent;
                    if show {
                        break Request::Window(Call::new(self.owner, 0x94, &[]));
                    }
                }
                S::InvalidateParent => {
                    self.stage = S::InvalidateOne;
                    break Request::Window(Call::new(self.owner, 0x18, &[]));
                }
                S::InvalidateOne => {
                    self.stage = if self.word() == 0 {
                        S::InvalidateSelf
                    } else {
                        S::InvalidateTwo
                    };
                    if self.stage == S::InvalidateTwo {
                        break Request::Window(Call::new(self.owner, 0x18, &[]));
                    }
                }
                S::InvalidateTwo => {
                    self.parent = self.word();
                    self.stage = S::InvalidateSelf;
                    break Request::Window(Call::new(self.parent, 0x168, &[]));
                }
                S::InvalidateSelf => {
                    self.stage = S::Complete;
                    break Request::Window(Call::new(self.owner, 0x168, &[]));
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
