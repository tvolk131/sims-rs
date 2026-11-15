//! When the lot ticks: the game's speed and pause, turned into a count of
//! ticks owed for the time that has passed.
//!
//! The host owns the clock and passes in the microseconds since the last
//! call, so this is tested without a wall clock. Nothing here reaches the
//! simulation: it decides only how often [`sims_vm::boot::Lot::step`] is
//! called.
//!
//! At normal speed a tick is 50 milliseconds, the step the animator clock
//! moves by each tick (`docs/decisions.md`, "people-viewer"). The fast
//! and ultra periods are **unverified**: three and ten times normal speed
//! are this crate's choice until the original's are measured.

use sims_ui::command::Speed;

/// Microseconds of one tick at normal speed.
pub const NORMAL_US: u64 = 50_000;

/// The most ticks one call owes. Time beyond that is dropped, so a slow
/// tick, a stall or a long pause in the window never makes the lot race
/// to catch up; the lot then runs slower than the clock.
pub const CATCH_UP: u32 = 20;

/// Microseconds of one tick at `speed`.
pub fn period_us(speed: Speed) -> u64 {
    match speed {
        Speed::Normal => NORMAL_US,
        Speed::Fast => NORMAL_US / 3,
        Speed::Ultra => NORMAL_US / 10,
    }
}

/// The speed and pause the player chose, and the time owed to the lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    pub speed: Speed,
    pub paused: bool,
    /// Microseconds that passed and are not yet a tick.
    owed: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            speed: Speed::Normal,
            paused: false,
            owed: 0,
        }
    }
}

impl Pace {
    /// Sets the speed. A paused game stays paused, as the original's
    /// speed buttons leave the pause button alone (**unverified**).
    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
    }

    /// Pauses, or runs again. Time paused is never owed.
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.owed = 0;
    }

    /// The ticks owed after `elapsed_us` more microseconds, at most
    /// [`CATCH_UP`]. The remainder carries to the next call.
    pub fn due(&mut self, elapsed_us: u64) -> u32 {
        if self.paused {
            self.owed = 0;
            return 0;
        }
        let period = period_us(self.speed);
        self.owed = self.owed.saturating_add(elapsed_us);
        let ticks = self.owed / period;
        if ticks > u64::from(CATCH_UP) {
            self.owed = 0;
            return CATCH_UP;
        }
        self.owed -= ticks * period;
        ticks as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_speed_is_a_tick_every_fifty_milliseconds() {
        let mut pace = Pace::default();
        assert_eq!(pace.due(49_999), 0);
        assert_eq!(pace.due(1), 1);
        assert_eq!(pace.due(125_000), 2);
        // The 25 ms left over carries on.
        assert_eq!(pace.due(25_000), 1);
    }

    #[test]
    fn faster_speeds_owe_more_ticks() {
        let mut pace = Pace::default();
        pace.set_speed(Speed::Fast);
        assert_eq!(pace.due(100_000), 6);
        pace.set_speed(Speed::Ultra);
        assert_eq!(pace.due(100_000), 20);
    }

    #[test]
    fn a_stall_is_capped_and_a_pause_owes_nothing() {
        let mut pace = Pace::default();
        assert_eq!(pace.due(60_000_000), CATCH_UP);
        assert_eq!(pace.due(0), 0);
        pace.toggle_pause();
        assert_eq!(pace.due(1_000_000), 0);
        pace.toggle_pause();
        assert_eq!(pace.due(49_000), 0);
        assert_eq!(pace.due(1_000), 1);
    }
}
