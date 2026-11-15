//! Find Location For (opcode 16): put the stack object on a free tile
//! near a place.
//!
//! Recovered from the original's 0x44b3c8. The old prototype ported it as
//! `service_find_location` (legacy/crates/sims-sim/src/placement.rs) and
//! `Search` (legacy/crates/sims-sim/src/lot_find_location.rs).
//!
//! Operands: the mode (byte 0), a local (byte 1) and flags (byte 2). The
//! object placed is the stack object (native error 0x15 if it names no
//! object). The search starts from the middle of tile (1, 1) on level 1,
//! or, with flag bit 0, from the position and level of the object the
//! local names (native error 0x33 for a local the frame does not have,
//! 0x17 if it names no object). Flag bit 1 lets the tile hold other
//! objects; without it the tile must be empty. Flag bit 2 refuses tiles
//! with ground flag 0x20.
//!
//! Only mode 0 is ported, along with modes 6 and up, which the original
//! treats the same: the lot's search ([`crate::Interpreter::search`])
//! looks for a tile in the start tile's room, then anywhere. It draws no
//! random numbers. A tile found is tested once more with the place test,
//! the object is put there with the lot told (virtual function 0x58,
//! 0x433340), and virtual function 0x74 runs. The true branch is taken;
//! if no tile is found or the second test fails nothing moves and the
//! false branch is taken.
//!
//! Mode 1 takes the object off the lot (0x44b519 to 0x44b565): its virtual
//! function 0x50 runs, asked to tell the registry (as Remove Object
//! Instance's queue does, [`crate::Interpreter::take_off`]), and the true
//! branch is taken. A person can take only itself off, during its own
//! tick (`Interpreter::take_self_off`, for its animator's refresh). A
//! tile of a group (definition word 10, +0x14, set, and the object one of
//! a group, 0x44b52a to 0x44b545) is first asked whether its type names
//! function 8 (0x42344e). If it does, the tile leaves its group and the
//! group's offsets are worked out again (0x42346a: 0x4228d6, 0x4234ab),
//! which is not ported and fails loudly. If not, nothing happens before
//! the take-off, which takes the whole group off
//! ([`crate::Interpreter::take_off`], 0x422d86).
//!
//! Modes 3 and 4 search along a line through the object the local names,
//! following its facing (see `Machine::find_in_line`).
//!
//! Mode 5 is mode 0 with the search's random start (search record byte
//! 0x17, set at 0x44b5a5; see `Machine::random_start`): each of the two
//! searches first moves its start by a random number of tiles.
//!
//! Mode 2 (in front of the object) fails loudly.

use crate::factory::word::{GRAPHICS, GROUP};
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, Rules, on_lot};
use crate::visits::function;

