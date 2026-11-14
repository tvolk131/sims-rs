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
//! presentation and is not modeled. Making a record is assumed not to run
//! any object's code or change any simulation state: **unverified**; the
//! old prototype's port of 0x4443ab defers the record's construction for
//! some icon kinds because it can call back into the game
//! (legacy/crates/sims-vm/src/dialog.rs, `dialog_primitive`).
//!
//! In the original, step 3 makes no record while the interface has a
//! modal dialog open (interface word 0x7c, 0x451a8e). Here modal dialogs
//! are shown and answered inside the end of the lot update, so none is
//! open while scripts run.
//!
//! The evidence (`docs/decisions.md`): on the Goth house five controllers
//! reach a one-button dialog in the first tick and yield; the run of the
//! original answered each, and in the next tick each took the true
//! branch. Object 15 then made a two-button dialog, answered with the
//! second button, and took the false branch the tick after. The rest of
//! the protocol follows the original's code and the old prototype's
//! `Protocol::step`.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_sim::Dialog;

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
        let make = |records: &mut Vec<Dialog>| {
            records.push(Dialog {
                caller: me,
                operands: *operands,
                status: NEW,
            });
        };
        if informs(operands) {
            make(records);
            return Ok(Flow::Branch(true));
        }
        let Some(index) = found else {
            make(records);
            return Ok(Flow::Yield);
        };
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
