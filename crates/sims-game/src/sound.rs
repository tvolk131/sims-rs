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
//! The game sends [`Input::Camera`] for its first view and whenever the
//! camera moves: scrolled, zoomed, turned, to another floor, or the
//! window resized (a [`Shot`] that changes). The mixer then does what
//! the original's kind 33 does (`sims_audio::player::Player::camera_moved`):
//! every track is placed again from where its objects are on the moved
//! screen, the ambience's cursor moves, and the day and night loop
//! updates (`0x574978`). When the hour changes (`Input::Hour`, after
//! the initial hour), the mixer also runs kind 33, as the original's
//! clock does at minute zero (`0x46d4d3` → `0x4f66bc`..`0x4f66c6` →
//! `0x57aac5`). The original sends kind 33 from five places in its view
//! and clock code; that the camera moves above cover the other four is
//! **unverified**. The ambience's cursor also follows the latest hour
//! and view before each block the mixer renders.
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
//! where each object is on the lot and on the screen, are read between
//! ticks ([`sources`] and [`screen`]; `Game::sound_sources` gives both)
//! and sent as a [`Snapshot`], again whenever they change, the camera
//! too. Where an object is sets the volume and pan of the sounds started
//! for it (kinds 1 and 30; `sims_audio::placement`): every track's `Spl`
//! register starts at 20 and no script changes it, so the original works
//! them out from the object's place on the screen, the zoom and the floor
//! shown.

use sims_audio::SoundEvent;
use sims_audio::ambience::{AmbienceCatalog, AmbienceMixer, AmbienceZone, ZoomLevel};
use sims_audio::files::SoundFiles;
use sims_audio::levels;
use sims_audio::module::Module;
use sims_audio::music::{ALL_EXPANSIONS, ModeCatalog, ModeMusicPlayer, MusicMode};
use sims_audio::placement::Projection;
use sims_audio::player::{Player, RATE, VOLUME_STEPS};
use sims_audio::radio::{BuiltinStation, RadioCatalog, RadioPlayer, SpatialAttenuation, StationId};
use sims_audio::sources::{FIELDS, NO_ROOM, Snapshot};
use sims_audio::stations::StationLists;
use sims_audio::voices::{VoiceAgeGender, VoiceCatalog, VoicePitch, VoiceStream};
use sims_core::F32;
use sims_render::camera::Camera;
use sims_sim::World;
use sims_ui::command::{Mode, Speed};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

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
        screen: BTreeMap::new(),
        lot: world.globals.get(10).copied().unwrap_or(0),
    }
}

/// Where each object on the lot is on a screen `size` pixels wide and
/// high, through `camera`, as the original's sound code asks the view
/// (0x570e09): the object's fine position rotated into the view about
/// 1023, not the drawing's 1024 (0x47bc1f), then projected on its level
/// with the tile's terrain heights ([`object_point_on_lot`]). Empty for a
/// camera so far off that its arithmetic would overflow, which no view
/// the controls make comes near.
pub fn screen(
    world: &World,
    lot: &sims_render::Lot,
    camera: &Camera,
    size: [i32; 2],
) -> BTreeMap<i32, Projection> {
    if camera.origin.iter().any(|n| n.unsigned_abs() >= 1 << 26) {
        return BTreeMap::new();
    }
    world
        .objects
        .values()
        .filter(|object| object.id > 0)
        .map(|object| {
            let at = object_point_on_lot(camera, lot, object.position);
            let view = Projection {
                at,
                screen: size,
                zoom: camera.zoom.level() as i32,
                shown_level: camera.level,
            };
            (i32::from(object.id), view)
        })
        .collect()
}

/// The active lot's rotated tile bounds `[min_x, max_x, min_y, max_y]`
/// (0x4791a6).
pub(crate) fn lot_bounds(lot: &sims_render::Lot, camera: &Camera) -> [i32; 4] {
    if lot.size == 0 {
        return [0; 4];
    }
    let last = lot.size as i32 - 1;
    let a = camera.rotate_tile([0, 0]);
    let b = camera.rotate_tile([last, last]);
    [
        a[0].min(b[0]),
        a[0].max(b[0]),
        a[1].min(b[1]),
        a[1].max(b[1]),
    ]
}

/// A view tile's four corner terrain heights in view order (0x479567,
/// corner table 0x6c4ebc: left, top, right, bottom). Returns `[0; 4]` if
/// the unrotated tile is outside the lot.
pub(crate) fn raw_tile_corners(
    lot: &sims_render::Lot,
    camera: &Camera,
    view_tile: [i32; 2],
) -> [u8; 4] {
    let [x, y] = camera.unrotate_tile(view_tile);
    if !lot.contains(x, y) {
        return [0; 4];
    }
    let h = lot.heights[sims_render::Lot::cell(x as usize, y as usize)];
    let rotation = usize::from(camera.rotation & 3);
    std::array::from_fn(|n| h[(n + 4 - rotation) & 3])
}

/// A view tile's four corner terrain heights as `0x4a9c50` and `0x4a9dc7`
/// read them: [`raw_tile_corners`] on the lot's inner tiles
/// (`min_x + 1..=max_x - 1`, `min_y + 1..=max_y - 1`), and `[0; 4]` on
/// the border or off the lot.
pub fn tile_corners(lot: &sims_render::Lot, camera: &Camera, view_tile: [i32; 2]) -> [i32; 4] {
    if lot.size < 3 {
        return [0; 4];
    }
    let b = lot_bounds(lot, camera);
    if view_tile[0] <= b[0] || view_tile[0] >= b[1] || view_tile[1] <= b[2] || view_tile[1] >= b[3]
    {
        return [0; 4];
    }
    raw_tile_corners(lot, camera, view_tile).map(i32::from)
}

/// Rotates a fine lot position into view coordinates about 1023, as
/// `0x47bc1f` does with `param_4 = 0x400`.
pub(crate) fn rotate_object_fine(camera: &Camera, [x, y]: [i32; 2]) -> [i32; 2] {
    let m: i32 = 1023;
    match camera.rotation & 3 {
        1 => [m.wrapping_sub(y), x],
        2 => [m.wrapping_sub(x), m.wrapping_sub(y)],
        3 => [y, m.wrapping_sub(x)],
        _ => [x, y],
    }
}

/// The screen point of an object at `position` (fine, fine, level) on flat
/// ground, as the original's view works it out (0x4071a3, 0x47bc1f,
/// 0x4a9dc7): the fine position rotated into the view about 1023, not
/// the drawing's 1024, then projected on its level by [`fine_point`].
pub fn object_point(camera: &Camera, position: [i32; 3]) -> [i32; 2] {
    let [x, y, level] = position;
    fine_point(camera, rotate_object_fine(camera, [x, y]), level)
}

/// The screen point of an object at `position` (fine, fine, level) on
/// `lot`, including the tile's terrain heights (0x4071a3, 0x47bc1f,
/// 0x4a9dc7, 0x479567).
pub fn object_point_on_lot(
    camera: &Camera,
    lot: &sims_render::Lot,
    position: [i32; 3],
) -> [i32; 2] {
    let [x, y, level] = position;
    let rotated = rotate_object_fine(camera, [x, y]);
    let tile = [rotated[0] >> 4, rotated[1] >> 4].map(|n| i32::from(n as i16));
    let corners = tile_corners(lot, camera, tile);
    fine_point_with_heights(camera, rotated, level, corners)
}

