//! The game's speed and pause, as the simulation sees them: simulator
//! globals 16, 17 and 18.
//!
//! The speed is a number from 0 to 1000 (global 16). The setter
//! (`0x46d697`) clamps it, keeps it, and makes the time between two lot
//! updates from it: 172 − speed × 0.001 × 171 milliseconds, rounded to
//! the nearest (0.5 added, then truncated). The panel's buttons set 760
//! (normal, `0x4425a2`), 924 (fast, `0x46d264`) and 994 (ultra,
//! `0x46d26a`): 42, 14 and 2 milliseconds. Global 18 holds the speed to
//! go back to while the game runs at a speed of its own for a while
//! (`0x46d790` sets it, `0x46d7db` goes back and sets it to −1); while it
//! is not negative the setter updates it too.
//!
//! Pausing (`0x46d813`) sets global 17 and running again (`0x46d83a`)
//! clears it. The game goes on updating the lot while paused, at the
//! speed's pace (the idle loop at `0x46d9d6` does not look at the pause),
//! but each pass is a partial one ([`World::paused`]): the clock does not
//! move and only objects that run while paused are ticked.
//!
//! What the setter does besides is not the simulation's: it restarts the
//! idle loop's two clocks (`+0xb8`, `+0xbc` and `0x4732d4`), keeps two
//! numbers made from the period (`+0xd8`, `+0xdc`) and sends message
//! 0xc2 to the interface, as pausing and running again do.

use crate::World;

/// The speed of the panel's normal-speed button (`0x4425a2`).
pub const NORMAL: i16 = 760;
/// The speed of the panel's fast button (`0x46d264`).
pub const FAST: i16 = 924;
/// The speed of the panel's ultra button (`0x46d26a`).
pub const ULTRA: i16 = 994;

const SPEED: usize = 16;
const PAUSED: usize = 17;
const RETURN_SPEED: usize = 18;

/// Milliseconds between two lot updates at `speed` (0 to 1000), as the
/// setter (`0x46d697`) makes them, in the x87's precision: the factor
/// 0.001 is the single nearest it.
pub fn period_ms(speed: i16) -> u32 {
    let speed = f64::from(speed.clamp(0, 1000));
    let thousandth = f64::from(0.001f32);
    let ms = 172.0 - speed * thousandth * 171.0 + 0.5;
    // Truncated, as 0x636128 does; never below 0 for a clamped speed.
    ms as u32
}

impl World {
    /// The game's speed (global 16).
    pub fn speed(&self) -> i16 {
        self.globals.get(SPEED).copied().unwrap_or(0)
    }

    /// Sets the game's speed as its setter does (`0x46d697`): clamped to
    /// 0 to 1000, into global 16, and into global 18 while that is not
    /// negative. Returns the milliseconds between two lot updates.
    pub fn set_speed(&mut self, speed: i32) -> u32 {
        let speed = speed.clamp(0, 1000) as i16;
        if let Some(slot) = self.globals.get_mut(SPEED) {
            *slot = speed;
        }
        if let Some(slot) = self.globals.get_mut(RETURN_SPEED)
            && *slot >= 0
        {
            *slot = speed;
        }
        period_ms(speed)
    }

    /// Whether the game is paused (global 17): the scheduler's passes are
    /// partial ones, and the clock stands.
    pub fn paused(&self) -> bool {
        self.globals.get(PAUSED).is_some_and(|&g| g != 0)
    }

    /// Pauses the game (`0x46d813`): global 17 becomes 1 if it was 0.
    /// The original first checks a field of the interface
    /// (`[0x6f2a58]+0x7c`), which refuses the pause while it is not 0;
    /// what that field is was not looked for, so here it never refuses
    /// (**unverified**). Returns whether the game was paused by it.
    pub fn pause(&mut self) -> bool {
        match self.globals.get_mut(PAUSED) {
            Some(slot) if *slot == 0 => {
                *slot = 1;
                true
            }
            _ => false,
        }
    }

    /// Runs the game again (`0x46d83a`): global 17 becomes 0. Returns
    /// whether it was paused.
    pub fn unpause(&mut self) -> bool {
        match self.globals.get_mut(PAUSED) {
            Some(slot) if *slot != 0 => {
                *slot = 0;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_speeds_make_the_originals_periods() {
        assert_eq!(period_ms(NORMAL), 42);
        assert_eq!(period_ms(FAST), 14);
        assert_eq!(period_ms(ULTRA), 2);
        assert_eq!(period_ms(0), 172);
        assert_eq!(period_ms(1000), 1);
        assert_eq!(period_ms(-5), 172);
    }

    #[test]
    fn the_setter_clamps_and_updates_the_speed_to_go_back_to() {
        let mut world = World::empty();
        world.globals[RETURN_SPEED] = -1;
        assert_eq!(world.set_speed(2000), 1);
        assert_eq!(world.speed(), 1000);
        assert_eq!(world.globals[RETURN_SPEED], -1);
        world.globals[RETURN_SPEED] = 760;
        assert_eq!(world.set_speed(-3), 172);
        assert_eq!(world.speed(), 0);
        assert_eq!(world.globals[RETURN_SPEED], 0);
    }

    #[test]
    fn pausing_sets_global_17_once() {
        let mut world = World::empty();
        assert!(!world.paused());
        assert!(world.pause());
        assert!(!world.pause());
        assert!(world.paused());
        assert_eq!(world.globals[PAUSED], 1);
        assert!(world.unpause());
        assert!(!world.unpause());
        assert!(!world.paused());
    }
}
