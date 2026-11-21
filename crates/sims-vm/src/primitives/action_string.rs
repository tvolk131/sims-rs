//! Add/Change Action String (opcode 50, 0x44cc07): name the running
//! interaction, or add an entry to the menu being built.
//!
//! The original loads string `byte 4` (counted from 1) of the string table
//! `word 0` in the running code's file (the frame's file record, frame
//! +0xc, at +8: 0x416920; 0x4704fa) and formats it (0x44bf1a) with four
//! argument words, all 0 to begin with. Then:
//!
//! - while the pie menu's listing checks an action (0x45876b: the three
//!   pointers at 0x6e5130, 0x6e50fc and 0x6e50f8 are set), the first
//!   argument, if it is still 0, becomes the frame's stack object (frame
//!   +4), and the string becomes a menu entry (0x458789): a copy of the
//!   action being checked (0x41ed87, from 0x6e50f8), as its check has left
//!   it so far, with flags 4 and 8 set and 0x10 clear, the string as its
//!   name (+0x28) and the four arguments as its own (+0x14). It is added
//!   to the menu being built (0x458b6b on 0x6e50fc). See
//!   [`crate::orders`].
//! - otherwise, if the byte 0x6c4d6c is set and the object is a person,
//!   the string names the person's running interaction (0x41f1ae on
//!   person +0xa48): the name goes to the action's +0x28 and its flag
//!   0x80 (+0x40) is set. The byte is 1 in the memory image and only the
//!   synchronous run (0x4749b5) changes it: it clears it for the run and
//!   puts it back after (0x474a37, 0x474ad0). So it is set in a thread's
//!   run and clear in any tree run at once.
//!
//! It always takes the true branch.
//!
//! **The formatting.** Calls the shared string formatter (`0x44bf1a`,
//! [`Machine::format_text`] in [`crate::primitives::dialog`]) in every run
//! before `0x44cc9a` asks where the string goes, so any unported token,
//! invalid local, or non-person career fault fails before state changes.
//! Passes that substitute into the label (`$Local`, `$Me`, `$Object`,
//! `$Neighbor`, and `$TimeLocal`) produce the formatted menu entry text;
//! literal `$` signs such as currency prefixes (`$$Local:1` -> `$50`) are
//! left intact. When adding a pie menu entry, `$Neighbor` writes `frame.stack_object` into
//! `args[0]` (`0x44c6c7`), and `0x44cca3` sets `args[0] = frame.stack_object`
//! whenever it is still 0; `$Family` (which writes `args[1]` at `0x44c0e3`)
//! fails loudly in `format_text`.
//!
//! The string table is loaded with `0x47028b` (argument 3 zero,
//! [`Machine::string_table`]), the same loader `dialog.rs` uses.
//!
//! In an entry's check tree run outside a listing (one Find Best
//! Interaction runs, or 0x458319's check of a queued or picked action),
//! and in an action's tree run at once (0x45d2d3 calls 0x474afb, which
//! runs it through 0x4749b5), no menu is being built (the three pointers
//! were 0 in a memory image of the original at Find Best Interaction's
//! first call), and the synchronous run (0x4749b5) clears 0x6c4d6c, so
//! the formatted string is dropped. A tree run at once from inside a
//! listing's check (Push Interaction on an entry that runs at once) still
//! has the listing's menu, which is asked about first, so there the
//! string adds a menu entry as in the check itself.
//!
//! In a thread's run the person's running action ([`sims_sim::Object`]'s
//! `person.active`) takes flag 0x80. Its name is not kept, as no action's
//! name is here (the save holds none either): the formatted string goes
//! only to the name. Another object's thread does nothing more than the
//! lookup.
//!
//! Any other tree run at once fails loudly: the original drops the string
//! there too, but which of the port's runs at once are the original's
//! synchronous run has not been checked one by one. A string the file
//! lacks faults as [`Trap::Invalid`]: `0x4704fa` returns null, which
//! `0x44cc07` passes to `0x5b882d` (`strlen`), whereas a present empty
//! string is valid. The old prototype's `change_action_string`
//! (legacy/crates/sims-vm/src/person.rs) reads the code the same way.

use crate::autonomy::checked;
use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::orders::MenuEntry;

/// The action flag 0x41f1ae sets when it names the action (+0x40,
/// 0x41f1bd), and that working the name out clears (0x41f167, at
/// 0x41f1a4).
pub(crate) const NAMED: u32 = 0x80;

impl Machine<'_, '_> {
    pub(crate) fn action_string(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        // The string is looked up (0x47028b, 0x4704fa) and formatted
        // (0x44bf1a) before anything looks at where it goes (0x44cc9a), so
        // a pass that fails, fails in every run.
        let raw = self.action_name(operands)?;
        let exact = self.menu.is_some();
        let formatted = self.format_text(raw, "Add/Change Action String's", exact)?;
        if let Some(menu) = self.menu {
            let name = formatted.text;
            let frame = self.frame()?;
            // `$Neighbor` writes `stack_object` to `args[0]` (0x44c6c7),
            // and otherwise `0x44cca3` sets `args[0] = stack_object` when
            // still 0; `$Family` (which writes `args[1]`, 0x44c0e3) fails
            // loudly in `format_text`.
            let args = [frame.stack_object, 0, 0, 0];
            let mut checking = menu.borrow_mut();
            let mut action = checking.action.clone();
            action.flags = (action.flags | checked::RAN | checked::PASSED) & !checked::MARKED;
            action.args = args;
            checking.added.push(MenuEntry {
                action,
                name: Some(name),
            });
            return Ok(true);
        }
        if self.checking {
            return Ok(true);
        }
        if self.synchronous {
            return Err(Trap::Unsupported(
                "Add/Change Action String in a tree run at once other than an entry's check \
                 tree or an action run at once; the synchronous run clears 0x6c4d6c (0x474a37), but which of these \
                 runs are the original's 0x4749b5 is not checked"
                    .into(),
            ));
        }
        // A thread's run, with 0x6c4d6c set: the string names the
        // person's running interaction, which sets its flag 0x80.
        if let Some(person) = self.me_mut()?.person.as_mut() {
            person.active.flags |= NAMED;
        }
        Ok(true)
    }

    /// String `byte 4` (counted from 1) of the string table `word 0` in
    /// the running code's file (0x47028b, 0x4704fa), unformatted.
    fn action_name(&self, operands: &[u8; 8]) -> Result<String, Trap> {
        let table = u16::from_le_bytes([operands[0], operands[1]]);
        let index = operands[4];
        let strings = self.string_table(table, "Add/Change Action String's")?;
        strings
            .and_then(|strings| strings.get(usize::from(index).checked_sub(1)?))
            .cloned()
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "Add/Change Action String names string {index} of table {table}, which the \
                     running code's file lacks (0x4704fa returns null, passed to strlen at \
                     0x5b882d)"
                ))
            })
    }
}
