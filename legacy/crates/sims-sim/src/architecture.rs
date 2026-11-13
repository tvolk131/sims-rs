//! Floor-aware architectural topology for authored fixtures and original lots.
use crate::{Point, Rect, Simulation, spatial};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Wall {
    pub a: Point,
    pub b: Point,
    /// Original door parts straddling this edge; room connectivity stays divided.
    pub door: Option<[ObjectId; 2]>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Footprint {
    pub kind: i16,
    pub size: i16,
    pub insets: u16,
    pub portal: bool,
    pub portal_style: i16,
    pub base_graphic: i16,
}
/// Authored surfaces and wall edges for one storey. A missing surface is a
/// routing/placement hole, not an invisible floor connecting the stair landings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FloorPlan {
    pub walls: Vec<Wall>,
    pub supported: Vec<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Floor {
    pub plan: FloorPlan,
    pub rooms: Vec<i16>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Architecture {
    pub width: i32,
    pub height: i32,
    pub floors: Vec<Floor>,
    pub outdoors: BTreeMap<i16, bool>,
    /// Static shape inputs from the loaded OBJD directory, including factories.
    pub footprints: BTreeMap<u32, Footprint>,
    /// Original arrays plus checked native-derived geometry. Authored fixtures
    /// have no imported lot and retain their existing topology policy.
    pub original: Option<crate::lot_geometry::Lot>,
}
impl Architecture {
    pub fn new(width: i32, height: i32, walls: Vec<Wall>, p: &Program) -> Result<Self, String> {
        Self::with_floors(
            width,
            height,
            vec![FloorPlan {
                walls,
                supported: vec![
                    true;
                    ((width.clamp(0, 1024) * height.clamp(0, 1024)) / 256).min(4096)
                        as usize
                ],
            }],
            p,
        )
    }
    pub fn with_floors(
        width: i32,
        height: i32,
        plans: Vec<FloorPlan>,
        p: &Program,
    ) -> Result<Self, String> {
        if width % 16 != 0
            || height % 16 != 0
            || !(32..=1024).contains(&width)
            || !(32..=1024).contains(&height)
            || !(1..=2).contains(&plans.len())
        {
            return Err("invalid architecture dimensions or storey count".into());
        }
        let tiles = (width * height / 256) as usize;
        let mut s = Self {
            width,
            height,
            floors: vec![],
            outdoors: BTreeMap::new(),
            footprints: BTreeMap::new(),
            original: None,
        };
        for plan in plans {
            if plan.supported.len() != tiles || plan.walls.len() > 8320 {
                return Err("invalid floor surfaces or wall count".into());
            }
            let mut seen = BTreeSet::new();
            for w in &plan.walls {
                if w.a.x % 16 != 0
                    || w.a.y % 16 != 0
                    || w.b.x % 16 != 0
                    || w.b.y % 16 != 0
                    || w.a.distance(w.b) != 16
                    || (w.a.x != w.b.x && w.a.y != w.b.y)
                    || w.a.x < 0
                    || w.a.y < 0
                    || w.b.x < 0
                    || w.b.y < 0
                    || w.a.x > width
                    || w.b.x > width
                    || w.a.y > height
                    || w.b.y > height
                    || !seen.insert((w.a.min(w.b), w.a.max(w.b)))
                {
                    return Err("invalid or duplicate wall edge".into());
                }
            }
            s.floors.push(Floor {
                plan,
                rooms: vec![-1; tiles],
            });
        }
        // Unique across storeys: the native portal cache compares room words.
        // Imported lots instead retain the original 1024-ID ranges below.
        let mut next = 1;
        for floor in 0..s.floors.len() {
            let level = floor as i32 + 1;
            let exterior = if floor == 0 {
                0
            } else {
                let n = next;
                next += 1;
                n
            };
            for y in 0..height / 16 {
                for x in 0..width / 16 {
                    let start = Point::new(x * 16 + 8, y * 16 + 8);
                    if s.room_at(level, start) != -1 {
                        continue;
                    }
                    let mut visited = BTreeSet::from([start]);
                    let mut queue = VecDeque::from([start]);
                    let mut outside = false;
                    while let Some(a) = queue.pop_front() {
                        for (dx, dy) in [(0, -16), (16, 0), (0, 16), (-16, 0)] {
                            let b = Point::new(a.x + dx, a.y + dy);
                            if s.wall_between(level, a, b).is_some() {
                                continue;
                            }
                            if b.x < 0 || b.x >= width || b.y < 0 || b.y >= height {
                                outside = true;
                            } else if visited.insert(b) {
                                queue.push_back(b);
                            }
                        }
                    }
                    let id = if outside {
                        exterior
                    } else {
                        let n = next;
                        next += 1;
                        n
                    };
                    for tile in visited {
                        let n = s.index(tile).unwrap();
                        s.floors[floor].rooms[n] = id;
                    }
                    s.outdoors.insert(
                        id,
                        spatial::room_outdoors(id, &[(id, if outside { 4 } else { 0 })]),
                    );
                }
            }
        }
        s.bind_footprints(p)?;
        Ok(s)
    }
    /// Bind recovered original architecture without running load/reset scripts.
    /// Spatial activation and lifecycle callbacks remain separate operations.
    pub fn from_original(lot: crate::lot_geometry::Lot, p: &Program) -> Result<Self, String> {
        lot.validate()?;
        let mut s = Self {
            width: lot.size as i32 * 16,
            height: lot.size as i32 * 16,
            floors: vec![],
            outdoors: BTreeMap::new(),
            footprints: BTreeMap::new(),
            original: None,
        };
        for (i, f) in lot.floors.iter().enumerate() {
            let size = lot.size;
            let walls = lot
                .wall_grid(i as i32 + 1)?
                .owned_segments()
                .map(|(a, b, _)| Wall { a, b, door: None })
                .collect();
            // Floor plans use row-major storage; original resources remain
            // column-major. Build-support bit 1 is not a walkable surface.
            let cells = (0..size).flat_map(|y| (0..size).map(move |x| x * size + y));
            let rooms = cells.clone().map(|c| f.rooms.primary[c]).collect();
            let supported = cells
                .map(|c| i == 0 || f.source.coverings[c] != 0)
                .collect();
            s.floors.push(Floor {
                plan: FloorPlan { walls, supported },
                rooms,
            });
            s.outdoors
                .extend(f.metrics.iter().map(|(&id, m)| (id, m.outdoors)));
        }
        s.original = Some(lot);
        s.bind_footprints(p)?;
        Ok(s)
    }
    /// Publish the room directory before ordered object loading. This must not
    /// update object positions or data word 29: each native post-load visit
    /// publishes that object's room immediately before its callbacks.
    pub fn publish_room_context(&self, world: &mut sims_vm::World) {
        world.room_outdoors.clone_from(&self.outdoors);
        world.room_half_tiles.clear();
        if let Some(lot) = &self.original {
            for f in &lot.floors {
                world
                    .room_half_tiles
                    .extend(f.metrics.iter().map(|(&id, m)| (id, m.half_tiles)));
            }
        } else {
            for &room in self.floors.iter().flat_map(|f| &f.rooms) {
                *world.room_half_tiles.entry(room).or_default() += 2;
            }
        }
        if let Some(refresh) = &mut world.refresh {
            for &id in self.outdoors.keys() {
                // 0x466b07 takes the category from room-ID bits 10..11.
                // Preserve existing dirty state when republishing the directory.
                refresh.rooms.entry(id as u16).or_default().category =
                    ((id as u16 >> 10) & 3) as u8;
            }
        }
    }
    /// Refresh static OBJD shape metadata after lazy factory-resource linking.
    /// Room geometry, live placement links and mutable tile flags are preserved.
    pub fn bind_footprints(&mut self, p: &Program) -> Result<(), String> {
        for m in p.modules.values() {
            for (&definition, w) in &m.object_definition_words {
                if w.len() <= 0x9a / 2 {
                    return Err("incomplete OBJD footprint".into());
                }
                let guid = w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16);
                let shape = Footprint {
                    kind: w[9],
                    size: w[0x86 / 2],
                    insets: w[0x9a / 2] as u16,
                    portal: m
                        .object_functions
                        .get(&definition)
                        .and_then(|f| f.get(15))
                        .is_some_and(|f| f.action != 0),
                    portal_style: w[0x8a / 2],
                    base_graphic: w[8],
                };
                if let Some(old) = self.footprints.insert(guid, shape.clone())
                    && old != shape
                {
                    return Err("conflicting GUID footprint".into());
                }
            }
        }
        Ok(())
    }
    fn index(&self, p: Point) -> Option<usize> {
        (p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height)
            .then(|| ((p.y / 16) * (self.width / 16) + p.x / 16) as usize)
    }
    pub fn floor(&self, level: i32) -> Option<&Floor> {
        level
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| self.floors.get(n))
    }
    pub fn room_at(&self, level: i32, p: Point) -> i16 {
        if let Some(lot) = &self.original {
            return lot.floor(level).map_or(-5, |f| {
                crate::lot_rooms::object_room_in_active(
                    lot.size,
                    lot.active_extent(),
                    &f.source.walls,
                    &f.rooms.primary,
                    p,
                )
                .expect("validated original room geometry")
            });
        }
        self.primary_room_at(level, p)
    }
    /// Direct coarse room word used by native route candidate classification.
    pub fn primary_room_at(&self, level: i32, p: Point) -> i16 {
        self.floor(level)
            .zip(self.index(p))
            .map_or(-5, |(f, i)| f.rooms[i])
    }
    pub fn supported(&self, level: i32, p: Point) -> bool {
        self.floor(level)
            .zip(self.index(p))
            .is_some_and(|(f, i)| f.plan.supported[i])
    }
    pub fn walls(&self, level: i32) -> &[Wall] {
        self.floor(level).map_or(&[], |f| &f.plan.walls)
    }
    pub fn wall_between(&self, level: i32, a: Point, b: Point) -> Option<&Wall> {
        self.walls(level).iter().find(|w| {
            if w.a.x != w.b.x && w.a.y != w.b.y {
                let side =
                    |p: Point| (w.b.x - w.a.x) * (p.y - w.a.y) - (w.b.y - w.a.y) * (p.x - w.a.x);
                return side(a).signum() != side(b).signum()
                    && a.x.min(b.x) <= w.a.x.max(w.b.x)
                    && a.x.max(b.x) >= w.a.x.min(w.b.x)
                    && a.y.min(b.y) <= w.a.y.max(w.b.y)
                    && a.y.max(b.y) >= w.a.y.min(w.b.y);
            }
            let min = w.a.min(w.b);
            let max = w.a.max(w.b);
            if min.x == max.x {
                ((a.x < min.x && b.x >= min.x) || (b.x < min.x && a.x >= min.x))
                    && a.y.min(b.y) < max.y
                    && a.y.max(b.y) >= min.y
            } else {
                ((a.y < min.y && b.y >= min.y) || (b.y < min.y && a.y >= min.y))
                    && a.x.min(b.x) < max.x
                    && a.x.max(b.x) >= min.x
            }
        })
    }
    pub fn wall_mask(&self, level: i32, p: Point, rotation: u8) -> u8 {
        if let Some(lot) = &self.original {
            let bits = lot
                .floor(level)
                .zip(lot.index(p))
                .map_or(0, |(f, i)| f.source.walls[i].segments);
            return spatial::wall_mask(bits, 0, rotation);
        }
        let center = Point::new(p.x.div_euclid(16) * 16 + 8, p.y.div_euclid(16) * 16 + 8);
        let mut bits = 0;
        for (bit, dx, dy) in [(2, 0, -16), (4, 16, 0), (8, 0, 16), (1, -16, 0)] {
            if self
                .wall_between(level, center, Point::new(center.x + dx, center.y + dy))
                .is_some()
            {
                bits |= bit;
            }
        }
        spatial::wall_mask(bits, 0, rotation)
    }
    pub fn bounds_clear(&self, level: i32, b: Rect, allowed: &[ObjectId]) -> bool {
        self.walls_clear(level, b, allowed)
            && (b.min.x.div_euclid(16)..=(b.max.x - 1).max(b.min.x).div_euclid(16)).all(|x| {
                (b.min.y.div_euclid(16)..=(b.max.y - 1).max(b.min.y).div_euclid(16))
                    .all(|y| self.supported(level, Point::new(x * 16 + 8, y * 16 + 8)))
            })
    }
    /// Existing multipart upper stubs can cover a stairwell opening. Floor
    /// support is a placement/movement query, not an invariant of every object.
    pub fn walls_clear(&self, level: i32, b: Rect, allowed: &[ObjectId]) -> bool {
        if let Some(lot) = &self.original {
            // Door openings are original nonblocking wall styles. Portal
            // interaction/object footprints are resolved by the host separately.
            return lot
                .wall_grid(level)
                .and_then(|grid| grid.blocks_bounds(b))
                .is_ok_and(|blocked| !blocked);
        }
        b.min.x >= 0
            && b.min.y >= 0
            && b.max.x <= self.width
            && b.max.y <= self.height
            && self.floor(level).is_some()
            && !self.walls(level).iter().any(|w| {
                if w.door
                    .is_some_and(|d| d.iter().any(|id| allowed.contains(id)))
                {
                    return false;
                }
                let a = w.a.min(w.b);
                let z = w.a.max(w.b);
                if a.x == z.x {
                    b.min.x < a.x && b.max.x > a.x && b.min.y < z.y && b.max.y > a.y
                } else {
                    b.min.y < a.y && b.max.y > a.y && b.min.x < z.x && b.max.x > a.x
                }
            })
    }
}
impl Simulation {
    pub fn set_original_lot(
        &mut self,
        p: &Program,
        lot: crate::lot_geometry::Lot,
    ) -> Result<(), String> {
        let architecture = Architecture::from_original(lot, p)?;
        if architecture.width != self.room.width || architecture.height != self.room.height {
            return Err("original architecture and room bounds differ".into());
        }
        self.architecture = Some(architecture);
        self.portal_cache.clear();
        self.sync_spatial_world();
        self.validate_architecture(p)
    }
    pub fn set_architecture(&mut self, p: &Program, walls: Vec<Wall>) -> Result<(), String> {
        self.architecture = Some(Architecture::new(
            self.room.width,
            self.room.height,
            walls,
            p,
        )?);
        self.portal_cache.clear();
        self.sync_spatial_world();
        self.validate_architecture(p)
    }
    pub fn set_floors(&mut self, p: &Program, plans: Vec<FloorPlan>) -> Result<(), String> {
        self.architecture = Some(Architecture::with_floors(
            self.room.width,
            self.room.height,
            plans,
            p,
        )?);
        self.portal_cache.clear();
        self.sync_spatial_world();
        self.validate_architecture(p)
    }
    pub(crate) fn sync_spatial_world(&mut self) {
        // 0x432220 updates the native free-will byte and script global 30 together.
        if self.household.is_some() {
            if let Some(p) = &self.vm.world.person {
                self.vm.world.globals.insert(30, i16::from(p.free_will));
            }
        }
        // Native placement owns coordinates, rooms and physical link order.
        // The authored fixture's bulk publication must never overwrite them.
        if self.native_lot.is_some() {
            if let Some(context) = self
                .vm
                .world
                .person
                .as_mut()
                .and_then(|p| p.autonomy.as_mut())
            {
                context.placements.clone_from(&self.vm.world.positions);
                context.selected_person = self.native_lot.as_ref().unwrap().selected_person;
            }
            return;
        }
        self.vm.world.positions = self
            .vm
            .world
            .objects
            .keys()
            .map(|&id| {
                (
                    id,
                    self.bodies.get(&id).map_or(
                        sims_vm::autonomy::Placement {
                            x: -16,
                            y: -16,
                            level: 1,
                        },
                        |b| sims_vm::autonomy::Placement {
                            x: b.position.y,
                            y: b.position.x,
                            level: b.level,
                        },
                    ),
                )
            })
            .collect();
        if let Some(context) = self
            .vm
            .world
            .person
            .as_mut()
            .and_then(|p| p.autonomy.as_mut())
        {
            context.placements.clone_from(&self.vm.world.positions);
        }
        if let Some(refresh) = &mut self.vm.world.refresh {
            for (&id, body) in &self.bodies {
                if let Some(draw) = refresh.draw.get_mut(&id) {
                    draw.level = body.level;
                }
            }
        }
        if let Some(a) = &self.architecture {
            a.publish_room_context(&mut self.vm.world);
            for (&id, b) in &self.bodies {
                let room = if self.unplaced.contains(&id) {
                    -5
                } else {
                    a.room_at(b.level, b.position)
                };
                self.vm
                    .world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .data
                    .insert(29, room);
            }
        }
    }
    pub(crate) fn collision_bounds(&self, id: ObjectId, p: Point) -> Rect {
        let body = &self.bodies[&id];
        let Some(a) = &self.architecture else {
            return body.bounds_at(p);
        };
        let object = &self.vm.world.objects[&id];
        let def = object
            .definition
            .as_ref()
            .expect("validated spatial definition");
        let f = &a.footprints[&def.guid];
        let fields = object.person.as_ref();
        let get = |i| fields.and_then(|p| p.get(&i)).copied().unwrap_or(0) as u16;
        let [mx, my, nx, ny] = spatial::footprint(
            [p.y, p.x],
            f.kind == 2,
            (get(68) | get(84)) & 1 != 0,
            f.size,
            f.insets,
            get(73),
            body.facing.trailing_zeros() as u8,
        );
        Rect {
            min: Point::new(ny, nx),
            max: Point::new(my, mx),
        }
    }
    pub(crate) fn validate_architecture(&self, p: &Program) -> Result<(), String> {
        let Some(a) = &self.architecture else {
            return Ok(());
        };
        let rebuilt = if let Some(lot) = &a.original {
            Architecture::from_original(lot.clone(), p)?
        } else {
            Architecture::with_floors(
                self.room.width,
                self.room.height,
                a.floors.iter().map(|f| f.plan.clone()).collect(),
                p,
            )?
        };
        if *a != rebuilt || a.width != self.room.width || a.height != self.room.height {
            return Err("architecture topology or footprint binding mismatch".into());
        }
        for (&id, o) in &self.vm.world.objects {
            if self.bodies.contains_key(&id)
                && o.definition
                    .as_ref()
                    .is_none_or(|d| !a.footprints.contains_key(&d.guid))
            {
                return Err("missing native footprint definition".into());
            }
        }
        for (floor, w) in a
            .floors
            .iter()
            .enumerate()
            .flat_map(|(i, f)| f.plan.walls.iter().map(move |w| (i, w)))
        {
            if let Some(parts) = w.door {
                let b = self.body(parts[0])?;
                let c = self.body(parts[1])?;
                if b.level != floor as i32 + 1
                    || c.level != b.level
                    || b.position.distance(c.position) != 16
                    || a.wall_between(b.level, b.position, c.position) != Some(w)
                {
                    return Err("door does not straddle its wall edge".into());
                }
            }
        }
        Ok(())
    }
}

