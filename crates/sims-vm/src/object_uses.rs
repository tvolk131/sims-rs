//! The objects a person is using, and what ends its running action.
//!
//! A person keeps a list of the objects it uses (person +0xb00; the save's
//! object uses). Each entry names the object, the number of frames the
//! person's thread had once the frame that started the use was pushed,
//! and a flag. Each object counts the people using it in data word 62.
//!
//! - When a person runs a tree of another object (Run Tree By Name in
//!   mode 2, which calls the person's virtual function 0x34, 0x45e59b),
//!   the frame is pushed, an entry with the new frame count is added and
//!   the object's count goes up. See [`Machine::count_use`].
//! - After every frame a person's thread pops (0x4752f6, then the
//!   person's virtual function 8, 0x45e5fd), the last entry is looked at.
//!   If its frame is gone, the entry is removed and the object's count
//!   goes down. If that empties the list, the person's running action
//!   ends (0x45e6cd). See [`Machine::release_use`].
//!
//! Ported from the old prototype's `push_target_tree_args` (the count),
//! `pop_person_reset_frame` and `clear_person_action`
//! (legacy/crates/sims-vm/src/person.rs, legacy/crates/sims-vm/src/action.rs),
//! and checked against the code at the addresses given.
//!
//! Compared with the original: only the start of a use. In the Goth
//! house's tick 14 person 259 starts using object 51; its list and object
//! 51's count then equal the capture's. No use ends in the ticks that
//! run, so the end of a use and of an action are **unverified**: they are
//! checked only by the synthetic tests against our reading of the code.

use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_formats::saved_objects::{Interaction, ObjectUse};

/// Data word 62: how many people are using the object.
const USERS: usize = 62;

/// Person data words that the end of a running action clears or sets.
mod person {
    /// Takes the new action's priority (0x45e745).
    pub const PRIORITY: usize = 33;
    /// Cleared (0x45e77d).
    pub const CLEARED: [usize; 2] = [72, 71];
    /// If set, cleared, and the person's footprint is worked out again
    /// (0x45e756, 0x433d16).
    pub const WIDE: usize = 73;
}

/// The running action a person has when it has none (0x41ed43): no
/// target, menu entry -1, everything else zero.
fn no_action() -> Interaction {
    Interaction {
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: -1,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    }
}

impl Machine<'_, '_> {
    /// The running person starts using object `target`, whose tree it has
    /// just pushed (0x45e5bd to 0x45e5e9). The entry's flag is the
    /// argument Run Tree By Name passes as 0; with it set the original
    /// would also note the object at person +0xa54.
    pub(crate) fn count_use(&mut self, target: u16) -> Result<(), Trap> {
        let depth = self.me()?.stack.len() as i32;
        let object = self
            .world
            .objects
            .get_mut(&target)
            .ok_or(Trap::native(0x15, "missing object"))?;
        let users = object
            .data
            .get_mut(USERS)
            .ok_or_else(|| Trap::Invalid("an object without data word 62".into()))?;
        *users = users.wrapping_add(1);
        let person = self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        person.object_uses.push(ObjectUse {
            target: target as i16,
            stack_depth: depth,
            unknown: 0,
        });
        Ok(())
    }

    /// After the running person's thread has popped a frame (0x45e5fd).
    /// With `n` frames left and the last entry made at depth `d`:
    ///
    /// - if `n` is `d - 1`, the entry's object's count goes down, and the
    ///   entry is removed (with the original, after it notes at person
    ///   +0xa54 the object of each entry whose flag is set; an entry with
    ///   the flag fails, as nothing ported makes one);
    /// - if `n` is less than that, the same, without the noting;
    /// - otherwise nothing changes.
    ///
    /// If the list was not empty and now is, the running action ends
    /// ([`Machine::end_action`]). `last_result` is the result the thread
    /// had before this return: the original writes the return's result
    /// only after this (0x474d8d to 0x474daa).
    pub(crate) fn release_use(&mut self, last_result: bool) -> Result<(), Trap> {
        let frames = self.me()?.stack.len() as i32;
        let Some(person) = self.me()?.person.as_ref() else {
            return Ok(());
        };
        let Some(last) = person.object_uses.last() else {
            return Ok(());
        };
        let (target, depth) = (last.target, last.stack_depth);
        let release = frames == depth.wrapping_sub(1) || frames < depth;
        if frames == depth.wrapping_sub(1) && person.object_uses.iter().any(|u| u.unknown != 0) {
            return Err(Trap::Unsupported(
                "an object use with its flag set ends; the original then notes the object at \
                 person +0xa54, which is not modeled"
                    .into(),
            ));
        }
        if release {
            let object = u16::try_from(target)
                .ok()
                .and_then(|id| self.world.objects.get_mut(&id))
                .ok_or_else(|| {
                    Trap::Invalid(format!("the object {target} a person uses is gone"))
                })?;
            let users = object
                .data
                .get_mut(USERS)
                .ok_or_else(|| Trap::Invalid("an object without data word 62".into()))?;
            *users = users.wrapping_sub(1);
            let person = self
                .me_mut()?
                .person
                .as_mut()
                .ok_or(Trap::native(0x1c, "object is not a person"))?;
            person.object_uses.pop();
        }
        let emptied = self
            .me()?
            .person
            .as_ref()
            .is_some_and(|p| p.object_uses.is_empty());
        if emptied {
            self.end_action(last_result)?;
        }
        Ok(())
    }

    /// The running person's action ends: it becomes the empty action
    /// (0x45e6cd with the action 0x41ed43 makes).
    ///
    /// 1. If the old action has a target, it becomes the last action, with
    ///    flag 0x20 set to `last_result` (person +0x20).
    /// 2. The empty action becomes the running one. It has no target, so
    ///    what the original does next for an action with one (clear
    ///    person words 38 and 39, and 0x41f158) is skipped.
    /// 3. Person word 33 takes the new action's priority, 0.
    /// 4. The person's motive changes are all removed (0x46085a).
    /// 5. If person word 73 is set, it is cleared and the person's
    ///    footprint worked out again (0x433d16); that is not ported and
    ///    fails.
    /// 6. Person words 72 and 71 are cleared.
    pub(crate) fn end_action(&mut self, last_result: bool) -> Result<(), Trap> {
        let person = self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        if person.data.get(person::WIDE).is_none_or(|&w| w != 0) {
            return Err(Trap::Unsupported(
                "a person's action ends with person word 73 set, after which the original \
                 works out its footprint again (0x433d16); not ported"
                    .into(),
            ));
        }
        if person.active.target != 0 {
            let mut last = person.active.clone();
            last.flags = (last.flags & !0x20) | (u32::from(last_result) << 5);
            person.last = last;
        }
        person.active = no_action();
        person.data[person::PRIORITY] = person.active.priority as i16;
        person.motive_deltas.clear();
        for word in person::CLEARED {
            person.data[word] = 0;
        }
        Ok(())
    }
}
