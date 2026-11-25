//! A deterministic tick-level fuzzer: seeded `sims_net` [`Command`]s over
//! a live Goth lot, with every branch checked tick by tick against the run
//! it came from.
//!
//! The design is the legacy exploration fuzzer's
//! (`legacy-final:crates/sims-runtime/src/exploration.rs`), driven by the
//! co-op layer, which is the port's deterministic command executor
//! ([`Lot::apply_coop_tick_batch`]):
//!
//! - A seeded SplitMix64 stream picks each step from the live lot's
//!   candidates (a pie-menu entry for a person on an object not parked
//!   off the lot, a cancel of a queued action, speed, pause, free will, selection,
//!   camera, a dialog's answer, or nothing) by weight, with bounded draws
//!   by rejection sampling. A dialog's answer is one of the buttons it
//!   offers, and a special panel's is its cancel or dismissal. An object
//!   parked off the lot at [-16, -16] (a factory's or a removal's: the
//!   helpers of the lot's people and interactions, the off-lot people) is
//!   never drawn, so no player can open its pie menu. Some of those menus'
//!   scripts fault when run that way (a Scold whose stack object is 0).
//!   Objects on the lot stay candidates, contents included (they keep
//!   their holder's position), even one the lot hides. Each step is one tick's
//!   batch and then up to a few empty ticks. The step is journaled before
//!   it runs, so a step that panics or fails is still in the list that
//!   reproduces it.
//! - The journal replays with no RNG at all. The replay of a journal must
//!   trace what the run that drew it traced.
//! - The reference run is warm: it never restores. At several command
//!   boundaries, deduplicated and all required, a second lot is restored
//!   cold from the reference's co-op snapshot ([`Lot::coop_snapshot_bytes`]:
//!   `Lot::save` → `Saved::encode`, then `Saved::decode` → `Lot::restore`
//!   in [`Lot::restore_coop_snapshot`]), and at one boundary it is forked
//!   warm instead ([`Lot::fork_coop`] → [`Lot::restore_coop_fork`]): the
//!   reference's in-memory [`sims_vm::boot::save::Saved`], whose world
//!   shares copy-on-write storage with the reference, with the reference's
//!   co-op context as it is. `Lot` has no `Clone` (a Buy cursor token names
//!   one lot, and a lot's store origin fences one save), so this is the
//!   port's warm fork. The warm fork is required: the boundary is drawn from
//!   those the lot allows a fork at ([`Lot::coop_fork_blocker`]: no dialog
//!   waiting, no cursor), and a run that never takes it fails.
//! - Each branch runs the rest of the journal at once, before the reference
//!   goes on, and must trace what the reference traced from there: after
//!   every tick, the lot's tick count, [`sims_sim::World::hash`], the co-op
//!   world hash, the session's combined state fingerprint, the three RNG
//!   streams and what each command did. A failure names the first step
//!   and tick whose trace differs. The reference must be unchanged by its
//!   branch (no write through shared storage), and must still trace what a
//!   run with no branches did.
//! - A panic is caught and reported with its seed, and a failing journal
//!   is shrunk by delta debugging before it is reported, keeping only
//!   shorter journals that fail the same way.
//!
//! The lots step strictly, as the shell's co-op does: a script error
//! fails the run, as it halts the lot. `SIMS_FUZZ_PLAY=1` steps in play
//! mode instead, where the faulting object is reset as the game does, to
//! hunt divergence past known script errors.
//!
//! The defaults keep a release run to a few seconds. `SIMS_FUZZ_SEEDS`,
//! `SIMS_FUZZ_STEPS` and `SIMS_FUZZ_SEED_BASE` run more, longer or other
//! seeds. Needs `SIMS_INSTALL`; skipped otherwise. No wall clock reaches a
//! trace.

