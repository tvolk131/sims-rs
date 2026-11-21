//! The HIT virtual machine: runs sound events and mixes their samples.
//!
//! A [`SoundEvent::Play`] finds its HOT event, whose kind says what to do
//! with its track (see Events, below); most start a *thread*: the
//! track's HIT routine, run for one object. A
//! thread has its own registers, a call stack and a loop point; it starts
//! notes (samples of a patch) and waits for them, for a time, or for a
//! register condition. The player runs every thread that is not waiting
//! at the start of each block of [`BLOCK`] frames, then mixes the playing
//! notes into stereo 16-bit PCM at [`RATE`] ([`Player::render`] gives its
//! mono mix).
//!
//! Where the instructions' meanings come from. Marked *(code)*: the
//! original's own code, read from a memory image of the running game
//! (never written, not traced at run time). The register names are the
//! HSM files' own. The rest follows public notes (FreeSO's HIT player,
//! the Niotso wiki) and what the install's programs need.
//!
//! - Registers below 100 belong to a thread; 100 and above are globals
//!   shared by every thread (`SimSpeed` is 100). A new thread's `patch`
//!   register (50) holds its track's patch. *(code)* `arg1` to `arg3`
//!   (registers 1 to 3) hold the arguments it was started with, whatever
//!   the track's argument kind (0x57d9cb): the first is the object that
//!   started it, which routines pass to `getsrcdatafield`
//!   ([`crate::sources`]). A track a script starts (`play_trk`) gets
//!   the caller's own `arg1` to `arg3` (0x57f1f2). A fresh start also
//!   writes `extvol` and `pan` from the arguments as the argument kind
//!   says (0x57db01): `arg1` and `arg2` for `kArgsVolPan`, `arg2` and
//!   `arg3` for `kArgsIdVolPan`.
//! - *(code)* The registers in [`DEFAULTS`] start as the original's
//!   default register bank holds them, then `vol` (18) and `duckpri` (25)
//!   take the HOT track's volume and duck priority; every other starts
//!   at 0.
//! - *(code)* Every register write sets the written thread's comparison
//!   flags from the value; `cmp a, b` sets them from `a - b` and `test r`
//!   from `r`. The `if` branches test them.
//! - *(code)* Writing `target` (53) with a registered track number
//!   (0x57e023 → 0x573312) sends this thread's later reads and writes of
//!   registers other than 53 to that track, and on through that track's
//!   own `target`; 0, its own track, or an unregistered number sends them
//!   back here (0x57de57). When the destination track has stopped, writes
//!   still go to that stopped track rather than falling back to the
//!   caller (with `fade_on` forced to 0 at 0x57e059..0x57e064), and once
//!   that track starts again the link reaches it again.
//! - *(code)* `main_duckpri` (123) is a request per thread: the global
//!   holds the largest, and a thread's request goes as soon as it ends.
//! - *(code)* Volume, pan and pitch: see [`crate::levels`]. Each note
//!   plays at its thread's levels, updated after the threads run in each
//!   block and on placement or volume changes (the original updates them
//!   as registers change). A sample plays at the rate
//!   its pitch sets, whatever its own rate.
//! - `note_on` starts the patch in register 50 and writes the note's ID
//!   to its operand; `wait_samp` waits until the thread's last note ends.
//! - *(code)* `end` (and `return`, 0x57e3ee) returns from a `call`, or
//!   with an empty call stack calls fade-out-and-kill (0x57edc8 → vtable
//!   +0x34, 0x57dca3): with no note playing it kills the track at once
//!   (0x57dd3e); with a note still playing it fades `extvol` to 0 at
//!   speed 100 and keeps the track running until the fade ends, while the
//!   run loop (0x57d8b3) goes on at the next byte because nothing moves
//!   the wait deadline. (No program in the install has a `return`,
//!   against 2,504 calls, and no routine in the install reaches `end`
//!   with a note still playing.)
//! - `set_loop` marks the next instruction; `loop` goes back to it, or to
//!   the routine's start when nothing was marked (the looping generic
//!   routine is `note_on v1; wait_samp; loop`). The thread then waits for
//!   the next block, so a loop that starts no note cannot spin.
//! - `smart_setlist` picks a hit list; `smart_choose` writes one of its
//!   patches, chosen uniformly with the player's own random numbers.
//! - `wait r` waits `r` milliseconds; the `wait` comparisons wait until
//!   their condition holds.
//! - *(code)* Sequence groups. A group is a slot holding the track number
//!   that last claimed it. `seqgroup_return g` claims group `g` and ends
//!   its own thread if another track held it. `seqgroup_kill g` claims it
//!   too; if another track held it, that track's newest thread ends at
//!   once when its `extvol` is 0, and otherwise starts fading `extvol` at
//!   speed 200 (unless it already is) while this thread runs the
//!   instruction again 2 ms later. Slots are never cleared when a thread
//!   ends (only [`SoundEvent::StopAll`] clears them here). `seqgroup_trkid
//!   g, r` writes group `g`'s holder to `r`.
//! - *(code)* A fade moves register `fade_var` toward `fade_dest` by
//!   `fade_speed` once per block, before the thread's instructions, and
//!   reads and writes it through `target`; how often the original updates
//!   is not known.
//!
//! Events. *(code)* Every kind goes through the original's dispatcher
//! (0x573617) with the event's track and three arguments: each is the
//! event's own number when not 0, else the caller's (the object for the
//! first, 0 for the others). [`kind`] lists them; ported are:
//!
//! - 1, play: starts the track (or refreshes it, when it runs), and lets
//!   it go. Only when the track's arguments are
//!   `kArgsIdVolPan`, its object is recorded as an *instance* of the
//!   track, and the track's `extvol` and `pan` are set from where all its
//!   objects are ([`crate::placement`]; 0x573756, 0x5737b3); an object
//!   of 0 starts nothing.
//!   A track with no running thread loses its stale instances first.
//! - 18, fade: fades the track's `extvol` to 0 (see [`Player::play_from`]
//!   for the speed), ending it after when the third argument says so.
//! - 19, `radio_next_song`: stops the radio tracks' notes.
//! - 20, kill all; 23, kill an object's instances (what
//!   [`SoundEvent::StopObject`] does); 105, kill the instances of every
//!   object above 0. Killing an instance lets go of its track: a track no
//!   object still has fades out at speed 100 and ends, or ends at once
//!   when it has no note playing (0x57dca3). Kill all also stops every
//!   ambient score.
//! - 21, pause, and 22, unpause ([`SoundEvent::Pause`] and
//!   [`SoundEvent::Unpause`], what the game sends): a pause holds every
//!   track that runs or plays, but for the interface's clicks, the speed
//!   buttons and the main music loop ([`PAUSE_SPARES`]), stops the day and
//!   night loop and pauses the live ambience; an unpause (while sound is
//!   active) lets them all go on from where they were, updates the day and
//!   night loop, and lets `MainMusicLoop` go. See [`Player::pause`] and
//!   [`Player::unpause`].
//! - 25, sound active ([`Player::set_active`], 0x573c64): what the
//!   original's `WM_ACTIVATEAPP` handler sends when the window loses or
//!   regains focus (0x4f7608 → 0x4f76d5). Inactive sound skips kind 1,
//!   holds kind 30 starts, suppresses unpause's resume, stops the day and
//!   night loop, and pauses all ambience scores in live mode. Turning it
//!   back on starts a 100-update volume-nudge countdown (`+0x114`,
//!   0x573019, 0x574701), resumes if unpaused (or in neighbourhood mode),
//!   and lets `MainMusicLoop` go.
//! - 30, `kTurnOnTv` (radios too): kills the object's instances, then
//!   records the object as an instance of the track (even 0), starts it
//!   and sets its `extvol` and `pan` as kind 1 does.
//! - 53, `kSequenceTrackHitList`: picks a patch from a hit list and
//!   chains it to the note the track is playing, to start with no gap
//!   when that one ends; with no note playing, starts it.
//! - 52, `kSetAmbienceMode`: makes a score current and starts it,
//!   stopping the one before (see [`crate::ambience`] and
//!   [`Player::add_scores`]); a mode with no score stops the current one.
//! - 36, `kSetMusicMode`: changes the music for the game's mode, then
//!   the ambience and the day and night loop (see `Player::set_music_mode`).
//! - 43, `kPlayPiano`: starts the piano (track [`PIANO`]) where its
//!   object is, and the skill track [`PIANO_SKILL`] (see
//!   `Player::play_piano`).
//!
//! The game's code also calls kind 33 when the camera moves: it places
//! every track again from where its objects are now, moves the
//! ambience's cursor from the camera and the hour, and updates the day
//! and night loop (`nite_loop` 2985 or `BeachLoop` 68259, 0x574978)
//! ([`Player::camera_moved`]; the cursor alone is
//! [`Player::set_ambience_view`]). The scores'
//! voices mix with the notes, at the effects group's volume.
//!
//! The radio, television and music hit lists are built from directories
//! (see [`crate::stations`] and [`Player::add_stations`]). An MP3 is
//! resampled to [`RATE`] when loaded: the original decodes MP3s through
//! a data source and then sets the buffer's rate from `pitch` as for any
//! sound, so a 44.1 kHz song at pitch 3600 would otherwise play an octave
//! low. That the data source delivers 22,050 Hz is **unverified**.
//!
//! *(code)* The original has one track object per track number, shared
//! by every object playing it; here a track runs one thread at most. A
//! start of a track whose thread runs starts no second one: it refreshes
//! that one, and starts it anew only when its `interrupt` is set (see
//! `Player::start`). An object letting go of a track that others still
//! have leaves it playing, placed again from where they are (0x5746d9).
//! When a script reaches `end` (or `return`) with nothing to return to
//! while a note is still playing (0x57e3ee → 0x57edc8, vtable +0x34,
//! 0x57dca3), the track stays running, fades `extvol` to 0 at speed 100
//! (writing the fade registers through `target` and setting its own
//! kill-after-fade flag at +0x1cf), and continues executing at the next
//! byte in the run loop (0x57d8b3) because nothing moves the wait
//! deadline. A start during that fade still sees the track running,
//! clears the fade, and reads `interrupt`. If execution runs past the
//! end of the program buffer (where the original would read past its
//! heap buffer), this player faults and stops the thread and its notes.
//! Kinds not ported (pausing, reloading, and the sound object calls only
//! the game's code makes) are reported as faults; kinds whose case does
//! nothing for sound are ignored.
//!
//! **Unverified** against the original at run time: all of the above.
//! Left out: priorities, and the limit of 16 voices kind 1 checks first
//! (0x574a87); the track group's own volume beyond the host's [`Player::set_group_volume`], and
//! the second pitch the original adds (taken as 3600).
//! `seqgroup_wait`, `add_to_group`, `remove_from_group` and `test1` to
//! `test4` are not ported: a thread that reaches one ends with a fault.
//! (The original does nothing for the first three; no program in the
//! install uses them. The tests are debug routines.) `setll`, `setlt` and
//! `settl` copy like `set`.
//!
//! Limits, so no script can hang or exhaust the host: one thread runs at
//! most [`STEPS`] instructions per block and all threads together
//! [`BLOCK_STEPS`] (threads a block does not reach run first in the
//! next); at most [`THREADS`] threads, [`NOTES`] notes and [`SEQGROUPS`]
//! sequence groups, [`INSTANCES`] instances, and [`CHAIN`] patches
//! chained to one note exist at once; calls nest at most [`CALL_DEPTH`] deep;
//! a thread started by `play_trk` first runs in the next block; a patch
//! that does not exist is not cached. A track, patch, hit list or track
//! data ID that several modules define is the last one registered
//! ([`owners`], [`hit_lists`], [`entry_points`]). How the original picks
//! among duplicate events is unknown; here the module whose name sorts
//! first wins.

use crate::ambience::{self, Fresh, Host, Key, Voice};
use crate::files::SoundFiles;
use crate::fsc::Score;
use crate::hit::{Instruction, Operand};
use crate::hot::Object;
use crate::levels::{self, Inputs, Levels};
use crate::module::Module;
use crate::placement::{self, Projection};
use crate::registry::Registry;
use crate::stations::StationLists;
use crate::{Pcm, SoundEvent};
use std::collections::{BTreeMap, BTreeSet};

/// The output sample rate: that of nearly every sample in the install.
pub const RATE: u32 = 22_050;
/// Frames mixed between two runs of the threads (about 2.9 ms).
pub const BLOCK: usize = 64;
/// Instructions one thread may run in one block before the player calls
/// it stuck and ends it.
pub const STEPS: usize = 10_000;
/// Instructions all threads together may run in one block. When they are
/// spent, the running thread stops where it is and it and every thread
/// not yet run go on in the next block.
pub const BLOCK_STEPS: usize = 100_000;
/// Threads that may exist at once, those waiting to start included. A
/// thread started beyond it is a fault.
pub const THREADS: usize = 256;
/// Notes that may play at once. A note started beyond it is a fault and
/// does not play.
pub const NOTES: usize = 1024;
/// Faults the player records before it drops further ones.
pub const FAULTS: usize = 1000;
/// How deep `call`s may nest in one thread.
pub const CALL_DEPTH: usize = 64;
/// How many sequence groups may exist; slots are never freed.
pub const SEQGROUPS: usize = 256;
/// The first global register.
pub const GLOBALS: u8 = 100;
/// The register a new note's patch is read from.
pub const PATCH: u8 = 50;

/// The track argument kind `kArgsVolPan`: `arg1` and `arg2` are the
/// levels.
pub const ARGS_VOL_PAN: i64 = 1;
/// The track argument kind `kArgsIdVolPan`: `arg1` is the object.
pub const ARGS_ID_VOL_PAN: i64 = 2;
/// The piano's track, which kind 43 starts (0x573e5a).
pub const PIANO: u32 = 1906;
/// The track kind 43 starts after the piano, with the player's skill.
pub const PIANO_SKILL: u32 = 1907;
/// *(code)* The night loop track (`nite_loop`, 0x6ceb08), picked by
/// `0x57492a` when the hour is below 6 or above 18 outside lots 20 to 29.
pub const NITE_LOOP: u32 = 2985;
/// *(code)* The beach loop track (`BeachLoop`, 0x6ceb10), picked by
/// `0x57492a` when the current ambience is Beach (mode 1, manager +0x108).
pub const BEACH_LOOP: u32 = 68_259;
/// *(code)* How many sound-manager updates kind 25 on (`0x573c64`) nudges
/// placed tracks' volumes for (`+0x114`, decremented in `0x573019`).
const NUDGE_UPDATES: u32 = 100;
/// *(code)* `fade_speed` written when starting a day and night loop
/// (`0x574a46`: 0x1388 = 5000).
const DAY_NIGHT_FADE_SPEED: i32 = 5000;
/// The `extvol` register, which `seqgroup_kill` fades out.
pub const EXTVOL: u8 = 19;
/// The fade registers: the value to reach, the register that moves, how
/// far it moves per block, and whether it is moving.
pub const FADE_DEST: u8 = 40;
pub const FADE_VAR: u8 = 41;
pub const FADE_SPEED: u8 = 42;
pub const FADE_ON: u8 = 45;
/// `source`, which `play_trk` hands on to the track it starts.
pub const SOURCE: u8 = 47;
/// `interrupt`: when not 0, a start of the track while it runs starts it
/// again (see `Player::start`). In the install only `loadb` writes it;
/// `set` can too (0x57e219 to 0x57e2da).
pub const INTERRUPT: u8 = 55;
/// `isplaying`, which a fresh start sets to 1.
pub const ISPLAYING: u8 = 46;
/// `Spl`, which the placement reads ([`crate::placement`]).
pub const SPL: u8 = 57;
/// A new thread's registers that do not start at 0, as the original's
/// default register banks hold them (written at 0x57c83a to 0x57c97b):
/// `priority`, `vol`, `extvol`, `pan`, `pitch`, `MinDistance`,
/// `MaxDistance`, `BufSizeMult`, `tempo` and `Spl`. A fresh start copies
/// both banks into the track's registers 17 to 47 and 50 to 62
/// (0x57da3a to 0x57da87); a register never written reads the bank's
/// value too (0x5802cf).
pub const DEFAULTS: [(u8, i32); 10] = [
    (17, 32),
    (18, 1024),
    (EXTVOL, 1024),
    (20, 512),
    (21, 3600),
    (28, 100),
    (29, 10000),
    (39, 1),
    (52, 120),
    (SPL, SPL_NORMAL),
];
/// `Spl`'s default, the HSM files' `spl_normal`.
pub const SPL_NORMAL: i32 = 20;
/// The registers a fresh start sets to their starting values, first and
/// last: the two default banks (0x57d727 to 0x57d741).
pub const BANKS: [(u8, u8); 2] = [(17, 47), (50, 62)];
/// Registers with side effects: `vol`, `duckpri`, 3D, `target` and the
/// global `main_duckpri`.
pub const VOL: u8 = 18;
pub const PAN: u8 = 20;
pub const PITCH: u8 = 21;
pub const DUCKPRI: u8 = 25;
pub const THREE_D: u8 = 26;
pub const TARGET: u8 = 53;
pub const MAIN_DUCKPRI: u8 = 123;
/// The music tracks and hit lists kind 36 uses (0x575170).
pub mod music {
    /// `MainMusicLoop`.
    pub const MAIN_MUSIC_LOOP: u32 = 515;
    /// `SetMainMusicSongAndHitList`.
    pub const SET_MAIN_MUSIC: u32 = 516;
    /// `SetMainAndHitListKillFirst`.
    pub const SET_MAIN_KILL_FIRST: u32 = 517;
    /// `FadeAndKillTrackSlow`, which fades out the track its first
    /// argument names.
    pub const FADE_AND_KILL_SLOW: u32 = 519;
    /// `river_loop`.
    pub const RIVER_LOOP: u32 = 2957;
    /// `Grimreaper_loop`.
    pub const GRIM_REAPER_LOOP: u32 = 5323;
    /// `mt_river_loop`.
    pub const MT_RIVER_LOOP: u32 = 114_370;
    /// `ui_nhood_bdoze_loop`.
    pub const NHOOD_DOZE_LOOP: u32 = 1551;
    /// The buy, build and neighbourhood mode lists.
    pub const BUY: u32 = 221;
    pub const BUILD: u32 = 222;
    pub const NEIGHBOURHOOD: u32 = 223;
    /// The lists of modes 10 to 18: `dt_nhood`, `Vacation`, `UnlSpeak`,
    /// Superstar's mode and station lists, then Makin' Magic's, its
    /// credits', build's and buy's.
    pub const EXPANSION_LISTS: [u32; 9] = [
        6_100, 14_093, 20_902, 100_102, 100_101, 103_100, 103_101, 103_102, 103_104,
    ];
    /// The expansions in, as a byte of the configuration
    /// (`[0x6e48bc]+0x66`) has them: every one, as in the memory image
    /// read (as [`crate::stations`] registers them all).
    pub const EXPANSIONS: u8 = 0xff;

