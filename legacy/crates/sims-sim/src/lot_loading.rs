//! Spatial part of original post-load, before each object's Load/Reset/Init.
//! This operates on saved VM storage, without publishing all objects' rooms
//! early or advancing a simulation tick.
use crate::{Point, architecture::Architecture, spatial};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World, autonomy::Placement};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpatialUpdate {
    pub object: ObjectId,
    pub position: [i32; 3],
    pub coarse: [i8; 3],
    /// Native rectangle storage: max X, max Y, min X, min Y.
    pub bounds: [i32; 4],
    pub room: i16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// 0x434101(1): invalidate only this object's drawing cache, before moving.
    Invalidate(ObjectId),
    /// 0x40760d(0xf3, room). Subscribers remain a host boundary.
    Room(i16),
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementUpdate {
    pub objects: Vec<SpatialUpdate>,
    pub effects: Vec<Effect>,
}

/// Native 0x433e50, called by base post-load with its existing location. The
/// caller normalizes a zero root level to one first (0x431e2e). Children take
/// the parent's physical position and level, in physical slot order; slot zero
/// is a tile link and is deliberately absent from `Object::slots`.
pub fn restore_position(
    world: &mut World,
    architecture: &Architecture,
    root: ObjectId,
    position: Placement,
) -> Result<PlacementUpdate, String> {
    let update = plan_position(world, architecture, root, position)?;
    // Validate before mutation; a malformed import cannot leave half a subtree
    // in a new room. Successful updates retain the original visitation order.
    for o in &update.objects {
        world.positions.insert(o.object, position);
        world
            .objects
            .get_mut(&o.object)
            .unwrap()
            .data
            .insert(29, o.room);
    }
    Ok(update)
}

/// Calculate the same spatial traversal without publishing its writes. Live
/// placement interleaves these writes with synchronous drawing/room observers;
/// planning must not move children before those observers have run.
pub(crate) fn plan_position(
    world: &World,
    architecture: &Architecture,
    root: ObjectId,
    position: Placement,
) -> Result<PlacementUpdate, String> {
    if architecture.original.is_none() || !(1..=2).contains(&position.level) {
        return Err("loaded placement needs original architecture and a supported floor".into());
    }
    let room = architecture.room_at(position.level, Point::new(position.y, position.x));
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut update = PlacementUpdate::default();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err("cyclic or duplicate loaded containment".into());
        }
        let object = world
            .objects
            .get(&id)
            .ok_or("loaded placement root is absent")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("loaded placement definition absent")?;
        let shape = architecture
            .footprints
            .get(&definition.guid)
            .ok_or("loaded placement footprint absent")?;
        let facing = *object.data.get(&1).ok_or("loaded direction absent")?;
        if !(0..=7).contains(&facing) {
            return Err("loaded direction outside native footprint contract".into());
        }
        let person_field = |index| -> Result<u16, String> {
            if shape.kind != 2 {
                return Ok(0);
            }
            Ok(*object
                .person
                .as_ref()
                .and_then(|p| p.get(&index))
                .ok_or("loaded person footprint field absent")? as u16)
        };
        let bounds = spatial::footprint(
            [position.x, position.y],
            shape.kind == 2,
            (person_field(68)? | person_field(84)?) & 1 != 0,
            shape.size,
            shape.insets,
            person_field(73)?,
            facing as u8,
        );
        update.effects.push(Effect::Invalidate(id));
        let old_room = *object.data.get(&29).ok_or("loaded room field absent")?;
        if old_room != room && *object.data.get(&7).ok_or("loaded lighting field absent")? != 0 {
            update
                .effects
                .extend([Effect::Room(old_room), Effect::Room(room)]);
        }
        update.objects.push(SpatialUpdate {
            object: id,
            position: [position.x, position.y, position.level],
            coarse: sims_vm::loading::coarse_position(position),
            bounds,
            room,
        });
        // Native registry lookup skips absent child IDs. A root must exist.
        pending.extend(
            object
                .slots
                .iter()
                .rev()
                .flatten()
                .copied()
                .filter(|id| world.objects.contains_key(id)),
        );
    }
    Ok(update)
}