/// The screen point of a fine view position on `level` on flat ground, as
/// the original's view works it out (0x4a9dc7): see
/// [`fine_point_with_heights`].
pub fn fine_point(camera: &Camera, fine: [i32; 2], level: i32) -> [i32; 2] {
    fine_point_with_heights(camera, fine, level, [0; 4])
}

/// The screen point of a fine view position on `level`, as the
/// original's view works it out (0x4a9dc7, 0x4a9c50): the tile's top
/// corner is projected at `corners[1]` and cut to 16 bits each way, then
/// each part of the step across the tile (slanted by `corners`, in view
/// order: left, top, right, bottom) is cut toward zero on its own and
/// added in 16 bits. The result can be a pixel off the drawing's
/// [`Camera::project_fine`].
pub fn fine_point_with_heights(
    camera: &Camera,
    [x, y]: [i32; 2],
    level: i32,
    corners: [i32; 4],
) -> [i32; 2] {
    let shift = camera.zoom.level();
    let [tx, ty] = [x >> 4, y >> 4].map(|n| i32::from(n as i16));
    let [fx, fy] = [x & 15, y & 15];
    let [cx, cy] = camera
        .project([tx, ty], level, corners[1])
        .map(|n| n as i16);
    // Down the tile, (top - corner + 2) or (top - bottom + 4) steps of
    // the zoom's half tile, for a part `f` of 16 (0x4a9dc7).
    let down = |steps: i32, f: i32| (steps << (shift + 1)) * f / 16;
    let across = 8 << shift;
    let (dx, dy) = if fx == fy {
        (0, down(corners[1] - corners[3] + 4, fx))
    } else if fx > fy {
        let side = down(corners[1] - corners[2] + 2, fx);
        let diagonal = down(corners[1] - corners[3] + 4, fx);
        let right = across * fx / 16;
        (right - right * fy / fx, (diagonal - side) * fy / fx + side)
    } else {
        let side = down(corners[1] - corners[0] + 2, fy);
        let diagonal = down(corners[1] - corners[3] + 4, fy);
        let left = across * fy / -16;
        (left - left * fx / fy, (diagonal - side) * fx / fy + side)
    };
    [
        i32::from(cx.wrapping_add(dx as i16)),
        i32::from(cy.wrapping_add(dy as i16)),
    ]
}

/// The sound scripts' speed register for the game's speed (unverified).
pub fn speed_register(speed: Speed) -> i32 {
    match speed {
        Speed::Normal => 1,
        Speed::Fast => 2,
        Speed::Ultra => 3,
    }
}

/// The sound events the window sends when opening a residential lot
/// directly into live mode: neighbourhood mode (`bkground_nhood1`,
/// unlocking music mode tracking as entering from the neighbourhood
/// does), backyard ambience (`setambience_backyard`), and live mode
/// (`bkground_live`, `0x4ce6d2`, starting backyard ambience and the day
/// and night loop `0x574978`).
pub fn enter_live() -> Vec<SoundEvent> {
    ["bkground_nhood1", "setambience_backyard", "bkground_live"]
        .into_iter()
        .map(|name| SoundEvent::Play {
            object: 0,
            name: name.into(),
        })
        .collect()
}

/// The sound event for a live-mode pause change (`0x4ce5c5` for pause,
/// `0x4ce5d9` for unpause).
pub fn pause_event(paused: bool) -> SoundEvent {
    if paused {
        SoundEvent::Pause
    } else {
        SoundEvent::Unpause
    }
}

/// Bridges a [`crate::Game`] into a [`Mixer`] (or [`crate::output::Output`]):
/// sends object sources, live-mode entry ([`enter_live`]), live-mode
/// pause/unpause ([`pause_event`], `0x4ce5c5` / `0x4ce5d9`), window
/// focus changes ([`Input::Active`], kind 25 at `0x4f7608` → `0x4f76d5`,
/// with live-mode auto-pause matching `byte [0x6c9fa1] == 1` at
/// `0x4f7635`..`0x4f7666`), the simulation's sound events, the ambience
/// view/hour/speed, and kind 33 camera moves in the order the window uses.
/// Dirty-tracking key for world-side sound source fields (`objects`,
/// `positions`, `absent`, `lot`). Pure UI/hover wakeups leave both this key
/// and [`Shot`] unchanged, so [`Feed::update`] skips rebuilding [`Snapshot`]
/// entirely; pure camera pans change only [`Shot`], reusing `objects` and
/// `positions` from the prior snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SoundWorldKey {
    pub(crate) ticks: u32,
    pub(crate) restore_gen: u64,
    pub(crate) sound_rev: u64,
    pub(crate) error_count: usize,
    pub(crate) object_count: usize,
    pub(crate) hour: i16,
    pub(crate) lot_id: i16,
    pub(crate) buy_held_pos: Option<(u16, [i32; 3])>,
}

pub struct Feed {
    /// The hour, speed and ambience view the sound was last told.
    heard: Option<(u32, Speed, i32)>,
    /// The object answers the sound was last told, shared zero-copy with the
    /// audio mixer thread via `Arc`.
    told: Option<Arc<Snapshot>>,
    /// World-side key of `self.told` for O(1) dirty gating.
    told_world_key: Option<SoundWorldKey>,
    /// Camera shot of `self.told` so pure camera pans reuse `objects`/`positions`.
    told_shot: Option<Shot>,
    /// The camera the sound last heard move.
    shot: Option<Shot>,
    /// The last `game.paused()` value observed from [`crate::Game`], used to
    /// distinguish manual pause toggles from a focus-loss pause request that a
    /// modal dialog refused on the simulation side.
    observed_game_paused: bool,
    /// The requested pause state (`byte [esi + 0x40]` at `0x4cee5d`, read by
    /// `0x4939c0` on focus loss and tested at `0x4ce5b0` to choose kind 21 vs
    /// kind 22 independently of whether the simulation accepted the pause).
    requested_paused: bool,
    /// Whether `0x4cee4a` marked the pause flag dirty (`or [esi + 0xc], 0x800040`,
    /// where `0x800000` is tested at `0x4ce59e` for pause dispatch) so the next
    /// [`Feed::update`] dispatches `pause_event(self.requested_paused)` even
    /// when the requested boolean matches the previous one.
    pause_dirty: bool,
    /// Whether the window is focused (`true` at startup, matching
    /// `0x4eee14` and `0x4f7610` deduplication).
    focused: bool,
    /// The requested pause state saved on focus loss (`[esi + 0xbc]` at
    /// `0x4f7645`..`0x4f764a`), restored on focus regain (`0x4f765c`..`0x4f7666`).
    saved_paused: bool,
    /// Whether the most recent focus loss auto-paused the lot (true when
    /// `Sim In Background` was off at blur time, `[0x6c9fa1] == 1`).
    paused_for_blur: bool,
    /// The last `[fx, music, vox]` volume steps (`0..=10`) sent to the mixer
    /// (matching `Player::new`'s `[10, 10, 10]` startup defaults).
    volumes: [u8; 3],
    /// The last streaming mode-music station sent to the mixer.
    music_mode: Option<MusicMode>,
    /// The last lot environmental ambience zone sent to the mixer.
    ambience_zone: Option<AmbienceZone>,
    /// The last zoom level (`0..=2`) sent to the mixer's ambience attenuator.
    zoom: Option<u32>,
}

