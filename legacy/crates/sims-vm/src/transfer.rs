//! Grab/Drop target selection from 0x451dda and 0x454430. Both primitives
//! ignore their operand bytes; the spatial host checks/commits placement.
use crate::{Location, ObjectId, World};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub grab: bool,
    /// Drop Onto resolves source/destination slots from literals or frame args.
    pub onto: Option<[i16; 2]>,
}
pub fn onto_slots(operands: &[u8; 8], args: &[i16]) -> Result<[i16; 2], String> {
    let mut slots = [0; 2];
    for (n, slot) in slots.iter_mut().enumerate() {
        let offset = n * 4;
        let mode = i16::from_le_bytes([operands[offset], operands[offset + 1]]);
        let value = i16::from_le_bytes([operands[offset + 2], operands[offset + 3]]);
        *slot = if mode == 0 {
            value
        } else {
            *usize::try_from(value)
                .ok()
                .and_then(|n| args.get(n))
                .ok_or("Drop Onto argument outside frame (native error 0x08)")?
        };
    }
    Ok(slots)
}
pub fn onto_object(
    world: &World,
    caller: ObjectId,
    target: ObjectId,
    slot: i16,
) -> Result<Option<ObjectId>, String> {
    let object = world
        .objects
        .get(&caller)
        .ok_or("Drop Onto caller missing")?;
    let held = usize::try_from(slot)
        .ok()
        .and_then(|n| object.slots.get(n))
        .ok_or("Drop Onto source slot missing (native error 0x1b)")?;
    if !world.objects.contains_key(&target) {
        return Err("Drop Onto target missing (native error 0x15)".into());
    }
    Ok(*held)
}
pub fn object(world: &World, caller: ObjectId, target: ObjectId, grab: bool) -> Option<ObjectId> {
    let id = if grab {
        target
    } else {
        *world.objects.get(&caller)?.slots.first()?.as_ref()?
    };
    world.objects.contains_key(&id).then_some(id)
}