use sims_core::{CrtRng, GameRng};
use sims_net::{
    CameraRotation, Command, DialogAnswer, SessionConfig, SessionState, SimSpeed, StampedCommand,
    StateFingerprint, TickBatch,
};
use sims_vm::boot::{Frames, Lot};
use sims_vm::coop::special_panel_answer_command;
use sims_vm::{
    CoopCommandStatus, CoopVmContext, SpecialPanelResponse, cancel_uid_to_wire,
    menu_entry_to_pick_command,
};
use std::cell::RefCell;
use std::fmt::{self, Debug};
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

    fn pick<T: Copy>(&mut self, values: &[T]) -> Option<T> {
        (!values.is_empty()).then(|| values[self.below(values.len() as u64) as usize])
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("?")
        .to_owned()
}

/// Runs `check` on `commands`; on failure, shrinks the list by delta
/// debugging (removing ever smaller chunks while the check still fails the
/// same way) and returns the seed, the original failure, the shrunk list
/// and its failure.
///
/// A failure's kind is its text up to the first `": "`; a shorter list only
/// counts as failing when it fails with the original's kind, so a
/// divergence doesn't shrink into a script error that cutting a step
/// caused.
fn shrink<C: Clone + Debug>(
    what: &str,
    seed: u64,
    commands: Vec<C>,
    check: impl Fn(&[C]) -> Result<(), String>,
) -> Result<(), String> {
    let run = |commands: &[C]| match catch_unwind(AssertUnwindSafe(|| check(commands))) {
        Ok(result) => result,
        Err(panic) => Err(format!("panicked: {}", panic_message(&*panic))),
    };
    let Err(first) = run(&commands) else {
        return Ok(());
    };
    let kind = |failure: &str| failure.split(": ").next().unwrap_or("").to_owned();
    let first_kind = kind(&first);
    let mut failing = commands;
    let mut failure = first.clone();
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
            match run(&candidate) {
                Err(why) if kind(&why) == first_kind => {
                    failing = candidate;
                    failure = why;
                    shrunk = true;
                }
                _ => start = end,
            }
        }
        if chunk == 1 && !shrunk {
            break;
        }
        if !shrunk {
            chunk = chunk.div_ceil(2);
        }
    }
    let shrunk = if failure == first {
        String::new()
    } else {
        format!(", which fails: {failure}")
    };
    Err(format!(
        "{what} seed {seed:#x} fails: {first}\nshrunk to {failing:#?}{shrunk}"
    ))
}

/// One tick's commands, then `idle` empty ticks.
#[derive(Clone, PartialEq, Eq)]
struct Step {
    commands: Vec<Command>,
    idle: u8,
}

impl Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} then {} idle", self.commands, self.idle)
    }
}

/// What a tick leaves: the lot's tick count, the World's hash, the co-op
/// world hash (the session-side state the World leaves out: the dialog
/// being answered, the answers, the frames, the activity owners) and the
/// three RNG streams.
#[derive(Clone, PartialEq, Eq)]
struct Observation {
    ticks: u32,
    world: [u8; 32],
    coop: [u8; 32],
    /// The session and the world together, as lockstep peers compare them.
    fingerprint: StateFingerprint,
    rng: (GameRng, GameRng, CrtRng),
    /// What each of the tick's commands did (none at a boundary), so a
    /// command applied on one side and refused on the other differs even
    /// when the state agrees.
    results: Vec<CoopCommandStatus>,
}

impl Debug for Observation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hex = |h: &[u8; 32]| {
            h[..6]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        write!(
            f,
            "tick {} world {} co-op {} state {} rng {:?} results {:?}",
            self.ticks,
            hex(&self.world),
            hex(&self.coop),
            hex(&self.fingerprint.0),
            self.rng,
            self.results
        )
    }
}

fn observe(lot: &Lot) -> Observation {
    let coop = lot.coop_world_hash();
    let fingerprint = lot.coop_state_fingerprint(&session(lot));
    observe_with(lot, coop, fingerprint, Vec::new())
}

/// [`observe`], given what a batch reported.
fn observe_with(
    lot: &Lot,
    coop: [u8; 32],
    fingerprint: StateFingerprint,
    results: Vec<CoopCommandStatus>,
) -> Observation {
    let world = lot.world();
    Observation {
        ticks: lot.ticks(),
        world: world.hash(),
        coop,
        fingerprint,
        rng: (world.lot_rng, world.global_rng, world.crt_rng),
        results,
    }
}