impl Default for Feed {
    fn default() -> Self {
        Self {
            heard: None,
            told: None,
            told_world_key: None,
            told_shot: None,
            shot: None,
            observed_game_paused: false,
            requested_paused: false,
            pause_dirty: false,
            focused: true,
            saved_paused: false,
            paused_for_blur: false,
            volumes: [sims_ui::options::VOLUME_MAX; 3],
            music_mode: None,
            ambience_zone: None,
            zoom: None,
        }
    }
}

/// Maps a lot ID to its environmental [`AmbienceZone`].
pub fn ambience_zone_for_lot(lot_id: i16) -> AmbienceZone {
    match lot_id {
        21..=30 => AmbienceZone::Downtown,
        40..=42 => AmbienceZone::VacationBeach,
        43..=45 => AmbienceZone::VacationForest,
        46..=48 => AmbienceZone::VacationSnow,
        81..=89 => AmbienceZone::StudioTown,
        90..=98 => AmbienceZone::MagicTown,
        _ => AmbienceZone::Residential,
    }
}

/// Maps a camera zoom level (`1` = Far, `2` = Medium, `3` = Near) to [`ZoomLevel`].
pub fn zoom_level_from_u32(zoom: u32) -> ZoomLevel {
    ZoomLevel::from_zoom(zoom)
}

/// Returns whether `name` is a native stereo/TV turn-off HIT event.
pub fn is_radio_stop_event(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower == "tv_turn_off" || lower == "stereo_turnoff" || lower.starts_with("stereo_turnoff_")
}

/// Maps a SimAntics/HIT stereo or television play event name to its built-in
/// streaming [`BuiltinStation`], if any.
///
/// Covers both direct station names (`radio_rock`, `tv_comedy`) and Complete
/// Collection 1.91's native HIT event aliases (`tv_action_cheap`, `tv_cartoon_mod`,
/// `stereo_rock_exp`, `stereo_jazz_cheap`, etc.).
pub fn station_for_sound_event(name: &str) -> Option<BuiltinStation> {
    let lower = name.trim().to_ascii_lowercase();
    let base = lower
        .strip_suffix("_cheap")
        .or_else(|| lower.strip_suffix("_mod"))
        .or_else(|| lower.strip_suffix("_exp"))
        .unwrap_or(lower.as_str());
    match base {
        "radio_country" | "stereo_country" => Some(BuiltinStation::Country),
        "radio_rock" | "stereo_rock" => Some(BuiltinStation::Rock),
        "radio_latin" | "stereo_latin" | "radio_jazz" | "stereo_jazz" => {
            Some(BuiltinStation::Latin)
        }
        "radio_classical" | "radio_classica" | "stereo_classical" => {
            Some(BuiltinStation::Classical)
        }
        "radio_countrydance" | "radio_countryd" | "stereo_countrydance" | "stereo_countryd" => {
            Some(BuiltinStation::CountryDance)
        }
        "radio_rap" | "stereo_rap" => Some(BuiltinStation::Rap),
        "radio_rave" | "stereo_rave" => Some(BuiltinStation::Rave),
        "radio_disco" | "stereo_disco" => Some(BuiltinStation::Disco),
        "radio_beach" | "stereo_beach" => Some(BuiltinStation::Beach),
        "radio_ez" | "radio_easylistening" | "stereo_ez" | "stereo_easylistening" => {
            Some(BuiltinStation::EasyListening)
        }
        "radio_ezexp" | "stereo_ezexp" => Some(BuiltinStation::EasyListeningExp),
        "radio_unlcheap" | "radio_oldtown" | "stereo_unlcheap" | "stereo_oldtown" => {
            Some(BuiltinStation::UnleashedCheap)
        }
        "radio_unlspeak" | "radio_zydeco" | "stereo_unlspeak" | "stereo_zydeco" => {
            Some(BuiltinStation::UnleashedSpeakeasy)
        }
        "radio_superstar" | "stereo_superstar" => Some(BuiltinStation::Superstar),
        "tv_romance" | "tv_play_romance" => Some(BuiltinStation::TvRomance),
        "tv_action" | "tv_play_action" => Some(BuiltinStation::TvAction),
        "tv_comedy" | "tv_comedy_cartoon" | "tv_cartoon" | "tv_play_comedy" => {
            Some(BuiltinStation::TvComedy)
        }
        "tv_mystery" | "tv_horror" | "tv_play_mystery" => Some(BuiltinStation::TvMystery),
        "tv_animal" | "tv_play_animal" => Some(BuiltinStation::TvAnimal),
        "tv_entertainment" | "tv_play_entertainment" => Some(BuiltinStation::TvEntertainment),
        "tv_news" | "tv_play_news" => Some(BuiltinStation::TvNews),
        "tv_sports" | "tv_play_sports" => Some(BuiltinStation::TvSports),
        _ => None,
    }
}

/// Maps a live-lot control panel mode and lot ID to its 1.91 [`MusicMode`].
pub fn lot_music_mode(mode: Mode, lot_id: i16) -> MusicMode {
    let magic = (90..=98).contains(&lot_id);
    match mode {
        Mode::Live => MusicMode::Live,
        Mode::Buy => {
            if magic {
                MusicMode::MagicTownBuy
            } else {
                MusicMode::Buy
            }
        }
        Mode::Build => {
            if magic {
                MusicMode::MagicTownBuild
            } else {
                MusicMode::Build
            }
        }
        Mode::Options => MusicMode::Options,
    }
}

impl Feed {
    /// Clears lot-scoped tracking when transitioning between lots or returning
    /// to the neighborhood map, while preserving window focus and volume steps.
    pub fn reset_for_lot(&mut self) {
        self.heard = None;
        self.told = None;
        self.told_world_key = None;
        self.told_shot = None;
        self.shot = None;
        self.observed_game_paused = false;
        self.requested_paused = false;
        self.pause_dirty = false;
        self.saved_paused = false;
        self.paused_for_blur = false;
        self.music_mode = None;
        self.ambience_zone = None;
        self.zoom = None;
    }

    /// Syncs `[fx, music, vox]` slider steps (`0..=10`) outside a running lot
    /// (e.g. on startup in the Neighborhood screen).
    pub fn sync_volume_steps(&mut self, volumes: [u8; 3], mut send: impl FnMut(Input)) {
        if volumes[0] != self.volumes[0] {
            send(Input::FxVolume(volumes[0]));
            self.volumes[0] = volumes[0];
        }
        if volumes[1] != self.volumes[1] {
            send(Input::MusicVolume(volumes[1]));
            self.volumes[1] = volumes[1];
        }
        if volumes[2] != self.volumes[2] {
            send(Input::VoxVolume(volumes[2]));
            self.volumes[2] = volumes[2];
        }
    }

