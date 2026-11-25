//! A deterministic fuzzer for the state that outlives a tick: the World's
//! snapshots, the stored neighborhood and its save inputs, and the REDB
//! store that keeps them.
//!
//! The design comes from the legacy exploration fuzzer
//! (`legacy-final:crates/sims-runtime/src/exploration.rs`):
//!
//! - A seeded SplitMix64 stream chooses each command by weight, with
//!   bounded draws by rejection sampling, so a seed always gives the same
//!   run on every platform.
//! - The whole command list is drawn, and so journaled, before any of it
//!   runs, and a panic in a run is caught and reported with its seed.
//! - World runs compare a reference run with branches taken at several
//!   command boundaries: a warm fork (a clone, which shares copy-on-write
//!   storage with the run it came from), a cold restore from the snapshot
//!   encoding, and a cold restore through a REDB [`Snapshots`] file. Each
//!   records a trace after every command, and a failure names the first
//!   command whose trace differs. The run the forks came from must still
//!   match a run that never forked, so a fork that writes through to its
//!   parent is caught.
//! - Store runs check a REDB [`NeighborhoodStore`] against a model after
//!   every operation, reopening the file (a cold restore) at random
//!   boundaries.
//! - A failing command list is shrunk by delta debugging before it is
//!   reported.
//!
//! Everything is synthetic: made-up families, names and GUIDs. No wall
//! clock reaches a trace.

use crate::common::{TempDb, TempDir};
use sims_core::{CrtRng, GameRng};
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbor, Relationship};
use sims_formats::saved_objects::{Frame, Slot};
use sims_sim::characters::Characters;
use sims_sim::neighborhood_save::SaveInputs;
use sims_sim::{IndicatorQueues, NeighborhoodStore, Object, Snapshots, StoredNeighborhood, World};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// SplitMix64 (Steele, Lea and Flood), as the legacy fuzzer used.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`, by rejection so no value is favoured.
    fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        let zone = u64::MAX - u64::MAX % n;
        loop {
            let x = self.next();
            if x < zone {
                return x % n;
            }
        }
    }

    fn range(&mut self, low: i64, high: i64) -> i64 {
        low + self.below((high - low + 1) as u64) as i64
    }

    fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.below(denominator) < numerator
    }

    /// An index into `weights`, chosen with probability proportional to
    /// its weight.
    fn weighted(&mut self, weights: &[u64]) -> usize {
        let mut x = self.below(weights.iter().sum());
        for (i, &w) in weights.iter().enumerate() {
            if x < w {
                return i;
            }
            x -= w;
        }
        unreachable!()
    }

    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let n = self.below(max as u64 + 1) as usize;
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Runs `check` on `commands`; on failure, shrinks the list by delta
/// debugging (removing ever smaller chunks while the check still fails)
/// and panics with the seed, the shrunk list and its failure.
fn check_or_shrink<C: Clone + Debug>(
    what: &str,
    seed: u64,
    commands: Vec<C>,
    check: impl Fn(&[C]) -> Result<(), String>,
) {
    let run = |commands: &[C]| match catch_unwind(AssertUnwindSafe(|| check(commands))) {
        Ok(result) => result,
        Err(panic) => Err(format!(
            "panicked: {}",
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("?")
        )),
    };
    let Err(first) = run(&commands) else {
        return;
    };
    let mut failing = commands;
    let mut failure = first;
    let mut chunk = failing.len().div_ceil(2).max(1);
    loop {
        let mut shrunk = false;
        let mut start = 0;
        while start < failing.len() {
            let end = (start + chunk).min(failing.len());
            let candidate: Vec<C> = failing[..start]
                .iter()
                .chain(&failing[end..])
                .cloned()
                .collect();
            if let Err(why) = run(&candidate) {
                failing = candidate;
                failure = why;
                shrunk = true;
            } else {
                start = end;
            }
        }
        if chunk == 1 && !shrunk {
            break;
        }
        if !shrunk {
            chunk = chunk.div_ceil(2);
        }
    }
    panic!("{what} seed {seed:#x} fails: {failure}\nshrunk to {failing:#?}");
}

// ----- World snapshots -----

/// One change to a World, or one query whose answer goes into the trace.
#[derive(Debug, Clone)]
enum WorldCommand {
    DrawLot(u8),
    DrawGlobal(u8),
    DrawCrt(u8),
    SetGlobal(usize, i16),
    Tick(u16),
    AddObject(u16, u32),
    EditObject {
        pick: usize,
        word: usize,
        value: i16,
    },
    PushFrame {
        pick: usize,
        tree: u16,
    },
    RemoveObject(usize),
    InsertType(u16, u32),
    LookUpType(u32),
    SetFlags {
        pick: usize,
        mask: u16,
        on: bool,
    },
    SetCountdown {
        pick: usize,
        countdown: i16,
    },
    SetCursor(i16),
    Queue {
        pick: usize,
        date: Option<i16>,
    },
}

