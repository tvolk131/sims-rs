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
//! - The ambience's view, the share of the screen's tiles that are
//!   outdoors, which the original works out from the camera each frame
//!   (0x570c3e, `sims_audio::ambience::view`; the game sends
//!   `Game::ambience_view`).
//!
//! The game pauses the sound as the original's does (its mode handler,
//! 0x4ce470): it sends `sims_audio::SoundEvent::Pause` (in
//! [`Input::Events`]) when the game pauses in live mode and when it
//! leaves live mode for another (buy, build, ...), and
//! `SoundEvent::Unpause` when it is back in live mode and not paused.
//! Sounds playing then are held where they are; sounds started while
//! paused play. `SoundEvent::StopObject` is what to send when an object
//! goes, and `SoundEvent::StopAll` stops everything at once.
//!
//! What the sound scripts ask about an object (`getsrcdatafield`), and
//! where each object is, are read from the lot between ticks
//! ([`sources`]) and sent as a [`Snapshot`]. Where an object is sets the
//! volume and pan of the sounds started for it (kinds 1 and 30;
//! `sims_audio::placement`). The original works them out from the
//! object's place on the screen only for a track whose `Spl` register is
//! not 0, and no script in the install sets it: every object on the lot
//! plays at full volume in the centre, whatever the camera. So the game
//! sends no screen positions (`sims_audio::player::Sources::projection`
//! is left unanswered).

use sims_audio::SoundEvent;
use sims_audio::files::SoundFiles;
use sims_audio::module::Module;
use sims_audio::player::Player;
use sims_audio::sources::{FIELDS, NO_ROOM, Snapshot};
use sims_audio::stations::StationLists;
use sims_core::F32;
use sims_sim::World;
use sims_ui::command::Speed;
use std::collections::BTreeSet;
use std::path::Path;

/// The register the sound scripts read the game's speed from.
const SPEED_REGISTER: u8 = 100;

/// The most distinct faults reported. Later ones are dropped unseen, so
/// a long game's missing sounds do not grow without end.
const REPORTED: usize = 256;

/// The player's seed. The sound's randomness never reaches the
/// simulation, so any fixed seed does.
const SEED: u64 = 1;

/// Person data words the object query reads (0x570e61).
const FLAGS: usize = 74;
const AGE: usize = 58;
const GENDER: usize = 65;

/// Object data word 29: the room the object was last put in.
const ROOM: usize = 29;

/// The motive field 14 reads.
const MOTIVE: usize = 3;

/// The single 0.1 at 0x669420 that field 14 scales the motive by.
const TENTH: F32 = F32::from_bits(0x3dcc_cccd);

/// The person data word fields 2, 3, 5, 6, 7, 11, 12 and 15 to 20 read,
/// each divided by 10.
const TENTHS: [(usize, usize); 13] = [
    (2, 10),
    (3, 9),
    (5, 15),
    (6, 11),
    (7, 12),
    (11, 17),
    (12, 18),
    (15, 2),
    (16, 3),
    (17, 4),
    (18, 5),
    (19, 6),
    (20, 7),
];

/// Field 14's answer for a motive: the motive times the single 0.1 on
/// the FPU, cut toward zero (`__ftol`).
pub fn tenth(motive: f32) -> i32 {
    (F32::from_f32(motive).ext() * TENTH.ext()).to_i32_truncate()
}

/// The original's answers to `getsrcdatafield` for every object on the
/// lot (0x570e61; `sims_audio::sources` lists them). A word the object
/// does not have reads as 0.
pub fn sources(world: &World) -> Snapshot {
    let hour = i32::from(world.globals.first().copied().unwrap_or(0));
    // 0x466fcc then 0x46810a: the room's half tiles over 2, as `idiv`.
    let room_tiles = |room: Option<i16>| {
        room.and_then(|id| world.rooms.rooms.get(&id))
            .map_or(NO_ROOM, |room| room.half_tiles / 2)
    };
    let mut absent = [0; FIELDS];
    absent[4] = hour;
    absent[13] = room_tiles(None);
    let objects = world
        .objects
        .values()
        .map(|object| {
            let mut answers = [0; FIELDS];
            answers[4] = hour;
            answers[13] = room_tiles(object.data.get(ROOM).copied());
            if let Some(person) = &object.person {
                let word = |i: usize| i32::from(person.data.get(i).copied().unwrap_or(0));
                answers[0] = if word(FLAGS) & 0b1100 != 0 {
                    1000
                } else if word(AGE) < 12 {
                    2
                } else {
                    word(GENDER)
                };
                answers[1] = word(AGE);
                for (field, i) in TENTHS {
                    answers[field] = word(i) / 10;
                }
                answers[14] = tenth(person.motives.get(MOTIVE).copied().unwrap_or(0.0));
            }
            (i32::from(object.id), answers)
        })
        .filter(|(id, _)| *id > 0)
        .collect();
    let positions = world
        .objects
        .values()
        .map(|object| (i32::from(object.id), object.position))
        .filter(|(id, _)| *id > 0)
        .collect();
    Snapshot {
        objects,
        absent,
        positions,
    }
}