    /// Updates window focus outside a running lot (e.g. Neighborhood / Family Bin).
    pub fn set_shell_focused(&mut self, focused: bool, mut send: impl FnMut(Input)) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        send(Input::Active(focused));
    }
    /// Whether the window is currently considered focused.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Refreshes `Input::Sources` only when simulation state or camera view
    /// changed since the last sync, reusing `objects`/`positions` on pure
    /// camera pans and sharing the `Snapshot` with the audio thread via `Arc`.
    fn sync_sources(&mut self, game: &crate::Game, send: &mut impl FnMut(Input)) {
        let world_key = game.sound_world_key();
        let shot = game.shot();
        if self.told.is_some()
            && self.told_world_key == Some(world_key)
            && self.told_shot == Some(shot)
        {
            return;
        }
        let next = if let Some(prev) = self.told.as_ref()
            && self.told_world_key == Some(world_key)
        {
            // Pure camera pan/zoom/rotate: `objects`, `positions`, `absent`, and
            // `lot` are unchanged; only recompute screen projections.
            let screen = game.sound_screen();
            if prev.screen == screen {
                self.told_shot = Some(shot);
                return;
            }
            Snapshot {
                objects: prev.objects.clone(),
                absent: prev.absent,
                positions: prev.positions.clone(),
                screen,
                lot: prev.lot,
            }
        } else {
            game.sound_sources()
        };
        if self.told.as_deref() != Some(&next) {
            let shared = Arc::new(next);
            send(Input::Sources(Arc::clone(&shared)));
            self.told = Some(shared);
        }
        self.told_world_key = Some(world_key);
        self.told_shot = Some(shot);
    }

    /// Handles a window focus change (`WM_ACTIVATEAPP`, `0x4f7608`..`0x4f76dc`),
    /// returning whether the lot's pause state changed (so the window should
    /// redraw).
    ///
    /// Repeated notifications with the same state are ignored, including an
    /// initial `true` at window creation (`0x4eee14`, `0x4f7610`).
    ///
    /// - On focus loss (`true -> false`): flushes any already-pending game
    ///   sources, sounds, and camera moves first so kind 25 off does not
    ///   overtake earlier events; saves the requested pause state and requests
    ///   pause (`0x4f7635`..`0x4f765a`, matching `byte [0x6c9fa1] == 1`);
    ///   then sends [`Input::Active`]`(false)` (`0x4f76d5`). The pending
    ///   pause reaches sound as [`SoundEvent::Pause`] on the next
    ///   [`Feed::update`] (`0x4ce5b0`..`0x4ce5c5`), even if a modal dialog
    ///   kept `game.paused()` false.
    /// - On focus regain (`false -> true`): refreshes [`Input::Sources`]
    ///   (and initial [`enter_live`] if needed) without flushing a stale
    ///   pending focus-loss pause; restores the saved pause state
    ///   (`0x4f765c`..`0x4f7666`); then sends [`Input::Active`]`(true)`
    ///   (`0x4f76d5`). The restored pause request reaches sound as
    ///   [`SoundEvent::Pause`] or [`SoundEvent::Unpause`] on the next
    ///   [`Feed::update`] (`0x4ce5b0`..`0x4ce5d9`).
    pub fn set_focused(
        &mut self,
        focused: bool,
        game: &mut crate::Game,
        mut send: impl FnMut(Input),
    ) -> bool {
        if self.focused == focused {
            return false;
        }
        // Pick up any manual pause toggle or mode transition (`0x4ce4c4..0x4ce4d1`)
        // that happened since the last update so `0x4f7645 -> 0x4939c0` saves
        // the latest requested pause state.
        let live_mode = game.mode() == sims_ui::command::Mode::Live;
        let current_audio_paused = game.paused() || !live_mode;
        if current_audio_paused != self.observed_game_paused {
            self.observed_game_paused = current_audio_paused;
            self.requested_paused = current_audio_paused;
            self.pause_dirty = true;
        }
        let pause_changed = if !focused {
            // Flush prior clicks, sounds, and camera moves before focus loss
            // so kind 25 off cannot drop or overtake earlier events, without
            // prematurely emitting a pending focus-regain `Unpause` when a
            // rapid refocus-then-blur occurs before the next `update`.
            self.flush_before_blur(game, &mut send);
            // `0x4f762b..0x4f763c`: auto-pause only in Live mode (`mode == 2`)
            // and when `Sim In Background` is off (`byte [0x6c9fa1] == 1`,
            // toggled at `0x4e628d`).
            if !live_mode || game.options().play.sim_in_background {
                self.paused_for_blur = false;
                false
            } else {
                self.paused_for_blur = true;
                self.saved_paused = self.requested_paused;
                let before = game.paused();
                if !before {
                    game.toggle_pause();
                }
                self.observed_game_paused = game.paused() || !live_mode;
                // `0x4cee52`..`0x4cee5d` records requested pause = true and marks
                // pause dirty (`0x800000`) unconditionally.
                self.requested_paused = true;
                self.pause_dirty = true;
                game.paused() != before
            }
        } else {
            let first = self.told.is_none();
            self.sync_sources(game, &mut send);
            if first {
                send(Input::Events(enter_live()));
            }
            if !self.paused_for_blur || !live_mode {
                if live_mode {
                    self.paused_for_blur = false;
                }
                false
            } else {
                self.paused_for_blur = false;
                let before = game.paused();
                if before != self.saved_paused {
                    game.toggle_pause();
                }
                self.observed_game_paused = game.paused() || !live_mode;
                // `0x4cee52`..`0x4cee5d` records requested pause = saved_paused and
                // marks pause dirty (`0x800000`) unconditionally, so a rapid
                // blur-then-refocus still dispatches the final pause state on the
                // next update (`0x4ce59e` → `0x4ce5b0`).
                self.requested_paused = self.saved_paused;
                self.pause_dirty = true;
                game.paused() != before
            }
        };
        self.focused = focused;
        send(Input::Active(focused));
        pause_changed
    }

    /// Flushes prior game sources, sounds, and camera moves before focus loss
    /// without flushing pending pause changes (both manual toggles at
    /// `0x53a5bf` and focus changes at `0x4f7655` write the same deferred
    /// pause state at `0x4cee52`..`0x4cee5d`, dispatched only on the next
    /// [`Feed::update`] at `0x4ce59e`).
    fn flush_before_blur(&mut self, game: &mut crate::Game, mut send: impl FnMut(Input)) {
        let first = self.told.is_none();
        self.sync_sources(game, &mut send);
        if first {
            send(Input::Events(enter_live()));
        }
        let sounds = game.take_sounds();
        if !sounds.is_empty() {
            send(Input::Events(sounds.into_iter().map(event).collect()));
        }
        for voice_input in game.take_voice_inputs() {
            send(voice_input);
        }
        let now = (game.hour(), game.pace.speed, game.ambience_view());
        if self.heard != Some(now) {
            send(Input::View(now.2));
            send(Input::Hour(now.0));
            send(Input::Speed(now.1));
            self.heard = Some(now);
        }
        let shot = game.shot();
        if self.shot != Some(shot) {
            send(Input::Camera);
            self.shot = Some(shot);
        }
        self.sync_volumes(game, &mut send);
    }

    /// Sends changed `[fx, music, vox]` slider steps (`0..=10`) to the mixer
    /// (kinds 37, 38, and 39, `0x4e6182..0x4e620e`).
    fn sync_volumes(&mut self, game: &crate::Game, mut send: impl FnMut(Input)) {
        let volumes = game.volumes();
        if volumes[0] != self.volumes[0] {
            send(Input::FxVolume(volumes[0]));
            self.volumes[0] = volumes[0];
        }
        if volumes[1] != self.volumes[1] {
            send(Input::MusicVolume(volumes[1]));
            self.volumes[1] = volumes[1];
        }
        if volumes[2] != self.volumes[2] {
            send(Input::VoxVolume(volumes[2]));
            self.volumes[2] = volumes[2];
        }
    }

    /// Sends every sound input that changed on `game` since the last call.
    pub fn update(&mut self, game: &mut crate::Game, mut send: impl FnMut(Input)) {
        // What the tick's sounds may ask about their objects, before the
        // sounds themselves. Dirty-gated on simulation mutations + camera shot
        // so pure UI/hover wakeups skip rebuilding `Snapshot`, and pure camera
        // pans reuse `objects`/`positions`.
        let first = self.told.is_none();
        self.sync_sources(game, &mut send);
        if first {
            send(Input::Events(enter_live()));
        }
        self.sync_volumes(game, &mut send);
        let desired_music = lot_music_mode(game.mode(), game.lot().lot_id());
        if self.music_mode != Some(desired_music) {
            send(Input::MusicMode(desired_music));
            self.music_mode = Some(desired_music);
        }
        let desired_zone = ambience_zone_for_lot(game.lot().lot_id());
        if self.ambience_zone != Some(desired_zone) {
            send(Input::AmbienceZone(desired_zone));
            self.ambience_zone = Some(desired_zone);
        }
        // `0x4ce4c4..0x4ce4d1`: leaving Live mode (e.g. entering Options) pauses
        // live audio; returning to Live unpauses unless the lot is paused.
        let audio_paused = game.paused() || game.mode() != sims_ui::command::Mode::Live;
        if audio_paused != self.observed_game_paused {
            self.observed_game_paused = audio_paused;
            self.requested_paused = audio_paused;
            self.pause_dirty = true;
        }
        if self.pause_dirty {
            send(Input::Events(vec![pause_event(self.requested_paused)]));
            self.pause_dirty = false;
        }
        let sounds = game.take_sounds();
        if !sounds.is_empty() {
            send(Input::Events(sounds.into_iter().map(event).collect()));
        }
        for voice_input in game.take_voice_inputs() {
            send(voice_input);
        }
        let now = (game.hour(), game.pace.speed, game.ambience_view());
        if self.heard != Some(now) {
            send(Input::View(now.2));
            send(Input::Hour(now.0));
            send(Input::Speed(now.1));
            self.heard = Some(now);
        }
        // Kind 33, after the sources and the view the moved camera shows.
        let shot = game.shot();
        if self.zoom != Some(shot.zoom) {
            send(Input::Zoom(shot.zoom));
            self.zoom = Some(shot.zoom);
        }
        if self.shot != Some(shot) {
            send(Input::Camera);
            self.shot = Some(shot);
        }
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

/// What of the camera, when it changes, sends kind 33
/// ([`Input::Camera`]): where the view is, its zoom, turn and floor, and
/// the screen's size. The walls and what the cutaway looks at leave
/// every object where it is on the screen, so they are not in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shot {
    pub origin: [i32; 2],
    pub zoom: u32,
    pub rotation: u8,
    pub level: i32,
    pub size: [i32; 2],
}

