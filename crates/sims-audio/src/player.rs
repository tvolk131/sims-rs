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
//!   its object and two zeros; what the original passes there is not
//!   read.
//! - *(code)* The registers in [`DEFAULTS`] start as the original's
//!   default register bank holds them, then `vol` (18) and `duckpri` (25)
//!   take the HOT track's volume and duck priority; every other starts
//!   at 0.
//! - *(code)* Every register write sets the written thread's comparison
//!   flags from the value; `cmp a, b` sets them from `a - b` and `test r`
//!   from `r`. The `if` branches test them.
//! - *(code)* Writing `target` (53) with a track number sends this
//!   thread's later reads and writes of registers other than 53 to the
//!   newest running thread of that track, and on through that thread's
//!   own `target`; 0, or a track with none running, sends them back here.
//!   How the original picks among several threads of one track, and what
//!   it does once the target has ended, are not known (see
//!   `Player::resolve`).
//! - *(code)* `main_duckpri` (123) is a request per thread: the global
//!   holds the largest, and a thread's request goes as soon as it ends.
//! - *(code)* Volume, pan and pitch: see [`crate::levels`]. Each note
//!   plays at its thread's levels, updated after the threads run in each
//!   block and when the thread ends (the original updates them as
//!   registers change). A note whose thread has ended keeps its last
//!   levels. A sample plays at the rate
//!   its pitch sets, whatever its own rate.
//! - `note_on` starts the patch in register 50 and writes the note's ID
//!   to its operand; `wait_samp` waits until the thread's last note ends.
//! - `end` returns from a `call`, or ends the thread when nothing called
//!   it. (No program in the install has a `return`, against 2,504 calls.)
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
//! - 1, play: starts the track. When the track's arguments are
//!   `kArgsIdVolPan`, its object is recorded as an *instance* of the
//!   track, and the track's `extvol` and `pan` are set from where all its
//!   objects are ([`crate::placement`]); an object of 0 starts nothing.
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
//!   buttons and the main music loop ([`PAUSE_SPARES`]), and the live
//!   ambience; an unpause lets them all go on from where they were. See
//!   [`Player::pause`].
//! - 30, `kTurnOnTv` (radios too): kills the object's instances, then
//!   records the object as an instance of the track (even 0), starts it
//!   and sets its `extvol` and `pan` as kind 1 does.
//! - 53, `kSequenceTrackHitList`: picks a patch from a hit list and
//!   chains it to the note the track is playing, to start with no gap
//!   when that one ends; with no note playing, starts it.
//! - 52, `kSetAmbienceMode`: makes a score current and starts it,
//!   stopping the one before (see [`crate::ambience`] and
//!   [`Player::add_scores`]); a mode with no score stops the current one.
//! - 36, `kSetMusicMode`: only its ambience half (see
//!   `Player::set_music_mode`); the music half is reported as a fault.
//!
//! The game's code also calls kind 33 to move the ambience's cursor from
//! the camera and the hour: [`Player::set_ambience_view`]. The scores'
//! voices mix with the notes, at the effects group's volume.
//!
//! The radio, television and music hit lists are built from directories
//! (see [`crate::stations`] and [`Player::add_stations`]). An MP3 is
//! resampled to [`RATE`] when loaded: the original decodes MP3s through
//! a data source and then sets the buffer's rate from `pitch` as for any
//! sound, so a 44.1 kHz song at pitch 3600 would otherwise play an octave
//! low. That the data source delivers 22,050 Hz is **unverified**.
//!
//! The model differs from the original's here: the original has one
//! track object per track number, shared by every object playing it,
//! while this player runs a thread per play, and takes a track's newest
//! thread where the original would use its one object. When an object
//! lets go of a track others still have, the original re-evaluates the
//! shared track (0x5746d9, not read); here that object's threads fade
//! out and end. Kinds not ported (the music, the piano, reloading, and the sound object calls only the game's
//! code makes) are reported as faults; kinds whose case does nothing for
//! sound are ignored.
//!
//! **Unverified** against the original at run time: all of the above.
//! Left out: priorities, and the limit of 16 voices kind 1 checks first
//! (0x574a87); placing every track again when the camera moves (kind 33,
//! see [`crate::placement`]); the track group's own volume beyond the host's [`Player::set_group_volume`], and
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
//! that does not exist is not cached. How the original picks among
//! duplicate events, patches and hit lists across modules is unknown;
//! here the module whose name sorts first wins.

