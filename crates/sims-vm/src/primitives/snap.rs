//! Snap (opcode 46, 0x454976): put me at a place an object names, at
//! once, without walking there.
//!
//! Operands: word 0 names the slot or local, word 1 is the mode and byte
//! 4 holds flags. The object is the stack object (native error 0x15 if it
//! names none).
//!
//! The modes (0x4549c7 on): 2 is the tile in front of the object, 1 the
//! object itself, 0 the routing slot a local names, 3 the routing slot
//! word 0 names, and 4 the lot's global routing slot word 0 names; any
//! other is native error 0x24. Modes 3 and 4 are ported, and share all
//! but the table (both reach 0x454ac5):
//!
//! 1. The routing slot is the stack object's in mode 3 (its routing slot
//!    table, object +0x13c, which holds only the SLOT entries of kind 3,
//!    as Go To Routing Slot reads it: a record written over or the type's
//!    entry with the flags the table now has). In mode 4 it is the lot's
//!    (0x454ab6: the stack object's lot, +0xdc, at +0x9c), the kind 3
//!    entries of the global file's SLOT 100
//!    ([`crate::Program::global_routing_slot`]), with the flags the lot's
//!    one copy now has ([`GLOBAL_SLOTS`]), as Go To Routing Slot's source
//!    2 reads it. Either way the place is still the stack object's. An
//!    index outside the table (a negative one included) is native error
//!    0x1b.
//! 2. The direction: with slot flag 0x1000, the object's facing (data
//!    word 1) turned by the number of the lowest of the flag's bits 0 to
//!    6 that is set (0 for none); without it, my own facing. It is kept
//!    in 0 to 7.
//! 3. A slot whose snap target (+0x20) is 0 or more puts me into that
//!    slot of the object, as scripts count its container slots (0x454b19):
//!    one past the last is native error 0x20 (the object's container
//!    records, object +0x130, count its tile's chain first). The place is
//!    the object's position and level (+0x110), and the place test is the
//!    one for a slot of the object (`Interpreter::fits_in`). Otherwise
//!    the place is the object's position with the slot's offset turned
//!    to the object's facing and added (0x454b93): the position goes to
//!    world units (times 3/16, 0x454f5c), the offset's first two
//!    coordinates too (0x454ecc), the offset is turned by a matrix of
//!    the object's facing (0x455070, 0x5c39a0) and added, and the sum
//!    comes back to the lot's units (times 16/3, each coordinate rounded
//!    half away from zero, 0x454fb9); see [`place_by_offset`]. The level
//!    is the object's.
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
//!    Not fitting takes the false branch (below).
//! 5. I am put there if I fit (the place test, virtual function 0x54,
//!    `Interpreter::fits` or `Interpreter::fits_in`), in this order:
//!    - the lot is told (virtual function 0x58, `Interpreter::put`), with
//!      the object and its slot for a place inside it;
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
//! When I do not fit, word 52 and word 54 (+0xb6) are set from the
//! placement error and the object collided with (0x454e09 to 0x454e9b,
//! `not_fit`), and the branch is false; for a multi-tile object the
//! object collided with is the last that any of its tiles' tests wrote. A
//! person blocked by another person with flag bit 1 fails loudly.
//!
//! Flag bit 0 makes the animator's offset my distance from the place
//! (0x454c7a), which the animator's later placing adds to my position;
//! that fails loudly, as the offset is not kept. Flag bit 1 only matters
//! when I do not fit. Bit 2, for a place inside an object, then tests the
//! walls there once the place test has passed (0x45522c: my footprint at
//! the object, facing the direction, 0x433d16, crossed by a wall on its
//! level, 0x476dbc; `Machine::walls_in_the_way`). My facing is put back
//! reduced to 0 to 7. A wall in the way sets word 54 to 0 and goes on as
//! when I do not fit, with no placement error and no object collided
//! with: word 52 and word 54 become 0 and the branch is false.
//!
//! Ported from the original's code (above). The comparisons that cover
//! it are whole ticks of the Goth house.

use crate::animator::place;
use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::placement::{Place, Refusal, closed_directions, on_lot};
use crate::primitives::go_to_slot::GLOBAL_SLOTS;
use crate::routing::slot::{
    direction, facing_cos_sin, route_slot, step, to_lot_units, to_world_units,
};
use crate::routing::walls_cross;
use sims_core::{Ext, F32};
use sims_sim::animator::{Animator, LOADED, UNSET};

