//! Burn (opcode 9, 0x44d96f): set fire to the stack object's tile, or to
//! the tile in front of it, by placing fire objects (type 0x24C95F99).
//!
//! Operands: byte 0 is where (1: the tile in front of the stack object;
//! anything else: where it stands), and bit 0 of byte 1 lets fire take
//! objects the flag test below refuses. In the order the original runs it:
//!
//! 1. **The stack object** (frame +4, 0x40743b). One naming no object is
//!    native error 0x15. One not standing on the lot (0x432595) takes the
//!    false branch.
//! 2. **The place.** The stack object's position and level (+0x110). In
//!    front adds one tile's step in its direction (data word 1, 0x42228a).
//!    A tile below 1, or at least the lot's size less 1, takes the false
//!    branch.
//! 3. **In front, the walls** of that tile (0x4795c5). The wall bit
//!    facing back at the stack object is bit 3 turned by its direction
//!    halved (the table at 0x6f4670, filled at 0x53ba5f: 8, 1, 2, 4). If
//!    the tile has that wall, its style must be one a person can see
//!    through (3, 5, 6, 0xf or 0x17), or the false branch is taken. A
//!    diagonal wall (segments 0x30) takes the false branch.
//! 4. **The objects on the tile**, as the lot links them (0x43f331,
//!    0x43f379). Any with data word 8 flag 0x800, or that is a fire,
//!    takes the false branch. The last with data word 40 flag 0x20 is the
//!    one that catches.
//! 5. **None catches.** A fire is made (0x440b2d) and placed at the exact
//!    place (place test, virtual function 0x54, and put, 0x58, with the
//!    lot told). A refusal removes it at once (0x44123f) and takes the
//!    false branch; otherwise the true branch.
//! 6. **One catches.** The objects the fire spreads to are gathered,
//!    starting with it (the vector at 0x6e4f70). After each addition the
//!    gathering starts again from the first object, and for each object
//!    adds, in order:
//!    - with a group (definition word 10) and data word 40 flag 0x20, and
//!      no tree for function 8 (0x42344e), every tile of its group, lead
//!      first;
//!    - every object on its tile with data word 40 flag 0x20;
//!    - on each of the eight tiles around (0x6c2934, 0x6c2958), the first
//!      object whose footprint (+0x100) overlaps its tile, a person's
//!      widened by 2 on every side.
//! 7. **The gathered objects are tested.** Any with data word 8 flag
//!    0x800 takes the false branch, and so, without bit 0 of byte 1, does
//!    any with data word 8 flag 0x20 or data word 62 above 0.
//! 8. **A person among them** moves to the centre of its tile (0x44df25,
//!    [`Machine::burn_person`]): its position and level are read, it
//!    resets (virtual function 0x38 with 0, 0x45e791), it is put at the
//!    centre of the tile it stood on with the lot told (virtual function
//!    0x58, 0x433340), and its virtual function 0x74 runs (0x45e519).
//! 9. **Each gathered object** whose tile has no fire on it gets one, made
//!    and placed at the centre of its tile, as in step 5. The place test
//!    takes the object's level as it was; the put reads it again
//!    (0x44e06f), in case the test's callbacks moved the object. A fire
//!    refused is removed at once. The branch is true when any fire
//!    was placed. (When the factory gives 0 the original takes the false
//!    branch at once; the port's factory faults instead, which fails
//!    loudly.)
//!
//! A fire's place test, put, Init and first turn are the factory's and
//! the placement's ([`crate::Interpreter::create`],
//! [`crate::Interpreter::fits`], [`crate::Interpreter::put`]). A content
//! without the fire type, or a wall the original would read from memory
//! it never set (0x4795c5), fails loudly.
//!
//! **Unverified**: no capture covers a Burn. The base game's trees use it
//! in the fire itself, the stoves, the fireplace, the grill and the
//! dragon, when something catches fire.

use crate::factory::word::GROUP;
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Cursor, Place, on_lot, stands_on_lot};
use crate::routing::slot::step;
use crate::visits::function;
use sims_sim::World;
use sims_sim::load::ExpandedWall;