fn session(lot: &Lot) -> SessionState {
    lot.coop().expect("co-op is attached").session.clone()
}

/// Runs one batch at the session's next tick, observing after it.
fn batch(lot: &mut Lot, commands: &[Command]) -> Result<Observation, String> {
    let session = session(lot);
    let tick = session.current_tick.next();
    let batch = TickBatch {
        epoch: session.epoch,
        tick,
        commands: commands
            .iter()
            .enumerate()
            .map(|(sequence, command)| StampedCommand {
                tick,
                sequence: sequence as u32,
                issuer: session.host_id,
                command: command.clone(),
            })
            .collect(),
        pacing_hint_ms: 0,
        host_fingerprint: None,
    };
    let report = lot
        .apply_coop_tick_batch(&batch)
        .map_err(|e| format!("tick {tick:?}: {e}"))?;
    let results = report
        .command_results
        .into_iter()
        .map(|(_, status)| status)
        .collect();
    Ok(observe_with(
        lot,
        report.world_hash,
        report.state_fingerprint,
        results,
    ))
}

/// Runs `step`, observing after each tick.
fn run_step(lot: &mut Lot, step: &Step) -> Result<Vec<Observation>, String> {
    let mut trace = Vec::with_capacity(1 + usize::from(step.idle));
    trace.push(batch(lot, &step.commands)?);
    for _ in 0..step.idle {
        trace.push(batch(lot, &[])?);
    }
    Ok(trace)
}

/// Where an object off the lot is parked (`sims_vm` factory and removal).
const PARKED: [i32; 2] = [-16, -16];

/// The commands the live lot offers, one chosen by weight.
fn draw_step(lot: &Lot, rng: &mut Rng) -> Step {
    let world = lot.world();
    let people: Vec<u16> = world.people.clone();
    // A player opens a pie menu by clicking an object the lot draws. An
    // object parked off the lot (a factory's or a removal's [-16, -16]:
    // the lot's people's and interactions' helpers, the off-lot people)
    // is never drawn, so no player can pick from its menu.
    let objects: Vec<u16> = world
        .objects
        .values()
        .filter(|o| o.position[..2] != PARKED)
        .map(|o| o.id)
        .collect();
    let session = session(lot);
    let host = session.host_id;
    let answers = answers(lot);
    let mut commands = Vec::new();
    // Nothing, a pie-menu pick, a cancel, speed, pause, free will,
    // selection, camera, a dialog's answer.
    let weights = [
        6,
        5,
        1,
        1,
        1,
        1,
        1,
        1,
        if answers.is_empty() { 0 } else { 6 },
    ];
    match rng.weighted(&weights) {
        0 => {}
        1 => {
            // Most objects offer a person nothing: a few tries find a menu.
            for _ in 0..6 {
                if let (Some(sim), Some(target)) = (rng.pick(&people), rng.pick(&objects))
                    && let Ok(menu) = lot.coop_pie_menu_for_sim(host, sim, target)
                    && !menu.entries.is_empty()
                {
                    let entry = &menu.entries[rng.below(menu.entries.len() as u64) as usize];
                    commands.push(menu_entry_to_pick_command(entry));
                    break;
                }
            }
        }
        2 => {
            if let Some(sim) = rng.pick(&people) {
                let uids: Vec<i32> = world.objects[&sim]
                    .person
                    .iter()
                    .flat_map(|p| p.queue.iter().map(|i| i.uid))
                    .collect();
                if let Some(uid) = rng.pick(&uids) {
                    commands.push(Command::SimActionCancel {
                        sim_id: sim,
                        interaction_uid: cancel_uid_to_wire(uid),
                    });
                }
            }
        }
        3 => commands.push(Command::SetSpeed {
            speed: [SimSpeed::NORMAL, SimSpeed::FAST, SimSpeed::ULTRA][rng.below(3) as usize],
        }),
        4 => commands.push(Command::SetPaused {
            paused: rng.chance(1, 3),
        }),
        5 => commands.push(Command::SetFreeWill {
            enabled: rng.chance(1, 2),
        }),
        6 => {
            if let Some(sim) = rng.pick(&people) {
                commands.push(Command::SelectPerson { sim_id: Some(sim) });
            }
        }
        7 => commands.push(Command::SetCameraRotation {
            rotation: CameraRotation::new(rng.below(4) as u8),
        }),
        _ => commands.push(answers[rng.below(answers.len() as u64) as usize].clone()),
    }
    Step {
        commands,
        idle: rng.below(6) as u8,
    }
}