impl Shot {
    /// The shot of `camera` on a screen `size` pixels wide and high.
    pub fn of(camera: &Camera, size: [i32; 2]) -> Self {
        Self {
            origin: camera.origin,
            zoom: camera.zoom.level(),
            rotation: camera.rotation,
            level: camera.level,
            size,
        }
    }
}

/// Maps a Sim's person data words (`AGE` word 58, `GENDER` word 65, `FLAGS`
/// word 74) to its [`VoiceAgeGender`] bucket (`0x570e61` voice classification).
pub fn voice_for_person(age: i16, gender: i16, flags: i16) -> VoiceAgeGender {
    if flags & 0b0100 != 0 {
        VoiceAgeGender::Dog
    } else if flags & 0b1000 != 0 {
        VoiceAgeGender::Cat
    } else if age < 3 {
        VoiceAgeGender::Baby
    } else if age < 12 {
        VoiceAgeGender::Child
    } else if gender != 0 {
        VoiceAgeGender::AdultFemale
    } else {
        VoiceAgeGender::AdultMale
    }
}

/// What the mixer is told between blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Events from a tick, in order.
    Events(Vec<SoundEvent>),
    /// The hour of the game's clock (global 0), sent after
    /// [`Input::Sources`] and [`Input::View`]. Once set, a change of
    /// hour runs kind 33 (`camera_moved`), as the original's clock does
    /// at minute zero (`0x46d4d3` → `0x4f66bc`..`0x4f66c6` → `0x57aac5`).
    Hour(u32),
    /// The ambience's camera value (see the module documentation).
    View(i32),
    Speed(Speed),
    /// What the sound scripts may ask about the lot's objects, shared zero-copy
    /// between the UI thread's [`Feed`] and the mixer thread.
    Sources(Arc<Snapshot>),
    /// The camera moved (kind 33): sent after the [`Input::Sources`]
    /// and [`Input::View`] that show the lot through the moved camera.
    Camera,
    /// The window gained or lost focus (kind 25, `0x4f7608` → `0x4f76d5`).
    Active(bool),
    /// Sound effects volume slider step `0..=10` (kind 37, `0x4e6182` → `0x57408f`).
    FxVolume(u8),
    /// Music volume slider step `0..=10` (kind 38, `0x4e61bb` → `0x5740f9`).
    MusicVolume(u8),
    /// Voice volume slider step `0..=10` (kind 39, `0x4e61f4` → `0x57410d`).
    VoxVolume(u8),
    /// Streaming mode music transition (`kSetMusicMode`, `0x575170`).
    MusicMode(MusicMode),
    /// Environmental lot ambience zone (`Residential`, `Downtown`, `Vacation*`,
    /// `StudioTown`, `MagicTown`).
    AmbienceZone(AmbienceZone),
    /// Camera zoom level (`0` = Far, `1` = Medium, `2` = Near) for environmental
    /// ambience attenuation.
    Zoom(u32),
    /// Tunes an in-lot stereo or TV emitter (`object_id`) to `station`.
    RadioTune {
        emitter: u32,
        station: StationId,
    },
    /// Skips to the next shuffled track on an active stereo/TV emitter.
    RadioSkip {
        emitter: u32,
    },
    /// Turns off one active stereo/TV emitter.
    RadioStop {
        emitter: u32,
    },
    /// Stops all active stereo/TV emitters.
    RadioStopAll,
    /// Plays a Simlish voice clip from [`VoiceCatalog`] (for Sim speech/reactions
    /// or Create-a-Sim voice previews).
    PlayVoice {
        voice: VoiceAgeGender,
        category: Option<String>,
        pitch: VoicePitch,
        seed: u64,
        /// Optional lot object ID for spatial placement & pause gating (`None`
        /// for UI / Create-a-Family previews).
        object: Option<u32>,
    },
    /// Stops all active Simlish voice streams immediately.
    StopVoices,
}

/// Maximum concurrent [`VoiceStream`] instances mixed simultaneously.
const MAX_ACTIVE_VOICES: usize = 16;

