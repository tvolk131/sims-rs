//! Snap (opcode 46, 0x454976): put me at a place an object names, at
//! once, without walking there.
//!
//! Operands: word 0 names the slot or local, word 1 is the mode and byte
//! 4 holds flags. The object is the stack object (native error 0x15 if it
//! names none).
//!
//! The modes (0x4549c7 on): 2 is the tile in front of the object, 1 the
//! object itself, 0 the routing slot a local names, 3 the routing slot
//! word 0 names, and 4 one of the lot's own slots; any other is native
//! error 0x24. Only mode 3 is ported:
//!
//! 1. The routing slot is the stack object's (its routing slot table,
//!    object +0x13c, which holds only the SLOT entries of kind 3, as Go
//!    To Routing Slot reads it: a record written over or the type's entry
//!    with the flags the table now has). An index outside the table is
//!    native error 0x1b.
//! 2. The direction: with slot flag 0x1000, the object's facing (data
//!    word 1) turned by the number of the lowest of the flag's bits 0 to
//!    6 that is set (0 for none); without it, my own facing. It is kept
//!    in 0 to 7.
//! 3. A slot whose snap target (+0x20) is 0 or more puts me into a slot
//!    of the object; that fails loudly. Otherwise the place is the
//!    object's position and level with the slot's offset turned to the
//!    object's facing and added (0x454b93): the position goes to world
//!    units (times 3/16, 0x454f5c), the offset's first two coordinates
//!    too (0x454ecc), the offset is turned by a matrix of the object's
//!    facing (0x455070, 0x5c39a0) and added, and the sum comes back to
//!    the lot's units (times 16/3, each coordinate rounded half away from
//!    zero, 0x454fb9). For an offset of zero the place is the object's
//!    own position, which the round trip keeps (a coordinate times 3/16
//!    is exact in a float, and the error of 16/3 as a float moves it by
//!    far less than a half for any position up to [`FAR`]). An offset
//!    that is not zero turns through a sine and a cosine and fails
//!    loudly, as does a position further out than [`FAR`].
//! 4. Without slot flag 0x800 the placement first checks the place
//!    itself (0x4550d8 to 0x4551e9), in this order:
//!    - its tile must be inside the lot's border (both tile coordinates
//!      from 1 to the lot's size minus 2), or the placement error
//!      (0x6e4acc) becomes 1 and I do not fit;
//!    - the room its tile's cell names on its level (0x435b25, the map
//!      as it is, with no look at diagonal walls) must be mine (object
//!      data word 29), or I do not fit;
//!    - if my own tile is inside the border and the place is not where I
//!      stand, the direction from me to it (0x4520e9) must not be closed
//!      off from my tile (0x4359ea, bit 0 turned to that direction), nor
//!      the opposite direction from the place's tile on my level, or I do
//!      not fit.
//!
//!    Not fitting fails loudly (below).
//! 5. I am put there if I fit (the place test, virtual function 0x54,
//!    `Interpreter::fits`), in this order:
//!    - the lot is told (virtual function 0x58, `Interpreter::put`);
//!    - my facing (data word 1) becomes the direction;
//!    - if I am a person, my animator's position is unset (its virtual
//!      function 0x20, 0x589361);
//!    - object data word 52 (+0xb2) becomes 0;
//!    - if I am a person, Snap zeroes the animator's offset from me (+0x8c
//!      to +0x94, virtual function 0x2c, 0x58933b) and the animator places
//!      itself on my tile again (virtual function 0x60, `animator::place`).
//!      A person with no animator entry has the one the load leaves, which
//!      the write makes its entry.
//!
//!    The branch is true. An object that is not a person snaps the same
//!    way, without the animator.
//!
//! When I do not fit, the original sets word 52, and word 54 (+0xb6),
//! from the placement error and the object collided with (0x454e09 to
//! 0x454e9b), and the branch is false; that fails loudly.
//!
//! Flag bit 0 makes the animator's offset my distance from the place
//! (0x454c7a), which the animator's later placing adds to my position;
//! that fails loudly, as the offset is not kept. Flag bit 1 only matters
//! when I do not fit, and bit 2 only for a place inside an object.
//!
//! Ported from the original's code (above). The comparisons that cover
//! it are whole ticks of the Goth house.

use crate::animator::place;
use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::placement::{Place, closed_directions, on_lot};
use crate::routing::slot::{direction, route_slot};
use sims_sim::animator::{Animator, LOADED, UNSET};

/// The mode that names a routing slot of the stack object by word 0.
const SLOT: i16 = 3;
/// Slot flags: the direction follows the object's facing (0x454aeb),
/// and the placement skips its own checks (0x454b83).
const TURNED: i32 = 0x1000;
const SKIP_CHECKS: i32 = 0x800;
/// Object data words: the facing, and the word Snap clears (+0xb2).
const FACING: usize = 1;
const WORD_52: usize = 52;
/// Object data word 29: the room the object is in (+0x84).
const ROOM: usize = 29;
/// How far out a coordinate may be for the round trip through world
/// units to keep it.
const FAR: i32 = 0x8000;

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("Snap {what} ({at}), which is not ported"))
}

