//! The HIT virtual machine: runs sound events and mixes their samples.
//!
//! A [`SoundEvent::Play`] finds its HOT event and the event's track, and
//! starts a *thread*: the track's HIT routine, run for one object. A
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
//!   register (50) holds its track's patch. When the track's arguments
//!   are `kArgsIdVolPan` (2, nearly every track), `arg1` holds the object
//!   that started it: routines pass it to `getsrcdatafield`.
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
//! **Unverified** against the original at run time: all of the above.
//! Left out: priorities, 3D position (a 3D track is centred), the track
//! group's own volume beyond the host's [`Player::set_group_volume`], and
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
//! sequence groups exist at once; calls nest at most [`CALL_DEPTH`] deep;
//! a thread started by `play_trk` first runs in the next block; a patch
//! that does not exist is not cached. How the original picks among
//! duplicate events, patches and hit lists across modules is unknown;
//! here the module whose name sorts first wins.

use crate::files::SoundFiles;
use crate::hit::{Instruction, Operand};
use crate::levels::{self, Inputs, Levels};
use crate::module::Module;
use crate::{Pcm, SoundEvent};
use std::collections::BTreeMap;

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
/// A new thread's registers that do not start at 0, as the original's
/// default register bank holds them: `priority`, `vol`, `extvol`, `pan`,
/// `pitch`, `MinDistance`, `MaxDistance` and `fade_dest`.
pub const DEFAULTS: [(u8, i32); 8] = [
    (17, 32),
    (18, 1024),
    (EXTVOL, 1024),
    (20, 512),
    (21, 3600),
    (28, 100),
    (29, 10000),
    (FADE_DEST, 1),
];
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

/// What scripts may ask about the object that started them
/// (`getsrcdatafield`). The simulation would answer through a read-only
/// view; the field numbers' meanings are **unverified**.
pub trait Sources {
    fn field(&self, object: u32, source: i32, field: i32) -> i32;
}

/// Answers 0 to everything.
pub struct NoSources;

impl Sources for NoSources {
    fn field(&self, _: u32, _: i32, _: i32) -> i32 {
        0
    }
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Own,
    Thread(u32),
}

#[derive(Debug, Clone)]
struct Note {
    id: u32,
    object: u32,
    thread: u32,
    patch: u32,
    /// Position in source frames, 16.16 fixed point.
    position: u64,
    step: u64,
    /// Left and right gains, 16.16 fixed point.
    gains: [i32; 2],
}

