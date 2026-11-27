//! A simulation dialog's 40 s auto-answer timer.
//!
//! When the original makes an ordinary dialog's widget (`0x436c89`), it
//! sets a timer of 40000 ms on it through `0x523a76`, storing the time at
//! widget +0x11c and the button the timer presses at +0x120: the last
//! one (the second of two, status 3; the third of three, status 4; the
//! only one otherwise, status 2). No timer is set for a dialog of kind 3
//! (text entry) or 4, or for one whose caller owns interface events (the
//! dialog anchored at the window's top right, record +0x6c). The clock is
//! the wall clock (`0x5daf06`, `timeGetTime`), not the simulation's: the
//! timer runs whether or not the game is paused, modal dialog or not.
//!
//! The widget's class (vtable `0x666ddc`) handles it:
//! - +0x118 (`0x5248a8`), on the timer's message (0x1a), presses the
//!   stored button, which answers the dialog as a press of it does
//!   (`0x4370be`).
//! - +0xe4 (`0x5249f9`), on a mouse button going down on the dialog
//!   (message 7), stops the timer.
//! - +0xf4 (`0x524a13`), on the button coming up (message 11), starts
//!   it again with the whole 40 s.
//!
//! A non-modal dialog's caller closes its record and makes it again each
//! tick it runs (`0x451552` → `0x437058`), which makes a new widget, so
//! its timer starts over each time ([`AutoAnswer::arm`] again).

/// How long the timer runs (`0x436c89`: 40000 ms), in microseconds.
pub const AUTO_ANSWER_US: u64 = 40_000_000;

/// The answer the timer of a dialog with the Dialog instruction's
/// `operands` gives, as a status (`sims_vm::Answer::status`): the last
/// button's, or None when no timer is set: a dialog of kind 3 or 4, one
/// anchored at the window's top right, or a special panel (kinds 5 and
/// up, which are not this widget).
pub fn auto_answer_status(operands: &[u8; 8], anchored: bool) -> Option<u8> {
    if anchored {
        return None;
    }
    match operands[5] & 0x7f {
        0 => Some(2),
        1 => Some(3),
        2 => Some(4),
        _ => None,
    }
}

/// One dialog's timer: when it is due, or that a press on the dialog
/// stopped it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoAnswer {
    /// When the timer presses its button, on the host's clock in
    /// microseconds, or None while it does not run.
    deadline_us: Option<u64>,
    /// A press on the dialog stopped the timer; its release starts it
    /// again.
    held: bool,
}

impl AutoAnswer {
    /// Starts the timer at `now_us` with the whole 40 s: a dialog shown,
    /// or shown again.
    pub fn arm(&mut self, now_us: u64) {
        self.deadline_us = Some(now_us.saturating_add(AUTO_ANSWER_US));
        self.held = false;
    }

    /// No timer: no dialog, or one that sets none.
    pub fn disarm(&mut self) {
        *self = Self::default();
    }

    /// A mouse button went down on the dialog (`0x5249f9`): a running
    /// timer stops until the release.
    pub fn press(&mut self) {
        if self.deadline_us.take().is_some() {
            self.held = true;
        }
    }

    /// The button came up (`0x524a13`): a timer the press stopped starts
    /// again with the whole 40 s at `now_us`.
    pub fn release(&mut self, now_us: u64) {
        if std::mem::take(&mut self.held) {
            self.arm(now_us);
        }
    }

    /// Whether the timer runs (it is armed and no press holds it).
    pub fn running(&self) -> bool {
        self.deadline_us.is_some()
    }

    /// How long until the timer runs out at `now_us` (0 once it has), or
    /// None while it does not run: when a host must wake to press it.
    pub fn remaining_us(&self, now_us: u64) -> Option<u64> {
        self.deadline_us
            .map(|deadline| deadline.saturating_sub(now_us))
    }

    /// Whether the timer has run out at `now_us`. Once it has, it does
    /// not run again until armed.
    pub fn take_due(&mut self, now_us: u64) -> bool {
        if self.deadline_us.is_some_and(|deadline| now_us >= deadline) {
            self.disarm();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operands(kind: u8) -> [u8; 8] {
        [0, 0, 0, 0, 0, kind, 0, 0]
    }

    #[test]
    fn the_timer_presses_the_last_button_of_kinds_0_to_2_only() {
        assert_eq!(auto_answer_status(&operands(0), false), Some(2));
        assert_eq!(auto_answer_status(&operands(1), false), Some(3));
        assert_eq!(auto_answer_status(&operands(2), false), Some(4));
        // Operand 5 bit 7 (large) is not part of the kind.
        assert_eq!(auto_answer_status(&operands(0x81), false), Some(3));
        for kind in 3..16 {
            assert_eq!(auto_answer_status(&operands(kind), false), None);
        }
        assert_eq!(auto_answer_status(&operands(1), true), None);
    }

    #[test]
    fn the_timer_runs_40_s_stops_on_a_press_and_restarts_on_its_release() {
        let mut timer = AutoAnswer::default();
        assert!(!timer.take_due(u64::MAX), "an unarmed timer never fires");
        assert_eq!(timer.remaining_us(0), None);
        timer.arm(1_000);
        assert_eq!(timer.remaining_us(1_000), Some(AUTO_ANSWER_US));
        assert_eq!(timer.remaining_us(u64::MAX), Some(0));
        assert!(!timer.take_due(1_000 + AUTO_ANSWER_US - 1));
        // A press stops it, however long it is held.
        timer.press();
        assert!(!timer.running());
        assert!(!timer.take_due(1_000 + 10 * AUTO_ANSWER_US));
        // The release starts the whole 40 s again.
        timer.release(5_000_000);
        assert!(!timer.take_due(5_000_000 + AUTO_ANSWER_US - 1));
        assert!(timer.take_due(5_000_000 + AUTO_ANSWER_US));
        assert!(!timer.take_due(u64::MAX), "it fires once");
        // A release with no press before it starts nothing.
        timer.release(0);
        assert!(!timer.running());
        // Showing the dialog again starts it over.
        timer.arm(0);
        timer.arm(30_000_000);
        assert!(!timer.take_due(AUTO_ANSWER_US));
        assert!(timer.take_due(30_000_000 + AUTO_ANSWER_US));
    }
}