use crate::ambience::{self, Fresh, Host, Key, Voice};
use crate::files::SoundFiles;
use crate::fsc::Score;
use crate::hit::{Instruction, Operand};
use crate::levels::{self, Inputs, Levels};
use crate::module::Module;
use crate::placement::{self, Projection};
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

/// The track argument kind `kArgsIdVolPan`: `arg1` is the object.
pub const ARGS_ID_VOL_PAN: i64 = 2;
/// The `extvol` register, which `seqgroup_kill` fades out.
pub const EXTVOL: u8 = 19;
/// The fade registers: the value to reach, the register that moves, how
/// far it moves per block, and whether it is moving.
pub const FADE_DEST: u8 = 40;
pub const FADE_VAR: u8 = 41;
pub const FADE_SPEED: u8 = 42;
pub const FADE_ON: u8 = 45;
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
/// Registers with side effects: `vol`, `duckpri`, 3D, `target` and the
/// global `main_duckpri`.
pub const VOL: u8 = 18;
pub const PAN: u8 = 20;
pub const PITCH: u8 = 21;
pub const DUCKPRI: u8 = 25;
pub const THREE_D: u8 = 26;
pub const TARGET: u8 = 53;
pub const MAIN_DUCKPRI: u8 = 123;
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
    /// reported when an event has one.
    /// Kind 36 is listed though its ambience half is ported (see
    /// `Player::set_music_mode`).
    pub const NOT_PORTED: [i64; 33] = [
        2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 24, 25, 26, 32, 33, 34, 35, 36, 37,
        38, 39, 40, 41, 42, 43, 103, 104,
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
}

/// *(code)* Writes a starting track's argument registers 1 to 3, as the
/// original's start does whatever the track's argument kind (0x57d9cb to
/// 0x57d9eb, each a register write that sets the comparison flags).
fn set_arguments(thread: &mut Thread, args: [u32; 3]) {
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

/// *(code)* An `extvol` and `pan` to work out from where a track's objects
/// are, at the next block, for the threads a start started or let go.
/// The objects and `Spl` are as they were when the event came.
#[derive(Debug, Clone)]
struct Placing {
    objects: Vec<u32>,
    spl: i32,
    threads: Vec<u32>,
}

#[derive(Debug, Clone)]
struct Thread {
    id: u32,
    object: u32,
    module: usize,
    track: u32,
    pc: u32,
    registers: [i32; GLOBALS as usize],
    stack: Vec<u32>,
    loop_point: u32,
    kept: i64,
    hit_list: Vec<u32>,
    last_note: u32,
    wake: u64,
    /// The track's HOT group (`kGroupSfx` 1, `kGroupMusic` 2, `kGroupVox`
    /// 3), whose volume scales its own.
    group: i64,
    /// Where register reads and writes go: this thread, or the thread
    /// register 53 named (see [`Player::resolve`]).
    target: Target,
    /// End the thread when its fade ends (the original's flag at +0x1cf).
    kill_after_fade: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Own,
    Thread(u32),
}

#[derive(Debug, Clone)]
struct Note {
    id: u32,
    /// The thread that started it: 0 for one a kind 53 event started on
    /// a track with no thread.
    thread: u32,
    track: u32,
    object: u32,
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
fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)).max(1)
}

/// What one instruction did to its thread.
enum Flow {
    Next,
    Jump(u32),
    /// Stay on this instruction and run it again next block.
    Block,
    /// Go on to the next instruction next block.
    Yield,
    /// Go to this address next block.
    JumpYield(u32),
    End,
    /// End the thread, whatever called it, and stop its notes.
    Kill,
}

