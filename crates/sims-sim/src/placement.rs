//! Find Location's native spiral; tile metadata/placement acceptance are supplied
//! by the authored one-floor room, not a full native architecture/footprint map.
use crate::{Body, Event, Point, Simulation};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, ThreadId, placement::Request};

/// Native camera inputs for unanchored 4347f0 searches. The projection result
/// must come from the host camera; neither the Sim's tile nor a demo default is
/// a replacement. Keep this view state fixed across a suspended search.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchView {
    /// Camera rectangle +28..34, native left/top/right/bottom words.
    pub viewport: [i32; 4],
    /// World-view +70..7c: column minimum/maximum, row minimum/maximum.
    pub bounds: [i32; 4],
    pub rotation: i32,
}
impl SearchView {
    /// 4a677c/4a7b58 packs height-half in the low word and width-half in the
    /// high word. It does not add the rectangle's left/top offsets.
    pub fn midpoint(&self) -> [i16; 2] {
        let [left, top, right, bottom] = self.viewport;
        [
            (bottom.wrapping_sub(top) / 2) as i16,
            (right.wrapping_sub(left) / 2) as i16,
        ]
    }

    /// 434861..434921 after the camera's screen-to-world projection (mode 1).
    /// The fixed 1024-unit rotation extent is native, even on smaller lots.
    pub fn origin(&self, size: i32, projected: [i32; 2]) -> [i32; 2] {
        let tile = [
            (projected[1] >> 4) as i8 as i32,
            (projected[0] >> 4) as i8 as i32,
        ];
        let [min_column, max_column, min_row, max_row] = self.bounds;
        let inside = tile[0] >= min_column.wrapping_add(1)
            && tile[0] <= max_column.wrapping_sub(1)
            && tile[1] >= min_row.wrapping_add(1)
            && tile[1] <= max_row.wrapping_sub(1);
        let [x, y] = if inside {
            projected
        } else {
            let half = (size / 2) as i8;
            let column = half.wrapping_add(min_column as i8);
            let row = half.wrapping_add(min_row as i8);
            [(i32::from(row) << 4) | 8, (i32::from(column) << 4) | 8]
        };
        match self.rotation.wrapping_neg() & 3 {
            0 => [x, y],
            1 => [y, 1023_i32.wrapping_sub(x)],
            2 => [1023_i32.wrapping_sub(x), 1023_i32.wrapping_sub(y)],
            _ => [1023_i32.wrapping_sub(y), x],
        }
    }
}

/// Mode5's cumulative random origin (43494a..434b3c). Each advance consumes
/// exactly two lot-stream draws. An unsuccessful bounds check continues from
/// the accumulated point; only a new search pass resets to the descriptor.
/// Keeping this cursor lets an instruction budget suspend a long random walk
/// without yielding the synchronous primitive to another simulation thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RandomOrigin {
    original: [i32; 2],
    point: [i32; 2],
}
impl RandomOrigin {
    pub fn new(original: [i32; 2]) -> Self {
        Self {
            original,
            point: original,
        }
    }
    pub fn reset(&mut self) {
        self.point = self.original;
    }
    pub fn advance(
        &mut self,
        size: i32,
        bounds: [i32; 4],
        rotation: i32,
        rng: &mut u32,
    ) -> Option<[i32; 2]> {
        let a = sims_vm::rng::next(rng);
        let b = sims_vm::rng::next(rng);
        if self.original == [1, 1] {
            // The original uses size directly here, not size * 16.
            self.point = [(b % size as u32) as i32, (a % size as u32) as i32];
        } else {
            let column = (a % 10) as i32 - 5;
            let row = (b % 10) as i32 - 5;
            self.point[0] = self.point[0].wrapping_add((row << 4) | 8);
            self.point[1] = self.point[1].wrapping_add((column << 4) | 8);
            let [column, row] = [self.point[1], self.point[0]].map(|n| (n >> 4) as i8 as i32);
            let [left, right, top, bottom] = bounds;
            if column < left.wrapping_add(1)
                || column > right.wrapping_sub(1)
                || row < top.wrapping_add(1)
                || row > bottom.wrapping_sub(1)
            {
                return None;
            }
        }
        let [x, y] = self.point;
        Some(match rotation.wrapping_neg() & 3 {
            0 => [x, y],
            1 => [y, 1023_i32.wrapping_sub(x)],
            2 => [1023_i32.wrapping_sub(x), 1023_i32.wrapping_sub(y)],
            _ => [1023_i32.wrapping_sub(y), x],
        })
    }
}

