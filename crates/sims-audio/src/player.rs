//! The HIT virtual machine: runs sound events and mixes their samples.
//!
//! A [`SoundEvent::Play`] finds its HOT event and the event's track, and
//! starts a *thread*: the track's HIT routine, run for one object. A
//! thread has its own registers, a call stack and a loop point; it starts
//! notes (samples of a patch) and waits for them, for a time, or for a
//! register condition. The player runs every thread that is not waiting
//! at the start of each block of [`BLOCK`] frames, then mixes the playing
//! notes into mono 16-bit PCM at [`RATE`].
//!
//! Where the instructions' meanings come from: the register names are the
//! HSM files' own; the rest follows public notes (FreeSO's HIT player, the
//! Niotso wiki) and what the install's programs need:
//!
//! - Registers below 100 belong to a thread; 100 and above are globals
//!   shared by every thread (`SimSpeed` is 100). A new thread's `patch`
//!   register (50) holds its track's patch. When the track's arguments
//!   are `kArgsIdVolPan` (2, nearly every track), `arg1` holds the object
//!   that started it: routines pass it to `getsrcdatafield`. Every other
//!   register starts at 0.
//! - `note_on` starts the patch in register 50 and writes the note's ID
//!   to its operand; `wait_samp` waits until the thread's last note ends.
//! - `end` returns from a `call`, or ends the thread when nothing called
//!   it. (No program in the install has a `return`, against 2,504 calls.)
//! - `cmp a, b` keeps `a - b`; `test r` keeps `r`; the `if` branches test
//!   what was kept. `set_loop` marks the next instruction; `loop` goes
//!   back to it, or to the routine's start when nothing was marked (the
//!   looping generic routine is `note_on v1; wait_samp; loop`). The
//!   thread then waits for the next block, so a loop that starts no note
//!   cannot spin.
//! - `smart_setlist` picks a hit list; `smart_choose` writes one of its
//!   patches, chosen uniformly with the player's own random numbers.
//! - `wait r` waits `r` milliseconds; the `wait` comparisons wait until
//!   their condition holds.
//!
//! **Unverified** against the original: all of the above, and what this
//! player leaves out. Volume, pan, pitch, ducking, priorities and 3D
//! position are not applied: every note plays at full volume, centred,
//! at its own speed. The `seqgroup_*` instructions and `test1` to `test4`
//! do nothing; `setll`, `setlt` and `settl` copy like `set`. How the
//! original picks among duplicate events, patches and hit lists across
//! modules is unknown; here the module whose name sorts first wins.

use crate::files::SoundFiles;
use crate::hit::{Instruction, Operand};
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
/// The first global register.
pub const GLOBALS: u8 = 100;
/// The register a new note's patch is read from.
pub const PATCH: u8 = 50;

