//! Native 0x4329c1 containment branch, after its shared environmental prefix.
//! Raw slot occupancy is not a registry lookup. Ancestor capture can normalize
//! sentinel predecessors, but never moves the object or runs placement scripts.
use super::Outcome;
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, World};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slot {
    pub offset: [f32; 3],
    pub height: i32,
    pub occupant: ObjectId,
}
/// Current native slot metadata. A retained cache takes precedence over the
/// SLOT initialization resource and the person's built-in appended profile.
pub fn slots(world: &World, p: &Program, container: ObjectId) -> Result<Vec<Slot>, String> {
    let o = world
        .objects
        .get(&container)
        .ok_or("contained placement parent absent")?;
    if let Some(cached) = &o.native_slot_geometry {
        if cached.len() != o.slots.len() {
            return Err("retained containment geometry/allocation mismatch".into());
        }
        return cached
            .iter()
            .zip(&o.slots)
            .map(|(s, occupant)| {
                s.validate()?;
                Ok(Slot {
                    offset: s.offset.map(f32::from_bits),
                    height: s.height,
                    occupant: occupant.unwrap_or(0),
                })
            })
            .collect();
    }
    // Resource-initialized authored/load profiles retain the existing path.
    // The completed native composer requires observed geometry for every
    // object and cannot use this as a missing-live-cache substitution.
    let d = o
        .definition
        .as_ref()
        .ok_or("contained placement parent definition absent")?;
    let m = p
        .modules
        .get(&d.source)
        .ok_or("contained placement parent resource absent")?;
    let mut slots = vec![];
    if d.slots != 0 {
        for s in m
            .slots
            .get(&d.slots)
            .ok_or("contained placement SLOT not loaded")?
            .entries
            .iter()
            .filter(|s| s.kind == 0)
        {
            let (height, z) = sims_vm::routing::containment_height(s.height, s.offset[2]);
            slots.push(Slot {
                offset: [s.offset[0], s.offset[1], z],
                height,
                occupant: 0,
            });
        }
    }
    if o.person.is_some() {
        slots.extend(
            sims_vm::person::BUILTIN_SLOT_HEIGHTS
                .into_iter()
                .map(|height| Slot {
                    offset: [0.; 3],
                    height,
                    occupant: 0,
                }),
        );
    }
    if slots.len() != o.slots.len() {
        return Err("native containment slot allocation/resource mismatch".into());
    }
    for (slot, occupant) in slots.iter_mut().zip(&o.slots) {
        slot.occupant = occupant.unwrap_or(0);
    }
    Ok(slots)
}

pub fn check(
    world: &mut World,
    links: &Links,
    object: ObjectId,
    container: ObjectId,
    slot: i32,
    slots: &[Slot],
    mode: i16,
    error: i32,
) -> Result<Outcome, String> {
    let reject = |error| {
        Ok(Outcome {
            accepted: false,
            error,
        })
    };
    let o = world
        .objects
        .get(&object)
        .ok_or("contained placement object absent")?;
    let parent = world
        .objects
        .get(&container)
        .ok_or("contained placement parent absent")?;
    let word = |o: &sims_vm::Object, n| {
        o.data
            .get(&n)
            .copied()
            .ok_or("contained placement word absent")
    };
    let flags = word(o, 8)? as u16;
    let person = o.person.is_some();
    let weight = word(o, 27)?;
    let height_mask = word(o, 4)?;
    if flags & 4 != 0 || word(parent, 8)? as u16 & 4 != 0 {
        return reject(12);
    }
    let Some(target) = usize::try_from(slot).ok().and_then(|i| slots.get(i)) else {
        return reject(13);
    };
    if target.occupant != 0 && target.occupant != object {
        return reject(11);
    }
    let center = |s: &Slot| s.offset[0] == 0. && s.offset[1] == 0.;
    if !person && flags & 0x2000 == 0 && mode == 2 && !center(target) && slots.len() > 1 {
        return reject(3);
    }
    if parent.person.is_some() && mode != 0 {
        return reject(13);
    }
    for other in slots.iter().rev() {
        if other.height == target.height && other.occupant != 0 && other.occupant != object {
            if center(other) && !person {
                let occupied = world
                    .objects
                    .get(&other.occupant)
                    .ok_or("native same-height slot occupant absent")?;
                if word(occupied, 8)? as u16 & 0x2000 == 0 {
                    return reject(11);
                }
            }
            if center(target) && !person && flags & 0x2000 == 0 {
                return reject(11);
            }
        }
    }
    if weight > word(parent, 28)? {
        return reject(14);
    }
    let mut current = Some(container);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = current {
        if id == object {
            return reject(11);
        }
        if !seen.insert(id) {
            return Err("cyclic native placement ancestors".into());
        }
        current = links.capture(world, id)?.container;
    }
    // x86 shifts use the low five bits; the height mask is sign-extended.
    if target.height == 0
        || height_mask == 0
        || (i32::from(height_mask) as u32) & 1u32.wrapping_shl(target.height.wrapping_sub(1) as u32)
            == 0
    {
        return reject(24);
    }
    Ok(Outcome {
        accepted: true,
        error,
    })
}
