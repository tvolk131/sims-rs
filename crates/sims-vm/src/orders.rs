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
//! 4. When the player picks one, that action is queued on its caller
//!    unchanged (0x48ddbe calls 0x45d20b): on P, but for the offer to
//!    join P's running interaction, whose caller is O. The rest is the
//!    menu closing and the interface's feedback.
//!
//! [`Interpreter::pie_menu`] lists what the menu offers (steps 1 to 3,
//! 0x48d392 and 0x45859c, and the offer to join, below), and
//! [`Interpreter::pick`] queues one of its actions (step 4). [`Interpreter::order`] does steps 1, 2 and 4 for an
//! entry the caller names, without the check.
//!
//! **Listing the menu changes the lot.** Each entry's check tree runs
//! synchronously on the person and is not undone; only the person's
//! temps are put back (saved before the first check tree, restored when
//! the listing ends, 0x48d492). The offer to join runs its check tree on
//! the other person. The person's object data word 50 is left
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
//!    The check tree adds an entry with Add/Change Action String
//!    (0x458789): a copy of the action being checked, as the check has
//!    left it so far, with flags 4 and 8 set and 0x10 clear, the string
//!    as its name and the string's four arguments as its own. It is added
//!    to the menu then, whether or not the check passes, and the action
//!    itself is not.
//! 4. After the table's entries, the offer to join P's running
//!    interaction (0x45866d), when its entry in its target's own table
//!    has flag 2 (may be joined) and O is another person. The action
//!    (0x41ee99) is a copy of P's running one (0x41edbf) with flags 2 and
//!    4 cleared, word +0 1 (it joins), O as caller and priority 50. It
//!    goes to the running target's interaction target (0x435286), to the
//!    entry whose menu number is the joining index of that target's first
//!    entry with the running menu number. Without a table, a first entry,
//!    a joining index other than -1 or an entry with it, the action has
//!    no target and keeps the running menu number. Its arguments are
//!    cleared and its number is 0; its tree, attenuation (by whether O is
//!    a visitor) and flag bit 0 come from its entry, or are 0 without
//!    one. The listing then gives it P's object ID as its third argument
//!    (0x4586c7) and checks and offers it as step 3 does. The check reads
//!    O, the caller, and runs the check tree on O's stack with the
//!    arguments 0, 1, P's ID and 0 (0x458372); one without a target is
//!    not offered. The original copies the running action's name too, and
//!    then gives it the entry's name (0x41efc1); here a [`MenuEntry`]
//!    without a name shows its entry's, which is the same. The menu's
//!    temps are those of the first person a check tree ran on, so O's
//!    when the offer's check tree is the listing's first. A target
//!    without a table offers nothing, this offer included (0x4585ff).
//! 5. When the listing offers nothing (0x48d423), each object on the
//!    clicked object's tile (object +0xfc, read through 0x43f331 and
//!    0x43f379, in the order the lot links them, the links read again
//!    after each listing; none outside the lot's size or off levels 1
//!    and 2) is listed in turn, steps 1 to 4, the pet's check included,
//!    and what it offers is added to the menu. The clicked object
//!    is left out, and so are an object of type 8 (definition word 9) and
//!    one whose definition word 91 (+0xb6) lacks bit 0. Each of these
//!    listings has its own temps: saved before its first check tree and
//!    put back when it ends (0x4582dd), before the next object's; the
//!    first listing's are put back last. A menu that still offers
//!    nothing is empty. The tile is the one the object's position is on;
//!    for an object held in a slot that is **unverified**.
//!
//! The listing names no entry: the menu's window works the names out
//! when it is built (0x48d5ce, 0x41f167), which also clears each action's
//! flag 0x80 (0x41f1a4). So a listed action may still have that flag
//! (the offer copies it from the running action), and [`Interpreter::pick`]
//! and [`Interpreter::highlight`] clear it as the window would have.
//!
//! When the menu's mode (+0x3c) is 2 or 3, 0x48d392 lists differently
//! (0x48d4b7), with actions made by 0x41ee2f, whose word +0 is 2. That
//! listing is not ported.

use crate::autonomy::{JOINS, SavedTemps, attenuation, checked, entry, find, person};
use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Machine};
use crate::placement::Cursor;
use crate::primitives::action_string::NAMED;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;

/// An entry the pie menu offers: the action picking it queues, and the
/// name a check tree gave it (Add/Change Action String, 0x458789). An
/// entry without a name shows its interaction table entry's.
///
/// The original keeps the name in the action (+0x28) and queues it with
/// the action; the queue here keeps actions only, so a picked entry's
/// name is not kept.
#[derive(Debug, Clone)]
pub struct MenuEntry {
    pub action: Interaction,
    pub name: Option<String>,
}

/// The listing's state while it checks an action (the original's
/// pointers 0x6e5130 and 0x6e50fc, the menu being built, and 0x6e50f8,
/// the action being checked): see [`crate::Interpreter`]'s `menu`.
#[derive(Debug)]
pub(crate) struct Checking {
    /// The action being checked, as the check has left it so far.
    pub action: Interaction,
    /// The entries the check has added.
    pub added: Vec<MenuEntry>,
}