struct ActiveVoice {
    stream: VoiceStream,
    object: Option<u32>,
}

/// The sound player with every module, station and ambient score of an
/// install, paired with streaming Neighborhood/Buy/Build/expansion mode music,
/// in-lot stereo/TV radio streaming, environmental lot ambience beds, and
/// Simlish voice bank playback.
pub struct Mixer {
    player: Player<Arc<SoundFiles>>,
    music: ModeMusicPlayer<Arc<SoundFiles>>,
    radio: RadioPlayer<Arc<SoundFiles>>,
    ambience: AmbienceMixer<Arc<SoundFiles>>,
    voices: VoiceCatalog,
    active_voices: Vec<ActiveVoice>,
    voice_scratch: Vec<i16>,
    hour: u32,
    view: i32,
    sources: Arc<Snapshot>,
    /// Whether the host window is focused (`Input::Active`).
    active: bool,
    /// Whether live gameplay audio is paused (`SoundEvent::Pause` / `Unpause`).
    paused_by_game: bool,
    /// Whether an active lot has enabled the streaming environmental ambience bed.
    ambience_active: bool,
    /// Faults already reported, so each is reported once.
    reported: BTreeSet<String>,
}

impl Mixer {
    /// Reads the install's sound files at `root`.
    pub fn open(root: &Path) -> Result<Self, String> {
        let files = Arc::new(SoundFiles::scan(root).map_err(|e| format!("sound files: {e}"))?);
        let modules = Module::load_all(&files).map_err(|e| format!("sound modules: {e}"))?;
        let lists = StationLists::build(&files);
        let catalog = ModeCatalog::from_sound_files(&files, true, ALL_EXPANSIONS);
        let music = ModeMusicPlayer::new(catalog, Arc::clone(&files), SEED);
        let radio_catalog = RadioCatalog::from_sound_files(&files);
        let radio = RadioPlayer::new(radio_catalog, Arc::clone(&files), SEED ^ 0x5241_4449);
        let ambience_catalog = AmbienceCatalog::from_sound_files(&files);
        let ambience = AmbienceMixer::new(ambience_catalog, Arc::clone(&files));
        let voices = VoiceCatalog::from_shared_sound_files(Arc::clone(&files));
        let mut player = Player::new(modules, Arc::clone(&files), SEED);
        player.add_stations(&lists);
        player.add_scores(&files);
        player.set_global(SPEED_REGISTER, speed_register(Speed::Normal));
        Ok(Self {
            player,
            music,
            radio,
            ambience,
            voices,
            active_voices: Vec::new(),
            voice_scratch: Vec::new(),
            hour: u32::MAX,
            view: 0,
            sources: Arc::new(Snapshot::default()),
            active: true,
            paused_by_game: false,
            ambience_active: false,
            reported: BTreeSet::new(),
        })
    }

    fn start_voice_stream(&mut self, stream: VoiceStream, object: Option<u32>) {
        if let Some(obj) = object {
            self.active_voices.retain(|v| v.object != Some(obj));
        } else {
            // Keep only one UI / CAF preview voice active at once so rapid
            // appearance clicks stay crisp rather than overlapping.
            self.active_voices.retain(|v| v.object.is_some());
        }
        if self.active_voices.len() >= MAX_ACTIVE_VOICES {
            self.active_voices.remove(0);
        }
        self.active_voices.push(ActiveVoice { stream, object });
    }

    fn mix_active_voices(&mut self, out: &mut [i16]) {
        if !self.active || self.active_voices.is_empty() || out.is_empty() {
            return;
        }
        let vox_step = self.player.vox_volume_step();
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        let mut scratch = std::mem::take(&mut self.voice_scratch);
        for voice in &mut self.active_voices {
            if voice.object.is_some() && self.paused_by_game {
                continue;
            }
            let ch = usize::from(voice.stream.channels());
            if ch == 0 {
                continue;
            }
            scratch.resize(frames * ch, 0i16);
            let read = voice.stream.read_frames(&mut scratch);
            if read == 0 || vox_step == 0 {
                continue;
            }
            let spatial = voice.object.map(|obj| {
                let key = obj as i32;
                self.sources
                    .screen
                    .get(&key)
                    .and_then(|proj| {
                        let level = self.sources.positions.get(&key).map_or(1, |pos| pos[2]);
                        SpatialAttenuation::from_projection(20, level, proj).ok()
                    })
                    .unwrap_or(SpatialAttenuation::FULL)
            });
            let [left_gain, right_gain] = voice_stereo_gains(vox_step, spatial);
            if left_gain <= 0.0 && right_gain <= 0.0 {
                continue;
            }
            for i in 0..read {
                let (sl, sr) = if ch == 1 {
                    let s = f32::from(scratch[i]);
                    (s, s)
                } else {
                    (f32::from(scratch[i * ch]), f32::from(scratch[i * ch + 1]))
                };
                let dl = (f32::from(out[i * 2]) + sl * left_gain)
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    as i16;
                let dr = (f32::from(out[i * 2 + 1]) + sr * right_gain)
                    .round()
                    .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                    as i16;
                out[i * 2] = dl;
                out[i * 2 + 1] = dr;
            }
        }
        self.voice_scratch = scratch;
        self.active_voices.retain(|v| !v.stream.is_finished());
    }

    fn sync_stream_pause(&mut self) {
        let paused = !self.active || self.paused_by_game;
        self.radio.set_paused(paused);
        self.ambience.set_paused(paused);
    }

    fn spatial_for_emitter(&self, emitter_id: u32) -> SpatialAttenuation {
        let key = emitter_id as i32;
        if let Some(proj) = self.sources.screen.get(&key) {
            let level = self.sources.positions.get(&key).map_or(1, |pos| pos[2]);
            if let Ok(spatial) = SpatialAttenuation::from_projection(20, level, proj) {
                return spatial;
            }
        }
        SpatialAttenuation::FULL
    }

    fn refresh_radio_spatial(&mut self) {
        if self.radio.active_emitters() == 0 {
            return;
        }
        let ids: Vec<u32> = self
            .sources
            .screen
            .keys()
            .filter_map(|&id| u32::try_from(id).ok())
            .filter(|&id| self.radio.emitter_station(id).is_some())
            .collect();
        for id in ids {
            let spatial = self.spatial_for_emitter(id);
            self.radio.set_emitter_spatial(id, spatial);
        }
    }

