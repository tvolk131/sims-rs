//! Base window flags5dc083, Show5de71a, Hide5de727, focus request436d71
//! and manager focus switch5d9987. Virtual children and focus events remain
//! explicit. The pure getters/invalidation below implement their concrete base
//! bodies; an overriding window class must supply its own virtual service.
use super::visibility::Call;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub parent: u32, //+40
    pub dirty: u8,   //+60
    pub flags: u32,  //+9c
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manager {
    pub focus: u32,           //+24
    pub root: u32,            //+2c
    pub special: u32,         //+1c0, membership fast path
    pub registered: Vec<u32>, //+34..38
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub global_manager: u32, //6fe944
    pub windows: BTreeMap<u32, Fields>,
    pub managers: BTreeMap<u32, Manager>,
}
impl State {
    pub fn window(&self, id: u32) -> Result<&Fields, String> {
        self.windows
            .get(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "base window owner absent".into())
    }
    pub fn manager(&self, id: u32) -> Result<&Manager, String> {
        self.managers
            .get(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "window manager owner absent".into())
    }
    fn window_mut(&mut self, id: u32) -> Result<&mut Fields, String> {
        self.windows
            .get_mut(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "base window owner absent".into())
    }
    fn manager_mut(&mut self, id: u32) -> Result<&mut Manager, String> {
        self.managers
            .get_mut(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "window manager owner absent".into())
    }
}
///436fef tests any overlapping bit, not whether every requested bit is set.
pub fn flags(fields: &Fields, mask: u32) -> u32 {
    u32::from(fields.flags & mask != 0)
}
///436e78, no callback or traversal.
pub fn parent(fields: &Fields) -> u32 {
    fields.parent
}
///5de715, no recursive invalidation in this concrete base implementation.
pub fn invalidate(fields: &mut Fields) {
    fields.dirty = 1;
}

