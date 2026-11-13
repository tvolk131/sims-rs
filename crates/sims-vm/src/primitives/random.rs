//! Random Number (opcode 8): store a draw modulo a range.
//!
//! Ported from the old prototype (legacy/crates/sims-vm/src/memory.rs:747).
//!
//! Operands: destination index (word 0), destination scope (word 2), range
//! index (word 4), range scope (word 6). A scope word above 255 is an
//! invalid scope.

use crate::fault::Trap;
use crate::interpreter::{Machine, Stream, word};
use crate::memory::Cell;
use sims_core::Ext;

impl Machine<'_, '_> {
    pub(crate) fn random(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let scope = |offset: usize| {
            u8::try_from(word(operands, offset) as u16)
                .map_err(|_| Trap::native(9, "invalid memory scope"))
        };
        let (destination, writable) = self.resolve(scope(2)?, word(operands, 0), true)?;
        // Resolution also reads the old value, so an unreadable destination
        // faults before the range is read or a number drawn.
        self.value(destination)?;
        let (range_cell, range_pointer) = self.resolve(scope(6)?, word(operands, 4), false)?;
        let range = self.value(range_cell)?;
        if range == 0 {
            return Err(Trap::native(4, "Random Number range is zero"));
        }
        // A read-only resolution that still yields a word pointer (global 34)
        // overwrites the destination pointer, so the draw lands in the range
        // cell. This is the original's own aliasing.
        let (destination, writable) = if range_pointer && !matches!(range_cell, Cell::Motive(..)) {
            (range_cell, true)
        } else {
            (destination, writable)
        };
        if !writable {
            return Err(Trap::native(
                0x13,
                "Random Number destination is not writable",
            ));
        }
        // Check that the store will succeed before consuming a draw.
        if !matches!(destination, Cell::Motive(..)) {
            let old = self.value(destination)?;
            self.store(destination, old)?;
        }
        let rng = match self.stream {
            Stream::Lot => &mut self.world.lot_rng,
            Stream::Global => &mut self.world.global_rng,
        };
        // A negative range is reinterpreted as unsigned, as the original's
        // unsigned modulo does.
        let value = rng.next_u32() % (i32::from(range) as u32);
        if let Cell::Motive(id, index) = destination {
            self.set_motive(id, index, Ext::from_i32(value as i32).to_f32())?;
        } else {
            self.store(destination, value as i16)?;
        }
        Ok(true)
    }
}