impl Machine<'_, '_> {
    pub(crate) fn find_location(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let (mode, local, flags) = (operands[0], usize::from(operands[1]), operands[2]);
        let object = |id: i16| {
            u16::try_from(id)
                .ok()
                .filter(|id| *id > 0)
                .filter(|id| self.world.objects.contains_key(id))
        };
        let target = object(self.frame()?.stack_object).ok_or(Trap::native(
            0x15,
            "Find Location For has no object to place",
        ))?;
        let mut from = [16, 16];
        let mut level = 1;
        if flags & 1 != 0 {
            let frame = self.frame()?;
            if local >= frame.locals.len() {
                return Err(Trap::native(
                    0x33,
                    "Find Location For names a missing local",
                ));
            }
            self.check_local(local)?;
            let reference = object(self.frame()?.locals[local]).ok_or(Trap::native(
                0x17,
                "Find Location For has no object to search from",
            ))?;
            let p = self.world.objects[&reference].position;
            from = [p[0], p[1]];
            level = p[2];
        }
        if mode == 1 {
            let me = self.me;
            let interpreter = self.interpreter();
            if self.world.objects[&target].multipart.is_some()
                && interpreter.words(self.world, target)?[GROUP] != 0
                && interpreter.has_function(self.world, target, function::DYNAMIC_GROUP)
            {
                return Err(Trap::Unsupported(
                    "Find Location For mode 1 on a tile of a group whose type has the dynamic \
                     group function (8): the tile leaves its group first and the group's \
                     offsets are worked out again (0x42346a), which is not ported"
                        .into(),
                ));
            }
            if target == me && self.world.objects[&target].person.is_some() {
                interpreter
                    .take_self_off(self.world, target)
                    .map_err(|fault| fault.nested(me))?;
            } else {
                interpreter
                    .take_off(self.world, target, true)
                    .map_err(|fault| fault.nested(me))?;
            }
            return Ok(true);
        }
        let rules = Rules {
            same_room: true,
            empty: flags & 2 == 0,
            unmarked: flags & 4 != 0,
        };
        let interpreter = self.interpreter();
        let unsupported =
            |fault| Trap::Unsupported(format!("Find Location For on object {target}: {fault}"));
        if mode == 3 || mode == 4 {
            return self.find_in_line(target, mode, flags, local, from, level, rules);
        }
        if mode == 2 {
            return Err(Trap::Unsupported(format!(
                "Find Location For mode {mode} is not ported"
            )));
        }
        // Mode 5 differs from mode 0 only in the search record's byte 0x17
        // (0x44b5a5), so each search draws its own start from the same
        // `from` (0x434921).
        let random = mode == 5;
        let start = if random {
            self.random_start(from)?
        } else {
            from
        };
        let mut found = interpreter
            .search(self.world, target, start, level, rules)
            .map_err(unsupported)?;
        if found.is_none() {
            let anywhere = Rules {
                same_room: false,
                ..rules
            };
            let start = if random {
                self.random_start(from)?
            } else {
                from
            };
            found = interpreter
                .search(self.world, target, start, level, anywhere)
                .map_err(unsupported)?;
        }
        let Some(position) = found else {
            return Ok(false);
        };
        if !interpreter
            .fits(self.world, target, position, level)
            .map_err(unsupported)?
        {
            return Ok(false);
        }
        interpreter
            .put(
                self.world,
                target,
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
        // Virtual function 0x74 (0x433bb3) refreshes the object's own
        // graphics, if it has any; that is presentation, but an object
        // with its own graphics is not expected here.
        let guid = self.world.objects[&target].guid;
        if self.definition_words(guid)?[GRAPHICS] & 1 != 0 {
            return Err(Trap::Unsupported(format!(
                "Find Location For placed object {target}, which has its own graphics; \
                 refreshing them (0x433bb3) is not ported"
            )));
        }
        Ok(true)
    }

    /// The search's random start (search record byte 0x17, 0x434921 to
    /// 0x4349e5), the point the spiral then starts from, clamped as ever.
    /// `from` is the record's start point (+4, +8).
    ///
    /// 1. A start of exactly (1, 1) is replaced by two draws from the
    ///    active random stream (0x46f457), each modulo the lot's size: the
    ///    first is the second coordinate, the second the first (0x434ab3).
    ///    They are points, not tiles.
    /// 2. Otherwise, two draws modulo 10, less 5, are added as tile
    ///    offsets, each the middle of its tile (`(n << 4) | 8`, 0x434c98):
    ///    the first to the second coordinate, the second to the first.
    ///    This repeats, adding to the point already moved, until its tile
    ///    (each coordinate shifted right 4 and cut to a byte, 0x433067) is
    ///    inside the lot's bounds less a tile at each side (0x434cb7). The
    ///    bounds (the world's +0x70 to +0x7c) are the box round the lot's
    ///    four corner tiles, turned by the world's rotation (0x478fef,
    ///    0x4791a6, and the resize at 0x478026), so tiles 1 to size - 2,
    ///    whatever the rotation.
    /// 3. Either way the point is turned within a square of 1024 (0x47bc1f)
    ///    by the inverse of the view's rotation (the world's +0x84, negated
    ///    modulo 4; [`crate::Interpreter::view_rotation`]).
    ///
    /// A lot smaller than three tiles has no tile inside its bounds, so
    /// the original's loop never ends, and a lot of size 0 divides by zero:
    /// both fail.
    fn random_start(&mut self, from: [i32; 2]) -> Result<[i32; 2], Trap> {
        let size = i32::from(self.world.size);
        if size < 3 {
            return Err(Trap::Unsupported(format!(
                "Find Location For mode 5 on a lot {size} tiles wide: no tile is inside its \
                 bounds, so the random start never ends (0x434951)"
            )));
        }
        let [mut x, mut y] = from;
        if [x, y] == [1, 1] {
            y = (self.draw() % size as u32) as i32;
            x = (self.draw() % size as u32) as i32;
        } else {
            let inside = |n: i32| (1..=size - 2).contains(&i32::from((n >> 4) as i8));
            loop {
                let first = (self.draw() % 10) as i32 - 5;
                let second = (self.draw() % 10) as i32 - 5;
                y = y.wrapping_add((first << 4) | 8);
                x = x.wrapping_add((second << 4) | 8);
                if inside(y) && inside(x) {
                    break;
                }
            }
        }
        Ok(turn([x, y], self.view_rotation))
    }

    /// Modes 3 and 4 (0x44b676 to 0x44b716): a search along a line
    /// through the object the local names.
    ///
    /// 1. Without flag bit 0 there is no such object: native error 0x3d.
    /// 2. The line follows that object's facing (data word 1), or, in mode
    ///    4, its facing plus 2, remainder 8.
    /// 3. One search, in the start tile's room
    ///    ([`crate::Interpreter::search_line`]). No tile found takes the
    ///    false branch.
    /// 4. The place test once more at the tile found
    ///    ([`crate::Interpreter::fits`]); a refusal takes the false branch.
    /// 5. The object is put there with the lot told (virtual function
    ///    0x58) and the true branch is taken. Unlike mode 0, virtual
    ///    function 0x74 does not run.
    #[allow(clippy::too_many_arguments)]
    fn find_in_line(
        &mut self,
        target: u16,
        mode: u8,
        flags: u8,
        local: usize,
        from: [i32; 2],
        level: i32,
        rules: Rules,
    ) -> Result<bool, Trap> {
        if flags & 1 == 0 {
            return Err(Trap::native(
                0x3d,
                "Find Location For along a line has no object to search from",
            ));
        }
        let reference = u16::try_from(self.frame()?.locals[local])
            .map_err(|_| Trap::native(0x17, "Find Location For has no object to search from"))?;
        let facing = i32::from(
            self.world
                .objects
                .get(&reference)
                .ok_or(Trap::native(
                    0x17,
                    "Find Location For has no object to search from",
                ))?
                .data[1],
        );
        let direction = if mode == 4 { (facing + 2) % 8 } else { facing };
        let interpreter = self.interpreter();
        let unsupported =
            |fault| Trap::Unsupported(format!("Find Location For on object {target}: {fault}"));
        let Some(position) = interpreter
            .search_line(self.world, target, from, level, rules, direction)
            .map_err(unsupported)?
        else {
            return Ok(false);
        };
        if !interpreter
            .fits(self.world, target, position, level)
            .map_err(unsupported)?
        {
            return Ok(false);
        }
        interpreter
            .put(
                self.world,
                target,
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
}

/// The largest coordinate of the square 0x47bc1f turns points in (its
/// size, 0x400, less 1).
const SPAN: i32 = 0x3ff;

/// A point turned within the square of [`SPAN`] by the inverse of the
/// view's `rotation` (0x47bc1f with (-rotation) & 3).
fn turn([x, y]: [i32; 2], rotation: u8) -> [i32; 2] {
    match (4 - i32::from(rotation & 3)) & 3 {
        1 => [y, SPAN.wrapping_sub(x)],
        2 => [SPAN.wrapping_sub(x), SPAN.wrapping_sub(y)],
        3 => [SPAN.wrapping_sub(y), x],
        _ => [x, y],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_is_turned_by_the_inverse_of_the_view() {
        // 0x47bc1f with its first argument the second coordinate: a
        // quarter turn (view 3) gives y' = 1023 - x and x' = y.
        assert_eq!(turn([40, 100], 0), [40, 100]);
        assert_eq!(turn([40, 100], 3), [100, 983]);
        assert_eq!(turn([40, 100], 2), [983, 923]);
        assert_eq!(turn([40, 100], 1), [923, 40]);
        // Four quarter turns give the point back.
        let mut p = [40, 100];
        for _ in 0..4 {
            p = turn(p, 1);
        }
        assert_eq!(p, [40, 100]);
    }
}