fn object(id: u16, guid: u32) -> Object {
    Object {
        id,
        guid,
        footprint: [16, 16, 32, 32],
        position: [24, 24, 1],
        unknown_short: 1,
        attributes: vec![3, -4],
        temps: vec![0; 8],
        data: (0..72).collect(),
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![Slot {
            unknown: 0,
            object: 0,
        }],
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: Some(0x0002_0005),
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

fn world_commands(rng: &mut Rng, count: usize) -> Vec<WorldCommand> {
    (0..count)
        .map(
            |_| match rng.weighted(&[3, 2, 2, 3, 2, 4, 5, 2, 2, 3, 3, 1, 1, 1, 2]) {
                0 => WorldCommand::DrawLot(rng.below(4) as u8 + 1),
                1 => WorldCommand::DrawGlobal(rng.below(4) as u8 + 1),
                2 => WorldCommand::DrawCrt(rng.below(4) as u8 + 1),
                3 => WorldCommand::SetGlobal(rng.below(64) as usize, rng.next() as i16),
                4 => WorldCommand::Tick(rng.below(30) as u16 + 1),
                5 => WorldCommand::AddObject(
                    rng.range(1, 40) as u16,
                    0x5000_0000 | rng.below(8) as u32,
                ),
                6 => WorldCommand::EditObject {
                    pick: rng.next() as usize,
                    word: rng.below(72) as usize,
                    value: rng.next() as i16,
                },
                7 => WorldCommand::PushFrame {
                    pick: rng.next() as usize,
                    tree: rng.range(4096, 4200) as u16,
                },
                8 => WorldCommand::RemoveObject(rng.next() as usize),
                9 => WorldCommand::InsertType(
                    rng.range(1, 12) as u16,
                    0x5000_0000 | rng.below(8) as u32,
                ),
                10 => WorldCommand::LookUpType(0x5000_0000 | rng.below(8) as u32),
                11 => WorldCommand::SetFlags {
                    pick: rng.next() as usize,
                    mask: rng.next() as u16,
                    on: rng.chance(1, 2),
                },
                12 => WorldCommand::SetCountdown {
                    pick: rng.next() as usize,
                    countdown: rng.range(-3, 300) as i16,
                },
                13 => WorldCommand::SetCursor(rng.range(0, 40) as i16),
                _ => WorldCommand::Queue {
                    pick: rng.next() as usize,
                    date: rng.chance(1, 2).then(|| rng.range(0, 100) as i16),
                },
            },
        )
        .collect()
}

/// Runs one command and returns what it answered, for the trace.
fn apply(world: &mut World, command: &WorldCommand) -> u64 {
    let pick = |world: &World, pick: usize| -> Option<u16> {
        let ids: Vec<u16> = world.objects.keys().copied().collect();
        (!ids.is_empty()).then(|| ids[pick % ids.len()])
    };
    match *command {
        WorldCommand::DrawLot(n) => (0..n).map(|_| u64::from(world.lot_rng.next_u32())).sum(),
        WorldCommand::DrawGlobal(n) => (0..n).map(|_| u64::from(world.global_rng.next_u32())).sum(),
        WorldCommand::DrawCrt(n) => (0..n).map(|_| u64::from(world.crt_rng.next_u15())).sum(),
        WorldCommand::SetGlobal(i, value) => {
            let n = world.globals.len();
            world.globals[i % n] = value;
            0
        }
        WorldCommand::Tick(n) => {
            world.ticks = world.ticks.wrapping_add(u32::from(n));
            0
        }
        WorldCommand::AddObject(id, guid) => {
            if world.objects.insert(id, object(id, guid)).is_none() {
                world.order.push(id);
            }
            0
        }
        WorldCommand::EditObject {
            pick: p,
            word,
            value,
        } => match pick(world, p) {
            Some(id) => {
                let object = world.objects.get_mut(&id).unwrap();
                object.data[word] = value;
                1
            }
            None => 0,
        },
        WorldCommand::PushFrame { pick: p, tree } => match pick(world, p) {
            Some(id) => {
                let object = world.objects.get_mut(&id).unwrap();
                object.stack.push(Frame {
                    stack_object: id as i16,
                    tree,
                    node: 0,
                    parameters: vec![1, 2, 3, 4],
                    locals: vec![0; 2],
                    primitive_state: 0,
                    code_owner_type: 1,
                });
                object.stack.len() as u64
            }
            None => 0,
        },
        WorldCommand::RemoveObject(p) => match pick(world, p) {
            Some(id) => {
                world.objects.remove(&id);
                world.order.retain(|&o| o != id);
                world.indicator_queues.forget(id);
                1
            }
            None => 0,
        },
        WorldCommand::InsertType(id, guid) => {
            world.insert_type(id, guid);
            0
        }
        WorldCommand::LookUpType(guid) => world.type_id_for_guid(guid).map_or(0, u64::from) + 1,
        WorldCommand::SetFlags { pick: p, mask, on } => match pick(world, p) {
            Some(id) => {
                let object = world.objects.get_mut(&id).unwrap();
                object.set_flags(mask, on);
                u64::from(object.flags()) + 1
            }
            None => 0,
        },
        WorldCommand::SetCountdown { pick: p, countdown } => match pick(world, p) {
            Some(id) => {
                let object = world.objects.get_mut(&id).unwrap();
                object.set_countdown(countdown);
                object.countdown().map_or(0, |c| c as u16 as u64 + 1)
            }
            None => 0,
        },
        WorldCommand::SetCursor(cursor) => {
            world.scheduler_cursor = cursor;
            0
        }
        WorldCommand::Queue { pick: p, date } => match pick(world, p) {
            Some(id) => {
                match date {
                    Some(date) => world.indicator_queues.queue_date(id, date),
                    None => world.indicator_queues.queue_star(id),
                }
                1
            }
            None => 0,
        },
    }
}

/// What a command leaves: its answer, the World's hash, and the pending
/// indicator refreshes, which the hash leaves out (they are
/// `#[serde(skip)]`, kept only by the snapshot payload).
type TraceEntry = (u64, [u8; 32], IndicatorQueues);

fn trace_entry(world: &mut World, command: &WorldCommand) -> TraceEntry {
    let answer = apply(world, command);
    (answer, world.hash(), world.indicator_queues.clone())
}

/// The first place two traces differ, counting from `offset`.
fn first_difference<T: PartialEq + Debug>(offset: usize, a: &[T], b: &[T]) -> Option<String> {
    let at = a.iter().zip(b).position(|(x, y)| x != y);
    match at {
        Some(i) => Some(format!(
            "command {}: {:?} against {:?}",
            offset + i,
            a[i],
            b[i]
        )),
        None if a.len() != b.len() => Some(format!("lengths {} and {}", a.len(), b.len())),
        None => None,
    }
}

/// The invariants of one World: its hash digests its encoding, and its
/// encodings read back to the same state.
fn world_invariants(world: &World) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let encoded = world.encode();
    let digest: [u8; 32] = Sha256::digest(&encoded).into();
    if world.hash() != digest {
        return Err("hash is not the digest of the encoding".into());
    }
    let decoded = World::decode(&encoded).map_err(|e| e.to_string())?;
    if decoded.encode() != encoded {
        return Err("decode(encode) encodes differently".into());
    }
    let snapshot = world.encode_snapshot();
    if !snapshot.starts_with(&encoded) {
        return Err("the snapshot does not start with the encoding".into());
    }
    let restored = World::decode_snapshot(&snapshot).map_err(|e| e.to_string())?;
    if restored.encode_snapshot() != snapshot {
        return Err("decode_snapshot(encode_snapshot) encodes differently".into());
    }
    if restored.indicator_queues != world.indicator_queues {
        return Err("the snapshot lost the indicator refreshes".into());
    }
    // The plain decoder reads a snapshot payload too, as an older build
    // does, and stops before the refreshes.
    let older = World::decode(&snapshot).map_err(|e| e.to_string())?;
    if older.encode() != encoded {
        return Err("decode(snapshot) reads another world".into());
    }
    Ok(())
}

fn start_world() -> World {
    let mut world = World::empty();
    world.lot_rng = GameRng::from_state(0x1234_5678);
    world.global_rng = GameRng::from_state(0x9abc_def0);
    world.crt_rng = CrtRng::from_state(17);
    world
}

/// Runs `commands` once without forking (the reference), then again
/// taking branches at each command index in `branches`. Each branch runs
/// the rest of the commands at once, before the run it came from goes on,
/// and must trace what the reference traced from there; the run it came
/// from must still trace what the reference did.
fn check_world_run(
    commands: &[WorldCommand],
    branches: &[usize],
    snapshots: Option<&Snapshots>,
) -> Result<(), String> {
    let mut reference = start_world();
    let expected: Vec<TraceEntry> = commands
        .iter()
        .map(|c| trace_entry(&mut reference, c))
        .collect();
    world_invariants(&reference)?;

    let mut main = start_world();
    let mut trace = Vec::with_capacity(commands.len());
    for (i, command) in commands.iter().enumerate() {
        if branches.contains(&i) {
            world_invariants(&main).map_err(|e| format!("before command {i}: {e}"))?;
            let mut forks = vec![
                ("warm fork", main.clone()),
                (
                    "cold restore",
                    World::decode_snapshot(&main.encode_snapshot()).map_err(|e| e.to_string())?,
                ),
            ];
            if let Some(snapshots) = snapshots {
                let name = format!("at-{i}");
                snapshots.save(&name, &main).map_err(|e| e.to_string())?;
                let mut loaded = snapshots
                    .load(&name)
                    .map_err(|e| e.to_string())?
                    .ok_or("a saved snapshot is missing")?;
                // A `Snapshots` record holds the world encoding alone; the
                // save payload that carries the refreshes is another
                // store's.
                if loaded.indicator_queues != Default::default() {
                    return Err(format!("at {i}: the REDB snapshot made up refreshes"));
                }
                loaded.indicator_queues = main.indicator_queues.clone();
                forks.push(("REDB restore", loaded));
            }
            // The parent as it stood when it forked, to show that running a
            // fork changed nothing in it, even a write the parent's next
            // command would make again and so hide from its trace.
            let parent = main.encode_snapshot();
            for (name, mut fork) in forks {
                let replay: Vec<TraceEntry> = commands[i..]
                    .iter()
                    .map(|c| trace_entry(&mut fork, c))
                    .collect();
                if let Some(why) = first_difference(i, &expected[i..], &replay) {
                    return Err(format!("{name} at {i} left the reference at {why}"));
                }
                world_invariants(&fork).map_err(|e| format!("{name} at {i}, at the end: {e}"))?;
                if main.encode_snapshot() != parent {
                    return Err(format!(
                        "{name} at {i} wrote through to the run it came from"
                    ));
                }
            }
        }
        trace.push(trace_entry(&mut main, command));
    }
    if let Some(why) = first_difference(0, &expected, &trace) {
        return Err(format!("the run that forked left the reference at {why}"));
    }
    Ok(())
}

#[test]
fn world_snapshots_and_forks_replay_the_reference() {
    let file = TempDb::new("fuzz-snapshots");
    let snapshots = Snapshots::open(&file.0).unwrap();
    for seed in 0..48u64 {
        let mut rng = Rng(seed);
        let commands = world_commands(&mut rng, 48);
        let mut branches: Vec<usize> = (0..3).map(|_| rng.below(48) as usize).collect();
        branches.sort_unstable();
        branches.dedup();
        // The REDB file only for some seeds: it is the slowest branch.
        let redb = seed % 4 == 0;
        check_or_shrink("world", seed, commands, |commands| {
            let branches: Vec<usize> = branches
                .iter()
                .copied()
                .filter(|&b| b < commands.len())
                .collect();
            check_world_run(commands, &branches, redb.then_some(&snapshots))
        });
    }
}

// ----- Stored neighborhoods and save inputs -----

fn family(rng: &mut Rng, number: i32) -> Family {
    let members = (0..rng.below(4))
        .map(|_| 0x1000_0000 | rng.below(64) as u32)
        .collect();
    Family {
        version: 9,
        house: rng.range(0, 12) as i32,
        number,
        budget: rng.range(0, 100_000) as i32,
        architecture_value: rng.range(0, 5_000) as i32,
        friends: rng.range(0, 6) as i32,
        flags: rng.below(16) as u32,
        members,
        tail: vec![0; 16],
    }
}

fn neighbor(rng: &mut Rng, id: i16) -> Neighbor {
    Neighbor {
        version: 10,
        version_extra: Some(9),
        name: format!("N{}", rng.below(1000)),
        unknown_zero: 0,
        person_mode: if rng.chance(1, 4) { -1 } else { 1 },
        person_data: (0..256).map(|_| rng.range(-50, 50) as i16).collect(),
        id,
        guid: 0x2000_0000 | rng.below(1 << 16) as u32,
        unknown_negative: -1,
        relationships: (0..rng.below(3))
            .map(|_| Relationship {
                target: rng.range(1, 8) as i32,
                values: vec![rng.range(-100, 100) as i32, rng.range(-100, 100) as i32],
            })
            .collect(),
    }
}

/// Family IDs a run draws from: a few small ones, the unset family 0 and
/// the Patch import's 4000.
const FAMILY_IDS: [u16; 7] = [0, 1, 2, 3, 5, 9, 4000];

fn stored_neighborhood(rng: &mut Rng, name: &str) -> StoredNeighborhood {
    let mut families = BTreeMap::new();
    let mut family_surnames = BTreeMap::new();
    for &id in &FAMILY_IDS {
        if rng.chance(1, 2) {
            let number = rng.range(0, 6) as i32;
            families.insert(id, family(rng, number));
            if rng.chance(2, 3) {
                family_surnames.insert(id, format!("S{}", rng.below(100)));
            }
        }
    }
    let neighbor_count = rng.below(5) as i16;
    let neighbors = (0..neighbor_count)
        .map(|id| (id > 0 && rng.chance(3, 4)).then(|| neighbor(rng, id)))
        .collect();
    let inventories = (0..rng.below(3) as i32)
        .map(|n| {
            rng.chance(1, 2).then(|| Inventory {
                neighbor: n as i16,
                items: (0..rng.below(3))
                    .map(|_| InventoryItem {
                        kind: rng.range(0, 2) as i32,
                        guid: 0x3000_0000 | rng.below(256) as u32,
                        count: rng.range(1, 5) as u16,
                    })
                    .collect(),
            })
        })
        .collect();
    let mut data = [0i16; 16];
    for word in &mut data {
        *word = rng.range(-5, 20) as i16;
    }
    let options = rng
        .chance(2, 3)
        .then(|| std::array::from_fn(|_| rng.chance(1, 2)));
    let zoning = (0..rng.below(4))
        .map(|_| (rng.range(1, 30) as i16, rng.range(1, 2) as i32))
        .collect();
    let lot_rows = (0..rng.below(4)).map(|_| rng.range(1, 30) as i32).collect();
    let deleted_characters: BTreeSet<u32> = (0..rng.below(3))
        .map(|_| 0x2000_0000 | rng.below(1 << 16) as u32)
        .collect();
    let deletion_epoch = rng.below(4);
    let deleted_at_epoch = deleted_characters
        .iter()
        .map(|&guid| (guid, rng.below(deletion_epoch + 1)))
        .collect();
    StoredNeighborhood {
        name: name.to_owned(),
        families,
        family_surnames,
        neighbors,
        inventories,
        data,
        options,
        type_attributes: None,
        zoning,
        lot_rows,
        made_characters: Characters::default(),
        deleted_characters,
        deletion_epoch,
        deleted_at_epoch,
        instance_id: (u128::from(rng.next()) << 64) | u128::from(rng.next()),
        revision: rng.below(1000),
    }
}

fn save_inputs(rng: &mut Rng) -> SaveInputs {
    let mut inputs = SaveInputs::default();
    for &id in &FAMILY_IDS {
        if rng.chance(1, 3) {
            inputs.histories.insert(id, rng.bytes(24));
        }
        if rng.chance(1, 3) {
            inputs
                .family_names
                .insert(id, format!(" S{} ", rng.below(100)));
        }
        if rng.chance(1, 3) {
            inputs
                .uninitialized
                .family_names
                .insert(id, rng.next() as u8);
        }
    }
    if rng.chance(1, 2) {
        inputs.option_bytes = rng.bytes(11);
    }
    inputs.uninitialized.neighbor_names = rng.bytes(4);
    inputs.uninitialized.map_padding = rng.bytes(4);
    inputs
}

#[test]
fn stored_neighborhoods_and_save_inputs_round_trip() {
    for seed in 0..256u64 {
        let mut rng = Rng(seed ^ 0x5eed);
        let state = stored_neighborhood(&mut rng, "Fuzz");
        let bytes = state.encode();
        let decoded = StoredNeighborhood::decode("Fuzz", &bytes)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(decoded, state, "seed {seed}");
        assert_eq!(decoded.encode(), bytes, "seed {seed}");
        let strict = StoredNeighborhood::decode_strict("Fuzz", &bytes)
            .unwrap_or_else(|e| panic!("seed {seed}: {e:?}"));
        assert_eq!(strict, state, "seed {seed}");
        // A cut encoding is refused, never read as something else.
        let cut = rng.below(bytes.len() as u64) as usize;
        assert!(
            StoredNeighborhood::decode("Fuzz", &bytes[..cut]).is_err(),
            "seed {seed}: a {cut}-byte prefix decoded"
        );

        let inputs = save_inputs(&mut rng);
        let bytes = inputs.encode();
        let decoded = SaveInputs::decode(&bytes).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(decoded, inputs, "seed {seed}");
        assert_eq!(decoded.encode(), bytes, "seed {seed}");

        // Every generated record saves, and the save is a function of the
        // record and the inputs.
        let first = state
            .neighborhood_iff(&inputs)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let second = decoded_state_save(&state, &inputs);
        assert_eq!(Ok(first), second, "seed {seed}");
    }
}

fn decoded_state_save(state: &StoredNeighborhood, inputs: &SaveInputs) -> Result<Vec<u8>, String> {
    let state = StoredNeighborhood::decode("Fuzz", &state.encode()).unwrap();
    let inputs = SaveInputs::decode(&inputs.encode()).unwrap();
    state.neighborhood_iff(&inputs).map_err(|e| e.to_string())
}

// ----- The REDB store -----

#[derive(Debug, Clone)]
enum StoreOp {
    /// `save_neighborhood`, which creates the record or advances it.
    Save(u8, Box<StoredNeighborhood>),
    /// `save_neighborhood_checked` with the committed version, or with a
    /// stale one, which must change nothing.
    SaveChecked {
        id: u8,
        state: Box<StoredNeighborhood>,
        stale: bool,
    },
    /// The commit that also changes the save inputs: a move-in, then a
    /// renumbering.
    Commit {
        id: u8,
        state: Box<StoredNeighborhood>,
        files: Vec<(String, Vec<u8>)>,
        move_in: (u16, Option<Vec<u8>>),
        renumber: Option<(u16, u16)>,
    },
    UpdateInputs(u8, Box<SaveInputs>),
    CharacterFile(u8, String, Vec<u8>),
    BaseHouse(u8, u32, Vec<u8>),
    /// Closes the store and opens its file again.
    Reopen,
}

const STORE_IDS: [&str; 2] = ["Alpha", "Beta"];

fn store_ops(rng: &mut Rng, count: usize) -> Vec<StoreOp> {
    (0..count)
        .map(|_| {
            let id = rng.below(STORE_IDS.len() as u64) as u8;
            let name = STORE_IDS[id as usize];
            match rng.weighted(&[3, 4, 3, 2, 2, 2, 1]) {
                0 => StoreOp::Save(id, Box::new(stored_neighborhood(rng, name))),
                1 => StoreOp::SaveChecked {
                    id,
                    state: Box::new(stored_neighborhood(rng, name)),
                    stale: rng.chance(1, 4),
                },
                2 => StoreOp::Commit {
                    id,
                    state: Box::new(stored_neighborhood(rng, name)),
                    files: (0..rng.below(3))
                        .map(|_| (format!("User{:05}.iff", rng.below(3)), rng.bytes(16)))
                        .collect(),
                    move_in: (
                        FAMILY_IDS[rng.below(FAMILY_IDS.len() as u64) as usize],
                        rng.chance(1, 2).then(|| rng.bytes(16)),
                    ),
                    renumber: rng.chance(1, 3).then(|| {
                        (
                            FAMILY_IDS[rng.below(FAMILY_IDS.len() as u64) as usize],
                            FAMILY_IDS[rng.below(FAMILY_IDS.len() as u64) as usize],
                        )
                    }),
                },
                3 => StoreOp::UpdateInputs(id, Box::new(save_inputs(rng))),
                4 => StoreOp::CharacterFile(
                    id,
                    format!("User{:05}.iff", rng.below(4)),
                    rng.bytes(32),
                ),
                5 => StoreOp::BaseHouse(id, rng.range(1, 12) as u32, rng.bytes(32)),
                _ => StoreOp::Reopen,
            }
        })
        .collect()
}

/// What the store should hold for one neighborhood.
#[derive(Debug, Clone, Default)]
struct Model {
    record: Option<StoredNeighborhood>,
    inputs: SaveInputs,
    characters: BTreeMap<String, Vec<u8>>,
    houses: BTreeMap<u32, Vec<u8>>,
}

impl Model {
    /// A commit of `next` over the record: the version advances, and the
    /// save inputs of each family the commit drops are forgotten (but not
    /// those of a family that was never in the record).
    fn commit(&mut self, mut next: StoredNeighborhood, version: (u128, u64)) {
        let previous = self.record.take().expect("a commit needs a record");
        next.instance_id = version.0;
        next.revision = version.1;
        // Written out here rather than through `SaveInputs::retain_families`,
        // so the model does not share the code it checks.
        for family in previous.families.keys() {
            if !next.families.contains_key(family) {
                forget_family(&mut self.inputs, *family);
            }
        }
        self.record = Some(next);
    }

    fn bump(&mut self) {
        if let Some(record) = &mut self.record {
            record.revision += 1;
        }
    }
}

/// A family's three save-input entries: its FAMh, its name as read and
/// its name's stray byte. The model keeps them by hand, apart from the
/// `SaveInputs` methods the store runs.
fn forget_family(inputs: &mut SaveInputs, family: u16) {
    inputs.histories.remove(&family);
    inputs.family_names.remove(&family);
    inputs.uninitialized.family_names.remove(&family);
}

/// A move-in from a `.FAM` file: the file's FAMh, or none, and no name
/// read for the ID.
fn model_move_in(inputs: &mut SaveInputs, family: u16, history: Option<Vec<u8>>) {
    forget_family(inputs, family);
    if let Some(history) = history {
        inputs.histories.insert(family, history);
    }
}

/// A renumbering: `to` takes `from`'s three entries, or loses its own
/// where `from` has none.
fn model_renumber(inputs: &mut SaveInputs, from: u16, to: u16) {
    if from == to {
        return;
    }
    let history = inputs.histories.remove(&from);
    let name = inputs.family_names.remove(&from);
    let byte = inputs.uninitialized.family_names.remove(&from);
    forget_family(inputs, to);
    if let Some(history) = history {
        inputs.histories.insert(to, history);
    }
    if let Some(name) = name {
        inputs.family_names.insert(to, name);
    }
    if let Some(byte) = byte {
        inputs.uninitialized.family_names.insert(to, byte);
    }
}

fn check_store_run(ops: &[StoreOp]) -> Result<(), String> {
    let file = TempDb::new("fuzz-store");
    let mut store = NeighborhoodStore::open(&file.0).map_err(|e| e.to_string())?;
    let mut models: [Model; 2] = Default::default();
    let mut instances = BTreeSet::new();
    let fail = |step: usize, what: String| Err(format!("op {step}: {what}"));
    for (step, op) in ops.iter().enumerate() {
        // A rejected operation leaves the block early, and still reaches
        // the comparison below, which shows it changed nothing.
        'apply: {
            match op {
                StoreOp::Save(id, state) => {
                    let model = &mut models[*id as usize];
                    let name = STORE_IDS[*id as usize];
                    store
                        .save_neighborhood(name, state)
                        .map_err(|e| e.to_string())?;
                    match &model.record {
                        Some(previous) => {
                            let version = (previous.instance_id, previous.revision + 1);
                            model.commit((**state).clone(), version);
                        }
                        None => {
                            // A new record gets a fresh instance and revision 0.
                            let loaded = store
                                .load_neighborhood(name)
                                .map_err(|e| e.to_string())?
                                .ok_or("a saved neighborhood is missing")?;
                            if loaded.revision != 0 || !instances.insert(loaded.instance_id) {
                                return fail(
                                    step,
                                    format!(
                                        "new record has version {:?}",
                                        (loaded.instance_id, loaded.revision)
                                    ),
                                );
                            }
                            let mut record = (**state).clone();
                            record.instance_id = loaded.instance_id;
                            record.revision = 0;
                            model.record = Some(record);
                        }
                    }
                }
                StoreOp::SaveChecked { id, state, stale } => {
                    let model = &mut models[*id as usize];
                    let name = STORE_IDS[*id as usize];
                    let Some(record) = &model.record else {
                        if store.save_neighborhood_checked(name, 0, 0, state).is_ok() {
                            return fail(step, "a checked save made a record".into());
                        }
                        break 'apply;
                    };
                    let (instance, revision) = (record.instance_id, record.revision);
                    let expected = if *stale {
                        revision.wrapping_sub(1)
                    } else {
                        revision
                    };
                    let result = store
                        .save_neighborhood_checked(name, instance, expected, state)
                        .map_err(|e| e.to_string())?;
                    match (result, *stale) {
                        (Ok(version), false) => {
                            if version != (instance, revision + 1) {
                                return fail(step, format!("committed {version:?}"));
                            }
                            model.commit((**state).clone(), version);
                        }
                        (Err(_), true) => {}
                        (result, stale) => {
                            return fail(step, format!("stale {stale} gave {result:?}"));
                        }
                    }
                }
                StoreOp::Commit {
                    id,
                    state,
                    files,
                    move_in,
                    renumber,
                } => {
                    let model = &mut models[*id as usize];
                    let name = STORE_IDS[*id as usize];
                    let batch = files.iter().map(|(n, b)| (n.as_str(), b.as_slice()));
                    let Some(record) = &model.record else {
                        let result = store.save_characters_neighborhood_and_save_inputs_checked(
                            name,
                            0,
                            0,
                            batch,
                            state,
                            |_| {},
                        );
                        if result.is_ok() {
                            return fail(step, "a commit made a record".into());
                        }
                        break 'apply;
                    };
                    let (instance, revision) = (record.instance_id, record.revision);
                    let change = |inputs: &mut SaveInputs| {
                        inputs.move_in_family(move_in.0, move_in.1.clone());
                        if let Some((from, to)) = *renumber {
                            inputs.renumber_family(from, to);
                        }
                    };
                    let unique: BTreeSet<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
                    let result = store.save_characters_neighborhood_and_save_inputs_checked(
                        name, instance, revision, batch, state, change,
                    );
                    if unique.len() != files.len() {
                        if result.is_ok() {
                            return fail(
                                step,
                                "a batch with a repeated file name committed".into(),
                            );
                        }
                        break 'apply;
                    }
                    let version = result
                        .map_err(|e| e.to_string())?
                        .map_err(|e| format!("{e:?}"))?;
                    if version != (instance, revision + 1) {
                        return fail(step, format!("committed {version:?}"));
                    }
                    model_move_in(&mut model.inputs, move_in.0, move_in.1.clone());
                    if let Some((from, to)) = *renumber {
                        model_renumber(&mut model.inputs, from, to);
                    }
                    for (n, b) in files {
                        model.characters.insert(n.clone(), b.clone());
                    }
                    model.commit((**state).clone(), version);
                }
                StoreOp::UpdateInputs(id, inputs) => {
                    let model = &mut models[*id as usize];
                    let name = STORE_IDS[*id as usize];
                    let result =
                        store.update_save_inputs(name, |stored| *stored = (**inputs).clone());
                    match (&model.record, result) {
                        (Some(_), Ok(())) => model.inputs = (**inputs).clone(),
                        (None, Err(_)) => {}
                        (record, result) => {
                            return fail(
                                step,
                                format!("record {} gave {result:?}", record.is_some()),
                            );
                        }
                    }
                }
                StoreOp::CharacterFile(id, file, bytes) => {
                    let model = &mut models[*id as usize];
                    store
                        .save_character_file(STORE_IDS[*id as usize], file, bytes)
                        .map_err(|e| e.to_string())?;
                    model.characters.insert(file.clone(), bytes.clone());
                    model.bump();
                }
                StoreOp::BaseHouse(id, house, bytes) => {
                    let model = &mut models[*id as usize];
                    store
                        .save_base_house(STORE_IDS[*id as usize], *house, bytes)
                        .map_err(|e| e.to_string())?;
                    model.houses.insert(*house, bytes.clone());
                    model.bump();
                }
                StoreOp::Reopen => {
                    drop(store);
                    store = NeighborhoodStore::open(&file.0).map_err(|e| e.to_string())?;
                }
            }
        }
        for (id, model) in models.iter().enumerate() {
            compare(&store, STORE_IDS[id], model)
                .map_err(|e| format!("op {step} ({op:?}): {e}"))?;
        }
    }
    // A cold restore at the end holds the same.
    drop(store);
    let store = NeighborhoodStore::open(&file.0).map_err(|e| e.to_string())?;
    for (id, model) in models.iter().enumerate() {
        compare(&store, STORE_IDS[id], model).map_err(|e| format!("after reopening: {e}"))?;
    }
    Ok(())
}

