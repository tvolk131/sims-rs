//! Push Interaction (opcode 13, 0x44cd2b): queue an interaction of an
//! object on the person in the stack object, as a script does to make a
//! person do something.
//!
//! Operands: byte 0 is the interaction's menu number, byte 1 the index
//! of the parameter or local that names the object, byte 2 the priority,
//! byte 3 flags and byte 4 the index of the local that names the icon.
//!
//! 1. The object: the parameter at byte 1, or with flag bit 1 the local
//!    at byte 1. An index past the count is native error 8 for a
//!    parameter and 0x33 for a local. The original's bound lets the index
//!    equal the count, which reads the word past the last one; that fails
//!    loudly. An ID that names no object is native error 0x17.
//! 2. The object's interaction table (0x457cb0): none is native error
//!    0x1f. Its first entry with the menu number (+0x18): none is native
//!    error 0x16.
//! 3. The person: the frame's stack object. An ID that names no object
//!    is native error 0x15; an object that is not a person (its type,
//!    definition word 9, is not 2) is native error 0x1c.
//! 4. The priority, by byte 2: 0 is the running action's (person data
//!    word 33, +0x5ce) when I am a person and 1 otherwise; 1 is 100, 2 is
//!    2 and 3 is 50. Any other is native error 0x2d.
//! 5. The action (0x41f00e): the person as caller, the object as target
//!    and icon, the menu number, the priority, four zero arguments, and
//!    from the entry its action tree, its attenuation (0x475ec6, by
//!    whether the person is a visitor) and flag bit 0 from its flag 0x100.
//!    Then flag 0x40 is set from operand flag bit 3, and with flag bit 0
//!    the icon becomes the object the local at byte 4 names, or none (0)
//!    for an ID that names no object; an index of at least the count of
//!    locals is native error 0x33. Flag 2 (the queue's front) is set from
//!    operand flag bit 2.
//! 6. The action is queued on the person (0x45d20b,
//!    [`Machine::queue_action`]): the branch is true when it was queued
//!    and false when it was not.
//!
//! A local that still holds half of a pointer the original left in the
//! frame storage fails loudly, as every local read does.
//!
//! What the queue does not port fails loudly there: an entry that runs at
//! once (its flag 4), an action put at the queue's front, and one that
//! goes before queued actions.
//!
//! Ported from the original's code (above). The old prototype's port
//! (legacy/crates/sims-vm/src/action.rs) agrees on the steps.

use crate::autonomy::{attenuation, find, person};
use crate::fault::Trap;
use crate::interpreter::Machine;
use sims_formats::saved_objects::Interaction;

/// Person data word 33: the running action's priority.
const RUNNING_PRIORITY: usize = 33;
/// The interaction table entry flag that runs the action at once
/// (0x45d235).
const AT_ONCE: u32 = 4;
/// The interaction table entry flag that sets the action's flag bit 0
/// (0x41f09e).
const ENTRY_FLAG: u32 = 0x100;
/// Action flags Push Interaction sets from its operand flags.
const FLAG_40: u32 = 0x40;
const TO_FRONT: u32 = 2;

impl Machine<'_, '_> {
    pub(crate) fn push_interaction(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let menu = u32::from(operands[0]);
        let index = usize::from(operands[1]);
        let flags = operands[3];
        let frame = self.frame()?;
        let (values, code, what) = if flags & 2 != 0 {
            (&frame.locals, 0x33, "local")
        } else {
            (&frame.parameters, 8, "parameter")
        };
        if index > values.len() {
            return Err(Trap::native(code, "Push Interaction names no such word"));
        }
        let Some(&value) = values.get(index) else {
            return Err(Trap::Unsupported(format!(
                "Push Interaction reading {what} {index}, one past the frame's last, which the \
                 original's bound lets through (0x44cd5a)"
            )));
        };
        let stack_object = frame.stack_object;
        if flags & 2 != 0 {
            self.check_local(index)?;
        }
        let target = u16::try_from(value)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x17, "Push Interaction names no object"))?;
        let entry = self
            .table(target)?
            .ok_or(Trap::native(
                0x1f,
                "Push Interaction's object has no interaction table",
            ))
            .and_then(|table| {
                find(table, menu).cloned().ok_or(Trap::native(
                    0x16,
                    "Push Interaction names no interaction of the object",
                ))
            })?;
        let person = u16::try_from(stack_object)
            .ok()
            .filter(|id| *id > 0 && self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x15, "Push Interaction has no stack object"))?;
        let visitor = self.world.objects[&person]
            .person
            .as_ref()
            .ok_or(Trap::native(
                0x1c,
                "Push Interaction's stack object is not a person",
            ))?
            .data
            .get(person::VISITOR)
            .is_some_and(|&word| word != 0);
        let priority = match operands[2] {
            0 => match &self.me()?.person {
                Some(me) => i32::from(me.data.get(RUNNING_PRIORITY).copied().ok_or_else(|| {
                    Trap::Invalid("the running person has no data word 33".into())
                })?),
                None => 1,
            },
            1 => 100,
            2 => 2,
            3 => 50,
            _ => {
                return Err(Trap::native(
                    0x2d,
                    "Push Interaction has an unknown priority",
                ));
            }
        };
        let constants = self.program.autonomy_constants();
        let mut action = Interaction {
            uid: 0,
            caller: person as i16,
            target: target as i16,
            icon: target as i16,
            table_index: menu as i32,
            args: [0; 4],
            priority,
            action: entry.action,
            attenuation: attenuation(&constants, visitor, &entry).to_f32(),
            flags: u32::from(entry.flags & ENTRY_FLAG != 0),
        };
        if flags & 8 != 0 {
            action.flags |= FLAG_40;
        }
        if flags & 1 != 0 {
            let icon = usize::from(operands[4]);
            let value = *self
                .frame()?
                .locals
                .get(icon)
                .ok_or(Trap::native(0x33, "Push Interaction's icon names no local"))?;
            self.check_local(icon)?;
            action.icon = u16::try_from(value)
                .ok()
                .filter(|id| *id > 0 && self.world.objects.contains_key(id))
                .map_or(0, |id| id as i16);
        }
        if flags & 4 != 0 {
            action.flags |= TO_FRONT;
        }
        let at_once = entry.flags & AT_ONCE != 0;
        Ok(self.queue_action(person, action, Some(at_once))?.is_some())
    }
}