/// The answers the open dialog or special panel accepts: each button an
/// ordinary dialog offers, or the cancel or dismissal a special panel takes
/// (a lot picked in a picker would leave the lot, which a co-op lot can't).
fn answers(lot: &Lot) -> Vec<Command> {
    let ctx = lot.coop().expect("co-op is attached");
    if let Some((id, shown)) = &ctx.active_special_panel {
        return [
            SpecialPanelResponse::Lot(None),
            SpecialPanelResponse::Neighbor(None),
            SpecialPanelResponse::Selection {
                accepted: false,
                index: 0,
            },
            SpecialPanelResponse::Dismissed,
        ]
        .iter()
        .filter(|response| shown.request.accepts(response))
        .map(|response| special_panel_answer_command(*id, response))
        .collect();
    }
    let (Some(id), Some(asking)) = (ctx.active_dialog, lot.asking()) else {
        return Vec::new();
    };
    asking
        .shown
        .answers
        .iter()
        .map(|&status| Command::AnswerBlockingDialog {
            dialog_id: id,
            answer: DialogAnswer {
                status,
                bitmap_id: None,
                text: None,
            },
        })
        .collect()
}

/// The reference lot, the branch lot, and the boot both start from.
struct Lots {
    reference: Lot,
    branch: Lot,
    boot: Vec<u8>,
    boot_session: SessionState,
}

impl Lots {
    fn load() -> Option<Self> {
        let root = crate::common::install_root()?;
        let house = root.join("UserData/Houses/House05.iff");
        let play = env("SIMS_FUZZ_PLAY", 0) != 0;
        let attach = || {
            let mut lot = Lot::load(&root, &house, &[], Frames::default()).unwrap();
            let mut session = SessionState::new("Host", None, SessionConfig::default());
            lot.sync_initial_coop_session(&mut session);
            lot.attach_coop_context(CoopVmContext::new(session).with_play_mode(play))
                .unwrap();
            lot
        };
        let reference = attach();
        let branch = attach();
        let boot = reference.coop_snapshot_bytes().unwrap();
        let boot_session = session(&reference);
        Some(Self {
            reference,
            branch,
            boot,
            boot_session,
        })
    }

    /// The reference as it booted.
    fn reset(&mut self) -> Result<(), String> {
        self.reference
            .restore_coop_snapshot(&self.boot, self.boot_session.clone())
            .map_err(|e| format!("restoring the boot: {e}"))
    }
}

/// Draws `count` steps from the live reference, journaling each before it
/// runs. A step that fails or panics ends the journal there, and is the
/// last step of it, and its failure is returned. Also returns the
/// boundaries a warm fork can be taken at ([`Lot::coop_fork_blocker`]).
fn explore(lots: &mut Lots, seed: u64, count: usize) -> Explored {
    lots.reset().unwrap();
    let mut rng = Rng(seed);
    let mut journal = Vec::with_capacity(count);
    let mut traces = Vec::with_capacity(count);
    let mut forks = Vec::new();
    let mut ended = None;
    for i in 0..count {
        if forkable(&lots.reference) {
            forks.push(i);
        }
        let step = draw_step(&lots.reference, &mut rng);
        journal.push(step.clone());
        match catch_unwind(AssertUnwindSafe(|| run_step(&mut lots.reference, &step))) {
            Ok(Ok(trace)) => traces.push(trace),
            Ok(Err(e)) => {
                ended = Some(format!("step {i}: {e}"));
                break;
            }
            Err(panic) => {
                ended = Some(format!("step {i} panicked: {}", panic_message(&*panic)));
                break;
            }
        }
    }
    Explored {
        journal,
        traces,
        forks,
        ended,
    }
}