/// Checks the store against the model, and that saving `Neighborhood.iff`
/// from the store writes what saving the model's record with the model's
/// inputs writes.
fn compare(store: &NeighborhoodStore, name: &str, model: &Model) -> Result<(), String> {
    let record = store.load_neighborhood(name).map_err(|e| e.to_string())?;
    if record != model.record {
        return Err(format!("{name}: record differs"));
    }
    let inputs = store.load_save_inputs(name).map_err(|e| e.to_string())?;
    if inputs != model.inputs {
        return Err(format!(
            "{name}: save inputs {inputs:?}, expected {:?}",
            model.inputs
        ));
    }
    let characters = store
        .load_character_files(name)
        .map_err(|e| e.to_string())?;
    if characters != model.characters {
        return Err(format!("{name}: character files differ"));
    }
    for (house, bytes) in &model.houses {
        let stored = store
            .load_base_house(name, *house)
            .map_err(|e| e.to_string())?;
        if stored.as_ref() != Some(bytes) {
            return Err(format!("{name}: base house {house} differs"));
        }
    }
    if let Some(record) = &model.record {
        let directory = TempDir::new("fuzz-iff");
        std::fs::create_dir_all(&directory.0).map_err(|e| e.to_string())?;
        let saved = store
            .save_neighborhood_iff(name, &directory.0)
            .map_err(|e| e.to_string())
            .and_then(|path| std::fs::read(path).map_err(|e| e.to_string()));
        // Every generated record can be saved, so a failure on either side
        // is one.
        let expected = record
            .neighborhood_iff(&model.inputs)
            .map_err(|e| format!("{name}: the model's record cannot be saved: {e}"))?;
        let saved = saved.map_err(|e| format!("{name}: the store failed to save: {e}"))?;
        if saved != expected {
            return Err(format!("{name}: Neighborhood.iff differs"));
        }
    }
    Ok(())
}

