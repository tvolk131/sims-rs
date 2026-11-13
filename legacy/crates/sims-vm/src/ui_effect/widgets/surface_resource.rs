//! Surface ownership5dd563, inherited drawing-surface binding5dd3cf, and
//! binding propagation5dd413. Allocation/format/palette methods are backend
//! services. Delayed resource calls retain their original vtable target even
//! when a callback changes the window's current resource owner.
use super::{surface_geometry, visibility::Call};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub table: u32,
    pub size: [i32; 2], //+1c,+20
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub geometry: surface_geometry::State,
    pub drawing: BTreeMap<u32, u32>, //window+5c
    pub backend: u32,                //global6fe90c
    pub resources: BTreeMap<u32, Resource>,
    /// Returned format identities. The two words are read after the format
    /// callback, including when it returns its caller-owned scratch result.
    pub formats: BTreeMap<u32, [u32; 2]>,
}
impl State {
    fn owner_surface(&self, owner: u32) -> Result<u32, String> {
        Ok(self.geometry.layout.window(owner)?.surface)
    }
    fn set_surface(&mut self, owner: u32, value: u32) -> Result<(), String> {
        self.geometry
            .layout
            .windows
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or("surface window absent")?
            .surface = value;
        Ok(())
    }
    fn set_drawing(&mut self, owner: u32, value: u32) -> Result<(), String> {
        *self
            .drawing
            .get_mut(&owner)
            .filter(|_| owner != 0)
            .ok_or("drawing-surface field absent")? = value;
        Ok(())
    }
    fn resource(&self, id: u32) -> Result<Resource, String> {
        self.resources
            .get(&id)
            .copied()
            .filter(|_| id != 0)
            .ok_or_else(|| "window surface resource absent".into())
    }
    fn dimensions(&self, owner: u32) -> Result<[i32; 2], String> {
        let r = self.geometry.layout.window(owner)?.rectangle;
        Ok([r[2].wrapping_sub(r[0]), r[3].wrapping_sub(r[1])])
    }
    fn call(&self, owner: u32, slot: u16, arguments: &[u32]) -> Result<Request, String> {
        Ok(Request::Surface {
            owner,
            table: self.resource(owner)?.table,
            slot,
            arguments: arguments.to_vec(),
        })
    }
}
///5e7956 returns the singleton object itself, not a global pointer load.
pub const fn palette_owner() -> u32 {
    0x6fea08
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Function {
    Ownership { enabled: u32, option: u32 },
    Bind,
    RefreshBindings,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Window(Call),
    BackendFactory {
        backend: u32,
    }, //virtual1c
    FactoryCreate {
        factory: u32,
    }, //virtual4
    /// Factory virtual0c with a two-word output scratch; the reply identifies
    /// the returned format in State::formats (which need not be that scratch).
    FactoryFormat {
        factory: u32,
    },
    Surface {
        owner: u32,
        table: u32,
        slot: u16,
        arguments: Vec<u32>,
    },
    Palette {
        owner: u32,
        index: u32,
    }, //virtual44
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Word(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    CreateFactory,
    CreateResult,
    Ready,
    ReadyResult,
    Dimensions,
    FormatFactory,
    FormatResult,
    CreateBuffer,
    CreateBufferResult,
    PaletteFlag,
    PaletteValue,
    PaletteFinish,
    DisposeFailure,
    ClearFailure,
    DisableReady,
    DisposeDisabled,
    ClearDisabled,
    Publish,
    Invalidate,
    Finish,
    BindSelf,
    BindParent,
    BindSurface,
    BindRefresh,
    First,
    Next,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invocation {
    owner: u32,
    function: Function,
    stage: Stage,
    answer: Reply,
    pending: Option<Request>,
    size: [i32; 2],
    table: u32,
    parent: u32,
    node: u32,
    success: bool,
    result: Option<u8>,
}
impl Invocation {
    pub fn new(owner: u32, function: Function) -> Self {
        Self {
            owner,
            function,
            stage: Stage::Entry,
            answer: Reply::Done,
            pending: None,
            size: [0; 2],
            table: 0,
            parent: 0,
            node: 0,
            success: false,
            result: None,
        }
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn result(&self) -> Option<u8> {
        self.result
    }
    pub fn validate(&self) -> Result<(), String> {
        use Function as F;
        use Stage as S;
        let stage = match self.function {
            F::Ownership { .. } => !matches!(
                self.stage,
                S::BindSelf | S::BindParent | S::BindSurface | S::BindRefresh | S::First | S::Next
            ),
            F::Bind => matches!(
                self.stage,
                S::Entry
                    | S::BindSelf
                    | S::BindParent
                    | S::BindSurface
                    | S::BindRefresh
                    | S::Complete
            ),
            F::RefreshBindings => matches!(self.stage, S::Entry | S::First | S::Next | S::Complete),
        };
        let result = if matches!(self.function, F::Ownership { .. }) && self.stage == S::Complete {
            self.result == Some(u8::from(self.success))
        } else {
            self.result.is_none()
        };
        let child = match &self.pending {
            None => {
                !matches!(
                    self.stage,
                    S::CreateFactory
                        | S::CreateResult
                        | S::ReadyResult
                        | S::FormatFactory
                        | S::FormatResult
                        | S::CreateBuffer
                        | S::CreateBufferResult
                        | S::PaletteFlag
                        | S::PaletteValue
                        | S::DisableReady
                        | S::BindSelf
                        | S::BindParent
                        | S::BindSurface
                ) || matches!(self.answer, Reply::Word(_))
            }
            Some(r) => {
                let window =
                    |owner, slot, args: &[u32]| *r == Request::Window(Call::new(owner, slot, args));
                let resource = |slot, args: &[u32]| matches!(r,Request::Surface {owner,slot:s,arguments,..} if *owner!=0 && *s==slot && arguments==args);
                match (self.stage, r) {
                    (S::CreateFactory | S::FormatFactory, Request::BackendFactory { backend }) => {
                        *backend != 0
                    }
                    (S::CreateResult, Request::FactoryCreate { factory }) => *factory != 0,
                    (S::ReadyResult | S::DisableReady, _) => resource(0x2c, &[]),
                    (S::Dimensions | S::DisposeDisabled, _) => resource(0x10, &[]),
                    (S::FormatResult, Request::FactoryFormat { factory }) => *factory != 0,
                    (
                        S::CreateBufferResult,
                        Request::Surface {
                            table,
                            slot: 0xc,
                            arguments,
                            ..
                        },
                    ) => {
                        *table == self.table
                            && arguments.len() == 7
                            && arguments[0] == self.size[0] as u32
                            && arguments[1] == self.size[1] as u32
                            && arguments[4] == 0
                            && arguments[6] == 0
                            && matches!(self.function,F::Ownership {option,..} if arguments[5]==if option as u8!=0 {2}else{0})
                    }
                    (S::PaletteFlag, _) => window(self.owner, 0x8c, &[0x100]),
                    (S::PaletteValue, Request::Palette { owner, index: 0 }) => {
                        *owner == palette_owner()
                    }
                    (
                        S::PaletteFinish,
                        Request::Surface {
                            owner,
                            table,
                            slot: 0x68,
                            arguments,
                        },
                    ) => *owner != 0 && *table == self.table && arguments.len() == 1,
                    (S::ClearFailure | S::ClearDisabled, _) => resource(0, &[1]),
                    (S::Invalidate, _) => window(self.owner, 0x190, &[]),
                    (S::Finish, _) => window(self.owner, 0x168, &[]),
                    (S::BindSelf, _) => window(self.owner, 0x184, &[]),
                    (S::BindParent, _) => window(self.parent, 0x18, &[]),
                    (S::BindSurface, _) => self.parent != 0 && window(self.parent, 0x184, &[]),
                    (S::Complete, _) => self.function == F::Bind && window(self.owner, 0x198, &[]),
                    (S::First, _) => window(self.owner, 0x18c, &[]),
                    (S::Next, Request::Window(c)) => {
                        self.node != 0 && c.slot == 0x190 && c.arguments.is_empty()
                    }
                    _ => false,
                }
            }
        };
        if stage && result && child {
            Ok(())
        } else {
            Err("window surface ownership lost its continuation".into())
        }
    }
    pub fn acknowledge(&mut self, r: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        let word = match r {
            Request::Window(c) => matches!(c.slot, 0x18 | 0x184 | 0x8c),
            Request::Surface { slot, .. } => matches!(slot, 0x2c | 0xc),
            _ => true,
        };
        if self.pending.as_ref() != Some(r) || word != matches!(reply, Reply::Word(_)) {
            return Err("window surface ownership reply differs".into());
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
                S::Entry => match self.function {
                    F::Ownership { enabled, .. } => {
                        let surface = s.owner_surface(self.owner)?;
                        if enabled as u8 != 0 {
                            if surface == 0 {
                                if s.backend == 0 {
                                    return Err("window surface backend absent".into());
                                }
                                self.stage = S::CreateFactory;
                                break Request::BackendFactory { backend: s.backend };
                            }
                            self.stage = S::Ready;
                        } else if surface != 0 {
                            self.stage = S::DisableReady;
                            break s.call(surface, 0x2c, &[])?;
                        } else {
                            self.success = true;
                            self.stage = S::Publish;
                        }
                    }
                    F::Bind => {
                        self.stage = S::BindSelf;
                        break Request::Window(Call::new(self.owner, 0x184, &[]));
                    }
                    F::RefreshBindings => {
                        self.stage = S::First;
                        break Request::Window(Call::new(self.owner, 0x18c, &[]));
                    }
                },
                S::CreateFactory => {
                    let factory = self.word();
                    if factory == 0 {
                        return Err("surface factory absent".into());
                    }
                    self.stage = S::CreateResult;
                    break Request::FactoryCreate { factory };
                }
                S::CreateResult => {
                    s.set_surface(self.owner, self.word())?;
                    self.stage = S::Ready;
                }
                S::Ready => {
                    let surface = s.owner_surface(self.owner)?;
                    if surface == 0 {
                        self.stage = S::Publish;
                        continue;
                    }
                    self.stage = S::ReadyResult;
                    break s.call(surface, 0x2c, &[])?;
                }
                S::ReadyResult => {
                    self.stage = S::Dimensions;
                    if self.word() as u8 != 0 {
                        let current = s.owner_surface(self.owner)?;
                        if s.dimensions(self.owner)? == s.resource(current)?.size {
                            self.success = true;
                            self.result = Some(1);
                            self.stage = S::Complete;
                            continue;
                        }
                        break s.call(current, 0x10, &[])?;
                    }
                }
                S::Dimensions => {
                    let size = s.dimensions(self.owner)?;
                    if size[0] == 0 || size[1] == 0 {
                        self.stage = S::Publish;
                        continue;
                    }
                    if s.backend == 0 {
                        return Err("window surface backend absent".into());
                    }
                    self.stage = S::FormatFactory;
                    break Request::BackendFactory { backend: s.backend };
                }
                S::FormatFactory => {
                    let factory = self.word();
                    if factory == 0 {
                        return Err("surface format factory absent".into());
                    }
                    self.size = s.dimensions(self.owner)?;
                    self.table = s.resource(s.owner_surface(self.owner)?)?.table;
                    self.stage = S::FormatResult;
                    break Request::FactoryFormat { factory };
                }
                S::FormatResult => {
                    self.stage = S::CreateBuffer;
                }
                S::CreateBuffer => {
                    let format = *s
                        .formats
                        .get(&self.word())
                        .filter(|_| self.word() != 0)
                        .ok_or("returned surface format absent")?;
                    let F::Ownership { option, .. } = self.function else {
                        unreachable!()
                    };
                    let owner = s.owner_surface(self.owner)?;
                    s.resource(owner)?;
                    self.stage = S::CreateBufferResult;
                    break Request::Surface {
                        owner,
                        table: self.table,
                        slot: 0xc,
                        arguments: vec![
                            self.size[0] as u32,
                            self.size[1] as u32,
                            format[0],
                            format[1],
                            0,
                            if option as u8 != 0 { 2 } else { 0 },
                            0,
                        ],
                    };
                }
                S::CreateBufferResult => {
                    if self.word() as u8 == 0 {
                        self.stage = S::DisposeFailure;
                        continue;
                    }
                    self.stage = S::PaletteFlag;
                    break Request::Window(Call::new(self.owner, 0x8c, &[0x100]));
                }
                S::PaletteFlag => {
                    if self.word() as u8 == 0 {
                        self.success = true;
                        self.stage = S::Publish;
                        continue;
                    }
                    self.table = s.resource(s.owner_surface(self.owner)?)?.table;
                    self.stage = S::PaletteValue;
                    break Request::Palette {
                        owner: palette_owner(),
                        index: 0,
                    };
                }
                S::PaletteValue => {
                    let value = self.word();
                    let owner = s.owner_surface(self.owner)?;
                    s.resource(owner)?;
                    self.stage = S::PaletteFinish;
                    break Request::Surface {
                        owner,
                        table: self.table,
                        slot: 0x68,
                        arguments: vec![value],
                    };
                }
                S::PaletteFinish => {
                    self.success = true;
                    self.stage = S::Publish;
                }
                S::DisableReady => {
                    self.stage = S::DisposeDisabled;
                    if self.word() as u8 != 0 {
                        break s.call(s.owner_surface(self.owner)?, 0x10, &[])?;
                    }
                }
                S::DisposeFailure | S::DisposeDisabled => {
                    let surface = s.owner_surface(self.owner)?;
                    self.stage = if self.stage == S::DisposeFailure {
                        S::ClearFailure
                    } else {
                        S::ClearDisabled
                    };
                    if surface != 0 {
                        break s.call(surface, 0, &[1])?;
                    }
                }
                S::ClearFailure | S::ClearDisabled => {
                    s.set_surface(self.owner, 0)?;
                    self.success = self.stage == S::ClearDisabled;
                    self.stage = S::Publish;
                }
                S::Publish => {
                    self.stage = S::Invalidate;
                    break Request::Window(Call::new(self.owner, 0x190, &[]));
                }
                S::Invalidate => {
                    self.stage = S::Finish;
                    break Request::Window(Call::new(self.owner, 0x168, &[]));
                }
                S::Finish => {
                    self.result = Some(u8::from(self.success));
                    self.stage = S::Complete;
                }
                S::BindSelf => {
                    let surface = self.word();
                    s.set_drawing(self.owner, surface)?;
                    if surface != 0 {
                        self.stage = S::BindRefresh;
                        continue;
                    }
                    self.parent = self.owner;
                    self.stage = S::BindParent;
                    break Request::Window(Call::new(self.parent, 0x18, &[]));
                }
                S::BindParent => {
                    self.parent = self.word();
                    if self.parent == 0 {
                        self.stage = S::BindRefresh;
                        continue;
                    }
                    self.stage = S::BindSurface;
                    break Request::Window(Call::new(self.parent, 0x184, &[]));
                }
                S::BindSurface => {
                    let surface = self.word();
                    if surface != 0 {
                        s.set_drawing(self.owner, surface)?;
                        self.stage = S::BindRefresh;
                        continue;
                    }
                    self.stage = S::BindParent;
                    break Request::Window(Call::new(self.parent, 0x18, &[]));
                }
                S::BindRefresh => {
                    self.stage = S::Complete;
                    break Request::Window(Call::new(self.owner, 0x198, &[]));
                }
                S::First | S::Next => {
                    let sentinel = s.geometry.layout.window(self.owner)?.sentinel;
                    let node = s
                        .geometry
                        .layout
                        .link(if self.stage == S::First {
                            sentinel
                        } else {
                            self.node
                        })?
                        .next;
                    if node == sentinel {
                        self.stage = S::Complete;
                        continue;
                    }
                    let owner = s.geometry.layout.link(node)?.window;
                    self.node = node;
                    self.stage = S::Next;
                    break Request::Window(Call::new(owner, 0x190, &[]));
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r.clone());
        Ok(Some(r))
    }
}
