//! People let go of an object that resets or is deleted (0x444306 and
//! the person's reset 0x45e791 with the object).
//!
//! Before an object resets, and when it is deleted, the original tells
//! every person on the lot's list of people (lot +0x84, in its order,
//! the object itself skipped) to let go of it: each person's virtual
//! function 0x38 (0x45e791) with the object T. Objects that are not
//! people are not told. That function, for a person P:
//!
//! 1. Returns at once if P is already in the middle of it (object flag
//!    0x800), which skips P in the broadcasts it sets off; otherwise it
//!    sets the flag, and clears it at the end.
//! 2. Resets P fully if P is using T (a use record names it) or P's
//!    running action shows T's icon (0x45e814): the reset of a person
//!    with no object named ([`Interpreter::reset_fully`]), which unwinds
//!    P's uses, restarts P's thread and cancels P's queue. Each object P
//!    used resets too, unless T has object flag 0x1000 (where the reset
//!    with no object reads P's own flag). Then it goes on to step 3.
//! 3. Filters, in this order:
//!    - **The queue** (0x45eac7), when an action in it names T as target
//!      or icon: the front action is taken off, one after another, until
//!      the queue is empty. One that names T is dropped and, when it has a
//!      target, that target is told it was cancelled (function 4, with P
//!      as the stack object), so an action on T tells T itself. The others
//!      are kept, at most 8, and put back in order at the end. While a
//!      cancel runs, P's queue holds only the actions not yet looked at,
//!      and an action it queues is looked at in the same walk.
//!    - **The last action** (0x45ebfb): with T as its target it becomes
//!      the empty action; with T as its icon the icon becomes 0.
//!    - **The running action** (0x45ec3d): with T as its target P's
//!      running action becomes the empty action ([`Machine::set_action`]),
//!      which makes the old one the last action, naming T again; with T
//!      only as its icon (a cancel above can start such an action), the
//!      icon becomes 0 (0x45ec70).
//!    - **Routes** (0x45ec81), when T is a person: each of P's route
//!      contexts that names T as the person asked to move (+0x80) forgets
//!      it: byte +0x84 is set and +0x80 and +0x90 are cleared.
//!
//! A cancel's fault keeps the place it happened. A failure partway leaves
//! the queue no longer than its ring of 8. A broadcast while the factory
//! makes a person is refused, as for any walk of the list of people then.
//!
//! What the filter leaves naming T, the original leaves as a pointer to
//! the freed object: the last action after the running action ended,
//! route contexts aimed at an object that is not a person, frames whose
//! stack object is T. Here they keep T's ID (`docs/decisions/`).
//!
//! Ported from the original's code and the old prototype's
//! `person_reset.rs`, which reads it the same way; **unverified**, as no
//! capture holds a person letting go of an object.

use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::object_uses::no_action;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;

/// An object's function for an action that was cancelled.
const CANCELLED: usize = 4;
/// The most actions the filter keeps (its temporary queue's ring).
const KEPT: usize = 8;

impl Interpreter<'_> {
    /// Tells every person but `id`, in the order of the lot's list of
    /// people, to let go of object `id` (0x444306; see the module's
    /// documentation).
    pub(crate) fn drop_references(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let Some(busy) = self.letting_go else {
            let busy = RefCell::new(BTreeSet::new());
            let outer = Interpreter {
                letting_go: Some(&busy),
                ..*self
            };
            return outer.drop_references(world, id);
        };
        if let Some(making) = self.making_person {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(format!(
                    "people letting go of object {id} while the factory makes person {making}; \
                     when in the making the original appends a person is not read (0x444306)"
                )),
            });
        }
        let person_target = world.objects.get(&id).is_some_and(|o| o.person.is_some());
        // The original's cursor reads the list's end again at each step.
        let mut at = 0;
        while let Some(&other) = world.people.get(at) {
            at += 1;
            if other == id || busy.borrow().contains(&other) {
                continue;
            }
            busy.borrow_mut().insert(other);
            let result = self.let_go(world, other, id, person_target);
            busy.borrow_mut().remove(&other);
            result?;
        }
        Ok(())
    }

    /// Person `me` lets go of object `target` (0x45e791 with the object).
    fn let_go(
        &self,
        world: &mut World,
        me: u16,
        target: u16,
        person_target: bool,
    ) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: me,
            at: None,
            trap,
        };
        let t = target as i16;
        let names = |action: &Interaction| action.target == t || action.icon == t;
        let Some(person) = world.objects.get(&me).and_then(|o| o.person.as_ref()) else {
            return Ok(());
        };
        if person.object_uses.iter().any(|u| u.target == t) || person.active.icon == t {
            // 0x45e814: the full reset, with T's flag 0x1000 deciding
            // whether the objects P used reset. P is already marked as in
            // the middle of a reset. It falls through to the filter.
            let reset_used = world
                .objects
                .get(&target)
                .is_some_and(|o| !o.removal_skips_resets);
            self.reset_fully(world, me, &self.program.people, false, reset_used)?;
        }
        if person_of(world, me).map_err(fail)?.queue.iter().any(names) {
            self.filter_queue(world, me, target)?;
        }
        let person = person_of(world, me).map_err(fail)?;
        if person.last.target == t {
            person.last = no_action();
        } else if person.last.icon == t {
            person.last.icon = 0;
        }
        if person.active.target == t {
            let mut machine = self.machine(world, me, 0, false);
            let last_result = machine.me().map_err(fail)?.last_result;
            machine.end_action(last_result).map_err(fail)?;
        } else if person.active.icon == t {
            // A cancel above can start an action showing T's icon.
            person.active.icon = 0;
        }
        if person_target && let Some(route) = world.routing.routes.get_mut(&me) {
            for context in &mut route.contexts {
                if context.objects[1] == target {
                    context.byte_84 = true;
                    context.objects[1] = 0;
                    context.action = 0;
                }
            }
        }
        Ok(())
    }

    /// The queue's filter (0x45eac7 to 0x45ebfb).
    fn filter_queue(&self, world: &mut World, me: u16, target: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: me,
            at: None,
            trap,
        };
        let t = target as i16;
        let mut kept = Vec::new();
        let result = (|| {
            loop {
                let queue = &mut person_of(world, me).map_err(fail)?.queue;
                if queue.is_empty() {
                    return Ok(());
                }
                let front = queue.remove(0);
                if front.target != t && front.icon != t {
                    // The temporary ring takes no more than it holds.
                    if kept.len() < KEPT {
                        kept.push(front);
                    }
                } else if front.target != 0 {
                    self.callback(world, front.target as u16, CANCELLED, me as i16)?;
                }
            }
        })();
        // The kept actions go back, in front of any a failed cancel left,
        // and the queue holds no more than its ring. (Only a failure can
        // leave more; the original has none.)
        let queue = &mut person_of(world, me).map_err(fail)?.queue;
        kept.append(queue);
        kept.truncate(KEPT);
        *queue = kept;
        result
    }
}

fn person_of(world: &mut World, id: u16) -> Result<&mut sims_formats::saved_objects::Person, Trap> {
    world
        .objects
        .get_mut(&id)
        .and_then(|o| o.person.as_mut())
        .ok_or(Trap::native(0x1c, "object is not a person"))
}