    /// Takes one input.
    pub fn input(&mut self, input: Input) {
        match input {
            Input::Events(events) => {
                // Placed where the last snapshot has their objects, which
                // the game sends before a tick's events.
                for event in &events {
                    match event {
                        SoundEvent::StopAll => {
                            self.music.stop();
                            self.radio.stop_all();
                            self.ambience.stop();
                            self.active_voices.clear();
                            self.ambience_active = false;
                            self.paused_by_game = false;
                            self.sync_stream_pause();
                        }
                        SoundEvent::Pause => {
                            self.paused_by_game = true;
                            self.sync_stream_pause();
                        }
                        SoundEvent::Unpause => {
                            self.paused_by_game = false;
                            self.sync_stream_pause();
                        }
                        SoundEvent::StopObject { object } => {
                            self.radio.stop_emitter(*object);
                            self.active_voices.retain(|v| v.object != Some(*object));
                        }
                        SoundEvent::Play { object, name } => {
                            if is_radio_stop_event(name) {
                                self.radio.stop_emitter(*object);
                            } else if let Some(station) = station_for_sound_event(name) {
                                let spatial = self.spatial_for_emitter(*object);
                                if self.radio.tune(*object, station, spatial) {
                                    continue;
                                }
                            } else if name.eq_ignore_ascii_case("radio_next_song")
                                && self.radio.skip_track(*object)
                            {
                                continue;
                            } else if let Some(clip) = self.voices.resolve_path(name)
                                && let Ok(stream) =
                                    self.voices.open_stream(clip, VoicePitch::NATIVE, RATE)
                            {
                                let obj = (*object != 0).then_some(*object);
                                self.start_voice_stream(stream, obj);
                                continue;
                            }
                        }
                    }
                    self.player.handle(event, &*self.sources);
                }
            }
            Input::Hour(hour) => {
                let changed = self.hour != u32::MAX && self.hour != hour;
                self.hour = hour;
                if self.ambience_active {
                    self.ambience.set_hour((hour % 24) as u8);
                }
                if changed {
                    self.player
                        .camera_moved(self.view, self.hour, &*self.sources);
                }
            }
            Input::View(view) => {
                self.view = view;
                self.ambience.set_outdoor_percent(view.clamp(0, 100) as u8);
            }
            Input::Sources(sources) => {
                self.sources = sources;
                self.refresh_radio_spatial();
            }
            Input::Camera => {
                self.player
                    .camera_moved(self.view, self.hour, &*self.sources);
                self.refresh_radio_spatial();
            }
            Input::Active(active) => {
                self.active = active;
                self.player.set_active(active, &*self.sources);
                self.music.set_paused(!active);
                self.sync_stream_pause();
            }
            Input::FxVolume(step) => {
                let idx = usize::from(step);
                self.player.set_fx_volume_step(idx, &*self.sources);
                self.radio.set_fx_volume_step(idx);
                self.ambience.set_volume_step(idx);
            }
            Input::MusicVolume(step) => {
                let idx = usize::from(step);
                self.player.set_music_volume_step(idx);
                self.music.set_volume_step(idx);
                self.radio.set_music_volume_step(idx);
            }
            Input::VoxVolume(step) => self.player.set_vox_volume_step(usize::from(step)),
            Input::Speed(speed) => self
                .player
                .set_global(SPEED_REGISTER, speed_register(speed)),
            Input::MusicMode(mode) => self.music.set_mode(mode),
            Input::AmbienceZone(zone) => {
                self.ambience_active = true;
                self.ambience.set_zone(zone);
                if self.hour != u32::MAX {
                    self.ambience.set_hour((self.hour % 24) as u8);
                }
                self.ambience
                    .set_outdoor_percent(self.view.clamp(0, 100) as u8);
            }
            Input::Zoom(zoom) => {
                self.ambience.set_zoom(zoom_level_from_u32(zoom));
            }
            Input::RadioTune { emitter, station } => {
                let spatial = self.spatial_for_emitter(emitter);
                self.radio.tune(emitter, station, spatial);
            }
            Input::RadioSkip { emitter } => {
                self.radio.skip_track(emitter);
            }
            Input::RadioStop { emitter } => {
                self.radio.stop_emitter(emitter);
            }
            Input::RadioStopAll => {
                self.radio.stop_all();
            }
            Input::PlayVoice {
                voice,
                category,
                pitch,
                seed,
                object,
            } => {
                let clip = self
                    .voices
                    .pick(voice, category.as_deref(), seed)
                    .or_else(|| self.voices.pick(voice, None, seed));
                if let Some(clip) = clip
                    && let Ok(stream) = self.voices.open_stream(clip, pitch, RATE)
                {
                    self.start_voice_stream(stream, object);
                }
            }
            Input::StopVoices => {
                self.active_voices.clear();
            }
        }
    }

    /// Current `[fx, music, vox]` slider steps (`0..=10`) stored in the player.
    pub fn volume_steps(&self) -> [u8; 3] {
        [
            self.player.fx_volume_step(),
            self.player.music_volume_step(),
            self.player.vox_volume_step(),
        ]
    }

    /// Current streaming mode-music station.
    pub fn music_mode(&self) -> MusicMode {
        self.music.mode()
    }

    /// Whether streaming mode music currently has an active or fading deck.
    pub fn music_playing(&self) -> bool {
        self.music.is_playing()
    }

    /// In-lot streaming stereo/TV radio player.
    pub fn radio(&self) -> &RadioPlayer<Arc<SoundFiles>> {
        &self.radio
    }

    /// Streaming environmental lot ambience mixer.
    pub fn ambience(&self) -> &AmbienceMixer<Arc<SoundFiles>> {
        &self.ambience
    }

    /// Indexed Simlish voice bank catalog.
    pub fn voices(&self) -> &VoiceCatalog {
        &self.voices
    }

    /// Number of currently playing Simlish [`VoiceStream`] instances.
    pub fn active_voice_count(&self) -> usize {
        self.active_voices.len()
    }

    /// Renders `frames` stereo frames at [`sims_audio::player::RATE`],
    /// moving the ambience's cursor to the latest hour and view first, then
    /// mixing streaming mode music, stereo/TV radio, environmental ambience,
    /// and Simlish voice playback into the output buffer.
    pub fn render(&mut self, frames: usize) -> Vec<i16> {
        let mut out = Vec::new();
        self.render_into(frames, &mut out);
        out
    }

    /// Renders `frames` stereo frames into `out`, reusing `out`'s allocation
    /// and the mixer's internal voice scratch buffer for zero steady-state
    /// per-block heap allocations.
    pub fn render_into(&mut self, frames: usize, out: &mut Vec<i16>) {
        self.player.set_ambience_view(self.view, self.hour);
        self.player.render_stereo_into(frames, &*self.sources, out);
        self.music.mix_into_stereo(out);
        self.radio.mix_into_stereo(out);
        self.ambience.mix_into_stereo(out);
        self.mix_active_voices(out);
    }