/// The HIT VM and mixer.
pub struct Player<S> {
    modules: Vec<Module>,
    samples: S,
    /// Patch ID to (module, path), the first module's winning.
    patches: BTreeMap<u32, (usize, String)>,
    hit_lists: BTreeMap<u32, Vec<u32>>,
    cache: BTreeMap<u32, Option<Pcm>>,
    globals: [i32; 256 - GLOBALS as usize],
    threads: Vec<Thread>,
    /// Threads started by `play_trk`, which first run in the next block.
    pending: Vec<Thread>,
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
    /// *(code)* Object instances: a track and an object it plays for, as
    /// the original records them (its map at manager +0xd4).
    instances: BTreeSet<(u32, u32)>,
    /// The ambience scores, by mode (see [`crate::ambience`]).
    scores: BTreeMap<u32, Fresh>,
    /// *(code)* The current ambience (manager +0x100), by mode.
    ambience: Option<u32>,
    /// *(code)* The music mode (manager +0xe8, 4 at first), and whether
    /// mode 3 has been set, after which modes are recorded (0x6f6428).
    music_mode: i64,
    music_started: bool,
    /// *(code)* Whether the sound is paused (manager +0xd).
    paused: bool,
    /// *(code)* The tracks a pause holds (each track's +0x1c8): their
    /// threads do not run or fade and their notes stay where they are,
    /// silent, until an unpause or until the track starts again.
    held: BTreeSet<u32>,
    /// Volumes and pans to work out at the next block, in order.
    placing: Vec<Placing>,
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
        let mut patches = BTreeMap::new();
        let mut hit_lists = BTreeMap::new();
        for (index, module) in modules.iter().enumerate() {
            for (&id, patch) in &module.hot.patches {
                patches
                    .entry(id)
                    .or_insert_with(|| (index, patch.path.clone()));
            }
            for (&id, list) in &module.hot.hit_lists {
                hit_lists.entry(id).or_insert_with(|| list.clone());
            }
        }
        Self {
            modules,
            samples,
            patches,
            hit_lists,
            cache: BTreeMap::new(),
            globals: [0; 256 - GLOBALS as usize],
            threads: Vec::new(),
            pending: Vec::new(),
            over_budget: false,
            seqgroups: BTreeMap::new(),
            duck: BTreeMap::new(),
            group_volumes: BTreeMap::new(),
            master: 1024,
            notes: Vec::new(),
            instances: BTreeSet::new(),
            scores: BTreeMap::new(),
            ambience: None,
            music_mode: 4,
            music_started: false,
            paused: false,
            held: BTreeSet::new(),
            placing: Vec::new(),
            voices: Vec::new(),
            ambient_patches: BTreeMap::new(),
            ambient_cache: BTreeMap::new(),
            frame: 0,
            ran_at: None,
            next_id: 1,
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
    }