/// The fire's type.
const FIRE: u32 = 0x24c9_5f99;
/// Byte 0: the tile in front of the stack object.
const IN_FRONT: u8 = 1;
/// Byte 1 bit 0: the gathered objects' flag 0x20 and word 62 do not stop
/// the fire.
const ANY_STATE: u8 = 0x01;
/// Data word 1: the direction an object faces.
const FACING: usize = 1;
/// Data word 8 (+0x5a): flag 0x800 stops the fire; so, without
/// [`ANY_STATE`], does flag 0x20.
const FLAGS: usize = 8;
const STOPS: i16 = 0x800;
const STOPS_UNLESS_ANY: i16 = 0x20;
/// Data word 40 (+0x9a): flag 0x20 lets the object catch.
const FLAGS_2: usize = 40;
const CATCHES: i16 = 0x20;
/// Data word 62 (+0xc6): above 0 stops the fire without [`ANY_STATE`].
const WORD_62: usize = 62;
/// The person type in definition word 9.
const PERSON: i16 = 2;
/// The wall arrays of levels 1 and 2.
const WALLS: [u16; 2] = [2, 102];
const WALL_CELL: usize = 14;
const FULL: usize = 64;
/// The wall styles a person sees through (as at 0x476dbc).
const SEE_THROUGH: [u16; 5] = [3, 5, 6, 0xf, 0x17];
/// The diagonal wall segments.
const DIAGONALS: u32 = 0x30;
/// The eight tiles around, as the tables at 0x6c2934 and 0x6c2958 list
/// them: first and second offsets.
const AROUND: [(i8, i8); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

impl Machine<'_, '_> {
    pub(crate) fn burn(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let source = self.frame()?.stack_object;
        let target = u16::try_from(source)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Burn has no stack object"))?;
        if !stands_on_lot(self.world, target)? {
            return Ok(false);
        }
        if self.catalog.object(FIRE).is_none() {
            return Err(Trap::Unsupported(format!(
                "Burn with no fire type (0x{FIRE:08X}) in the content"
            )));
        }
        let object = self.burn_object(target)?;
        let [mut p0, mut p1, level] = object.position;
        let facing = datum(object, FACING)?;
        if operands[0] == IN_FRONT {
            // The step's first word goes to +0xf4, which is position[1].
            let [first, second] = step(i32::from(facing));
            p0 = p0.wrapping_add(second);
            p1 = p1.wrapping_add(first);
        }
        let tile = [(p0 >> 4) as i8, (p1 >> 4) as i8, level as i8];
        let size = i32::from(self.world.size);
        let inside = |t: i8| t >= 1 && i32::from(t) < size - 1;
        if !inside(tile[0]) || !inside(tile[1]) {
            return Ok(false);
        }
        if operands[0] == IN_FRONT && !self.burn_walls_allow(tile, level, facing)? {
            return Ok(false);
        }
        let mut catching = None;
        for id in walk(self.world, tile)? {
            let object = self.burn_object(id)?;
            if datum(object, FLAGS_2)? & CATCHES != 0 {
                catching = Some(id);
            }
            if datum(object, FLAGS)? & STOPS != 0 || object.guid == FIRE {
                return Ok(false);
            }
        }
        let Some(first) = catching else {
            let fire = self.make_fire()?;
            return self.put_fire(fire, [p0, p1], level, None);
        };
        let gathered = self.gather(first)?;
        for &id in &gathered {
            let object = self.burn_object(id)?;
            let flags = datum(object, FLAGS)?;
            if flags & STOPS != 0 {
                return Ok(false);
            }
            if operands[1] & ANY_STATE == 0
                && (flags & STOPS_UNLESS_ANY != 0 || datum(object, WORD_62)? > 0)
            {
                return Ok(false);
            }
        }
        for &id in &gathered {
            if self.interpreter().kind(self.world, id)? == PERSON {
                self.burn_person(id)?;
            }
        }
        let mut lit = false;
        for &id in &gathered {
            let at = World::tile(self.burn_object(id)?.position);
            let mut fire_there = false;
            for other in walk(self.world, at)? {
                if self.burn_object(other)?.guid == FIRE {
                    fire_there = true;
                    break;
                }
            }
            if fire_there {
                continue;
            }
            lit |= self.place_fire_under(id)?;
        }
        Ok(lit)
    }

    /// Step 3: whether the walls of the tile in front let the fire through.
    fn burn_walls_allow(&self, tile: [i8; 3], level: i32, facing: i16) -> Result<bool, Trap> {
        let level_index = usize::try_from(level - 1)
            .ok()
            .filter(|&l| l < 2)
            .ok_or_else(|| Trap::Unsupported(format!("Burn reading the walls of level {level}")))?;
        let array = self.world.arrays.get(&WALLS[level_index]).ok_or_else(|| {
            Trap::Unsupported(format!("the lot has no wall array {}", WALLS[level_index]))
        })?;
        if array.stride != WALL_CELL || array.data.len() != FULL * FULL * WALL_CELL {
            return Err(Trap::Invalid(format!(
                "lot array {} has the wrong shape",
                WALLS[level_index]
            )));
        }
        let mask = facing_wall(facing)?;
        let wall = ExpandedWall::get(
            &array.data,
            usize::try_from(self.world.size).unwrap_or(0),
            tile[0] as usize,
            tile[1] as usize,
        );
        let segments = u32::from(wall.segments);
        if segments & mask == mask {
            // 0x43517c with 0: the style of the wall's bit.
            let index = mask.trailing_zeros() as usize;
            let style = wall.styles[index].ok_or_else(|| {
                Trap::Unsupported(
                    "Burn reading the style of a wall whose neighbour does not own it; the \
                     original reads a style it never set (0x4795c5)"
                        .into(),
                )
            })?;
            if !SEE_THROUGH.contains(&style) {
                return Ok(false);
            }
        }
        Ok(segments & DIAGONALS == 0)
    }

    /// Step 6: the objects the fire spreads to from `first`, in the order
    /// the original gathers them.
    fn gather(&self, first: u16) -> Result<Vec<u16>, Trap> {
        let interpreter = self.interpreter();
        let mut gathered = vec![first];
        'again: loop {
            for index in 0..gathered.len() {
                let id = gathered[index];
                let object = self.burn_object(id)?;
                let catches = datum(object, FLAGS_2)? & CATCHES != 0;
                if interpreter.words(self.world, id)?[GROUP] != 0
                    && catches
                    && !interpreter.has_function(self.world, id, function::DYNAMIC_GROUP)
                {
                    let mut added = false;
                    for part in interpreter.parts(self.world, id)? {
                        if !gathered.contains(&part) {
                            gathered.push(part);
                            added = true;
                        }
                    }
                    if added {
                        continue 'again;
                    }
                }
                let tile = World::tile(object.position);
                let mut added = false;
                for other in walk(self.world, tile)? {
                    if datum(self.burn_object(other)?, FLAGS_2)? & CATCHES != 0
                        && !gathered.contains(&other)
                    {
                        gathered.push(other);
                        added = true;
                    }
                }
                if added {
                    continue 'again;
                }
                let (t0, t1) = (i32::from(tile[0]), i32::from(tile[1]));
                for (d0, d1) in AROUND {
                    let around = [tile[0].wrapping_add(d0), tile[1].wrapping_add(d1), tile[2]];
                    for other in walk(self.world, around)? {
                        // The footprint: least first and second coordinates,
                        // then greatest (+0x100 holds them the other way
                        // round).
                        let mut fp = self.burn_object(other)?.footprint;
                        if interpreter.kind(self.world, other)? == PERSON {
                            fp = [fp[0] - 2, fp[1] - 2, fp[2] + 2, fp[3] + 2];
                        }
                        let overlaps = fp[1] < (t1 + 1) * 16
                            && fp[3] > t1 * 16
                            && fp[0] < (t0 + 1) * 16
                            && fp[2] > t0 * 16;
                        if overlaps && !gathered.contains(&other) {
                            gathered.push(other);
                            continue 'again;
                        }
                    }
                }
            }
            return Ok(gathered);
        }
    }

    /// Step 9 for one gathered object: a fire at the centre of its tile,
    /// read after the fire is made. The place test takes its level as it
    /// is then; the put reads it again (0x44e050, 0x44e06f), so an object
    /// the test moved takes the fire to its new level.
    fn place_fire_under(&mut self, id: u16) -> Result<bool, Trap> {
        let fire = self.make_fire()?;
        let [p0, p1, level] = self
            .world
            .objects
            .get(&id)
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "Burn: object {id} went away while fire {fire} was made, and the original \
                     reads it after it is freed"
                ))
            })?
            .position;
        let centre = |n: i32| (n & !0xf) | 8;
        self.put_fire(fire, [centre(p0), centre(p1)], level, Some(id))
    }

    /// A person the fire spreads to moves to the centre of its tile
    /// (0x44df25). Its position and level are read first. Then it resets
    /// (its virtual function 0x38 with 0, 0x45e791,
    /// [`crate::Interpreter::reset_person`]), which fails loudly for the
    /// cases that reset does not handle. It is put at the centre of the
    /// tile it stood on, at the level it had, with the lot told (virtual
    /// function 0x58, 0x433340, called as the fire's put is: no container,
    /// notify 1). Last its virtual function 0x74 runs (0x45e519,
    /// [`crate::Interpreter::refresh_person`]): its animator's position is
    /// unset and person word 64 says whether it stands on a pool tile.
    ///
    /// The put is not place-tested first, as the original does not test
    /// it (**unverified** what the original does when the centre is
    /// taken).
    ///
    /// A reset that restarts the thread running this Burn, the person's
    /// own or one it sets off, fails loudly after the primitive
    /// ([`Machine::step`]).
    ///
    /// A fault in the reset or the put keeps where it happened
    /// ([`crate::fault::Fault::nested`]): an unported case stays
    /// unsupported, and anything else is invalid state.
    fn burn_person(&mut self, id: u16) -> Result<(), Trap> {
        let [p0, p1, level] = self.burn_object(id)?.position;
        let interpreter = self.interpreter();
        let me = self.me;
        let nested = |fault: crate::fault::Fault| {
            let context = format!("Burn moving person {id} to its tile's centre");
            match fault.nested(me) {
                Trap::Unsupported(message) => Trap::Unsupported(format!("{context}: {message}")),
                Trap::Invalid(message) => Trap::Invalid(format!("{context}: {message}")),
                Trap::Within {
                    context: inner,
                    fault,
                } => Trap::Within {
                    context: if inner.is_empty() {
                        context
                    } else {
                        format!("{context}: {inner}")
                    },
                    fault,
                },
                trap => trap,
            }
        };
        interpreter
            .reset_person(self.world, id, &interpreter.program.people)
            .map_err(nested)?;
        let centre = |n: i32| (n & !0xf) | 8;
        let position = [centre(p0), centre(p1)];
        interpreter
            .put(
                self.world,
                id,
                Place {
                    on_lot: on_lot(position),
                    position,
                    container: None,
                    slot: -1,
                    level,
                },
                true,
            )
            .map_err(nested)?;
        interpreter.refresh_person(self.world, id)
    }

    /// A fire, made by the factory (0x440b2d), its Init and first turn
    /// run. The original takes the false branch when the factory gives 0;
    /// the port's factory faults instead, which fails loudly here.
    fn make_fire(&mut self) -> Result<u16, Trap> {
        self.interpreter()
            .create(self.world, FIRE)
            .map_err(|fault: crate::fault::Fault| fault.unsupported_in("Burn making a fire"))
    }

    /// The place test, then the put with the lot told, or the fire's
    /// removal at once. With `under`, the put takes that object's level
    /// as it is after the test.
    fn put_fire(
        &mut self,
        fire: u16,
        position: [i32; 2],
        level: i32,
        under: Option<u16>,
    ) -> Result<bool, Trap> {
        let interpreter = self.interpreter();
        let unsupported =
            |fault: crate::fault::Fault| fault.unsupported_in(format!("Burn placing fire {fire}"));
        if !interpreter
            .fits(self.world, fire, position, level)
            .map_err(unsupported)?
        {
            interpreter
                .remove_now(self.world, fire)
                .map_err(unsupported)?;
            return Ok(false);
        }
        let level = match under {
            None => level,
            Some(id) => {
                self.world
                    .objects
                    .get(&id)
                    .ok_or_else(|| {
                        Trap::Unsupported(format!(
                            "Burn: object {id} went away while fire {fire} was place-tested, \
                             and the original reads its level after it is freed (0x44e06f)"
                        ))
                    })?
                    .position[2]
            }
        };
        interpreter
            .put(
                self.world,
                fire,
                Place {
                    on_lot: on_lot(position),
                    position,
                    container: None,
                    slot: -1,
                    level,
                },
                true,
            )
            .map_err(unsupported)?;
        Ok(true)
    }

    fn burn_object(&self, id: u16) -> Result<&sims_sim::Object, Trap> {
        self.world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))
    }
}

