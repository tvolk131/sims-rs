//! Deterministic, bounded exploration of real application commands.
//!
//! The driver owns all simulation/UI semantics. This module only chooses input,
//! records it, and compares independent executions. Its RNG never touches a
//! game's RNG. A successful run is a robustness result, not native-engine parity.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Hash a borrowed serializable value without allocating a JSON tree or clone.
pub fn digest_serialized(value: &impl Serialize) -> Result<String> {
    use std::io::Write;
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut hash = HashWriter(Sha256::new());
    serde_json::to_writer(&mut hash, value)?;
    Ok(format!("{:x}", hash.0.finalize()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Observation {
    pub tick: u64,
    pub rng: [Option<u32>; 3],
    /// Full retained state, including host/UI continuation owners, not a visual hash.
    pub state_sha256: String,
}

#[derive(Debug, Clone)]
pub struct Candidate<C> {
    pub command: C,
    pub weight: u32,
}

/// Backend operations are bounded by `Limits::service_polls`. Implementations
/// must use a virtual input clock, never the elapsed wall time of the test.
pub trait Driver: Sized {
    type Command: Clone + Serialize + for<'a> Deserialize<'a> + PartialEq;
    fn observe(&self) -> Result<Observation>;
    fn candidates(&self) -> Result<Vec<Candidate<Self::Command>>>;
    fn execute(&mut self, command: &Self::Command, limits: &Limits) -> Result<()>;
    /// Optional read-only observations at completed simulation ticks.
    fn begin_tick_trace(&mut self) {}
    fn take_tick_trace(&mut self) -> Vec<Observation> {
        Vec::new()
    }
    fn fork(&self) -> Result<Self>;
    fn checkpoint(&self) -> Result<Vec<u8>>;
    fn restore(&self, bytes: &[u8]) -> Result<Self>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub commands: usize,
    pub completed_ticks: u64,
    pub service_polls: usize,
    pub wall_seconds: f64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            commands: 64,
            completed_ticks: 256,
            service_polls: 10_000,
            wall_seconds: 120.,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub seed: u64,
    /// Required progress, separate from the maximum in Limits.
    #[serde(default)]
    pub minimum_completed_ticks: u64,
    #[serde(default)]
    pub compare_ticks: bool,
    pub limits: Limits,
    pub compare_fork: bool,
    /// Replace the comparison branch with a real serialized/cold restore before
    /// this zero-based command. Other branches retain their warm resource state.
    pub cold_restore_at: Option<usize>,
    /// Additional zero-based command boundaries. Duplicate boundaries, including
    /// the legacy single boundary above, request only one restore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cold_restore_at_commands: Vec<usize>,
}
impl Default for Plan {
    fn default() -> Self {
        Self {
            seed: 1,
            minimum_completed_ticks: 0,
            compare_ticks: false,
            limits: Limits::default(),
            compare_fork: true,
            cold_restore_at: None,
            cold_restore_at_commands: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Command,
    Panic,
    Observation,
    Comparison,
    Restore,
    Candidates,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Failure {
    pub command_index: usize,
    pub kind: FailureKind,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step<C> {
    pub command: C,
    pub before: Observation,
    pub after: Option<Observation>,
    pub seconds: f64,
    #[serde(default)]
    pub ticks: Vec<Observation>,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Passed,
    Incomplete,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report<C> {
    pub schema: u32,
    pub plan: Plan,
    pub initial: Observation,
    pub steps: Vec<Step<C>>,
    pub failure: Option<Failure>,
    pub stop: String,
    pub seconds: f64,
    pub comparisons: usize,
    pub cold_restores: usize,
    #[serde(default)]
    pub tick_comparisons: usize,
    #[serde(default)]
    pub outcome: Outcome,
}
impl<C> Report<C> {
    pub fn completed_ticks(&self) -> u64 {
        self.steps
            .iter()
            .rev()
            .find_map(|s| s.after.as_ref())
            .map_or(0, |s| s.tick.saturating_sub(self.initial.tick))
    }
    pub fn passed(&self) -> bool {
        self.outcome == Outcome::Passed
    }
    pub fn commands(&self) -> Vec<C>
    where
        C: Clone,
    {
        self.steps.iter().map(|s| s.command.clone()).collect()
    }
}

/// Stable SplitMix64, local to the test driver. The concrete commands are still
/// saved: candidate enumeration can legitimately change in a later engine build.
pub struct Random(u64);
impl Random {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn choose<C>(&mut self, values: Vec<Candidate<C>>) -> Result<C> {
        let total: u64 = values.iter().map(|c| u64::from(c.weight)).sum();
        ensure!(total > 0, "no weighted commands available");
        // Rejection avoids modulo bias while remaining exactly reproducible.
        let threshold = total.wrapping_neg() % total;
        let mut draw = loop {
            let value = self.next_u64();
            if value >= threshold {
                break value % total;
            }
        };
        for candidate in values {
            if draw < u64::from(candidate.weight) {
                return Ok(candidate.command);
            }
            draw -= u64::from(candidate.weight);
        }
        unreachable!()
    }
}

/// `commands=None` explores; `Some` replays exact recorded input without RNG.
/// Errors after initialization become report data so one failing branch never
/// prevents the caller from preserving evidence or trying the next branch.
pub fn run<D: Driver>(
    driver: &mut D,
    plan: Plan,
    commands: Option<&[D::Command]>,
) -> Result<Report<D::Command>> {
    run_observed(driver, plan, commands, |_, _| Ok(()))
}

pub enum Event<'a, C> {
    /// Journal before entering game code, so a hung command remains replayable.
    Input {
        index: usize,
        command: &'a C,
        before: &'a Observation,
    },
    /// Sample images only after execution and comparison have completed.
    Completed {
        index: usize,
        after: &'a Observation,
    },
}

/// Optional read-only journal/checkpoint observer. It never receives mutable
/// gameplay; image sampling belongs to Completed, not Input.
pub fn run_observed<D: Driver>(
    driver: &mut D,
    plan: Plan,
    commands: Option<&[D::Command]>,
    mut observe: impl FnMut(&D, Event<'_, D::Command>) -> Result<()>,
) -> Result<Report<D::Command>> {
    ensure!(
        plan.limits.commands > 0
            && plan.limits.service_polls > 0
            && plan.limits.completed_ticks > 0
            && plan.minimum_completed_ticks <= plan.limits.completed_ticks
            && plan.limits.wall_seconds.is_finite()
            && plan.limits.wall_seconds > 0.,
        "invalid exploration limits"
    );
    let start = Instant::now();
    let restore_commands: std::collections::BTreeSet<_> = plan
        .cold_restore_at_commands
        .iter()
        .copied()
        .chain(plan.cold_restore_at)
        .collect();
    let initial = driver.observe()?;
    let mut report = Report {
        schema: 2,
        plan: plan.clone(),
        initial: initial.clone(),
        steps: vec![],
        failure: None,
        stop: "command_limit".into(),
        seconds: 0.,
        comparisons: 0,
        cold_restores: 0,
        tick_comparisons: 0,
        outcome: Outcome::Passed,
    };
    let mut shadow = if plan.compare_fork {
        Some(driver.fork()?)
    } else {
        None
    };
    if let Some(s) = &shadow {
        ensure!(
            s.observe()? == initial,
            "initial warm fork changed retained state"
        );
    }
    let mut random = Random::new(plan.seed);
    let mut before = initial;
    for index in 0..plan.limits.commands {
        if start.elapsed().as_secs_f64() > plan.limits.wall_seconds {
            report.stop = "wall_limit".into();
            break;
        }
        if before.tick.saturating_sub(report.initial.tick) >= plan.limits.completed_ticks {
            report.stop = "tick_limit".into();
            break;
        }
        if restore_commands.contains(&index) {
            match driver.checkpoint().and_then(|bytes| driver.restore(&bytes)) {
                Ok(restored) => match restored.observe() {
                    Ok(observed) if observed == before => {
                        shadow = Some(restored);
                        report.cold_restores += 1;
                    }
                    result => {
                        report.failure = Some(Failure {
                            command_index: index,
                            kind: FailureKind::Restore,
                            message: format!("cold restore changed retained state: {result:?}"),
                        });
                        break;
                    }
                },
                Err(e) => {
                    report.failure = Some(Failure {
                        command_index: index,
                        kind: FailureKind::Restore,
                        message: format!("{e:#}"),
                    });
                    break;
                }
            }
        }
        let command = if let Some(commands) = commands {
            let Some(command) = commands.get(index) else {
                report.stop = "replay_complete".into();
                break;
            };
            command.clone()
        } else {
            match driver.candidates().and_then(|c| random.choose(c)) {
                Ok(command) => command,
                Err(e) => {
                    report.failure = Some(Failure {
                        command_index: index,
                        kind: FailureKind::Candidates,
                        message: format!("{e:#}"),
                    });
                    break;
                }
            }
        };
        let timed = Instant::now();
        report.steps.push(Step {
            command: command.clone(),
            before: before.clone(),
            after: None,
            seconds: 0.,
            ticks: vec![],
        });
        if let Err(error) = observe(
            driver,
            Event::Input {
                index,
                command: &command,
                before: &before,
            },
        ) {
            report.failure = Some(Failure {
                command_index: index,
                kind: FailureKind::Observation,
                message: format!("input journal: {error:#}"),
            });
            break;
        }
        if plan.compare_ticks {
            driver.begin_tick_trace();
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            driver.execute(&command, &plan.limits)
        }));
        report.steps.last_mut().unwrap().seconds = timed.elapsed().as_secs_f64();
        if plan.compare_ticks {
            report.steps.last_mut().unwrap().ticks = driver.take_tick_trace();
        }
        match result {
            Ok(Ok(())) => (),
            Ok(Err(e)) => {
                report.failure = Some(Failure {
                    command_index: index,
                    kind: FailureKind::Command,
                    message: format!("{e:#}"),
                });
                break;
            }
            Err(payload) => {
                let message = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).into()))
                    .unwrap_or_else(|| "non-string panic payload".into());
                report.failure = Some(Failure {
                    command_index: index,
                    kind: FailureKind::Panic,
                    message,
                });
                break;
            }
        }
        let after = match driver.observe() {
            Ok(after) => after,
            Err(e) => {
                report.failure = Some(Failure {
                    command_index: index,
                    kind: FailureKind::Observation,
                    message: format!("{e:#}"),
                });
                break;
            }
        };
        report.steps.last_mut().unwrap().after = Some(after.clone());
        if plan.compare_ticks {
            let ticks = &report.steps.last().unwrap().ticks;
            let delta = after.tick.saturating_sub(before.tick);
            if ticks.len() as u64 != delta
                || ticks
                    .iter()
                    .enumerate()
                    .any(|(i, t)| t.tick != before.tick + i as u64 + 1)
            {
                report.failure = Some(Failure {
                    command_index: index,
                    kind: FailureKind::Observation,
                    message: format!("tick trace did not cover all {delta} completed ticks"),
                });
                break;
            }
        }

        if let Some(shadow) = &mut shadow {
            if plan.compare_ticks {
                shadow.begin_tick_trace();
            }
            let comparison = shadow
                .execute(&command, &plan.limits)
                .and_then(|_| shadow.observe());
            if plan.compare_ticks {
                let ticks = shadow.take_tick_trace();
                let expected = &report.steps.last().unwrap().ticks;
                if ticks != *expected {
                    let first = ticks
                        .iter()
                        .zip(expected)
                        .position(|(a, b)| a != b)
                        .unwrap_or(ticks.len().min(expected.len()));
                    report.failure = Some(Failure {
                        command_index: index,
                        kind: FailureKind::Comparison,
                        message: format!(
                            "completed tick trace differs at offset {first}: expected {:?}, got {:?}",
                            expected.get(first),
                            ticks.get(first)
                        ),
                    });
                    break;
                }
                report.tick_comparisons += ticks.len();
            }
            match comparison {
                Ok(observed) if observed == after => report.comparisons += 1,
                result => {
                    report.failure = Some(Failure {
                        command_index: index,
                        kind: FailureKind::Comparison,
                        message: format!("expected {after:?}, comparison returned {result:?}"),
                    });
                    break;
                }
            }
        }
        if let Err(error) = observe(
            driver,
            Event::Completed {
                index,
                after: &after,
            },
        ) {
            report.failure = Some(Failure {
                command_index: index,
                kind: FailureKind::Observation,
                message: format!("checkpoint observer: {error:#}"),
            });
            break;
        }
        before = after;
    }
    if report.failure.is_some() {
        report.stop = "failure".into();
        report.outcome = Outcome::Failed;
    } else if report.completed_ticks() < plan.minimum_completed_ticks
        || report.cold_restores != restore_commands.len()
        || (commands.is_some_and(|c| report.steps.len() < c.len()))
        || report.stop == "wall_limit"
    {
        report.outcome = Outcome::Incomplete;
    }
    report.seconds = start.elapsed().as_secs_f64();
    Ok(report)
}

/// Compare exact replay evidence while excluding wall-clock performance data.
pub fn verify_replay<C: PartialEq>(expected: &Report<C>, actual: &Report<C>) -> Result<()> {
    ensure!(
        expected.initial == actual.initial,
        "replay initial state differs"
    );
    ensure!(
        expected.steps.len() == actual.steps.len(),
        "replay command count differs"
    );
    for (index, (a, b)) in expected.steps.iter().zip(&actual.steps).enumerate() {
        ensure!(
            a.command == b.command
                && a.before == b.before
                && a.after == b.after
                && a.ticks == b.ticks,
            "replay differs at command {index}"
        );
    }
    ensure!(expected.failure == actual.failure, "replay failure differs");
    ensure!(expected.outcome == actual.outcome, "replay outcome differs");
    ensure!(
        expected.cold_restores == actual.cold_restores,
        "replay cold restore count differs"
    );
    Ok(())
}

/// Delta-debug commands; only an explicitly supplied failure predicate may
/// accept a reduction. Inapplicable/stale commands must not count as the bug.
pub fn minimize<C: Clone>(
    commands: &[C],
    max_attempts: usize,
    mut reproduces: impl FnMut(&[C]) -> Result<bool>,
) -> Result<(Vec<C>, usize)> {
    let mut best = commands.to_vec();
    let mut attempts = 0;
    let mut partitions = 2;
    while !best.is_empty() && attempts < max_attempts {
        let size = best.len().div_ceil(partitions);
        let mut reduced = false;
        for start in (0..best.len()).step_by(size.max(1)) {
            if attempts >= max_attempts {
                break;
            }
            let mut candidate = best.clone();
            candidate.drain(start..(start + size).min(best.len()));
            attempts += 1;
            if reproduces(&candidate)? {
                best = candidate;
                partitions = partitions.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
        }
        if !reduced {
            if partitions >= best.len() {
                break;
            }
            partitions = (partitions * 2).min(best.len());
        }
    }
    Ok((best, attempts))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct Counter {
        value: u64,
        broken_restore: bool,
    }
    impl Driver for Counter {
        type Command = u64;
        fn observe(&self) -> Result<Observation> {
            Ok(Observation {
                tick: self.value,
                rng: [Some(12), None, Some(34)],
                state_sha256: digest(&self.value.to_le_bytes()),
            })
        }
        fn candidates(&self) -> Result<Vec<Candidate<u64>>> {
            Ok(vec![
                Candidate {
                    command: 1,
                    weight: 3,
                },
                Candidate {
                    command: 2,
                    weight: 1,
                },
            ])
        }
        fn execute(&mut self, c: &u64, _: &Limits) -> Result<()> {
            ensure!(*c != 99, "reproducible command failure");
            self.value += c;
            Ok(())
        }
        fn fork(&self) -> Result<Self> {
            Ok(self.clone())
        }
        fn checkpoint(&self) -> Result<Vec<u8>> {
            Ok(self.value.to_le_bytes().to_vec())
        }
        fn restore(&self, bytes: &[u8]) -> Result<Self> {
            Ok(Self {
                value: u64::from_le_bytes(bytes.try_into()?) + u64::from(self.broken_restore),
                ..self.clone()
            })
        }
    }
    fn counter() -> Counter {
        Counter {
            value: 0,
            broken_restore: false,
        }
    }
    #[test]
    fn seeded_sequences_replay_and_compare_both_checkpoint_paths() -> Result<()> {
        for seed in 0..32 {
            let plan = Plan {
                seed,
                cold_restore_at: Some(3),
                ..Plan::default()
            };
            let a = run(&mut counter(), plan.clone(), None)?;
            let b = run(&mut counter(), plan, Some(&a.commands()))?;
            verify_replay(&a, &b)?;
            assert!(a.failure.is_none());
            assert_eq!(a.cold_restores, 1);
            assert_eq!(a.comparisons, a.steps.len());
        }
        Ok(())
    }
    #[test]
    fn preserves_failing_command_and_detects_incorrect_restore() -> Result<()> {
        let a = run(&mut counter(), Plan::default(), Some(&[1, 99, 2]))?;
        assert_eq!(a.commands(), [1, 99]);
        assert_eq!(a.failure.unwrap().kind, FailureKind::Command);
        let mut broken = Counter {
            broken_restore: true,
            ..counter()
        };
        let a = run(
            &mut broken,
            Plan {
                cold_restore_at: Some(1),
                ..Plan::default()
            },
            Some(&[1, 2]),
        )?;
        assert_eq!(a.failure.unwrap().kind, FailureKind::Restore);
        Ok(())
    }
    #[test]
    fn reduction_preserves_the_specified_bug_and_respects_budget() -> Result<()> {
        let (minimal, attempts) = minimize(&[1, 2, 99, 3, 4], 30, |v| Ok(v.contains(&99)))?;
        assert_eq!(minimal, [99]);
        assert!(attempts <= 30);
        Ok(())
    }
    #[test]
    fn journals_the_failing_command_before_entering_the_driver() -> Result<()> {
        let mut events = vec![];
        let report = run_observed(
            &mut counter(),
            Plan::default(),
            Some(&[1, 99]),
            |_, event| {
                match event {
                    Event::Input { index, command, .. } => {
                        events.push(format!("input:{index}:{command}"))
                    }
                    Event::Completed { index, .. } => events.push(format!("complete:{index}")),
                }
                Ok(())
            },
        )?;
        assert_eq!(events, ["input:0:1", "complete:0", "input:1:99"]);
        assert_eq!(report.failure.unwrap().kind, FailureKind::Command);
        Ok(())
    }
    #[test]
    fn weighting_rejects_empty_and_never_selects_zero_weight() -> Result<()> {
        let mut r = Random::new(2);
        assert!(r.choose::<u8>(vec![]).is_err());
        for _ in 0..100 {
            assert_eq!(
                r.choose(vec![
                    Candidate {
                        command: 0,
                        weight: 0
                    },
                    Candidate {
                        command: 1,
                        weight: 1
                    }
                ])?,
                1
            );
        }
        Ok(())
    }
    #[test]
    fn progress_is_required_and_an_unreached_restore_is_incomplete() -> Result<()> {
        let plan = Plan {
            minimum_completed_ticks: 10,
            ..Plan::default()
        };
        let short = run(&mut counter(), plan.clone(), Some(&[1, 2]))?;
        assert_eq!(short.outcome, Outcome::Incomplete);
        assert_eq!(short.completed_ticks(), 3);
        let enough = run(&mut counter(), plan, Some(&[10]))?;
        assert!(enough.passed());
        let missing_restore = run(
            &mut counter(),
            Plan {
                cold_restore_at: Some(2),
                ..Plan::default()
            },
            Some(&[1]),
        )?;
        assert_eq!(missing_restore.outcome, Outcome::Incomplete);
        let limited = run(
            &mut counter(),
            Plan {
                limits: Limits {
                    commands: 1,
                    ..Limits::default()
                },
                ..Plan::default()
            },
            Some(&[1, 2]),
        )?;
        assert_eq!(limited.outcome, Outcome::Incomplete);
        Ok(())
    }
    #[test]
    fn repeated_restore_schedule_is_deduplicated_and_requires_every_boundary() -> Result<()> {
        let plan = Plan {
            cold_restore_at: Some(1),
            cold_restore_at_commands: vec![3, 1, 3],
            ..Plan::default()
        };
        let report = run(&mut counter(), plan.clone(), Some(&[1, 2, 3, 4]))?;
        assert!(report.passed());
        assert_eq!(report.cold_restores, 2);
        assert_eq!(report.comparisons, 4);
        verify_replay(
            &report,
            &run(&mut counter(), plan.clone(), Some(&report.commands()))?,
        )?;
        let short = run(&mut counter(), plan, Some(&[1, 2]))?;
        assert_eq!(short.cold_restores, 1);
        assert_eq!(short.outcome, Outcome::Incomplete);
        // Old serialized plans remain accepted and acquire no extra field on
        // serialization unless a repeated schedule is actually requested.
        let old = serde_json::to_value(Plan::default())?;
        assert!(old.get("cold_restore_at_commands").is_none());
        let restored: Plan = serde_json::from_value(old)?;
        assert!(restored.cold_restore_at_commands.is_empty());
        Ok(())
    }
    #[test]
    fn an_error_before_a_requested_restore_never_counts_as_reaching_it() -> Result<()> {
        let report = run(
            &mut counter(),
            Plan {
                cold_restore_at_commands: vec![1, 3],
                ..Plan::default()
            },
            Some(&[1, 99, 2, 3]),
        )?;
        assert_eq!(report.cold_restores, 1);
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.failure.unwrap().kind, FailureKind::Command);
        Ok(())
    }
    #[test]
    fn tick_trace_catches_transient_divergence_despite_equal_command_endpoints() -> Result<()> {
        struct Transient {
            value: u64,
            shadow: bool,
            tracing: bool,
        }
        impl Driver for Transient {
            type Command = u64;
            fn observe(&self) -> Result<Observation> {
                counter_at(self.value)
            }
            fn candidates(&self) -> Result<Vec<Candidate<u64>>> {
                Ok(vec![Candidate {
                    command: 1,
                    weight: 1,
                }])
            }
            fn execute(&mut self, _: &u64, _: &Limits) -> Result<()> {
                self.value += 1;
                Ok(())
            }
            fn begin_tick_trace(&mut self) {
                self.tracing = true;
            }
            fn take_tick_trace(&mut self) -> Vec<Observation> {
                self.tracing = false;
                vec![counter_at(self.value + u64::from(self.shadow)).unwrap()]
            }
            fn fork(&self) -> Result<Self> {
                Ok(Self {
                    value: self.value,
                    shadow: true,
                    tracing: false,
                })
            }
            fn checkpoint(&self) -> Result<Vec<u8>> {
                Ok(vec![])
            }
            fn restore(&self, _: &[u8]) -> Result<Self> {
                self.fork()
            }
        }
        fn counter_at(tick: u64) -> Result<Observation> {
            Ok(Observation {
                tick,
                rng: [None; 3],
                state_sha256: digest(&tick.to_le_bytes()),
            })
        }
        let mut driver = Transient {
            value: 0,
            shadow: false,
            tracing: false,
        };
        let report = run(
            &mut driver,
            Plan {
                compare_ticks: true,
                ..Plan::default()
            },
            Some(&[1]),
        )?;
        assert_eq!(report.failure.unwrap().kind, FailureKind::Comparison);
        assert_eq!(report.tick_comparisons, 0);
        Ok(())
    }
}