///5dc9a9's two focus-message paths. Other messages are outside this helper's
/// scope. The caller forwards only word1 to the actual virtual override.
pub fn focus_event(window: u32, words: [u32; 4]) -> Option<Call> {
    let slot = match words[0] {
        20 => 0x134,
        21 => 0x130,
        _ => return None,
    };
    Some(Call::new(window, slot, &[words[1]]))
}
///4663ca is the concrete base/ViewControl handler for both virtual130/134.
/// Its argument is ignored and AL=1; other window classes may override it.
pub fn accept_focus(_other: u32) -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Flags { mask: u32, value: u32 },
    Show,
    Hide,
    TakeFocus,
    SwitchFocus { target: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Virtual(Call),
    TakeFocus(u32),                            //436d71
    SwitchFocus { manager: u32, window: u32 }, //5d9987
    Registered { manager: u32, window: u32 },  //5da49f
    Event { window: u32, words: [u32; 4] },    //virtual0, messages20/21
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Word(u32),
    Registered(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    pc: u8,
    old: u32,
    reply: Reply,
    result: Option<u32>,
    pending: Option<Request>,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            pc: 0,
            old: 0,
            reply: Reply::Done,
            result: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.pc == 255 && self.pending.is_none()
    }
    pub fn result(&self) -> Option<u32> {
        self.result
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        use Function as F;
        let word = matches!(self.reply, Reply::Word(_));
        let present = matches!(self.reply, Reply::Registered(_));
        let max = match self.function {
            F::Flags { .. } => 7,
            F::Show => 1,
            F::Hide => 4,
            F::TakeFocus => 2,
            F::SwitchFocus { .. } => 5,
        };
        let valid = if self.pc == 255 {
            self.pending.is_none()
                && self.result.is_none() == matches!(self.function, F::Show | F::Hide)
        } else if self.pc > max || self.result.is_some() {
            false
        } else if let Some(r) = &self.pending {
            let virtual_call =
                |slot, args: &[u32]| *r == Request::Virtual(Call::new(self.owner, slot, args));
            match (self.function, self.pc, r) {
                (F::Flags { mask: 128, value }, 1, _) => {
                    value as u8 != 0 && virtual_call(0x8c, &[1])
                }
                (F::Flags { mask: 1, .. }, 2, _) => virtual_call(0x8c, &[128]),
                (F::Flags { mask: 1, value }, 3, _) => virtual_call(0x174, &[value, 0]),
                (F::Flags { mask: 128, value }, 4, _) => {
                    let enabled = if value as u8 == 0 {
                        Some(0)
                    } else if let Reply::Word(v) = self.reply {
                        Some(u32::from(v as u8 != 0))
                    } else {
                        None
                    };
                    enabled.is_some_and(|v| virtual_call(0x174, &[v, 0]))
                }
                (F::Flags { mask: 1, .. }, 4, Request::Virtual(c)) => {
                    matches!(self.reply,Reply::Word(p) if *c==Call::new(p,0x168,&[]))
                }
                (F::Flags { mask: 1, .. }, 5, _) => virtual_call(0x168, &[]),
                (F::Flags { mask: 1, .. }, 6 | 7, _) => virtual_call(0x18, &[]),
                (F::Show, 1, _) => virtual_call(0x90, &[1, 1]),
                (F::Hide, 1, _) => virtual_call(0x90, &[1, 0]),
                (F::Hide, 2 | 3, _) => virtual_call(0x18, &[]),
                (F::Hide, 4, Request::TakeFocus(_)) => true,
                (F::TakeFocus, 1, _) => virtual_call(0x8c, &[0x2000]),
                (F::TakeFocus, 2, Request::SwitchFocus { window, .. }) => *window == self.owner,
                (F::SwitchFocus { .. }, 1, Request::Registered { manager, window }) => {
                    *manager == self.owner && *window == self.old
                }
                (F::SwitchFocus { target }, 2, Request::Event { window, words }) => {
                    *window == self.old && *words == [20, target, 0, 0]
                }
                (F::SwitchFocus { target }, 3, Request::Registered { manager, window }) => {
                    *manager == self.owner && *window == target
                }
                (F::SwitchFocus { target }, 4, Request::Event { window, words }) => {
                    *window == target && *words == [21, self.old, 0, 0]
                }
                (F::SwitchFocus { .. }, 5, Request::Virtual(c)) => {
                    c.slot == 0x18 && c.arguments.is_empty()
                }
                _ => false,
            }
        } else {
            match (self.function, self.pc) {
                (F::Flags { .. }, 1 | 2 | 6 | 7) | (F::Hide, 2 | 3) | (F::TakeFocus, 1 | 2) => word,
                (F::SwitchFocus { .. }, 1 | 3) => present,
                _ => true,
            }
        };
        if valid {
            Ok(())
        } else {
            Err("base window invocation lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let kind = match r {
            Request::Virtual(c) if matches!(c.slot, 0x18 | 0x8c | 0x90) => {
                matches!(reply, Reply::Word(_))
            }
            Request::TakeFocus(_) | Request::SwitchFocus { .. } => matches!(reply, Reply::Word(_)),
            Request::Registered { .. } => matches!(reply, Reply::Registered(_)),
            _ => reply == Reply::Done,
        };
        if self.pending.as_ref() != Some(r) || !kind {
            return Err("base window service reply differs".into());
        }
        self.pending = None;
        self.reply = reply;
        Ok(())
    }
    fn word(&self) -> u32 {
        let Reply::Word(v) = self.reply else {
            unreachable!()
        };
        v
    }
    fn present(&self) -> bool {
        let Reply::Registered(v) = self.reply else {
            unreachable!()
        };
        v
    }
    fn finish(&mut self, result: Option<u32>) -> Option<Request> {
        self.pc = 255;
        self.result = result;
        None
    }
    pub fn advance(&mut self, s: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        if self.complete() {
            return Ok(None);
        }
        use Function as F;
        let v = |slot, args: &[u32]| Request::Virtual(Call::new(self.owner, slot, args));
        let r = loop {
            match self.function {
                F::Flags { mask, value } => match self.pc {
                    0 => {
                        let f = &mut s.window_mut(self.owner)?.flags;
                        let overlap = *f & mask != 0;
                        let change = if value as u8 != 0 {
                            *f |= mask;
                            !overlap
                        } else {
                            *f &= !mask;
                            overlap
                        };
                        if !change || !matches!(mask, 1 | 128) {
                            return Ok(self.finish(Some(*f)));
                        }
                        if mask == 1 {
                            self.pc = 2;
                            break v(0x8c, &[128]);
                        }
                        if value as u8 != 0 {
                            self.pc = 1;
                            break v(0x8c, &[1]);
                        }
                        self.pc = 4;
                        break v(0x174, &[0, 0]);
                    }
                    1 => {
                        self.pc = 4;
                        break v(0x174, &[u32::from(self.word() as u8 != 0), 0]);
                    }
                    2 => {
                        self.pc = 3;
                        if self.word() as u8 != 0 {
                            break v(0x174, &[value, 0]);
                        }
                    }
                    3 => {
                        self.pc = 5;
                        break v(0x168, &[]);
                    }
                    4 => return Ok(self.finish(Some(s.window(self.owner)?.flags))),
                    5 => {
                        self.pc = 6;
                        break v(0x18, &[]);
                    }
                    6 => {
                        if self.word() == 0 {
                            return Ok(self.finish(Some(s.window(self.owner)?.flags)));
                        }
                        self.pc = 7;
                        break v(0x18, &[]);
                    }
                    7 => {
                        self.pc = 4;
                        break Request::Virtual(Call::new(self.word(), 0x168, &[]));
                    }
                    _ => unreachable!(),
                },
                F::Show => match self.pc {
                    0 => {
                        self.pc = 1;
                        break v(0x90, &[1, 1]);
                    }
                    1 => return Ok(self.finish(None)),
                    _ => unreachable!(),
                },
                F::Hide => match self.pc {
                    0 => {
                        self.pc = 1;
                        break v(0x90, &[1, 0]);
                    }
                    1 => {
                        if s.manager(s.global_manager)?.focus != self.owner {
                            return Ok(self.finish(None));
                        }
                        self.pc = 2;
                        break v(0x18, &[]);
                    }
                    2 => {
                        if self.word() != 0 {
                            self.pc = 3;
                            break v(0x18, &[]);
                        }
                        self.pc = 4;
                        break Request::TakeFocus(s.manager(s.global_manager)?.root);
                    }
                    3 => {
                        self.pc = 4;
                        break Request::TakeFocus(self.word());
                    }
                    4 => return Ok(self.finish(None)),
                    _ => unreachable!(),
                },
                F::TakeFocus => match self.pc {
                    0 => {
                        self.pc = 1;
                        break v(0x8c, &[0x2000]);
                    }
                    1 => {
                        if self.word() as u8 != 0 {
                            return Ok(self.finish(Some(s.manager(s.global_manager)?.focus)));
                        }
                        self.pc = 2;
                        break Request::SwitchFocus {
                            manager: s.global_manager,
                            window: self.owner,
                        };
                    }
                    2 => return Ok(self.finish(Some(self.word()))),
                    _ => unreachable!(),
                },
                F::SwitchFocus { target } => match self.pc {
                    0 => {
                        self.old = s.manager(self.owner)?.focus;
                        self.pc = 1;
                        break Request::Registered {
                            manager: self.owner,
                            window: self.old,
                        };
                    }
                    1 => {
                        self.pc = 2;
                        if self.present() {
                            break Request::Event {
                                window: self.old,
                                words: [20, target, 0, 0],
                            };
                        }
                    }
                    2 => {
                        s.manager_mut(self.owner)?.focus = target;
                        if target == 0 {
                            return Ok(self.finish(Some(0)));
                        }
                        self.pc = 3;
                        break Request::Registered {
                            manager: self.owner,
                            window: target,
                        };
                    }
                    3 => {
                        self.pc = 4;
                        if self.present() {
                            break Request::Event {
                                window: target,
                                words: [21, self.old, 0, 0],
                            };
                        }
                    }
                    4 => {
                        let current = s.manager(self.owner)?.focus;
                        self.pc = 5;
                        break Request::Virtual(Call::new(current, 0x18, &[]));
                    }
                    5 => return Ok(self.finish(Some(s.manager(self.owner)?.focus))),
                    _ => unreachable!(),
                },
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