/// The wall bit on the side of the tile in front that faces back at an
/// object facing `facing` (the table at 0x6f4670, row `facing / 2`,
/// column 8): bit 3 turned a quarter for each step.
fn facing_wall(facing: i16) -> Result<u32, Trap> {
    // cdq, sub, sar: halved toward zero.
    let turns = facing / 2;
    if !(0..4).contains(&turns) {
        return Err(Trap::Unsupported(format!(
            "Burn in front of an object facing {facing}, past the wall table (0x6f4690)"
        )));
    }
    let mut bit = 8u32;
    for _ in 0..turns {
        bit <<= 1;
        if bit > 8 {
            bit = 1;
        }
    }
    Ok(bit)
}

/// The objects on a tile and what they hold, as 0x43f331 and 0x43f379
/// walk them: none for a tile outside the lot's size or off levels 1 and
/// 2. Burn changes nothing while it walks, so the walk is taken whole.
fn walk(world: &World, at: [i8; 3]) -> Result<Vec<u16>, Trap> {
    let mut cursor = Cursor::on_tile(world, at);
    let mut objects = Vec::new();
    while let Some(id) = cursor.current() {
        if objects.contains(&id) {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        objects.push(id);
        cursor.advance(world)?;
    }
    Ok(objects)
}

fn datum(object: &sims_sim::Object, index: usize) -> Result<i16, Trap> {
    object
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("object {} has no data word {index}", object.id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_facing_back_turns_with_the_direction() {
        let walls: Vec<u32> = [0, 1, 2, 3, 4, 5, 6, 7]
            .into_iter()
            .map(|f| facing_wall(f).unwrap())
            .collect();
        assert_eq!(walls, [8, 8, 1, 1, 2, 2, 4, 4]);
        assert!(facing_wall(8).is_err());
        assert!(facing_wall(-2).is_err());
        // -1 halves to 0, as cdq, sub and sar do.
        assert_eq!(facing_wall(-1).unwrap(), 8);
    }
}
