//! The game's sound: the simulation's sound events, played by
//! `sims-audio`'s player, with the ambience following the clock.
//!
//! [`Mixer`] has no device, so it is tested headless; [`crate::output`]
//! plays what it renders. Nothing here reaches the simulation.
//!
//! Two of the player's inputs come from the game, not the simulation:
//!
//! - Global register 100, which the sound scripts read as the game's
//!   speed (`SimSpeed`). Which value the original gives at each speed is
//!   **unverified**: normal is 1, as `sims-audio`'s `play` example uses,
//!   fast 2 and ultra 3.
//! - The ambience's view, a value the original works out from the camera
//!   (0x570c3e, not read further). It is [`VIEW`] here, the value that
//!   plays the backyard ambience in the `play` example: **unverified**.

use sims_audio::SoundEvent;
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::{NoSources, Player};
use sims_audio::stations::StationLists;
use sims_ui::command::Speed;
use std::collections::BTreeSet;
use std::path::Path;

/// The register the sound scripts read the game's speed from.
const SPEED_REGISTER: u8 = 100;

/// The ambience's view input (see the module documentation).
pub const VIEW: i32 = 50;

/// The player's seed. The sound's randomness never reaches the
/// simulation, so any fixed seed does.
const SEED: u64 = 1;

/// The sound scripts' speed register for the game's speed (unverified).
pub fn speed_register(speed: Speed) -> i32 {
    match speed {
        Speed::Normal => 1,
        Speed::Fast => 2,
        Speed::Ultra => 3,
    }
}

/// What the mixer is told between blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Events from a tick, in order.
    Events(Vec<SoundEvent>),
    /// The hour of the game's clock (global 0), for the ambience.
    Hour(u32),
    Speed(Speed),
}

/// The sound player with every module, station and ambient score of an
/// install.
pub struct Mixer {
    player: Player<SoundFiles>,
    hour: u32,
    /// Faults already reported, so each is reported once.
    reported: BTreeSet<String>,
}

impl Mixer {
    /// Reads the install's sound files at `root`.
    pub fn open(root: &Path) -> Result<Self, String> {
        let files = SoundFiles::scan(root).map_err(|e| format!("sound files: {e}"))?;
        let modules = Module::load_all(&files).map_err(|e| format!("sound modules: {e}"))?;
        let lists = StationLists::build(&files);
        let mut player = Player::new(modules, files.clone(), SEED);
        player.add_stations(&lists);
        player.add_scores(&files);
        player.set_global(SPEED_REGISTER, speed_register(Speed::Normal));
        Ok(Self {
            player,
            hour: u32::MAX,
            reported: BTreeSet::new(),
        })
    }

    /// Takes one input.
    pub fn input(&mut self, input: Input) {
        match input {
            Input::Events(events) => {
                for event in &events {
                    self.player.handle(event);
                }
            }
            Input::Hour(hour) => self.hour = hour,
            Input::Speed(speed) => self
                .player
                .set_global(SPEED_REGISTER, speed_register(speed)),
        }
    }

    /// Renders `frames` stereo frames at [`sims_audio::player::RATE`],
    /// moving the ambience's cursor first, as the original does once a
    /// frame.
    pub fn render(&mut self, frames: usize) -> Vec<i16> {
        self.player.set_ambience_view(VIEW, self.hour);
        self.player.render_stereo(frames, &NoSources)
    }

    /// Faults not reported yet, each once. The player's list is emptied,
    /// so it does not grow over a long game.
    pub fn new_faults(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for fault in self.player.faults.drain(..) {
            if self.reported.insert(fault.clone()) {
                out.push(fault);
            }
        }
        out
    }
}
