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
//!   ends: the empty action takes its place (0x45e6cd). See
//!   [`Machine::release_use`].
//! - Gosub Found Action makes the interaction Find Best Interaction chose
//!   the running action, through the same function. See
//!   [`Machine::set_action`].
//!
//! Ported from the old prototype's `push_target_tree_args` (the count),
//! `pop_person_reset_frame` and `clear_person_action`
//! (legacy/crates/sims-vm/src/person.rs, legacy/crates/sims-vm/src/action.rs),
//! and checked against the code at the addresses given.
//!
//! Compared with the original: the start of a use, and a new running
//! action. In the Goth house's tick 14 person 259 starts using object 51,
//! and in tick 233 person 21 starts using object 132 and gets a new
//! running action with a unique number; the lists, the objects' counts,
//! the running and last actions and the unique number then equal the
//! capture's. No use ends in the ticks that run, so the end of a use and
//! of an action are **unverified**: they are checked only by the
//! synthetic tests against our reading of the code.

use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_formats::saved_objects::{Interaction, ObjectUse};

/// Data word 62: how many people are using the object.
const USERS: usize = 62;

/// Person data words that the end of a running action clears or sets.
mod person {
    /// Takes the new action's priority (0x45e745).
    pub const PRIORITY: usize = 33;
    /// Cleared when the new action has a target (0x45e71c).
    pub const STARTED: [usize; 2] = [38, 39];
    /// Cleared (0x45e77d).
    pub const CLEARED: [usize; 2] = [72, 71];
    /// If set, cleared, and the person's footprint is worked out again
    /// (0x45e756, 0x433d16).
    pub const WIDE: usize = 73;
}

/// The running action a person has when it has none (0x41ed43): no
/// target, menu entry -1, everything else zero.
pub(crate) fn no_action() -> Interaction {
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
    /// just pushed (0x45e5bd to 0x45e5e9). The entry keeps `flag` (entry
    /// +8). Run Tree By Name passes 0; Run Functional Tree passes its
    /// flag. With the flag set, the object first becomes the icon of the
    /// person's running action (person +0xa54, 0x45e5d4).
    pub(crate) fn count_use(&mut self, target: u16, flag: bool) -> Result<(), Trap> {
        let depth = self.me()?.stack.len() as i32;
        if flag {
            self.me_mut()?
                .person
                .as_mut()
                .ok_or(Trap::native(0x1c, "object is not a person"))?
                .active
                .icon = target as i16;
        }
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
            unknown: u8::from(flag),
        });
        Ok(())
    }

    /// After the running person's thread has popped a frame (0x45e5fd).
    /// With `n` frames left and the last entry made at depth `d`:
    ///
    /// - if `n` is `d - 1`, the entry's object's count goes down, the
    ///   object of each entry whose flag is set, first to last, becomes
    ///   the icon of the running action (person +0xa54, 0x45e65d to
    ///   0x45e665), so the last such entry's object stays there, and the
    ///   entry is removed;
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
        let icon = if frames == depth.wrapping_sub(1) {
            person
                .object_uses
                .iter()
                .rev()
                .find(|u| u.unknown != 0)
                .map(|u| u.target)
        } else {
            None
        };
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
            if let Some(icon) = icon {
                person.active.icon = icon;
            }
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
    /// (0x45e6cd with the action 0x41ed43 makes). See
    /// [`Machine::set_action`].
    pub(crate) fn end_action(&mut self, last_result: bool) -> Result<(), Trap> {
        self.set_action(no_action(), last_result)
    }

    /// `action` becomes the running person's running action (0x45e6cd).
    ///
    /// 1. If the old action has a target, it becomes the last action, with
    ///    flag 0x20 set to `last_result` (person +0x20).
    /// 2. The new action becomes the running one. If it has a target,
    ///    person words 38 and 39 are cleared and, if it has no unique
    ///    number yet, it takes the next one ([`World::action_serial`],
    ///    0x41f158).
    /// 3. Person word 33 takes the new action's priority.
    /// 4. The person's motive changes are all removed (0x46085a).
    /// 5. If person word 73 is set, it is cleared and the person's
    ///    footprint (+0x100) worked out again where it stands (0x45e756 to
    ///    0x45e778: 0x433d16 with +0xf4 and +0x100, the call Generic Sims
    ///    Call routine 15 makes). The footprint reads person words 68, 84
    ///    and 73, so it is worked out before anything changes, from the
    ///    words as they will be: one that is not ported leaves the person
    ///    as it was.
    /// 6. Person words 72 and 71 are cleared.
    ///
    /// [`World::action_serial`]: sims_sim::World::action_serial
    pub(crate) fn set_action(
        &mut self,
        mut action: Interaction,
        last_result: bool,
    ) -> Result<(), Trap> {
        let mut data = self
            .me()?
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .clone();
        let wide = data
            .get_mut(person::WIDE)
            .ok_or_else(|| Trap::Invalid(format!("a person has no data word {}", person::WIDE)))?;
        let footprint = if *wide != 0 {
            *wide = 0;
            Some(
                self.interpreter()
                    .standing_footprint(self.world, self.me, Some(&data))?,
            )
        } else {
            None
        };
        if action.target != 0 && action.uid == 0 {
            self.world.action_serial = self.world.action_serial.wrapping_add(1);
            action.uid = self.world.action_serial;
        }
        let object = self.me_mut()?;
        if let Some(footprint) = footprint {
            object.footprint = footprint;
        }
        let person = object
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        if person.active.target != 0 {
            let mut last = person.active.clone();
            last.flags = (last.flags & !0x20) | (u32::from(last_result) << 5);
            person.last = last;
        }
        if action.target != 0 {
            for word in person::STARTED {
                person.data[word] = 0;
            }
        }
        person.active = action;
        person.data[person::PRIORITY] = person.active.priority as i16;
        person.motive_deltas.clear();
        person.data[person::WIDE] = 0;
        for word in person::CLEARED {
            person.data[word] = 0;
        }
        Ok(())
    }
}
