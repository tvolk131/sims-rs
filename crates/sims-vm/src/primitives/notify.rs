//! Notify Stack Object out of Idle (opcode 49): end another object's wait.
//!
//! Ported from the old prototype's `notify_idle`
//! (legacy/crates/sims-vm/src/idle.rs), recovered from the original's
//! 0x4521ba.
//!
//! The stack object must exist (native error 0x15). The primitive looks at
//! the instruction that object's thread is waiting on. If it is a Sleep
//! (opcode 0) or an Idle For Input (opcode 17), the waiting frame's first
//! parameter becomes 0, and for a Sleep the scheduler's countdown for the
//! object becomes 0 too, so its next turn finishes the wait.
//!
//! The waiting instruction is the top frame's. When an object notifies
//! itself from a callback, such as its Load tree, it is the frame the
//! callback was pushed on. An object that is in a callback further up the
//! chain of calls is not told apart from one that is not; that case is
//! **unverified**.

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::program::Tree;

impl Machine<'_, '_> {
    pub(crate) fn notify(&mut self) -> Result<bool, Trap> {
        let target = u16::try_from(self.frame()?.stack_object)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Notify stack object is missing"))?;
        let object = &self.world.objects[&target];
        let depth = if target == self.me && self.synchronous {
            self.base
        } else {
            object.stack.len()
        };
        let Some(index) = depth.checked_sub(1) else {
            return Ok(true);
        };
        let frame = &object.stack[index];
        let Tree::Behavior(_, behavior) = self.program.tree(self.owner(frame)?, frame.tree) else {
            return Err(Trap::MissingTree { id: frame.tree });
        };
        let opcode = behavior
            .instructions
            .get(usize::from(frame.node))
            .ok_or_else(|| Trap::Invalid("a waiting frame is outside its tree".into()))?
            .opcode
            & 0x7fff;
        if opcode == 0 || opcode == 17 {
            let object = self
                .world
                .objects
                .get_mut(&target)
                .ok_or(Trap::native(10, "missing object"))?;
            *object.stack[index]
                .parameters
                .first_mut()
                .ok_or_else(|| Trap::Invalid("a waiting frame has no parameter".into()))? = 0;
            if opcode == 0 {
                object.set_countdown(0);
            }
        }
        Ok(true)
    }
}
