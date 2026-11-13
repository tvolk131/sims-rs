//! Original 0x44d96f Burn controller. Physical queries are synchronous; reset,
//! factory, virtual placement and destruction remain explicit host services so
//! their callbacks can suspend without changing the native ordering.
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;

pub const FIRE_GUID: u32 = 0x24c95f99;
pub mod live;
pub mod world;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Object {
    pub position: [i32; 2],
    pub level: i32,
    pub tile: [i8; 3],
    pub direction: i16,
    pub flags: u16,
    pub flags2: u16,
    pub uses: i16,
    pub kind: i16,
    pub master: bool,
    pub group_excluded: bool,
    pub head: Option<ObjectId>,
    pub next: Option<ObjectId>,
    pub fire_resource: bool,
    /// Native right, bottom, left, top footprint fields.
    pub bounds: [i32; 4],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Wall {
    pub mask: u32,
    pub styles: [i16; 6],
}

/// Reads actual physical state; tile traversal includes contained objects and
/// must preserve the native slot-tree order. A snapshot of the initial world
/// is insufficient once service callbacks start changing objects.
pub trait World {
    fn size(&self) -> usize;
    fn object(&self, id: ObjectId) -> Result<Option<Object>, String>;
    fn on_lot(&mut self, id: ObjectId) -> Result<bool, String>;
    fn tile(&mut self, tile: [i8; 3]) -> Result<Vec<ObjectId>, String>;
    fn wall(&mut self, tile: [i8; 3]) -> Result<Wall, String>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Input {
    pub target: ObjectId,
    pub mode: u8,
    pub flags: u8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Position {
    pub xy: [i32; 2],
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Resource,
    Reset(ObjectId),
    Place(ObjectId, Position),
    Refresh(ObjectId),
    Factory,
    Check(ObjectId, Position),
    Delete(ObjectId),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Response {
    Resource(bool),
    Created(Option<ObjectId>),
    Accepted(bool),
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Start,
    Resource,
    Select,
    People,
    Reset(ObjectId, Position),
    PersonPlace(ObjectId, Position),
    Refresh(ObjectId),
    Fires,
    Factory,
    Capture(ObjectId),
    Check(ObjectId, Position),
    PlaceCapture(ObjectId, Position),
    Place(ObjectId, Position),
    Delete(ObjectId),
    Complete(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Execution {
    input: Input,
    phase: Phase,
    resource: bool,
    affected: Vec<ObjectId>,
    cursor: usize,
    direct: Option<Position>,
    placed: bool,
    pending: Option<Request>,
}

fn object(w: &impl World, id: ObjectId) -> Result<Object, String> {
    w.object(id)?
        .ok_or_else(|| format!("Burn object {id} disappeared"))
}

fn aligned(o: &Object) -> Position {
    Position {
        xy: [(o.position[0] & !7) | 8, (o.position[1] & !15) | 8],
        level: o.level,
    }
}

impl Execution {
    pub fn new(input: Input) -> Self {
        Self {
            input,
            phase: Phase::Start,
            resource: false,
            affected: vec![],
            cursor: 0,
            direct: None,
            placed: false,
            pending: None,
        }
    }
    pub fn result(&self) -> Option<bool> {
        if let Phase::Complete(result) = self.phase {
            Some(result)
        } else {
            None
        }
    }
    pub fn input(&self) -> Input {
        self.input
    }
    pub fn affected(&self) -> &[ObjectId] {
        &self.affected
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }

    /// Structural checks independent of mutable world state. A callback may
    /// have moved a selected object; saved coordinates deliberately need not
    /// match its current position. The live host validates service ownership.
    pub fn validate(&self) -> Result<(), String> {
        use Phase::*;
        let unique: std::collections::BTreeSet<_> = self.affected.iter().copied().collect();
        if self.cursor > self.affected.len()
            || self.affected.iter().any(|id| *id <= 0)
            || unique.len() != self.affected.len()
            || self.direct.is_some() && (!self.affected.is_empty() || self.cursor != 0)
        {
            return Err("invalid Burn affected-object continuation".into());
        }
        let expected = match self.phase {
            Resource => Some(Request::Resource),
            Reset(id, _) => Some(Request::Reset(id)),
            PersonPlace(id, at) | Place(id, at) => Some(Request::Place(id, at)),
            Refresh(id) => Some(Request::Refresh(id)),
            Factory => Some(Request::Factory),
            Check(id, at) => Some(Request::Check(id, at)),
            Delete(id) => Some(Request::Delete(id)),
            _ => None,
        };
        if self.pending.is_some() && self.pending != expected {
            return Err("Burn pending service does not match its phase".into());
        }
        match self.phase {
            Start | Resource | Select
                if !self.affected.is_empty()
                    || self.cursor != 0
                    || self.direct.is_some()
                    || self.placed
                    || matches!(self.phase, Start | Resource) && self.resource =>
            {
                return Err("invalid Burn selection continuation".into());
            }
            People | Fires
                if self.direct.is_some()
                    || self.affected.is_empty()
                    || self.phase == People && self.placed =>
            {
                return Err("invalid Burn traversal continuation".into());
            }
            Reset(id, _) | PersonPlace(id, _) | Refresh(id)
                if self.direct.is_some()
                    || self.affected.get(self.cursor) != Some(&id)
                    || self.placed =>
            {
                return Err("invalid Burn person service binding".into());
            }
            Factory | Capture(_) | Check(_, _) | PlaceCapture(_, _) | Place(_, _) | Delete(_)
                if self.direct.is_none() && self.cursor >= self.affected.len() =>
            {
                return Err("Burn fire traversal has no current object".into());
            }
            _ => {}
        }
        if matches!(self.phase, Capture(id) | Check(id, _) | PlaceCapture(id, _) | Place(id, _) | Delete(id) if id <= 0)
            || matches!(self.phase, PlaceCapture(_, _)) && self.direct.is_some()
            || self.phase == Complete(true) && !self.placed
        {
            return Err("invalid Burn fire service binding".into());
        }
        Ok(())
    }

    pub fn next(&mut self, w: &mut impl World) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.phase {
                Phase::Start => {
                    if w.object(self.input.target)?.is_none() {
                        return Err("Burn native error 0x15: Stack Object absent".into());
                    }
                    self.phase = if w.on_lot(self.input.target)? {
                        Phase::Resource
                    } else {
                        Phase::Complete(false)
                    };
                    continue;
                }
                Phase::Resource => Request::Resource,
                Phase::Select => {
                    if !self.select(w)? {
                        self.phase = Phase::Complete(false);
                    } else if self.direct.is_some() {
                        self.phase = Phase::Factory;
                    } else {
                        self.phase = Phase::People;
                    }
                    continue;
                }
                Phase::People => {
                    if self.cursor == self.affected.len() {
                        self.cursor = 0;
                        self.phase = Phase::Fires;
                    } else {
                        let id = self.affected[self.cursor];
                        let o = object(w, id)?;
                        if o.kind == 2 {
                            self.phase = Phase::Reset(id, aligned(&o));
                        } else {
                            self.cursor += 1;
                        }
                    }
                    continue;
                }
                Phase::Reset(id, _) => Request::Reset(id),
                Phase::PersonPlace(id, at) => Request::Place(id, at),
                Phase::Refresh(id) => Request::Refresh(id),
                Phase::Fires => {
                    if self.cursor == self.affected.len() {
                        self.phase = Phase::Complete(self.placed);
                    } else {
                        let o = object(w, self.affected[self.cursor])?;
                        let mut exists = false;
                        for id in w.tile(o.tile)? {
                            if self.resource && object(w, id)?.fire_resource {
                                exists = true;
                                break;
                            }
                        }
                        if exists {
                            self.cursor += 1;
                        } else {
                            self.phase = Phase::Factory;
                        }
                    }
                    continue;
                }
                Phase::Factory => Request::Factory,
                Phase::Capture(id) => {
                    // Group coordinates are read AFTER the factory. The
                    // no-flammable-object branch uses its earlier capture.
                    let at = match self.direct {
                        Some(at) => at,
                        None => aligned(&object(w, self.affected[self.cursor])?),
                    };
                    self.phase = Phase::Check(id, at);
                    continue;
                }
                Phase::Check(id, at) => Request::Check(id, at),
                Phase::PlaceCapture(id, mut at) => {
                    // The native group branch re-reads the source's level
                    // after acceptance, but retains the captured XY pair.
                    at.level = object(w, self.affected[self.cursor])?.level;
                    self.phase = Phase::Place(id, at);
                    continue;
                }
                Phase::Place(id, at) => Request::Place(id, at),
                Phase::Delete(id) => Request::Delete(id),
                Phase::Complete(_) => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }

    pub fn respond(&mut self, response: Response) -> Result<(), String> {
        self.validate()?;
        if self.pending.is_none() {
            return Err("Burn response without a pending service".into());
        }
        let next = match (&self.phase, response) {
            (Phase::Resource, Response::Resource(available)) => {
                self.resource = available;
                Phase::Select
            }
            (Phase::Reset(id, at), Response::Done) => Phase::PersonPlace(*id, *at),
            (Phase::PersonPlace(id, _), Response::Done) => Phase::Refresh(*id),
            (Phase::Refresh(_), Response::Done) => {
                self.cursor += 1;
                Phase::People
            }
            (Phase::Factory, Response::Created(Some(id))) if id > 0 => Phase::Capture(id),
            // Native allocation failure returns false even after an earlier
            // successful fire; rejected placement instead advances the group.
            (Phase::Factory, Response::Created(None)) => Phase::Complete(false),
            (Phase::Check(id, at), Response::Accepted(true)) => {
                if self.direct.is_some() {
                    Phase::Place(*id, *at)
                } else {
                    Phase::PlaceCapture(*id, *at)
                }
            }
            (Phase::Check(id, _), Response::Accepted(false)) => Phase::Delete(*id),
            (Phase::Place(_, _), Response::Done) => {
                self.placed = true;
                if self.direct.is_some() {
                    Phase::Complete(true)
                } else {
                    self.cursor += 1;
                    Phase::Fires
                }
            }
            (Phase::Delete(_), Response::Done) => {
                if self.direct.is_some() {
                    Phase::Complete(false)
                } else {
                    self.cursor += 1;
                    Phase::Fires
                }
            }
            _ => return Err("Burn service response does not match pending phase".into()),
        };
        self.pending = None;
        self.phase = next;
        Ok(())
    }

    fn select(&mut self, w: &mut impl World) -> Result<bool, String> {
        let o = object(w, self.input.target)?;
        let xy = if self.input.mode == 1 {
            if !(0..8).contains(&o.direction) {
                return Err("Burn direction table index outside 0..8".into());
            }
            sims_vm::create::in_front(o.position, o.direction as u8)
        } else {
            o.position
        };
        let tile = [(xy[1] >> 4) as i8, (xy[0] >> 4) as i8, o.level as i8];
        if tile[0] <= 0
            || tile[1] <= 0
            || tile[0] as usize >= w.size().saturating_sub(1)
            || tile[1] as usize >= w.size().saturating_sub(1)
        {
            return Ok(false);
        }
        if self.input.mode == 1 {
            let wall = w.wall(tile)?;
            let direction = [8, 1, 2, 4][o.direction as usize / 2];
            if wall.mask & direction == direction
                && ![3, 5, 6, 15, 23].contains(&wall.styles[direction.trailing_zeros() as usize])
                || wall.mask & 0x30 != 0
            {
                return Ok(false);
            }
        }
        let mut last = None;
        for id in w.tile(tile)? {
            let q = object(w, id)?;
            if q.flags2 & 32 != 0 {
                last = Some(id);
            }
            if q.flags & 0x800 != 0 || self.resource && q.fire_resource {
                return Ok(false);
            }
        }
        let Some(id) = last else {
            self.direct = Some(Position { xy, level: o.level });
            return Ok(true);
        };
        self.affected.push(id);
        // Native restarts from the first candidate whenever the closure grows.
        // Group and same-tile scans may append several; adjacency appends one.
        'restart: loop {
            for index in 0..self.affected.len() {
                let id = self.affected[index];
                let q = object(w, id)?;
                if q.master && q.flags2 & 32 != 0 && !q.group_excluded {
                    let mut member = Some(q.head.unwrap_or(id));
                    let mut seen = std::collections::BTreeSet::new();
                    let before = self.affected.len();
                    while let Some(part) = member {
                        if !seen.insert(part) {
                            return Err("Burn cyclic multipart chain".into());
                        }
                        if !self.affected.contains(&part) {
                            self.affected.push(part);
                        }
                        member = object(w, part)?.next;
                    }
                    if self.affected.len() != before {
                        continue 'restart;
                    }
                }
                let before = self.affected.len();
                for part in w.tile(q.tile)? {
                    if object(w, part)?.flags2 & 32 != 0 && !self.affected.contains(&part) {
                        self.affected.push(part);
                    }
                }
                if self.affected.len() != before {
                    continue 'restart;
                }
                let [column, row, level] = q.tile;
                let x = i32::from(row) * 16;
                let y = i32::from(column) * 16;
                for (dc, dr) in [
                    (0, -1),
                    (1, -1),
                    (1, 0),
                    (1, 1),
                    (0, 1),
                    (-1, 1),
                    (-1, 0),
                    (-1, -1),
                ] {
                    for part in w.tile([column.wrapping_add(dc), row.wrapping_add(dr), level])? {
                        let item = object(w, part)?;
                        let [mut right, mut bottom, mut left, mut top] = item.bounds;
                        if item.kind == 2 {
                            // 44de25..44de2e expands the person's rectangle.
                            // The strict intersection still excludes an edge
                            // that only touches after this two-unit expansion.
                            right = right.wrapping_add(2);
                            bottom = bottom.wrapping_add(2);
                            left = left.wrapping_sub(2);
                            top = top.wrapping_sub(2);
                        }
                        if left < x + 16
                            && x < right
                            && top < y + 16
                            && y < bottom
                            && !self.affected.contains(&part)
                        {
                            self.affected.push(part);
                            continue 'restart;
                        }
                    }
                }
            }
            break;
        }
        for id in &self.affected {
            let q = object(w, *id)?;
            if q.flags & 0x800 != 0
                || self.input.flags & 1 == 0 && (q.flags & 32 != 0 || q.uses > 0)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
