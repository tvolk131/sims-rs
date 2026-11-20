//! Dialog (opcode 36): ask the player something and read back the answer.
//!
//! Read from the original's 0x451552. The instruction runs again every
//! tick until it has an answer:
//!
//! 1. It looks up the first dialog record the object owns (0x44435c). A
//!    record that was shown and not answered (status 1) is closed
//!    (0x437058 sets status 5), and a closed one counts as none.
//! 2. With operand 7 bit 0 set the dialog only informs: a record is made
//!    (0x4443ab) and the true branch taken at once.
//! 3. Otherwise, with no record, a record is made and the instruction
//!    yields. The registry shows the record at the end of the lot update
//!    (`tick.rs`), and the player's answer goes into its status.
//! 4. With a record, the record is acknowledged (0x43718f: status 2 to 4
//!    becomes 5) and the status read: not answered yet (0 or 1) yields
//!    again; the first button (2) takes the true branch; the second (3)
//!    takes the false branch, clears the temp operand 7 names in a
//!    three-button dialog, and sets registry word 0x100 to -1; the third
//!    (4) takes the false branch and sets that temp to 1.
//!
//! Operand 5 (bits 0 to 6) is the kind of dialog. Kinds 0 and 4 have one
//! button, kind 1 two and kind 2 three; these are ported. Kind 3 asks for
//! text, and kinds 5 to 15 are the game's special panels (careers,
//! neighbors, the phone and others); they fail loudly. What a dialog
//! says and shows (its text, pictures and buttons' labels) is
//! presentation and is not modeled, except where showing a person's
//! portrait builds it ([`Machine::dialog_portrait`]): building one for a
//! person type with nobody on the lot makes a person and removes it at
//! once. Making a record is otherwise assumed not to run any object's
//! code or change any simulation state: **unverified**.
//!
//! In the original, step 3 makes no record while the interface has a
//! modal dialog open (interface word 0x7c, 0x451a8e). Here modal dialogs
//! are shown and answered inside the end of the lot update, so none is
//! open while scripts run.
//!
//! The evidence (`docs/decisions/`): on the Goth house five controllers
//! reach a one-button dialog in the first tick and yield; the run of the
//! original answered each, and in the next tick each took the true
//! branch. Object 15 then made a two-button dialog, answered with the
//! second button, and took the false branch the tick after. The rest of
//! the protocol follows the original's code and the old prototype's
//! `Protocol::step`.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_sim::Dialog;

/// A definition's type word (+0x12), and its value for a person.
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;

/// Record status: made and not shown yet.
pub const NEW: u8 = 0;
/// Shown, not answered.
pub const SHOWN: u8 = 1;
/// Answered with the first, second or third button.
pub const FIRST: u8 = 2;
pub const SECOND: u8 = 3;
pub const THIRD: u8 = 4;
/// Done: the registry destroys it at the end of the lot update.
pub const DONE: u8 = 5;

/// The answers a dialog of this kind (operand 5) offers, as statuses, or
/// None for a kind that is not ported. From the original's button setup
/// (0x43601c, 0x436a52), as the old prototype read it
/// (legacy/crates/sims-vm/src/dialog/presentation.rs, `buttons`).
pub fn answers(kind: u8) -> Option<&'static [u8]> {
    match kind & 0x7f {
        0 | 4 => Some(&[FIRST]),
        1 => Some(&[FIRST, SECOND]),
        2 => Some(&[FIRST, SECOND, THIRD]),
        _ => None,
    }
}

/// Whether a record's dialog stops the game until it is answered (operand
/// 7 bit 7 clear; record byte 0x69).
pub fn modal(operands: &[u8; 8]) -> bool {
    operands[7] & 0x80 == 0
}

/// Whether a record only informs: it is answered once and is done (operand
/// 7 bit 0).
pub fn informs(operands: &[u8; 8]) -> bool {
    operands[7] & 1 != 0
}

impl Machine<'_, '_> {
    pub(crate) fn dialog(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let kind = operands[5] & 0x7f;
        if answers(kind).is_none() {
            return Err(Trap::Unsupported(format!(
                "Dialog kind {kind} (operand 5) is {}, which is not ported",
                if kind == 3 {
                    "a dialog that asks for text"
                } else {
                    "one of the game's special panels"
                }
            )));
        }
        let me = self.me;
        let records = &mut self.world.dialogs.records;
        let mut found = records.iter().position(|record| record.caller == me);
        if let Some(index) = found {
            match records[index].status {
                SHOWN => {
                    records[index].status = DONE;
                    found = None;
                }
                DONE => found = None,
                _ => {}
            }
        }
        let record = Dialog {
            caller: me,
            operands: *operands,
            status: NEW,
        };
        if informs(operands) {
            self.dialog_portrait(operands)?;
            self.world.dialogs.records.push(record);
            return Ok(Flow::Branch(true));
        }
        let Some(index) = found else {
            self.dialog_portrait(operands)?;
            self.world.dialogs.records.push(record);
            return Ok(Flow::Yield);
        };
        let records = &mut self.world.dialogs.records;
        let status = records[index].status;
        if (FIRST..=THIRD).contains(&status) {
            records[index].status = DONE;
        }
        let temp = usize::from((operands[7] >> 4) & 7);
        Ok(match status {
            NEW | SHOWN => Flow::Yield,
            FIRST => Flow::Branch(true),
            SECOND => {
                if kind == 2 {
                    self.set_temp(temp, 0)?;
                }
                self.world.dialogs.declined = -1;
                Flow::Branch(false)
            }
            THIRD => {
                if kind == 2 {
                    self.set_temp(temp, 1)?;
                }
                Flow::Branch(false)
            }
            _ => Flow::Branch(true),
        })
    }

