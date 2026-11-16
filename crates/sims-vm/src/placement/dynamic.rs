//! A dynamic group (a group whose type has callback function 8, such as
//! flood.iff's puddles) joining the tiles around it once it is placed
//! (0x4235ee), and each of its tiles learning which of its neighbours
//! belong to it (0x4234ab, 0x4234d8).

use super::{Cursor, stands_on_lot};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::routing::slot::step;
use crate::visits::function;
use sims_sim::World;

/// The directions 0x4235ee looks in for a group to join: the four sides.
const SIDES: [i32; 4] = [0, 2, 4, 6];

impl Interpreter<'_> {
    /// 0x4235ee, for the tile `id` of a dynamic group just put on the
    /// lot: on each of the four tiles beside it (directions 0, 2, 4 and 6
    /// from its own position, 0x42228a, on its level), each object of its
    /// own type (+0x11c) takes it into its group ([`join_group`],
    /// 0x423368). The walk goes on after a join, over every object on
    /// all four tiles. Then every tile of its group works out its
    /// neighbours again ([`Interpreter::find_neighbours`]).
    ///
    /// The objects on each tile are listed as the lot links them, with
    /// everything they hold (0x423593: 0x43f331, 0x43f379).
    pub(crate) fn join_neighbours(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let me = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let [p0, p1, level] = me.position;
        let guid = me.guid;
        for dir in SIDES {
            let tile = World::tile(beside([p0, p1, level], dir));
            let mut cursor = Cursor::on_tile(world, tile);
            while let Some(other) = cursor.current() {
                let same = world.objects.get(&other).is_some_and(|o| o.guid == guid);
                if same {
                    join_group(world, other, id).map_err(fail)?;
                }
                cursor.advance(world).map_err(fail)?;
            }
        }
        self.find_neighbours(world, id)
    }

    /// 0x4234ab: when `id` stands on the lot, each tile of its group, from
    /// the lead along the chain (+0x190, +0x18c), works out its
    /// neighbours ([`Interpreter::tell_neighbours`]). The next tile is
    /// read after each one's call, as the original reads +0x18c then.
    pub(crate) fn find_neighbours(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !stands_on_lot(world, id).map_err(fail)? {
            return Ok(());
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let mut tile = match &object.multipart {
            Some(part) if part.parent != 0 => Some(part.parent as u16),
            _ => Some(id),
        };
        let mut seen = std::collections::BTreeSet::new();
        while let Some(current) = tile {
            if !seen.insert(current) {
                return Err(fail(Trap::Invalid("a group's tiles link in a loop".into())));
            }
            self.tell_neighbours(world, current)?;
            tile = world.next_parts.get(&current).copied();
        }
        Ok(())
    }

    /// 0x4234d8: when the tile `id` stands on the lot, a mask of the eight
    /// tiles around it that hold a tile of its own group, bit `d` for
    /// direction `(facing + d) % 8` from it (C's signed remainder; facing
    /// is data word 1, +0x4c), on its level. Membership is virtual
    /// function 0x5c of the group (0x42314e): the object is one of the
    /// tiles from the lead along the chain. Then the tile runs its
    /// function 8 with stack object 0 and the arguments mask, 0, 0, 0
    /// (0x422d46 with an argument buffer of four words).
    pub(crate) fn tell_neighbours(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !stands_on_lot(world, id).map_err(fail)? {
            return Ok(());
        }
        let group = self.parts(world, id).map_err(fail)?;
        let mut mask: i16 = 0;
        for d in 0..8 {
            let object = world
                .objects
                .get(&id)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            let facing = *object
                .data
                .get(1)
                .ok_or_else(|| fail(Trap::Invalid(format!("object {id} has no data word 1"))))?;
            let dir = (i32::from(facing) + d) % 8;
            let tile = World::tile(beside(object.position, dir));
            let mut cursor = Cursor::on_tile(world, tile);
            while let Some(other) = cursor.current() {
                if group.contains(&other) {
                    mask |= 1 << d;
                }
                cursor.advance(world).map_err(fail)?;
            }
        }
        self.callback_with(
            world,
            id,
            function::DYNAMIC_GROUP,
            0,
            Some(&[mask, 0, 0, 0]),
        )?;
        Ok(())
    }
}