    /// The credits' list (mode 6) for the expansions in: the newest's.
    pub fn credits(expansions: u8) -> u32 {
        [
            (0x80, 103_101),
            (0x40, 100_102),
            (0x20, 20_902),
            (0x08, 14_093),
            (0x02, 1_589),
            (0x04, 6_100),
        ]
        .into_iter()
        .find(|&(bit, _)| expansions & bit != 0)
        .map_or(NEIGHBOURHOOD, |(_, list)| list)
    }
}

/// How long `seqgroup_kill` waits after starting a fade, in milliseconds.
const SEQGROUP_WAIT_MS: u64 = 2;
/// Frames one call renders at most: an hour.
pub const RENDER_LIMIT: usize = 3600 * RATE as usize;
/// Frames a resampled sound holds at most: half an hour.
pub const RESAMPLE_LIMIT: usize = 1800 * RATE as usize;
/// Patches chained to one note at most.
pub const CHAIN: usize = 64;
/// Object instances recorded at most; more are not recorded.
pub const INSTANCES: usize = 4096;

/// *(code)* The tracks `radio_next_song` (kind 19) stops the notes of, in
/// the original's order.
pub const RADIO_TRACKS: [u32; 10] = [
    0x104, 0x10d, 0x10e, 0x118, 0x1a19, 0x1a1a, 0x1a1b, 0x1a1c, 0x1a1d, 0x18704,
];

/// *(code)* The tracks a pause leaves playing (0x5748a2 to 0x5748c3):
/// `ui_click` (1541), `MainMusicLoop` (515) and the speed buttons'
/// `ui_speed_1top` to `ui_speed_3top` (2727, 2733, 2739).
pub const PAUSE_SPARES: [u32; 5] = [0x605, 0x203, 0xaa7, 0xaad, 0xab3];

/// Event kinds: the HOT files' `kSndob` equates, and how the original's
/// dispatcher (0x573617) treats the rest.
pub mod kind {
    pub const PLAY: i64 = 1;
    pub const FADE: i64 = 18;
    pub const CANCEL_NOTE: i64 = 19;
    pub const KILL_ALL: i64 = 20;
    pub const PAUSE: i64 = 21;
    pub const UNPAUSE: i64 = 22;
    pub const KILL_INSTANCE: i64 = 23;
    pub const SET_ACTIVE: i64 = 25;
    pub const TURN_ON_TV: i64 = 30;
    pub const SET_MUSIC_MODE: i64 = 36;
    pub const PLAY_PIANO: i64 = 43;
    pub const SET_AMBIENCE_MODE: i64 = 52;
    pub const SEQUENCE_TRACK_HIT_LIST: i64 = 53;
    pub const KILL_OBJECT_SOUNDS: i64 = 105;
    /// *(code)* Kinds whose case does nothing for sound: the
    /// dispatcher's empty cases (27 to 29, 31 `kTurnOffTv`, 50, 51),
    /// debugging switches (44 to 49), a debugging dump (101) and a
    /// debugging flag (102).
    pub const NOTHING_FOR_SOUND: [i64; 14] =
        [27, 28, 29, 31, 44, 45, 46, 47, 48, 49, 50, 51, 101, 102];
    /// Kinds the original does something for that this player does not:
    /// reported when an event has one. (Kind 33 is ported for the game's
    /// own calls, [`super::Player::camera_moved`], which carry the
    /// camera; no event in the install has it.)
    pub const NOT_PORTED: [i64; 30] = [
        2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 24, 26, 32, 33, 34, 35, 37, 38, 39,
        40, 41, 42, 103, 104,
    ];
}

/// Where the player gets a patch's samples.
pub trait Samples {
    /// The decoded sample file at an install-relative path, if any.
    fn load(&mut self, path: &str) -> Option<Pcm>;
}

impl Samples for SoundFiles {
    fn load(&mut self, path: &str) -> Option<Pcm> {
        let bytes = self.get(path)?.read().ok()?;
        crate::decode::decode(&bytes)?.ok()
    }
}

impl Samples for BTreeMap<String, Pcm> {
    fn load(&mut self, path: &str) -> Option<Pcm> {
        self.get(path).cloned()
    }
}

/// What the player asks the game about objects. [`crate::sources`]
/// has the original's answers.
pub trait Sources {
    /// What a script asks about an object (`getsrcdatafield`): the
    /// answer for an object ID and a field, as both source registers hold
    /// them, or None for a field the original refuses (the destination
    /// then keeps its value).
    fn field(&self, source: i32, field: i32) -> Option<i32>;

    /// Where an object is, for its sound's volume and pan
    /// ([`crate::placement`]): `sims_sim::Object::position` (the
    /// original's object +0xf8, +0xf4 and +0x110, 0x570dc9), or None for
    /// an ID that names no object.
    fn position(&self, object: i32) -> Option<[i32; 3]>;

    /// Where an object is on the screen, with the view: asked for a
    /// track whose `Spl` (register 57) is not 0, which is every track,
    /// since `Spl` starts at 20 and no script writes it. Without it the
    /// placement is a fault, and the track keeps its `extvol` and `pan`.
    fn projection(&self, object: i32) -> Option<Projection>;

    /// *(code)* The lot's house number (global 10, `0x571093`:
    /// `[[0x6e48bc]+0x1c]+0x24`, sign-extended to 32 bits; 0 when no
    /// game is loaded). The day and night loop (`0x57492a`) uses it to
    /// skip [`NITE_LOOP`] on lots 20 to 29.
    fn lot(&self) -> i32 {
        0
    }
}

/// What [`Player::start`] did: the threads it started or refreshed
/// (none when it could not start), and whether it started one anew.
#[derive(Debug, Default)]
struct Started {
    threads: Vec<u32>,
    fresh: bool,
}

/// *(code)* What every start does first, whether or not the track runs
/// (0x57d9b2 to 0x57d9eb): it stops any fade and the end it was to bring
/// (`fade_on` and the flag at +0x1cf), points `target` back at the track
/// itself, and writes the argument registers 1 to 3, whatever the
/// track's argument kind (each a register write that sets the comparison
/// flags).
fn set_arguments(thread: &mut Thread, args: [u32; 3]) {
    thread.registers[usize::from(FADE_ON)] = 0;
    thread.kill_after_fade = false;
    thread.target = Target::Own;
    for (register, value) in (1..=3).zip(args) {
        thread.registers[register] = value as i32;
    }
    thread.kept = i64::from(args[2] as i32);
}

/// Answers 0 to every field, and has every object on the lot, at 1, 1
/// on the ground floor, in the middle of an 800 by 600 screen at the
/// near zoom: where the placement gives full volume in the centre.
pub struct NoSources;

/// Where [`NoSources`] has every object on the screen.
pub const MIDDLE: Projection = Projection {
    at: [400, 300],
    screen: [800, 600],
    zoom: 3,
    shown_level: 1,
};

impl Sources for NoSources {
    fn field(&self, _: i32, _: i32) -> Option<i32> {
        Some(0)
    }

    fn position(&self, _: i32) -> Option<[i32; 3]> {
        Some([1, 1, 1])
    }

    fn projection(&self, _: i32) -> Option<Projection> {
        Some(MIDDLE)
    }
}

#[derive(Debug, Clone)]
struct Thread {
    id: u32,
    /// The module whose program the thread is running: where its track's
    /// entry point, or the last `jump` or `call`, led ([`entry_points`]).
    module: usize,
    track: u32,
    pc: u32,
    registers: [i32; GLOBALS as usize],
    stack: Vec<Place>,
    loop_point: Place,
    kept: i64,
    hit_list: Vec<u32>,
    last_note: u32,
    wake: u64,
    /// The track's HOT group (`kGroupSfx` 1, `kGroupMusic` 2, `kGroupVox`
    /// 3), whose volume scales its own.
    group: i64,
    /// Where register reads and writes go: this thread, or the track
    /// register 53 named (see [`Player::resolve_target`]).
    target: Target,
    /// End the thread when its fade ends (the original's flag at +0x1cf).
    kill_after_fade: bool,
    /// Where the thread is in the original's list of running tracks:
    /// threads run in this order, from the next block. A fresh start and
    /// a let-go put it last (0x57dc9a, 0x57d017).
    order: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Own,
    /// A registered track object ID (`0x57e023` → `0x573312`), kept across
    /// stop and restart until `target` is written again or the holder starts
    /// anew (`0x57d9c3`) or is killed (`0x57dd87`).
    Track(u32),
}

/// Where a register read/write or placement routed through `target` lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    /// The starting thread itself.
    Own,
    /// A running thread in `self.threads`.
    Running(u32),
    /// A registered track object that is currently stopped (`is_running == 0`).
    Stopped(u32),
}

/// Default value of a track register when read on a stopped track object
/// (`0x5802cf`).
fn default_register(register: u8) -> i32 {
    DEFAULTS
        .iter()
        .find(|&&(r, _)| r == register)
        .map_or(0, |&(_, v)| v)
}

#[derive(Debug, Clone)]
struct Note {
    id: u32,
    /// The thread that started it: 0 for one a kind 53 event started on
    /// a track with no thread.
    thread: u32,
    track: u32,
    patch: u32,
    /// Position in source frames, 16.16 fixed point.
    position: u64,
    step: u64,
    /// Left and right gains, 16.16 fixed point.
    gains: [i32; 2],
    /// Patches to play after this one, without a gap, first first.
    chain: Vec<u32>,
    /// Whether it starts again from its beginning when it ends (an
    /// ambience cell's looping sample).
    looping: bool,
}

/// An ambience cell's sound: its note, and the volume and pan it was
/// given, to recompute its gains when the master volume changes.
#[derive(Debug, Clone)]
struct AmbientVoice {
    key: Key,
    volume: i32,
    pan: i32,
    note: Note,
}

/// Source frames per output frame, 16.16 fixed point, at a playback rate.
fn step(rate: u32) -> u64 {
    (u64::from(rate) << 16) / u64::from(RATE)
}

/// A sound at another rate, by linear interpolation in integers, so every
/// platform gets the same samples. `None` when the result would hold more
/// than [`RESAMPLE_LIMIT`] frames.
pub fn resample(pcm: &Pcm, rate: u32) -> Option<Pcm> {
    let channels = usize::from(pcm.channels);
    let length = pcm.frames() as u64;
    if pcm.rate == rate || pcm.rate == 0 || rate == 0 || channels == 0 || length == 0 {
        return Some(Pcm {
            rate,
            ..pcm.clone()
        });
    }
    // Source frames per output frame, 32.32 fixed point; at least 1 / 2^32,
    // as the rates are below 2^32.
    let step = (u128::from(pcm.rate) << 32) / u128::from(rate);
    let frames = usize::try_from((u128::from(length) << 32).div_ceil(step.max(1)))
        .ok()
        .filter(|&f| f <= RESAMPLE_LIMIT)?;
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        let position = step * frame as u128;
        let at = (position >> 32) as usize;
        let after = (at + 1).min(length as usize - 1);
        let fraction = (position & 0xffff_ffff) as i64;
        for channel in 0..channels {
            let first = i64::from(pcm.samples[at * channels + channel]);
            let second = i64::from(pcm.samples[after * channels + channel]);
            samples.push((first + (((second - first) * fraction) >> 32)) as i16);
        }
    }
    Some(Pcm {
        rate,
        channels: pcm.channels,
        samples,
    })
}

/// xorshift64*: the player's own random numbers, so a seed replays. A
/// number below `below` (at least 1).
fn xorshift(state: &mut u64, below: u32) -> u32 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    let value = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
    ((value >> 32) % u64::from(below.max(1))) as u32
}

/// Spreads a seed over all 64 bits (splitmix64), so nearby seeds start
/// far apart. Never 0, which xorshift cannot leave.
/// The registry of patch and track objects as the original fills it at
/// startup: the base game's tables, then the station patches, then the
/// expansions' and the downloads' tables ([`Registry::rank`]), each
/// module's in file order. Modules of one rank keep their order here.
fn registry(modules: &[Module], station_patches: &[(u32, String)]) -> Registry {
    let mut registry = Registry::new();
    let mut stations = Some(station_patches);
    for m in registration_order(modules) {
        if Registry::rank(&modules[m].name) > 0
            && let Some(ids) = stations.take()
        {
            ids.iter().for_each(|(id, _)| registry.insert(*id));
        }
        for &(id, _) in &modules[m].hot.registered {
            registry.insert(id);
        }
    }
    if let Some(ids) = stations {
        ids.iter().for_each(|(id, _)| registry.insert(*id));
    }
    registry
}

/// The modules in the original's order of registration
/// ([`Registry::rank`]), as indices.
fn registration_order(modules: &[Module]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..modules.len()).collect();
    order.sort_by_key(|&m| Registry::rank(&modules[m].name));
    order
}

/// *(code)* Each track's and each patch's module: the one whose line
/// registered it last. The registry holds one object per ID, and an ID
/// registered again replaces its object (0x57ce04 to 0x57ce32), so every
/// start of a track looks up that object (0x573312) and runs from its
/// descriptor (0x57db7a to 0x57db91), whichever module asked.
///
/// Patches and tracks share the registry, so an ID registered again as
/// the other kind is that kind only: it is left out of the other map. In
/// the install, track 68262's `[Track]` line comes before a `[Patch]` line
/// of the same ID in its file, and patches 6401 and 6402 are replaced by
/// tracks of a download registered later. A start that finds a patch
/// object starts nothing (its slot returns at once, 0x57bf80), and a note
/// whose patch is a track object fails (0x57f6ac to 0x57f703).
///
/// The station patches ([`Registrant::Station`]) register through the same routine
/// (0x5750dc calls 0x57cdd5), after the base game's tables and before the
/// expansions', with numbers their allocator hands out without looking
/// at the registry (0x57d386): one replaces a base game object of its
/// number, and an expansion's or a download's replaces it.
fn owners(
    modules: &[Module],
    station_patches: &[(u32, String)],
) -> (BTreeMap<u32, usize>, BTreeMap<u32, (Registrant, String)>) {
    let mut tracks = BTreeMap::new();
    let mut patches = BTreeMap::new();
    let mut stations = Some(station_patches);
    let add_stations = |tracks: &mut BTreeMap<u32, usize>,
                        patches: &mut BTreeMap<u32, (Registrant, String)>,
                        stations: &mut Option<&[(u32, String)]>| {
        for (at, (id, path)) in stations.take().into_iter().flatten().enumerate() {
            patches.insert(*id, (Registrant::Station(at), path.clone()));
            tracks.remove(id);
        }
    };
    for m in registration_order(modules) {
        if Registry::rank(&modules[m].name) > 0 {
            add_stations(&mut tracks, &mut patches, &mut stations);
        }
        let hot = &modules[m].hot;
        for &(id, object) in &hot.registered {
            match object {
                Object::Track => {
                    tracks.insert(id, m);
                    patches.remove(&id);
                }
                Object::Patch => {
                    patches.insert(id, (Registrant::Module(m), hot.patches[&id].path.clone()));
                    tracks.remove(&id);
                }
            }
        }
    }
    add_stations(&mut tracks, &mut patches, &mut stations);
    (tracks, patches)
}

/// Whose line registered a patch: a module's, by index, or the station
/// patch made `n`th ([`Player::add_stations`]). Each registration makes a
/// new object (0x5750a0 to 0x5750c5), so two are never the same object,
/// whatever their samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Registrant {
    Module(usize),
    Station(usize),
}

/// A place in a module's program: the module and the address.
type Place = (usize, u32);

/// *(code)* Each track data ID's place: the module whose `[TrackData]`
/// line registered it last, and the address there. The original keeps one
/// table of them for every module (manager +0x168): each line stores the
/// address in the program being loaded (0x57ce70 to 0x57ce98), so an ID
/// registered again points into the later module's program. A track's
/// entry point (0x57dd8d, 0x57dbe2), and every `jump`, `call` and `if`
/// (0x57e3b5, 0x57e3d4), look an ID up in it, whichever module's program
/// is running, so a thread can go on in another module's program.
fn entry_points(modules: &[Module]) -> BTreeMap<u32, Place> {
    let mut places = BTreeMap::new();
    for m in registration_order(modules) {
        for (&id, &address) in &modules[m].hot.track_data {
            places.insert(id, (m, address));
        }
    }
    places
}

/// *(code)* The hit lists, each from the module whose line registered it
/// last: the HOT reader makes a new list for each `[GlobalHitList]` line
/// (0x57ba20) and the table keeps the newest for its ID (0x57d261). The
/// stations add to the lists there are when they are built (0x574c9e to
/// 0x574cb7, 0x574e74), after the base game's tables and before the
/// expansions', so an expansion's list of the same ID replaces what they
/// added.
fn hit_lists(modules: &[Module], stations: &BTreeMap<u32, Vec<u32>>) -> BTreeMap<u32, Vec<u32>> {
    let mut lists: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut pending = Some(stations);
    for m in registration_order(modules) {
        if Registry::rank(&modules[m].name) > 0
            && let Some(stations) = pending.take()
        {
            add_station_lists(&mut lists, stations);
        }
        for (&id, list) in &modules[m].hot.hit_lists {
            lists.insert(id, list.clone());
        }
    }
    if let Some(stations) = pending {
        add_station_lists(&mut lists, stations);
    }
    lists
}

fn add_station_lists(lists: &mut BTreeMap<u32, Vec<u32>>, stations: &BTreeMap<u32, Vec<u32>>) {
    for (&id, list) in stations {
        lists.entry(id).or_default().extend(list);
    }
}

fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)).max(1)
}

