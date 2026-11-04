//! Viewport height4b136b and renderer/window sizing49b0bf/5cf1b6/5cf206.
//! The view getter, renderer synchronization and backend virtual14 remain
//! explicit services. Backend output is a live reference to window+4; it is
//! not a copied window description. The host owns actual drawing resources.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub window: u32,         //+68
    pub renderer: u32,       //+54, used by the synchronization child
    pub surface: u32,        //+6c
    pub rectangle: [i32; 4], //+28
    pub retained: [i32; 4],  //+4cc
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renderer {
    pub window: u32, //+4
    pub width: i16,  //+196
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub origin: [i32; 2],    //+8,+c
    pub size: [i32; 2],      //+10,+14
    pub scale: [u32; 4],     //+18..24, float bits
    pub rectangle: [i32; 4], //+68
    pub backend: u32,        //+d8
    pub result: u32,         //+5c
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub views: BTreeMap<u32, View>,
    pub renderers: BTreeMap<u32, Renderer>,
    pub windows: BTreeMap<u32, Window>,
    ///5cb029 returns the same surface's embedded+48 dimensions.
    pub surfaces: BTreeMap<u32, [i32; 2]>,
    ///Root+4c owner's rectangles+2a4,+2b4.
    pub targets: BTreeMap<u32, [[i32; 4]; 2]>,
    pub target: u32,
}
impl State {
    fn view(&self, id: u32) -> Result<&View, String> {
        self.views
            .get(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "viewport view absent".into())
    }
    fn view_mut(&mut self, id: u32) -> Result<&mut View, String> {
        self.views
            .get_mut(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "viewport view absent".into())
    }
    fn window(&self, id: u32) -> Result<&Window, String> {
        self.windows
            .get(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "viewport window absent".into())
    }
    fn window_mut(&mut self, id: u32) -> Result<&mut Window, String> {
        self.windows
            .get_mut(&id)
            .filter(|_| id != 0)
            .ok_or_else(|| "viewport window absent".into())
    }
}
///5cf206 treats the stored dimensions as unsigned before multiplying by0.5.
/// All u32 values and their halves are exact in f64 before the single f32 store.
pub fn scales(size: [i32; 2]) -> [u32; 4] {
    [
        ((f64::from(size[0] as u32) * 0.5) as f32).to_bits(),
        ((f64::from(size[1] as u32) * 0.5) as f32).to_bits(),
        0x3f800000,
        0x3f800000,
    ]
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Height(i32),
    RendererHeight(i32),
    Resize([i32; 2]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Renderer(u32), //view virtual10
    RendererHeight {
        renderer: u32,
        height: i32,
    },
    Resize {
        window: u32,
        size: [i32; 2],
    },
    ///Concrete backend virtual14 receives its own identity and window+4.
    Backend {
        backend: u32,
        window: u32,
    },
    Synchronize(u32), //4a7b22, after local rectangle publication
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Renderer(u32),
    Backend(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Renderer,
    Geometry,
    Regions,
    Backend,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    stage: Stage,
    pending: Option<Request>,
    answer: Reply,
    old_height: i32,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            stage: Stage::Entry,
            pending: None,
            answer: Reply::Done,
            old_height: 0,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        use Function as F;
        use Stage as S;
        let phase = matches!(
            (self.function, self.stage),
            (_, S::Entry | S::Complete)
                | (F::Height(_), S::Renderer | S::Geometry | S::Regions)
                | (F::Resize(_), S::Backend)
        );
        let child = match (self.function, self.stage, self.pending) {
            (_, _, None) => match self.stage {
                S::Renderer => matches!(self.answer, Reply::Renderer(_)),
                S::Backend => matches!(self.answer, Reply::Backend(_)),
                _ => true,
            },
            (F::Height(_), S::Renderer, Some(Request::Renderer(v))) => v == self.owner,
            (F::Height(h), S::Geometry, Some(Request::RendererHeight { height, .. })) => {
                h == height
            }
            (F::Height(_), S::Regions, Some(Request::Synchronize(v))) => v == self.owner,
            (F::RendererHeight(h), S::Complete, Some(Request::Resize { size, .. })) => size[1] == h,
            (F::Resize(_), S::Backend, Some(Request::Backend { window, .. })) => {
                window == self.owner
            }
            _ => false,
        };
        if phase && child {
            Ok(())
        } else {
            Err("viewport invocation lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: Request, answer: Reply) -> Result<(), String> {
        self.validate()?;
        let kind = matches!(
            (r, answer),
            (Request::Renderer(_), Reply::Renderer(_))
                | (Request::Backend { .. }, Reply::Backend(_))
                | (
                    Request::RendererHeight { .. }
                        | Request::Resize { .. }
                        | Request::Synchronize(_),
                    Reply::Done
                )
        );
        if self.pending != Some(r) || !kind {
            return Err("viewport reply differs".into());
        }
        self.pending = None;
        self.answer = answer;
        Ok(())
    }
    pub fn advance(&mut self, s: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Function as F;
        use Stage as S;
        let r = loop {
            match self.stage {
                S::Entry => match self.function {
                    F::Height(_) => {
                        let w = s.window(s.view(self.owner)?.window)?;
                        self.old_height = w.rectangle[3].wrapping_sub(w.rectangle[1]);
                        self.stage = S::Renderer;
                        break Request::Renderer(self.owner);
                    }
                    F::RendererHeight(height) => {
                        let renderer = s
                            .renderers
                            .get(&self.owner)
                            .filter(|_| self.owner != 0)
                            .ok_or("viewport renderer absent")?;
                        self.stage = S::Complete;
                        break Request::Resize {
                            window: renderer.window,
                            size: [i32::from(renderer.width), height],
                        };
                    }
                    F::Resize(size) => {
                        let w = s.window_mut(self.owner)?;
                        w.size = size;
                        w.rectangle = [
                            w.origin[0],
                            w.origin[1],
                            w.origin[0].wrapping_add(size[0]),
                            w.origin[1].wrapping_add(size[1]),
                        ];
                        w.scale = scales(size);
                        self.stage = S::Backend;
                        break Request::Backend {
                            backend: w.backend,
                            window: self.owner,
                        };
                    }
                },
                S::Renderer => {
                    let Reply::Renderer(renderer) = self.answer else {
                        unreachable!()
                    };
                    let F::Height(height) = self.function else {
                        unreachable!()
                    };
                    self.stage = S::Geometry;
                    break Request::RendererHeight { renderer, height };
                }
                S::Geometry => {
                    let [l, t, r, b] = s.window(s.view(self.owner)?.window)?.rectangle;
                    s.view_mut(self.owner)?.rectangle =
                        [0, 0, r.wrapping_sub(l), b.wrapping_sub(t)];
                    self.stage = S::Regions;
                    break Request::Synchronize(self.owner);
                }
                S::Regions => {
                    let F::Height(height) = self.function else {
                        unreachable!()
                    };
                    let surface = s.view(self.owner)?.surface;
                    let [w, h] = *s
                        .surfaces
                        .get(&surface)
                        .filter(|_| surface != 0)
                        .ok_or("viewport surface absent")?;
                    s.view_mut(self.owner)?.retained = if h == height {
                        [0, h.wrapping_sub(self.old_height), w, h]
                    } else {
                        [0; 4]
                    };
                    let width = s.view(self.owner)?.rectangle[2];
                    let target = s
                        .targets
                        .get_mut(&s.target)
                        .filter(|_| s.target != 0)
                        .ok_or("viewport target absent")?;
                    *target = [
                        [0, 0, width, height],
                        [0, height, width, height.wrapping_mul(2)],
                    ];
                    self.stage = S::Complete;
                }
                S::Backend => {
                    let Reply::Backend(result) = self.answer else {
                        unreachable!()
                    };
                    s.window_mut(self.owner)?.result = result;
                    self.stage = S::Complete;
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
