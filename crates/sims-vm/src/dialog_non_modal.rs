//! Dialogs that do not stop the game (Dialog operand 7 bit 7 set), shown
//! and answered while the lot runs.
//!
//! The original shows such a dialog at the end of the lot update
//! (`0x436da0`): the record's status becomes 1 (shown), the widget is kept
//! (byte `0x69` clear), and the game goes on. The player answers it
//! between updates, through its dismiss box or its 40 s timer (`0x4370be`
//! sets the record's status to the button's, or to done for a dialog that
//! only informs). Until then, the caller's next `Dialog` closes the shown
//! record (`0x437058`) and makes another: a choice is shown again every
//! tick its caller runs, and one that only informs stays until it is
//! answered or its caller shows another.
//!
//! [`Lot::present_non_modal_dialogs`] turns this on for play mode
//! ([`Lot::step_play`]). Off (the default), play mode answers such a
//! dialog at once as before, and strict mode ([`Lot::step`]) still stops
//! at one: answering one while the game runs is not something a recorded
//! run of the original can be compared against. Co-op answers it at once
//! either way: no command carries such an answer yet.

use crate::boot::{Lot, OrderError};
use crate::primitives::dialog::{self, DONE, SHOWN};
use sims_sim::Dialog;

/// A dialog that does not stop the game, shown and waiting for the
/// player ([`Lot::non_modal_dialogs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonModalDialog {
    /// The object that asked.
    pub caller: u16,
    /// The Dialog instruction's operands.
    pub operands: [u8; 8],
    /// The statuses its buttons answer with, first button first
    /// (`sims_vm::Shown::answers`).
    pub answers: &'static [u8],
    /// Whether it only informs (operand 7 bit 0): its caller has gone
    /// on, and an answer only closes it.
    pub informs: bool,
}

/// Why [`Lot::answer_non_modal`] took no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonModalAnswerError {
    /// The lot takes no answer now ([`Lot::queue_interaction`]'s reasons:
    /// halted, waiting on a modal dialog, not loaded).
    NotReady(OrderError),
    /// `caller` shows no dialog that does not stop the game.
    NotShown { caller: u16 },
    /// Co-op is attached: an answer changed on one peer alone would part
    /// the peers' worlds, and no command carries one.
    Coop,
    /// The dialog does not offer that answer.
    NotOffered { answer: u8, offered: &'static [u8] },
}

impl std::fmt::Display for NonModalAnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady(error) => write!(f, "{error}"),
            Self::NotShown { caller } => {
                write!(
                    f,
                    "object {caller} shows no dialog that lets the game go on"
                )
            }
            Self::NotOffered { answer, offered } => {
                write!(f, "the dialog offers {offered:?}, not {answer}")
            }
            Self::Coop => write!(
                f,
                "a dialog that lets the game go on is not answered in co-op"
            ),
        }
    }
}

impl std::error::Error for NonModalAnswerError {}

impl Lot {
    /// Whether play mode leaves a dialog that does not stop the game shown
    /// for the player to answer while the lot runs, as the original does,
    /// instead of answering it at once (see the module's doc). Off by
    /// default. A setting of the lot's host
    /// ([`crate::SpecialPanels::present_non_modal`]): no save holds it, so
    /// a restore or a trip turns it off, and a host sets it again before
    /// it steps. Strict mode and co-op are unchanged by it.
    pub fn present_non_modal_dialogs(&mut self, on: bool) {
        self.special_panels.present_non_modal = on;
    }

    /// Whether [`Lot::present_non_modal_dialogs`] is on.
    pub fn presents_non_modal_dialogs(&self) -> bool {
        self.special_panels.present_non_modal
    }

    /// The dialogs that do not stop the game shown now and waiting for
    /// the player, in the registry's order.
    pub fn non_modal_dialogs(&self) -> Vec<NonModalDialog> {
        shown(&self.world.dialogs.records)
    }

