//! The player cancels one of a person's actions: a press on its icon in
//! the queue the interface shows (0x4cb126 calls 0x4ea3e3, which calls
//! the person's 0x45d5f8 with the action's number).
//!
//! **What the original does** (read from its code, 0x45d5f8):
//!
//! - **The running action** (the number is the running action's, +0xa84):
//!   1. Person data word 33, the running action's priority, becomes 0
//!      (+0x5ce), and the running action's flag 0x100 is set (+0xa89 bit
//!      0). The interface then draws the cancel overlay on its icon
//!      (0x4cb3a4, `kQueueCancelOverlay`); the person's trees see the flag
//!      through the action.
//!   2. Unless the person would now be interrupted anyway (0x45d7c0, the
//!      test Idle For Input makes), an action is made (0x41f00e) with the
//!      person as caller, target and icon, menu number 33 (0x21) and
//!      priority 50 (0x32). When the person's own interaction table has an
//!      entry with menu number 33 (0x41f0ff), the action's icon is
//!      cleared and the action queued (0x45d20b); otherwise nothing is
//!      queued.
//!   3. The press counts: true.
//! - **A queued action**: each queued action with that number is
//!   dropped, its target, if any, running function 4 with the person as
//!   stack object first; the press counts when one was dropped. That is
//!   [`Machine::release_action`], which a route that asked a person to
//!   move also ends with, and which fails loudly where a function 4
//!   changes the queue. That route's end takes the running action's
//!   branch above as well, when the person already runs the action.
//!
//! The interface plays `UI_queue_delete` for a press that counts and
//! `UI_error` for one that does not (0x4cb16e); that is the interface's,
//! not this module's.

use crate::autonomy::person::VISITOR;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use sims_sim::World;

/// Person data word 33: the running action's priority (+0x5ce).
const PRIORITY: usize = 33;
/// The running action's flag that says the player cancelled it (+0xa89
/// bit 0).
const CANCELLED: u32 = 0x100;
/// The menu number and priority of the action a cancel queues on the
/// person itself (0x45d630, 0x45d632).
const CANCEL_MENU: u32 = 33;
const CANCEL_PRIORITY: i32 = 50;

impl Interpreter<'_> {
    /// Cancels `person`'s action numbered `uid` (see the module's
    /// documentation). Returns whether the press counts: true for the
    /// running action, and for a queued action when one was taken off.
    pub fn cancel(&self, world: &mut World, person: u16, uid: i32) -> Result<bool, Fault> {
        let mut machine = self.machine(world, person, 0, false);
        let result = (|| {
            let running = machine
                .me()?
                .person
                .as_ref()
                .ok_or(Trap::native(0x1c, "object is not a person"))?
                .active
                .uid;
            if running == uid {
                cancel_running(&mut machine, person)?;
                return Ok(true);
            }
            machine.release_action(person, uid)
        })();
        result.map_err(|trap| machine.fault(trap))
    }
}

/// The running action's branch of [`Interpreter::cancel`] (0x45d617 to
/// 0x45d667), for `person`, the machine's running object. A route that
/// asked `person` to move and ends while it runs the action takes this
/// branch too ([`Machine::release_action`]).
pub(crate) fn cancel_running(machine: &mut Machine<'_, '_>, person: u16) -> Result<(), Trap> {
    let me = machine
        .me_mut()?
        .person
        .as_mut()
        .ok_or(Trap::native(0x1c, "object is not a person"))?;
    let priority = me
        .data
        .get_mut(PRIORITY)
        .ok_or_else(|| Trap::Invalid(format!("person data has no word {PRIORITY}")))?;
    *priority = 0;
    me.active.flags |= CANCELLED;
    let visitor = me.data.get(VISITOR).is_some_and(|&word| word != 0);
    if machine.wants_interrupt()? {
        return Ok(());
    }
    let Some((mut action, at_once)) = machine.menu_action(person, person, CANCEL_MENU, visitor)?
    else {
        return Ok(());
    };
    action.priority = CANCEL_PRIORITY;
    action.icon = 0;
    machine.queue_action(person, action, Some(at_once))?;
    Ok(())
}
