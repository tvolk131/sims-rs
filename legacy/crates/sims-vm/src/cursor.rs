//!cXCursorObject421afb storage and42238d offset refresh. This is separate from
//!the4215e6 factory, its pickup/Reset/placement and application notifications.
use crate::{ObjectId, Program, World, construction::definition_words, multitile};
use serde::{Deserialize, Serialize};
pub mod orientation;
pub mod reset;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    ///Replacement continuation marker; not a native field. Registration and
    ///virtual+3c must remain separate across ordinary furniture's pickup call.
    pub postconstructed: bool,
    ///Native+1ac pointer represented by a non-reused registry lifetime handle.
    pub underlying: Option<u32>,
    pub valid: i16,
    pub purchase: i16,
    pub position: [i32; 2],
    pub level: i32,
    ///Unidentified+1c0 floating-point storage, preserved as exact bits.
    pub field_1c0: u32,
    pub container: i16,
    pub slot: i16,
    pub original_container: i16,
    pub original_slot: i16,
}
impl State {
    pub fn constructed(underlying: Option<u32>, container: Option<(i16, i16)>) -> Self {
        let (container, slot) = underlying.and(container).unwrap_or((0, 0));
        Self {
            postconstructed: false,
            underlying,
            valid: 0,
            purchase: 0,
            position: [-16, -16],
            level: 1,
            field_1c0: (-1f32).to_bits(),
            container,
            slot,
            original_container: container,
            original_slot: slot,
        }
    }
}
///421986's success tail starts at the returned cursor, not the multipart head.
///The dynamic cast stops on null or an ordinary object. No callbacks occur.
pub fn mark_purchase(world: &mut World, returned: ObjectId) -> Result<(), String> {
    let mut next = Some(returned);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = next {
        let Some(object) = world.objects.get_mut(&id).filter(|o| o.cursor.is_some()) else {
            break;
        };
        if !seen.insert(id) {
            return Err("cyclic purchase cursor chain".into());
        }
        next = object
            .multitile
            .as_ref()
            .ok_or("purchase cursor chain absent")?
            .next;
        object.cursor.as_mut().unwrap().purchase = 1;
    }
    Ok(())
}
fn id(world: &World, handle: u32) -> Result<ObjectId, String> {
    world
        .scheduler_storage
        .as_ref()
        .and_then(|s| s.object_for_handle(handle))
        .filter(|id| world.objects.contains_key(id))
        .ok_or("cursor furniture lifetime ended".into())
}
fn underlying_group(
    world: &World,
    p: &Program,
    handle: Option<u32>,
) -> Result<Option<ObjectId>, String> {
    let Some(handle) = handle else {
        return Ok(None);
    };
    let id = id(world, handle)?;
    let o = world.objects.get(&id).ok_or("cursor furniture absent")?;
    let d = o
        .definition
        .as_ref()
        .ok_or("cursor furniture definition absent")?;
    if definition_words(p, d)?[10] == 0 {
        return Ok(None);
    }
    if o.multitile.is_none() || o.cursor.is_some() {
        return Err("cursor offset refresh needs the original multipart furniture receiver".into());
    }
    Ok(Some(id))
}
pub fn origin(words: &[i16]) -> Result<[i32; 3], String> {
    //422872 calls43183c even though the cursor resource has no OBJD group.
    if words.len() <= 56 {
        return Err("cursor origin words absent".into());
    }
    Ok([
        (words[11] >> 8) as i8 as i32,
        words[11] as u8 as i32,
        i32::from(words[56]),
    ])
}
fn copy_offsets(world: &mut World, head: ObjectId) -> Result<(), String> {
    let mut current = Some(head);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(cursor) = current {
        if !seen.insert(cursor) {
            return Err("cyclic cursor chain".into());
        }
        let underlying = world
            .objects
            .get(&cursor)
            .and_then(|o| o.cursor.as_ref())
            .ok_or("cursor chain contains ordinary furniture")?
            .underlying;
        if let Some(handle) = underlying {
            let target = id(world, handle)?;
            let offset = world.objects[&target]
                .multitile
                .as_ref()
                .ok_or("cursor underlying offset storage absent")?
                .offset;
            world
                .objects
                .get_mut(&cursor)
                .unwrap()
                .multitile
                .as_mut()
                .ok_or("cursor links absent")?
                .offset = offset;
        }
        //A null underlying pointer leaves this member's old offset untouched.
        current = world.objects[&cursor]
            .multitile
            .as_ref()
            .ok_or("cursor links absent")?
            .next;
    }
    Ok(())
}
///42238d. The ordinary/null case zeros only this cursor; the multipart case
///refreshes the furniture first, then rereads the cursor head and each member.
pub fn refresh_offsets(world: &mut World, p: &Program, cursor: ObjectId) -> Result<(), String> {
    let underlying = world
        .objects
        .get(&cursor)
        .and_then(|o| o.cursor.as_ref())
        .ok_or("cursor receiver absent")?
        .underlying;
    if let Some(target) = underlying_group(world, p, underlying)? {
        multitile::update_offsets(world, target)?;
        let head = world.objects[&cursor]
            .multitile
            .as_ref()
            .ok_or("cursor links absent")?
            .head
            .unwrap_or(cursor);
        copy_offsets(world, head)
    } else {
        world
            .objects
            .get_mut(&cursor)
            .unwrap()
            .multitile
            .as_mut()
            .ok_or("cursor links absent")?
            .offset = [0; 3];
        Ok(())
    }
}
///Constructor's offset refresh before registration. The new unregistered tail
///is held in local storage. Its furniture+88 is the callback-free42369d body;
///only after that returns are existing cursor offsets and the new tail copied.
///The tail's ID-based link is published on registration, before any Reset/Init
///or graphic service can observe it. Cursor-backed multipart furniture is not
///silently dispatched through42369d.
pub(crate) fn constructed_offset(
    world: &mut World,
    p: &Program,
    underlying: Option<u32>,
    head: Option<ObjectId>,
) -> Result<[i32; 3], String> {
    let Some(target) = underlying_group(world, p, underlying)? else {
        return Ok([0; 3]);
    };
    multitile::update_offsets(world, target)?;
    if let Some(head) = head {
        copy_offsets(world, head)?;
    }
    Ok(world.objects[&target].multitile.as_ref().unwrap().offset)
}
pub(crate) fn validate_chain(
    world: &World,
    p: &Program,
    ids: &[ObjectId],
    retired: &[(ObjectId, u32)],
) -> Result<(), String> {
    let first = world.objects[&ids[0]]
        .definition
        .as_ref()
        .ok_or("cursor definition absent")?;
    for object in ids {
        let o = &world.objects[object];
        let cursor = o.cursor.as_ref().ok_or("mixed cursor/furniture chain")?;
        let d = o.definition.as_ref().ok_or("cursor definition absent")?;
        let part = o.multitile.as_ref().ok_or("cursor links absent")?;
        let expected_origin = if cursor.postconstructed {
            origin(definition_words(p, d)?)?
        } else {
            [0; 3]
        };
        if o.person.is_some() || d != first || part.origin != expected_origin {
            return Err("invalid cursor part".into());
        }
        if let Some(target) = cursor.underlying {
            if !retired.contains(&(*object, target)) {
                id(world, target)?;
            }
        }
        //Offsets are copies from represented objects, not resource-origin
        //rotations. Later furniture mutations may await a cursor refresh.
    }
    Ok(())
}

#[cfg(test)]
mod purchase_tests {
    use super::*;
    #[test]
    fn purchase_mark_starts_at_returned_part_and_stops_at_failed_cast() {
        let mut world = World::default();
        for (id, next, cursor) in [
            (1, Some(2), true),
            (2, Some(3), true),
            (3, Some(4), false),
            (4, None, true),
        ] {
            world.objects.insert(
                id,
                crate::Object {
                    cursor: cursor.then(|| State::constructed(None, None)),
                    multitile: Some(multitile::Part {
                        head: (id != 1).then_some(1),
                        next,
                        origin: [0; 3],
                        offset: [0; 3],
                    }),
                    ..Default::default()
                },
            );
        }
        mark_purchase(&mut world, 2).unwrap();
        assert_eq!(world.objects[&1].cursor.as_ref().unwrap().purchase, 0);
        assert_eq!(world.objects[&2].cursor.as_ref().unwrap().purchase, 1);
        assert_eq!(world.objects[&4].cursor.as_ref().unwrap().purchase, 0);
        let before = world.clone();
        mark_purchase(&mut world, 0).unwrap();
        mark_purchase(&mut world, 3).unwrap();
        assert_eq!(world, before);
    }
}
