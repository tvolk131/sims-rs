//! Execute declared fixture schedules against compiled runtime adapters.
//!
//! Observations are not checkpoints. Agreement under one adapter's declared
//! contract is not automatic certification of original-engine gameplay parity.
//! Evidence is fully verified before restoring or advancing a runtime.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_fixtures::{
    ArtifactFormat, BlobRef, Boundary, CaseManifest, InputEvent, Reference, ReferenceKind,
    ScheduleStep, Store,
    trace::{RecordKind, TraceIndex},
};

const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub case_id: String,
    pub initial_reference: String,
    pub observation_contract: String,
    /// Must identify a declared TraceIndex artifact in this case's closure.
    pub expected_trace: BlobRef,
    pub max_observations: usize,
    pub max_polls: usize,
    pub max_completed_ticks: u64,
}

/// Runtime adapters own every operation. Names/locators in fixture provenance
/// are never commands to execute, and a missing service is not acknowledged.
pub trait Driver {
    const RESUME_CONTRACT: &'static str;
    const OBSERVATION_CONTRACT: &'static str;
    fn tick(&self) -> u64;
    fn observe(&self) -> Result<Value>;
    fn input(&mut self, input: &InputEvent) -> Result<Boundary>;
    fn poll(&mut self) -> Result<Poll>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    CompletedTick,
    /// A registry partial pass does not complete a simulation tick.
    PartialPass,
    Boundary(Boundary),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Difference {
    /// RFC6901-style path; missing values remain distinct from JSON null.
    pub path: String,
    pub expected: Option<Value>,
    pub actual: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Agreement,
    Diverged,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub request: Request,
    pub case_source: BlobRef,
    pub initial_source: BlobRef,
    pub outcome: Outcome,
    pub compared_observations: usize,
    pub matched_completed_ticks: u64,
    pub completed_ticks: u64,
    pub polls: usize,
    pub schedule_step: Option<usize>,
    pub boundary: Boundary,
    pub difference: Option<Difference>,
    pub stop: Option<String>,
    pub actual_at_stop: Option<Value>,
    pub qualification: String,
}

/// Return the real backend even after a comparison/service failure, so callers
/// can retain the exact failed checkpoint rather than dropping its VM state.
pub struct Execution<D> {
    pub report: Report,
    pub driver: Option<D>,
}

struct Expected {
    boundary: Boundary,
    value: Value,
}

/// A verified, immutable input closure. Loading it drives no simulation.
pub struct Prepared {
    request: Request,
    case: CaseManifest,
    initial: Reference,
    expected: Vec<Expected>,
}

impl Prepared {
    pub fn load<D: Driver>(store: &Store, request: Request) -> Result<Self> {
        ensure!(
            request.schema == SCHEMA,
            "unsupported fixture replay schema"
        );
        ensure!(
            request.observation_contract == D::OBSERVATION_CONTRACT,
            "unsupported observation contract"
        );
        ensure!(
            (1..=100_000).contains(&request.max_observations)
                && (1..=10_000_000).contains(&request.max_polls)
                && request.max_completed_ticks <= 1_000_000,
            "invalid replay limits"
        );
        let case = store.load_case(&request.case_id)?;
        let references: Vec<_> = case
            .references
            .iter()
            .filter(|r| r.label == request.initial_reference)
            .collect();
        ensure!(
            references.len() == 1,
            "initial reference is absent or ambiguous"
        );
        let initial = references[0].clone();
        ensure!(
            initial.kind == ReferenceKind::Resumable,
            "initial reference is an observation, not a qualified resumable state"
        );
        ensure!(
            initial.resume_contract.as_deref() == Some(D::RESUME_CONTRACT),
            "unsupported starting-state resume contract"
        );
        ensure!(
            tick(&initial.boundary).is_some(),
            "starting boundary has no explicit simulation tick"
        );
        ensure!(
            case.artifacts
                .iter()
                .any(|a| a.blob == request.expected_trace
                    && a.format == ArtifactFormat::TraceIndex
                    && a.schema == SCHEMA),
            "expected trace is not a declared supported case artifact"
        );
        let trace: TraceIndex = store.get_value(&request.expected_trace)?;
        ensure!(
            trace.record_count <= request.max_observations as u64,
            "reference trace exceeds replay observation bound"
        );
        let mut expected = Vec::new();
        trace.reconstruct(store, |meta, value| {
            ensure!(
                meta.kind == RecordKind::Observation,
                "expected trace contains a non-observation"
            );
            ensure!(
                tick(&meta.boundary).is_some(),
                "expected boundary has no explicit tick"
            );
            expected.push(Expected {
                boundary: meta.boundary.clone(),
                value: value.clone(),
            });
            Ok(())
        })?;
        ensure!(
            expected
                .first()
                .is_some_and(|e| e.boundary == initial.boundary),
            "trace does not begin at the declared starting boundary"
        );
        // Every completed tick must be consecutive. Events/phases may share a
        // tick, but they cannot hide a gap or masquerade as completed updates.
        let mut current_tick = tick(&initial.boundary).unwrap();
        let mut completed = 0u64;
        for row in expected.iter().skip(1) {
            if let Boundary::CompletedTick { tick } = row.boundary {
                ensure!(
                    Some(tick) == current_tick.checked_add(1),
                    "reference completed-tick gap"
                );
                current_tick = tick;
                completed += 1;
            } else {
                ensure!(
                    tick(&row.boundary) == Some(current_tick),
                    "reference non-tick boundary advanced time"
                );
            }
        }
        ensure!(
            completed <= request.max_completed_ticks,
            "reference exceeds completed-tick bound"
        );
        Ok(Self {
            request,
            case,
            initial,
            expected,
        })
    }

