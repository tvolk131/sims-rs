//!42231d/423175 cursor Reset ordering. Each request is a synchronous service:
//!its caller must complete the actual body before acknowledging. In particular,
//!BaseReset includes its own resource-dependent Init, optional Update and flags.
use crate::{ObjectId, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    BaseReset { object: u32, update: i16 },
    Offsets(u32),
    Init(u32),
    Update(u32),
    RegistryFlags(u32),
    Invalidate(u32),
    Flags { object: u32, low: u32 },
}
pub trait View {
    fn head(&self, object: u32) -> Result<Option<u32>, String>;
    fn next(&self, object: u32) -> Result<Option<u32>, String>;
    fn underlying(&self, object: u32) -> Result<Option<u32>, String>;
    ///Copy the current furniture's data23 and direction to the cursor, and
    ///return the furniture's current native flags. The flag service acts on
    ///the furniture; the copied script words belong to the cursor.
    fn copy_words(&mut self, cursor: u32, furniture: u32) -> Result<u32, String>;
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Enter,
    Delegated,
    Base,
    AfterBase,
    AfterOffsets,
    Init,
    AfterInit,
    AfterUpdate,
    AfterFlags,
    Post,
    Invalidate,
    Copy,
    AfterFurnitureFlags,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Frame {
    object: u32,
    stage: Stage,
    current: Option<u32>,
    visited: Vec<u32>,
}
impl Frame {
    fn new(object: u32) -> Self {
        Self {
            object,
            stage: Stage::Enter,
            current: None,
            visited: vec![],
        }
    }
    fn visit(&mut self) -> Result<(), String> {
        let current = self.current.ok_or("cursor reset iteration absent")?;
        if self.visited.contains(&current) {
            return Err("cyclic cursor reset traversal".into());
        }
        self.visited.push(current);
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    object: u32,
    update: i16,
    stack: Vec<Frame>,
    pending: Option<Request>,
}
impl Call {
    pub fn new(object: u32, update: i16) -> Result<Self, String> {
        if object == 0 {
            return Err("null cursor reset receiver".into());
        }
        Ok(Self {
            object,
            update,
            stack: vec![Frame::new(object)],
            pending: None,
        })
    }
    pub fn complete(&self) -> bool {
        self.stack.is_empty()
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn binding(&self) -> (u32, i16) {
        (self.object, self.update)
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        if self.object == 0
            || self.stack.len() > crate::MAX_STACK
            || self.stack.first().is_some_and(|f| f.object != self.object)
            || self.stack.iter().any(|f| {
                f.object == 0
                    || !seen.insert(f.object)
                    || f.current == Some(0)
                    || f.visited.contains(&0)
                    || f.visited
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != f.visited.len()
                    || match f.stage {
                        Stage::Enter | Stage::Delegated => {
                            f.current.is_some() || !f.visited.is_empty()
                        }
                        Stage::AfterOffsets => f.current.is_some(),
                        Stage::AfterBase
                        | Stage::AfterInit
                        | Stage::AfterUpdate
                        | Stage::AfterFlags
                        | Stage::Copy
                        | Stage::AfterFurnitureFlags => {
                            f.current.is_none() || f.visited.last().copied() != f.current
                        }
                        _ => false,
                    }
            })
            || self
                .stack
                .iter()
                .rev()
                .skip(1)
                .any(|f| f.stage != Stage::Delegated)
            || self.complete() && self.pending.is_some()
        {
            return Err("invalid cursor reset continuation".into());
        }
        if let Some(request) = self.pending {
            let f = self.stack.last().unwrap();
            let valid = match request {
                Request::BaseReset { object, update } => {
                    f.stage == Stage::AfterBase
                        && f.current == Some(object)
                        && update == self.update
                }
                Request::Offsets(object) => f.stage == Stage::AfterOffsets && f.object == object,
                Request::Init(object) => f.stage == Stage::AfterInit && f.current == Some(object),
                Request::Update(object) => {
                    f.stage == Stage::AfterUpdate && f.current == Some(object) && self.update != 0
                }
                Request::RegistryFlags(object) => {
                    f.stage == Stage::AfterFlags && f.current == Some(object)
                }
                Request::Invalidate(object) => f.stage == Stage::Copy && object != 0,
                Request::Flags { object, low } => {
                    f.stage == Stage::AfterFurnitureFlags
                        && object != 0
                        && low & !0x1f == 0
                        && low & 1 != 0
                }
            };
            if !valid {
                return Err("cursor reset request lost its owner".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("cursor reset service mismatch".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, view: &mut impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        while let Some(f) = self.stack.last_mut() {
            let request = match f.stage {
                Stage::Enter => {
                    if let Some(head) = view.head(f.object)? {
                        f.stage = Stage::Delegated;
                        if self.stack.iter().any(|f| f.object == head)
                            || self.stack.len() >= crate::MAX_STACK
                        {
                            return Err("recursive cursor reset head".into());
                        }
                        self.stack.push(Frame::new(head));
                    } else {
                        f.current = Some(f.object);
                        f.stage = Stage::Base;
                    }
                    continue;
                }
                Stage::Delegated => {
                    f.stage = Stage::Post;
                    continue;
                }
                Stage::Base => {
                    if let Some(object) = f.current {
                        f.visit()?;
                        f.stage = Stage::AfterBase;
                        Request::BaseReset {
                            object,
                            update: self.update,
                        }
                    } else {
                        f.stage = Stage::AfterOffsets;
                        Request::Offsets(f.object)
                    }
                }
                Stage::AfterBase => {
                    f.current = view.next(f.current.unwrap())?;
                    f.stage = Stage::Base;
                    continue;
                }
                Stage::AfterOffsets => {
                    f.current = Some(f.object);
                    f.visited.clear();
                    f.stage = Stage::Init;
                    continue;
                }
                Stage::Init => {
                    let Some(object) = f.current else {
                        f.stage = Stage::Post;
                        continue;
                    };
                    f.visit()?;
                    f.stage = Stage::AfterInit;
                    Request::Init(object)
                }
                Stage::AfterInit => {
                    if self.update != 0 {
                        f.stage = Stage::AfterUpdate;
                        Request::Update(f.current.unwrap())
                    } else {
                        f.stage = Stage::AfterFlags;
                        Request::RegistryFlags(f.current.unwrap())
                    }
                }
                Stage::AfterUpdate => {
                    f.stage = Stage::AfterFlags;
                    Request::RegistryFlags(f.current.unwrap())
                }
                Stage::AfterFlags => {
                    f.current = view.next(f.current.unwrap())?;
                    f.stage = Stage::Init;
                    continue;
                }
                Stage::Post => {
                    f.current = Some(view.head(f.object)?.unwrap_or(f.object));
                    f.visited.clear();
                    f.stage = Stage::Invalidate;
                    continue;
                }
                Stage::Invalidate => {
                    let Some(object) = f.current else {
                        //422414 only traverses links; it writes nothing.
                        let mut current = Some(view.head(f.object)?.unwrap_or(f.object));
                        let mut seen = std::collections::BTreeSet::new();
                        while let Some(object) = current {
                            if !seen.insert(object) {
                                return Err("cyclic cursor reset final walk".into());
                            }
                            current = view.next(object)?;
                        }
                        self.stack.pop();
                        continue;
                    };
                    f.visit()?;
                    if let Some(target) = view.underlying(object)? {
                        f.stage = Stage::Copy;
                        Request::Invalidate(target)
                    } else {
                        f.current = view.next(object)?;
                        continue;
                    }
                }
                Stage::Copy => {
                    let cursor = f.current.unwrap();
                    let object = view
                        .underlying(cursor)?
                        .ok_or("cursor furniture cleared during invalidation")?;
                    let low = (view.copy_words(cursor, object)? & 0x1e) | 1;
                    f.stage = Stage::AfterFurnitureFlags;
                    Request::Flags { object, low }
                }
                Stage::AfterFurnitureFlags => {
                    f.current = view.next(f.current.unwrap())?;
                    f.stage = Stage::Invalidate;
                    continue;
                }
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
        Ok(None)
    }
}

///Borrow the actual registered objects. Native pointers are retained lifetime
///handles, so recycling a script ID cannot redirect a cursor to new furniture.
pub struct Borrowed<'a> {
    pub world: &'a mut World,
}
impl Borrowed<'_> {
    fn id(&self, h: u32) -> Result<ObjectId, String> {
        super::id(self.world, h)
    }
    fn handle(&self, id: ObjectId) -> Result<u32, String> {
        self.world
            .scheduler_storage
            .as_ref()
            .and_then(|s| s.handle(id))
            .ok_or("cursor reset linked lifetime absent".into())
    }
    fn part(&self, h: u32) -> Result<&crate::multitile::Part, String> {
        let o = &self.world.objects[&self.id(h)?];
        if o.cursor.is_none() {
            return Err("cursor reset chain contains furniture".into());
        }
        o.multitile
            .as_ref()
            .ok_or("cursor reset multipart storage absent".into())
    }
}
impl View for Borrowed<'_> {
    fn head(&self, h: u32) -> Result<Option<u32>, String> {
        self.part(h)?.head.map(|id| self.handle(id)).transpose()
    }
    fn next(&self, h: u32) -> Result<Option<u32>, String> {
        self.part(h)?.next.map(|id| self.handle(id)).transpose()
    }
    fn underlying(&self, h: u32) -> Result<Option<u32>, String> {
        self.part(h)?;
        Ok(self.world.objects[&self.id(h)?]
            .cursor
            .as_ref()
            .unwrap()
            .underlying)
    }
    fn copy_words(&mut self, c: u32, f: u32) -> Result<u32, String> {
        let c = self.id(c)?;
        let f = self.id(f)?;
        let o = &self.world.objects[&f];
        let value = *o.data.get(&23).ok_or("cursor furniture word23 absent")?;
        let direction = *o.data.get(&1).ok_or("cursor furniture direction absent")?;
        let flags = o
            .native_flags
            .ok_or("cursor furniture native flags absent")?;
        self.world
            .objects
            .get_mut(&c)
            .unwrap()
            .data
            .extend([(23, value), (1, direction)]);
        Ok(flags)
    }
}
