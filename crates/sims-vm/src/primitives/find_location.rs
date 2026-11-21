//! Find Location For (opcode 16): put the stack object on a free tile
//! near a place.
//!
//! Recovered from the original's 0x44b3c8. The old prototype ported it as
//! `service_find_location` (legacy/crates/sims-sim/src/placement.rs) and
//! `Search` (legacy/crates/sims-sim/src/lot_find_location.rs).
//!
//! Operands: the mode (byte 0), a local (byte 1) and flags (byte 2). The
//! object placed is the stack object (native error 0x15 if it names no
//! object). The search starts from `[16, 16]` (the corner of tile (1, 1),
//! 0x422022) on level 1, or, with flag bit 0, from the position and level
//! of the object the local names (native error 0x33 for a local the frame
//! does not have, 0x17 if it names no object). Flag bit 1 lets the tile
//! hold other objects; without it the tile must be empty. Flag bit 2
//! refuses tiles with ground flag 0x20.
//!
//! Mode 0, along with modes 6 and up, which the original treats the same:
//! the lot's search ([`crate::Interpreter::search`]) looks for a tile in
//! the start tile's room, then anywhere. It draws no random numbers. A
//! tile found is tested once more with the place test, the object is put
//! there with the lot told (virtual function 0x58, 0x433340), and virtual
//! function 0x74 runs. The true branch is taken; if no tile is found or
//! the second test fails nothing moves and the false branch is taken.
//!
//! Mode 1 takes the object off the lot (0x44b519 to 0x44b565): its virtual
//! function 0x50 runs, asked to tell the registry (as Remove Object
//! Instance's queue does, [`crate::Interpreter::take_off`]), and the true
//! branch is taken. A person can take only itself off, during its own
//! tick (`Interpreter::take_self_off`, for its animator's refresh). A
//! tile of a group (definition word 10, +0x14, set, and the object one of
//! a group, 0x44b52a to 0x44b545) is first asked whether its type names
//! function 8 (0x42344e). If it does, the tile leaves its group, and the
//! group it left and the tile work out their neighbours again (0x42346a:
//! 0x4228d6, 0x4234ab; `Interpreter::leave_group`), so the take-off
//! takes the tile alone. If not, nothing happens before the take-off,
//! which takes the whole group off ([`crate::Interpreter::take_off`],
//! 0x422d86).
//!
//! Modes 3 and 4 search along a line through the object the local names,
//! following its facing (see `Machine::find_in_line`).
//!
//! Mode 5 is mode 0 with the search's random start (search record byte
//! 0x17, set at 0x44b5a5; see `Machine::random_start`): each of the two
//! searches first replaces a `(1, 1)` start with a random fine point or
//! moves any other start by random tile offsets.
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
                interpreter
                    .leave_group(self.world, target)
                    .map_err(|fault| fault.nested(me))?;
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
        let unsupported = |fault: crate::fault::Fault| {
            fault.unsupported_in(format!("Find Location For on object {target}"))
        };
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
    /// `from` is the record's start point (+4 second coordinate, +8 first
    /// coordinate).
    ///
    /// 1. A start of exactly (1, 1) is replaced by two draws from the
    ///    active random stream (0x46f457), each modulo the lot's size: the
    ///    first is the first coordinate (`[ebp - 0x44]`), the second the
    ///    second (`[ebp - 0x48]`, 0x434ab3). They are points, not tiles.
    /// 2. Otherwise, two draws modulo 10, less 5, are added as tile
    ///    offsets, each the middle of its tile (`(n << 4) | 8`, 0x434c98):
    ///    the first draw is tile byte 0 (turned by 0x434c98 into point +4,
    ///    added to `[ebp - 0x44]`, the first coordinate), and the second is
    ///    tile byte 1 (turned into point +0, added to `[ebp - 0x48]`, the
    ///    second coordinate). This repeats, adding to the point already
    ///    moved, until its tile (each coordinate shifted right 4 and cut to
    ///    a byte, 0x433067) is inside the lot's bounds less a tile at each
    ///    side (0x434cb7; see [`interior`]).
    /// 3. Either way the point is turned within a square of 1024 (0x47bc1f,
    ///    its first argument `[ebp - 0x44]`, the first coordinate) by the
    ///    inverse of the view's rotation (the world's +0x84, negated modulo
    ///    4; [`crate::Interpreter::view_rotation`]).
    ///
    /// From `(1, 1)`, native 0x434ab3 uses unsigned division: size 0 faults
    /// after the first draw, and a negative size is taken as a large
    /// unsigned divisor. As a port guard, this rejects any `size <= 0`
    /// before drawing, whereas a 1- or 2-tile lot skips the walk, takes its
    /// two draws modulo `size`, and lets the search return false normally.
    /// From any other start, a lot smaller than three tiles has no tile
    /// inside its bounds, so the original's loop never ends, and a turned
    /// lot wider than 64 tiles would look its corners up past the table of
    /// turns: both fail.
    fn random_start(&mut self, from: [i32; 2]) -> Result<[i32; 2], Trap> {
        let size = i32::from(self.world.size);
        let [mut x, mut y] = from;
        if [x, y] == [1, 1] {
            if size <= 0 {
                return Err(Trap::Unsupported(format!(
                    "Find Location For mode 5 on a lot {size} tiles wide: the random start \
                     from (1, 1) divides by {size} (0x434ab3)"
                )));
            }
            x = (self.draw() % size as u32) as i32;
            y = (self.draw() % size as u32) as i32;
        } else {
            if size < 3 {
                return Err(Trap::Unsupported(format!(
                    "Find Location For mode 5 on a lot {size} tiles wide: no tile is inside its \
                     bounds, so the random start never ends (0x434951)"
                )));
            }
            if size > 64 && self.view_rotation & 3 != 0 {
                return Err(Trap::Unsupported(format!(
                    "Find Location For mode 5 on a lot {size} tiles wide, turned: its corners are \
                     past the table of turns (0x434f58)"
                )));
            }
            let [across, down] = interior(size, self.view_rotation);
            let tile = |n: i32| i32::from((n >> 4) as i8);
            loop {
                let first = (self.draw() % 10) as i32 - 5;
                let second = (self.draw() % 10) as i32 - 5;
                x = x.wrapping_add((first << 4) | 8);
                y = y.wrapping_add((second << 4) | 8);
                if across.contains(&tile(x)) && down.contains(&tile(y)) {
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
        let unsupported = |fault: crate::fault::Fault| {
            fault.unsupported_in(format!("Find Location For on object {target}"))
        };
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

/// The tiles inside the lot's bounds less one at each side, as the random
/// start tests them (0x434cb7): first along the first coordinate (tile byte 0,
/// +0x70 to +0x74), then along the second (tile byte 1, +0x78 to +0x7c).
///
/// The bounds (the world's +0x70 to +0x7c) are the box round the lot's
/// four corner tiles, each turned by the world's rotation (0x4791a6, and
/// the resize at 0x477fbb). The turn (0x434f58) looks the tile up in a
/// table (filled at 0x407704) that turns it within the whole 64-tile
/// square, not the lot: a turn of 1 takes tile (a, b), as 0x433067 cuts
/// it (the first coordinate in byte 0, the second in byte 1), to
/// (63 - b, a). So on a lot smaller than 64 tiles, a turned view moves
/// the bounds to the square's far side along one coordinate or both, and
/// the walk, which starts from the unturned point, has to go round to
/// them (the cut to a byte wraps).
fn interior(size: i32, rotation: u8) -> [std::ops::RangeInclusive<i32>; 2] {
    let near = 1..=size - 2;
    let far = 65 - size..=62;
    match rotation & 3 {
        1 => [far, near],
        2 => [far.clone(), far],
        3 => [near, far],
        _ => [near.clone(), near],
    }
}

/// The largest coordinate of the square 0x47bc1f turns points in (its
/// size, 0x400, less 1).
const SPAN: i32 = 0x3ff;

/// A point turned within the square of [`SPAN`] by the inverse of the
/// view's `rotation` (0x47bc1f with first argument `[ebp - 0x44]`, the
/// first coordinate, and `(-rotation) & 3`).
fn turn([x, y]: [i32; 2], rotation: u8) -> [i32; 2] {
    turn_in_square([x, y], (4 - i32::from(rotation & 3)) & 3)
}

/// 0x47bc1f with a square of 0x400: the point `[a, b]` (its first and
/// second arguments) turned `turns` quarter turns, which must be 0 to 3;
/// any other is no turn. One turn makes `[SPAN - b, a]`, two
/// `[SPAN - a, SPAN - b]`, three `[b, SPAN - a]`.
pub(crate) fn turn_in_square([a, b]: [i32; 2], turns: i32) -> [i32; 2] {
    match turns {
        1 => [SPAN.wrapping_sub(b), a],
        2 => [SPAN.wrapping_sub(a), SPAN.wrapping_sub(b)],
        3 => [b, SPAN.wrapping_sub(a)],
        _ => [a, b],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_view_moves_the_bounds_round_the_64_tile_square() {
        // Size 6: tiles 1 to 4 near, 59 to 62 far (0x407704's table).
        assert_eq!(interior(6, 0), [1..=4, 1..=4]);
        assert_eq!(interior(6, 1), [59..=62, 1..=4]);
        assert_eq!(interior(6, 2), [59..=62, 59..=62]);
        assert_eq!(interior(6, 3), [1..=4, 59..=62]);
        // A whole lot's bounds are the same whatever the turn.
        for rotation in 0..4 {
            assert_eq!(interior(64, rotation), [1..=62, 1..=62]);
        }
    }

    #[test]
    fn a_point_is_turned_by_the_inverse_of_the_view() {
        // 0x47bc1f with its first argument the first coordinate: a quarter
        // turn (view 3, inverse 1) gives x' = 1023 - y and y' = x.
        assert_eq!(turn([40, 100], 0), [40, 100]);
        assert_eq!(turn([40, 100], 3), [923, 40]);
        assert_eq!(turn([40, 100], 2), [983, 923]);
        assert_eq!(turn([40, 100], 1), [100, 983]);
        // Four quarter turns give the point back.
        let mut p = [40, 100];
        for _ in 0..4 {
            p = turn(p, 1);
        }
        assert_eq!(p, [40, 100]);
    }
}