/// What one instruction did to its thread.
enum Flow {
    Next,
    Jump(Place),
    /// Stay on this instruction and run it again next block.
    Block,
    /// Go on to the next instruction next block.
    Yield,
    /// Go to this address next block.
    JumpYield(Place),
    End,
    /// End the thread, whatever called it, and stop its notes.
    Kill,
}

/// The HIT VM and mixer.
pub struct Player<S> {
    modules: Vec<Module>,
    samples: S,
    /// Patch ID to (registrant, path): whose line registered it last
    /// ([`owners`]). An ID whose last line is
    /// a track's is not here.
    patches: BTreeMap<u32, (Registrant, String)>,
    /// Track ID to the module whose line registered it last
    /// ([`owners`]): the module every start of the track runs from. An ID
    /// whose last line is a patch's is not here.
    tracks: BTreeMap<u32, usize>,
    /// Track data ID to its place ([`entry_points`]).
    track_data: BTreeMap<u32, Place>,
    /// Hit list ID to its entries ([`hit_lists`]).
    hit_lists: BTreeMap<u32, Vec<u32>>,
    /// The stations' hit lists added, merged.
    station_lists: BTreeMap<u32, Vec<u32>>,
    /// *(code)* The registry of patch and track objects (manager +0x80),
    /// for the order the unpause goes through them.
    registry: Registry,
    /// The station patches added, in the order they were made, with their
    /// samples' paths.
    station_patches: Vec<(u32, String)>,
    cache: BTreeMap<u32, Option<Pcm>>,
    globals: [i32; 256 - GLOBALS as usize],
    threads: Vec<Thread>,
    /// Whether a block has run out of instructions; reported once.
    over_budget: bool,
    /// Sequence groups: the group a register names, to the ID of the
    /// thread that last claimed it with `seqgroup_kill` or
    /// `seqgroup_return`. A slot keeps its ID after that thread ends.
    seqgroups: BTreeMap<i32, u32>,
    /// `main_duckpri` requests by thread; the global holds the largest.
    duck: BTreeMap<u32, i32>,
    /// Group volumes set by the host (the options' sliders), 1024 when
    /// not set.
    group_volumes: BTreeMap<i64, i32>,
    /// The master volume, 0 to 1024.
    master: i32,
    notes: Vec<Note>,
    /// The registers kind 53 played a track's own note with when no
    /// thread of it ran ([`Player::sequence`]), by track, kept while
    /// that note (thread 0) plays: the original's idle track object, for
    /// a camera move to place.
    idle: BTreeMap<u32, Thread>,
    /// The thread each track's last start made or let go, by track: the
    /// one whose `target` the original's track object still has
    /// ([`Player::camera_moved`]). Every start sets that `target` back
    /// to itself (0x57d9c3).
    last_start: BTreeMap<u32, u32>,
    /// *(code)* Object instances: a track and an object it plays for, as
    /// the original records them (its hash multimap at manager +0xd4),
    /// each pair once, and each track's in the order the original goes
    /// through them ([`Player::record`]).
    instances: Vec<(u32, u32)>,
    /// *(code)* Each track object's own object (+0xb0, read by vtable
    /// +0x24): set only by a fresh start of a `kArgsIdVolPan` track
    /// (0x57db2e), so a start that lets a held track go or is refused
    /// leaves it. 0 until then, as the track's constructor leaves it.
    track_objects: BTreeMap<u32, u32>,
    /// The ambience scores, by mode (see [`crate::ambience`]).
    scores: BTreeMap<u32, Fresh>,
    /// *(code)* The current ambience (manager +0x100), by mode.
    ambience: Option<u32>,
    /// *(code)* The music mode (manager +0xe8, 4 at first), and whether
    /// mode 3 has been set, after which modes are recorded (0x6f6428).
    music_mode: i64,
    music_started: bool,
    /// *(code)* Whether sound is active (manager +0xc, `true` at
    /// startup, `0x5712e9`; set by kind 25, `0x573c64`).
    active: bool,
    /// *(code)* Whether the sound is paused (manager +0xd).
    paused: bool,
    /// *(code)* Volume-nudge countdown (manager +0x114, `0` at startup,
    /// `0x571433`): set to `100` by kind 25 on (`0x573c64`) and counted
    /// down once per update (`0x573019`). While nonzero, `place_track`
    /// (`0x574701`) writes `vol - 1` (or `1` for `vol <= 0`).
    nudge: u32,
    /// *(code)* The remembered day and night loop track (`0x6f6424`),
    /// updated by `0x574978`.
    day_night: Option<u32>,
    /// *(code)* The tracks a pause holds (each track's +0x1c8): their
    /// threads do not run or fade and their notes stay where they are,
    /// silent, until an unpause or until the track starts again.
    held: BTreeSet<u32>,
    /// The ambience cells' sounds.
    voices: Vec<AmbientVoice>,
    /// Ambience sample paths to their numbers in the patch cache.
    ambient_patches: BTreeMap<String, u32>,
    /// The ambience samples, by their number in `ambient_patches`: kept
    /// apart from `cache` so no HOT patch number can name one.
    ambient_cache: BTreeMap<u32, Option<Pcm>>,
    frame: u64,
    /// The frame at which the threads last ran.
    ran_at: Option<u64>,
    next_id: u32,
    /// The next [`Thread::order`].
    next_order: i64,
    random: u64,
    /// What went wrong while running, in order: unknown events, missing
    /// samples, threads ended for a bad instruction. At most [`FAULTS`].
    pub faults: Vec<String>,
}

impl<S: Samples> Player<S> {
    /// A player over these modules (sorted by name, as
    /// [`Module::load_all`] returns them). `seed` starts its random
    /// numbers.
    pub fn new(modules: Vec<Module>, samples: S, seed: u64) -> Self {
        let (tracks, patches) = owners(&modules, &[]);
        let track_data = entry_points(&modules);
        let hit_lists = hit_lists(&modules, &BTreeMap::new());
        let registry = registry(&modules, &[]);
        Self {
            modules,
            samples,
            patches,
            tracks,
            track_data,
            hit_lists,
            station_lists: BTreeMap::new(),
            registry,
            station_patches: Vec::new(),
            cache: BTreeMap::new(),
            globals: [0; 256 - GLOBALS as usize],
            threads: Vec::new(),
            over_budget: false,
            seqgroups: BTreeMap::new(),
            duck: BTreeMap::new(),
            group_volumes: BTreeMap::new(),
            master: 1024,
            notes: Vec::new(),
            idle: BTreeMap::new(),
            last_start: BTreeMap::new(),
            instances: Vec::new(),
            track_objects: BTreeMap::new(),
            scores: BTreeMap::new(),
            ambience: None,
            music_mode: 4,
            music_started: false,
            active: true,
            paused: false,
            nudge: 0,
            day_night: None,
            held: BTreeSet::new(),
            voices: Vec::new(),
            ambient_patches: BTreeMap::new(),
            ambient_cache: BTreeMap::new(),
            frame: 0,
            ran_at: None,
            next_id: 1,
            next_order: 0,
            random: splitmix(seed),
            faults: Vec::new(),
        }
    }

    /// Sets a global register (100 to 255), such as `SimSpeed`.
    pub fn set_global(&mut self, register: u8, value: i32) {
        assert!(register >= GLOBALS, "register {register} is not global");
        self.globals[usize::from(register - GLOBALS)] = value;
    }

    /// Sets a track group's volume, 0 to 1024 (the options' sliders):
    /// `kGroupSfx` 1, `kGroupMusic` 2, `kGroupVox` 3.
    ///
    /// *(code)* The effects group's volume is also every ambience
    /// player's volume, as kind 37 sets both (to its slider's step,
    /// table 0x67156c).
    pub fn set_group_volume(&mut self, group: i64, volume: i32) {
        let volume = volume.clamp(0, 1024);
        self.group_volumes.insert(group, volume);
        if group == 1 {
            let modes: Vec<u32> = self.scores.keys().copied().collect();
            for mode in modes {
                self.with_score(mode, |score, host| score.set_volume(volume, host));
            }
        }
        self.apply_levels();
    }

    /// Sets the master volume, 0 to 1024.
    pub fn set_master_volume(&mut self, volume: i32) {
        self.master = volume.clamp(0, 1024);
        for voice in &mut self.voices {
            voice.note.gains = ambient_gains(voice.volume, voice.pan, self.master);
        }
        self.apply_levels();
    }

    /// Adds an ambience score for a mode (see [`ambience::SCORES`]), its
    /// samples in `directory`. False (reported) when the score is larger
    /// than the original's grid. The score takes the effects group's
    /// volume if one was set (the original makes its players before the
    /// options set it); one that replaces a started score starts.
    pub fn add_score(&mut self, mode: u32, score: &Score, directory: &str) -> bool {
        match Fresh::new(mode, score, directory, RATE) {
            Some(fresh) => {
                let mut started = false;
                if let Some(mut old) = self.scores.insert(mode, fresh) {
                    started = old.is_started();
                    old.stop(&mut self.host());
                }
                let volume = self.group_volumes.get(&1).copied();
                let frame = self.frame;
                self.with_score(mode, |score, host| {
                    if let Some(volume) = volume {
                        score.set_volume(volume, host);
                    }
                    if started {
                        score.start(frame);
                    }
                });
                true
            }
            None => {
                self.report(format!("ambience {mode}: score larger than the grid"));
                false
            }
        }
    }

    /// Adds every score of [`ambience::SCORES`] the install has; a score
    /// that does not parse is reported.
    pub fn add_scores(&mut self, files: &SoundFiles) {
        for (mode, directory, file) in ambience::SCORES {
            let Some(entry) = files.get(&format!("{directory}\\{file}")) else {
                continue;
            };
            match entry.read().map(|bytes| Score::parse(&bytes)) {
                Ok(Ok(score)) => {
                    self.add_score(mode, &score, directory);
                }
                Ok(Err(error)) => self.report(format!("{}: {error}", entry.path)),
                Err(error) => self.report(format!("{}: {error}", entry.path)),
            }
        }
    }

    /// The current ambience's player, if any.
    pub fn ambience(&self) -> Option<&Fresh> {
        self.ambience.and_then(|mode| self.scores.get(&mode))
    }

    /// An ambience score's player by mode (`0..=3`), whether or not it is
    /// current.
    pub fn score(&self, mode: u32) -> Option<&Fresh> {
        self.scores.get(&mode)
    }

    /// *(code)* Kind 33 (0x574157), which the original's game sends when
    /// the camera moves (from five places: 0x4a80bb, 0x4a8d38, 0x4ce50a,
    /// 0x4ced58 and 0x4f66c6, through 0x57a9d4 and 0x57aac5). It places
    /// every track with object instances again from where its objects
    /// are now (0x57447f), then moves the ambience's cursor as
    /// [`Player::set_ambience_view`] does. `sources` is as for
    /// [`Player::handle`], with the screen as the moved camera shows it.
    ///
    /// The tracks go a track's instances at a time, in the order of the
    /// instance map (0x5746d9 for each). For each, the volume and pan
    /// come from the same sum as kind 1's ([`crate::placement`]), with
    /// the track's `Spl`, and are written as the track writes them,
    /// `extvol` then `pan` (vtable +0x40 and +0x48, 0x57d57a and
    /// 0x57d5a0). The write goes through the track's `target`
    /// (0x57de57). Here it goes to the running thread the track's last
    /// start made, which stands for the original's one track object, and
    /// on through its `target` ([`Player::place_through`]), not past a
    /// thread that is not its track's last start. A fade on the `extvol`
    /// the write lands in is undone by it and goes on from the new
    /// volume.
    ///
    /// A script that reached `end` while its note plays stays running
    /// while that note fades out (0x57dca3), so the write reaches it
    /// through its running thread too. A note kind 53 played with no
    /// thread running (thread 0) is placed from the registers it started
    /// with. `Spl` is that of the thread the last start's write lands in
    /// ([`Player::place_through`]), or else of the newest running thread
    /// or the kind 53 note, in that order. A thread that passes the
    /// write on keeps its notes' levels. A track with none of these gets
    /// nothing: the original writes to its idle track object, whose
    /// registers the next start sets again.
    ///
    /// While the kind 25 volume-nudge countdown (`+0x114`, set to 100
    /// by [`Player::set_active`]`(true)` and counted down once per
    /// update at `0x573019`) is nonzero, each placement writes `vol - 1`,
    /// or `1` when `vol <= 0` (`0x574701`). Last, it updates the day and
    /// night loop (`0x574978`: [`NITE_LOOP`] or [`BEACH_LOOP`]).
    pub fn camera_moved(&mut self, view: i32, hour: u32, sources: &dyn Sources) {
        self.place_all(sources);
        self.set_ambience_view(view, hour);
        self.update_day_night(sources);
    }

    /// *(code)* 0x57447f: places every track with object instances again,
    /// in the order of the instance map.
    fn place_all(&mut self, sources: &dyn Sources) {
        self.forget_silent();
        let mut tracks: Vec<u32> = Vec::new();
        for &(track, _) in &self.instances {
            if !tracks.contains(&track) {
                tracks.push(track);
            }
        }
        for track in tracks {
            self.place_track(track, sources);
        }
    }

    /// Drops idle kind-53 register state once its note has stopped.
    fn forget_silent(&mut self) {
        let notes = &self.notes;
        self.idle.retain(|&track, _| {
            notes
                .iter()
                .any(|note| note.thread == 0 && note.track == track)
        });
    }

    /// Places a track again from where its objects are (0x5746d9), as
    /// [`Player::camera_moved`] describes. The caller forgets silent
    /// idle tracks first ([`Player::forget_silent`]): a placement changes
    /// no note's life, so a camera move does it once for all its tracks.
    fn place_track(&mut self, track: u32, sources: &dyn Sources) {
        let Some(spl) = self.track_spl(track) else {
            return;
        };
        let last = self.last_start.get(&track).copied();
        let last_running = last.filter(|&id| self.threads.iter().any(|t| t.id == id));
        let Some((vol, pan)) = self.placed_with(track, spl, sources) else {
            return;
        };
        // *(code)* 0x574701: while `+0x114` is nonzero, write `vol - 1`,
        // or `1` when that is negative.
        let vol = if self.nudge != 0 {
            let nudged = vol.wrapping_sub(1);
            if nudged < 0 { 1 } else { nudged }
        } else {
            vol
        };
        // The last start's thread, running or about to, is written as
        // it would write; with none, the newest running one takes it
        // itself.
        if let Some(id) = last_running {
            self.place_through(id, vol, pan);
        } else if let Some(at) = self.newest(track) {
            place_itself(&mut self.threads[at], vol, pan);
            let thread = self.threads[at].clone();
            let levels = self.levels(&thread);
            for note in self.notes.iter_mut().filter(|n| n.thread == thread.id) {
                note.gains = levels.gains;
                note.step = step(levels.rate);
            }
        }
        self.place_idle(track, vol, pan);
    }

    /// Places a track's idle kind-53 thread (`self.idle`), updating its
    /// playing note immediately.
    fn place_idle(&mut self, track: u32, vol: i32, pan: i32) {
        if let Some(mut idle) = self.idle.remove(&track) {
            self.write(&mut idle, i64::from(EXTVOL), vol);
            self.write(&mut idle, i64::from(PAN), pan);
            let levels = self.levels(&idle);
            for note in self
                .notes
                .iter_mut()
                .filter(|n| n.thread == 0 && n.track == track)
            {
                note.gains = levels.gains;
                note.step = step(levels.rate);
            }
            self.idle.insert(track, idle);
        }
    }

    /// *(code)* The ambience half of kind 33 (0x574157): moves the
    /// current ambience's cursor. `view` is the
    /// share of the screen's tiles that are outdoors, in percent, which
    /// the original computes from the camera ([`crate::ambience::view`],
    /// 0x570c3e; below 30 counts as 0, as does any value while a flag of
    /// the sound manager, not identified, is set): the row goes to `view
    /// * 10` per mille. `hour` picks the column from [`ambience::HOUR_COLUMNS`]
    /// (`column * 1000 / 13` per mille); an hour past 23 leaves it.
    pub fn set_ambience_view(&mut self, view: i32, hour: u32) {
        let Some(mode) = self.ambience else {
            return;
        };
        let view = if view < 30 { 0 } else { view };
        let column = ambience::HOUR_COLUMNS.get(hour as usize).copied();
        if let Some(score) = self.scores.get_mut(&mode) {
            score.set_x(view.wrapping_mul(10));
            if let Some(column) = column {
                score.set_y(column * 1000 / 13);
            }
        }
    }

    /// Runs `act` on a score's player with the rest of the player as its
    /// host.
    fn with_score(&mut self, mode: u32, act: impl FnOnce(&mut Fresh, &mut dyn Host)) {
        let Some(mut score) = self.scores.remove(&mode) else {
            return;
        };
        act(&mut score, &mut self.host());
        self.scores.insert(mode, score);
    }

