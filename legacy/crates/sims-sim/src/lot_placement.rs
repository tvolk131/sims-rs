//! Base and multipart SetObject/RemoveFromLot ordering. Requests are synchronous boundaries:
//! the host must finish each request before advancing this operation. This is
//! the mutating path, including the person's shared SetObject dispatch. Native
//! placement acceptance and other person virtual services are separate.
use crate::{
    architecture::Architecture,
    lot_links::{Descriptor, Links},
    lot_loading,
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, World, autonomy::Placement};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
mod group_rotation;
pub mod live;
#[cfg(test)]
mod planning_tests;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub links: Links,
    /// Native +0x100..+0x10c, updated by spatial publication rather than
    /// inferred from the current object fields at collision-query time.
    pub bounds: BTreeMap<ObjectId, [i32; 4]>,
    /// Mutable native world flags. Exterior/support rebuild inputs are separate
    /// from these live bytes; 0x4330d7 only changes the selected marker bit.
    pub flags: Vec<Vec<u8>>,
    ///Historical placement-owned +44 alias. Fresh registry-owned profiles leave
    ///this absent; old snapshots retain their exact bool without a guessed merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_dirty: Option<bool>,
}
impl State {
    ///Fresh physical storage borrows registry invalidation ownership. This is
    ///an initialization boundary, never a migration of conflicting saved aliases.
    pub fn new_for_world(links: Links, a: &Architecture, world: &World) -> Result<Self, String> {
        let mut state = Self::new(links, a)?;
        if world.registry_order.is_some() {
            state.spatial_dirty = None;
        }
        Ok(state)
    }
    ///431aaf calls433e50(-16,-16,1) before dynamic/graphic construction.
    ///The VM publishes those coordinates and slots; retain433d16's bounds in
    ///the physical owner before any graphic, Reset/Init or caller can inspect
    ///the new object. A restored historical object is never repaired here.
    pub fn postconstruct_bounds(
        &mut self,
        world: &World,
        program: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        if !self.links.next.contains_key(&id) {
            return Err("postconstructor physical registration absent".into());
        }
        let object = world
            .objects
            .get(&id)
            .ok_or("postconstructor object absent")?;
        let position = world
            .positions
            .get(&id)
            .ok_or("postconstructor position absent")?;
        if (position.x, position.y, position.level) != (-16, -16, 1) {
            return Err("postconstructor no longer owns the off-lot sentinel".into());
        }
        let definition = object
            .definition
            .as_ref()
            .ok_or("postconstructor definition absent")?;
        let words = sims_vm::construction::definition_words(program, definition)?;
        let kind = words[9];
        let size = *words.get(0x86 / 2).ok_or("postconstructor size absent")?;
        let insets = *words.get(0x9a / 2).ok_or("postconstructor insets absent")? as u16;
        let facing = *object.data.get(&1).ok_or("postconstructor facing absent")?;
        if !(0..=7).contains(&facing) {
            return Err("postconstructor facing outside native footprint contract".into());
        }
        let person = |index| -> Result<u16, String> {
            if kind != 2 {
                return Ok(0);
            }
            object
                .person
                .as_ref()
                .and_then(|p| p.get(&index))
                .map(|&word| word as u16)
                .ok_or_else(|| "postconstructor person footprint field absent".into())
        };
        let bounds = crate::spatial::footprint(
            [position.x, position.y],
            kind == 2,
            (person(68)? | person(84)?) & 1 != 0,
            size,
            insets,
            person(73)?,
            facing as u8,
        );
        self.bounds.insert(id, bounds);
        Ok(())
    }