/// The sound scripts' speed register for the game's speed (unverified).
pub fn speed_register(speed: Speed) -> i32 {
    match speed {
        Speed::Normal => 1,
        Speed::Fast => 2,
        Speed::Ultra => 3,
    }
}

/// The sound player's event for one of the simulation's. The
/// simulation's object ID is 16 bits, as the script named it; it is
/// widened without its sign, so -1 is 65535 (which the original passes is
/// **unverified**).
pub fn event(event: sims_vm::SoundEvent) -> SoundEvent {
    let widen = |object: i16| u32::from(object as u16);
    match event {
        sims_vm::SoundEvent::Play { object, name } => SoundEvent::Play {
            object: widen(object),
            name,
        },
        sims_vm::SoundEvent::Stop { object } => SoundEvent::StopObject {
            object: widen(object),
        },
    }
}

/// What the mixer is told between blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Events from a tick, in order.
    Events(Vec<SoundEvent>),
    /// The hour of the game's clock (global 0), for the ambience.
    Hour(u32),
    /// The ambience's camera value (see the module documentation).
    View(i32),
    Speed(Speed),
    /// What the sound scripts may ask about the lot's objects.
    Sources(Snapshot),
}

/// The sound player with every module, station and ambient score of an
/// install.
pub struct Mixer {
    player: Player<SoundFiles>,
    hour: u32,
    view: i32,
    sources: Snapshot,
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
            view: 0,
            sources: Snapshot::default(),
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
            Input::View(view) => self.view = view,
            Input::Sources(sources) => self.sources = sources,
            Input::Speed(speed) => self
                .player
                .set_global(SPEED_REGISTER, speed_register(speed)),
        }
    }

    /// Renders `frames` stereo frames at [`sims_audio::player::RATE`],
    /// moving the ambience's cursor first, as the original does once a
    /// frame.
    pub fn render(&mut self, frames: usize) -> Vec<i16> {
        self.player.set_ambience_view(self.view, self.hour);
        self.player.render_stereo(frames, &self.sources)
    }

    /// Faults not reported yet, each once, up to `REPORTED` of them; a
    /// last line says when more are no longer shown. The player's list is
    /// emptied each time, so neither it nor this one grows without end.
    pub fn new_faults(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for fault in self.player.faults.drain(..) {
            if self.reported.len() >= REPORTED {
                continue;
            }
            if self.reported.insert(fault.clone()) {
                out.push(fault);
                if self.reported.len() == REPORTED {
                    out.push("further sound faults are not shown".into());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_motive_is_scaled_on_the_fpu_not_in_single_precision() {
        assert_eq!(tenth(81.57505), 8);
        assert_eq!(tenth(-81.57505), -8);
        assert_eq!(tenth(f32::NAN), 0);
        // 167772176 times 0.1 is 16777217.6 on the FPU; a single
        // precision division would round it to 16777218 first.
        assert_eq!(tenth(f32::from_bits(0x4d20_0001)), 16_777_217);
    }

    #[test]
    fn a_requests_object_is_widened_without_its_sign() {
        let play = event(sims_vm::SoundEvent::Play {
            object: -1,
            name: "door_open".into(),
        });
        assert!(
            matches!(&play, SoundEvent::Play { object: 65535, name } if name == "door_open"),
            "{play:?}"
        );
        let stop = event(sims_vm::SoundEvent::Stop { object: -32768 });
        assert!(
            matches!(stop, SoundEvent::StopObject { object: 32768 }),
            "{stop:?}"
        );
        let stop = event(sims_vm::SoundEvent::Stop { object: 12 });
        assert!(
            matches!(stop, SoundEvent::StopObject { object: 12 }),
            "{stop:?}"
        );
    }
}