    fn host(&mut self) -> AmbientHost<'_, S> {
        AmbientHost {
            samples: &mut self.samples,
            cache: &mut self.ambient_cache,
            ambient_patches: &mut self.ambient_patches,
            voices: &mut self.voices,
            random: &mut self.random,
            master: self.master,
            faults: &mut self.faults,
        }
    }

    /// *(code)* Kind 52 (0x573f7a): makes the score of a mode (the event's
    /// track) the current ambience. Nothing happens when it already is;
    /// otherwise the current one stops and the new one starts (paused if
    /// the sound is paused, 0x573fd4). (A mode with no score crashes the
    /// original; here the current one stops and none is current.)
    fn set_ambience_mode(&mut self, mode: u32) {
        let new = self.scores.contains_key(&mode).then_some(mode);
        if new == self.ambience {
            return;
        }
        if let Some(current) = self.ambience {
            self.with_score(current, |score, host| score.stop(host));
        }
        self.ambience = new;
        if let Some(new) = new {
            let frame = self.frame;
            let paused = self.paused;
            self.with_score(new, |score, _| {
                score.start(frame);
                score.set_paused(paused);
            });
        }
    }

    /// Kind 36 (0x573fe2): *(code)* the music mode is recorded (once mode
    /// 3 has been set), with its music ([`Player::change_music`]) when it
    /// changes, and then, on every event (the same mode again included),
    /// for the ambience: in mode 0 (live), the current ambience starts if
    /// it is stopped and takes the pause state (0x57400d); in mode 3 (the
    /// neighbourhood) the backyard score becomes current and starts
    /// unpaused (0x574072), its cursor first set to (1000, 500) per mille,
    /// which starting resets when it was stopped; in any other mode the
    /// current ambience stops. Last, it updates the day and night loop
    /// (0x5741b7 → 0x574978).
    fn set_music_mode(&mut self, mode: i64, sources: &dyn Sources) {
        if mode != self.music_mode {
            if mode == 3 {
                self.music_started = true;
            }
            if self.music_started {
                let previous = std::mem::replace(&mut self.music_mode, mode);
                self.change_music(mode, previous);
            }
        }
        let frame = self.frame;
        match self.music_mode {
            0 => {
                if let Some(current) = self.ambience {
                    let paused = self.paused;
                    self.with_score(current, |score, _| {
                        score.start(frame);
                        score.set_paused(paused);
                    });
                }
            }
            3 => {
                if self.scores.contains_key(&0) {
                    if let Some(current) = self.ambience.filter(|&m| m != 0) {
                        self.with_score(current, |score, host| score.stop(host));
                    }
                    self.ambience = Some(0);
                    self.with_score(0, |score, _| {
                        score.set_x(1000);
                        score.set_y(500);
                        score.start(frame);
                        score.set_paused(false);
                    });
                }
            }
            _ => {
                if let Some(current) = self.ambience {
                    self.with_score(current, |score, host| score.stop(host));
                }
            }
        }
        self.update_day_night(sources);
    }

    /// The music half of kind 36 (0x575170): *(code)* what a change of
    /// music mode does to the music tracks, which their scripts then play
    /// from the music hit lists ([`crate::stations`]).
    ///
    /// - The river loops stop at once ([`Player::kill_track`]):
    ///   `river_loop` but in mode 3 (the neighbourhood), and
    ///   `mt_river_loop` but in mode 15 (Makin' Magic's). In mode 3 the
    ///   Grim Reaper's loop stops too, when the first expansion is in
    ///   ([`EXPANSIONS`]).
    /// - Modes 0 (live), 4 and 9 (`bkground_fade`): `MainMusicLoop` stops
    ///   at once after buy or build (modes 1, 2, 17, 18), and otherwise
    ///   `FadeAndKillTrackSlow` starts for it.
    /// - Modes 1 (buy) and 2 (build): `SetMainMusicSongAndHitList`
    ///   starts for the hit list 221 or 222.
    /// - Mode 3: unless the mode before was 8 (the family screen), a slow
    ///   fade in progress stops and `SetMainAndHitListKillFirst` starts
    ///   for the neighbourhood's list, 223; then `river_loop` starts.
    /// - Mode 6 (the credits): `SetMainAndHitListKillFirst` for the list
    ///   of the newest expansion in (by [`EXPANSIONS`]), or 223.
    /// - Mode 8: the river loops and `ui_nhood_bdoze_loop` fade out and
    ///   end ([`Player::fade_and_kill`]).
    /// - Modes 10 to 18, the expansions': a slow fade in progress stops
    ///   and `SetMainAndHitListKillFirst` starts for the mode's list;
    ///   mode 15 then starts `mt_river_loop`. 17 and 18 (Makin' Magic's
    ///   build and buy) do nothing after each other.
    /// - Modes 5 (`bkground_load`) and 7 (the options): nothing.
    ///
    /// The tracks are started as kind 1 starts them, with the hit list as
    /// the first argument (vtable +0x2c). The original waits 100 ms on
    /// its own thread around most of these starts (`Sleep`), so its
    /// sound thread runs between; here the events are acted on at once,
    /// in order, with nothing between.
    fn change_music(&mut self, mode: i64, previous: i64) {
        use music::*;
        if mode == 3 {
            if EXPANSIONS & 1 != 0 {
                self.kill_track(GRIM_REAPER_LOOP);
            }
        } else {
            self.kill_track(RIVER_LOOP);
        }
        if mode != 15 {
            self.kill_track(MT_RIVER_LOOP);
        }
        // *(code)* Each start lets the track go (vtable +0x2c, 0x57523f).
        let start = |player: &mut Self, track: u32, list: u32| {
            let started = player.start([list, 0, 0], track);
            player.let_go_track(track, &started.threads);
        };
        let switch = |player: &mut Self, list: u32| {
            player.kill_track(FADE_AND_KILL_SLOW);
            start(player, SET_MAIN_KILL_FIRST, list);
        };
        match mode {
            0 | 4 | 9 => {
                if matches!(previous, 1 | 2 | 17 | 18) {
                    self.kill_track(MAIN_MUSIC_LOOP);
                } else {
                    start(self, FADE_AND_KILL_SLOW, MAIN_MUSIC_LOOP);
                }
            }
            1 => start(self, SET_MAIN_MUSIC, BUY),
            2 => start(self, SET_MAIN_MUSIC, BUILD),
            3 => {
                if previous != 8 {
                    switch(self, NEIGHBOURHOOD);
                }
                start(self, RIVER_LOOP, 0);
            }
            6 => start(self, SET_MAIN_KILL_FIRST, credits(EXPANSIONS)),
            8 => {
                for track in [RIVER_LOOP, MT_RIVER_LOOP, NHOOD_DOZE_LOOP] {
                    self.fade_and_kill(track);
                }
            }
            10..=14 | 16 => switch(self, EXPANSION_LISTS[(mode - 10) as usize]),
            15 => {
                switch(self, EXPANSION_LISTS[5]);
                start(self, MT_RIVER_LOOP, 0);
            }
            17 | 18 if !matches!(previous, 17 | 18) => {
                switch(self, EXPANSION_LISTS[(mode - 10) as usize]);
            }
            _ => {}
        }
    }

    /// Ends a track at once (vtable +0x38, 0x57dd3e): *(code)* its
    /// threads and its notes, whether or not they were fading. A track
    /// that is not running is left alone.
    fn kill_track(&mut self, track: u32) {
        self.end_track(track, None);
    }

    /// [`Player::kill_track`], sparing the thread running (`kill_trk`
    /// ends it on its own).
    fn end_track(&mut self, track: u32, running: Option<u32>) {
        let spared = |id: u32| running == Some(id);
        let ended: Vec<u32> = self
            .threads
            .iter()
            .filter(|t| t.track == track && !spared(t.id))
            .map(|t| t.id)
            .collect();
        self.threads.retain(|t| !ended.contains(&t.id));
        self.notes
            .retain(|n| !(n.track == track && !spared(n.thread)));
        self.forget_ended();
    }

    /// Updates every started ambience player, once per block.
    fn update_ambience(&mut self) {
        let modes: Vec<u32> = self
            .scores
            .iter()
            .filter(|(_, s)| s.is_started())
            .map(|(&m, _)| m)
            .collect();
        let frame = self.frame;
        for mode in modes {
            self.with_score(mode, |score, host| score.update(frame, host));
        }
    }

    /// Frames mixed so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Whether nothing is running or playing.
    pub fn is_idle(&self) -> bool {
        self.threads.is_empty()
            && self.notes.is_empty()
            && self.voices.is_empty()
            && self.scores.values().all(|s| !s.is_started())
    }

    /// Adds the stations' hit lists and patches (see
    /// [`crate::stations`]), which the original builds from directories
    /// when it starts. A hit list the sound tables already list is added
    /// to, as [`hit_lists`] says; a patch replaces a base game object of
    /// its number, and an expansion's or a download's replaces it, as
    /// [`owners`] says.
    pub fn add_stations(&mut self, lists: &StationLists) {
        add_station_lists(&mut self.station_lists, &lists.hit_lists);
        self.hit_lists = hit_lists(&self.modules, &self.station_lists);
        for (&id, path) in &lists.patches {
            // Numbered in the order they are made, each registered then
            // (0x574f0e).
            self.station_patches.push((id, path.clone()));
        }
        let (tracks, patches) = owners(&self.modules, &self.station_patches);
        // An object replaced goes, with what it was doing (0x5750dc calls
        // 0x57cdd5, which destroys it, 0x57ce04 to 0x57ce32). The original
        // builds the stations before any sound plays; here they may come
        // later.
        let replaced: BTreeSet<u32> = self
            .tracks
            .iter()
            .filter(|&(id, owner)| tracks.get(id) != Some(owner))
            .map(|(&id, _)| id)
            .chain(
                self.patches
                    .iter()
                    .filter(|&(id, owner)| patches.get(id) != Some(owner))
                    .map(|(&id, _)| id),
            )
            .collect();
        (self.tracks, self.patches) = (tracks, patches);
        for id in replaced {
            self.destroy(id);
        }
        // The original builds the stations between the base game's
        // tables and the expansions' ([`crate::registry`]).
        self.registry = registry(&self.modules, &self.station_patches);
    }

    /// Acts on one event from the simulation. `sources` is where the
    /// lot's objects are: a sound started for an object is placed when
    /// the event comes, as the original places it ([`crate::placement`]).
    pub fn handle(&mut self, event: &SoundEvent, sources: &dyn Sources) {
        match event {
            SoundEvent::Play { object, name } => self.play(*object, name, sources),
            // What the game calls with kind 23 when an object goes.
            SoundEvent::StopObject { object } => self.kill_instance(*object, sources),
            // Kind 20, as the game calls it.
            SoundEvent::StopAll => self.kill_all(),
            // Kinds 21 and 22, as the game calls them (0x57a7f2, 0x57a8e3).
            SoundEvent::Pause => self.pause(),
            SoundEvent::Unpause => self.unpause(sources),
        }
    }

    fn play(&mut self, object: u32, name: &str, sources: &dyn Sources) {
        match (0..self.modules.len()).find(|&m| self.modules[m].hot.event(name).is_some()) {
            Some(module) => self.play_from(module, object, name, sources),
            None => self.report(format!("no event {name}")),
        }
    }

    /// Acts on the event of that name as one module defines it, whether
    /// or not an earlier module's definition would win in
    /// [`SoundEvent::Play`]. The module is an index into the modules
    /// given to [`Player::new`]. `sources` is as for [`Player::handle`].
    ///
    /// *(code)* As the original's event dispatcher (0x573617) does, the
    /// event's kind picks what happens, with its track and three
    /// arguments. Each argument is the event's own number after the
    /// track when that is not 0, and otherwise the caller's: the object
    /// for the first, 0 for the others.
    pub fn play_from(&mut self, module: usize, object: u32, name: &str, sources: &dyn Sources) {
        let Some(event) = self.modules.get(module).and_then(|m| m.hot.event(name)) else {
            self.report(format!("no event {name}"));
            return;
        };
        let owner = &self.modules[module];
        let kind = owner.number(&event.kind);
        let track = event.track.as_ref().and_then(|t| owner.number(t));
        let numbers: Vec<i64> = (0..3)
            .map(|at| {
                event
                    .rest
                    .get(at)
                    .and_then(|v| owner.number(v))
                    .unwrap_or(0)
            })
            .collect();
        let Some(kind) = kind else {
            self.report(format!("event {name}: its kind is not a number"));
            return;
        };
        // The original's fields are 32 bits; a number that fits neither
        // signed nor unsigned 32 bits is a fault, not a wrapped value.
        let fits = |v: i64| (i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&v);
        let track = track.unwrap_or(0);
        if !fits(track) || numbers.iter().any(|&v| !fits(v)) {
            self.report(format!("event {name}: a number does not fit 32 bits"));
            return;
        }
        let given = |at: usize, caller: u32| match numbers[at] {
            0 => caller,
            v => v as u32,
        };
        let args = [given(0, object), given(1, 0), given(2, 0)];
        let track = track as u32;
        match kind {
            kind::PLAY => self.play_track(track, args, sources),
            kind::FADE => self.fade_track(track, args),
            kind::CANCEL_NOTE => self.cancel_radio_notes(),
            kind::KILL_ALL => self.kill_all(),
            kind::PAUSE => self.pause(),
            kind::UNPAUSE => self.unpause(sources),
            kind::KILL_INSTANCE => self.kill_instance(track, sources),
            kind::SET_ACTIVE => self.set_active(track != 0, sources),
            kind::SET_AMBIENCE_MODE => self.set_ambience_mode(track),
            kind::SET_MUSIC_MODE => self.set_music_mode(i64::from(track as i32), sources),
            kind::TURN_ON_TV => self.turn_on_tv(track, args, sources),
            kind::PLAY_PIANO => self.play_piano(args, sources),
            kind::SEQUENCE_TRACK_HIT_LIST => self.sequence(track, args[1]),
            kind::KILL_OBJECT_SOUNDS => self.kill_object_sounds(sources),
            kind if kind::NOTHING_FOR_SOUND.contains(&kind) => {}
            kind if kind::NOT_PORTED.contains(&kind) => self.report(format!(
                "event {name}: kind {kind} is not one this player plays"
            )),
            // The dispatcher has no case for it.
            _ => {}
        }
    }

    /// Kind 1: starts a track (0x5736ee). *(code)* When nothing of the
    /// track runs or plays, the object instances recorded for it are
    /// dropped first. When the track's arguments are `kArgsIdVolPan`, a
    /// first argument of 0 starts nothing; any other is recorded as an
    /// instance of the track, and the track's `extvol` and `pan` are set
    /// from where its objects are ([`crate::placement`]).
    fn play_track(&mut self, track: u32, args: [u32; 3], sources: &dyn Sources) {
        if !self.active {
            return;
        }
        if self.owner(track).is_none() {
            self.no_track(track);
            return;
        }
        let alive = self.threads.iter().any(|t| t.track == track)
            || self.notes.iter().any(|n| n.track == track);
        if !alive {
            self.instances.retain(|&(t, _)| t != track);
        }
        // *(code)* The argument kind is the track object's own (+0x28 on
        // the object looked up, 0x573748 to 0x573756).
        if self.track_args(track) != Some(ARGS_ID_VOL_PAN) {
            let started = self.start(args, track);
            self.let_go_track(track, &started.threads);
            return;
        }
        if args[0] == 0 {
            return;
        }
        self.record(track, args[0]);
        let placed = self.placed(track, sources);
        let started = self.start(args, track);
        self.place(track, &started.threads, placed);
        self.let_go_track(track, &started.threads);
    }

    /// A track's volume and pan from where its objects are, as the sum
    /// over them (0x57473b) works them out: before the start, with the
    /// track's `Spl` read from its newest thread through its `target`.
    /// When none runs `Spl` is [`SPL_NORMAL`]: the original's track keeps
    /// its last run's registers, and before its first run reads the
    /// default, but no script writes `Spl`. A fault is reported, and
    /// gives `None`.
    fn placed(&mut self, track: u32, sources: &dyn Sources) -> Option<(i32, i32)> {
        let spl = self.spl(track);
        self.placed_with(track, spl, sources)
    }

    /// [`Player::placed`] with the given `Spl`.
    fn placed_with(&mut self, track: u32, spl: i32, sources: &dyn Sources) -> Option<(i32, i32)> {
        let objects: Vec<u32> = self
            .instances
            .iter()
            .filter(|&&(t, _)| t == track)
            .map(|&(_, object)| object)
            .collect();
        match placement::combine(&objects, spl, sources) {
            Ok(placed) => Some(placed),
            Err(fault) => {
                self.report(fault);
                None
            }
        }
    }

    /// A track's `Spl` (`0x57314b`, register 57), read through its
    /// `target` from its running thread (preferring its last start), else
    /// its idle kind-53 thread. `None` when the track has no live state.
    fn track_spl(&self, track: u32) -> Option<i32> {
        let last = self.last_start.get(&track).copied();
        let newest = self.newest(track).map(|at| self.threads[at].id);
        let running = |id: u32| self.threads.iter().find(|t| t.id == id);
        let spl_of = |thread: &Thread| {
            if Some(thread.id) != last {
                return thread.registers[usize::from(SPL)];
            }
            match self.resolve_target(thread) {
                Destination::Own => thread.registers[usize::from(SPL)],
                Destination::Running(id) => {
                    self.threads
                        .iter()
                        .find(|t| t.id == id)
                        .unwrap_or(thread)
                        .registers[usize::from(SPL)]
                }
                Destination::Stopped(stopped_track) => self
                    .idle
                    .get(&stopped_track)
                    .filter(|_| {
                        self.notes
                            .iter()
                            .any(|n| n.thread == 0 && n.track == stopped_track)
                    })
                    .map_or(SPL_NORMAL, |idle| idle.registers[usize::from(SPL)]),
            }
        };
        if let Some(thread) = last.and_then(running) {
            Some(spl_of(thread))
        } else if let Some(thread) = newest.and_then(running) {
            Some(spl_of(thread))
        } else {
            self.idle
                .get(&track)
                .filter(|_| self.notes.iter().any(|n| n.thread == 0 && n.track == track))
                .map(|idle| self.read(idle, i64::from(SPL)))
        }
    }

    /// A track's `Spl`, read from its live thread (running or idle)
    /// through its `target`; [`SPL_NORMAL`] when none exists (see
    /// [`Player::placed`]).
    fn spl(&self, track: u32) -> i32 {
        self.track_spl(track).unwrap_or(SPL_NORMAL)
    }

    /// *(code)* Writes a placement's `extvol` (0x57d57a) then `pan`
    /// (0x57d5a0) to the threads a start started, refreshed or left
    /// running (when a muted group returned early at `0x57d9a5`, `target`
    /// was not reset, so the write still follows `target` as `0x57de57`
    /// does; otherwise the start pointed `target` back at itself). When
    /// no script thread is running (for instance, a muted start on a
    /// track with an idle kind-53 note in `self.idle`), places that idle
    /// thread too. Also updates any playing notes of the written thread
    /// at once, matching the original's immediate voice update
    /// (0x57df13 → 0x57f759). With no placement both stay as they were.
    fn place(&mut self, track: u32, threads: &[u32], placed: Option<(i32, i32)>) {
        let Some((vol, pan)) = placed else {
            return;
        };
        if !threads.is_empty() {
            for &id in threads {
                self.place_through(id, vol, pan);
            }
            return;
        }
        self.forget_silent();
        self.place_idle(track, vol, pan);
    }

    /// Records an object instance, unless it is already (0x57442d) or
    /// [`INSTANCES`] are. *(code)* The original's multimap puts a new
    /// pair right after the first of its track (0x5761c2), so a track's
    /// instances go in the order first recorded, then the others newest
    /// first; ending one keeps the others' order. **Unverified:** the
    /// map's growth (0x5762c7) moves its nodes to new buckets, which may
    /// turn a track's order around; that is not ported.
    fn record(&mut self, track: u32, object: u32) {
        if self.instances.contains(&(track, object)) {
            return;
        }
        if self.instances.len() >= INSTANCES {
            self.report(format!("track {track}: more than {INSTANCES} instances"));
            return;
        }
        match self.instances.iter().position(|&(t, _)| t == track) {
            Some(first) => self.instances.insert(first + 1, (track, object)),
            None => self.instances.push((track, object)),
        }
    }

    /// The `args` field of a track, as the module it runs from defines
    /// it.
    fn track_args(&self, track: u32) -> Option<i64> {
        let owner = self.owner(track)?;
        let owner = &self.modules[owner];
        owner.number(&owner.hot.tracks[&track].args)
    }

    /// The module a track runs from, whichever module asks for it: the
    /// one whose line registered it last ([`owners`]). `None` when no
    /// module lists it.
    fn owner(&self, track: u32) -> Option<usize> {
        self.tracks.get(&track).copied()
    }

    /// Reports a start of a track no module lists. A patch object of that
    /// ID is no fault: its start slot returns at once (0x57bf80).
    fn no_track(&mut self, track: u32) {
        if !self.patches.contains_key(&track) {
            self.report(format!("no track {track}"));
        }
    }

    /// Forgets everything the object of this ID was doing, as a track and
    /// as a patch: its threads and notes, any note playing it as a patch,
    /// and its cached samples. A note playing another patch with this one
    /// queued plays on without it: releasing a patch releases its samples
    /// (0x57fd25), not the voice it is queued on (0x57e09e to 0x57e0b6).
    /// **Unverified:** whether that voice still plays it, from the path it
    /// copied (0x57ff7e to 0x57ff9a). The manager's instance records stay
    /// (0x57442d to 0x574473).
    fn destroy(&mut self, id: u32) {
        self.kill_track(id);
        self.held.remove(&id);
        self.idle.remove(&id);
        self.track_objects.remove(&id);
        self.last_start.remove(&id);
        self.notes.retain(|n| n.patch != id);
        for note in &mut self.notes {
            note.chain.retain(|&patch| patch != id);
        }
        self.cache.remove(&id);
    }

    /// Whether the registry has an object of this ID (0x573312), a track
    /// or a patch.
    fn registered(&self, id: u32) -> bool {
        self.tracks.contains_key(&id) || self.patches.contains_key(&id)
    }

    /// The newest running thread of a track, as an index.
    fn newest(&self, track: u32) -> Option<usize> {
        self.threads
            .iter()
            .enumerate()
            .filter(|(_, t)| t.track == track)
            .max_by_key(|(_, t)| t.id)
            .map(|(at, _)| at)
    }

    /// Writes a camera placement as the thread `id`, the track's last
    /// start, writes it: `extvol`, then `pan`, through its `target`
    /// (0x57de57). Only the destination the chain ends at takes the
    /// placement: a running thread updates its notes at once, a stopped
    /// track updates any idle kind-53 note it holds and leaves the caller
    /// untouched, and any thread that passes the write on keeps its
    /// levels.
    fn place_through(&mut self, id: u32, vol: i32, pan: i32) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let target_id = match self.resolve_target(thread) {
            Destination::Own => id,
            Destination::Running(dest_id) => dest_id,
            Destination::Stopped(stopped_track) => {
                self.place_idle(stopped_track, vol, pan);
                return;
            }
        };
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == target_id) {
            place_itself(thread, vol, pan);
            let thread = thread.clone();
            let levels = self.levels(&thread);
            for note in self.notes.iter_mut().filter(|n| n.thread == target_id) {
                note.gains = levels.gains;
                note.step = step(levels.rate);
            }
        }
    }

    /// Writes a register of the thread at `at` as that thread would.
    fn write_at(&mut self, at: usize, register: u8, value: i32) {
        let mut thread = self.threads[at].clone();
        self.write(&mut thread, i64::from(register), value);
        let id = thread.id;
        if let Some(slot) = self.threads.iter_mut().find(|t| t.id == id) {
            *slot = thread;
        }
    }

    /// Kind 18: *(code)* fades a track's `extvol` to 0 at the first
    /// argument's speed (100 when 0), and ends it when the fade ends if
    /// the third argument is not 0. No event in the install gives a
    /// speed, so the speed is the caller's object, or 100 for none, as
    /// the original's dispatcher passes it. The registers are written as the
    /// track would write them. A track with no running thread is left
    /// alone (the original writes to its idle track object).
    fn fade_track(&mut self, track: u32, args: [u32; 3]) {
        let Some(at) = self.newest(track) else {
            return;
        };
        let speed = if args[0] == 0 { 100 } else { args[0] as i32 };
        self.write_at(at, FADE_DEST, 0);
        self.write_at(at, FADE_VAR, i32::from(EXTVOL));
        self.write_at(at, FADE_SPEED, speed);
        if let Some(at) = self.newest(track) {
            self.threads[at].kill_after_fade = args[2] != 0;
            self.write_at(at, FADE_ON, 1);
        }
    }

    /// Kind 19 (`radio_next_song`): *(code)* stops the note each radio
    /// station's track is playing, so its script goes on to the next
    /// song. The original stops at the first of these tracks that does
    /// not exist.
    fn cancel_radio_notes(&mut self) {
        for track in RADIO_TRACKS {
            // A patch object of the number is no track, but it exists
            // (0x573a29 to 0x573a5c).
            if !self.registered(track) {
                return;
            }
            if let Some(at) = self.newest(track) {
                let id = self.threads[at].id;
                self.notes.retain(|n| n.thread != id);
            }
        }
    }

    /// Kind 21: *(code)* pauses the sound (0x573c20, 0x57481e). The
    /// manager is marked paused; `main_duckpri` becomes 0 (0x57d361),
    /// which here drops every thread's ducking request; the day and night
    /// loop fades out and ends (0x574844 → 0x574978); every running track
    /// is held except [`PAUSE_SPARES`] (0x574869..0x5748c9, and hold
    /// itself checks `is_running` at 0x57dc3d, so idle kind-53 voices are
    /// untouched); and in music mode 0 the current ambience player is
    /// paused (see [`crate::ambience::Fresh::set_paused`]). A held track's
    /// threads do not run or fade, and its notes stay where they are,
    /// silent. A track started while paused plays: starting a track
    /// (0x57d960) clears its hold, and a held track whose script still
    /// runs is let go rather than started again.
    pub fn pause(&mut self) {
        self.paused = true;
        self.duck.clear();
        self.update_main_duckpri();
        self.stop_day_night();
        let tracks: BTreeSet<u32> = self
            .threads
            .iter()
            .map(|t| t.track)
            .filter(|t| !PAUSE_SPARES.contains(t))
            .collect();
        self.held.extend(tracks);
        if self.music_mode == 0
            && let Some(current) = self.ambience
        {
            self.with_score(current, |score, _| score.set_paused(true));
        }
    }

    /// Kind 22: *(code)* unpauses the sound (0x573c44, 0x574904): clears
    /// the paused flag, and while sound is active (manager +0xc,
    /// [`Player::is_active`]) lets every track go (0x57cd2c), updates
    /// the day and night loop (0x574978), lets `MainMusicLoop` go
    /// (0x575155) and unpauses the current ambience player in any music
    /// mode. When sound is inactive, only the paused flag is cleared;
    /// turning sound back on ([`Player::set_active`]) then resumes.
    ///
    /// Letting a track go (vtable +0x5c, 0x57dc63) also moves its wait
    /// deadline (+0x198, which `wait` sets at 0x57e453) to now, so every
    /// running thread's wait ends at the next update, whether or not it
    /// was held, and whether or not the sound was paused. `main_duckpri`
    /// stays as the pause left it until a thread asks again.
    ///
    /// The original lets go of the tracks in the order of its registry of
    /// patch and track objects (0x57cd56 to 0x57cd82): bucket by bucket,
    /// a track's bucket being its number modulo their count, and each
    /// bucket from its head, as [`crate::registry`] replays them. With
    /// the whole install registered there are 24,593 buckets, so the
    /// order is not the tracks' numbers. Each let go is put last in the list of
    /// running tracks, so from the next block the tracks run in that
    /// order.
    ///
    /// A track no module lists has no place in the registry; such
    /// tracks go last, by number. **Unverified:** the order the original
    /// takes the files of one directory in, and the downloads' order
    /// (here, by name).
    pub fn unpause(&mut self, sources: &dyn Sources) {
        self.paused = false;
        if !self.active {
            return;
        }
        self.unpause_tracks(sources);
        if let Some(current) = self.ambience {
            self.with_score(current, |score, _| score.set_paused(false));
        }
    }

    /// *(code)* 0x574904: clears pause, lets every running track go in
    /// registry order (0x57cd2c), updates the day and night loop
    /// (0x574978), and lets `MainMusicLoop` go (0x575155).
    fn unpause_tracks(&mut self, sources: &dyn Sources) {
        self.paused = false;
        self.held.clear();
        let running: BTreeSet<u32> = self.threads.iter().map(|t| t.track).collect();
        let mut place: BTreeMap<u32, usize> = BTreeMap::new();
        for (at, id) in self.registry.order().enumerate() {
            if running.contains(&id) {
                place.insert(id, at);
            }
            if place.len() == running.len() {
                break;
            }
        }
        let mut running: Vec<(usize, u32, u32)> = self
            .threads
            .iter()
            .map(|t| {
                (
                    place.get(&t.track).copied().unwrap_or(usize::MAX),
                    t.track,
                    t.id,
                )
            })
            .collect();
        running.sort_unstable();
        let ids: Vec<u32> = running.into_iter().map(|(_, _, id)| id).collect();
        self.let_go(&ids);
        self.update_day_night(sources);
        self.unpause_music();
    }

    /// *(code)* 0x575155: lets go of `MainMusicLoop` (track 515, vtable
    /// +0x5c). Called at the end of unpause (0x574923) and kind 25 on
    /// (0x573cc0).
    fn unpause_music(&mut self) {
        if self.owner(music::MAIN_MUSIC_LOOP).is_none() {
            return;
        }
        self.held.remove(&music::MAIN_MUSIC_LOOP);
        let ids: Vec<u32> = self
            .threads
            .iter()
            .filter(|t| t.track == music::MAIN_MUSIC_LOOP)
            .map(|t| t.id)
            .collect();
        self.let_go(&ids);
    }

    /// Kind 25: *(code)* sets the sound-active flag (manager +0xc,
    /// `0x573c64`), which the original's `WM_ACTIVATEAPP` handler sends
    /// when the window loses or regains focus (`0x5d6ccb` → `0x5d6e2e`
    /// → `0x4f7608` → `0x4f76d5`). The app constructor initializes its
    /// own active flag to `1` (`0x4eee14`) and `0x4f7610` returns when
    /// unchanged, so startup sends no kind 25.
    ///
    /// When turning sound on (`active == true`):
    /// - nudges every placed track's volume (`+0x114 = 100`, `0x57447f`,
    ///   then `+0x114 = 0`, `0x57447f`) and arms the 100-update
    ///   countdown (`+0x114 = 100`, `0x573c71`..`0x573c98`);
    /// - if not paused, calls `0x574904` (unpause tracks, day and night
    ///   loop, `MainMusicLoop`) and unpauses the current ambience;
    /// - if `music_mode == 3`, calls `0x574904` (even when paused);
    /// - calls `0x575155` (`MainMusicLoop` let-go).
    ///
    /// When turning sound off (`active == false`):
    /// - in live music mode (`music_mode == 0`), pauses every ambience
    ///   score (`+0x104`..`+0x110`, `0x573cca`..`0x573d0f`).
    pub fn set_active(&mut self, active: bool, sources: &dyn Sources) {
        self.active = active;
        if active {
            self.nudge = NUDGE_UPDATES;
            self.place_all(sources);
            self.nudge = 0;
            self.place_all(sources);
            self.nudge = NUDGE_UPDATES;
            if !self.paused {
                self.unpause_tracks(sources);
                if let Some(current) = self.ambience {
                    self.with_score(current, |score, _| score.set_paused(false));
                }
            }
            if self.music_mode == 3 {
                self.unpause_tracks(sources);
            }
            self.unpause_music();
        } else if self.music_mode == 0 {
            let modes: Vec<u32> = self.scores.keys().copied().collect();
            for mode in modes {
                self.with_score(mode, |score, _| score.set_paused(true));
            }
        }
    }

    /// Whether the sound is active (manager +0xc, `true` until kind 25
    /// turns it off).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// *(code)* 0x57492a: picks the day and night background loop track:
    /// - [`BEACH_LOOP`] (68259) when the current ambience is Beach (mode
    ///   1, manager +0x108);
    /// - none when the lot number (global 10, `0x571093`) is 20 to 29;
    /// - [`NITE_LOOP`] (2985) when the hour (`sources.field(0, 4)`,
    ///   `0x570e61`) is below 6 or above 18;
    /// - else none.
    fn pick_day_night(&self, sources: &dyn Sources) -> Option<u32> {
        if self.ambience == Some(1) {
            return Some(BEACH_LOOP);
        }
        let lot = sources.lot() as u32;
        if (20..=29).contains(&lot) {
            return None;
        }
        let hour = sources.field(0, 4).unwrap_or(0);
        if !(6..=18).contains(&hour) {
            Some(NITE_LOOP)
        } else {
            None
        }
    }

    /// *(code)* The stop branch of `0x574978` (`0x574a6d`), used by
    /// [`Player::pause`] where `paused` is always true and `sources` is
    /// not queried.
    fn stop_day_night(&mut self) {
        if let Some(track) = self.day_night {
            self.fade_day_night(track);
        }
    }

    /// *(code)* `0x574978`: starts, switches or stops the day and night
    /// loop (`nite_loop` 2985 or `BeachLoop` 68259).
    fn update_day_night(&mut self, sources: &dyn Sources) {
        let old = self.day_night;
        if self.paused || !self.active || self.music_mode != 0 {
            if let Some(track) = old {
                self.fade_day_night(track);
            }
            return;
        }
        let new = self.pick_day_night(sources);
        self.day_night = new;
        if new != old
            && let Some(track) = old
        {
            self.fade_day_night(track);
        }
        let Some(track) = new else {
            return;
        };
        // Unregistered ID -> NULL (0x5749f6); patch object -> vtable
        // +0x70 returns true (0x57bfa6), so neither starts nor faults.
        if self.owner(track).is_none() || self.day_night_playing(track) {
            return;
        }
        let target_vol = self.group_volumes.get(&1).copied().unwrap_or(1024);
        let started = self.start([0; 3], track);
        if let Some(&id) = started.threads.last()
            && let Some(at) = self.threads.iter().position(|t| t.id == id)
        {
            self.write_at(at, EXTVOL, 0);
            self.write_at(at, FADE_DEST, target_vol);
            self.write_at(at, FADE_VAR, i32::from(EXTVOL));
            self.write_at(at, FADE_SPEED, DAY_NIGHT_FADE_SPEED);
            self.write_at(at, FADE_ON, 1);
        }
        self.let_go_track(track, &started.threads);
    }

    /// *(code)* Whether a day and night loop track is running (vtable
    /// +0x70, `0x574a00`). Excludes idle kind-53 notes (`thread == 0`).
    fn day_night_playing(&self, track: u32) -> bool {
        self.threads.iter().any(|t| t.track == track)
    }

    /// Fades out and ends a day and night loop track if it is a running
    /// track object (vtable +0x70 / +0x34, `0x5749d6`, `0x574a80`).
    fn fade_day_night(&mut self, track: u32) {
        if self.owner(track).is_some() && self.day_night_playing(track) {
            self.fade_and_kill(track);
        }
    }

    /// Whether the sound is paused (kind 21 since the last 22).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The registry of patch and track objects, which orders the
    /// unpause ([`Player::unpause`]).
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Kind 20: *(code)* ends every track at once, forgets every object
    /// instance and stops every ambience player (the current one stays
    /// current). (The original also stops its music stations, which this
    /// player does not have.) Sequence groups are cleared too.
    fn kill_all(&mut self) {
        self.threads.clear();
        self.notes.clear();
        self.idle.clear();
        self.last_start.clear();
        self.held.clear();
        self.seqgroups.clear();
        self.instances.clear();
        let modes: Vec<u32> = self.scores.keys().copied().collect();
        for mode in modes {
            self.with_score(mode, |score, host| score.stop(host));
        }
        self.forget_ended();
    }

    /// Kind 23: *(code)* forgets every instance of an object (0: none)
    /// and lets each of its tracks go (0x574688): a track no other
    /// object still has fades out and ends; one that others still have
    /// plays on for them, its `extvol` and `pan` set again from where
    /// they are, as a camera move sets them (0x5746d9,
    /// [`Player::place_track`]).
    fn kill_instance(&mut self, object: u32, sources: &dyn Sources) {
        if object == 0 {
            return;
        }
        let tracks: Vec<u32> = self
            .instances
            .iter()
            .filter(|&&(_, o)| o == object)
            .map(|&(t, _)| t)
            .collect();
        for track in tracks {
            self.instances.retain(|&pair| pair != (track, object));
            if self.instances.iter().any(|&(t, _)| t == track) {
                // A fade before may have ended notes.
                self.forget_silent();
                self.place_track(track, sources);
            } else {
                self.fade_and_kill(track);
            }
        }
    }

    /// Kind 105 (`kill_all_objectsounds`, 0x574560): *(code)* lets go of
    /// every instance whose object is above 0 (as a signed number),
    /// fading out and ending its track, then places every track with
    /// remaining instances (`<= 0`) again (0x574647 → 0x57447f).
    fn kill_object_sounds(&mut self, sources: &dyn Sources) {
        let (killed, kept): (Vec<_>, Vec<_>) = self
            .instances
            .iter()
            .copied()
            .partition(|&(_, object)| (object as i32) > 0);
        self.instances = kept;
        for (track, _) in killed {
            self.fade_and_kill(track);
        }
        self.place_all(sources);
    }

    /// *(code)* The original's fade-out-and-kill (vtable +0x34, 0x57dca3)
    /// on a track's running threads: a track that is not running returns
    /// at once (0x57dca8..0x57dcaf); a running thread with a note playing
    /// writes `fade_dest = 0`, `fade_var = extvol`, `fade_speed = 100`
    /// through `target`, sets `kill_after_fade` (+0x1cf) on itself, and
    /// writes `fade_on = 1` through `target`; one without a playing note
    /// ends at once (vtable +0x38, 0x57dd3e).
    fn fade_and_kill(&mut self, track: u32) {
        let ids: Vec<u32> = self
            .threads
            .iter()
            .filter(|t| t.track == track)
            .map(|t| t.id)
            .collect();
        for id in ids {
            let Some(at) = self.threads.iter().position(|t| t.id == id) else {
                continue;
            };
            if self.notes.iter().any(|n| n.thread == id) {
                self.write_at(at, FADE_DEST, 0);
                self.write_at(at, FADE_VAR, i32::from(EXTVOL));
                self.write_at(at, FADE_SPEED, 100);
                if let Some(at) = self.threads.iter().position(|t| t.id == id) {
                    self.threads[at].kill_after_fade = true;
                    self.write_at(at, FADE_ON, 1);
                }
            } else {
                self.threads.remove(at);
            }
        }
        self.forget_ended();
    }

    /// Kind 30 (`kTurnOnTv`, which radios use too): *(code)* lets go of
    /// whatever the object (the first argument) had playing, records it
    /// as an instance of the track, and starts the track for it. A track
    /// started anew is left held (the start ends by holding it, 0x57dbd6)
    /// and let go only when the sound is not paused (0x573d71), so a
    /// television turned on during a pause is silent until the unpause.
    fn turn_on_tv(&mut self, track: u32, args: [u32; 3], sources: &dyn Sources) {
        self.kill_instance(args[0], sources);
        if !self.registered(track) {
            self.no_track(track);
            return;
        }
        // The instance is recorded before the object's start (0x573d36,
        // 0x573d5a), which for a patch object does nothing.
        self.record(track, args[0]);
        if self.owner(track).is_none() {
            return;
        }
        let placed = self.placed(track, sources);
        let started = self.start(args, track);
        if started.fresh && (self.paused || !self.active) {
            self.held.insert(track);
        }
        self.place(track, &started.threads, placed);
        if !self.paused && self.active {
            self.let_go_track(track, &started.threads);
        }
    }

    /// Kind 43 (`kPlayPiano`, 0x573e5a): *(code)* the event's track is
    /// not used; the piano is track [`PIANO`]. With no such track nothing
    /// happens. Otherwise the object (the first argument) is placed as
    /// one object, with the piano's `Spl` ([`placement::place`]), into
    /// the second and third arguments, which keep the event's values when
    /// it is not placed; the piano starts (and is let go) with them. The
    /// piano's instances are then replaced by the one object the track
    /// object holds: the object of the last fresh start, which a start
    /// that lets a held piano go or is refused does not change, and 0
    /// before any. Last, the object's source field 5 is asked for, and
    /// [`PIANO_SKILL`] starts, when it exists, with it as its first
    /// argument.
    fn play_piano(&mut self, args: [u32; 3], sources: &dyn Sources) {
        // A patch object of the number goes on: its start does nothing
        // (0x573e95, 0x57bf80), its object is 0x80000000 (0x573e9c calls
        // 0x57bf4e, which reads +0xb0; the patch's constructor sets it,
        // 0x57bf0e to 0x57bf19), and the skill track is still started
        // (0x573ec3 to 0x573ef2).
        if !self.registered(PIANO) {
            return;
        }
        let mut args = args;
        match placement::place(args[0] as i32, self.spl(PIANO), sources) {
            Ok(Some((vol, pan))) => {
                args[1] = vol as u32;
                args[2] = pan as u32;
            }
            Ok(None) => {}
            Err(fault) => self.report(fault),
        }
        let started = self.start(args, PIANO);
        self.let_go_track(PIANO, &started.threads);
        let object = match self.owner(PIANO) {
            Some(_) => self.track_objects.get(&PIANO).copied().unwrap_or(0),
            None => 0x8000_0000,
        };
        self.instances.retain(|&(t, _)| t != PIANO);
        self.record(PIANO, object);
        // The skill is asked for before the skill track is looked up.
        let skill = sources.field(args[0] as i32, 5).unwrap_or(0);
        if self.owner(PIANO_SKILL).is_some() {
            let started = self.start([skill as u32, 0, 0], PIANO_SKILL);
            self.let_go_track(PIANO_SKILL, &started.threads);
        }
    }

    /// Kind 53 (`kSequenceTrackHitList`): *(code)* picks a patch from a
    /// hit list (the second argument) and chains it on a track
    /// (0x57e088): when the track's newest thread has a note playing, the
    /// patch plays right after it; otherwise the thread's `patch`
    /// register takes it and a note starts. A track with no running
    /// thread plays the patch at its starting levels, as the original's
    /// idle track object does, chained to the patch an earlier such event
    /// started if that still plays.
    fn sequence(&mut self, track: u32, hit_list: u32) {
        if !self.registered(track) {
            return;
        }
        let Some(list) = self.hit_lists.get(&hit_list).cloned() else {
            self.report(format!("no hit list {hit_list}"));
            return;
        };
        let patch = match list.len() {
            0 => 0,
            n => list[self.random(n as u32) as usize],
        };
        // A patch object of the number still picks from the list. **Not
        // ported:** its slot (0x573f2b, 0x57fcd2) can then stop the
        // patch's playback and let go of its samples; no kind 53 event
        // in the install names a patch.
        let Some(owner) = self.owner(track) else {
            return;
        };
        match self.newest(track) {
            Some(at) => {
                let last = self.threads[at].last_note;
                if let Some(note) = self.notes.iter().position(|n| n.id == last && last != 0) {
                    self.chain(note, track, patch);
                } else {
                    self.write_at(at, PATCH, patch as i32);
                    let thread = self.threads[at].clone();
                    let id = self.note_on(&thread);
                    self.threads[at].last_note = id;
                }
            }
            None => {
                // The track's own note from an earlier kind 53, if it
                // plays: the patch chains to it.
                let idle = self
                    .notes
                    .iter()
                    .rposition(|n| n.thread == 0 && n.track == track);
                if let Some(note) = idle {
                    self.chain(note, track, patch);
                } else if let Some(mut thread) = self.thread([0; 3], owner, track) {
                    thread.id = 0;
                    thread.registers[usize::from(PATCH)] = patch as i32;
                    self.note_on(&thread);
                    self.idle.insert(track, thread);
                }
            }
        }
    }

    /// Chains a patch to the note at `note`, to play when it ends.
    fn chain(&mut self, note: usize, track: u32, patch: u32) {
        if !self.load_patch(patch) {
            return;
        }
        if self.notes[note].chain.len() >= CHAIN {
            self.report(format!("track {track}: more than {CHAIN} chained"));
        } else {
            self.notes[note].chain.push(patch);
        }
    }

    /// Starts a track's routine for an object (vtable +0x30, 0x57d960).
    /// The module given is tried first, then every other. A new thread
    /// first runs in the next block ([`Player::run`] runs the tracks
    /// that were running when its block began).
    ///
    /// *(code)* The original has one track object per track, so a start
    /// of a track whose script still runs starts no second one. It
    /// refreshes the running one instead ([`set_arguments`]); then, when
    /// the pause holds it, lets it go (0x57d9f1 to 0x57da12, see
    /// [`Player::resume_held`]), and otherwise reads its `interrupt`
    /// ([`INTERRUPT`], 0x57da26): 0 leaves the script where it is, and
    /// anything else starts the same track object anew, which cuts its
    /// note ([`Player::restart`]). The caller then places and lets go of
    /// the threads returned as it does a fresh start's
    /// ([`Player::let_go`]). Only a start of a track that does not run
    /// makes a thread, so only then does the thread limit apply.
    ///
    /// *(code)* When the track's nonzero group has volume 0 (manager
    /// lookup `0x57bde7`, its `+0xc0 == 0`), the start returns at once
    /// without starting, refreshing or restarting (`0x57d969`..`0x57d9a5`);
    /// any already-running thread of the track is still returned so the
    /// caller can apply placement and `let_go` (which clears hold at
    /// `0x57dc68`) as the original's callers do.
    fn start(&mut self, args: [u32; 3], track: u32) -> Started {
        let Some(module) = self.owner(track) else {
            self.no_track(track);
            return Started::default();
        };
        let owner = &self.modules[module];
        let group = owner.number(&owner.hot.tracks[&track].group).unwrap_or(0);
        if group != 0 && self.group_volumes.get(&group).copied().unwrap_or(1024) == 0 {
            let existing: Vec<u32> = self
                .threads
                .iter()
                .filter(|t| t.track == track)
                .map(|t| t.id)
                .collect();
            return Started {
                threads: existing,
                fresh: false,
            };
        }
        if self.held.remove(&track) {
            let resumed = self.resume_held(args, track);
            if !resumed.is_empty() {
                return Started {
                    threads: resumed,
                    fresh: false,
                };
            }
        }
        let running = self.refresh(args, track);
        if let Some(&newest) = running.iter().max() {
            let interrupt = self
                .threads
                .iter()
                .find(|t| t.id == newest)
                .map_or(0, |t| t.registers[usize::from(INTERRUPT)]);
            self.last_start.insert(track, newest);
            if interrupt == 0 {
                return Started {
                    threads: running,
                    fresh: false,
                };
            }
            // The track object's own descriptor (0x57db7a to 0x57db91).
            if !self.restart(newest, module, track) {
                return Started::default();
            }
            self.set_levels(newest, args, track);
            return Started {
                threads: vec![newest],
                fresh: true,
            };
        }
        if self.threads.len() >= THREADS {
            self.report(format!("track {track}: more than {THREADS} threads"));
            return Started::default();
        }
        let Some(thread) = self.thread(args, module, track) else {
            return Started::default();
        };
        // *(code)* A fresh start drops the track object's sound (0x57db67
        // to 0x57f625, 0x57f657 to 0x57f676): the note kind 53 played on
        // the idle track stops.
        self.notes.retain(|n| !(n.thread == 0 && n.track == track));
        self.idle.remove(&track);
        let id = thread.id;
        self.last_start.insert(track, id);
        self.threads.push(thread);
        self.set_levels(id, args, track);
        Started {
            threads: vec![id],
            fresh: true,
        }
    }

    /// *(code)* Starts a running track's thread `id` anew in place, as the
    /// original's fresh start does to its one track object (0x57da3a to
    /// 0x57dbc5): the default banks (registers 17 to 47 and 50 to 62,
    /// [`BANKS`]) take their starting values again, the routine starts
    /// from the top with no loop mark (0x57db8b, 0x57db91) and the
    /// track's own hit list (0x57db85 to 0x57dba7), and its sound is cut
    /// (0x57f625). Registers 1 to 16 and the return stack (+0x1a8) are
    /// left as they were. (Registers 48 and 49 are in no bank of the
    /// original, 0x57d727 to 0x57d775 and 0x57c7a8 to 0x57c82e; here they
    /// are kept too.) The thread keeps its ID, so a
    /// `target` naming it still reaches it. Its wait deadline is now and
    /// `isplaying` is 1. False (reported) when the track has no entry
    /// point; the thread is then left alone.
    fn restart(&mut self, id: u32, module: usize, track: u32) -> bool {
        let Some((place, fresh, _)) = self.template(module, track) else {
            return false;
        };
        // A track has one thread; its notes are the one voice the
        // original destroys (0x57f625).
        self.threads.retain(|t| t.track != track || t.id == id);
        self.notes.retain(|n| n.track != track);
        self.idle.remove(&track);
        let now = self.frame;
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            for (first, last) in BANKS {
                let bank = usize::from(first)..=usize::from(last);
                thread.registers[bank.clone()].copy_from_slice(&fresh[bank]);
            }
            (thread.module, thread.pc) = place;
            thread.loop_point = place;
            thread.hit_list.clear();
            thread.last_note = 0;
            thread.wake = now;
            thread.registers[usize::from(ISPLAYING)] = 1;
            thread.kept = 1;
        }
        self.forget_ended();
        true
    }

    /// The levels a fresh start sets on thread `id` (see below).
    fn set_levels(&mut self, id: u32, args: [u32; 3], track: u32) {
        // *(code)* A fresh start sets the levels from the arguments as the
        // track's `args` field says (0x57db01): `kArgsVolPan` has `extvol`
        // and `pan` in the first two, `kArgsIdVolPan` in the last two
        // (after the object). The stores leave the flags alone. (A track
        // never started, as kind 53 plays on, keeps its defaults.)
        // A `kArgsIdVolPan` start also makes the object the track
        // object's own (vtable +0x20).
        let levels = match self.track_args(track) {
            Some(ARGS_VOL_PAN) => Some((args[0], args[1])),
            Some(ARGS_ID_VOL_PAN) => {
                self.track_objects.insert(track, args[0]);
                Some((args[1], args[2]))
            }
            _ => None,
        };
        let Some((vol, pan)) = levels else {
            return;
        };
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
            thread.registers[usize::from(EXTVOL)] = vol as i32;
            thread.registers[usize::from(PAN)] = pan as i32;
        }
    }

    /// What every start does to a track whose script runs (see
    /// [`set_arguments`]), to each of its threads. The threads, oldest
    /// first: none when the track does not run.
    fn refresh(&mut self, args: [u32; 3], track: u32) -> Vec<u32> {
        let mut running = Vec::new();
        for thread in self.threads.iter_mut().filter(|t| t.track == track) {
            set_arguments(thread, args);
            running.push(thread.id);
        }
        running
    }

    /// Lets go of a track unconditionally (`0x57dc68` clears the track's
    /// hold even when no script thread is running, such as a muted replay
    /// of a track with an idle kind-53 note), then lets go of any
    /// running `threads`.
    fn let_go_track(&mut self, track: u32, threads: &[u32]) {
        self.held.remove(&track);
        self.let_go(threads);
    }

    /// Lets go of a started track (vtable +0x5c, 0x57dc63): *(code)* its
    /// wait deadline moves to now (0x57dc8e), so a running script's `wait`
    /// ends at the next update. (Its hold ends too; [`Player::start`]
    /// already ended it.)
    ///
    /// *(code)* A running track let go also moves to the end of the list
    /// of running tracks (0x57dc9a: 0x57d017 takes it out and appends
    /// it), so it runs after the others from the next block. (The block
    /// running goes through its own copy of the list.)
    fn let_go(&mut self, threads: &[u32]) {
        let now = self.frame;
        for &id in threads {
            let order = self.last_in_order();
            if let Some(thread) = self.threads.iter_mut().find(|t| t.id == id) {
                self.held.remove(&thread.track);
                thread.wake = now;
                thread.order = order;
            }
        }
    }

    /// An order after every thread's (see [`Thread::order`]).
    fn last_in_order(&mut self) -> i64 {
        self.next_order += 1;
        self.next_order
    }

    /// Lets a held track's running threads go on, as starting it does
    /// (see [`Player::start`]). Each one's `target` points back at itself
    /// (0x57d9c3), its argument registers take `args` (see
    /// [`Player::thread`]) and its wait ends. The threads let go: none
    /// when none of the track runs, so it is started anew. (A held thread
    /// does not run, so none of these is the thread running `play_trk`.)
    /// The original sets its track object's `target` back to itself on
    /// every start, this one too, so the newest thread let go becomes
    /// the one [`Player::camera_moved`] writes through.
    fn resume_held(&mut self, args: [u32; 3], track: u32) -> Vec<u32> {
        let resumed = self.refresh(args, track);
        self.let_go(&resumed);
        if let Some(&newest) = resumed.iter().max() {
            self.last_start.insert(track, newest);
        }
        resumed
    }

    /// A new thread of a track the module defines, with its starting
    /// registers, or `None` (reported) when the track has no entry point.
    ///
    /// **Unverified** gap: the original's track object outlives its
    /// script, so a start after the track ended keeps its registers 1 to
    /// 16 from the run before ([`Player::restart`]); a new thread here
    /// starts them at 0 (but for the arguments).
    fn thread(&mut self, args: [u32; 3], module: usize, track: u32) -> Option<Thread> {
        let (place, registers, group) = self.template(module, track)?;
        let id = self.id();
        let mut thread = Thread {
            id,
            module: place.0,
            track,
            pc: place.1,
            registers,
            stack: Vec::new(),
            loop_point: place,
            kept: 0,
            hit_list: Vec::new(),
            last_note: 0,
            wake: 0,
            group,
            target: Target::Own,
            kill_after_fade: false,
            order: self.last_in_order(),
        };
        set_arguments(&mut thread, args);
        // *(code)* A fresh start then writes `isplaying` 1 (0x57dbc5),
        // which leaves the comparison flags positive. (It also sets the
        // wait deadline to now and holds the track, 0x57dbd6, which its
        // callers let go; kind 1 always does.)
        thread.registers[usize::from(ISPLAYING)] = 1;
        thread.kept = 1;
        Some(thread)
    }

    /// A track's entry point, starting registers and group, or `None`
    /// (reported) when it has no entry point. `module` is the track's
    /// own: its descriptor names the entry point's track data ID, which
    /// may lead into another module's program ([`entry_points`]).
    fn template(
        &mut self,
        module: usize,
        track: u32,
    ) -> Option<(Place, [i32; GLOBALS as usize], i64)> {
        let owner = &self.modules[module];
        let Some(pc) = owner
            .entry(track)
            .and_then(|entry| self.track_data.get(&entry).copied())
        else {
            self.report(format!("track {track}: entry point not found"));
            return None;
        };
        let fields = &owner.hot.tracks[&track];
        let patch = owner.number(&fields.patch).unwrap_or(0);
        let mut registers = [0; GLOBALS as usize];
        for (register, value) in DEFAULTS {
            registers[usize::from(register)] = value;
        }
        // The original's track constructor writes the HOT track's volume
        // and duck priority.
        if let Some(volume) = owner.number(&fields.volume) {
            registers[usize::from(VOL)] = volume as i32;
        }
        registers[usize::from(DUCKPRI)] = owner.number(&fields.duck_priority).unwrap_or(0) as i32;
        let group = owner.number(&fields.group).unwrap_or(0);
        registers[usize::from(PATCH)] = patch as i32;
        Some((pc, registers, group))
    }

    /// Records a fault, up to [`FAULTS`] of them; later ones are dropped,
    /// so a script that faults every block cannot grow the list forever.
    fn report(&mut self, message: String) {
        if self.faults.len() < FAULTS {
            self.faults.push(message);
        }
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// A random number below `below`, from the player's own generator.
    fn random(&mut self, below: u32) -> u32 {
        xorshift(&mut self.random, below)
    }

    /// Runs the threads and mixes `frames` frames, in blocks of [`BLOCK`].
    pub fn render(&mut self, frames: usize, sources: &dyn Sources) -> Vec<i16> {
        let stereo = self.render_stereo(frames, sources);
        stereo
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[left, right]| ((i32::from(*left) + i32::from(*right)) / 2) as i16)
            .collect()
    }

    /// Renders `frames` frames of interleaved stereo (left, then right):
    /// the player's own output. [`Player::render`] is its mono mix.
    pub fn render_stereo(&mut self, frames: usize, sources: &dyn Sources) -> Vec<i16> {
        let Some(samples) = frames.checked_mul(2).filter(|_| frames <= RENDER_LIMIT) else {
            self.report(format!("cannot render {frames} frames at once"));
            return Vec::new();
        };
        let mut out = Vec::with_capacity(samples.min(2 * RATE as usize));
        while out.len() < samples {
            self.run_at_block_start(sources);
            let until_block = BLOCK - (self.frame % BLOCK as u64) as usize;
            let count = until_block.min(frames - out.len() / 2);
            self.mix(count, &mut out);
            self.frame += count as u64;
        }
        out
    }

    /// Renders until nothing runs or plays, at most `limit` frames (and
    /// at most [`RENDER_LIMIT`]). The output ends with the block in which
    /// the last note ended.
    pub fn render_until_idle(&mut self, limit: usize, sources: &dyn Sources) -> Vec<i16> {
        let limit = limit.min(RENDER_LIMIT);
        let mut out = Vec::new();
        while out.len() < limit {
            self.run_at_block_start(sources);
            if self.is_idle() {
                break;
            }
            let until_block = BLOCK - (self.frame % BLOCK as u64) as usize;
            out.extend(self.render(until_block.min(limit - out.len()), sources));
        }
        out
    }

    /// Runs the threads once at the start of each block.
    fn run_at_block_start(&mut self, sources: &dyn Sources) {
        if self.frame.is_multiple_of(BLOCK as u64) && self.ran_at != Some(self.frame) {
            self.ran_at = Some(self.frame);
            // *(code)* 0x573019: before stepping the HIT tracks, while
            // `+0x114` is nonzero, place every track nudged then normal
            // and decrement the countdown.
            if self.nudge != 0 {
                let remaining = self.nudge;
                self.place_all(sources);
                self.nudge = 0;
                self.place_all(sources);
                self.nudge = remaining - 1;
            }
            self.run(sources);
            self.update_ambience();
        }
    }

    /// Mixes `frames` stereo frames of the notes and the ambience
    /// voices onto `out` (see [`mix_note`]).
    fn mix(&mut self, frames: usize, out: &mut Vec<i16>) {
        let start = out.len();
        let mut sums = vec![0i64; 2 * frames];
        let cache = &self.cache;
        let ambient_cache = &self.ambient_cache;
        let held = &self.held;
        // A held track's notes keep their place and are not heard.
        self.notes
            .retain_mut(|note| held.contains(&note.track) || mix_note(note, cache, &mut sums));
        let notes = &self.notes;
        self.idle.retain(|track, _| {
            notes
                .iter()
                .any(|note| note.thread == 0 && note.track == *track)
        });
        self.voices
            .retain_mut(|voice| mix_note(&mut voice.note, ambient_cache, &mut sums));
        out.extend(sums.iter().map(|&s| s.clamp(-32768, 32767) as i16));
        debug_assert_eq!(out.len(), start + 2 * frames);
    }

    /// A thread's levels from its registers and the player's settings.
    fn levels(&self, thread: &Thread) -> Levels {
        let register = |r: u8| thread.registers[usize::from(r)];
        levels::levels(&Inputs {
            vol: register(VOL),
            extvol: register(EXTVOL),
            pan: register(PAN),
            pitch: register(PITCH),
            duck_priority: register(DUCKPRI),
            three_d: register(THREE_D) != 0,
            group_volume: self
                .group_volumes
                .get(&thread.group)
                .copied()
                .unwrap_or(1024),
            main_duck_priority: self.globals[usize::from(MAIN_DUCKPRI - GLOBALS)],
            master: self.master,
        })
    }

    /// Gives every note of a running track its thread's levels
    /// (`0x57c0ea` walks the running-track list and does not include
    /// idle kind-53 voices). (The original sets them as registers
    /// change; here once per block, after the threads run, and on group
    /// or master volume changes.)
    fn apply_levels(&mut self) {
        let levels: BTreeMap<u32, Levels> = self
            .threads
            .iter()
            .map(|t| (t.id, self.levels(t)))
            .collect();
        for note in &mut self.notes {
            if let Some(levels) = levels.get(&note.thread) {
                note.gains = levels.gains;
                note.step = step(levels.rate);
            }
        }
    }

    /// Drops the ducking requests of threads that have ended (the
    /// original drops a track's request when it is killed) and sets
    /// `main_duckpri` to the largest left, or 0.
    fn forget_ended(&mut self) {
        let alive: Vec<u32> = self.threads.iter().map(|t| t.id).collect();
        self.duck.retain(|id, _| alive.contains(id));
        self.update_main_duckpri();
    }

    fn update_main_duckpri(&mut self) {
        let largest = self.duck.values().copied().fold(0, i32::max);
        self.globals[usize::from(MAIN_DUCKPRI - GLOBALS)] = largest;
    }

    /// Runs every track once, in the order of the list of running tracks
    /// when the block began ([`Thread::order`]). The original goes through
    /// a copy of that list, calling each track object (0x57cf8f to
    /// 0x57cfda), so a track that ends or is ended on the way does not
    /// make it skip another, and a track ended and started again before
    /// its turn still runs in it, as its new thread. A track started on
    /// the way first runs in the next block. A held track is not in the
    /// list (a hold takes it out, 0x57dc53 to 0x57dc5a), so one let go on
    /// the way first runs in the next block too.
    fn run(&mut self, sources: &dyn Sources) {
        let mut budget = BLOCK_STEPS;
        self.threads.sort_by_key(|t| t.order);
        let schedule: Vec<u32> = self
            .threads
            .iter()
            .map(|t| t.track)
            .filter(|t| !self.held.contains(t))
            .collect();
        for (next, &track) in schedule.iter().enumerate() {
            // A track runs one thread at most.
            let Some(mut thread) = self.threads.iter().find(|t| t.track == track).cloned() else {
                continue;
            };
            // A held track's threads wait, without fading (0x57d7f3).
            if self.held.contains(&track) {
                continue;
            }
            if budget == 0 {
                self.out_of_budget(&schedule[next..]);
                break;
            }
            let alive = self.step(&mut thread, sources, &mut budget);
            // A thread may have stopped others (kill_trk) or started some
            // (play_trk); find this one again by ID.
            let id = thread.id;
            if alive {
                if let Some(slot) = self.threads.iter_mut().find(|t| t.id == id) {
                    *slot = thread;
                }
                continue;
            }
            if !self.threads.iter().any(|t| t.id == id) {
                continue;
            }
            self.threads.retain(|t| t.id != id);
            // Its ducking request goes now, before the threads after it
            // run.
            self.forget_ended();
        }
        self.forget_ended();
        self.apply_levels();
    }

    /// The block has run out of instructions before the tracks `rest`
    /// had their turn. They still fade, and end when a fade that was to
    /// end them does, as in their turn (see [`Player::fade_or_end`]); the
    /// rest of their turn waits for the next block, where they run first,
    /// so busy threads cannot starve them.
    fn out_of_budget(&mut self, rest: &[u32]) {
        if !self.over_budget {
            self.over_budget = true;
            self.report(format!(
                "threads ran {BLOCK_STEPS} instructions in one block; the rest wait"
            ));
        }
        let first = self.threads.iter().map(|t| t.order).min().unwrap_or(0);
        let mut ended = Vec::new();
        for (place, &track) in rest.iter().enumerate() {
            if self.held.contains(&track) {
                continue;
            }
            let Some(mut thread) = self.threads.iter().find(|t| t.track == track).cloned() else {
                continue;
            };
            let id = thread.id;
            if !self.fade_or_end(&mut thread) {
                ended.push(id);
                continue;
            }
            thread.order = first - (rest.len() - place) as i64;
            if let Some(slot) = self.threads.iter_mut().find(|t| t.id == id) {
                *slot = thread;
            }
        }
        self.threads.retain(|t| !ended.contains(&t.id));
    }

    /// Runs one thread until it waits, ends, or the block's instructions
    /// run out (`budget`). False when it ended.
    fn step(&mut self, thread: &mut Thread, sources: &dyn Sources, budget: &mut usize) -> bool {
        if !self.fade_or_end(thread) {
            return false;
        }
        if thread.wake > self.frame {
            return true;
        }
        for _ in 0..STEPS {
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            let program = &self.modules[thread.module].program;
            let track_data = &self.track_data;
            let instruction = match program.decode(thread.pc, &|id| track_data.contains_key(&id)) {
                Ok(instruction) => instruction,
                Err(error) => return self.fault(thread, &error.to_string()),
            };
            let next = thread.pc + instruction.length;
            match self.execute(thread, &instruction, next, sources) {
                Ok(Flow::Next) => thread.pc = next,
                Ok(Flow::Jump(place)) => (thread.module, thread.pc) = place,
                Ok(Flow::Block) => return true,
                Ok(Flow::Yield) => {
                    thread.pc = next;
                    return true;
                }
                Ok(Flow::JumpYield(place)) => {
                    (thread.module, thread.pc) = place;
                    return true;
                }
                Ok(Flow::End) => match thread.stack.pop() {
                    Some(place) => (thread.module, thread.pc) = place,
                    None => {
                        // *(code)* 0x57e3ee → 0x57edc8 → vtable +0x34 (0x57dca3):
                        // with nothing to return to, `end` (and `return`) calls
                        // fade-out-and-kill. With a note still playing, the track
                        // writes FADE_DEST=0, FADE_VAR=EXTVOL, FADE_SPEED=100
                        // through `target`, sets its own `kill_after_fade` (+0x1cf),
                        // writes FADE_ON=1 through `target`, and stays running with
                        // its wait deadline (+0x198) untouched, so the run loop
                        // (0x57d8b3) continues at `next`. With no note playing, it
                        // kills the track at once (vtable +0x38, 0x57dd3e).
                        if self.notes.iter().any(|n| n.thread == thread.id) {
                            self.write(thread, i64::from(FADE_DEST), 0);
                            self.write(thread, i64::from(FADE_VAR), i32::from(EXTVOL));
                            self.write(thread, i64::from(FADE_SPEED), 100);
                            thread.kill_after_fade = true;
                            self.write(thread, i64::from(FADE_ON), 1);
                            thread.pc = next;
                        } else {
                            return false;
                        }
                    }
                },
                Ok(Flow::Kill) => {
                    let id = thread.id;
                    self.notes.retain(|n| n.thread != id);
                    return false;
                }
                Err(message) => return self.fault(thread, &message),
            }
        }
        self.fault(thread, "ran too long without waiting")
    }

    /// Moves the thread's fade one step; false when that fade was to end
    /// the track and has ended (0x57d86e to 0x57d88d): the track ends at
    /// once, with its notes.
    fn fade_or_end(&mut self, thread: &mut Thread) -> bool {
        self.fade(thread);
        if thread.kill_after_fade && thread.registers[usize::from(FADE_ON)] == 0 {
            let id = thread.id;
            self.notes.retain(|n| n.thread != id);
            return false;
        }
        true
    }

    /// Moves a fading register one step, as the original's run loop does
    /// before it runs a track's instructions: toward `fade_dest` by
    /// `fade_speed`, whichever sign the speed has, stopping when it passes
    /// the destination. A speed of 0 moves nothing and ends the fade.
    /// (The original adds in 32 bits and can wrap past the destination;
    /// here the sum is wide, so it stops there.)
    fn fade(&mut self, thread: &mut Thread) {
        let register = |r: u8| thread.registers[usize::from(r)];
        if register(FADE_ON) == 0 {
            return;
        }
        let (var, dest, speed) = (
            i64::from(register(FADE_VAR)),
            register(FADE_DEST),
            register(FADE_SPEED),
        );
        if speed == 0 {
            thread.registers[usize::from(FADE_ON)] = 0;
        }
        let value = self.read(thread, var);
        let mut speed = i64::from(speed);
        if (value < dest) != (speed > 0) {
            speed = -speed;
        }
        let mut value = i64::from(value) + speed;
        let dest = i64::from(dest);
        if (speed > 0 && value > dest) || (speed < 0 && value < dest) {
            value = dest;
            thread.registers[usize::from(FADE_ON)] = 0;
        }
        // Short of the destination, the value lies between it and an i32.
        self.write(thread, var, value as i32);
    }

    /// The group a `seqgroup_*` instruction names: the value of its
    /// register operand, which must be 1 to 127. With `claim`, a group
    /// with no slot yet is an error once [`SEQGROUPS`] exist.
    fn seqgroup(
        &self,
        thread: &Thread,
        instruction: &Instruction,
        claim: bool,
    ) -> Result<i32, String> {
        let group = match instruction.values().first() {
            Some(&r) if (1..=127).contains(&r) => self.read(thread, r),
            _ => return Err(format!("{}: bad register", instruction.name())),
        };
        if claim && self.seqgroups.len() >= SEQGROUPS && !self.seqgroups.contains_key(&group) {
            return Err(format!("more than {SEQGROUPS} sequence groups"));
        }
        Ok(group)
    }

    /// Records a fault for `thread` without stopping it or cutting its
    /// notes (used for non-fatal diagnostics such as a refused
    /// `getsrcdatafield` query, 0x57f33a → 0x57f3f9 → 0x57f408).
    fn report_fault(&mut self, thread: &Thread, message: &str) {
        let name = &self.modules[thread.module].name;
        self.report(format!(
            "track {} ({name}) at {:#x}: {message}",
            thread.track, thread.pc
        ));
    }

    /// Records a fatal fault for `thread`, cuts its active note, and
    /// returns `false` so [`Player::step`] terminates the thread.
    fn fault(&mut self, thread: &Thread, message: &str) -> bool {
        self.report_fault(thread, message);
        let id = thread.id;
        self.notes.retain(|n| n.thread != id);
        false
    }

    /// *(code)* Where a register access other than to `target` (53) goes
    /// (`0x57de16`, `0x57de57`): follows `target` (`[ecx + 0xc8]`) across
    /// track objects. A stopped destination track has its own `target`
    /// cleared by kill (`0x57dd87`), so the chain ends at
    /// [`Destination::Stopped`] without falling back to the caller. A
    /// cycle stops before revisiting a track already passed.
    fn resolve_target(&self, thread: &Thread) -> Destination {
        let mut dest = Destination::Own;
        let mut target = thread.target;
        let mut passed = vec![thread.track];
        while let Target::Track(track) = target {
            if passed.contains(&track) {
                break;
            }
            passed.push(track);
            if let Some(next) = self.threads.iter().find(|t| t.track == track) {
                dest = Destination::Running(next.id);
                target = next.target;
            } else {
                dest = Destination::Stopped(track);
                break;
            }
        }
        dest
    }

    /// Reads a register. Registers below 100 other than `target` (53)
    /// come from the destination [`Player::resolve_target`] picks;
    /// globals are shared.
    fn read(&self, thread: &Thread, register: i64) -> i32 {
        let Ok(r) = u8::try_from(register) else {
            return 0;
        };
        if r >= GLOBALS {
            return self.globals[usize::from(r - GLOBALS)];
        }
        if r == TARGET {
            return thread.registers[usize::from(r)];
        }
        match self.resolve_target(thread) {
            Destination::Own => thread.registers[usize::from(r)],
            Destination::Running(id) => {
                self.threads
                    .iter()
                    .find(|t| t.id == id)
                    .unwrap_or(thread)
                    .registers[usize::from(r)]
            }
            Destination::Stopped(track) => self.idle.get(&track).map_or_else(
                || default_register(r),
                |idle| idle.registers[usize::from(r)],
            ),
        }
    }

    /// Writes a register, with the original's side effects (`0x57de57`,
    /// `0x57de98`). Every write other than to `target` (53) goes to the
    /// destination [`Player::resolve_target`] picks, globals included,
    /// and sets that destination's comparison flags from the value.
    /// Writing `target` (`0x57e018`..`0x57e036`) looks up the track in
    /// the registry (`0x573312`): a registered track (other than this
    /// one) is stored as [`Target::Track`], which outlives a kill and
    /// reaches the track again on a later start; 0, this track, or an
    /// unregistered ID sets [`Target::Own`]. Writing to a stopped
    /// destination does not fall back to the caller, and forces
    /// `fade_on` (45) to 0 (`0x57e059`..`0x57e064`). `main_duckpri`
    /// (123) records the written thread's request and holds the largest.
    fn write(&mut self, thread: &mut Thread, register: i64, value: i32) {
        let Ok(r) = u8::try_from(register) else {
            return;
        };
        if r == TARGET {
            thread.registers[usize::from(r)] = value;
            thread.kept = i64::from(value);
            thread.target = if value != 0 {
                let target_track = value as u32;
                if target_track != thread.track && self.owner(target_track).is_some() {
                    Target::Track(target_track)
                } else {
                    Target::Own
                }
            } else {
                Target::Own
            };
            return;
        }
        let dest = self.resolve_target(thread);
        let written = match dest {
            Destination::Own => thread,
            Destination::Running(id) => match self.threads.iter_mut().find(|t| t.id == id) {
                Some(other) => other,
                None => thread,
            },
            Destination::Stopped(stopped_track) => {
                if r < GLOBALS {
                    if let Some(idle) = self.idle.get_mut(&stopped_track) {
                        idle.kept = i64::from(value);
                        // *(code)* 0x57e059..0x57e064: writing `fade_on`
                        // to a non-running track clears it back to 0.
                        idle.registers[usize::from(r)] = if r == FADE_ON { 0 } else { value };
                    }
                } else if r != MAIN_DUCKPRI {
                    self.globals[usize::from(r - GLOBALS)] = value;
                }
                return;
            }
        };
        written.kept = i64::from(value);
        let id = written.id;
        if r < GLOBALS {
            written.registers[usize::from(r)] = value;
        } else if r == MAIN_DUCKPRI {
            if value == 0 {
                self.duck.remove(&id);
            } else {
                self.duck.insert(id, value);
            }
            self.update_main_duckpri();
        } else {
            self.globals[usize::from(r - GLOBALS)] = value;
        }
    }

    /// Where a track data ID leads, whichever module's program is
    /// running ([`entry_points`]).
    fn target(&self, id: u32) -> Result<Place, String> {
        self.track_data
            .get(&id)
            .copied()
            .ok_or_else(|| format!("no track data {id:#x}"))
    }

    /// Loads a patch's samples into the cache, if they are not there.
    /// False (reported) when it has none. An MP3 is resampled to [`RATE`]
    /// (see the module notes).
    fn load_patch(&mut self, patch: u32) -> bool {
        if !self.cache.contains_key(&patch) {
            let pcm = match self.patches.get(&patch).map(|(_, path)| path.clone()) {
                Some(path) => {
                    let pcm = self.samples.load(&path);
                    if pcm.is_none() {
                        self.report(format!("patch {patch}: no sample at {path}"));
                    }
                    let mp3 = path.to_ascii_lowercase().ends_with(".mp3");
                    match pcm {
                        Some(pcm) if mp3 => {
                            let resampled = resample(&pcm, RATE);
                            if resampled.is_none() {
                                self.report(format!("patch {patch}: too long at {RATE} Hz"));
                            }
                            resampled
                        }
                        pcm => pcm,
                    }
                }
                None => {
                    // Not cached: any number may name a missing patch,
                    // so caching them would grow without bound. (A track
                    // object fails too, 0x57f703.)
                    if self.tracks.contains_key(&patch) {
                        self.report(format!("patch {patch} is a track"));
                    } else {
                        self.report(format!("no patch {patch}"));
                    }
                    return false;
                }
            };
            self.cache.insert(patch, pcm);
        }
        matches!(self.cache.get(&patch), Some(Some(_)))
    }

    fn note_on(&mut self, thread: &Thread) -> u32 {
        let patch = thread.registers[usize::from(PATCH)] as u32;
        if !self.load_patch(patch) {
            return 0;
        }
        // *(code)* 0x57f595 → 0x58015a..0x58018d: each track owns one
        // active voice ([esi + 0x1d0]), so starting a note replaces any
        // prior note of the same thread (or idle track when thread == 0).
        if thread.id != 0 {
            self.notes.retain(|n| n.thread != thread.id);
        } else {
            self.notes
                .retain(|n| !(n.thread == 0 && n.track == thread.track));
        }
        if self.notes.len() >= NOTES {
            self.report(format!("patch {patch}: more than {NOTES} notes"));
            return 0;
        }
        let levels = self.levels(thread);
        let id = self.id();
        self.notes.push(Note {
            id,
            thread: thread.id,
            track: thread.track,
            patch,
            position: 0,
            step: step(levels.rate),
            gains: levels.gains,
            chain: Vec::new(),
            looping: false,
        });
        id
    }

    fn execute(
        &mut self,
        thread: &mut Thread,
        instruction: &Instruction,
        next: u32,
        sources: &dyn Sources,
    ) -> Result<Flow, String> {
        let v = instruction.values();
        let arg = |i: usize| v[i];
        let kept = thread.kept;
        let branch = |taken: bool| -> Result<Flow, String> {
            if taken {
                Ok(Flow::Jump(self.target(arg(0) as u32)?))
            } else {
                Ok(Flow::Next)
            }
        };
        match instruction.name() {
            "ifeq" => return branch(kept == 0),
            "ifne" => return branch(kept != 0),
            "ifgt" => return branch(kept > 0),
            "iflt" => return branch(kept < 0),
            "ifge" => return branch(kept >= 0),
            "ifle" => return branch(kept <= 0),
            _ => {}
        }
        let wait_until = |player: &Self, thread: &Thread, holds: fn(i32, i32) -> bool| {
            if holds(player.read(thread, arg(0)), player.read(thread, arg(1))) {
                Flow::Next
            } else {
                Flow::Block
            }
        };
        let flow = match instruction.name() {
            "note_on" => {
                let id = self.note_on(thread);
                thread.last_note = id;
                self.write(thread, arg(0), id as i32);
                Flow::Next
            }
            "note_off" => {
                // *(code)* 0x57e192..0x57e1a9 → 0x5801a0: stops the track's
                // single active voice ([esi + 0x1d0]) unconditionally; the
                // voice's stop method ignores the raw operand byte.
                let id = thread.id;
                self.notes.retain(|n| n.thread != id);
                thread.last_note = 0;
                Flow::Next
            }
            "loadb" | "loadl" => {
                self.write(thread, arg(0), arg(1) as i32);
                Flow::Next
            }
            "set" | "setll" | "setlt" | "settl" => {
                let value = self.read(thread, arg(1));
                self.write(thread, arg(0), value);
                Flow::Next
            }
            "call" => {
                if thread.stack.len() >= CALL_DEPTH {
                    return Err(format!("calls nested deeper than {CALL_DEPTH}"));
                }
                thread.stack.push((thread.module, next));
                Flow::Jump(self.target(arg(0) as u32)?)
            }
            "return" | "end" => Flow::End,
            "wait" => {
                let ms = i64::from(self.read(thread, arg(0)).max(0));
                thread.wake = self.frame + (ms as u64 * u64::from(RATE)).div_ceil(1000);
                Flow::Yield
            }
            "wait_samp" => {
                if self.notes.iter().any(|n| n.id == thread.last_note) {
                    Flow::Block
                } else {
                    Flow::Next
                }
            }
            "jump" => {
                let id = match instruction.operands[0].0 {
                    Operand::Register => self.read(thread, arg(0)) as u32,
                    _ => arg(0) as u32,
                };
                Flow::Jump(self.target(id)?)
            }
            "test" => {
                thread.kept = i64::from(self.read(thread, arg(0)));
                Flow::Next
            }
            "nop" => Flow::Next,
            "add" | "sub" | "mul" | "div" => {
                let (a, b) = (self.read(thread, arg(0)), self.read(thread, arg(1)));
                let value = match instruction.name() {
                    "add" => a.wrapping_add(b),
                    "sub" => a.wrapping_sub(b),
                    "mul" => a.wrapping_mul(b),
                    _ if b == 0 => return Err("division by zero".into()),
                    _ => a.wrapping_div(b),
                };
                self.write(thread, arg(0), value);
                Flow::Next
            }
            "cmp" => {
                thread.kept =
                    i64::from(self.read(thread, arg(0))) - i64::from(self.read(thread, arg(1)));
                Flow::Next
            }
            "rand" => {
                let (low, high) = (self.read(thread, arg(1)), self.read(thread, arg(2)));
                let span =
                    (i64::from(high) - i64::from(low) + 1).clamp(1, i64::from(u32::MAX)) as u32;
                let value = low.wrapping_add(self.random(span) as i32);
                self.write(thread, arg(0), value);
                Flow::Next
            }
            "set_loop" => {
                thread.loop_point = (thread.module, next);
                Flow::Next
            }
            "loop" => Flow::JumpYield(thread.loop_point),
            "smart_setlist" => {
                let id = self.read(thread, arg(0)) as u32;
                match self.hit_lists.get(&id) {
                    Some(list) => thread.hit_list = list.clone(),
                    None => return Err(format!("no hit list {id}")),
                }
                Flow::Next
            }
            "smart_choose" => {
                let value = match thread.hit_list.len() {
                    0 => 0,
                    n => thread.hit_list[self.random(n as u32) as usize] as i32,
                };
                self.write(thread, arg(0), value);
                Flow::Next
            }
            "play_trk" => {
                // *(code)* 0x57f1f2: the track starts (and is let go,
                // vtable +0x2c) with the caller's own argument registers
                // (read straight from its bank at +0x178), then takes
                // the caller's `source` (47), read as any register is,
                // through its `target` (vtable +0x88), which sets its
                // flags. The new thread's `target` is itself.
                let track = self.read(thread, arg(0)) as u32;
                let args = [1, 2, 3].map(|r| thread.registers[r] as u32);
                // A track that starts itself is the one running: the
                // start acts on it as it is now.
                let own = track == thread.track;
                if own && let Some(stored) = self.threads.iter_mut().find(|t| t.id == thread.id) {
                    *stored = thread.clone();
                }
                let started = self.start(args, track);
                self.let_go_track(track, &started.threads);
                if own && let Some(stored) = self.threads.iter().find(|t| t.id == thread.id) {
                    *thread = stored.clone();
                }
                // `source` is read after the start (0x57f256), through the
                // caller's `target` as the start left it, and written to
                // the started track through its own `target` (vtable +0x84,
                // 0x57f261).
                let source = self.read(thread, i64::from(SOURCE));
                if let Some(stored) = self.threads.iter_mut().find(|t| t.id == thread.id) {
                    *stored = thread.clone();
                }
                for &started_id in &started.threads {
                    if let Some(at) = self.threads.iter().position(|t| t.id == started_id) {
                        self.write_at(at, SOURCE, source);
                    }
                }
                if let Some(stored) = self.threads.iter().find(|t| t.id == thread.id) {
                    *thread = stored.clone();
                }
                // Started anew, it runs from the top, from the next block
                // (the original goes on in the same update).
                if own && started.fresh {
                    return Ok(Flow::JumpYield((thread.module, thread.pc)));
                }
                Flow::Next
            }
            "kill_trk" => {
                // *(code)* 0x57f284: the track ends at once (vtable
                // +0x38), whatever object it plays for; one track
                // object plays for all of them in the original.
                let track = self.read(thread, arg(0)) as u32;
                self.end_track(track, Some(thread.id));
                if track == thread.track {
                    Flow::Kill
                } else {
                    Flow::Next
                }
            }
            "seqgroup_kill" => {
                let group = self.seqgroup(thread, instruction, true)?;
                let me = thread.track;
                let mut flow = Flow::Next;
                let holder = self.seqgroups.get(&group).copied().unwrap_or(0);
                // The holder is a track number; its newest running thread
                // is the one acted on. A track with none running is
                // skipped (the original looks it up and does not check).
                let newest = self
                    .threads
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.track == holder)
                    .max_by_key(|(_, t)| t.id)
                    .map(|(at, _)| at);
                if holder != 0
                    && holder != me
                    && let Some(at) = newest
                {
                    let other_id = self.threads[at].id;
                    let other = &mut self.threads[at];
                    let reg = |r: u8| usize::from(r);
                    if other.registers[reg(EXTVOL)] == 0 {
                        self.threads.remove(at);
                        self.notes.retain(|n| n.thread != other_id);
                        self.forget_ended();
                    } else {
                        let other_kept = &mut other.kept;
                        let other = &mut other.registers;
                        let fading =
                            other[reg(FADE_ON)] != 0 && other[reg(FADE_VAR)] == i32::from(EXTVOL);
                        if !fading {
                            other[reg(FADE_SPEED)] = 200;
                            other[reg(FADE_VAR)] = i32::from(EXTVOL);
                            other[reg(FADE_ON)] = 1;
                            *other_kept = 1;
                        }
                        // Run this instruction again shortly; by then
                        // this thread holds the group.
                        thread.wake =
                            self.frame + (SEQGROUP_WAIT_MS * u64::from(RATE)).div_ceil(1000);
                        flow = Flow::Block;
                    }
                }
                self.seqgroups.insert(group, me);
                flow
            }
            "seqgroup_return" => {
                let group = self.seqgroup(thread, instruction, true)?;
                let me = thread.track;
                let holder = self.seqgroups.insert(group, me).unwrap_or(0);
                if holder != 0 && holder != me {
                    Flow::Kill
                } else {
                    Flow::Next
                }
            }
            "seqgroup_trkid" => {
                // The group's register first, then where its holder goes.
                let group = self.seqgroup(thread, instruction, false)?;
                let holder = self.seqgroups.get(&group).copied().unwrap_or(0);
                self.write(thread, arg(1), holder as i32);
                Flow::Next
            }
            "getsrcdatafield" => {
                // The original passes both registers' values (0x57f2e8)
                // and logs a refused field (0x57f33a → 0x57f3f9 → 0x57f408),
                // going on to the next instruction with its voice intact.
                let (source, field) = (self.read(thread, arg(1)), self.read(thread, arg(2)));
                match sources.field(source, field) {
                    Some(value) => self.write(thread, arg(0), value),
                    None => {
                        self.report_fault(thread, "Illegal source parameter request");
                    }
                }
                Flow::Next
            }
            "waiteq" => wait_until(self, thread, |a, b| a == b),
            "waitne" => wait_until(self, thread, |a, b| a != b),
            "waitgt" => wait_until(self, thread, |a, b| a > b),
            "waitlt" => wait_until(self, thread, |a, b| a < b),
            "waitge" => wait_until(self, thread, |a, b| a >= b),
            "waitle" => wait_until(self, thread, |a, b| a <= b),
            other => return Err(format!("{other} is not supported")),
        };
        Ok(flow)
    }
}

