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
//! On a residential lot the idle loop also runs the game at a speed of
//! its own while nobody holds it to the player's ([`World::idle_speed`]):
//! 1000, a period of 1 millisecond, while everyone is asleep or away.
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

/// The speed the idle loop runs the game at while nobody holds it
/// (`0x46d9d6` pushes 1000).
pub const IDLE: i16 = 1000;

/// Object data word 34 (`+0x8e`): nonzero, the person is hidden.
const HIDDEN: usize = 34;
/// Person data word 32 (`+0x5cc`): nonzero, the person does not live on
/// the lot.
const VISITOR: usize = 32;
/// Person data word 65 (`+0x60e`).
const WORD_65: usize = 65;
/// Motive 11 (`+0x7b8`; motives at `+0x78c`): below 0, the person does
/// not hold the speed (`0x45edc5`). Which motive it is was not looked up.
const MOTIVE_11: usize = 11;

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

    /// Whether someone on the lot holds the game to the player's speed
    /// (`0x442f9b`): a person in the registry's list, in order, whose
    /// object data word 34 is 0 (not hidden), whose person data word 65
    /// is at most 1, whose motive 11 is not below 0 (`0x45edc5`: a NaN
    /// counts as below, as the x87 compare branches) and whose person data
    /// word 32 is 0 (they live on the lot).
    pub fn speed_held(&self) -> bool {
        self.people.iter().any(|id| {
            let Some(object) = self.objects.get(id) else {
                return false;
            };
            let Some(person) = object.person.as_ref() else {
                return false;
            };
            object.data.get(HIDDEN) == Some(&0)
                && person.data.get(WORD_65).is_some_and(|&w| w <= 1)
                && person.motives[MOTIVE_11] >= 0.0
                && person.data.get(VISITOR) == Some(&0)
        })
    }

    /// The speed the idle loop sets of its own before it runs an update
    /// (the head of `0x46d9d6`), on a `residential` lot (`0x40745f`: the
    /// lot's zoning is 1, `0x42b9e3`); on any other it does nothing.
    ///
    /// - While nobody holds the speed ([`World::speed_held`]) and global
    ///   18 is negative, the game runs at [`IDLE`] (`0x46d790`): the
    ///   setter makes the speed 1000, then global 18 keeps the speed it
    ///   was. While global 18 is not negative this does nothing, so a
    ///   speed the player sets meanwhile stands.
    /// - While someone holds it and global 18 is not negative, the game
    ///   goes back (`0x46d7db`): the setter makes the speed global 18's,
    ///   then global 18 becomes -1.
    ///
    /// Both also restart the idle loop's clocks and send message 0x109
    /// (with 1 going to 1000, 0 going back) to the interface, which is
    /// not the simulation's. Returns whether the speed was set.
    pub fn idle_speed(&mut self, residential: bool) -> bool {
        if !residential {
            return false;
        }
        let back = self.globals.get(RETURN_SPEED).copied().unwrap_or(-1);
        let held = self.speed_held();
        let (speed, kept) = match (held, back >= 0) {
            (false, false) => (IDLE, self.speed()),
            (true, true) => (back, -1),
            _ => return false,
        };
        self.set_speed(i32::from(speed));
        if let Some(slot) = self.globals.get_mut(RETURN_SPEED) {
            *slot = kept;
        }
        true
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

    /// A world whose registry lists one person, object 5, who holds the
    /// speed, with the speed at normal and global 18 at -1.
    fn one_person() -> World {
        use sims_formats::saved_objects::{Interaction, Person};
        let idle = Interaction {
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let person = Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: vec![0; 256],
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        };
        let object = crate::Object {
            id: 5,
            guid: 0,
            footprint: [0; 4],
            position: [0; 3],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data: vec![0; 128],
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: Some(person),
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        };
        let mut world = World::empty();
        world.objects.insert(5, object);
        world.people.push(5);
        world.globals[SPEED] = NORMAL;
        world.globals[RETURN_SPEED] = -1;
        world
    }

    fn person(world: &mut World) -> &mut sims_formats::saved_objects::Person {
        world.objects.get_mut(&5).unwrap().person.as_mut().unwrap()
    }

    #[test]
    fn each_of_four_words_lets_a_person_go_from_holding_the_speed() {
        assert!(one_person().speed_held());
        let changes: [fn(&mut World); 6] = [
            |w| w.objects.get_mut(&5).unwrap().data[HIDDEN] = 1,
            |w| person(w).data[WORD_65] = 2,
            |w| person(w).motives[MOTIVE_11] = -0.5,
            |w| person(w).motives[MOTIVE_11] = f32::NAN,
            |w| person(w).data[VISITOR] = 1,
            |w| w.people.clear(),
        ];
        for (i, change) in changes.iter().enumerate() {
            let mut world = one_person();
            change(&mut world);
            assert!(!world.speed_held(), "change {i}");
        }
        // Word 65 at 1 or below, or motive 11 at 0, still holds it.
        let mut world = one_person();
        person(&mut world).data[WORD_65] = -3;
        person(&mut world).motives[MOTIVE_11] = 0.0;
        assert!(world.speed_held());
    }

    #[test]
    fn with_nobody_holding_it_the_game_runs_at_1000_until_someone_does() {
        let mut world = one_person();
        // Not residential: nothing.
        person(&mut world).data[VISITOR] = 1;
        assert!(!world.idle_speed(false));
        assert_eq!((world.speed(), world.globals[RETURN_SPEED]), (NORMAL, -1));
        // Nobody holds it: 1000, and normal kept to go back to, once.
        assert!(world.idle_speed(true));
        assert_eq!((world.speed(), world.globals[RETURN_SPEED]), (1000, NORMAL));
        assert!(!world.idle_speed(true));
        assert_eq!(world.speed(), 1000);
        // A speed set meanwhile updates the one to go back to and stands.
        world.set_speed(i32::from(FAST));
        assert!(!world.idle_speed(true));
        assert_eq!((world.speed(), world.globals[RETURN_SPEED]), (FAST, FAST));
        // Someone holds it again: back to it, and global 18 is -1.
        person(&mut world).data[VISITOR] = 0;
        assert!(world.idle_speed(true));
        assert_eq!((world.speed(), world.globals[RETURN_SPEED]), (FAST, -1));
        assert!(!world.idle_speed(true));
        assert_eq!(world.speed(), FAST);
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