impl Interpreter<'_> {
    /// 0x42346a: the tile `id` of a dynamic group leaves its group
    /// ([`unlink_tile`], 0x4228d6). Then the group it leaves, from its
    /// lead (the tile after it, if it led), and the tile itself, if it
    /// stands on the lot, work out their neighbours again
    /// ([`Interpreter::find_neighbours`]).
    pub(crate) fn leave_group(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let parent = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?
            .multipart
            .as_ref()
            .ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "object {id} is not a tile of a group"
                )))
            })?
            .parent;
        let rest = if parent == 0 {
            world.next_parts.get(&id).copied()
        } else {
            Some(parent as u16)
        };
        unlink_tile(world, id).map_err(fail)?;
        if let Some(rest) = rest {
            self.find_neighbours(world, rest)?;
        }
        if stands_on_lot(world, id).map_err(fail)? {
            self.find_neighbours(world, id)?;
        }
        Ok(())
    }
}

/// 0x4228d6: takes the tile `id` out of its group's chain (+0x18c) and
/// makes it a group of its own (lead +0x190 and next both 0). A lead's
/// tiles, itself and those after it, first take the tile after it as
/// their lead, and that tile then leads (its lead 0). Any other tile is
/// passed over by the tile before it, the others' leads unchanged.
fn unlink_tile(world: &mut World, id: u16) -> Result<(), Trap> {
    let set_parent = |world: &mut World, tile: u16, parent: u16| -> Result<(), Trap> {
        world
            .objects
            .get_mut(&tile)
            .and_then(|o| o.multipart.as_mut())
            .ok_or_else(|| Trap::Invalid(format!("object {tile} is not a tile of a group")))?
            .parent = parent as i16;
        Ok(())
    };
    let parent = world
        .objects
        .get(&id)
        .and_then(|o| o.multipart.as_ref())
        .ok_or_else(|| Trap::Invalid(format!("object {id} is not a tile of a group")))?
        .parent;
    let mut seen = std::collections::BTreeSet::new();
    if parent == 0 {
        let next = world.next_parts.get(&id).copied();
        let mut tile = Some(id);
        while let Some(current) = tile {
            if !seen.insert(current) {
                return Err(Trap::Invalid("a group's tiles link in a loop".into()));
            }
            set_parent(world, current, next.unwrap_or(0))?;
            tile = world.next_parts.get(&current).copied();
        }
        if let Some(next) = next {
            set_parent(world, next, 0)?;
        }
    } else {
        // The walk reads the link again after passing the tile over.
        let mut before = parent as u16;
        while let Some(&after) = world.next_parts.get(&before) {
            if after == id {
                match world.next_parts.get(&id).copied() {
                    Some(next) if next == id => {
                        return Err(Trap::Invalid("a group's tiles link in a loop".into()));
                    }
                    Some(next) => world.next_parts.insert(before, next),
                    None => world.next_parts.remove(&before),
                };
                continue;
            }
            if !seen.insert(after) {
                return Err(Trap::Invalid("a group's tiles link in a loop".into()));
            }
            before = after;
        }
    }
    set_parent(world, id, 0)?;
    world.next_parts.remove(&id);
    Ok(())
}

