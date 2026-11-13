//! 53b2eb expansion. The caller owns the field write, captured panel and height,
//! and ordered virtual/layout children. Rectangles are borrowed after callbacks
//! at the original read points. Missing required owners are errors, not no-ops.
use crate::ui_effect::widgets::visibility::Call;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub value: i32,
    pub source: u32,
} //+1a0,+12c
pub struct Inputs<'a> {
    /// None means the global App itself is absent. Some(0) is App+88 null.
    pub app_panel: Option<u32>,
    pub policy: u8,                            //singleton6f2628+1
    pub camera: u32,                           //current global6e48bc+4
    pub screen: u32,                           //current global6e48bc+3c
    pub windows: &'a BTreeMap<u32, [i32; 4]>,  //window+7c
    pub surfaces: &'a BTreeMap<u32, [i32; 4]>, //surface+14
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Window(Call),
    Flag4000 { window: u32, value: u32 }, //5de0d0(1), recursive virtual90(flag4000,value)
    Viewport { camera: u32, height: i32 }, //4b136b
    Parent(u32),                          //virtual18, called twice on the positioning path
    Move { window: u32, x: i32, y: i32 }, //4cb0bb
    Invalidate(u32),                      //virtual168
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Parent(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Flag4000,
    PolicyHeight,
    ShownHeight,
    HiddenHeight,
    Parent,
    CheckParent,
    Move,
    Invalidate,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expansion {
    view: u32,
    value: i32,
    stage: Stage,
    panel: u32,
    height: Option<i32>,
    answer: Reply,
    pending: Option<Request>,
}
impl Expansion {
    pub fn new(view: u32, value: i32) -> Self {
        Self {
            view,
            value,
            stage: Stage::Entry,
            panel: 0,
            height: None,
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
        use Stage as S;
        let valid = match (self.stage, &self.pending) {
            (S::ShownHeight | S::HiddenHeight, _) if self.height.is_none() => false,
            (S::CheckParent | S::Move, None) => matches!(self.answer, Reply::Parent(_)),
            (_, None) => true,
            (S::Flag4000, Some(Request::Window(c))) => {
                *c == Call::new(self.panel, 0xa8, &[0, 0, 0])
            }
            (S::PolicyHeight, Some(Request::Flag4000 { window, value: 1 })) => {
                *window == self.panel
            }
            (S::ShownHeight, Some(Request::Window(c))) => *c == Call::new(self.panel, 0x94, &[]),
            (S::HiddenHeight, Some(Request::Window(c))) => *c == Call::new(self.panel, 0x98, &[]),
            (S::Parent, Some(Request::Window(c))) => *c == Call::new(self.panel, 0xa8, &[1, 0, 0]),
            (S::Parent, Some(Request::Viewport { .. })) => true,
            (S::CheckParent | S::Move, Some(Request::Parent(w))) => *w == self.view,
            (S::Invalidate, Some(Request::Move { window, x: 0, .. })) => *window == self.view,
            (S::Complete, Some(Request::Invalidate(w))) => *w == self.view,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("ViewControl expansion lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let kind = matches!((r, reply), (Request::Parent(_), Reply::Parent(_)))
            || (!matches!(r, Request::Parent(_)) && reply == Reply::Done);
        if self.pending.as_ref() != Some(r) || !kind {
            return Err("ViewControl expansion reply differs".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        i: Inputs<'_>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        use Stage as S;
        let screen_height = || -> Result<i32, String> {
            let r = i
                .surfaces
                .get(&i.screen)
                .filter(|_| i.screen != 0)
                .ok_or("expansion screen surface absent")?;
            Ok(r[3].wrapping_sub(r[1]))
        };
        let r = loop {
            match self.stage {
                S::Entry => {
                    if fields.value == self.value {
                        self.stage = S::Complete;
                        continue;
                    }
                    fields.value = self.value;
                    self.panel = i.app_panel.ok_or("expansion App owner absent")?;
                    if self.panel == 0 {
                        self.stage = S::Complete;
                        continue;
                    }
                    if i.policy != 0 {
                        let show = u32::from(fields.value == 0);
                        self.stage = if show != 0 { S::Parent } else { S::Flag4000 };
                        break Request::Window(Call::new(self.panel, 0xa8, &[show, 0, 0]));
                    }
                    self.height = Some(screen_height()?);
                    let show = fields.value == 0;
                    self.stage = if show {
                        S::ShownHeight
                    } else {
                        S::HiddenHeight
                    };
                    break Request::Window(Call::new(
                        self.panel,
                        if show { 0x94 } else { 0x98 },
                        &[],
                    ));
                }
                S::Flag4000 => {
                    self.stage = S::PolicyHeight;
                    break Request::Flag4000 {
                        window: self.panel,
                        value: 1,
                    };
                }
                S::PolicyHeight | S::ShownHeight | S::HiddenHeight => {
                    let height = match self.stage {
                        S::PolicyHeight => screen_height()?,
                        S::HiddenHeight => self.height.unwrap(),
                        S::ShownHeight => {
                            let r = i
                                .windows
                                .get(&self.panel)
                                .ok_or("expansion captured panel rectangle absent")?;
                            r[1].wrapping_sub(r[3])
                                .wrapping_add(self.height.unwrap())
                                .wrapping_add(50)
                        }
                        _ => unreachable!(),
                    };
                    self.stage = S::Parent;
                    break Request::Viewport {
                        camera: i.camera,
                        height,
                    };
                }
                S::Parent => {
                    self.stage = S::CheckParent;
                    break Request::Parent(self.view);
                }
                S::CheckParent => {
                    let Reply::Parent(parent) = self.answer else {
                        unreachable!()
                    };
                    if parent != 0 && fields.source != 0 {
                        self.stage = S::Move;
                        break Request::Parent(self.view);
                    }
                    self.stage = S::Invalidate;
                }
                S::Move => {
                    let Reply::Parent(parent) = self.answer else {
                        unreachable!()
                    };
                    let p = i
                        .windows
                        .get(&parent)
                        .filter(|_| parent != 0)
                        .ok_or("expansion second parent rectangle absent")?;
                    let source = i
                        .surfaces
                        .get(&fields.source)
                        .filter(|_| fields.source != 0)
                        .ok_or("expansion source surface absent")?;
                    let y = source[1]
                        .wrapping_sub(source[3])
                        .wrapping_sub(p[1])
                        .wrapping_add(p[3]);
                    self.stage = S::Invalidate;
                    break Request::Move {
                        window: self.view,
                        x: 0,
                        y,
                    };
                }
                S::Invalidate => {
                    self.stage = S::Complete;
                    break Request::Invalidate(self.view);
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
