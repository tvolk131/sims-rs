//! Gosub Found Action (opcode 30, 0x45a859): a person starts the
//! interaction Find Best Interaction chose.
//!
//! Find Best Interaction leaves the chosen object in the frame's stack
//! object and the menu number of its interaction in the person's object
//! data word 20 ([`crate::autonomy`]). This primitive:
//!
//! 1. Fails with native error 0x15 if the stack object is 0. An ID that
//!    names no object, or an object without an interaction table
//!    (0x457cb0), is the false branch.
//! 2. Finds the table's first entry with that menu number; none is native
//!    error 0x16.
//! 3. Runs the entry's action tree as the person's virtual function 0x34
//!    does (0x45e59b): the person is woken (0x44574b), the tree of the
//!    object's type is pushed on the person's thread with the object as
//!    its stack object and the arguments 0, 0, 0, 0 (0x452b98, 0x47484a),
//!    and the person starts using the object ([`Machine::count_use`]).
//! 4. Makes the action (0x41f00e, 0x41f0d9): the person as caller, the
//!    object as target and icon, the menu number, priority 2, the
//!    entry's action tree, the entry's attenuation (0x475ec6, by whether
//!    the person is a visitor), flag bit 0 from the entry's flag 0x100,
//!    and four zero arguments.
//! 5. Makes it the person's running action ([`Machine::set_action`],
//!    0x45e6cd).
//!
//! The pushed tree runs next; when it returns, the instruction takes the
//! branch of its result (0x474dae: the frame holds no primitive state).
//!
//! If the tree cannot be pushed the original returns -1, which the
//! interpreter treats as a yield; that fails loudly here, as does a tree
//! with more than four arguments (0x47484a would read them past the four
//! it is given).
//!
//! Ported from the original's code at the addresses given; the old
//! prototype's `gosub_found_action` (legacy/crates/sims-vm/src/action.rs)
//! was not trusted. Compared with captures of the original in the Goth
//! house's tick 233 (`docs/decisions/`).

use crate::autonomy::{attenuation, data, find, person};
use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_formats::saved_objects::Interaction;

/// The priority the action gets (0x45a940).
const PRIORITY: i32 = 2;

/// The interaction table entry flag that sets the action's flag bit 0
/// (0x41f09e).
const ENTRY_FLAG: u32 = 0x100;

impl Machine<'_, '_> {
    pub(crate) fn gosub_found_action(&mut self) -> Result<Flow, Trap> {
        let me = self.me;
        let stack_object = self.frame()?.stack_object;
        if stack_object == 0 {
            return Err(Trap::native(0x15, "Gosub Found Action without an object"));
        }
        let Some(target) = u16::try_from(stack_object)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
        else {
            return Ok(Flow::Branch(false));
        };
        let Some(table) = self.table(target)? else {
            return Ok(Flow::Branch(false));
        };
        let object = self.me()?;
        let words = &object
            .person
            .as_ref()
            .ok_or_else(|| Trap::Unsupported("Gosub Found Action on a non-person".into()))?
            .data;
        let visitor = words.get(person::VISITOR).is_some_and(|&w| w != 0);
        let menu = i32::from(object.data.get(data::CHOSEN).copied().unwrap_or(0));
        let record = find(table, menu as u32).ok_or(Trap::native(
            0x16,
            "no interaction with the found menu number",
        ))?;
        let constants = self.program.autonomy_constants();
        let action = Interaction {
            uid: 0,
            caller: me as i16,
            target: target as i16,
            icon: target as i16,
            table_index: menu,
            args: [0; 4],
            priority: PRIORITY,
            action: record.action,
            attenuation: attenuation(&constants, visitor, record).to_f32(),
            flags: u32::from(record.flags & ENTRY_FLAG != 0),
        };
        let tree = action.action;
        self.wake(me)?;
        self.enter_on(target, tree, &[0; 4]).map_err(|fault| {
            Trap::Unsupported(format!(
                "Gosub Found Action could not push tree {tree:#06x} of object {target}, where \
                 the original yields: {fault}"
            ))
        })?;
        self.count_use(target, false)?;
        let last_result = self.me()?.last_result;
        self.set_action(action, last_result)?;
        Ok(Flow::Pushed)
    }
}