/// Writes a placement into a thread's own registers, not through its
/// `target`, as the write sets the flags: `extvol`, then `pan`.
fn place_itself(thread: &mut Thread, vol: i32, pan: i32) {
    thread.registers[usize::from(EXTVOL)] = vol;
    thread.registers[usize::from(PAN)] = pan;
    thread.kept = i64::from(pan);
}

/// Mixes a note onto `sums` (stereo frames): a mono sample plays on both
/// channels; a stereo one keeps its own. A note that reaches its end goes
/// on with the first patch chained to it, or from its start when it
/// loops, from where its position passed the end, so nothing is left out
/// between them. False when it has ended.
fn mix_note(note: &mut Note, cache: &BTreeMap<u32, Option<Pcm>>, sums: &mut [i64]) -> bool {
    let mut slots = sums.as_chunks_mut::<2>().0.iter_mut();
    loop {
        let Some(Some(pcm)) = cache.get(&note.patch) else {
            return false;
        };
        let channels = usize::from(pcm.channels).max(1);
        let length = pcm.frames() as u64;
        if note.position >> 16 >= length {
            if note.looping && length > 0 {
                note.position %= length << 16;
                continue;
            }
            // Patches are loaded when chained, so each is cached.
            if note.chain.is_empty() {
                return false;
            }
            note.position -= length << 16;
            note.patch = note.chain.remove(0);
            continue;
        }
        let sample = |frame: usize, channel: usize| -> i64 {
            i64::from(pcm.samples[frame * channels + channel.min(channels - 1)])
        };
        let mut ended = false;
        for slot in slots.by_ref() {
            let index = note.position >> 16;
            let at = index as usize;
            let after = if index + 1 < length { at + 1 } else { at };
            // A difference of two samples times a 16-bit fraction needs
            // more than 32 bits.
            let fraction = i64::from((note.position & 0xffff) as u32);
            for (channel, sum) in slot.iter_mut().enumerate() {
                let (first, second) = (sample(at, channel), sample(after, channel));
                let value = first + (((second - first) * fraction) >> 16);
                *sum += (value * i64::from(note.gains[channel])) >> 16;
            }
            note.position += note.step;
            if note.position >> 16 >= length {
                ended = true;
                break;
            }
        }
        if !ended {
            return true;
        }
    }
}