    /// The portrait an ordinary dialog's record shows, built as the
    /// record is made (0x4443ab, 0x435d8a, then 0x43601c), before it joins
    /// the registry. The icon mode is operand 7's bits 1 to 3 (0x436279):
    ///
    /// - mode 0 (0x436308): the type of the object the stack object names,
    ///   when it is on the lot, unless the dialog's text named a neighbor
    ///   whose type it shows instead (0x44bf1a, the text's tokens; not
    ///   ported, so such a dialog builds the stack object's portrait here:
    ///   **unverified**);
    /// - mode 2 (0x4362f4): the type of the neighbor record the stack
    ///   object names (0x425afe: 0x4256f4, record +8);
    /// - modes 1 and 4 to 7 show no person (the type stays 0 from
    ///   0x436271), and mode 3 shows a picture of the type's
    ///   (0x4362ca, 0x436362).
    ///
    /// A type that is a person (definition +0x12 is 2, 0x4575d2) has its
    /// entry in the interface's table of person types looked up by GUID
    /// (0x4572af, 0x4e9753), and 0x4e92a8 builds its first image when the
    /// entry has none: 0x4e8c5d draws the first person of that type on
    /// the lot (0x4e9608 walks the lot's people, +0x84). With none, it
    /// makes one (0x4e9652: the factory 0x440b2d, [`crate::Interpreter::create_person`]),
    /// finds it a tile from the house view's centre (0x4347f0 from
    /// 0x4a677c) and puts it there (virtual function 0x88), draws it,
    /// and removes it at once (0x44123f, [`crate::Interpreter::remove_now`]).
    /// The table holds every person type the catalog had when the game
    /// started (0x4e8b3d); a type made later ([`sims_sim::World::characters`]) may
    /// not be in it, and fails loudly.
    ///
    /// Which entries have their image is kept in [`sims_sim::World::portraits`].
    /// Only the dialog's builds are counted. The household's panel draws
    /// the family's portraits too, so a family member's type with nobody
    /// on the lot fails loudly. The person made is not put on a tile: the
    /// house view's centre is presentation, and the person is removed
    /// before anything runs. If no tile is found the original removes it
    /// without drawing it and the entry stays without its image; here the
    /// image is taken to be built (**unverified**).
    pub(crate) fn dialog_portrait(&mut self, operands: &[u8; 8]) -> Result<(), Trap> {
        let stack_object = self.frame()?.stack_object;
        let guid = match (operands[7] >> 1) & 7 {
            0 => match u16::try_from(stack_object)
                .ok()
                .and_then(|id| self.world.objects.get(&id))
            {
                Some(object) => object.guid,
                None => return Ok(()),
            },
            2 => match self.world.neighbors.get(stack_object) {
                Some(record) => record.guid,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        if self.world.characters.made.contains_key(&guid) {
            return Err(Trap::Unsupported(format!(
                "a dialog's portrait of type 0x{guid:08X}, made while the game ran: whether the \
                 interface's table of person types (0x6f2698) has it is not known"
            )));
        }
        let person = crate::interpreter::object_definition(self.catalog, self.world, guid)
            .is_some()
            && self.definition_words(guid)?[TYPE] == PERSON_TYPE;
        if !person {
            return Ok(());
        }
        let on_lot = self.world.people.iter().any(|id| {
            self.world
                .objects
                .get(id)
                .is_some_and(|object| object.guid == guid)
        });
        if !on_lot {
            let family = i32::from(self.world.globals.get(9).copied().unwrap_or(0));
            if self
                .world
                .families
                .iter()
                .any(|f| f.id == family && f.members.contains(&guid))
            {
                return Err(Trap::Unsupported(format!(
                    "a dialog's portrait of the household's type 0x{guid:08X} with nobody of \
                     it on the lot (0x4e9652): whether the household's panel built it before \
                     is not kept"
                )));
            }
            let interpreter = self.interpreter();
            let unsupported = |fault| {
                Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652): \
                     {fault}"
                ))
            };
            let id = interpreter
                .create_person(self.world, guid)
                .map_err(unsupported)?;
            interpreter
                .remove_now(self.world, id)
                .map_err(unsupported)?;
        }
        self.world.portraits.insert(guid);
        Ok(())
    }

    fn set_temp(&mut self, temp: usize, value: i16) -> Result<(), Trap> {
        let object = self.me_mut()?;
        let slot = object
            .temps
            .get_mut(temp)
            .ok_or(Trap::native(2, "temp out of range"))?;
        *slot = value;
        Ok(())
    }
}
