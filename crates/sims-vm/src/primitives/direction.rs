//! Get Direction To (opcode 12, 0x452003): the direction from an object
//! to the stack object, into a word an operand names.
//!
//! Operands: the destination's index (word 0) and scope (word 2), which
//! are resolved for writing first, as Random Number resolves its
//! destination (0x453141, which also reads the old value); flags (byte
//! 4); and, with flag bit 0, the scope (byte 5) and index (word 6) that
//! name the object to look from. Without that bit the object is the one
//! in my object data word 11 (scope 3, index 11), which is my own ID.
//!
//! That object missing is native error 0x17, and the stack object
//! missing is 0x15. The branch is always true.
//!
//! The direction is the one route slots use (0x4520e9,
//! [`crate::routing::slot::direction`]), from the first object's
//! position (+0xf4, +0xf8) to the stack object's, in eighths of a turn.
//! As there, +0xf4 is the second coordinate of
//! [`sims_sim::Object::position`] and +0xf8 the first: the native order
//! puts Y first.
//! The original stores it, adds 8 and keeps the remainder of a signed
//! division by 8, which leaves a direction of 0 to 7 as it was.
//!
//! As in Random Number, a read-only resolution of the object that still
//! yields a word pointer (global 34) takes the destination's place.
//!
//! A destination with no word behind it fails loudly: for a literal the
//! original's destination pointer stays null, and for a motive the
//! resolution writes the float's pointer into a slot this primitive
//! never sets (+0x12c), and the word is then written through null.
//!
//! Ported from the original's code (above). **Unverified** against a
//! capture of the primitive itself: the comparisons that cover it are
//! whole ticks.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use crate::memory::Cell;
use crate::routing::slot::direction;

/// My object data word 11, which holds my own ID.
const MY_ID: (u8, i16) = (3, 11);

impl Machine<'_, '_> {
    pub(crate) fn get_direction_to(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let scope = u8::try_from(word(operands, 2) as u16)
            .map_err(|_| Trap::native(9, "invalid memory scope"))?;
        // 0x45202b: binds `[esi + 0x128]` to the local scratch slot before
        // resolving the destination operand and never clears it.
        self.world.scratch_ptr_set.insert(self.me);
        let (destination, writable) = self.resolve(scope, word(operands, 0), true)?;
        self.probe(destination)?;
        let (scope, index) = if operands[4] & 1 != 0 {
            (operands[5], word(operands, 6))
        } else {
            MY_ID
        };
        let (cell, pointer) = self.resolve(scope, index, false)?;
        let from = self.value(cell)?;
        // A read-only resolution that still yields a word pointer (global
        // 34) overwrites the destination pointer, as in Random Number: the
        // direction lands in the object's cell.
        let (destination, writable) = if pointer && !matches!(cell, Cell::Motive(..)) {
            (cell, true)
        } else {
            (destination, writable)
        };
        let position = |id: i16, code: u16| {
            u16::try_from(id)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .map(|o| [o.position[1], o.position[0]])
                .ok_or(Trap::native(code, "Get Direction To object is missing"))
        };
        let from = position(from, 0x17)?;
        let to = position(self.frame()?.stack_object, 0x15)?;
        let value = (direction(from, to) as i16).wrapping_add(8) % 8;
        if !writable || matches!(destination, Cell::Motive(..)) {
            return Err(Trap::Unsupported(
                "Get Direction To into a destination with no word behind it, which the \
                 original writes through a null pointer (0x4520c9)"
                    .into(),
            ));
        }
        self.store(destination, value)?;
        Ok(true)
    }
}