/// An ambience voice's gains: its volume through the sound layer, as a
/// track's (see [`crate::levels`]), and its pan.
fn ambient_gains(volume: i32, pan: i32, master: i32) -> [i32; 2] {
    levels::gains(
        levels::attenuation(i64::from(volume), i64::from(master)),
        levels::pan(pan),
    )
}

/// The player's parts an ambience player works on.
struct AmbientHost<'a, S> {
    samples: &'a mut S,
    cache: &'a mut BTreeMap<u32, Option<Pcm>>,
    ambient_patches: &'a mut BTreeMap<String, u32>,
    voices: &'a mut Vec<AmbientVoice>,
    random: &'a mut u64,
    master: i32,
    faults: &'a mut Vec<String>,
}

impl<S: Samples> AmbientHost<'_, S> {
    fn report(&mut self, message: String) {
        if self.faults.len() < FAULTS {
            self.faults.push(message);
        }
    }

    /// The sample's number in the cache, loading it the first time.
    fn patch(&mut self, path: &str) -> u32 {
        if let Some(&patch) = self.ambient_patches.get(path) {
            return patch;
        }
        let patch = u32::try_from(self.ambient_patches.len()).unwrap_or(u32::MAX);
        let pcm = self.samples.load(path);
        if pcm.is_none() {
            self.report(format!("ambience: no sample at {path}"));
        }
        let pcm = match pcm {
            Some(pcm) if path.to_ascii_lowercase().ends_with(".mp3") => resample(&pcm, RATE),
            pcm => pcm,
        };
        self.ambient_patches.insert(path.to_owned(), patch);
        self.cache.insert(patch, pcm);
        patch
    }
}