/// A drawn run: its journal, its trace, the boundaries it could be forked
/// warm at, and how it ended, if a step failed.
struct Explored {
    journal: Vec<Step>,
    traces: Vec<Vec<Observation>>,
    forks: Vec<usize>,
    ended: Option<String>,
}

/// Whether the lot can be forked warm now, by the lot's own rule.
fn forkable(lot: &Lot) -> bool {
    lot.coop_fork_blocker().is_none()
}

/// The first step and tick where two traces differ, counting from step
/// `offset`.
fn first_difference(
    offset: usize,
    expected: &[Vec<Observation>],
    actual: &[Vec<Observation>],
) -> Option<String> {
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if let Some(t) = e.iter().zip(a).position(|(x, y)| x != y) {
            return Some(format!(
                "step {}, tick {t} of it: expected {:?}, got {:?}",
                offset + i,
                e[t],
                a[t]
            ));
        }
        if e.len() != a.len() {
            return Some(format!(
                "step {}: {} ticks against {}",
                offset + i,
                e.len(),
                a.len()
            ));
        }
    }
    (expected.len() != actual.len())
        .then(|| format!("{} steps against {}", expected.len(), actual.len()))
}

/// The branches to take at each command boundary of a journal.
#[derive(Clone, Debug, Default)]
struct Branches {
    cold: Vec<usize>,
    warm: Option<usize>,
}

/// Replays `steps` on the reference (it must trace `drawn`, when given, as
/// the run that drew them did), then again taking the branches: each runs
/// the rest of the steps on the branch lot, at once, and must trace what
/// the reference did from there, and leave the reference as it was.
fn check_run(
    lots: &mut Lots,
    steps: &[Step],
    branches: &Branches,
    drawn: Option<&[Vec<Observation>]>,
) -> Result<(), String> {
    lots.reset()?;
    let expected: Vec<Vec<Observation>> = steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            run_step(&mut lots.reference, step).map_err(|e| format!("script error: step {i}: {e}"))
        })
        .collect::<Result<_, _>>()?;
    if let Some(drawn) = drawn
        && let Some(why) = first_difference(0, drawn, &expected)
    {
        return Err(format!(
            "replay: the replay left the run that drew it at {why}"
        ));
    }

    lots.reset()?;
    let mut trace = Vec::with_capacity(steps.len());
    let mut warm_taken = false;
    for (i, step) in steps.iter().enumerate() {
        let cold = branches.cold.contains(&i);
        // A lot waiting on a dialog keeps the world from before the tick
        // that asked, and only the co-op snapshot asks again on restore; the
        // warm fork waits for a boundary with no dialog open.
        let warm =
            branches.warm.is_some_and(|w| w <= i) && !warm_taken && forkable(&lots.reference);
        warm_taken |= warm;
        if cold || warm {
            let parent = lots
                .reference
                .coop_snapshot_bytes()
                .map_err(|e| format!("snapshot: at {i}: {e}"))?;
            let at = observe(&lots.reference);
            let mut kinds = Vec::new();
            if cold {
                kinds.push("cold restore");
            }
            if warm {
                kinds.push("warm fork");
            }
            for kind in kinds {
                match kind {
                    "cold restore" => lots
                        .branch
                        .restore_coop_snapshot(&parent, session(&lots.reference))
                        .map_err(|e| format!("restore: cold restore at {i}: {e}"))?,
                    _ => {
                        let fork = lots
                            .reference
                            .fork_coop()
                            .map_err(|e| format!("restore: warm fork at {i}: {e}"))?;
                        lots.branch
                            .restore_coop_fork(fork)
                            .map_err(|e| format!("restore: warm fork at {i}: {e}"))?;
                    }
                }
                if observe(&lots.branch) != at {
                    return Err(format!(
                        "divergence: {kind} at {i} changed the lot: {:?} against {at:?}",
                        observe(&lots.branch)
                    ));
                }
                let replay: Vec<Vec<Observation>> = steps[i..]
                    .iter()
                    .enumerate()
                    .map(|(j, step)| {
                        run_step(&mut lots.branch, step).map_err(|e| format!("step {}: {e}", i + j))
                    })
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("divergence: {kind} at {i}: {e}"))?;
                if let Some(why) = first_difference(i, &expected[i..], &replay) {
                    return Err(format!(
                        "divergence: {kind} at {i} left the reference at {why}"
                    ));
                }
                if lots
                    .reference
                    .coop_snapshot_bytes()
                    .map_err(|e| format!("snapshot: {e}"))?
                    != parent
                {
                    return Err(format!(
                        "divergence: {kind} at {i} wrote through to the run it came from"
                    ));
                }
            }
        }
        trace.push(
            run_step(&mut lots.reference, step)
                .map_err(|e| format!("divergence: the run that branched, step {i}: {e}"))?,
        );
    }
    if let Some(why) = first_difference(0, &expected, &trace) {
        return Err(format!(
            "divergence: the run that branched left the reference at {why}"
        ));
    }
    if let Some(w) = branches.warm
        && !warm_taken
    {
        return Err(format!(
            "no warm fork: a dialog stayed open from boundary {w} on"
        ));
    }
    Ok(())
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The command kinds the summary counts.
const KINDS: [&str; 8] = [
    "SimActionPick",
    "SimActionCancel",
    "SetSpeed",
    "SetPaused",
    "SetFreeWill",
    "SelectPerson",
    "SetCameraRotation",
    "AnswerBlockingDialog",
];