impl Machine<'_, '_> {
    pub(crate) fn snap(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let me = self.me;
        let target = u16::try_from(self.frame()?.stack_object)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Snap has no stack object"))?;
        let mode = word(operands, 2);
        match mode {
            0..=2 | 4 => {
                return Err(unsupported("0x4549c7", &format!("in mode {mode}")));
            }
            SLOT => {}
            _ => return Err(Trap::native(0x24, "Snap has an unknown mode")),
        }
        let guid = self.world.objects[&target].guid;
        let definition = self.definition(guid)?;
        let (entry, type_slot) = usize::try_from(word(operands, 0))
            .ok()
            .and_then(|i| {
                self.program
                    .routing_slot(definition.resource, definition.chunk, i)
            })
            .ok_or(Trap::native(0x1b, "Snap names no slot"))?;
        let key = (target, word(operands, 0) as u16);
        let slot = match self.world.routing.slots.get(&key) {
            Some(record) => *record,
            None => route_slot(
                type_slot,
                entry,
                self.world.routing.slot_flags.get(&key).copied(),
            ),
        };
        let facing = |machine: &Self, id: u16| -> Result<i32, Trap> {
            machine.world.objects[&id]
                .data
                .get(FACING)
                .copied()
                .map(i32::from)
                .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word {FACING}")))
        };
        let direction = if slot.flags & TURNED != 0 {
            let turn = (0..7).find(|bit| slot.flags & (1 << bit) != 0).unwrap_or(0);
            facing(self, target)?.wrapping_add(turn)
        } else {
            facing(self, me)?
        };
        let direction = direction.rem_euclid(8);
        if slot.snap_target >= 0 {
            return Err(unsupported("0x454b19", "into a slot of the object"));
        }
        if slot.offset[0] & 0x7fff_ffff != 0 || slot.offset[1] & 0x7fff_ffff != 0 {
            return Err(unsupported(
                "0x455070",
                "to a slot whose offset turns with the object's facing",
            ));
        }
        let position = self.world.objects[&target].position;
        if position[..2].iter().any(|n| n.unsigned_abs() > FAR as u32) {
            return Err(unsupported("0x454fb9", "from a position this far out"));
        }
        if operands[4] & 1 != 0 {
            return Err(unsupported(
                "0x454c7a",
                "keeping my distance from the place in the animator",
            ));
        }
        let person = self.me()?.person.is_some();
        let to = [position[0], position[1]];
        let level = position[2];
        if slot.flags & SKIP_CHECKS == 0 && !self.may_stand(to, level)? {
            return Err(unsupported(
                "0x4550f9",
                "to a place the lot's border, the room or a wall refuses",
            ));
        }
        let interpreter = self.interpreter();
        if !interpreter
            .fits(self.world, me, to, level)
            .map_err(|f| f.nested(me))?
        {
            return Err(unsupported("0x454e09", "to a place where I do not fit"));
        }
        let place_at = Place {
            on_lot: on_lot(to),
            position: to,
            container: None,
            slot: -1,
            level,
        };
        interpreter
            .put(self.world, me, place_at, true)
            .map_err(|f| f.nested(me))?;
        self.set_datum(FACING, direction as i16)?;
        if person {
            self.animator().position = [UNSET; 3];
        }
        self.set_datum(WORD_52, 0)?;
        if person {
            let own = self.me()?.position;
            place(self.animator(), own, direction as i16);
        }
        Ok(true)
    }

    /// The placement's own checks of the place `to` on `level` (0x4550d8
    /// to 0x4551e9), which slot flag 0x800 skips; see step 4.
    fn may_stand(&self, to: [i32; 2], level: i32) -> Result<bool, Trap> {
        let world = &*self.world;
        let tile = |p: [i32; 2]| [(p[0] >> 4) as i8, (p[1] >> 4) as i8];
        let size = i32::from(world.size);
        let inside = |t: [i8; 2]| t.iter().all(|&n| n >= 1 && i32::from(n) < size - 1);
        let [a, b] = tile(to);
        if !inside([a, b]) {
            return Ok(false);
        }
        let me = self.me()?;
        let room = *me
            .data
            .get(ROOM)
            .ok_or_else(|| Trap::Invalid(format!("object {} has no data word {ROOM}", self.me)))?;
        let map = usize::try_from(level - 1)
            .ok()
            .and_then(|l| world.rooms.map.get(l))
            .filter(|map| map.len() == 64 * 64)
            .ok_or_else(|| {
                Trap::Invalid(format!("Snap to level {level}, which has no room map"))
            })?;
        if map[a as usize * 64 + b as usize] != room {
            return Ok(false);
        }
        let own = me.position;
        let from = [own[0], own[1]];
        let mine = tile(from);
        if !inside(mine) || from == to {
            return Ok(true);
        }
        // The direction works on the original's order of coordinates.
        let dir = direction([from[1], from[0]], [to[1], to[0]]);
        let own_level = own[2] as i8;
        if closed_directions(world, [mine[0], mine[1], own_level], dir)? & 1 != 0 {
            return Ok(false);
        }
        Ok(closed_directions(world, [a, b, own_level], (dir + 4) % 8)? & 1 == 0)
    }

    /// My animator. A person with no entry has the one the load leaves
    /// ([`LOADED`]), which is written here as the entry.
    fn animator(&mut self) -> &mut Animator {
        self.world.animators.entry(self.me).or_insert(LOADED)
    }
}