/// Resumable cursor for native 0x4347f0. Coordinates here are native physical
/// X/Y; grid indices returned by `center_tile` are column/row (Y/X).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spiral {
    size: i32,
    base: [i8; 2],
    offset: [i32; 2],
    outside: i32,
    iterations: u32,
    direction: i32,
}
impl Spiral {
    pub fn new(size: i32, position: [i32; 2]) -> Result<Self, String> {
        Self::new_direction(size, position, -1)
    }
    /// -1 is the spiral; 0/4 alternate native X, 2/6 alternate native Y.
    /// Other signed directions retain the center until the 4096-step cap.
    pub fn new_direction(size: i32, position: [i32; 2], direction: i32) -> Result<Self, String> {
        if !(3..=64).contains(&size) {
            return Err("unsupported Find Location lot dimension".into());
        }
        Ok(Self {
            size,
            base: [position[1], position[0]]
                .map(|v| ((v >> 4) as i8 as i32).clamp(1, size - 2) as i8),
            offset: [0, 0],
            outside: 0,
            iterations: 0,
            direction,
        })
    }
    pub fn direction(&self) -> i32 {
        self.direction
    }
    pub fn center_tile(&self) -> [i8; 2] {
        self.base
    }
    pub fn last_point(&self) -> Option<[i32; 2]> {
        let mut cursor = Self {
            size: self.size,
            base: self.base,
            offset: [0, 0],
            outside: 0,
            iterations: 0,
            direction: self.direction,
        };
        let mut point = None;
        for _ in 0..self.iterations.min(4096) {
            point = cursor.step();
        }
        point
    }
    pub fn validate(&self, size: i32, position: [i32; 2]) -> Result<(), String> {
        let mut expected = Self::new_direction(size, position, self.direction)?;
        if self.iterations > 4096 {
            return Err("invalid Find Location cursor".into());
        }
        while expected.iterations < self.iterations {
            if expected.outside >= size * 8 {
                return Err("Find Location cursor beyond termination".into());
            }
            expected.step();
        }
        if &expected != self {
            return Err("invalid Find Location cursor".into());
        }
        Ok(())
    }
    fn step(&mut self) -> Option<[i32; 2]> {
        let [x, y] = [
            self.base[0].wrapping_add(self.offset[0] as i8),
            self.base[1].wrapping_add(self.offset[1] as i8),
        ];
        let point =
            if x < 1 || y < 1 || i32::from(x) >= self.size - 1 || i32::from(y) >= self.size - 1 {
                self.outside += 1;
                None
            } else {
                self.outside = 0;
                Some([i32::from(y) * 16 + 8, i32::from(x) * 16 + 8])
            };
        let [x, y] = &mut self.offset;
        if self.direction != -1 {
            let value = match self.direction {
                0 | 4 => Some(y),
                2 | 6 => Some(x),
                _ => None,
            };
            if let Some(value) = value {
                *value = if *value < 1 { 1 - *value } else { -*value };
            }
        } else if -*x < *y {
            if *x < *y {
                *x += 1;
            } else {
                *y -= 1;
            }
        } else if *x < *y {
            if *y < -*x {
                *y += 1;
            } else {
                *x += 1;
            }
        } else {
            *x -= 1;
        }
        self.iterations += 1;
        point
    }
}
impl Iterator for Spiral {
    type Item = [i32; 2];
    fn next(&mut self) -> Option<Self::Item> {
        while self.iterations < 4096 && self.outside < self.size * 8 {
            if let Some(p) = self.step() {
                return Some(p);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SearchTile {
    pub segments: u8,
    pub head: i16,
    pub room: i16,
    pub pool: u8,
    /// World +0x34 is the ground array, even when searching upstairs.
    pub ground_flags: u8,
}
pub fn search_tile_allowed(
    tile: SearchTile,
    level: i32,
    flags: u8,
    room: Option<i16>,
    ignore_marker: bool,
) -> bool {
    tile.segments & 0x30 == 0
        && (flags & 2 != 0 || tile.head == 0)
        && room.is_none_or(|r| r == tile.room)
        && (level != 1 || tile.pool >= 16)
        && (flags & 4 == 0 || ignore_marker || tile.ground_flags & 0x20 == 0)
}

/// 0x4347f0 mode-zero spiral in native X/Y units. The search excludes the outer
/// border, clamps its center to interior tiles, and tries at most 4096 positions.
pub fn spiral(
    size: i32,
    center: Point,
    mut accept: impl FnMut(Point) -> bool,
) -> Result<Option<Point>, String> {
    for [x, y] in Spiral::new(size, [center.x, center.y])? {
        let point = Point::new(x, y);
        if accept(point) {
            return Ok(Some(point));
        }
    }
    Ok(None)
}
impl Simulation {
    pub(crate) fn service_find_location(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if self.native_lot.is_some() {
            return Err(
                "native Find Location inside a synchronous callback requires a nested host owner"
                    .into(),
            );
        }
        if r.operands[0] == 1 {
            // 0x44b3c8 calls virtual RemoveFromLot(1). Single-part groups
            // have no sibling-link split effects; broader groups remain gated.
            self.empty_detach_callbacks(p, r.target)?;
            self.detach_tree(r.target)?;
            self.vm.finish_find_location(p, thread, r.id, true)?;
            self.event(
                events,
                thread,
                format!("Find Location detached #{} from lot", r.target),
            );
            return Ok(());
        }
        if r.operands[0] != 0 {
            return Err("directional Find Location requires the native lot host".into());
        }
        let reference = self.body(r.reference)?.clone();
        let object = &self.vm.world.objects[&r.target];
        let def = object
            .definition
            .as_ref()
            .ok_or("Find Location needs target definition")?;
        let module = &p.modules[&def.source];
        if object.person.is_some()
            || module.object_definition_words[&def.object][0x14 / 2] != 0
            || object.slots.iter().any(Option::is_some)
            || module.object_functions[&def.object]
                .get(9)
                .is_some_and(|f| f.action != 0)
        {
            return Err(
                "Find Location target subclass/contained children/placement callback unsupported"
                    .into(),
            );
        }
        let mut prototype = self.bodies.get(&r.target).cloned().unwrap_or(Body {
            level: self.body(r.reference)?.level,
            position: Point::new(8, 8),
            radius: 1,
            elevation: 0,
            facing: 1 << object.data[&1],
        });
        prototype.level = reference.level;
        let mut spatial = self.clone();
        spatial.bodies.insert(r.target, prototype.clone());
        let same_room = self.vm.world.objects[&r.reference].data[&29];
        let check = |point: Point, room_only: bool| {
            let pos = Point::new(point.y, point.x);
            if let Some(arch) = &self.architecture {
                if (room_only && arch.room_at(reference.level, pos) != same_room)
                    || !spatial.clear(r.target, pos, None)
                {
                    return false;
                }
                // Native bit 1 permits occupied tiles; bit 2 excludes floor
                // flag 0x20. The authored house has no tiles with that flag.
                r.operands[2] & 2 != 0
                    || !self.bodies.iter().any(|(&id, b)| {
                        id != r.target
                            && b.level == reference.level
                            && self.vm.world.objects[&id].container.is_none()
                            && b.position.x / 16 == pos.x / 16
                            && b.position.y / 16 == pos.y / 16
                    })
            } else {
                let bounds = prototype.bounds_at(pos);
                self.room.contains(bounds)
                    && !self.room.obstacles.iter().any(|r| r.overlaps(bounds))
                    && !self.bodies.iter().any(|(&id, b)| {
                        id != r.target
                            && b.level == reference.level
                            && self.vm.world.objects[&id].container.is_none()
                            && b.bounds_at(b.position).overlaps(bounds)
                    })
            }
        };
        let center = Point::new(reference.position.y, reference.position.x);
        let size = (self.room.width.max(self.room.height) + 15) / 16;
        let mut destination = spiral(size, center, |p| check(p, true))?;
        if destination.is_none() {
            destination = spiral(size, center, |p| check(p, false))?;
        }
        if let Some(point) = destination.filter(|p| check(*p, false)) {
            if let Some((parent, slot)) = object.container {
                self.vm.world.objects.get_mut(&parent).unwrap().slots[slot as usize] = None;
            }
            let room = self.architecture.as_ref().map_or(same_room, |a| {
                a.room_at(reference.level, Point::new(point.y, point.x))
            });
            let target = self.vm.world.objects.get_mut(&r.target).unwrap();
            target.container = None;
            target
                .data
                .extend([(2, 0), (3, -1), (14, -1), (26, 0), (29, room)]);
            self.bodies.insert(
                r.target,
                Body {
                    level: self.body(r.reference)?.level,
                    position: Point::new(point.y, point.x),
                    elevation: 0,
                    ..prototype
                },
            );
            self.unplaced.remove(&r.target);
            self.advertise_placed_object(r.target);
            self.vm.refresh_object(r.caller, r.target, 1, 0)?;
            self.vm.finish_find_location(p, thread, r.id, true)?;
            self.event(
                events,
                thread,
                format!(
                    "Find Location placed #{} at ({}, {})",
                    r.target, point.y, point.x
                ),
            );
        } else {
            self.vm.finish_find_location(p, thread, r.id, false)?;
        }
        Ok(())
    }
}
