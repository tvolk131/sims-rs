//! The two debugging primitives, both from the general table (0x45246a),
//! which a person's dispatcher (0x45cbb3) passes them on to.
//!
//! **Breakpoint** (opcode 15, 0x451e34) reads a word, operand word 1
//! naming the scope and word 0 the index, with the operand reader every
//! primitive uses (0x453141, for reading; a scope above 38 is native
//! error 9). A word that is not 0 is native error 0x3e; otherwise the
//! branch is true. So a script stops at a breakpoint whose word is set,
//! which strict mode reports and play mode answers with the object's
//! reset.
//!
//! **Show String** (opcode 21, 0x451ab7) looks up string operand word 1
//! of the string set operand word 0 names in the running tree's file
//! (0x47028b: STR#, else CTSS, through 0x46feac, neither of which fails
//! on a missing set; 0x4704fa gives none for a missing string). A string
//! that is not empty is put after the definition's name ("%s: %s") and
//! sent as signal 0xeb (0x40760d). None of the game's eight subscribers
//! has a case for 0xeb: the lot (0x41637f, after the empty handler at
//! lot +0x10), 0x43c0e4, 0x44302e, the simulator (0x46d2c2), the
//! architecture (0x47b22a), the house viewer (0x4a82db), the panels
//! (0x4c3ea5) and the interface (0x4f65bf). So nothing the simulation
//! keeps changes, and the branch is always true.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

impl Machine<'_, '_> {
    /// Breakpoint (0x451e34).
    pub(crate) fn breakpoint(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        // 0x453141 compares the scope with 0x26 unsigned: a negative
        // word, or one past 255, is no scope.
        let scope =
            u8::try_from(word(operands, 2)).map_err(|_| Trap::native(9, "invalid memory scope"))?;
        if self.read(scope, word(operands, 0))? != 0 {
            return Err(Trap::native(0x3e, "Breakpoint"));
        }
        Ok(true)
    }

    /// Show String (0x451ab7): its message is presentation.
    pub(crate) fn show_string(&mut self) -> bool {
        true
    }
}