impl<S: Samples> Host for AmbientHost<'_, S> {
    fn rand(&mut self) -> i32 {
        xorshift(self.random, 32_768) as i32
    }

    fn load(&mut self, path: &str) -> Option<u32> {
        let patch = self.patch(path);
        self.cache.get(&patch)?.as_ref().map(|pcm| pcm.rate)
    }

    fn play(&mut self, voice: Voice) {
        self.voices.retain(|v| v.key != voice.key);
        if self.voices.len() >= NOTES {
            self.report(format!("ambience: more than {NOTES} voices"));
            return;
        }
        let patch = self.patch(&voice.path);
        self.voices.push(AmbientVoice {
            key: voice.key,
            volume: voice.volume,
            pan: voice.pan,
            note: Note {
                id: 0,
                thread: 0,
                track: 0,
                patch,
                position: 0,
                step: step(voice.rate),
                gains: ambient_gains(voice.volume, voice.pan, self.master),
                chain: Vec::new(),
                looping: voice.looping,
            },
        });
    }

    fn set_volume(&mut self, key: Key, volume: i32) {
        let master = self.master;
        for voice in self.voices.iter_mut().filter(|v| v.key == key) {
            voice.volume = volume;
            voice.note.gains = ambient_gains(volume, voice.pan, master);
        }
    }

    fn stop(&mut self, key: Key) {
        self.voices.retain(|v| v.key != key);
    }

    fn playing(&self, key: Key) -> bool {
        self.voices.iter().any(|v| v.key == key)
    }
}
