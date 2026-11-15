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
//! [`Interpreter::pie_menu`] lists what the menu offers (steps 1 to 3,
//! 0x48d392 and 0x45859c), and [`Interpreter::pick`] queues one of its
//! actions (step 4). [`Interpreter::order`] does steps 1, 2 and 4 for an
//! entry the caller names, without the check.
//!
//! **Listing the menu changes the lot.** Each entry's check tree runs
//! synchronously on the person and is not undone; only the person's
//! temps are put back (saved before the first check tree, restored when
//! the listing ends, 0x48d492). The person's object data word 50 is left
//! as the last check left it. The original lists the menu when the
//! cursor reaches an object (0x48c834), not when it is clicked, and the
//! click shows that list. So a faithful caller lists the menu at the
//! same moments, and a recording of play must note them as inputs. The
//! hover's highlight bits (object +0x114, 0x4320da) are not kept here.
//!
//! The listing ([`Machine::pie_menu`]):
//!
//! 1. A pet is offered only an object of the marker type 0x7c4, unless
//!    interface state 0x6e4f68 is set (0x4585b1). That state is not
//!    modeled, so a pet's menu fails loudly.
//! 2. Each entry of the lead tile's table, first to last, makes an action
//!    (0x41f00e) for its menu number; the action's tree, attenuation and
//!    flag bit 0 come from the first entry with that number, so a menu
//!    number that appears twice makes the same action twice. The flag
//!    0x80 is not read: it only puts "*" before the entry's name.
//! 3. Each action is checked ([`Machine::check_chosen`]) and offered when
//!    the check leaves flag 8 set and 0x10 clear, and the check tree added
//!    no entry of its own (0x458724); there is no greyed-out entry. The
//!    original also hides every entry while an interface flag (0x6f2a54)
//!    is set; only interface code writes it, so it is taken to be clear.
//! 4. What is not ported fails loudly: a check tree adding an entry
//!    (Add/Change Action String, 0x458789); the entry to join another
//!    person's interaction, which the original adds when the person's
//!    running interaction may be joined and the object is another person
//!    (0x45866d); and an object that offers nothing, for which the
//!    original lists the objects on its tile instead (0x48d423).