    /// Faults not reported yet, each once, up to `REPORTED` of them; a
    /// last line says when more are no longer shown. The player's list is
    /// emptied each time, so neither it nor this one grows without end.
    pub fn new_faults(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for fault in self
            .player
            .faults
            .drain(..)
            .chain(self.music.faults.drain(..))
            .chain(self.radio.faults.drain(..))
        {
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

/// Converts a 1.91 `vox_volume_step` (`0..=10`) and optional [`SpatialAttenuation`]
/// (`volume: 0..=1024`, `pan: 0..=1024` with `512` at center) into linear `[left, right]`
/// stereo gains using the exact same [`sims_audio::levels`] conversion as [`RadioPlayer`].
pub fn voice_stereo_gains(vox_step: u8, spatial: Option<SpatialAttenuation>) -> [f32; 2] {
    let vox_vol = VOLUME_STEPS[usize::from(vox_step.min(10))];
    if vox_vol <= 0 {
        return [0.0, 0.0];
    }
    let spatial = spatial.unwrap_or(SpatialAttenuation::FULL);
    if spatial.volume <= 0 {
        return [0.0, 0.0];
    }
    let eff_vol = (i64::from(spatial.volume) * i64::from(vox_vol)) / 1024;
    if eff_vol <= 0 {
        return [0.0, 0.0];
    }
    let att = levels::attenuation(eff_vol, 1024);
    let [gl, gr] = levels::gains(att, levels::pan(spatial.pan));
    [gl as f32 / 65536.0, gr as f32 / 65536.0]
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
    fn a_sound_s_point_is_cut_as_the_original_s_view_cuts_it() {
        use sims_render::camera::{Walls, Zoom};
        let mut camera = Camera {
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            origin: [400, 17],
            walls: Walls::Up,
            focus: sims_render::Focus::default(),
        };
        // 0x4a9dc7 gives (402, 178); the drawing's projection 179 down.
        assert_eq!(fine_point(&camera, [163, 161], 1), [402, 178]);
        assert_eq!(camera.project_fine([163, 161], 1, 0), [402, 179]);
        // Along the diagonal, straight down; at the corner, the corner.
        assert_eq!(fine_point(&camera, [165, 165], 1), [400, 182]);
        assert_eq!(fine_point(&camera, [160, 160], 1), [400, 177]);
        // The other side of the diagonal. x: 16 * 3 / -16 = -3, less -3 *
        // 1 / 3 = -1: -2. y: side 8 * 3 / 16 = 1, diagonal 3, (3 - 1) * 1
        // / 3 + 1 = 1, where the drawing's is 2.
        assert_eq!(fine_point(&camera, [161, 163], 1), [398, 178]);
        assert_eq!(camera.project_fine([161, 163], 1, 0), [398, 179]);
        // Near, with nothing to cut, the two agree.
        camera.zoom = Zoom::Near;
        assert_eq!(
            fine_point(&camera, [163, 167], 1),
            camera.project_fine([163, 167], 1, 0)
        );
    }

    #[test]
    fn terrain_heights_raise_and_slant_fine_point() {
        use sims_render::camera::{Walls, Zoom};
        let camera = Camera {
            zoom: Zoom::Far,
            rotation: 0,
            level: 1,
            origin: [400, 100],
            walls: Walls::Up,
            focus: sims_render::Focus::default(),
        };
        // A flat elevated tile raises y by `h << (zoom + 1)` without
        // changing x or the sub-tile step.
        let flat = fine_point(&camera, [163, 161], 1);
        assert_eq!(
            fine_point_with_heights(&camera, [163, 161], 1, [6, 6, 6, 6]),
            [flat[0], flat[1] - (6 << 2)]
        );
        // Sloped tile: top=6, right=2, bottom=0, left=4.
        // Diagonal (fx == fy == 8): corner raised by 6*4=24, step down by
        // ((6 - 0 + 4) << 2) * 8 / 16 = 20 (vs flat 8).
        let diag_flat = fine_point(&camera, [168, 168], 1);
        assert_eq!(
            fine_point_with_heights(&camera, [168, 168], 1, [4, 6, 2, 0]),
            [diag_flat[0], diag_flat[1] - 24 + 12]
        );
        // fx > fy (fx=8, fy=4): side = ((6-2+2)<<2)*8/16 = 12,
        // diagonal = ((6-0+4)<<2)*8/16 = 20 -> dy = (20-12)*4/8 + 12 = 16.
        let right_base = camera.project([10, 10], 1, 6);
        assert_eq!(
            fine_point_with_heights(&camera, [168, 164], 1, [4, 6, 2, 0]),
            [right_base[0] + 4, right_base[1] + 16]
        );
        // fx < fy (fx=4, fy=8): side = ((6-4+2)<<2)*8/16 = 8,
        // diagonal = ((6-0+4)<<2)*8/16 = 20 -> dy = (20-8)*4/8 + 8 = 14.
        assert_eq!(
            fine_point_with_heights(&camera, [164, 168], 1, [4, 6, 2, 0]),
            [right_base[0] - 4, right_base[1] + 14]
        );
    }

    #[test]
    fn tile_corners_rotate_and_zero_the_border() {
        use sims_render::camera::{Walls, Zoom};
        let mut world = World::empty();
        world.size = 52;
        let full = sims_render::lot::FULL * sims_render::lot::FULL;
        for (id, stride) in [(0, 4), (1, 1), (2, 8), (101, 1), (102, 8)] {
            world.arrays.insert(
                id,
                sims_formats::lot::Array {
                    width: sims_render::lot::FULL,
                    height: sims_render::lot::FULL,
                    stride,
                    encoding: 0,
                    data: vec![0; full * stride],
                },
            );
        }
        // Set inner tile (10, 20) to [1, 2, 3, 4] and border tile (0, 20)
        // to [9, 9, 9, 9].
        let cell = |x: usize, y: usize| (x * sims_render::lot::FULL + y) * 4;
        let arr0 = world.arrays.get_mut(&0).unwrap();
        arr0.data[cell(10, 20)..cell(10, 20) + 4].copy_from_slice(&[1, 2, 3, 4]);
        arr0.data[cell(0, 20)..cell(0, 20) + 4].copy_from_slice(&[9, 9, 9, 9]);
        let lot = sims_render::Lot::from_world(&world).unwrap();

        let expected = [[1, 2, 3, 4], [4, 1, 2, 3], [3, 4, 1, 2], [2, 3, 4, 1]];
        for rotation in 0u8..4 {
            let camera = Camera {
                zoom: Zoom::Far,
                rotation,
                level: 1,
                origin: [0, 0],
                walls: Walls::Up,
                focus: sims_render::Focus::default(),
            };
            let view_inner = camera.rotate_tile([10, 20]);
            assert_eq!(
                tile_corners(&lot, &camera, view_inner),
                expected[usize::from(rotation)],
                "rotation {rotation}"
            );
            // Border tile is zeroed by 0x4a9c50 / 0x4a9dc7.
            let view_border = camera.rotate_tile([0, 20]);
            assert_eq!(tile_corners(&lot, &camera, view_border), [0; 4]);
        }
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

    #[test]
    fn feed_starts_focused_like_app_constructor() {
        assert!(Feed::default().focused());
    }

    #[test]
    fn voice_stereo_gains_pan_left_center_and_right() {
        assert_eq!(
            voice_stereo_gains(0, Some(SpatialAttenuation::FULL)),
            [0.0, 0.0]
        );

        let [cl, cr] = voice_stereo_gains(
            10,
            Some(SpatialAttenuation {
                volume: 1024,
                pan: 512,
            }),
        );
        assert!(
            cl > 0.9 && (cl - cr).abs() < 1e-4,
            "center pan must be balanced: [{cl}, {cr}]"
        );

        let [ll, lr] = voice_stereo_gains(
            10,
            Some(SpatialAttenuation {
                volume: 1024,
                pan: 0,
            }),
        );
        assert!(
            ll > 0.9 && lr < ll * 0.1,
            "left pan must favor left: [{ll}, {lr}]"
        );

        let [rl, rr] = voice_stereo_gains(
            10,
            Some(SpatialAttenuation {
                volume: 1024,
                pan: 1024,
            }),
        );
        assert!(
            rr > 0.9 && rl < rr * 0.1,
            "right pan must favor right: [{rl}, {rr}]"
        );

        let [ul, ur] = voice_stereo_gains(10, None);
        assert_eq!(
            [ul, ur],
            [cl, cr],
            "UI/CAF preview must be centered at full volume"
        );
    }
}