#[test]
fn seeded_commands_replay_across_warm_forks_and_cold_restores() {
    let Some(lots) = Lots::load() else {
        eprintln!("SIMS_INSTALL not set; skipping the tick-level fuzzer");
        return;
    };
    let lots = RefCell::new(lots);
    let seeds = env("SIMS_FUZZ_SEEDS", 3);
    let count = env("SIMS_FUZZ_STEPS", 20) as usize;
    let base = env("SIMS_FUZZ_SEED_BASE", 0);
    for seed in base..base + seeds {
        let Explored {
            journal,
            traces: drawn,
            forks,
            ended,
        } = explore(&mut lots.borrow_mut(), seed, count);
        let mut rng = Rng(seed ^ 0x5eed_b4a7);
        let n = journal.len() as u64;
        let mut cold: Vec<usize> = std::iter::once(0)
            .chain((0..2).map(|_| rng.below(n) as usize))
            .collect();
        cold.sort_unstable();
        cold.dedup();
        let branches = Branches {
            cold,
            warm: rng.pick(&forks),
        };
        assert!(
            branches.warm.is_some(),
            "seed {seed:#x}: no boundary without a dialog open to fork at"
        );
        let full = journal.len();
        let kinds: std::collections::BTreeMap<&str, usize> = journal
            .iter()
            .flat_map(|s| &s.commands)
            .fold(Default::default(), |mut kinds, command| {
                let name = format!("{command:?}");
                let name = &name[..name.find([' ', '{', '(']).unwrap_or(name.len())];
                let key = KINDS
                    .iter()
                    .find(|k| **k == name)
                    .copied()
                    .unwrap_or("other");
                *kinds.entry(key).or_default() += 1;
                kinds
            });
        eprintln!(
            "seed {seed:#x}: {full} steps, {} ticks, {kinds:?}, branches {branches:?}",
            drawn.iter().map(Vec::len).sum::<usize>()
        );
        let checked = shrink("tick fuzzer", seed, journal, |steps| {
            let branches = Branches {
                cold: branches
                    .cold
                    .iter()
                    .copied()
                    .filter(|&b| b < steps.len())
                    .collect(),
                warm: branches.warm.filter(|&b| b < steps.len()),
            };
            // The drawn trace belongs to the whole journal only.
            let drawn = (steps.len() == full).then_some(drawn.as_slice());
            check_run(&mut lots.borrow_mut(), steps, &branches, drawn)
        });
        // The drawn run's own failure is reported too: a panic or error that
        // its replay doesn't repeat would otherwise show only as a shorter
        // trace.
        if let Err(report) = checked {
            match &ended {
                Some(ended) => panic!("{report}\nthe drawn run ended at {ended}"),
                None => panic!("{report}"),
            }
        }
    }
}

