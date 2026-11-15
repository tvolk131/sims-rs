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
//! tick (`Interpreter::take_self_off`, for its animator's refresh). The original first tests a tile of a group (definition
//! +0x14 set) with 0x42344e and may then call 0x42346a on it. That is not
//! decoded, so a tile of a group fails loudly, as it does for Remove
//! Object Instance.
//!
//! Modes 3 and 4 search along a line through the object the local names,
//! following its facing (see `Machine::find_in_line`).
//!
//! Mode 2 (in front of the object) and 5 (a search from a random start,
//! which draws random numbers) fail loudly.

use crate::factory::word::GRAPHICS;
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::placement::{Place, Rules, on_lot};

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
            if self.world.objects[&target].multipart.is_some() {
                return Err(Trap::Unsupported(
                    "Find Location For mode 1 on a tile of a group, which the original tests \
                     first (0x44b52a, 0x42344e)"
                        .into(),
                ));
            }
            let me = self.me;
            let interpreter = self.interpreter();
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
        if (2..=5).contains(&mode) {
            return Err(Trap::Unsupported(format!(
                "Find Location For mode {mode} is not ported"
            )));
        }
        let mut found = interpreter
            .search(self.world, target, from, level, rules)
            .map_err(unsupported)?;
        if found.is_none() {
            let anywhere = Rules {
                same_room: false,
                ..rules
            };
            found = interpreter
                .search(self.world, target, from, level, anywhere)
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
