//! Player orders: the player picks an interaction of an object for a
//! person, as from the pie menu.
//!
//! **What the original does** (read from its code). The pie menu opens on
//! an object O for the selected person P (0x48d392):
//!
//! 1. O becomes its interaction target (0x435286: the lead tile of a
//!    multi-tile object; [`crate::Interpreter`]'s `interaction_target`).
//! 2. For each entry of O's interaction table, first to last (0x45859c),
//!    an action is made (0x41f00e) with P as caller, O as target and
//!    icon, the entry's menu number, priority 50, four zero arguments,
//!    and from the first entry with that menu number its action tree,
//!    its attenuation (0x475ec6, by whether P is a visitor) and flag bit
//!    0 from its flag 0x100.
//! 3. Each action is checked (0x458319 with the menu's mode): rules on P
//!    and O and the entry's check tree. One that passes is offered with
//!    flags 4 and 8 set and 0x10 clear (0x458705).
//! 4. When the player picks one, that action is queued on P unchanged
//!    (0x48dd67 calls 0x45d20b). The rest is the menu closing and the
//!    interface's feedback.
//!
//! [`Interpreter::order`] does steps 1, 2 and 4 for an entry the caller
//! names: the action it queues is the one the menu would offer. It does
//! not run the check of step 3, so it does not say whether the menu would
//! offer the entry at all.

use crate::autonomy::{attenuation, find, person};
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;

/// The priority of an action the pie menu makes (0x458633).
pub const MENU_PRIORITY: i32 = 50;
/// The interaction table entry flag that sets the action's flag bit 0
/// (0x41f09e).
const ENTRY_FLAG: u32 = 0x100;
/// The interaction table entry flag that runs the action at once
/// (0x45d235).
const AT_ONCE: u32 = 4;
/// The flags a check that passes leaves on an action (0x458333, 0x458319).
const CHECKED: u32 = 4 | 8;

impl Interpreter<'_> {
    /// The action the pie menu makes for `person` and the entry of
    /// `object`'s interaction table with menu number `menu` (steps 1 and
    /// 2 of the module documentation), as a check that passes leaves it.
    pub fn menu_action(
        &self,
        world: &mut World,
        person: u16,
        object: u16,
        menu: u32,
    ) -> Result<(Interaction, bool), Fault> {
        let fail = |trap| Fault {
            object: person,
            at: None,
            trap,
        };
        let visitor = world
            .objects
            .get(&person)
            .ok_or_else(|| fail(Trap::Invalid(format!("no object {person}"))))?
            .person
            .as_ref()
            .ok_or_else(|| fail(Trap::Invalid(format!("object {person} is not a person"))))?
            .data
            .get(person::VISITOR)
            .is_some_and(|&word| word != 0);
        let machine = self.machine(world, person, 0, false);
        let target = machine.interaction_target(object).map_err(fail)?;
        let entry = machine
            .table(target)
            .map_err(fail)?
            .and_then(|table| find(table, menu))
            .cloned()
            .ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "object {target} has no interaction with menu number {menu}"
                )))
            })?;
        let constants = self.program.autonomy_constants();
        let action = Interaction {
            uid: 0,
            caller: person as i16,
            target: target as i16,
            icon: target as i16,
            table_index: menu as i32,
            args: [0; 4],
            priority: MENU_PRIORITY,
            action: entry.action,
            attenuation: attenuation(&constants, visitor, &entry).to_f32(),
            flags: u32::from(entry.flags & ENTRY_FLAG != 0) | CHECKED,
        };
        Ok((action, entry.flags & AT_ONCE != 0))
    }

    /// Queues on `person` the action the pie menu makes for the entry of
    /// `object` with menu number `menu`, as picking it does (0x48dd67,
    /// 0x45d20b). Returns the action's number, or None when the queue
    /// took nothing (it is full). What the queue does not port fails
    /// loudly: an entry that runs at once, and an action that goes before
    /// queued ones.
    pub fn order(
        &self,
        world: &mut World,
        person: u16,
        object: u16,
        menu: u32,
    ) -> Result<Option<i32>, Fault> {
        let (action, at_once) = self.menu_action(world, person, object, menu)?;
        let mut machine = self.machine(world, person, 0, false);
        machine
            .queue_action(person, action, Some(at_once))
            .map_err(|trap| machine.fault(trap))
    }
}