    /// Answers the dialog that does not stop the game `caller` shows with
    /// the status `answer` (one of [`NonModalDialog::answers`]), between
    /// steps, as a press of its button or its timer does (`0x4370be`): a
    /// dialog that only informs is done, and a choice takes the answer,
    /// which its caller reads on its next turn.
    ///
    /// For a text-entry dialog (kind 3), `text` is its field's text
    /// (empty when None), which the caller's next `Dialog` copies to the
    /// global it reads (`0x451a6b..0x451a85`); other kinds ignore it.
    ///
    /// Refused while co-op is attached ([`NonModalAnswerError::Coop`]).
    pub fn answer_non_modal(
        &mut self,
        caller: u16,
        answer: u8,
        text: Option<String>,
    ) -> Result<(), NonModalAnswerError> {
        self.ready().map_err(NonModalAnswerError::NotReady)?;
        if self.coop.is_some() {
            return Err(NonModalAnswerError::Coop);
        }
        if answer_record(&mut self.world.dialogs.records, caller, answer)? {
            self.last_dialog_text = text.unwrap_or_default();
        }
        Ok(())
    }
}

/// The records of `records` that do not stop the game, shown and waiting.
fn shown(records: &[Dialog]) -> Vec<NonModalDialog> {
    records
        .iter()
        .filter(|record| record.status == SHOWN && !dialog::modal(&record.operands))
        .filter_map(|record| {
            Some(NonModalDialog {
                caller: record.caller,
                operands: record.operands,
                answers: dialog::answers(record.operands[5])?,
                informs: dialog::informs(&record.operands),
            })
        })
        .collect()
}

/// Answers `caller`'s shown record in `records` with `answer`
/// ([`Lot::answer_non_modal`]). Returns whether it asks for text (kind 3).
fn answer_record(
    records: &mut [Dialog],
    caller: u16,
    answer: u8,
) -> Result<bool, NonModalAnswerError> {
    let found = shown(records)
        .into_iter()
        .find(|d| d.caller == caller)
        .ok_or(NonModalAnswerError::NotShown { caller })?;
    if !found.answers.contains(&answer) {
        return Err(NonModalAnswerError::NotOffered {
            answer,
            offered: found.answers,
        });
    }
    let record = records
        .iter_mut()
        .find(|record| {
            record.caller == caller && record.status == SHOWN && !dialog::modal(&record.operands)
        })
        .ok_or(NonModalAnswerError::NotShown { caller })?;
    record.status = if found.informs { DONE } else { answer };
    Ok(found.operands[5] & 0x7f == 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(caller: u16, kind: u8, flags: u8, status: u8) -> Dialog {
        Dialog {
            caller,
            operands: [0, 0, 0, 0, 0, kind, 0, flags],
            status,
        }
    }

    #[test]
    fn only_shown_non_modal_records_are_listed() {
        let records = [
            record(10, 1, 0x80, SHOWN),
            record(11, 1, 0x00, SHOWN),
            record(12, 0, 0x81, SHOWN),
            record(13, 2, 0x80, 0),
            record(14, 6, 0x80, SHOWN),
        ];
        let listed = shown(&records);
        assert_eq!(
            listed.iter().map(|d| d.caller).collect::<Vec<_>>(),
            [10, 12],
            "modal, new and special-panel records are not listed"
        );
        assert_eq!(listed[0].answers, [2, 3]);
        assert!(!listed[0].informs);
        assert!(listed[1].informs);
    }

    #[test]
    fn an_answer_sets_the_button_or_closes_an_informing_dialog() {
        let mut records = [
            record(10, 2, 0x80, SHOWN),
            record(11, 0, 0x81, SHOWN),
            record(12, 3, 0x80, SHOWN),
            record(13, 1, 0x00, SHOWN),
        ];
        assert_eq!(answer_record(&mut records, 10, 4), Ok(false));
        assert_eq!(records[0].status, 4);
        // Answered, it is no longer shown.
        assert_eq!(
            answer_record(&mut records, 10, 2),
            Err(NonModalAnswerError::NotShown { caller: 10 })
        );
        assert_eq!(answer_record(&mut records, 11, 2), Ok(false));
        assert_eq!(records[1].status, DONE);
        assert_eq!(
            answer_record(&mut records, 12, 3),
            Err(NonModalAnswerError::NotOffered {
                answer: 3,
                offered: &[2]
            })
        );
        assert_eq!(records[2].status, SHOWN);
        assert_eq!(answer_record(&mut records, 12, 2), Ok(true));
        // A modal record is the modal path's (`Lot::answer`).
        assert_eq!(
            answer_record(&mut records, 13, 2),
            Err(NonModalAnswerError::NotShown { caller: 13 })
        );
        assert_eq!(records[3].status, SHOWN);
    }
}
