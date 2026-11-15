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
    /// from its own position, 0x42228a, on its level), an object of its
    /// own type (+0x11c) would take it into its group (0x423368, which is
    /// not ported and fails loudly). Then every tile of its group works
    /// out its neighbours again ([`Interpreter::find_neighbours`]).
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
                    return Err(fail(Trap::Unsupported(format!(
                        "object {id}, a tile of a dynamic group, placed beside object \
                         {other} of its own type (direction {dir}): joining its group \
                         (0x423368) is not ported"
                    ))));
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

/// The position one step in direction `dir` from `position` (0x42228a):
/// the step's first word goes to +0xf4, which is position[1].
fn beside([p0, p1, level]: [i32; 3], dir: i32) -> [i32; 3] {
    let [first, second] = step(dir);
    [p0.wrapping_add(second), p1.wrapping_add(first), level]
}
