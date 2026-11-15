//! Add/Change Action String (opcode 50, 0x44cc07): name the running
//! interaction, or add an entry to the menu being built.
//!
//! The original loads string `byte 4` of the string table `word 0` from
//! the frame's file and formats it (0x44bf1a). Then:
//!
//! - while the player's menu is being built (0x45876b: the three pointers
//!   at 0x6e5130, 0x6e50fc and 0x6e50f8 are set), the string becomes a
//!   menu entry (0x458789);
//! - otherwise, if the byte 0x6c4d6c is set and the object is a person,
//!   the string names the person's running interaction (0x41f1ae on
//!   person +0xa48).
//!
//! It always takes the true branch.
//!
//! Only a check tree that Find Best Interaction runs is ported. There no
//! menu is being built (the three pointers were 0 in a memory image of
//! the original at its first call), and the synchronous run (0x4749b5)
//! clears 0x6c4d6c, so the string is dropped. Whether the formatting can
//! fail on some strings is **unverified**; it is not run. Anywhere else
//! the primitive fails loudly. The old prototype's
//! `change_action_string` (legacy/crates/sims-vm/src/person.rs) reads the
//! code the same way.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn action_string(&mut self) -> Result<bool, Trap> {
        if self.advertisements.is_none() {
            return Err(Trap::Unsupported(
                "Add/Change Action String outside autonomy's check trees, where it names the \
                 running interaction (0x41f1ae) or adds an entry to the menu being built \
                 (0x458789)"
                    .into(),
            ));
        }
        Ok(true)
    }
}
