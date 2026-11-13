//! cXMTObject links and grid offsets from the recovered 1.91 executable.
//! Group placement and deletion services are owned by the simulation host.
use crate::{Module, ObjectId, Program, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Part {
    /// Native +0x190 is null on the head; every secondary points to that head.
    pub head: Option<ObjectId>,
    pub next: Option<ObjectId>,
    pub origin: [i32; 3],
    pub offset: [i32; 3],
}
/// 0x43acf5 selects the primary resource, then 0x43ad79 supplies other parts
/// in resource order. The catalog definition itself is never instantiated.
pub fn construction_plan(module: &Module, catalog: u16) -> Result<Vec<u16>, String> {
    let words = module
        .object_definition_words
        .get(&catalog)
        .ok_or("missing catalog definition")?;
    if words.len() <= 63 || words[10] == 0 || words[11] != -1 {
        return Err("group construction requires a catalog master".into());
    }
    let definitions: Vec<_> = module
        .object_definition_order
        .iter()
        .copied()
        .filter(|id| {
            module
                .object_definition_words
                .get(id)
                .is_some_and(|w| w.len() > 63 && w[10] == words[10])
        })
        .collect();
    let primary = definitions
        .iter()
        .copied()
        .find(|id| module.object_definition_words[id][63] != 0)
        .or_else(|| {
            definitions.iter().copied().find(|id| {
                let w = &module.object_definition_words[id];
                w[11] == 0 && w[56] == 0
            })
        })
        .ok_or("multi-tile group has no primary part")?;
    if module.object_definition_words[&primary][11] == -1 {
        return Err("catalog resource selected as group primary is unsupported".into());
    }
    Ok(std::iter::once(primary)
        .chain(
            definitions
                .into_iter()
                .filter(|id| *id != primary && module.object_definition_words[id][11] != -1),
        )
        .collect())
}
pub fn origin(words: &[i16]) -> Result<[i32; 3], String> {
    if words.len() <= 0x70 / 2 || words[10] == 0 || words[11] == -1 {
        return Err("multi-tile part needs a non-catalog group definition".into());
    }
    Ok([
        (words[11] >> 8) as i8 as i32,
        words[11] as u8 as i32,
        words[0x70 / 2] as i32,
    ])
}
/// Native 0x42369d: diagonal directions deliberately produce zero XY offsets.
pub fn rotate([x, y, z]: [i32; 3], direction: i16) -> [i32; 3] {
    let (x, y) = match direction & 7 {
        0 => (x, y),
        2 => (y.wrapping_neg(), x),
        4 => (x.wrapping_neg(), y.wrapping_neg()),
        6 => (y, x.wrapping_neg()),
        _ => (0, 0),
    };
    [x, y, z]
}
/// Native 0x422022 converts resource grid offsets to physical coordinates.
pub fn world_offset([x, y, _]: [i32; 3]) -> [i32; 2] {
    [y.wrapping_mul(16), x.wrapping_mul(16)]
}
/// Original 0x422c2f: relocate any selected part, deriving all siblings from its
/// rotated offset. Position is native X/Y; the level offset is in whole storeys.
pub fn placement(
    position: [i32; 2],
    level: i32,
    selected: [i32; 3],
    sibling: [i32; 3],
) -> ([i32; 2], i32) {
    let from = world_offset(selected);
    let to = world_offset(sibling);
    (
        std::array::from_fn(|i| position[i].wrapping_sub(from[i]).wrapping_add(to[i])),
        level.wrapping_sub(selected[2]).wrapping_add(sibling[2]),
    )
}
pub fn next(world: &World, current: ObjectId) -> ObjectId {
    world
        .objects
        .get(&current)
        .and_then(|o| o.multitile.as_ref())
        .and_then(|p| p.next.or(p.head))
        .unwrap_or(0)
}
/// Native Test Object Type accepts the part's own GUID or its catalog GUID.
pub fn is_type(
    world: &World,
    program: &Program,
    object: ObjectId,
    guid: u32,
) -> Result<bool, String> {
    let object = world
        .objects
        .get(&object)
        .ok_or("test object type missing object (native error 0x17)")?;
    let Some(def) = &object.definition else {
        return Ok(false);
    };
    if def.guid == guid {
        return Ok(true);
    }
    let module = program
        .modules
        .get(&def.source)
        .ok_or("type resource module unavailable")?;
    let words = crate::construction::definition_words(program, def)?;
    if words[10] == 0 {
        return Ok(false);
    }
    let catalog = module
        .object_definition_order
        .iter()
        .find_map(|id| {
            let w = &module.object_definition_words[id];
            (w[10] == words[10] && w[11] == -1).then_some(w)
        })
        .ok_or("multi-tile catalog parent unavailable")?;
    Ok(guid == (catalog[14] as u16 as u32 | ((catalog[15] as u16 as u32) << 16)))
}
pub fn chain(world: &World, member: ObjectId) -> Result<Vec<ObjectId>, String> {
    chain_with(member, |id| {
        world.objects.get(&id).and_then(|o| o.multitile.as_ref())
    })
}

/// Validate an explicitly retained part graph without constructing unrelated
/// World/object state. Uses exactly the live graph traversal below.
pub fn retained_chain(
    parts: &std::collections::BTreeMap<ObjectId, Part>,
    member: ObjectId,
) -> Result<Vec<ObjectId>, String> {
    chain_with(member, |id| parts.get(&id))
}

fn chain_with<'a>(
    member: ObjectId,
    mut part: impl FnMut(ObjectId) -> Option<&'a Part>,
) -> Result<Vec<ObjectId>, String> {
    let head = part(member)
        .ok_or("missing multi-tile part")?
        .head
        .unwrap_or(member);
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    let mut cursor = Some(head);
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return Err("cyclic multi-tile next link".into());
        }
        let part = part(id).ok_or("dangling multi-tile link")?;
        if part.head != (id != head).then_some(head) {
            return Err("inconsistent multi-tile head link".into());
        }
        result.push(id);
        cursor = part.next;
    }
    if !seen.contains(&member) {
        return Err("multi-tile member is absent from its chain".into());
    }
    Ok(result)
}
/// 0x4228d6, used by the multipart storage destructor and the separate split
/// operation. Promote the next part when removing the head; cached physical
/// offsets are retained until an explicit 0x42369d refresh.
pub fn unlink(world: &mut World, member: ObjectId) -> Result<(), String> {
    let ids = chain(world, member)?;
    let index = ids.iter().position(|&id| id == member).unwrap();
    let next = world.objects[&member].multitile.as_ref().unwrap().next;
    if index == 0 {
        for &id in &ids[1..] {
            world
                .objects
                .get_mut(&id)
                .unwrap()
                .multitile
                .as_mut()
                .unwrap()
                .head = next.filter(|&head| head != id);
        }
    } else {
        world
            .objects
            .get_mut(&ids[index - 1])
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .next = next;
    }
    let selected = world
        .objects
        .get_mut(&member)
        .unwrap()
        .multitile
        .as_mut()
        .unwrap();
    selected.head = None;
    selected.next = None;
    Ok(())
}
pub fn update_offsets(world: &mut World, member: ObjectId) -> Result<(), String> {
    let ids = chain(world, member)?;
    let head = &world.objects[&ids[0]];
    let base = head.multitile.as_ref().unwrap().origin;
    let direction = *head.data.get(&1).ok_or("missing group direction")?;
    for id in ids {
        let part = world
            .objects
            .get_mut(&id)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap();
        part.offset = rotate(
            std::array::from_fn(|i| part.origin[i].wrapping_sub(base[i])),
            direction,
        );
    }
    Ok(())
}
/// Native 0x42344e: dynamic groups are identified by their function-8 action.
pub fn dynamic(world: &World, program: &Program, member: ObjectId) -> Result<bool, String> {
    let object = world
        .objects
        .get(&member)
        .ok_or("dynamic group object absent")?;
    if object.multitile.is_none() {
        return Ok(false);
    }
    let d = object
        .definition
        .as_ref()
        .ok_or("dynamic group definition absent")?;
    let table = program
        .modules
        .get(&d.source)
        .and_then(|m| m.object_functions.get(&d.object))
        .ok_or("dynamic group function table absent")?;
    Ok(table.get(8).is_some_and(|f| f.action != 0))
}
/// Native 0x423368 appends the incoming group to the selected member's group.
/// It rewrites runtime offsets and facing, leaving physical positions and Z
/// offsets alone. In particular this is not a resource-offset refresh.
pub fn merge(world: &mut World, selected: ObjectId, incoming: ObjectId) -> Result<(), String> {
    let left = chain(world, selected)?;
    let right = chain(world, incoming)?;
    if left[0] == right[0] {
        return Ok(());
    }
    let head = &world.objects[&left[0]];
    let direction = *head.data.get(&1).ok_or("dynamic group direction absent")?;
    let offset = world_offset(head.multitile.as_ref().unwrap().offset);
    let position = world
        .positions
        .get(&left[0])
        .ok_or("dynamic group position absent")?;
    // Validate all reads before publishing this callback-free native operation.
    let changes = right
        .iter()
        .map(|&id| {
            let q = world
                .positions
                .get(&id)
                .ok_or("incoming group position absent")?;
            let mut part = world.objects[&id].multitile.as_ref().unwrap().clone();
            part.head = Some(left[0]);
            part.offset[0] = q.y.wrapping_sub(position.y).wrapping_add(offset[1]) >> 4;
            part.offset[1] = q.x.wrapping_sub(position.x).wrapping_add(offset[0]) >> 4;
            part.origin = rotate(
                [part.offset[0], part.offset[1], part.origin[2]],
                direction.wrapping_neg(),
            );
            Ok((id, part))
        })
        .collect::<Result<Vec<_>, String>>()?;
    world
        .objects
        .get_mut(left.last().unwrap())
        .unwrap()
        .multitile
        .as_mut()
        .unwrap()
        .next = Some(right[0]);
    for (id, part) in changes {
        let object = world.objects.get_mut(&id).unwrap();
        object.multitile = Some(part);
        object.data.insert(1, direction);
    }
    Ok(())
}
pub fn validate(world: &World, program: &Program) -> Result<(), String> {
    validate_with_cursor_cleanup(world, program, &[])
}
pub(crate) fn validate_with_cursor_cleanup(
    world: &World,
    program: &Program,
    retired: &[(ObjectId, u32)],
) -> Result<(), String> {
    validate_with_construction(world, program, retired, &BTreeSet::new())
}
///The snapshot caller has validated the complete construction proof, including
///resource order, pending raw origin, zero offset cache and no script lifecycle.
pub(crate) fn validate_with_construction(
    world: &World,
    program: &Program,
    retired: &[(ObjectId, u32)],
    constructing: &BTreeSet<ObjectId>,
) -> Result<(), String> {
    let unique: BTreeSet<_> = retired.iter().copied().collect();
    if unique.len() != retired.len()
        || retired.iter().any(|(id, h)| {
            world
                .objects
                .get(id)
                .and_then(|o| o.cursor.as_ref())
                .is_none_or(|c| c.underlying != Some(*h))
                || world
                    .scheduler_storage
                    .as_ref()
                    .is_none_or(|s| !s.retired_handle(*h))
        })
    {
        return Err("invalid cursor cleanup lifetime proof".into());
    }
    let mut seen = BTreeSet::new();
    for (&id, object) in &world.objects {
        if object.cursor.is_some() && object.multitile.is_none() {
            return Err("cursor object has no multipart storage".into());
        }
        if object.multitile.is_none() || seen.contains(&id) {
            continue;
        }
        let ids = chain(world, id)?;
        if ids.iter().any(|id| constructing.contains(id)) {
            if !ids.iter().all(|id| constructing.contains(id)) {
                return Err("partially owned group construction chain".into());
            }
            seen.extend(ids);
            continue;
        }
        if object.cursor.is_some() {
            crate::cursor::validate_chain(world, program, &ids, retired)?;
            seen.extend(ids);
            continue;
        }
        let head = &world.objects[&ids[0]];
        let def = head.definition.as_ref().ok_or("group definition missing")?;
        let words = crate::construction::definition_words(program, def)?;
        let base = head.multitile.as_ref().unwrap().origin;
        let cached_base = head.multitile.as_ref().unwrap().offset;
        let dynamic = ids.iter().try_fold(false, |found, &id| {
            dynamic(world, program, id).map(|value| found || value)
        })?;
        // Cached +1a0/+1a4/+1a8 offsets are rebuilt by 42369d, not by a raw
        // direction write. In particular 4550a0 stores the selected part's
        // facing after group placement without rotating the offset cache.
        // 4228d6 can also promote a head without refreshing offsets. Validate
        // relative cached offsets; the promoted head need not have offset zero.
        let mut cached_directions: Vec<i16> = (0..8).collect();
        for part_id in ids {
            let object = &world.objects[&part_id];
            let part = object.multitile.as_ref().unwrap();
            let d = object
                .definition
                .as_ref()
                .ok_or("part definition missing")?;
            let w = crate::construction::definition_words(program, d)?;
            let resource_origin = origin(w)?;
            if object.person.is_some()
                || object.cursor.is_some()
                || d.source != def.source
                || w[10] != words[10]
                || if dynamic {
                    part.origin[2] != resource_origin[2]
                } else {
                    part.origin != resource_origin
                }
                || !seen.insert(part_id)
            {
                return Err("invalid multi-tile part".into());
            }
            cached_directions.retain(|&direction| {
                std::array::from_fn(|i| part.offset[i].wrapping_sub(cached_base[i]))
                    == rotate(
                        std::array::from_fn(|i| part.origin[i].wrapping_sub(base[i])),
                        direction,
                    )
            });
        }
        // Joining dynamic groups stores runtime origins, not OBJD constants;
        // 423368 also retains each part's Z cache and the receiving head's
        // origin. The static resource-relative cache invariant does not apply.
        if !dynamic && cached_directions.is_empty() {
            return Err("inconsistent multi-tile cached offsets".into());
        }
    }
    Ok(())
}

/// Native 0x435286: positive OBJD tree-table indices redirect to the first
/// group part with the corresponding negative index, retaining the original
/// target if absent. This is distinct from unconditional group-head selection.
pub fn interaction_target(
    world: &World,
    p: &Program,
    target: ObjectId,
) -> Result<ObjectId, String> {
    let object = world
        .objects
        .get(&target)
        .ok_or("interaction target missing")?;
    let d = object
        .definition
        .as_ref()
        .ok_or("interaction target definition missing")?;
    let words = crate::construction::definition_words(p, d)?;
    let index = words[8];
    if words[10] == 0 || index <= 0 || object.multitile.is_none() {
        return Ok(target);
    }
    let ids = chain(world, target)?;
    for id in ids {
        let d = world.objects[&id]
            .definition
            .as_ref()
            .ok_or("interaction group definition missing")?;
        if crate::construction::definition_words(p, d)?[8] == -index {
            return Ok(id);
        }
    }
    Ok(target)
}
