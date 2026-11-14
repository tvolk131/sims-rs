//! Get Distance To (opcode 11, 0x451f27): the distance from an object to
//! the stack object, into a temp.
//!
//! Operands: the temp that takes the result (word 0, 0 to 7; anything
//! else is native error 2); flags (byte 2); and, with flag bit 0, the
//! scope (byte 3) and index (word 2) that name the object to measure
//! from. Without that bit the object is the one in my object data word
//! 11 (scope 3, index 11), which is my own ID.
//!
//! Either object missing is native error 0x17. The branch is always true.
//!
//! The distance (0x434148), with positions in sixteenths of a tile and
//! levels counted 20 apart:
//!
//! - to an object whose definition has object type 7, 0.0;
//! - otherwise `sqrt((dx² + dy²) / 256 + (20 · dlevel)²)`. The sum of
//!   squares is 32-bit integer arithmetic; the rest is the FPU's, which
//!   runs in single precision here, so every operation rounds to single
//!   precision.
//!
//! The result is truncated toward zero (`__ftol`, 0x636128) and its low
//! word stored.
//!
//! Ported from the original's code (above); the old prototype has no
//! version of it. **Unverified** against a capture of the primitive
//! itself: the comparisons that cover it are whole ticks.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use sims_core::{Ext, F32};

/// My object data word 11, which holds my own ID.
const MY_ID: (u8, i16) = (3, 11);

/// The object type whose distance is always zero.
const NO_DISTANCE_TYPE: i16 = 7;

/// Definition word 9: the object type (OBJD byte 0x12).
const OBJECT_TYPE: usize = 9;

/// The float at 0x666dbc, 1/256: turns squared sixteenths into squared
/// tiles.
const SIXTEENTHS_SQUARED: f32 = 0.003_906_25;

impl Machine<'_, '_> {
    pub(crate) fn get_distance_to(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let temp = usize::try_from(word(operands, 0))
            .ok()
            .filter(|&t| t < 8)
            .ok_or(Trap::native(2, "invalid Get Distance To temp"))?;
        let (scope, index) = if operands[2] & 1 != 0 {
            (operands[3], word(operands, 4))
        } else {
            MY_ID
        };
        let from = self.read(scope, index)?;
        let object = |id: i16| {
            u16::try_from(id)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .ok_or(Trap::native(0x17, "Get Distance To object is missing"))
        };
        let from = object(from)?;
        let to = object(self.frame()?.stack_object)?;
        let distance = if self.definition_words(to.guid)?[OBJECT_TYPE] == NO_DISTANCE_TYPE {
            Ext::ZERO
        } else {
            let [ax, ay, alevel] = from.position;
            let [bx, by, blevel] = to.position;
            let (dx, dy) = (ax.wrapping_sub(bx), ay.wrapping_sub(by));
            let squares = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
            let rise = Ext::from_i32(blevel.wrapping_sub(alevel).wrapping_mul(20));
            let flat = (Ext::from_i32(squares) * F32::from_f32(SIXTEENTHS_SQUARED).ext()).single();
            (flat + (rise * rise).single()).single().sqrt().single()
        };
        let value = distance.to_i32_truncate() as i16;
        self.me_mut()?.temps[temp] = value;
        Ok(true)
    }
}