/// Why Snap found that I do not fit.
enum NotFit {
    /// Its own checks of the place refused, leaving this placement error.
    Code(u32),
    /// The place test refused, leaving the object it last wrote as
    /// collided with (0x6e4ad0), if any.
    Refused(Refusal, Option<u16>),
}

/// The SLOT entry kind of a container slot.
const CONTAINER: u16 = 0;

/// The mode that names a routing slot of the stack object by word 0.
const SLOT: i16 = 3;
/// The mode that names one of the lot's global routing slots by word 0.
const GLOBAL: i16 = 4;
/// Slot flags: the direction follows the object's facing (0x454aeb),
/// and the placement skips its own checks (0x454b83).
const TURNED: i32 = 0x1000;
const SKIP_CHECKS: i32 = 0x800;
/// Object data words: the facing, and the word Snap clears (+0xb2) or
/// sets when I do not fit.
const FACING: usize = 1;
const WORD_52: usize = 52;
/// Object data word 54 (+0xb6): the object a Snap that did not fit
/// collided with.
const WORD_54: usize = 54;
/// Object data word 29: the room the object is in (+0x84).
const ROOM: usize = 29;

/// The place of a slot that puts me into no container slot (0x454b93 to
/// 0x454c54): the object's `position` with the slot's `offset` across
/// (its first two coordinates, as float bits) turned to the object's
/// facing `dir` and added, in world units:
///
/// 1. The matrix of the facing (0x455070, the raw data word, not kept in
///    0 to 7): its rows are `[c, 0, -s, 0]`, `[0, 1, 0, 0]`, `[s, 0, c,
///    0]` and `[0, 0, 0, 1]` ([`facing_cos_sin`]).
/// 2. The position in world units (0x454f5c): `[k x, 0, k y]`, `k` being
///    3/16 ([`to_world_units`]); the height is 0 as the level passed is
///    0.
/// 3. The offset in world units (0x454ecc): `[k o1, 0, k o0]`, the
///    offset's second coordinate first; its third is not read.
/// 4. The offset times the matrix (0x5c39a0, a general matrix, so
///    0x5c4510), each coordinate divided by the fourth (1).
/// 5. The two added, coordinate by coordinate, each sum stored (0x454c0a
///    to 0x454c37), and the first and third back in the lot's units
///    ([`to_lot_units`], 0x454fb9) give the place.
///
/// Every term is kept, zeros included, in the original's order. Every
/// add, multiply and divide is rounded to single precision.
pub(crate) fn place_by_offset(position: [i32; 2], offset: [u32; 2], dir: i32) -> [i32; 2] {
    let (c, s) = facing_cos_sin(dir);
    let zero = Ext::from_i32(0);
    let one = Ext::from_i32(1);
    // 0x454f5c.
    let p = [
        to_world_units(Ext::from_i32(position[0])),
        zero,
        to_world_units(Ext::from_i32(position[1])),
    ];
    // 0x454ecc.
    let v = [
        to_world_units(F32::from_bits(offset[1]).ext()),
        zero,
        to_world_units(F32::from_bits(offset[0]).ext()),
    ];
    // 0x5c4510, with m[r][c] the matrix's row r, column c.
    let m = [
        [c, zero, -s, zero],
        [zero, one, zero, zero],
        [s, zero, c, zero],
        [zero, zero, zero, one],
    ];
    let w = ((((m[0][3] * v[0]).single() + (m[2][3] * v[2]).single()).single()
        + (m[1][3] * v[1]).single())
    .single()
        + m[3][3])
        .single();
    let reciprocal = (one / w).single();
    // The first coordinate adds the third term to the second, then the
    // first; the third coordinate adds the first term to the third, then
    // the second.
    let sum = ((m[2][0] * v[2]).single() + (m[1][0] * v[1]).single()).single();
    let sum = (sum + (v[0] * m[0][0]).single()).single();
    let out0 = ((sum + m[3][0]).single() * reciprocal).to_f32().ext();
    let sum = ((m[0][2] * v[0]).single() + (m[2][2] * v[2]).single()).single();
    let sum = (sum + (m[1][2] * v[1]).single()).single();
    let out2 = ((sum + m[3][2]).single() * reciprocal).to_f32().ext();
    let first = (out0 + p[0]).to_f32().ext();
    let third = (p[2] + out2).to_f32().ext();
    [to_lot_units(first), to_lot_units(third)]
}

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
        let owner = match mode {
            1 | 2 => None,
            0 => return Err(unsupported("0x454a4b", "in mode 0")),
            SLOT => Some(target),
            GLOBAL => Some(GLOBAL_SLOTS),
            _ => return Err(Trap::native(0x24, "Snap has an unknown mode")),
        };
        let facing = |machine: &Self, id: u16| -> Result<i32, Trap> {
            machine.world.objects[&id]
                .data
                .get(FACING)
                .copied()
                .map(i32::from)
                .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word {FACING}")))
        };
        let (container, position, direction, skip_checks) = match owner {
            None => {
                // 0x4549d1 and 0x454a1d: the object's place, my own facing,
                // and the placement's own checks.
                let [p0, p1, level] = self.world.objects[&target].position;
                let direction = facing(self, me)?;
                if mode == 2 {
                    let [first, second] = step(facing(self, target)?);
                    let position = [p0.wrapping_add(second), p1.wrapping_add(first), level];
                    (None, position, direction, false)
                } else {
                    if self.world.objects[&target].slots.len() < 2 {
                        return Err(unsupported(
                            "0x4550a0",
                            "in mode 1 into an object with no container slot 0",
                        ));
                    }
                    self.container_offset(target, 0)?;
                    (Some((target, 0)), [p0, p1, level], direction, false)
                }
            }
            Some(owner) => self.slot_place(owner, target, word(operands, 0))?,
        };
        // 0x454d7a.
        let direction = direction.rem_euclid(8);
        if operands[4] & 1 != 0 {
            return Err(unsupported(
                "0x454c7a",
                "keeping my distance from the place in the animator",
            ));
        }
        let person = self.me()?.person.is_some();
        let to = [position[0], position[1]];
        let level = position[2];
        let refusal = if !skip_checks && let Some(code) = self.may_stand(to, level)? {
            Some(NotFit::Code(code))
        } else {
            let interpreter = self.interpreter();
            let (test, collided) = match container {
                Some((target, slot)) => interpreter
                    .place_test_in(self.world, me, target, slot, to, level)
                    .map(|test| {
                        let collided = match test {
                            Err(Refusal::Collision(other)) => other,
                            _ => None,
                        };
                        (test, collided)
                    }),
                None => interpreter.place_test_collided(self.world, me, to, level),
            }
            .map_err(|f| f.nested(me))?;
            test.err().map(|refusal| NotFit::Refused(refusal, collided))
        };
        if let Some(refusal) = refusal {
            self.not_fit(refusal, operands[4] & 2 != 0)?;
            return Ok(false);
        }
        if let Some((target, _)) = container
            && operands[4] & 4 != 0
            && self.walls_in_the_way(target, direction)?
        {
            // The test passed, so the placement error and the object
            // collided with are as Snap cleared them (0x454d64).
            self.not_fit(NotFit::Code(0), operands[4] & 2 != 0)?;
            return Ok(false);
        }
        let place_at = Place {
            on_lot: on_lot(to),
            position: to,
            container: container.map(|(target, _)| target),
            slot: container.map_or(-1, |(_, slot)| slot),
            level,
        };
        self.interpreter()
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

    /// The place of modes 3 and 4 (0x454ac5 to 0x454c6d), from the routing
    /// slot `index` of `owner`'s table (the stack object `target`'s, or
    /// [`GLOBAL_SLOTS`]): the slot or the place, my direction, and whether
    /// the slot skips the placement's own checks (flag 0x800). See steps 1
    /// to 3 of the module's doc.
    #[allow(clippy::type_complexity)]
    fn slot_place(
        &self,
        owner: u16,
        target: u16,
        index: i16,
    ) -> Result<(Option<(u16, i32)>, [i32; 3], i32, bool), Trap> {
        let me = self.me;
        let guid = self.world.objects[&target].guid;
        let definition = self.definition(guid)?;
        let at = usize::try_from(index).ok();
        let (entry, type_slot) = if owner == GLOBAL_SLOTS {
            at.and_then(|i| self.program.global_routing_slot(i))
        } else {
            at.and_then(|i| {
                self.program
                    .routing_slot(definition.resource, definition.chunk, i)
            })
        }
        .ok_or(Trap::native(0x1b, "Snap names no slot"))?;
        let key = (owner, index as u16);
        let slot = match self.world.routing.slots.get(&key) {
            Some(record) => *record,
            None => route_slot(
                type_slot,
                entry,
                self.world.routing.slot_flags.get(&key).copied(),
            ),
        };
        let facing = |id: u16| -> Result<i32, Trap> {
            self.world.objects[&id]
                .data
                .get(FACING)
                .copied()
                .map(i32::from)
                .ok_or_else(|| Trap::Invalid(format!("object {id} has no data word {FACING}")))
        };
        let direction = if slot.flags & TURNED != 0 {
            let turn = (0..7).find(|bit| slot.flags & (1 << bit) != 0).unwrap_or(0);
            facing(target)?.wrapping_add(turn)
        } else {
            facing(me)?
        };
        let position = self.world.objects[&target].position;
        let skip_checks = slot.flags & SKIP_CHECKS != 0;
        if slot.snap_target >= 0 {
            // The object's container records, its tile's chain first.
            let records = self.world.objects[&target].slots.len();
            if usize::try_from(slot.snap_target).is_ok_and(|s| s + 1 >= records) {
                return Err(Trap::native(0x20, "Snap names no slot of the object"));
            }
            self.container_offset(target, slot.snap_target)?;
            return Ok((
                Some((target, slot.snap_target)),
                position,
                direction,
                skip_checks,
            ));
        }
        let [x, y] = place_by_offset(
            [position[0], position[1]],
            [slot.offset[0], slot.offset[1]],
            facing(target)?,
        );
        Ok((None, [x, y, position[2]], direction, skip_checks))
    }

    /// The container slot `slot` of `target`: its offset across, turned
    /// to the object's facing, is added to my animator's offset (0x454ceb
    /// to 0x454d5e), which is not kept, so an offset that is not zero
    /// fails loudly.
    fn container_offset(&self, target: u16, slot: i32) -> Result<(), Trap> {
        let guid = self.world.objects[&target].guid;
        let definition = self.definition(guid)?;
        let held = self
            .program
            .slots(definition.resource, definition.chunk)
            .iter()
            .filter(|entry| entry.kind == CONTAINER)
            .nth(slot as usize);
        if held.is_some_and(|entry| {
            entry.offset[..2]
                .iter()
                .any(|n| n.to_bits() & 0x7fff_ffff != 0)
        }) {
            return Err(unsupported(
                "0x454ceb",
                "into a container slot whose offset across moves my animator",
            ));
        }
        Ok(())
    }

    /// Flag bit 2's wall test for a place in a slot of `target` (0x45522c
    /// to 0x455289), after the place test has passed: my footprint at
    /// `target`'s position, facing `direction` (my facing is set for the
    /// call, 0x455263, and 0x433d16 reads it), crossed by a wall on
    /// `target`'s level ([`crate::routing::walls_cross`], 0x476dbc). My
    /// facing is then put back reduced to 0 to 7 (0x45522c, 0x45527f).
    /// When a wall crosses it, word 54 (+0xb6) becomes 0 and the answer
    /// is true: I am not put there.
    fn walls_in_the_way(&mut self, target: u16, direction: i32) -> Result<bool, Trap> {
        let me = self.me;
        let saved = self
            .me()?
            .data
            .get(FACING)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("object {me} has no data word {FACING}")))?
            .rem_euclid(8);
        let at = self
            .world
            .objects
            .get(&target)
            .ok_or(Trap::native(10, "missing object"))?
            .position;
        let ours = self
            .interpreter()
            .footprint_at(self.world, me, at, direction as i16)?;
        let crossed = walls_cross(self.world, [ours[3], ours[2], ours[1], ours[0]], at[2])?;
        self.set_datum(FACING, saved)?;
        if crossed {
            self.set_datum(WORD_54, 0)?;
        }
        Ok(crossed)
    }

    /// The placement's own checks of the place `to` on `level` (0x4550d8
    /// to 0x4551e9), which slot flag 0x800 skips; see step 4. A refusal
    /// gives the placement error it leaves: 1 for the lot's border
    /// (0x4550f9), and 0, as Snap cleared it (0x454d64), for the room and
    /// the walls.
    fn may_stand(&self, to: [i32; 2], level: i32) -> Result<Option<u32>, Trap> {
        let world = &*self.world;
        let tile = |p: [i32; 2]| [(p[0] >> 4) as i8, (p[1] >> 4) as i8];
        let size = i32::from(world.size);
        let inside = |t: [i8; 2]| t.iter().all(|&n| n >= 1 && i32::from(n) < size - 1);
        let [a, b] = tile(to);
        if !inside([a, b]) {
            return Ok(Some(1));
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
            return Ok(Some(0));
        }
        let own = me.position;
        let from = [own[0], own[1]];
        let mine = tile(from);
        if !inside(mine) || from == to {
            return Ok(None);
        }
        // The direction works on the original's order of coordinates.
        let dir = direction([from[1], from[0]], [to[1], to[0]]);
        let own_level = own[2] as i8;
        if closed_directions(world, [mine[0], mine[1], own_level], dir)? & 1 != 0
            || closed_directions(world, [a, b, own_level], (dir + 4) % 8)? & 1 != 0
        {
            return Ok(Some(0));
        }
        Ok(None)
    }

    /// What Snap does when I do not fit (0x454dd1, 0x454e09 to 0x454e9b),
    /// from the placement error and the object collided with (0x6e4acc
    /// and 0x6e4ad0, both cleared first at 0x454d64; only a collision,
    /// 0x433b42, writes an object):
    ///
    /// 1. Object data word 52 (+0xb2) becomes 0.
    /// 2. If I am a person, flag bit 1 (`blocked_by_person`) is set, and
    ///    another person is in the way (code 11), word 52 becomes 1 when
    ///    0x45bd5e says so of that person with mine at person +0xa64;
    ///    that test is not ported and fails loudly.
    /// 3. If word 52 is still 0 and the error is 1 (off the lot), it
    ///    becomes 2.
    /// 4. Word 54 (+0xb6) becomes the ID of the object collided with, or
    ///    0.
    ///
    /// The branch is then false. For a multi-tile object the code is the
    /// one its group's test keeps and the object the last collision any
    /// of its tiles wrote (0x422abb,
    /// [`crate::Interpreter`]`::place_test_collided`). Fails loudly for a
    /// refusal that leaves whatever code an earlier test left
    /// ([`Refusal::Unrecorded`]).
    fn not_fit(&mut self, refusal: NotFit, blocked_by_person: bool) -> Result<(), Trap> {
        let (code, collided) = match refusal {
            NotFit::Code(code) => (Some(code), None),
            NotFit::Refused(Refusal::OffLot, collided) => (Some(1), collided),
            NotFit::Refused(Refusal::Wall, collided) => (Some(7), collided),
            NotFit::Refused(Refusal::Collision(_), collided) => (Some(11), collided),
            // A code other than 1, 7 and 11, which Snap does not test.
            NotFit::Refused(Refusal::Other, collided) => (None, collided),
            NotFit::Refused(Refusal::Unrecorded, _) => {
                return Err(unsupported(
                    "0x454e66",
                    "to a place where I do not fit, refused with whatever placement error an \
                     earlier test left",
                ));
            }
        };
        let code = code.unwrap_or(0);
        self.set_datum(WORD_52, 0)?;
        if self.me()?.person.is_some()
            && blocked_by_person
            && code == 11
            && collided.is_some_and(|id| {
                self.world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.person.is_some())
            })
        {
            return Err(unsupported(
                "0x454e54",
                "to a place another person is in, with flag bit 1, which asks 0x45bd5e",
            ));
        }
        if code == 1 {
            self.set_datum(WORD_52, 2)?;
        }
        self.set_datum(WORD_54, collided.map_or(0, |id| id as i16))?;
        Ok(())
    }

    /// My animator. A person with no entry has the one the load leaves
    /// ([`LOADED`]), which is written here as the entry.
    fn animator(&mut self) -> &mut Animator {
        self.world.animators.entry(self.me).or_insert(LOADED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_rounds_away_from_zero_either_way() {
        // Half a unit in the lot's is 0.09375 in world units, which comes
        // back as exactly 0.5.
        let half = |n: f32| [0, n.to_bits()];
        assert_eq!(place_by_offset([0, 0], half(0.5), 0), [1, 0]);
        assert_eq!(place_by_offset([0, 0], half(-0.5), 0), [-1, 0]);
        assert_eq!(place_by_offset([0, 0], half(0.25), 0), [0, 0]);
        // With no offset a position comes back as it was.
        for n in [0, 1, -1, 17, -17, 1023, 0x7fff, -0x8000] {
            assert_eq!(place_by_offset([n, -n], [0, 0], 3), [n, -n], "{n}");
        }
    }
}