    /// Sets the master volume, 0 to 1024.
    pub fn set_master_volume(&mut self, volume: i32) {
        self.master = volume.clamp(0, 1024);
        for voice in &mut self.voices {
            voice.note.gains = ambient_gains(voice.volume, voice.pan, self.master);
        }
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

    /// *(code)* What the game's kind 33 does, it seems once a frame
    /// (0x574157): moves the current ambience's cursor. `view` is the
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
    /// otherwise the current one stops and the new one starts. (A mode
    /// with no score crashes the original; here the current one stops
    /// and none is current.)
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
            self.with_score(new, |score, _| score.start(frame));
        }
    }

    /// *(code)* The ambience half of kind 36 (0x573fe2): the music mode is
    /// recorded (once mode 3 has been set) and then, in mode 0 (live),
    /// the current ambience starts if it is stopped; in mode 3 (the
    /// neighbourhood) the backyard score becomes current and starts, its
    /// cursor first set to (1000, 500) per mille, which starting resets
    /// when it was stopped; in any other mode the current ambience stops.
    /// The music half is not ported.
    fn set_music_mode(&mut self, mode: i64) {
        if mode != self.music_mode {
            if mode == 3 {
                self.music_started = true;
            }
            if self.music_started {
                self.music_mode = mode;
            }
        }
        let frame = self.frame;
        match self.music_mode {
            0 => {
                if let Some(current) = self.ambience {
                    self.with_score(current, |score, _| score.start(frame));
                }
            }
            3 => {
                if !self.scores.contains_key(&0) {
                    return;
                }
                if let Some(current) = self.ambience.filter(|&m| m != 0) {
                    self.with_score(current, |score, host| score.stop(host));
                }
                self.ambience = Some(0);
                self.with_score(0, |score, _| {
                    score.set_x(1000);
                    score.set_y(500);
                    score.start(frame);
                });
            }
            _ => {
                if let Some(current) = self.ambience {
                    self.with_score(current, |score, host| score.stop(host));
                }
            }
        }
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
            && self.pending.is_empty()
            && self.notes.is_empty()
            && self.voices.is_empty()
            && self.scores.values().all(|s| !s.is_started())
    }

    /// Adds the stations' hit lists and patches (see
    /// [`crate::stations`]), which the original builds from directories
    /// when it starts. A hit list the sound tables already list is added
    /// to; a patch number already taken is left alone.
    pub fn add_stations(&mut self, lists: &StationLists) {
        for (&id, list) in &lists.hit_lists {
            self.hit_lists.entry(id).or_default().extend(list);
        }
        for (&id, path) in &lists.patches {
            self.patches.entry(id).or_insert_with(|| (0, path.clone()));
        }
    }

    /// Acts on one event from the simulation.
    pub fn handle(&mut self, event: &SoundEvent) {
        match event {
            SoundEvent::Play { object, name } => self.play(*object, name),
            // What the game calls with kind 23 when an object goes.
            SoundEvent::StopObject { object } => self.kill_instance(*object),
            // Kind 20, as the game calls it.
            SoundEvent::StopAll => self.kill_all(),
            // Kinds 21 and 22, as the game calls them (0x57a7f2, 0x57a8e3).
            SoundEvent::Pause => self.pause(),
            SoundEvent::Unpause => self.unpause(),
        }
    }

    fn play(&mut self, object: u32, name: &str) {
        match (0..self.modules.len()).find(|&m| self.modules[m].hot.event(name).is_some()) {
            Some(module) => self.play_from(module, object, name),
            None => self.report(format!("no event {name}")),
        }
    }

    /// Acts on the event of that name as one module defines it, whether
    /// or not an earlier module's definition would win in
    /// [`SoundEvent::Play`]. The module is an index into the modules
    /// given to [`Player::new`].
    ///
    /// *(code)* As the original's event dispatcher (0x573617) does, the
    /// event's kind picks what happens, with its track and three
    /// arguments. Each argument is the event's own number after the
    /// track when that is not 0, and otherwise the caller's: the object
    /// for the first, 0 for the others.
    pub fn play_from(&mut self, module: usize, object: u32, name: &str) {
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
            kind::PLAY => self.play_track(module, track, args),
            kind::FADE => self.fade_track(track, args),
            kind::CANCEL_NOTE => self.cancel_radio_notes(),
            kind::KILL_ALL => self.kill_all(),
            kind::PAUSE => self.pause(),
            kind::UNPAUSE => self.unpause(),
            kind::KILL_INSTANCE => self.kill_instance(track),
            kind::SET_AMBIENCE_MODE => self.set_ambience_mode(track),
            kind::SET_MUSIC_MODE => {
                self.set_music_mode(i64::from(track as i32));
                self.report(format!(
                    "event {name}: kind {kind} is not one this player plays (its music half)"
                ));
            }
            kind::TURN_ON_TV => self.turn_on_tv(module, track, args),
            kind::SEQUENCE_TRACK_HIT_LIST => self.sequence(track, args[1]),
            kind::KILL_OBJECT_SOUNDS => self.kill_object_sounds(),
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
    fn play_track(&mut self, module: usize, track: u32, args: [u32; 3]) {
        if self.owner(module, track).is_none() {
            self.report(format!("no track {track}"));
            return;
        }
        let alive = self.threads.iter().any(|t| t.track == track)
            || self.notes.iter().any(|n| n.track == track);
        if !alive {
            self.instances.retain(|&(t, _)| t != track);
        }
        if self.track_args(module, track) != Some(ARGS_ID_VOL_PAN) {
            self.start(args, module, track, false);
            return;
        }
        if args[0] == 0 {
            return;
        }
        self.record(track, args[0]);
        let placing = self.placing(track);
        let threads = self.start(args, module, track, false);
        self.place_later(placing, threads);
    }

    /// The objects of a track and its `Spl`, as the sum over them
    /// (0x57473b) reads them: before the start. `Spl` is read from the
    /// track's newest thread, through its `target`. When none runs it is
    /// [`SPL_NORMAL`]: the original's track keeps its last run's
    /// registers, and before its first run reads the default, but no
    /// script writes `Spl`.
    fn placing(&self, track: u32) -> Placing {
        let objects = self
            .instances
            .range((track, 0)..=(track, u32::MAX))
            .map(|&(_, object)| object)
            .collect();
        let spl = self
            .threads
            .iter()
            .chain(&self.pending)
            .filter(|t| t.track == track)
            .max_by_key(|t| t.id)
            .map_or(SPL_NORMAL, |t| self.read(t, i64::from(SPL)));
        Placing {
            objects,
            spl,
            threads: Vec::new(),
        }
    }

    /// Sets the started threads' `extvol` and `pan` at the next block,
    /// when the player has the game's [`Sources`].
    fn place_later(&mut self, placing: Placing, threads: Vec<u32>) {
        if !threads.is_empty() {
            self.placing.push(Placing { threads, ..placing });
        }
    }

    /// *(code)* Writes each placing's `extvol` (0x57d57a) then `pan`
    /// (0x57d5a0), as the start left them pointing at themselves: the
    /// comparison flags are the pan's. A fault leaves both as they were.
    fn place(&mut self, sources: &dyn Sources) {
        for placing in std::mem::take(&mut self.placing) {
            let (vol, pan) = match placement::combine(&placing.objects, placing.spl, sources) {
                Ok(placed) => placed,
                Err(fault) => {
                    self.report(fault);
                    continue;
                }
            };
            for thread in self
                .threads
                .iter_mut()
                .chain(&mut self.pending)
                .filter(|t| placing.threads.contains(&t.id))
            {
                thread.registers[usize::from(EXTVOL)] = vol;
                thread.registers[usize::from(PAN)] = pan;
                thread.kept = i64::from(pan);
            }
        }
    }

    /// Records an object instance, unless [`INSTANCES`] are recorded.
    fn record(&mut self, track: u32, object: u32) {
        if self.instances.len() >= INSTANCES && !self.instances.contains(&(track, object)) {
            self.report(format!("track {track}: more than {INSTANCES} instances"));
            return;
        }
        self.instances.insert((track, object));
    }

    /// The `args` field of the track a module (or the first other)
    /// defines.
    fn track_args(&self, module: usize, track: u32) -> Option<i64> {
        let owner = self.owner(module, track)?;
        let owner = &self.modules[owner];
        owner.number(&owner.hot.tracks[&track].args)
    }

    /// The module that defines a track: the one given, or else the first.
    fn owner(&self, module: usize, track: u32) -> Option<usize> {
        std::iter::once(module)
            .chain(0..self.modules.len())
            .find(|&m| {
                self.modules
                    .get(m)
                    .is_some_and(|m| m.hot.tracks.contains_key(&track))
            })
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
            if self.owner(0, track).is_none() {
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
    /// which here drops every thread's ducking request; every track
    /// that runs or plays is held except [`PAUSE_SPARES`]; and in music
    /// mode 0 the current ambience player is paused (see
    /// [`crate::ambience::Fresh::set_paused`]). A held track's threads
    /// do not run or fade, and its notes stay where they are, silent. A
    /// track started while paused plays: starting a track (0x57d960)
    /// clears its hold, and a held track whose script still runs is let
    /// go rather than started again. The original also stops its day and night
    /// loop (0x574978), which this player does not have.
    pub fn pause(&mut self) {
        self.paused = true;
        self.duck.clear();
        self.update_main_duckpri();
        let tracks: BTreeSet<u32> = self
            .threads
            .iter()
            .chain(&self.pending)
            .map(|t| t.track)
            .chain(self.notes.iter().map(|n| n.track))
            .filter(|t| !PAUSE_SPARES.contains(t))
            .collect();
        self.held.extend(tracks);
        if self.music_mode == 0
            && let Some(current) = self.ambience
        {
            self.with_score(current, |score, _| score.set_paused(true));
        }
    }

    /// Kind 22: *(code)* unpauses the sound (0x573c44, 0x574904): every
    /// track is let go (0x57cd2c) and the current ambience player, in
    /// any music mode, unpaused. Letting a track go (vtable +0x5c,
    /// 0x57dc63) also moves its wait deadline (+0x198, which `wait`
    /// sets at 0x57e453) to now, so every running thread's wait ends at
    /// the next update, whether or not it was held, and whether or not
    /// the sound was paused. (The original does this only while the
    /// sound is on, manager +0xc, which this player always is; it also
    /// restarts its day and night loop and its music, 0x575155, which
    /// this player does not have.) `main_duckpri` stays as the pause
    /// left it until a thread asks again.
    pub fn unpause(&mut self) {
        self.paused = false;
        self.held.clear();
        let now = self.frame;
        for thread in self.threads.iter_mut().chain(&mut self.pending) {
            thread.wake = now;
        }
        if let Some(current) = self.ambience {
            self.with_score(current, |score, _| score.set_paused(false));
        }
    }

    /// Whether the sound is paused (kind 21 since the last 22).
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Kind 20: *(code)* ends every track at once, forgets every object
    /// instance and stops every ambience player (the current one stays
    /// current). (The original also stops its music stations, which this
    /// player does not have.) Sequence groups are cleared too.
    fn kill_all(&mut self) {
        self.threads.clear();
        self.pending.clear();
        self.notes.clear();
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
    /// object still has fades out and ends. Here, where each object has
    /// its own thread, the object's threads of a track that others still
    /// have fade out and end too; the original keeps the one track
    /// object playing for the others.
    fn kill_instance(&mut self, object: u32) {
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
            self.instances.remove(&(track, object));
            let shared = self.instances.iter().any(|&(t, _)| t == track);
            self.fade_and_kill(track, shared.then_some(object));
        }
    }

    /// Kind 105 (`kill_all_objectsounds`): *(code)* lets go of every
    /// instance whose object is above 0 (as a signed number), fading out
    /// and ending its track.
    fn kill_object_sounds(&mut self) {
        let (killed, kept): (Vec<_>, Vec<_>) = self
            .instances
            .iter()
            .partition(|&&(_, object)| (object as i32) > 0);
        self.instances = kept.into_iter().collect();
        for (track, _) in killed {
            self.fade_and_kill(track, None);
        }
    }

    /// The original's fade-out-and-kill (0x57dca3) on a track's threads
    /// (only `object`'s, when given): a thread with a note playing fades
    /// its `extvol` to 0 at speed 100 and then ends; one without ends
    /// now. The track's notes whose thread has ended stop now (the
    /// original would fade them; here they have no registers).
    fn fade_and_kill(&mut self, track: u32, object: Option<u32>) {
        let ids: Vec<u32> = self
            .threads
            .iter()
            .filter(|t| t.track == track && object.is_none_or(|o| t.object == o))
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
        self.pending
            .retain(|t| !(t.track == track && object.is_none_or(|o| t.object == o)));
        // Notes that outlived their thread have no registers to fade, and
        // end at once.
        let running: BTreeSet<u32> = self.threads.iter().map(|t| t.id).collect();
        self.notes.retain(|n| {
            running.contains(&n.thread)
                || !(n.track == track && object.is_none_or(|o| n.object == o))
        });
        self.forget_ended();
    }

    /// Kind 30 (`kTurnOnTv`, which radios use too): *(code)* lets go of
    /// whatever the object (the first argument) had playing, records it
    /// as an instance of the track, and starts the track for it. A track
    /// started anew is left held (the start ends by holding it, 0x57dbd6)
    /// and let go only when the sound is not paused (0x573d71), so a
    /// television turned on during a pause is silent until the unpause.
    fn turn_on_tv(&mut self, module: usize, track: u32, args: [u32; 3]) {
        self.kill_instance(args[0]);
        if self.owner(module, track).is_none() {
            self.report(format!("no track {track}"));
            return;
        }
        self.record(track, args[0]);
        let placing = self.placing(track);
        let resumes = self.held.contains(&track)
            && self
                .threads
                .iter()
                .chain(&self.pending)
                .any(|t| t.track == track);
        let threads = self.start(args, module, track, false);
        if !threads.is_empty() && self.paused && !resumes {
            self.held.insert(track);
        }
        self.place_later(placing, threads);
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
        let Some(owner) = self.owner(0, track) else {
            return;
        };
        let Some(list) = self.hit_lists.get(&hit_list).cloned() else {
            self.report(format!("no hit list {hit_list}"));
            return;
        };
        let patch = match list.len() {
            0 => 0,
            n => list[self.random(n as u32) as usize],
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

    /// Starts a track's routine for an object. The module given is tried
    /// first, then every other. A deferred thread first runs in the next
    /// block. The threads started or let go: none when it could not
    /// start.
    fn start(&mut self, args: [u32; 3], module: usize, track: u32, defer: bool) -> Vec<u32> {
        let Some(module) = self.owner(module, track) else {
            self.report(format!("no track {track}"));
            return Vec::new();
        };
        // *(code)* Starting a track the pause holds while its script runs
        // does not start it again (0x57d9f1 to 0x57da12): it lets it go
        // (vtable +0x5c), so the script goes on from where it was, its
        // wait ended, with the new arguments. No thread is made, so the
        // thread limit does not apply.
        if self.held.remove(&track) {
            let resumed = self.resume_held(args, track);
            if !resumed.is_empty() {
                return resumed;
            }
        }
        if self.threads.len() + self.pending.len() >= THREADS {
            self.report(format!("track {track}: more than {THREADS} threads"));
            return Vec::new();
        }
        let Some(thread) = self.thread(args, module, track) else {
            return Vec::new();
        };
        let id = thread.id;
        if defer {
            self.pending.push(thread);
        } else {
            self.threads.push(thread);
        }
        vec![id]
    }

    /// Lets a held track's running threads go on, as starting it does
    /// (see [`Player::start`]). Each one's `target` points back at itself
    /// (0x57d9c3), its argument registers take `args` (see
    /// [`Player::thread`]) and its wait ends. The threads let go: none
    /// when none of the track runs, so it is started anew. (A held thread
    /// does not run, so none of these is the thread running `play_trk`.)
    fn resume_held(&mut self, args: [u32; 3], track: u32) -> Vec<u32> {
        let now = self.frame;
        let mut resumed = Vec::new();
        for thread in self
            .threads
            .iter_mut()
            .chain(&mut self.pending)
            .filter(|t| t.track == track)
        {
            set_arguments(thread, args);
            thread.wake = now;
            resumed.push(thread.id);
        }
        resumed
    }

    /// A new thread of a track the module defines, with its starting
    /// registers, or `None` (reported) when the track has no entry point.
    fn thread(&mut self, args: [u32; 3], module: usize, track: u32) -> Option<Thread> {
        let object = args[0];
        let owner = &self.modules[module];
        let Some(pc) = owner.entry(track).and_then(|entry| owner.address(entry)) else {
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
        let id = self.id();
        let mut thread = Thread {
            id,
            object,
            module,
            track,
            pc,
            registers,
            stack: Vec::new(),
            loop_point: pc,
            kept: 0,
            hit_list: Vec::new(),
            last_note: 0,
            wake: 0,
            group,
            target: Target::Own,
            kill_after_fade: false,
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

    /// Gives every note of a running thread that thread's levels. (The
    /// original sets them as registers change; here once per block, after
    /// the threads run.) A note whose thread has ended keeps its last.
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
        let alive: Vec<u32> = self
            .threads
            .iter()
            .chain(&self.pending)
            .map(|t| t.id)
            .collect();
        self.duck.retain(|id, _| alive.contains(id));
        self.update_main_duckpri();
    }

    fn update_main_duckpri(&mut self) {
        let largest = self.duck.values().copied().fold(0, i32::max);
        self.globals[usize::from(MAIN_DUCKPRI - GLOBALS)] = largest;
    }

    fn run(&mut self, sources: &dyn Sources) {
        self.place(sources);
        let mut budget = BLOCK_STEPS;
        let mut index = 0;
        while index < self.threads.len() {
            // A held track's threads wait, without fading (0x57d7f3).
            if self.held.contains(&self.threads[index].track) {
                index += 1;
                continue;
            }
            if budget == 0 {
                if !self.over_budget {
                    self.over_budget = true;
                    self.report(format!(
                        "threads ran {BLOCK_STEPS} instructions in one block; the rest wait"
                    ));
                }
                // The threads not reached still fade, and run first next
                // block, so busy threads cannot starve them.
                for at in index..self.threads.len() {
                    if self.held.contains(&self.threads[at].track) {
                        continue;
                    }
                    let mut thread = self.threads[at].clone();
                    self.fade(&mut thread);
                    self.threads[at] = thread;
                }
                self.threads.rotate_left(index);
                break;
            }
            let mut thread = self.threads[index].clone();
            let alive = self.step(&mut thread, sources, &mut budget);
            // A thread may have stopped others (kill_trk) or started some
            // (play_trk); find this one again by ID.
            match self.threads.iter().position(|t| t.id == thread.id) {
                Some(at) if alive => {
                    self.threads[at] = thread;
                    index = at + 1;
                }
                Some(at) => {
                    // Its last register writes still reach its notes, as
                    // they would have when written.
                    let levels = self.levels(&thread);
                    for note in self.notes.iter_mut().filter(|n| n.thread == thread.id) {
                        note.gains = levels.gains;
                        note.step = step(levels.rate);
                    }
                    self.threads.remove(at);
                    // Its ducking request goes now, before the threads
                    // after it run.
                    self.forget_ended();
                    index = at;
                }
                None => {}
            }
        }
        // Threads started this block run from the next one, so a track
        // that starts itself cannot keep this loop going.
        self.threads.append(&mut self.pending);
        self.forget_ended();
        self.apply_levels();
    }

    /// Runs one thread until it waits, ends, or the block's instructions
    /// run out (`budget`). False when it ended.
    fn step(&mut self, thread: &mut Thread, sources: &dyn Sources, budget: &mut usize) -> bool {
        self.fade(thread);
        // *(code)* A fade that was to end the track has ended: the track
        // ends at once, with its notes.
        if thread.kill_after_fade && thread.registers[usize::from(FADE_ON)] == 0 {
            let id = thread.id;
            self.notes.retain(|n| n.thread != id);
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
            let module = &self.modules[thread.module];
            let instruction = match module.program.decode(thread.pc, &|id| module.is_target(id)) {
                Ok(instruction) => instruction,
                Err(error) => return self.fault(thread, &error.to_string()),
            };
            let next = thread.pc + instruction.length;
            match self.execute(thread, &instruction, next, sources) {
                Ok(Flow::Next) => thread.pc = next,
                Ok(Flow::Jump(pc)) => thread.pc = pc,
                Ok(Flow::Block) => return true,
                Ok(Flow::Yield) => {
                    thread.pc = next;
                    return true;
                }
                Ok(Flow::JumpYield(pc)) => {
                    thread.pc = pc;
                    return true;
                }
                Ok(Flow::End) => match thread.stack.pop() {
                    Some(pc) => thread.pc = pc,
                    None => return false,
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

    fn fault(&mut self, thread: &Thread, message: &str) -> bool {
        let name = &self.modules[thread.module].name;
        self.report(format!(
            "track {} ({name}) at {:#x}: {message}",
            thread.track, thread.pc
        ));
        false
    }

    /// The thread a register access other than to `target` (53) goes
    /// to, or `None` for `thread` itself. As in the original, an access
    /// goes to the thread `target` names, then on to the one that
    /// thread's `target` names, and so on. A link to a thread that has
    /// ended (the original keeps a pointer to it; what it then does is
    /// not known) stops the chain there, as does a thread already passed
    /// (the original would recurse for ever).
    fn resolve(&self, thread: &Thread) -> Option<u32> {
        let mut at = None;
        let mut target = thread.target;
        let mut passed = vec![thread.id];
        while let Target::Thread(id) = target
            && !passed.contains(&id)
            && let Some(next) = self
                .threads
                .iter()
                .chain(&self.pending)
                .find(|t| t.id == id)
        {
            passed.push(id);
            at = Some(id);
            target = next.target;
        }
        at
    }

    /// Reads a register. Registers below 100 other than `target` (53)
    /// come from the thread [`Player::resolve`] picks; globals are shared.
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
        let source = self
            .resolve(thread)
            .and_then(|id| {
                self.threads
                    .iter()
                    .chain(&self.pending)
                    .find(|t| t.id == id)
            })
            .unwrap_or(thread);
        source.registers[usize::from(r)]
    }

    /// Writes a register, with the original's side effects. Every write
    /// other than to `target` (53) goes to the thread [`Player::resolve`]
    /// picks, globals included, and sets that thread's comparison flags
    /// from the value. `target` picks the newest running thread of the
    /// track it names; 0, or a track with none running, is this thread
    /// (the original's lookup returns null, which it takes as itself).
    /// `main_duckpri` (123) records the written thread's request and
    /// holds the largest.
    fn write(&mut self, thread: &mut Thread, register: i64, value: i32) {
        let Ok(r) = u8::try_from(register) else {
            return;
        };
        if r == TARGET {
            thread.registers[usize::from(r)] = value;
            thread.kept = i64::from(value);
            thread.target = if value == 0 {
                Target::Own
            } else {
                self.threads
                    .iter()
                    .chain(&self.pending)
                    .filter(|t| t.track == value as u32)
                    .map(|t| t.id)
                    .max()
                    .map_or(Target::Own, Target::Thread)
            };
            return;
        }
        let written = match self.resolve(thread) {
            Some(id) => match self
                .threads
                .iter_mut()
                .chain(&mut self.pending)
                .find(|t| t.id == id)
            {
                Some(other) => other,
                None => thread,
            },
            None => thread,
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

    fn target(&self, thread: &Thread, id: u32) -> Result<u32, String> {
        self.modules[thread.module]
            .address(id)
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
                    // so caching them would grow without bound.
                    self.report(format!("no patch {patch}"));
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
            object: thread.object,
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
                Ok(Flow::Jump(self.target(thread, arg(0) as u32)?))
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
                let id = self.read(thread, arg(0)) as u32;
                self.notes.retain(|n| n.id != id);
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
                thread.stack.push(next);
                Flow::Jump(self.target(thread, arg(0) as u32)?)
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
                Flow::Jump(self.target(thread, id)?)
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
                thread.loop_point = next;
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
                let track = self.read(thread, arg(0)) as u32;
                self.start([thread.object, 0, 0], thread.module, track, true);
                Flow::Next
            }
            "kill_trk" => {
                let track = self.read(thread, arg(0)) as u32;
                let (object, me) = (thread.object, thread.id);
                let killed: Vec<u32> = self
                    .threads
                    .iter()
                    .filter(|t| t.object == object && t.track == track && t.id != me)
                    .map(|t| t.id)
                    .collect();
                self.threads.retain(|t| !killed.contains(&t.id));
                self.pending
                    .retain(|t| !(t.object == object && t.track == track));
                self.notes.retain(|n| !killed.contains(&n.thread));
                self.forget_ended();
                Flow::Next
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
                // and logs a refused field, going on to the next
                // instruction either way.
                let (source, field) = (self.read(thread, arg(1)), self.read(thread, arg(2)));
                match sources.field(source, field) {
                    Some(value) => self.write(thread, arg(0), value),
                    None => {
                        self.fault(thread, "Illegal source parameter request");
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
                object: 0,
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
