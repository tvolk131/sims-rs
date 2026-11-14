//! Look Towards (opcode 22, 0x45a98e): a person turns its head, or its
//! body, towards something.
//!
//! The person's own dispatcher runs this opcode; anything that is not a
//! person fails loudly here.
//!
//! Operand word 0 is the mode:
//!
//! - **0**: the head follows the stack object (0x58a035 on the person's
//!   animator): person data word 41 becomes the stack object's number,
//!   word 42 whether it is not 0, words 43 and 45 become 0 and word 44
//!   becomes 1. The branch is true.
//! - **1 to 3**: the person turns on the spot, through a path of its own
//!   tile twice and a point to face (1: one tile towards the camera; 2:
//!   the stack object; 3: away from it), which the animator follows as a
//!   route (0x589c85, 0x589d47, 0x589e2d). **Not ported**; fails loudly.
//! - Any other mode is native error 0x27.
//!
//! The old prototype's `request_turn` (legacy/crates/sims-vm/src/look.rs)
//! and `prepare` (legacy/crates/sims-sim/src/native_turn.rs) port modes 1
//! to 3; this port follows the original's code above.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine, word};

/// Person data words the head's target is kept in (person +0x5de to
/// +0x5e6, data at +0x58c).
const HEAD_TARGET: usize = 41;

impl Machine<'_, '_> {
    pub(crate) fn look_towards(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Look Towards on an object that is not a person: the general primitive table \
                 handles it, which is not ported"
                    .into(),
            ));
        }
        match word(operands, 0) {
            0 => {
                let target = self.frame()?.stack_object;
                let words = [target, i16::from(target != 0), 0, 1, 0];
                let me = self.me;
                let data = &mut self
                    .me_mut()?
                    .person
                    .as_mut()
                    .ok_or(Trap::native(0x1c, "object is not a person"))?
                    .data;
                let slots = data
                    .get_mut(HEAD_TARGET..HEAD_TARGET + words.len())
                    .ok_or_else(|| Trap::Invalid("person data too short for the head".into()))?;
                slots.copy_from_slice(&words);
                for index in HEAD_TARGET..HEAD_TARGET + words.len() {
                    self.world.routing.unknown_words.remove(&(me, index));
                }
                Ok(Flow::Branch(true))
            }
            mode @ 1..=3 => Err(Trap::Unsupported(format!(
                "Look Towards in mode {mode}, a turn on the spot through the animator's \
                 route (0x45a9c1), which is not ported"
            ))),
            _ => Err(Trap::native(0x27, "invalid Look Towards mode")),
        }
    }
}