    pub fn new(links: Links, architecture: &Architecture) -> Result<Self, String> {
        let lot = architecture
            .original
            .as_ref()
            .ok_or("placement requires original lot")?;
        links.validate_arrays()?;
        if lot.size != links.size || lot.floors.len() != links.heads.len() {
            return Err("placement map dimensions disagree".into());
        }
        Ok(Self {
            bounds: links.next.keys().map(|&id| (id, [0; 4])).collect(),
            links,
            flags: lot.floors.iter().map(|f| f.flags.flags.clone()).collect(),
            spatial_dirty: Some(false),
        })
    }
    fn marker(&mut self, tile: [i8; 3], set: bool) -> Result<(), String> {
        let [x, y, level] = tile;
        if x <= 0
            || y <= 0
            || x as usize >= self.links.size - 1
            || y as usize >= self.links.size - 1
        {
            return Ok(());
        }
        let floor = usize::try_from(i32::from(level) - 1)
            .ok()
            .and_then(|i| self.flags.get_mut(i))
            .ok_or("placement marker floor unavailable")?;
        let cell = floor
            .get_mut(x as usize * self.links.size + y as usize)
            .ok_or("placement marker cell unavailable")?;
        *cell = (*cell & !8) | if set { 8 } else { 0 };
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Function {
        object: ObjectId,
        function: usize,
    },
    /// 0x434101(mode): true invalidates only this object; false ascends to
    /// its physical container and invalidates that subtree. The serialized
    /// `children_only` name is historical and does not mean child recursion.
    Invalidate {
        object: ObjectId,
        children_only: bool,
    },
    /// 0x434ed6, conditional screen/shadow invalidation in the presentation host.
    Shadow(ObjectId),
    /// Old screen rectangle, conditional on the native visible-world flag.
    Screen(ObjectId),
    /// 0x4b50b5 around an object with light-source data word 16 == 1.
    LightRange([i8; 3]),
    Signal {
        event: u16,
        value: i16,
    },
    Adjacency([i8; 3]),
    /// 0x434fac, including its function-6 callback when wall flags change.
    Walls(ObjectId),
    /// Virtual animator +0x54(1), if virtual GetAnimator returns a value.
    RefreshAnimator(ObjectId),
    ViewRemoved(ObjectId),
    ViewPlaced(ObjectId),
    /// Virtual object +0x74: view/animator +0x24(0), after Find Location commits.
    RefreshAppearance(ObjectId),
    /// Native placement diagnostic set by group acceptance's outside gate.
    PlacementError(i32),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Kind {
    Remove,
    Set(Descriptor),
    /// Native Create Object's at-most-four virtual Rotate(1) calls. The desired
    /// cardinal direction is captured from the live Stack Object after Init.
    CopyDirection(i16),
    /// One actual433f75 call; the argument is a notch count, not a facing.
    NativeRotate(i32),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Pending {
    Request(Request),
    Position(lot_loading::SpatialUpdate),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Operation {
    object: ObjectId,
    kind: Kind,
    notify: bool,
    phase: u8,
    old: Placement,
    old_tile: [i8; 3],
    new_tile: [i8; 3],
    old_facing: i16,
    old_outside: bool,
    parent: Option<ObjectId>,
    level: i32,
    marker: bool,
    pending: VecDeque<Pending>,
}
fn position(w: &World, id: ObjectId) -> Result<Placement, String> {
    w.positions
        .get(&id)
        .copied()
        .ok_or("placement position absent".into())
}
fn data(w: &World, id: ObjectId, index: i16) -> Result<i16, String> {
    w.objects
        .get(&id)
        .and_then(|o| o.data.get(&index))
        .copied()
        .ok_or("placement object data absent".into())
}
fn word(w: &World, p: &Program, id: ObjectId, index: usize) -> Result<i16, String> {
    let d = w
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("placement definition absent")?;
    p.modules
        .get(&d.source)
        .and_then(|m| m.object_definition_words.get(&d.object))
        .and_then(|v| v.get(index))
        .copied()
        .ok_or("placement definition word absent".into())
}
fn function(w: &World, p: &Program, id: ObjectId, index: usize) -> Result<bool, String> {
    let d = w
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("placement definition absent")?;
    let table = p
        .modules
        .get(&d.source)
        .and_then(|m| m.object_functions.get(&d.object))
        .ok_or("placement functions absent")?;
    Ok(table.get(index).is_some_and(|f| f.action != 0))
}
fn forward(tile: [i8; 3], facing: i16) -> Result<[i8; 3], String> {
    let (dx, dy) = crate::routing::DIRECTION_OFFSETS
        .get(usize::try_from(facing).map_err(|_| "negative placement facing")?)
        .ok_or("placement facing outside native direction table")?;
    Ok([
        tile[0].wrapping_add((dy / 16) as i8),
        tile[1].wrapping_add((dx / 16) as i8),
        tile[2],
    ])
}
/// Data word 5 from 0x434fac for ordinary floor levels. Its caller invokes
/// function 6 only if the returned value differs from the existing word.
pub fn wall_flags(
    world: &World,
    program: &Program,
    architecture: &Architecture,
    id: ObjectId,
) -> Result<i16, String> {
    let old = data(world, id, 5)?;
    if !function(world, program, id, 6)? {
        return Ok(old);
    }
    let lot = architecture
        .original
        .as_ref()
        .ok_or("wall placement requires original architecture")?;
    let q = position(world, id)?;
    if !(1..=2).contains(&q.level) {
        return Err("wall placement floor outside supported native range".into());
    }
    let [x, y, _] = sims_vm::loading::coarse_position(q);
    let mut flags = old as u16 & 0xffc0;
    if x > 0 && y > 0 && (x as usize) < lot.size - 1 && (y as usize) < lot.size - 1 {
        let expanded = lot.wall_grid(q.level)?.expanded(x as usize, y as usize)?;
        let rotation = (i32::from(data(world, id, 1)?) / -2) & 3;
        for source in 0..4 {
            if expanded.segments & (1 << source) == 0 {
                continue;
            }
            let style = expanded.styles[source].ok_or("wall placement edge has no owner style")?;
            if !matches!(style, 2 | 12 | 13 | 14) {
                flags |= [1, 4, 2, 8][(source + rotation as usize) % 4];
            }
        }
    }
    Ok(flags as i16)
}
impl Operation {
    pub fn binding(&self) -> (ObjectId, Option<Descriptor>) {
        (
            self.object,
            match self.kind {
                Kind::Remove | Kind::CopyDirection(_) | Kind::NativeRotate(_) => None,
                Kind::Set(d) => Some(d),
            },
        )
    }
    /// Import adapter: service original synchronous script and spatial
    /// callbacks, stopping at every observer/view boundary. The caller must
    /// handle the returned request before advancing again. Ordinary Main and
    /// the lot clock remain suspended.
    pub fn advance_loaded(
        &mut self,
        vm: &mut sims_vm::Vm,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        while let Some(request) = self.next(&mut vm.world, p, a, state)? {
            if let Some(other) = service_loaded(request, vm, p, a, state)? {
                return Ok(Some(other));
            }
        }
        state.links.publish_tile_chains(&mut vm.world)?;
        Ok(None)
    }
    fn new(object: ObjectId, kind: Kind, notify: bool) -> Self {
        Self {
            object,
            kind,
            notify,
            phase: 0,
            old: Placement {
                x: 0,
                y: 0,
                level: 0,
            },
            old_tile: [0; 3],
            new_tile: [0; 3],
            old_facing: 0,
            old_outside: false,
            parent: None,
            level: 0,
            marker: false,
            pending: VecDeque::new(),
        }
    }
    pub fn remove(object: ObjectId, notify: bool) -> Self {
        Self::new(object, Kind::Remove, notify)
    }
    pub fn set(object: ObjectId, destination: Descriptor, notify: bool) -> Self {
        Self::new(object, Kind::Set(destination), notify)
    }
    pub fn copy_direction(object: ObjectId, desired: i16) -> Result<Self, String> {
        if ![0, 2, 4, 6].contains(&desired) {
            return Err("creation direction must be cardinal".into());
        }
        Ok(Self::new(object, Kind::CopyDirection(desired), false))
    }
    pub fn rotate(object: ObjectId, notches: i32) -> Self {
        Self::new(object, Kind::NativeRotate(notches), false)
    }
    fn call(&self, function: usize) -> Request {
        Request::Function {
            object: self.object,
            function,
        }
    }
    fn move_position(&mut self, w: &World, a: &Architecture, to: Placement) -> Result<(), String> {
        // Validate the complete spatial subtree first, but publish each
        // position only after its drawing invalidation. Native observers must
        // see the state at their actual point in 0x433e50, not the final subtree.
        let update = lot_loading::plan_position(w, a, self.object, to)?;
        let mut positions = update.objects.into_iter();
        for effect in update.effects {
            match effect {
                lot_loading::Effect::Invalidate(object) => {
                    self.pending
                        .push_back(Pending::Request(Request::Invalidate {
                            object,
                            children_only: object != self.object,
                        }));
                    let position = positions.next().ok_or("spatial update order incomplete")?;
                    if position.object != object {
                        return Err("spatial update order mismatch".into());
                    }
                    self.pending.push_back(Pending::Position(position));
                }
                lot_loading::Effect::Room(value) => self
                    .pending
                    .push_back(Pending::Request(Request::Signal { event: 0xf3, value })),
            }
        }
        Ok(())
    }
    /// Advance until the next synchronous boundary. None means this base
    /// operation completed. A request is not permission to skip its consumer.
    pub fn next(
        &mut self,
        w: &mut World,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        loop {
            if let Some(pending) = self.pending.pop_front() {
                match pending {
                    Pending::Request(r) => return Ok(Some(r)),
                    Pending::Position(q) => {
                        state.bounds.insert(q.object, q.bounds);
                        w.positions.insert(
                            q.object,
                            Placement {
                                x: q.position[0],
                                y: q.position[1],
                                level: q.position[2],
                            },
                        );
                        w.objects
                            .get_mut(&q.object)
                            .ok_or("spatial child removed during observer callback")?
                            .data
                            .insert(29, q.room);
                        continue;
                    }
                }
            }
            let id = self.object;
            let phase = self.phase;
            self.phase = self.phase.saturating_add(1);
            let mut request = None;
            match self.kind {
                Kind::NativeRotate(notches) => match phase {
                    0 => {
                        if notches == 0 {
                            self.phase = 5;
                            return Ok(None);
                        }
                        let amount = data(w, id, 23)?;
                        if amount == 0 {
                            self.phase = 5;
                            return Ok(None);
                        }
                        self.old_facing = data(w, id, 1)?;
                        let direction = (self.old_facing as i32)
                            .wrapping_add(i32::from(amount).wrapping_mul(notches))
                            & 7;
                        w.objects
                            .get_mut(&id)
                            .unwrap()
                            .data
                            .insert(1, direction as i16);
                        request = Some(Request::Invalidate {
                            object: id,
                            children_only: false,
                        });
                    }
                    1 => {
                        self.marker = function(w, p, id, 26)?;
                        if self.marker {
                            let tile = sims_vm::loading::coarse_position(position(w, id)?);
                            request = Some(Request::Adjacency(forward(tile, self.old_facing)?));
                        }
                    }
                    2 => {
                        if self.marker {
                            let tile = sims_vm::loading::coarse_position(position(w, id)?);
                            request = Some(Request::Adjacency(forward(tile, data(w, id, 1)?)?));
                        }
                    }
                    3 => request = Some(Request::Walls(id)),
                    _ => return Ok(None),
                },
                Kind::CopyDirection(desired) => {
                    if phase >= 20 {
                        return Ok(None);
                    }
                    match phase % 5 {
                        0 => {
                            let direction = data(w, id, 1)?;
                            if direction == desired {
                                self.phase = 21;
                                return Ok(None);
                            }
                            let amount = data(w, id, 23)?;
                            if amount == 0 {
                                // A disabled base Rotate is still an attempted
                                // virtual call; the wrapper stops after four.
                                self.phase += 4;
                                continue;
                            }
                            self.old_facing = direction;
                            w.objects
                                .get_mut(&id)
                                .unwrap()
                                .data
                                .insert(1, direction.wrapping_add(amount) & 7);
                            request = Some(Request::Invalidate {
                                object: id,
                                children_only: false,
                            });
                        }
                        1 => {
                            self.marker = function(w, p, id, 26)?;
                            if self.marker {
                                let tile = sims_vm::loading::coarse_position(position(w, id)?);
                                request = Some(Request::Adjacency(forward(tile, self.old_facing)?));
                            }
                        }
                        2 => {
                            // The first adjacency callback may move or rotate
                            // this object. Native reads its position/facing again.
                            if self.marker {
                                let tile = sims_vm::loading::coarse_position(position(w, id)?);
                                request = Some(Request::Adjacency(forward(tile, data(w, id, 1)?)?));
                            }
                        }
                        3 => request = Some(Request::Walls(id)),
                        _ => {}
                    }
                }
                Kind::Remove => match phase {
                    0 => {
                        if word(w, p, id, 10)? == 0 {
                            request = Some(self.call(10));
                        }
                    }
                    1 => {
                        self.old = position(w, id)?;
                        self.old_tile = sims_vm::loading::coarse_position(self.old);
                        self.old_facing = data(w, id, 1)?;
                        request = Some(Request::Invalidate {
                            object: id,
                            children_only: false,
                        });
                    }
                    2 => request = Some(Request::Shadow(id)),
                    3 => request = Some(Request::Screen(id)),
                    4 => {
                        if data(w, id, 16)? == 1 {
                            request = Some(Request::LightRange(self.old_tile));
                        }
                    }
                    5 => {
                        if data(w, id, 40)? as u16 & 0x400 != 0 {
                            request = Some(Request::Signal {
                                event: 0xf2,
                                value: id,
                            });
                        }
                    }
                    6 => {
                        self.parent = state.links.capture(w, id)?.container;
                        state.links.apply(
                            w,
                            id,
                            Descriptor {
                                on_lot: false,
                                position: [0, 0],
                                container: None,
                                slot: -1,
                                level: 1,
                            },
                        )?;
                        let o = w
                            .objects
                            .get_mut(&id)
                            .ok_or("removed placement object missing")?;
                        o.data.extend([(2, 0), (3, -1)]);
                        o.container = None;
                        self.move_position(
                            w,
                            a,
                            Placement {
                                x: -16,
                                y: -16,
                                level: 1,
                            },
                        )?;
                    }
                    7 => {
                        let f = data(w, id, 8)?;
                        w.objects.get_mut(&id).unwrap().data.insert(8, f & !0x100);
                    }
                    8 => {
                        if function(w, p, id, 26)? {
                            let mut tile = self.old_tile;
                            tile[2] = position(w, id)?.level as i8;
                            request = Some(Request::Adjacency(forward(tile, self.old_facing)?));
                        }
                    }
                    9 => {
                        if let Some(object) = self.parent {
                            request = Some(Request::Function {
                                object,
                                function: 9,
                            });
                        }
                    }
                    10 => {
                        if word(w, p, id, 0x8a / 2)? == 6 {
                            let mut tile = self.old_tile;
                            tile[2] = position(w, id)?.level as i8;
                            state.marker(tile, false)?;
                        }
                    }
                    11 => request = Some(Request::RefreshAnimator(id)),
                    12 => {
                        if self.notify {
                            request = Some(Request::ViewRemoved(id));
                        }
                    }
                    _ => return Ok(None),
                },
                Kind::Set(d) => match phase {
                    0 => {
                        self.old = position(w, id)?;
                        self.old_outside = !state.links.capture_raw(w, id)?.on_lot;
                        request = Some(Request::Invalidate {
                            object: id,
                            children_only: false,
                        });
                    }
                    1 => {
                        if let Some(object) = d.container {
                            request = Some(Request::Invalidate {
                                object,
                                children_only: false,
                            });
                        }
                    }
                    2 => {
                        self.level = if let Some(parent) = d.container {
                            position(w, parent)?.level
                        } else {
                            d.level
                        };
                        request = Some(Request::Shadow(id));
                    }
                    3 => {
                        self.old_facing = data(w, id, 1)?;
                        self.old_tile = sims_vm::loading::coarse_position(position(w, id)?);
                        state.links.apply(
                            w,
                            id,
                            Descriptor {
                                on_lot: false,
                                position: [0, 0],
                                container: None,
                                slot: -1,
                                level: 1,
                            },
                        )?;
                    }
                    4 => {
                        if function(w, p, id, 26)? {
                            let mut tile = self.old_tile;
                            tile[2] = position(w, id)?.level as i8;
                            request = Some(Request::Adjacency(forward(tile, self.old_facing)?));
                        }
                    }
                    5 => {
                        self.marker = word(w, p, id, 0x8a / 2)? == 6;
                        if self.marker {
                            let mut tile = self.old_tile;
                            tile[2] = position(w, id)?.level as i8;
                            state.marker(tile, false)?;
                        }
                    }
                    6 => {
                        let point = if let Some(parent) = d.container {
                            let q = position(w, parent)?;
                            [q.x, q.y]
                        } else {
                            d.position
                        };
                        let descriptor = Descriptor {
                            on_lot: true,
                            position: point,
                            level: self.level,
                            slot: if d.container.is_some() { d.slot } else { -1 },
                            ..d
                        };
                        state.links.apply(w, id, descriptor)?;
                        let o = w
                            .objects
                            .get_mut(&id)
                            .ok_or("set placement object missing")?;
                        o.data
                            .extend([(2, d.container.unwrap_or(0)), (3, descriptor.slot as i16)]);
                        o.container = d.container.map(|parent| (parent, descriptor.slot as u16));
                        self.move_position(
                            w,
                            a,
                            Placement {
                                x: point[0],
                                y: point[1],
                                level: self.level,
                            },
                        )?;
                    }
                    7 => request = Some(Request::Shadow(id)),
                    8 => {
                        self.new_tile = sims_vm::loading::coarse_position(position(w, id)?);
                        request = Some(Request::Adjacency(self.new_tile));
                    }
                    9 => {
                        if function(w, p, id, 26)? {
                            let mut tile = self.new_tile;
                            tile[2] = position(w, id)?.level as i8;
                            request = Some(Request::Adjacency(forward(tile, data(w, id, 1)?)?));
                        }
                    }
                    10 => request = Some(Request::Walls(id)),
                    11 => {
                        if word(w, p, id, 10)? == 0 {
                            request = Some(self.call(9));
                        }
                    }
                    12 => {
                        if let Some(object) = d.container {
                            request = Some(Request::Function {
                                object,
                                function: 9,
                            });
                        }
                    }
                    13 => {
                        let now = position(w, id)?;
                        if [now.x, now.y] != [self.old.x, self.old.y] {
                            if w.registry_order.is_some() {
                                w.invalidate_registry_order();
                            } else {
                                state.spatial_dirty = Some(true);
                            }
                            if self.old_outside && self.notify {
                                request = Some(Request::ViewPlaced(id));
                            }
                        }
                    }
                    14 => {
                        if self.marker {
                            let mut tile = self.new_tile;
                            tile[2] = position(w, id)?.level as i8;
                            state.marker(tile, true)?;
                        }
                    }
                    15 => {
                        if data(w, id, 40)? as u16 & 0x400 != 0 {
                            request = Some(Request::Signal {
                                event: 0xf2,
                                value: id,
                            });
                        }
                    }
                    _ => return Ok(None),
                },
            }
            if let Some(r) = request {
                return Ok(Some(r));
            }
        }
    }
}

/// Virtual placement dispatch, including cXMTObject's two ordered passes
/// (0x422d86/0x422c2f). A part's live next link and offset are read after each
/// synchronous operation, so callback mutations affect the remaining traversal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VirtualOperation {
    Base(Operation),
    Group(Group),
    GroupRotation(group_rotation::Rotation),
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    selected: ObjectId,
    head: ObjectId,
    current: Option<ObjectId>,
    kind: Kind,
    notify: bool,
    selected_offset: [i32; 3],
    old_outside: bool,
    pass: u8,
    advance: bool,
    seen: BTreeSet<ObjectId>,
    base: Option<Operation>,
}
impl VirtualOperation {
    pub fn copy_direction(w: &World, id: ObjectId, desired: i16) -> Result<Self, String> {
        if w.objects.get(&id).is_some_and(|o| o.multitile.is_some()) {
            Ok(Self::GroupRotation(group_rotation::Rotation::new(
                w, id, desired,
            )?))
        } else {
            Ok(Self::Base(Operation::copy_direction(id, desired)?))
        }
    }
    pub fn binding(&self) -> (ObjectId, Option<Descriptor>) {
        match self {
            Self::Base(op) => op.binding(),
            Self::Group(g) => (
                g.selected,
                match g.kind {
                    Kind::Set(d) => Some(d),
                    _ => None,
                },
            ),
            Self::GroupRotation(r) => (r.object, None),
        }
    }
    fn validate_progress(&self, w: &World, rotation: bool) -> Result<bool, String> {
        match self {
            Self::Base(op) => {
                if matches!(op.kind, Kind::NativeRotate(_)) {
                    return Err("native notch call needs its rotation owner".into());
                }
                if rotation != matches!(op.kind, Kind::CopyDirection(_)) {
                    return Err("native placement/rotation kind mismatch".into());
                }
                if let Kind::CopyDirection(desired) = op.kind {
                    if ![0, 2, 4, 6].contains(&desired) || op.phase > 21 {
                        return Err("invalid native rotation continuation".into());
                    }
                }
                Ok(op.pending.is_empty()
                    && op.phase
                        > match op.kind {
                            Kind::Set(_) => 16,
                            Kind::Remove => 13,
                            Kind::CopyDirection(_) => 20,
                            Kind::NativeRotate(_) => unreachable!(),
                        })
            }
            Self::Group(g) => {
                if rotation
                    || g.pass > 4
                    || !sims_vm::multitile::chain(w, g.selected)?.contains(&g.head)
                    || g.current.is_some_and(|id| !w.objects.contains_key(&id))
                {
                    return Err("invalid live multipart placement".into());
                }
                Ok(g.pass == 4)
            }
            Self::GroupRotation(r) => {
                if !rotation {
                    return Err("unexpected multipart rotation".into());
                }
                r.validate(w)
            }
        }
    }
    fn validate_native_rotation(&self, object: ObjectId, notches: i32) -> Result<bool, String> {
        match self {
            Self::Base(op)
                if op.object == object
                    && op.kind == Kind::NativeRotate(notches)
                    && op.phase <= 5
                    && op.pending.is_empty() =>
            {
                Ok(op.phase == 5)
            }
            _ => Err("native notch rotation binding differs".into()),
        }
    }
    pub fn remove(w: &World, links: &Links, id: ObjectId, notify: bool) -> Result<Self, String> {
        Self::new(w, links, id, Kind::Remove, notify)
    }
    pub fn set(
        w: &World,
        links: &Links,
        id: ObjectId,
        destination: Descriptor,
        notify: bool,
    ) -> Result<Self, String> {
        Self::new(w, links, id, Kind::Set(destination), notify)
    }
    fn new(
        w: &World,
        links: &Links,
        id: ObjectId,
        kind: Kind,
        notify: bool,
    ) -> Result<Self, String> {
        let object = w
            .objects
            .get(&id)
            .ok_or("virtual placement object absent")?;
        // cXPerson vtable 0x667378 +0x58 is the same 0x433340 SetObject
        // routine as the base class. Keep its ordered callbacks and physical
        // writes; a person/group hybrid has no corresponding native dispatch.
        if object.person.is_some() && (!matches!(kind, Kind::Set(_)) || object.multitile.is_some())
        {
            return Err("person placement requires its own virtual services".into());
        }
        let Some(part) = &object.multitile else {
            return Ok(Self::Base(Operation::new(id, kind, notify)));
        };
        sims_vm::multitile::chain(w, id)?;
        let q = position(w, id)?;
        let head = part.head.unwrap_or(id);
        Ok(Self::Group(Group {
            selected: id,
            head,
            current: Some(head),
            kind,
            notify,
            selected_offset: part.offset,
            old_outside: !links.on_lot([q.x, q.y]),
            pass: 0,
            advance: false,
            seen: BTreeSet::new(),
            base: None,
        }))
    }
    pub fn next(
        &mut self,
        w: &mut World,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        match self {
            Self::Base(op) => op.next(w, p, a, state),
            Self::Group(g) => g.next(w, p, a, state),
            Self::GroupRotation(r) => r.next(w, p, a, state),
        }
    }
    /// Same explicit observer boundaries as the base import adapter.
    pub fn advance_loaded(
        &mut self,
        vm: &mut sims_vm::Vm,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        while let Some(request) = self.next(&mut vm.world, p, a, state)? {
            if let Some(other) = service_loaded(request, vm, p, a, state)? {
                return Ok(Some(other));
            }
        }
        state.links.publish_tile_chains(&mut vm.world)?;
        Ok(None)
    }
}
impl Group {
    fn next(
        &mut self,
        w: &mut World,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        loop {
            if self.pass >= 3 {
                self.pass = 4;
                return Ok(None);
            }
            if self.pass == 2 {
                self.pass = 3;
                return Ok(match self.kind {
                    Kind::Remove if self.notify => Some(Request::ViewRemoved(self.selected)),
                    Kind::Set(_) if self.old_outside && self.notify => {
                        Some(Request::ViewPlaced(self.selected))
                    }
                    _ => None,
                });
            }
            if let Some(base) = &mut self.base {
                if let Some(request) = base.next(w, p, a, state)? {
                    return Ok(Some(request));
                }
                self.base = None;
                self.advance = true;
            }
            if self.advance {
                let id = self.current.ok_or("group cursor absent")?;
                self.current = w
                    .objects
                    .get(&id)
                    .and_then(|o| o.multitile.as_ref())
                    .ok_or("group part removed during placement callback")?
                    .next;
                self.advance = false;
            }
            let Some(id) = self.current else {
                self.pass += 1;
                self.current = Some(self.head);
                self.seen.clear();
                continue;
            };
            if !self.seen.insert(id) {
                return Err("cyclic live multipart placement chain".into());
            }
            match (&self.kind, self.pass) {
                (Kind::Remove, 0) | (Kind::Set(_), 1) => {
                    self.advance = true;
                    return Ok(Some(Request::Function {
                        object: id,
                        function: if self.pass == 0 { 10 } else { 9 },
                    }));
                }
                (Kind::Remove, 1) => self.base = Some(Operation::remove(id, self.notify)),
                (Kind::Set(d), 0) => {
                    let offset = w
                        .objects
                        .get(&id)
                        .and_then(|o| o.multitile.as_ref())
                        .ok_or("group placement part absent")?
                        .offset;
                    let (position, level) = sims_vm::multitile::placement(
                        d.position,
                        d.level,
                        self.selected_offset,
                        offset,
                    );
                    // The native outside test uses the requested selected-part
                    // point, not the individual sibling point or its container.
                    self.base = Some(if state.links.on_lot(d.position) {
                        Operation::set(
                            id,
                            Descriptor {
                                position,
                                level,
                                ..*d
                            },
                            self.notify,
                        )
                    } else {
                        Operation::remove(id, self.notify)
                    });
                }
                _ => unreachable!(),
            }
        }
    }
}

fn service_loaded(
    request: Request,
    vm: &mut sims_vm::Vm,
    p: &Program,
    a: &Architecture,
    state: &mut State,
) -> Result<Option<Request>, String> {
    match request {
        Request::Function { object, function } => {
            loaded_function(vm, p, &state.links, object, function)?
        }
        Request::Adjacency(tile) => {
            if let Some(mut scan) =
                crate::lot_adjacency::Adjacency::begin(&state.links, &vm.world, p, tile)?
            {
                while let Some(update) = scan.next(&state.links, &mut vm.world, p)? {
                    if update.callback {
                        loaded_function(vm, p, &state.links, update.object, 9)?;
                    }
                }
            }
        }
        Request::Walls(id) => {
            let old = data(&vm.world, id, 5)?;
            let flags = wall_flags(&vm.world, p, a, id)?;
            if flags != old {
                vm.world.objects.get_mut(&id).unwrap().data.insert(5, flags);
                loaded_function(vm, p, &state.links, id, 6)?;
            }
        }
        Request::Signal { event, value } => vm.refresh_room_event(event, i32::from(value))?,
        other => return Ok(Some(other)),
    }
    Ok(None)
}

fn loaded_function(
    vm: &mut sims_vm::Vm,
    p: &Program,
    links: &Links,
    id: ObjectId,
    index: usize,
) -> Result<(), String> {
    links.publish_tile_chains(&mut vm.world)?;
    let prior = vm
        .object_threads
        .get(&id)
        .and_then(|thread| vm.threads.get(thread))
        .map(|t| t.status.clone());
    if let Some(thread) = vm.begin_loaded_function(p, id, index)? {
        vm.run_slice(p, thread, 100_000)?;
        let t = &vm.threads[&thread];
        if t.callback.is_some() || Some(&t.status) != prior.as_ref() {
            return Err(format!(
                "placement function {index} for object {id} needs callback service: {:?}",
                t.status
            ));
        }
    }
    Ok(())
}
