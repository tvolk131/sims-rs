//! Test Sim Interacting With (opcode 37, 0x45d87f): whether a person is
//! using the stack object.
//!
//! The person's own dispatcher (0x45cbb3) runs this opcode. The person is
//! the running one, unless byte 0 is 1: then it is the object whose ID the
//! operand at byte 1 (scope) and word 2 (index) holds, which must be a
//! person (native error 0x1c otherwise, also for no object). The result is
//! whether the stack object is in that person's list of objects it uses
//! (person +0xb00; see `object_uses.rs`).
//!
//! Ported from the original's code. In the Goth house the first call is a
//! check tree Find Best Interaction runs, where the list is empty.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

impl Machine<'_, '_> {
    pub(crate) fn sim_interacting_with(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        if self.me()?.person.is_none() {
            return Err(Trap::Unsupported(
                "Test Sim Interacting With on an object that is not a person: the general \
                 primitive table handles it, which is not ported"
                    .into(),
            ));
        }
        let stack_object = self.frame()?.stack_object;
        let person = if operands[0] == 1 {
            let id = self.read(operands[1], word(operands, 2))?;
            u16::try_from(id)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
                .and_then(|object| object.person.as_ref())
                .ok_or(Trap::native(0x1c, "object is not a person"))?
        } else {
            self.me()?
                .person
                .as_ref()
                .ok_or(Trap::native(0x1c, "object is not a person"))?
        };
        Ok(person
            .object_uses
            .iter()
            .any(|entry| entry.target == stack_object))
    }
}
