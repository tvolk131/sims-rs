//! Make New Character (opcode 19, 0x44ae08): a new person definition
//! cloned from a template, with a neighbour record for it.
//!
//! The general table (0x45246a) sends opcode 19 to 0x44ae08. Baby's
//! "process" tree reaches it when a baby grows into a child (tree 0x1010),
//! and a debug interaction for pets.
//!
//! **Operands.** Bytes 0, 1 and 2 index the frame's locals, read in that
//! order (0x47518f): the skin tone (string local + 1 of STR# 0x2000), the
//! age in years (below 18 a child; negative copies the running object's
//! own body, 0x44b23c), and the sex (0 male, any other female). An index
//! past the locals throws native error 8 (b0 at 0x44b3b3, b2 at 0x44b39e,
//! b1 at 0x44b389, checked in that order). Byte 3 picks the template
//! (0 TemplatePerson, 1 TemplateDog, 2 TemplateCat).
//!
//! **Not ported, and failing loudly.** Everything after the operands
//! starts with 0x4260e1, which copies the template to a new character file
//! with a new GUID (timeGetTime << 16 XOR one C runtime draw, 0x4725d5),
//! registers it and makes a neighbour record for it. The port has no
//! definitions made while the game runs, so the primitive stops there.
//! The rest (the body strings, one or two game draws for the head and
//! body, the name, the record ID in the stack object, and the failure path
//! that deletes the record) waits on that.

use crate::fault::Trap;
use crate::interpreter::Machine;

impl Machine<'_, '_> {
    pub(crate) fn make_new_character(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let frame = self.frame()?;
        let local = |byte: usize| {
            frame
                .locals
                .get(usize::from(operands[byte]))
                .copied()
                .ok_or(Trap::native(8, "Make New Character local out of range"))
        };
        let _skin = local(0)?;
        let _sex = local(2)?;
        let age = local(1)?;
        let species = operands[3];
        Err(Trap::Unsupported(format!(
            "Make New Character (template {species}, age {age}) makes a character's \
             definition while the game runs (0x4260e1: the template copied with a new \
             GUID and a neighbour record), which is not ported"
        )))
    }
}