/// The track argument kind `kArgsIdVolPan`: `arg1` is the object.
pub const ARGS_ID_VOL_PAN: i64 = 2;

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
    notes: Vec<Note>,
    frame: u64,
    /// The frame at which the threads last ran.
    ran_at: Option<u64>,
    next_id: u32,
    random: u64,
    /// What went wrong while running, in order: unknown events, missing
    /// samples, threads ended for a bad instruction.
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

    /// Frames mixed so far.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Whether nothing is running or playing.
    pub fn is_idle(&self) -> bool {
        self.threads.is_empty() && self.notes.is_empty()
    }

    /// Acts on one event from the simulation.
    pub fn handle(&mut self, event: &SoundEvent) {
        match event {
            SoundEvent::Play { object, name } => self.play(*object, name),
            SoundEvent::StopObject { object } => {
                self.threads.retain(|t| t.object != *object);
                self.notes.retain(|n| n.object != *object);
            }
            SoundEvent::StopAll => {
                self.threads.clear();
                self.notes.clear();
            }
        }
    }

    fn play(&mut self, object: u32, name: &str) {
        let found = self.modules.iter().enumerate().find_map(|(index, module)| {
            let event = module.hot.event(name)?;
            let kind = module.number(&event.kind);
            let track = event.track.as_ref().and_then(|t| module.number(t));
            Some((index, kind, track))
        });
        match found {
            Some((module, Some(1), Some(track))) => self.start(object, module, track as u32),
            Some((_, kind, _)) => self.faults.push(format!(
                "event {name}: kind {kind:?} is not one this player plays"
            )),
            None => self.faults.push(format!("no event {name}")),
        }
    }

    /// Starts a track's routine for an object. The module given is tried
    /// first, then every other.
    fn start(&mut self, object: u32, module: usize, track: u32) {
        let order = std::iter::once(module).chain(0..self.modules.len());
        let Some(module) = order
            .into_iter()
            .find(|&m| self.modules[m].hot.tracks.contains_key(&track))
        else {
            self.faults.push(format!("no track {track}"));
            return;
        };
        let owner = &self.modules[module];
        let Some(pc) = owner.entry(track).and_then(|entry| owner.address(entry)) else {
            self.faults
                .push(format!("track {track}: entry point not found"));
            return;
        };
        let fields = &owner.hot.tracks[&track];
        let patch = owner.number(&fields.patch).unwrap_or(0);
        let mut registers = [0; GLOBALS as usize];
        registers[usize::from(PATCH)] = patch as i32;
        if owner.number(&fields.args) == Some(ARGS_ID_VOL_PAN) {
            registers[1] = object as i32;
        }
        let id = self.id();
        self.threads.push(Thread {
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
        });
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
        let mut out = Vec::with_capacity(frames);
        while out.len() < frames {
            self.run_at_block_start(sources);
            let until_block = BLOCK - (self.frame % BLOCK as u64) as usize;
            let count = until_block.min(frames - out.len());
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

    fn mix(&mut self, frames: usize, out: &mut Vec<i16>) {
        let start = out.len();
        let mut sums = vec![0i32; frames];
        let cache = &self.cache;
        self.notes.retain_mut(|note| {
            let Some(Some(pcm)) = cache.get(&note.patch) else {
                return false;
            };
            let channels = usize::from(pcm.channels);
            let length = pcm.frames() as u64;
            let sample = |frame: usize| -> i32 {
                let at = frame * channels;
                let sum: i32 = pcm.samples[at..at + channels]
                    .iter()
                    .map(|&s| i32::from(s))
                    .sum();
                sum / channels as i32
            };
            for slot in &mut sums {
                let index = note.position >> 16;
                if index >= length {
                    return false;
                }
                let first = sample(index as usize);
                let second = if index + 1 < length {
                    sample(index as usize + 1)
                } else {
                    first
                };
                let fraction = (note.position & 0xffff) as i32;
                *slot += first + (((second - first) * fraction) >> 16);
                note.position += note.step;
            }
            note.position >> 16 < length
        });
        out.extend(sums.iter().map(|&s| s.clamp(-32768, 32767) as i16));
        debug_assert_eq!(out.len(), start + frames);
    }

    fn run(&mut self, sources: &dyn Sources) {
        let mut index = 0;
        while index < self.threads.len() {
            let mut thread = self.threads[index].clone();
            let alive = self.step(&mut thread, sources);
            // A thread may have stopped others (kill_trk) or started some
            // (play_trk); find this one again by ID.
            match self.threads.iter().position(|t| t.id == thread.id) {
                Some(at) if alive => {
                    self.threads[at] = thread;
                    index = at + 1;
                }
                Some(at) => {
                    self.threads.remove(at);
                    index = at;
                }
                None => {}
            }
        }
    }

    /// Runs one thread until it waits or ends. False when it ended.
    fn step(&mut self, thread: &mut Thread, sources: &dyn Sources) -> bool {
        if thread.wake > self.frame {
            return true;
        }
        for _ in 0..STEPS {
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
                Err(message) => return self.fault(thread, &message),
            }
        }
        self.fault(thread, "ran too long without waiting")
    }

    fn fault(&mut self, thread: &Thread, message: &str) -> bool {
        let name = &self.modules[thread.module].name;
        self.faults.push(format!(
            "track {} ({name}) at {:#x}: {message}",
            thread.track, thread.pc
        ));
        false
    }

    fn read(&self, thread: &Thread, register: i64) -> i32 {
        match u8::try_from(register) {
            Ok(r) if r < GLOBALS => thread.registers[usize::from(r)],
            Ok(r) => self.globals[usize::from(r - GLOBALS)],
            Err(_) => 0,
        }
    }

    fn write(&mut self, thread: &mut Thread, register: i64, value: i32) {
        match u8::try_from(register) {
            Ok(r) if r < GLOBALS => thread.registers[usize::from(r)] = value,
            Ok(r) => self.globals[usize::from(r - GLOBALS)] = value,
            Err(_) => {}
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
                        self.faults
                            .push(format!("patch {patch}: no sample at {path}"));
                    }
                    pcm
                }
                None => {
                    self.faults.push(format!("no patch {patch}"));
                    None
                }
            };
            self.cache.insert(patch, pcm);
        }
        let Some(Some(pcm)) = self.cache.get(&patch) else {
            return 0;
        };
        let step = (u64::from(pcm.rate) << 16) / u64::from(RATE);
        let id = self.id();
        self.notes.push(Note {
            id,
            object: thread.object,
            thread: thread.id,
            patch,
            position: 0,
            step,
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
            "nop" | "test1" | "test2" | "test3" | "test4" | "seqgroup_kill" | "seqgroup_wait"
            | "seqgroup_return" => Flow::Next,
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
                self.start(thread.object, thread.module, track);
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
                self.notes.retain(|n| !killed.contains(&n.thread));
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