use crate::autonomy::{attenuation, checked, entry, find, person};
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
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
        let (mut action, at_once) = machine
            .menu_action(person, target, menu, visitor)
            .map_err(fail)?
            .ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "object {target} has no interaction with menu number {menu}"
                )))
            })?;
        action.flags |= CHECKED;
        Ok((action, at_once))
    }

    /// What the pie menu offers `person` on `object`, in its order (see
    /// the module's documentation). Listing it runs the entries' check
    /// trees, which change the lot as the original's do.
    pub fn pie_menu(
        &self,
        world: &mut World,
        person: u16,
        object: u16,
    ) -> Result<Vec<Interaction>, Fault> {
        let mut machine = self.machine(world, person, 0, false);
        machine.pie_menu(object).map_err(|trap| machine.fault(trap))
    }

    /// Queues on `person` an action [`Self::pie_menu`] offered, unchanged,
    /// as picking it does (0x48dd67, 0x45d20b). Returns the action's
    /// number, or None when the queue took nothing: it is full, or the
    /// target's table no longer has the action's entry. What the
    /// queue does not port fails loudly, as for [`Self::order`].
    pub fn pick(
        &self,
        world: &mut World,
        person: u16,
        action: Interaction,
    ) -> Result<Option<i32>, Fault> {
        let mut machine = self.machine(world, person, 0, false);
        let result = (|| {
            let target = u16::try_from(action.target)
                .ok()
                .filter(|t| *t != 0)
                .ok_or_else(|| Trap::Invalid("a picked action has no target".into()))?;
            // No entry: the action is not queued (0x45d22a).
            let at_once = machine
                .table(target)?
                .and_then(|table| find(table, action.table_index as u32))
                .map(|record| record.flags & AT_ONCE != 0);
            machine.queue_action(person, action, at_once)
        })();
        result.map_err(|trap| machine.fault(trap))
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

impl Machine<'_, '_> {
    /// The action the pie menu makes for `person` and menu number `menu`
    /// of `target`'s table (0x41f00e), before its check, and whether the
    /// entry runs at once. None when the table has no entry with that
    /// number.
    fn menu_action(
        &self,
        person: u16,
        target: u16,
        menu: u32,
        visitor: bool,
    ) -> Result<Option<(Interaction, bool)>, Trap> {
        let Some(entry) = self.table(target)?.and_then(|table| find(table, menu)) else {
            return Ok(None);
        };
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
            attenuation: attenuation(&constants, visitor, entry).to_f32(),
            flags: u32::from(entry.flags & ENTRY_FLAG != 0),
        };
        Ok(Some((action, entry.flags & AT_ONCE != 0)))
    }

    /// Lists the pie menu on `object` for the running person (0x48d392;
    /// see the module's documentation).
    pub(crate) fn pie_menu(&mut self, object: u16) -> Result<Vec<Interaction>, Trap> {
        let me = self.me;
        let data = &self
            .me()?
            .person
            .as_ref()
            .ok_or_else(|| Trap::Invalid(format!("object {me} is not a person")))?
            .data;
        let word = |index: usize| data.get(index).copied().unwrap_or(0);
        if word(person::KIND) & 6 != 0 {
            return Err(Trap::Unsupported(
                "a pet's pie menu, which reads interface state (0x6e4f68, 0x4585b1)".into(),
            ));
        }
        let target = self.interaction_target(object)?;
        let menus: Vec<u32> = self
            .table(target)?
            .map(|table| table.interactions.iter().map(|e| e.string_index).collect())
            .unwrap_or_default();
        let mut saved = None;
        let outer = std::mem::replace(&mut self.building_menu, true);
        let listed = self.list_entries(target, &menus, &mut saved);
        self.building_menu = outer;
        if let Some(temps) = saved {
            self.me_mut()?.temps = temps;
        }
        let listed = listed?;
        if listed.is_empty() {
            return Err(Trap::Unsupported(format!(
                "the pie menu on object {target}, which offers nothing, where the original lists \
                 the objects on its tile (0x48d423)"
            )));
        }
        Ok(listed)
    }

    /// Steps 2 to 4 of the listing (0x45859c).
    fn list_entries(
        &mut self,
        target: u16,
        menus: &[u32],
        saved: &mut Option<Vec<i16>>,
    ) -> Result<Vec<Interaction>, Trap> {
        let me = self.me;
        let mut listed = Vec::new();
        for &menu in menus {
            // Read for each action, after the checks before it (0x41f08b).
            let visitor = self
                .me()?
                .person
                .as_ref()
                .and_then(|p| p.data.get(person::VISITOR).copied())
                .unwrap_or(0)
                != 0;
            let Some((mut action, _)) = self.menu_action(me, target, menu, visitor)? else {
                continue;
            };
            self.check_chosen(&mut action, saved)?;
            if action.flags & checked::PASSED != 0 && action.flags & checked::MARKED == 0 {
                listed.push(action);
            }
        }
        let running = self
            .me()?
            .person
            .as_ref()
            .map(|p| p.active.clone())
            .ok_or_else(|| Trap::Invalid(format!("object {me} is not a person")))?;
        // The running interaction's entry, read as autonomy reads it.
        let joinable = u16::try_from(running.target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .map(|id| self.table(id))
            .transpose()?
            .flatten()
            .and_then(|table| find(table, running.table_index as u32))
            .is_some_and(|record| record.flags & entry::JOINABLE != 0);
        let other_person = target != me
            && self
                .world
                .objects
                .get(&target)
                .is_some_and(|o| o.person.is_some());
        if joinable && other_person {
            return Err(Trap::Unsupported(format!(
                "the pie menu on person {target} while my running interaction may be joined, \
                 where the original offers to join it (0x45866d, 0x41ee99)"
            )));
        }
        Ok(listed)
    }
}