/// Definition word 9, the object's type; an object of type 8 is left out
/// of the tile's listing (0x48d44c).
const TYPE: usize = 9;
const TILE_SKIPPED: i16 = 8;
/// Definition word 91 (+0xb6): with bit 0 set, the object is listed when
/// an object on its tile offers nothing (0x48d453).
const MENU_WORD: usize = 91;
const ON_TILE: i16 = 1;

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
    ) -> Result<Vec<MenuEntry>, Fault> {
        let mut machine = self.machine(world, person, 0, false);
        machine.pie_menu(object).map_err(|trap| machine.fault(trap))
    }

    /// Checks again the action of the entry of `person`'s pie menu the
    /// pointer has reached, as the original does when the pointer reaches
    /// an entry (0x48df28). Returns the action with the flags the check
    /// left. The check reads the action's caller (`person`, but for the
    /// offer to join) and runs the entry's check tree on the lot, as
    /// autonomy's check does; it is not undone but for the caller's
    /// temps.
    ///
    /// The menu's window works out each entry's name when it is built
    /// (0x48d5ce, 0x41f167), which clears the action's flag 0x80
    /// (0x41f1a4), so the action is taken with that flag clear. An action
    /// without a caller is not checked (0x48dfa5) and comes back as it
    /// went in, but for that flag.
    pub fn highlight(
        &self,
        world: &mut World,
        person: u16,
        mut action: Interaction,
    ) -> Result<Interaction, Fault> {
        action.flags &= !NAMED;
        if action.caller == 0 {
            return Ok(action);
        }
        let mut machine = self.machine(world, person, 0, false);
        machine
            .highlight(&mut action)
            .map_err(|trap| machine.fault(trap))?;
        Ok(action)
    }

    /// Queues the action of an entry [`Self::pie_menu`] listed for
    /// `person` on the action's caller, unchanged, as picking it does
    /// (0x48ddbe, 0x45d20b). The caller is `person` for every entry but
    /// the offer to join `person`'s running interaction, whose caller is
    /// the person the menu was listed on: picking it queues the action on
    /// that person. Returns the action's number, or None when the queue
    /// took nothing: it is full, or the target's table no longer has the
    /// action's entry. What the queue does not port fails loudly, as for
    /// [`Self::order`].
    ///
    /// As for [`Self::highlight`], the action is taken with flag 0x80
    /// clear, as the menu's window leaves it. An action without a caller
    /// is not queued (0x48ddc6) and None comes back.
    pub fn pick(
        &self,
        world: &mut World,
        person: u16,
        mut action: Interaction,
    ) -> Result<Option<i32>, Fault> {
        action.flags &= !NAMED;
        if action.caller == 0 {
            return Ok(None);
        }
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
            let caller = machine.caller(&action)?;
            machine.queue_action(caller, action, at_once)
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
    pub(crate) fn menu_action(
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
            join: 0,
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
    pub(crate) fn pie_menu(&mut self, object: u16) -> Result<Vec<MenuEntry>, Trap> {
        let mut saved = None;
        let listed = self.list_with_tile(object, &mut saved);
        // The first listing's temps go back last (0x48d492), after the
        // tile's.
        self.put_back_temps(saved)?;
        listed
    }

    /// The listing on `object`, then, when it offers nothing, the listing
    /// on each other object on its tile (0x48d423; step 5 of the module's
    /// documentation).
    fn list_with_tile(
        &mut self,
        object: u16,
        saved: &mut SavedTemps,
    ) -> Result<Vec<MenuEntry>, Trap> {
        let mut listed = self.listing(object, saved)?;
        if !listed.is_empty() {
            return Ok(listed);
        }
        let position = self
            .world
            .objects
            .get(&object)
            .ok_or(Trap::native(0x15, "missing object"))?
            .position;
        // The walk reads the tile's links again after each listing, so a
        // check tree that moves or deletes an object changes what follows.
        let mut cursor = Cursor::on_tile(self.world, World::tile(position));
        while let Some(other) = cursor.current() {
            if other != object {
                let guid = self
                    .world
                    .objects
                    .get(&other)
                    .ok_or(Trap::native(0x15, "missing object"))?
                    .guid;
                let words = self.definition_words(guid)?;
                if words[TYPE] != TILE_SKIPPED && words[MENU_WORD] & ON_TILE != 0 {
                    // Each object's own listing puts its temps back when it
                    // ends (0x4582dd).
                    let mut own = None;
                    let more = self.listing(other, &mut own);
                    self.put_back_temps(own)?;
                    listed.extend(more?);
                }
            }
            cursor.advance(self.world)?;
        }
        Ok(listed)
    }

    /// One listing (0x4582ad, 0x458583): the entries of the table of
    /// `object`'s interaction target.
    fn listing(&mut self, object: u16, saved: &mut SavedTemps) -> Result<Vec<MenuEntry>, Trap> {
        // Read for each listing, after the checks of any before it.
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
        // A target without a table offers nothing, not even the offer to
        // join (0x4585ff).
        let Some(menus) = self.table(target)?.map(|table| {
            table
                .interactions
                .iter()
                .map(|e| e.string_index)
                .collect::<Vec<_>>()
        }) else {
            return Ok(Vec::new());
        };
        self.list_entries(target, &menus, saved)
    }

    /// Steps 2 to 4 of the listing (0x45859c).
    fn list_entries(
        &mut self,
        target: u16,
        menus: &[u32],
        saved: &mut SavedTemps,
    ) -> Result<Vec<MenuEntry>, Trap> {
        let me = self.me;
        let mut listed = Vec::new();
        for &menu in menus {
            // Read for each action, after the checks before it (0x41f08b).
            let visitor = self.visitor(me)?;
            let Some((action, _)) = self.menu_action(me, target, menu, visitor)? else {
                continue;
            };
            self.offer(action, saved, &mut listed)?;
        }
        // Read after the entries' checks (0x45866d).
        if let Some(action) = self.join_action(target)? {
            self.offer(action, saved, &mut listed)?;
        }
        Ok(listed)
    }

    /// Checks an action and offers it (0x458705): it is offered when the
    /// check leaves flag 8 set and 0x10 clear, and only when the check
    /// tree added no entry of its own (0x458724).
    fn offer(
        &mut self,
        mut action: Interaction,
        saved: &mut SavedTemps,
        listed: &mut Vec<MenuEntry>,
    ) -> Result<(), Trap> {
        let before = listed.len();
        self.check_chosen(&mut action, saved, Some(listed))?;
        if listed.len() == before
            && action.flags & checked::PASSED != 0
            && action.flags & checked::MARKED == 0
        {
            listed.push(MenuEntry { action, name: None });
        }
        Ok(())
    }

    /// Whether person `id` is a visitor (person data word 32, +0x5cc).
    fn visitor(&self, id: u16) -> Result<bool, Trap> {
        Ok(self
            .world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .ok_or_else(|| Trap::Invalid(format!("object {id} is not a person")))?
            .data
            .get(person::VISITOR)
            .is_some_and(|&word| word != 0))
    }

    /// The offer to join the running person's running interaction, on
    /// the pie menu of `target` (step 4 of the module's documentation;
    /// 0x45866d, 0x41ee99), or None when there is none.
    fn join_action(&self, target: u16) -> Result<Option<Interaction>, Trap> {
        let me = self.me;
        let running = self
            .me()?
            .person
            .as_ref()
            .map(|p| p.active.clone())
            .ok_or_else(|| Trap::Invalid(format!("object {me} is not a person")))?;
        // The running interaction's entry, in its target's own table
        // (0x41f0ff), read as autonomy reads it.
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
        if !joinable || !other_person {
            return Ok(None);
        }
        // A copy of the running action (0x41edbf), with flags 2 and 4
        // cleared, word +0 1, the other person as caller and priority 50
        // (0x41eed0 to 0x41eedd).
        let mut action = Interaction {
            join: JOINS,
            caller: target as i16,
            priority: MENU_PRIORITY,
            flags: running.flags & !6,
            ..running.clone()
        };
        // It goes to the running target's interaction target, to the
        // entry whose menu number is the joining index of that target's
        // first entry with the running menu number; without one, to no
        // target, the menu number left as it was (0x41eee4 to 0x41ef77).
        // `joinable` holds only if the running target exists.
        let goes_to = self.interaction_target(running.target as u16)?;
        let joining = self.table(goes_to)?.and_then(|table| {
            let first = find(table, running.table_index as u32)?;
            let index = first.joining_index;
            (index != -1 && find(table, index as u32).is_some()).then_some(index)
        });
        match joining {
            Some(index) => {
                action.target = goes_to as i16;
                action.table_index = index;
            }
            None => action.target = 0,
        }
        action.args = [0; 4];
        action.uid = 0;
        // The rest from the action's entry (0x41f0ff), as for any action
        // the menu makes, the attenuation by whether the other person is
        // a visitor (0x41ef8f to 0x41eff7).
        let record = match joining {
            Some(_) => self
                .table(goes_to)?
                .and_then(|table| find(table, action.table_index as u32))
                .cloned(),
            None => None,
        };
        match record {
            Some(record) => {
                let constants = self.program.autonomy_constants();
                action.action = record.action;
                action.attenuation =
                    attenuation(&constants, self.visitor(target)?, &record).to_f32();
                action.flags = (action.flags & !1) | u32::from(record.flags & ENTRY_FLAG != 0);
            }
            None => {
                action.action = 0;
                action.attenuation = 0.0;
                action.flags &= !1;
            }
        }
        // The listing gives it the running person's object ID as its third
        // argument (0x4586c7).
        action.args[2] = me as i16;
        Ok(Some(action))
    }
}