/// The shrinker reports the seed and cuts a failing journal down to the
/// commands that fail it. No install needed.
#[test]
fn the_fuzzer_reports_and_shrinks_a_failure() {
    let journal: Vec<Step> = (0..40)
        .map(|i| Step {
            commands: if i == 7 || i == 31 {
                vec![Command::SetFreeWill { enabled: false }]
            } else {
                vec![]
            },
            idle: (i % 3) as u8,
        })
        .collect();
    let disables = |steps: &[Step]| {
        steps
            .iter()
            .filter(|s| s.commands == [Command::SetFreeWill { enabled: false }])
            .count()
    };
    // Fails while both free-will commands are in the list.
    let report = shrink("synthetic", 0x2a, journal.clone(), |steps| {
        if disables(steps) >= 2 {
            Err("free will went off twice".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(report.starts_with("synthetic seed 0x2a fails: free will went off twice"));
    assert_eq!(report.matches("SetFreeWill").count(), 2, "{report}");
    assert_eq!(report.matches(" idle").count(), 2, "{report}");
    // A panic is a failure, reported with its message.
    let report = shrink("synthetic", 0x2b, journal.clone(), |steps| {
        assert!(disables(steps) == 0, "a step panicked");
        Ok(())
    })
    .unwrap_err();
    assert!(report.contains("panicked: a step panicked"), "{report}");
    assert_eq!(report.matches("SetFreeWill").count(), 1, "{report}");
    // A shorter list that fails another way doesn't count: the report
    // keeps the original failure and a list that still fails like it.
    let report = shrink("synthetic", 0x2d, journal.clone(), |steps| {
        match disables(steps) {
            2 => Err("divergence: both".into()),
            1 => Err("script error: one".into()),
            _ => Ok(()),
        }
    })
    .unwrap_err();
    assert!(
        report.starts_with("synthetic seed 0x2d fails: divergence: both"),
        "{report}"
    );
    assert!(!report.contains("script error"), "{report}");
    assert_eq!(report.matches("SetFreeWill").count(), 2, "{report}");
    // A passing list is not reported.
    assert!(shrink("synthetic", 0x2c, journal, |_| Ok(())).is_ok());
}

/// A warm fork keeps what the lot is, not what its session asks for: on a
/// residential lot nobody holds the speed of, the idle loop runs at 1000
/// while the session's requested speed stands, and the fork must too.
/// Attaching the context again would set the session's speed.
#[test]
fn a_warm_fork_keeps_a_speed_the_session_did_not_ask_for() {
    let Some(mut lots) = Lots::load() else {
        eprintln!("SIMS_INSTALL not set; skipping");
        return;
    };
    lots.reset().unwrap();
    lots.reference.set_speed(1000).unwrap();
    let requested = i32::from(session(&lots.reference).time.speed.value());
    assert_ne!(i32::from(lots.reference.world().speed()), requested);
    let fork = lots.reference.fork_coop().unwrap();
    lots.branch.restore_coop_fork(fork).unwrap();
    assert_eq!(observe(&lots.branch), observe(&lots.reference));
    assert_eq!(lots.branch.world().speed(), 1000);
    let steps: Vec<Step> = (0..4)
        .map(|_| Step {
            commands: Vec::new(),
            idle: 2,
        })
        .collect();
    for step in &steps {
        assert_eq!(
            run_step(&mut lots.branch, step).unwrap(),
            run_step(&mut lots.reference, step).unwrap()
        );
    }
    // The attach, by contrast, applies the requested speed. A lot out of
    // co-op can't be forked.
    let saved = lots.reference.save().unwrap();
    lots.branch.restore(saved).unwrap();
    assert_eq!(
        lots.branch.coop_fork_blocker(),
        Some("co-op is not attached")
    );
    lots.branch
        .attach_coop_context(lots.reference.coop().unwrap().clone())
        .unwrap();
    assert_ne!(
        lots.branch.world().speed(),
        lots.reference.world().speed(),
        "the attach no longer sets the session's speed; the fork test can go"
    );
}
