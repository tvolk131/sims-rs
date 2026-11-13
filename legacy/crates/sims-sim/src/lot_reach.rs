//! Native Reach preparation over the live physical containment graph.
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, World};
use std::collections::BTreeSet;

pub mod live;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepared {
    pub object: Option<ObjectId>,
    pub container: Option<ObjectId>,
    pub slot: i32,
    pub height_bits: u32,
}

fn registered(world: &World, id: ObjectId) -> Option<ObjectId> {
    (id > 0 && world.objects.contains_key(&id)).then_some(id)
}

/// 43404d reads the runtime slot's Z, not its height category or a world-space
/// bone transform. Each addition rounds to f32. An invalid initial slot returns
/// zero; invalid ancestor slots are skipped while traversal continues. The two
/// captures per ancestor are intentional: capture can normalize a dangling
/// zero-slot predecessor in the native object data.
pub fn height(
    world: &mut World,
    links: &Links,
    object: ObjectId,
    slot: i32,
    mut runtime_z: impl FnMut(&World, ObjectId, usize) -> Result<f32, String>,
) -> Result<f32, String> {
    let valid = |world: &World, object, slot| {
        usize::try_from(slot).ok().filter(|&slot| {
            world
                .objects
                .get(&object)
                .is_some_and(|o| slot < o.slots.len())
        })
    };
    if valid(world, object, slot).is_none() {
        return Ok(0.);
    }
    let (mut current, mut slot, mut height) = (Some(object), slot, 0f32);
    let mut seen = BTreeSet::new();
    while let Some(object) = current {
        if !seen.insert(object) {
            return Err("cyclic native Reach ancestry".into());
        }
        if let Some(slot) = valid(world, object, slot) {
            height += runtime_z(world, object, slot)?;
        }
        current = links.capture(world, object)?.container;
        slot = i32::from(links.capture(world, object)?.slot as i16);
    }
    Ok(height)
}

/// 45c848. Capture side effects are retained even on a false result. Native
/// argument errors occur only after the destination registry lookup succeeds.
/// Runtime slot data is a host input; unknown metadata must remain an error.
pub fn prepare(
    world: &mut World,
    links: &Links,
    caller: ObjectId,
    target: ObjectId,
    args: &[i16],
    operands: &[u8; 8],
    runtime_z: impl FnMut(&World, ObjectId, usize) -> Result<f32, String>,
) -> Result<Option<Prepared>, String> {
    let mode = i16::from_le_bytes(operands[..2].try_into().unwrap());
    if mode == 2 {
        return Ok(Some(Prepared {
            object: None,
            container: None,
            slot: 0,
            height_bits: (-1f32).to_bits(),
        }));
    }
    let (object, container, slot) = match mode {
        0 => {
            let Some(object) = registered(world, target) else {
                return Ok(None);
            };
            let d = links.capture(world, object)?;
            (Some(object), d.container, d.slot)
        }
        1 => {
            let Some(container) = registered(world, target) else {
                return Ok(None);
            };
            let object = world
                .objects
                .get(&caller)
                .ok_or("native Reach caller absent")?
                .slots
                .first()
                .copied()
                .flatten()
                .and_then(|id| registered(world, id));
            let index = i16::from_le_bytes(operands[4..6].try_into().unwrap());
            let slot = *usize::try_from(index)
                .ok()
                .and_then(|i| args.get(i))
                .ok_or("Reach slot argument invalid (native error 0x08)")?;
            (object, Some(container), i32::from(slot))
        }
        _ => return Ok(None),
    };
    let height = if let Some(container) = container {
        if usize::try_from(slot)
            .ok()
            .is_none_or(|slot| slot >= world.objects[&container].slots.len())
        {
            return Ok(None);
        }
        let z = height(world, links, container, slot, runtime_z)?;
        // 454f29 caches the reciprocal as f32, then performs both products in
        // x87 before 45c848 stores its final single-precision result.
        (f64::from(z) * f64::from((1. / (2. * 6f64.sqrt())) as f32) * 3.) as f32
    } else {
        0.
    };
    Ok(Some(Prepared {
        object,
        container,
        slot,
        height_bits: height.to_bits(),
    }))
}
