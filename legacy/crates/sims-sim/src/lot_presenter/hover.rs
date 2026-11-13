//! Native cutaway room history and the4ab789 mouse sampling continuation.
//! The base mouse handler and virtual screen projection are caller services.
use super::room_masks::rotate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub capacity: usize,
    pub rooms: Vec<u32>,
}
impl History {
    ///4a6b15 constructs its cCutawaySet with capacity3.
    pub fn house_view() -> Self {
        Self {
            capacity: 3,
            rooms: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.capacity == 0 || self.capacity > 4096 || self.rooms.len() > self.capacity {
            return Err("invalid native cutaway history".into());
        }
        Ok(())
    }
    pub fn insert(&mut self, room: u32) -> Result<(), String> {
        self.validate()?;
        if self.rooms.contains(&room) {
            return Ok(());
        }
        if self.rooms.len() == self.capacity {
            self.rooms.pop();
        }
        self.rooms.insert(0, room);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.rooms.clear();
    }
    pub fn ids(&self) -> Vec<u16> {
        self.rooms.iter().map(|&n| n as u16).collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub history: History,
    pub screen_bounds: [i32; 4],
    pub mode: i32,
    pub cutaway: u8,
    pub level: i32,
    pub bottom: i32,
    pub floor_height: i32,
    pub pointer_active: u8,
    pub pointer: [i32; 2],
}
pub struct Geometry<'a> {
    pub size: usize,
    pub rotation: u8,
    pub bounds: [i32; 4],
    pub primary: &'a [i16],
}
impl Geometry<'_> {
    fn room(&self, p: [i32; 2], level: i32) -> Result<Option<u16>, String> {
        if self.rotation > 3
            || !(1..=64).contains(&self.size)
            || self.primary.len() != self.size * self.size
        {
            return Err("native mouse room geometry unavailable".into());
        }
        let tile = [(p[1] >> 4) as i8, (p[0] >> 4) as i8, level as i8];
        let [x, y, _] = tile.map(i32::from);
        //484a6e narrows to signed bytes before the strict interior check.
        if x <= self.bounds[0] || x >= self.bounds[1] || y <= self.bounds[2] || y >= self.bounds[3]
        {
            return Ok(None);
        }
        if !(0..64).contains(&x) || !(0..64).contains(&y) {
            return Err("native mouse tile outside rotation table".into());
        }
        let [x, y, _] = rotate(tile, 4u8.wrapping_sub(self.rotation) & 3);
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return Err("native mouse room cell unavailable".into());
        }
        //4899d5 returns the primary cell directly, including -5 split markers.
        Ok(Some(
            self.primary[x as usize * self.size + y as usize] as u16,
        ))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Project { screen: [i16; 2], flag: u8 },
    ComposeVisible,
    ClearMask,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Project,
    Finish,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mouse {
    screen: [i16; 2],
    base_reply: u8,
    first: Option<u16>,
    last: u16,
    offset: i32,
    samples: usize,
    stage: Stage,
    pending: Option<Request>,
}
impl Mouse {
    /// Begin AFTER476af0. Its raw byte and any event/view mutations must be
    /// supplied by the caller; no base mouse processing is silently fabricated.
    pub fn begin(
        view: &mut View,
        screen: [i16; 2],
        flags: u32,
        base_reply: u8,
    ) -> Result<Self, String> {
        view.history.validate()?;
        let mut m = Self {
            screen,
            base_reply,
            first: None,
            last: 0,
            offset: 0,
            samples: 0,
            stage: Stage::Done,
            pending: None,
        };
        let [y, x] = screen.map(i32::from);
        let b = view.screen_bounds;
        let inside = x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3];
        if view.mode == 4 {
            view.pointer_active = u8::from(inside);
            if inside {
                view.pointer = [x, y];
            }
        }
        if inside && flags as u16 == 0 && view.cutaway != 0 {
            m.stage = Stage::Project;
            m.pending = Some(Request::Project { screen, flag: 1 });
        }
        Ok(m)
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn complete(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }
    pub fn returned(&self) -> Option<u8> {
        self.complete().then_some(self.base_reply)
    }
    pub fn validate(&self, view: &View) -> Result<(), String> {
        view.history.validate()?;
        let okay = match (&self.stage, &self.pending) {
            (Stage::Done, None) => true,
            (Stage::Project, Some(Request::Project { screen, flag })) => {
                *flag == 1
                    && screen[1] == self.screen[1]
                    && screen[0] == self.screen[0].wrapping_add(self.offset as i16)
            }
            (Stage::Finish, Some(Request::ClearMask)) => self.first.is_none(),
            (Stage::Finish, Some(Request::ComposeVisible)) => self.first.is_some(),
            _ => false,
        };
        if !okay || self.samples > 4096 {
            return Err("invalid native mouse continuation".into());
        }
        Ok(())
    }
    pub fn projected(
        &mut self,
        view: &mut View,
        geometry: &Geometry<'_>,
        screen: [i16; 2],
        point: [i32; 2],
    ) -> Result<(), String> {
        self.validate(view)?;
        if self.pending != Some(Request::Project { screen, flag: 1 }) {
            return Err("native mouse projection acknowledgement mismatch".into());
        }
        let room = geometry.room(point, view.level)?;
        self.samples += 1;
        if self.samples > 4096 {
            return Err("native mouse sampling instruction budget exhausted".into());
        }
        if self.first.is_none() {
            let Some(room) = room else {
                self.stage = Stage::Finish;
                self.pending = Some(Request::ClearMask);
                return Ok(());
            };
            self.first = Some(room);
            self.last = room;
            self.offset = view.bottom / 2;
        } else {
            if let Some(room) = room {
                self.last = room;
            }
            if self.last == self.first.unwrap() {
                self.offset = self.offset.wrapping_add(view.bottom / 2);
            }
        }
        if self.last == self.first.unwrap() && self.offset < view.floor_height / 2 {
            self.pending = Some(Request::Project {
                screen: [
                    self.screen[0].wrapping_add(self.offset as i16),
                    self.screen[1],
                ],
                flag: 1,
            });
        } else {
            for room in [self.first.unwrap(), self.last] {
                if room < 65531 {
                    view.history.insert(u32::from(room))?;
                }
            }
            self.stage = Stage::Finish;
            self.pending = Some(Request::ComposeVisible);
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, view: &View, request: &Request) -> Result<(), String> {
        self.validate(view)?;
        if !matches!(self.stage, Stage::Finish) || self.pending.as_ref() != Some(request) {
            return Err("native mouse final service acknowledgement mismatch".into());
        }
        self.pending = None;
        self.stage = Stage::Done;
        Ok(())
    }
}

///4ac07b scans live mask bits in row-major order before clearing all rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clear {
    size: usize,
    origin: [i8; 2],
    row: usize,
    column: usize,
    pending: Option<[i8; 3]>,
    complete: bool,
}
impl Clear {
    pub fn pending(&self) -> Option<[i8; 3]> {
        self.pending
    }
    pub fn begin(size: usize, origin: [i8; 2]) -> Result<Self, String> {
        let c = Self {
            size,
            origin,
            row: 0,
            column: 0,
            pending: None,
            complete: false,
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.size > 64
            || self
                .origin
                .iter()
                .any(|&n| n < 0 || n as usize + self.size > 64)
            || self.row > self.size
            || self.column >= self.size.max(1)
            || (self.row == self.size && (self.column != 0 || self.pending.is_some()))
            || (self.complete && self.row != self.size)
        {
            return Err("invalid native cutaway clear cursor".into());
        }
        if let Some(p) = self.pending {
            if p[..2]
                != [
                    self.origin[0] + self.column as i8,
                    self.origin[1] + self.row as i8,
                ]
                || self.complete
            {
                return Err("invalid native cutaway clear request".into());
            }
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        state: &mut super::visibility::State,
        level: i32,
    ) -> Result<Option<[i8; 3]>, String> {
        self.validate()?;
        if state.mask.first_row != 0 || state.mask.rows.len() != 64 {
            return Err("native clear mask unavailable".into());
        }
        if self.complete {
            return Ok(None);
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        while self.row < self.size {
            let x = self.origin[0] as usize + self.column;
            let y = self.origin[1] as usize + self.row;
            if state.mask.rows[y] & (1u64 << x) != 0 {
                self.pending = Some([x as i8, y as i8, level as i8]);
                return Ok(self.pending);
            }
            self.next();
        }
        state.mask.rows.fill(0);
        state.include_visible = 0;
        state.dirty = 1;
        self.complete = true;
        Ok(None)
    }
    fn next(&mut self) {
        self.column += 1;
        if self.column == self.size {
            self.column = 0;
            self.row += 1;
        }
    }
    pub fn acknowledge(&mut self, p: [i8; 3]) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(p) {
            return Err("native clear acknowledgement mismatch".into());
        }
        self.pending = None;
        self.next();
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
}