    pub fn initial(&self) -> &Reference {
        &self.initial
    }
    pub fn case(&self) -> &CaseManifest {
        &self.case
    }

    /// Restore only after the complete reference trace verifies. A caller may
    /// supply another compiled implementation of the same declared contracts.
    pub fn run<D: Driver>(
        &self,
        restore: impl FnOnce(&Reference) -> Result<D>,
    ) -> Result<Execution<D>> {
        ensure!(
            self.request.observation_contract == D::OBSERVATION_CONTRACT
                && self.initial.resume_contract.as_deref() == Some(D::RESUME_CONTRACT),
            "runtime adapter differs from prepared contracts"
        );
        let mut report = Report {
            schema: SCHEMA, request: self.request.clone(), case_source: self.case.source.blob.clone(),
            initial_source: self.initial.artifact.blob.clone(), outcome: Outcome::Incomplete,
            compared_observations: 0, matched_completed_ticks: 0, completed_ticks: 0,
            polls: 0, schedule_step: None, boundary: self.initial.boundary.clone(),
            difference: None, stop: None, actual_at_stop: None,
            qualification: "Agreement under the declared adapter contract; original-engine parity and goal acceptance require separate evidence/coverage qualification.".into(),
        };
        let mut retained_driver = None;
        let attempt = (|| -> Result<()> {
            retained_driver =
                Some(restore(&self.initial).context("restore starting-state contract")?);
            let driver = retained_driver.as_mut().unwrap();
            ensure!(
                tick(&self.initial.boundary) == Some(driver.tick()),
                "restored starting tick differs"
            );
            if !self.compare(driver, self.initial.boundary.clone(), &mut report)? {
                return Ok(());
            }
            for (index, step) in self.case.schedule.iter().enumerate() {
                report.schedule_step = Some(index);
                match step {
                    ScheduleStep::Input { input } => {
                        let before = driver.tick();
                        let boundary = driver
                            .input(input)
                            .with_context(|| format!("input {}", input.name))?;
                        ensure!(
                            driver.tick() == before
                                && tick(&boundary) == Some(before)
                                && !matches!(boundary, Boundary::CompletedTick { .. }),
                            "input advanced time or claimed a completed tick"
                        );
                        if !self.compare(driver, boundary, &mut report)? {
                            return Ok(());
                        }
                    }
                    ScheduleStep::WaitCompletedTicks { count } => {
                        ensure!(
                            report
                                .completed_ticks
                                .checked_add(*count)
                                .is_some_and(|n| n <= self.request.max_completed_ticks),
                            "schedule exceeds completed-tick bound"
                        );
                        let target = report.completed_ticks + count;
                        while report.completed_ticks < target {
                            let boundary = self.poll(driver, &mut report)?;
                            if let Some(boundary) = boundary {
                                ensure!(
                                    matches!(boundary, Boundary::CompletedTick { .. }),
                                    "runtime requires explicit input/service at {boundary:?}"
                                );
                                if !self.compare(driver, boundary, &mut report)? {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    ScheduleStep::WaitBoundary { boundary: target } => {
                        ensure!(tick(target).is_some(), "scheduled boundary lacks a tick");
                        loop {
                            if let Some(boundary) = self.poll(driver, &mut report)? {
                                let reached = &boundary == target;
                                if !self.compare(driver, boundary, &mut report)? {
                                    return Ok(());
                                }
                                if reached {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            ensure!(
                report.compared_observations == self.expected.len(),
                "schedule ended before all reference observations"
            );
            report.outcome = Outcome::Agreement;
            Ok(())
        })();
        if let Err(error) = attempt {
            report.stop = Some(format!("{error:#}"));
        }
        if report.outcome != Outcome::Agreement {
            report.actual_at_stop = retained_driver.as_ref().and_then(|d| d.observe().ok());
        }
        Ok(Execution {
            report,
            driver: retained_driver,
        })
    }

    fn poll<D: Driver>(&self, driver: &mut D, report: &mut Report) -> Result<Option<Boundary>> {
        ensure!(
            report.polls < self.request.max_polls,
            "runtime poll bound exhausted"
        );
        let before = driver.tick();
        report.polls += 1;
        let progress = driver.poll()?;
        let after = driver.tick();
        match progress {
            Poll::CompletedTick => {
                ensure!(
                    Some(after) == before.checked_add(1),
                    "completed runtime tick did not advance exactly once"
                );
                report.completed_ticks += 1;
                ensure!(
                    report.completed_ticks <= self.request.max_completed_ticks,
                    "runtime completed-tick bound exceeded"
                );
                Ok(Some(Boundary::CompletedTick { tick: after }))
            }
            Poll::PartialPass => {
                ensure!(after == before, "partial pass advanced simulation time");
                Ok(None)
            }
            Poll::Boundary(boundary) => {
                ensure!(
                    after == before
                        && tick(&boundary) == Some(after)
                        && !matches!(boundary, Boundary::CompletedTick { .. }),
                    "service boundary advanced time or claimed a completed tick"
                );
                report.boundary = boundary.clone();
                Ok(Some(boundary))
            }
        }
    }

    fn compare<D: Driver>(
        &self,
        driver: &D,
        boundary: Boundary,
        report: &mut Report,
    ) -> Result<bool> {
        report.boundary = boundary.clone();
        let expected = self
            .expected
            .get(report.compared_observations)
            .context("runtime produced an unreferenced boundary")?;
        let actual = driver.observe()?;
        let difference = if expected.boundary != boundary {
            first_difference(
                &serde_json::to_value(&expected.boundary)?,
                &serde_json::to_value(&boundary)?,
            )
            .map(|mut d| {
                d.path = format!("/boundary{}", d.path);
                d
            })
        } else {
            first_difference(&expected.value, &actual)
        };
        if let Some(difference) = difference {
            report.difference = Some(difference);
            report.outcome = Outcome::Diverged;
            return Ok(false);
        }
        report.compared_observations += 1;
        if report.compared_observations > 1 && matches!(boundary, Boundary::CompletedTick { .. }) {
            report.matched_completed_ticks += 1;
        }
        Ok(true)
    }
}

fn tick(boundary: &Boundary) -> Option<u64> {
    match boundary {
        Boundary::CompletedTick { tick } => Some(*tick),
        Boundary::Phase { tick, .. } | Boundary::Event { tick, .. } => *tick,
        Boundary::Unspecified => None,
    }
}

pub fn first_difference(expected: &Value, actual: &Value) -> Option<Difference> {
    fn walk(a: Option<&Value>, b: Option<&Value>, path: String) -> Option<Difference> {
        if a == b {
            return None;
        }
        match (a, b) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
                for key in keys {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    if let Some(d) = walk(a.get(key), b.get(key), format!("{path}/{escaped}")) {
                        return Some(d);
                    }
                }
            }
            (Some(Value::Array(a)), Some(Value::Array(b))) => {
                for i in 0..a.len().max(b.len()) {
                    if let Some(d) = walk(a.get(i), b.get(i), format!("{path}/{i}")) {
                        return Some(d);
                    }
                }
            }
            _ => {
                return Some(Difference {
                    path,
                    expected: a.cloned(),
                    actual: b.cloned(),
                });
            }
        }
        None
    }
    walk(Some(expected), Some(actual), String::new())
}

/// The initial production adapter compares complete Rust retained-state bytes.
/// This contract is useful for continuation/cold-restore checks, and deliberately
/// does not pretend a Rust snapshot digest is a native gameplay projection.
pub struct SessionDriver<'a> {
    index: &'a sims_content::Index,
    pub session: crate::Session,
}
impl<'a> SessionDriver<'a> {
    pub fn restore(
        index: &'a sims_content::Index,
        store: &Store,
        reference: &Reference,
    ) -> Result<Self> {
        ensure!(
            reference.kind == ReferenceKind::Resumable
                && reference.resume_contract.as_deref() == Some(Self::RESUME_CONTRACT),
            "managed Session resume contract absent"
        );
        ensure!(
            reference.artifact.format == ArtifactFormat::MessagePack
                && reference.artifact.schema == SCHEMA,
            "managed checkpoint encoding differs"
        );
        let saved: crate::Saved = store.get_value(&reference.artifact.blob)?;
        ensure!(
            saved.version == 2 && saved.managed.is_some(),
            "fixture starting state must own a version2 managed Session"
        );
        let session = crate::Session::restore(index, saved)?;
        ensure!(
            tick(&reference.boundary) == Some(session.simulation.vm.tick),
            "managed checkpoint tick differs"
        );
        Ok(Self { index, session })
    }
}
impl Driver for SessionDriver<'_> {
    const RESUME_CONTRACT: &'static str = "sims-runtime/managed-session-v2";
    const OBSERVATION_CONTRACT: &'static str = "sims-runtime/retained-session-bytes-v1";
    fn tick(&self) -> u64 {
        self.session.simulation.vm.tick
    }
    fn observe(&self) -> Result<Value> {
        use sha2::{Digest, Sha256};
        let saved = self.session.save();
        let bytes = postcard::to_allocvec(&saved).context("encode exact retained Session state")?;
        Ok(serde_json::json!({"tick": self.tick(),
            "rng": {"global": self.session.simulation.vm.rng_state,
                "lot": self.session.simulation.vm.lot_rng_state,
                "crt": self.session.simulation.vm.crt_rng_state},
            "retained_session_sha256": format!("{:x}", Sha256::digest(&bytes))}))
    }
    fn input(&mut self, input: &InputEvent) -> Result<Boundary> {
        ensure!(
            input.name == "managed_cold_restore" && input.payload.is_null(),
            "unsupported Session replay input {}",
            input.name
        );
        let before = self.observe()?;
        self.session = crate::Session::restore(self.index, self.session.save_managed()?)?;
        ensure!(
            self.observe()? == before,
            "managed cold restore changed retained state"
        );
        Ok(Boundary::Event {
            name: input.name.clone(),
            tick: Some(self.tick()),
        })
    }
    fn poll(&mut self) -> Result<Poll> {
        use crate::Progress;
        Ok(match self.session.step()? {
            Progress::Tick => Poll::CompletedTick,
            Progress::PartialPass => Poll::PartialPass,
            other => Poll::Boundary(Boundary::Phase {
                name: format!("runtime_requires_{other:?}"),
                tick: Some(self.tick()),
            }),
        })
    }
}

#[cfg(test)]
mod tests;