impl Simulation {
    pub(crate) fn active_portals(&self, actor: ObjectId) -> Vec<ObjectId> {
        self.vm
            .threads
            .values()
            .filter_map(|t| t.service.as_ref())
            .flat_map(|s| s.chain())
            .filter(|s| s.caller == actor)
            .map(|s| s.callee)
            .collect()
    }
    pub(crate) fn portal_location(&self, id: ObjectId) -> Option<crate::portals::Location> {
        let b = self.bodies.get(&id)?;
        let o = self.vm.world.objects.get(&id)?;
        Some(crate::portals::Location {
            position: b.position,
            level: b.level,
            room: *o.data.get(&29)?,
            zero_distance: o.definition.as_ref().is_some_and(|d| {
                self.architecture
                    .as_ref()
                    .is_some_and(|a| a.footprints[&d.guid].kind == 7)
            }),
        })
    }
    pub(crate) fn portal_nodes(&self) -> Vec<crate::portals::Portal> {
        let Some(a) = &self.architecture else {
            return vec![];
        };
        let parts: Vec<_> = self
            .vm
            .world
            .objects
            .iter()
            .filter_map(|(&id, o)| {
                let f = a.footprints.get(&o.definition.as_ref()?.guid)?;
                let part = o.multitile.as_ref()?;
                Some(crate::portals::PairPart {
                    id,
                    head: part.head,
                    next: part.next,
                    style: f.portal_style,
                    base_graphic: f.base_graphic,
                    portal: f.portal,
                })
            })
            .collect();
        let pairs: BTreeMap<_, _> = parts
            .iter()
            .filter(|p| p.portal)
            .filter_map(|p| Some((p.id, crate::portals::partner(&parts, p.id)?)))
            .collect();
        let order: Vec<_> = self.vm.world.person.as_ref().map_or_else(
            || pairs.keys().copied().collect(),
            |p| p.object_order.clone(),
        );
        order
            .into_iter()
            .filter_map(|id| {
                Some(crate::portals::Portal {
                    id,
                    pair: *pairs.get(&id)?,
                    location: self.portal_location(id)?,
                })
            })
            .collect()
    }
    pub(crate) fn next_portal(&mut self, request: &sims_vm::RouteRequest) -> Option<ObjectId> {
        let portals = self.portal_nodes();
        let actor = self.portal_location(request.caller)?;
        let target = self.portal_location(request.target)?;
        let cache = self.portal_cache.entry(request.caller).or_default();
        if self.route_control[&request.id].last_portal.is_none() {
            cache.reset();
        }
        cache.choose(&portals, actor, target)
    }
}