/// The position one step in direction `dir` from `position` (0x42228a):
/// the step's first word goes to +0xf4, which is position[1].
/// 0x423368, on the group tile `other` with the tile `joining`: the
/// group `joining` belongs to becomes part of `other`'s.
///
/// 1. Each tile's lead is its +0x190 ([`Multipart::parent`]), or the tile
///    itself when that is 0. With the same lead, nothing happens.
/// 2. From `other` along the chain ([`World::next_parts`], +0x18c) to its
///    last tile, which is then followed by the lead of `joining`.
/// 3. From that lead along its own chain, each tile takes `other`'s lead
///    as its lead and that lead's facing (data word 1, +0x4c). Its offset
///    (+0x1a0, +0x1a4, [`Multipart::offset`]) becomes its position less
///    the lead's, plus the lead's offset times 16, shifted right by 4
///    (arithmetic), word by word; its place in the group (+0x194, +0x198,
///    [`Multipart::group`]) is that offset turned by minus the facing
///    (0x42369d). The third words (+0x19c, +0x1a8) are left as they are.
///
/// The original casts the object found to a group (0x635abe to 0x6c0ef8)
/// and calls this on the result; an object of the type that is not a
/// group would be a null `this`, which is refused here.
///
/// [`Multipart::parent`]: sims_formats::saved_objects::Multipart::parent
/// [`Multipart::offset`]: sims_formats::saved_objects::Multipart::offset
/// [`Multipart::group`]: sims_formats::saved_objects::Multipart::group
fn join_group(world: &mut World, other: u16, joining: u16) -> Result<(), Trap> {
    let not_group = |id: u16| Trap::Invalid(format!("object {id} is not a tile of a group"));
    let lead_of = |world: &World, id: u16| -> Result<u16, Trap> {
        let part = world
            .objects
            .get(&id)
            .and_then(|o| o.multipart.as_ref())
            .ok_or_else(|| not_group(id))?;
        Ok(if part.parent == 0 {
            id
        } else {
            part.parent as u16
        })
    };
    let lead = lead_of(world, other)?;
    let first = lead_of(world, joining)?;
    if lead == first {
        return Ok(());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut last = other;
    while let Some(&next) = world.next_parts.get(&last) {
        if !seen.insert(next) {
            return Err(Trap::Invalid("a group's tiles link in a loop".into()));
        }
        last = next;
    }
    world.next_parts.insert(last, first);
    let head = world.objects.get(&lead).ok_or_else(|| not_group(lead))?;
    let [b0, b1, _] = head.position;
    let facing = *head
        .data
        .get(1)
        .ok_or_else(|| Trap::Invalid(format!("object {lead} has no data word 1")))?;
    let base = head
        .multipart
        .as_ref()
        .ok_or_else(|| not_group(lead))?
        .offset;
    let mut tile = Some(first);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(current) = tile {
        if !seen.insert(current) {
            return Err(Trap::Invalid("a group's tiles link in a loop".into()));
        }
        let object = world
            .objects
            .get_mut(&current)
            .ok_or_else(|| not_group(current))?;
        let [p0, p1, _] = object.position;
        let x = p0.wrapping_sub(b0).wrapping_add(base[0].wrapping_mul(16)) >> 4;
        let y = p1.wrapping_sub(b1).wrapping_add(base[1].wrapping_mul(16)) >> 4;
        *object
            .data
            .get_mut(1)
            .ok_or_else(|| Trap::Invalid(format!("object {current} has no data word 1")))? = facing;
        let part = object
            .multipart
            .as_mut()
            .ok_or_else(|| not_group(current))?;
        part.parent = lead as i16;
        part.offset[0] = x;
        part.offset[1] = y;
        let (g0, g1) = match i32::from(facing).wrapping_neg() & 7 {
            0 => (x, y),
            2 => (y.wrapping_neg(), x),
            4 => (x.wrapping_neg(), y.wrapping_neg()),
            6 => (y, x.wrapping_neg()),
            _ => (0, 0),
        };
        part.group[0] = g0;
        part.group[1] = g1;
        tile = world.next_parts.get(&current).copied();
    }
    Ok(())
}

fn beside([p0, p1, level]: [i32; 3], dir: i32) -> [i32; 3] {
    let [first, second] = step(dir);
    [p0.wrapping_add(second), p1.wrapping_add(first), level]
}