#[test]
fn the_store_matches_its_model_across_reopening() {
    for seed in 0..12u64 {
        let mut rng = Rng(seed ^ 0x570e);
        let ops = store_ops(&mut rng, 20);
        check_or_shrink("store", seed, ops, check_store_run);
    }
}

#[test]
fn the_fuzzer_reports_and_shrinks_a_failure() {
    // A check that fails whenever the list holds both 3 and 7 shrinks to
    // exactly those two.
    let result = catch_unwind(|| {
        check_or_shrink("demo", 9, (0..20).collect::<Vec<u32>>(), |list| {
            if list.contains(&3) && list.contains(&7) {
                Err("3 and 7".into())
            } else {
                Ok(())
            }
        });
    });
    let message = result.unwrap_err();
    let message = message.downcast_ref::<String>().unwrap();
    assert!(message.contains("seed 0x9 fails: 3 and 7"), "{message}");
    assert!(message.contains("[\n    3,\n    7,\n]"), "{message}");
    // A panicking check is reported, not propagated as is.
    let result = catch_unwind(|| {
        check_or_shrink("demo", 1, vec![1u8], |_| panic!("boom"));
    });
    let message = result.unwrap_err();
    assert!(
        message
            .downcast_ref::<String>()
            .unwrap()
            .contains("panicked: boom")
    );
    // Draws are reproducible and in range.
    let mut a = Rng(5);
    let mut b = Rng(5);
    for _ in 0..1000 {
        let x = a.below(7);
        assert_eq!(x, b.below(7));
        assert!(x < 7);
    }
}
