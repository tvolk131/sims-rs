//! When the lot ticks: the game's speed, turned into a count of ticks
//! owed for the time that has passed.
//!
//! The host owns the clock and passes in the microseconds since the last
//! call, so this is tested without a wall clock. Nothing here reaches the
//! simulation: it decides only how often [`sims_vm::boot::Lot::step`] is
//! called. The speed itself reaches the simulation through
//! [`sims_vm::boot::Lot::set_speed`], which the game calls with the same
//! speed.
//!
//! The time between two ticks is the original's, from its speed setter
//! (`0x46d697`, [`sims_sim::speed::period_ms`]): 42 milliseconds at
//! normal speed, 14 at fast and 2 at ultra.
//!
//! Pausing does not stop the ticks. The original goes on updating a
//! paused lot at the speed's pace, each pass a partial one in which the
//! clock stands (global 17, [`sims_sim::speed`]), so ticks are owed while
//! paused too.

use sims_sim::speed;
use sims_ui::command::Speed;

/// The most ticks one call owes. Time beyond that is dropped, so a slow
/// tick, a stall or a long pause in the window never makes the lot race
/// to catch up; the lot then runs slower than the clock. The original
/// keeps a cap of its own (`0x4732de`), whose size is not ported
/// (**unverified**).
pub const CATCH_UP: u32 = 20;

/// The simulation's speed (global 16) the panel's button for `speed`
/// sets.
pub fn level(speed: Speed) -> i16 {
    match speed {
        Speed::Normal => speed::NORMAL,
        Speed::Fast => speed::FAST,
        Speed::Ultra => speed::ULTRA,
    }
}

/// Microseconds of one tick at `speed`.
pub fn period_us(speed: Speed) -> u64 {
    u64::from(speed::period_ms(level(speed))) * 1000
}

/// The speed the player chose, and the time owed to the lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    pub speed: Speed,
    /// Microseconds that passed and are not yet a tick.
    owed: u64,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            speed: Speed::Normal,
            owed: 0,
        }
    }
}

impl Pace {
    /// Sets the speed. The time owed starts again, as the original's
    /// setter restarts its clocks (`0x46d697`).
    pub fn set_speed(&mut self, speed: Speed) {
        self.speed = speed;
        self.owed = 0;
    }

    /// Starts the time owed again, as running a paused game does
    /// (`0x46d83a`).
    pub fn restart(&mut self) {
        self.owed = 0;
    }

    /// The ticks owed after `elapsed_us` more microseconds, at most
    /// [`CATCH_UP`]. The remainder carries to the next call.
    pub fn due(&mut self, elapsed_us: u64) -> u32 {
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
    fn normal_speed_is_a_tick_every_42_milliseconds() {
        let mut pace = Pace::default();
        assert_eq!(pace.due(41_999), 0);
        assert_eq!(pace.due(1), 1);
        assert_eq!(pace.due(105_000), 2);
        // The 21 ms left over carries on.
        assert_eq!(pace.due(21_000), 1);
    }

    #[test]
    fn faster_speeds_owe_more_ticks() {
        let mut pace = Pace::default();
        pace.set_speed(Speed::Fast);
        assert_eq!(pace.due(100_000), 7);
        pace.set_speed(Speed::Ultra);
        assert_eq!(pace.due(30_000), 15);
        assert_eq!(pace.due(100_000), CATCH_UP);
    }

    #[test]
    fn a_stall_is_capped_and_a_new_speed_starts_the_time_again() {
        let mut pace = Pace::default();
        assert_eq!(pace.due(60_000_000), CATCH_UP);
        assert_eq!(pace.due(0), 0);
        assert_eq!(pace.due(41_000), 0);
        pace.set_speed(Speed::Normal);
        assert_eq!(pace.due(41_000), 0);
        pace.restart();
        assert_eq!(pace.due(41_000), 0);
        assert_eq!(pace.due(1_000), 1);
    }
}