/// Source frames per output frame, 16.16 fixed point, at a playback rate.
fn step(rate: u32) -> u64 {
    (u64::from(rate) << 16) / u64::from(RATE)
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
    pub fn set_group_volume(&mut self, group: i64, volume: i32) {
        self.group_volumes.insert(group, volume.clamp(0, 1024));
    }

    /// Sets the master volume, 0 to 1024.
    pub fn set_master_volume(&mut self, volume: i32) {
        self.master = volume.clamp(0, 1024);
    }

    /// Frames mixed so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Whether nothing is running or playing.
    pub fn is_idle(&self) -> bool {
        self.threads.is_empty() && self.pending.is_empty() && self.notes.is_empty()
    }

    /// Acts on one event from the simulation.
    pub fn handle(&mut self, event: &SoundEvent) {
        match event {
            SoundEvent::Play { object, name } => self.play(*object, name),
            SoundEvent::StopObject { object } => {
                self.threads.retain(|t| t.object != *object);
                self.pending.retain(|t| t.object != *object);
                self.notes.retain(|n| n.object != *object);
                self.forget_ended();
            }
            SoundEvent::StopAll => {
                self.threads.clear();
                self.pending.clear();
                self.notes.clear();
                self.seqgroups.clear();
                self.forget_ended();
            }
        }
    }

    fn play(&mut self, object: u32, name: &str) {
        match (0..self.modules.len()).find(|&m| self.modules[m].hot.event(name).is_some()) {
            Some(module) => self.play_from(module, object, name),
            None => self.report(format!("no event {name}")),
        }
    }

    /// Plays the event of that name as one module defines it, whether or
    /// not an earlier module's definition would win in
    /// [`SoundEvent::Play`]. The module is an index into the modules
    /// given to [`Player::new`].
    pub fn play_from(&mut self, module: usize, object: u32, name: &str) {
        let found = self.modules.get(module).and_then(|owner| {
            let event = owner.hot.event(name)?;
            let kind = owner.number(&event.kind);
            let track = event.track.as_ref().and_then(|t| owner.number(t));
            Some((module, kind, track))
        });
        match found {
            Some((module, Some(1), Some(track))) => {
                self.start(object, module, track as u32, false);
            }
            Some((_, kind, _)) => self.report(format!(
                "event {name}: kind {kind:?} is not one this player plays"
            )),
            None => self.report(format!("no event {name}")),
        }
    }

    /// Starts a track's routine for an object. The module given is tried
    /// first, then every other. A deferred thread first runs in the next
    /// block.
    fn start(&mut self, object: u32, module: usize, track: u32, defer: bool) {
        if self.threads.len() + self.pending.len() >= THREADS {
            self.report(format!("track {track}: more than {THREADS} threads"));
            return;
        }
        let order = std::iter::once(module).chain(0..self.modules.len());
        let Some(module) = order
            .into_iter()
            .find(|&m| self.modules[m].hot.tracks.contains_key(&track))
        else {
            self.report(format!("no track {track}"));
            return;
        };
        let owner = &self.modules[module];
        let Some(pc) = owner.entry(track).and_then(|entry| owner.address(entry)) else {
            self.report(format!("track {track}: entry point not found"));
            return;
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
        if owner.number(&fields.args) == Some(ARGS_ID_VOL_PAN) {
            registers[1] = object as i32;
        }
        let id = self.id();
        let queue = if defer {
            &mut self.pending
        } else {
            &mut self.threads
        };
        queue.push(Thread {
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
        });
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

    /// xorshift64*: the player's own random numbers, so a seed replays.
    fn random(&mut self, below: u32) -> u32 {
        self.random ^= self.random >> 12;
        self.random ^= self.random << 25;
        self.random ^= self.random >> 27;
        let value = self.random.wrapping_mul(0x2545_f491_4f6c_dd1d);
        ((value >> 32) % u64::from(below.max(1))) as u32
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
        let Some(samples) = frames.checked_mul(2) else {
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

    /// Renders until nothing runs or plays, at most `limit` frames. The
    /// output ends with the block in which the last note ended.
    pub fn render_until_idle(&mut self, limit: usize, sources: &dyn Sources) -> Vec<i16> {
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
        }
    }

    /// Mixes `frames` stereo frames onto `out`. A mono sample plays on
    /// both channels; a stereo one keeps its own.
    fn mix(&mut self, frames: usize, out: &mut Vec<i16>) {
        let start = out.len();
        let mut sums = vec![0i64; 2 * frames];
        let cache = &self.cache;
        self.notes.retain_mut(|note| {
            let Some(Some(pcm)) = cache.get(&note.patch) else {
                return false;
            };
            let channels = usize::from(pcm.channels).max(1);
            let length = pcm.frames() as u64;
            let sample = |frame: usize, channel: usize| -> i64 {
                i64::from(pcm.samples[frame * channels + channel.min(channels - 1)])
            };
            for slot in sums.as_chunks_mut::<2>().0 {
                let index = note.position >> 16;
                if index >= length {
                    return false;
                }
                let at = index as usize;
                let after = if index + 1 < length { at + 1 } else { at };
                // A difference of two samples times a 16-bit fraction
                // needs more than 32 bits.
                let fraction = i64::from((note.position & 0xffff) as u32);
                for (channel, sum) in slot.iter_mut().enumerate() {
                    let (first, second) = (sample(at, channel), sample(after, channel));
                    let value = first + (((second - first) * fraction) >> 16);
                    *sum += (value * i64::from(note.gains[channel])) >> 16;
                }
                note.position += note.step;
            }
            note.position >> 16 < length
        });
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
        let mut budget = BLOCK_STEPS;
        let mut index = 0;
        while index < self.threads.len() {
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

    fn note_on(&mut self, thread: &Thread) -> u32 {
        let patch = thread.registers[usize::from(PATCH)] as u32;
        if !self.cache.contains_key(&patch) {
            let pcm = match self.patches.get(&patch).map(|(_, path)| path.clone()) {
                Some(path) => {
                    let pcm = self.samples.load(&path);
                    if pcm.is_none() {
                        self.report(format!("patch {patch}: no sample at {path}"));
                    }
                    pcm
                }
                None => {
                    // Not cached: any number may name a missing patch,
                    // so caching them would grow without bound.
                    self.report(format!("no patch {patch}"));
                    return 0;
                }
            };
            self.cache.insert(patch, pcm);
        }
        if !matches!(self.cache.get(&patch), Some(Some(_))) {
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
            object: thread.object,
            thread: thread.id,
            patch,
            position: 0,
            step: step(levels.rate),
            gains: levels.gains,
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
                self.start(thread.object, thread.module, track, true);
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
                let (source, field) = (self.read(thread, arg(1)), self.read(thread, arg(2)));
                let value = sources.field(thread.object, source, field);
                self.write(thread, arg(0), value);
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
