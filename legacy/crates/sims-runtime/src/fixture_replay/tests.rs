use super::*;
use sims_fixtures::{
    Artifact, Coverage, Origin,
    trace::{TraceOptions, TraceRecordMeta, TraceWriter},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    store: Store,
    case: CaseManifest,
    request: Request,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Fixture {
    fn new(schedule: Vec<ScheduleStep>, rows: Vec<(Boundary, Value)>) -> Result<Self> {
        let directory = std::env::temp_dir().canonicalize()?.join(format!(
            "sims-runtime-fixture-replay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let store = Store::create(&directory)?;
        let seed = store.put_value(&serde_json::json!({"tick":10}))?;
        let mut writer = TraceWriter::new(&store, TraceOptions::default())?;
        for (boundary, value) in &rows {
            writer.append(
                value,
                TraceRecordMeta {
                    kind: RecordKind::Observation,
                    boundary: boundary.clone(),
                    source: None,
                },
            )?;
        }
        let trace = writer.finish()?;
        let trace_blob = store.put_value(&trace)?;
        let mut case = CaseManifest::minimal_import(
            "test-replay",
            "Authored runner protocol case",
            Artifact::self_contained(seed.clone(), ArtifactFormat::MessagePack, 1),
            vec![],
        );
        case.provenance.origin = Origin::Authored;
        case.coverage = Coverage::Component {
            scope: "Replay protocol only, not native parity".into(),
            limitations: vec![],
        };
        case.artifacts.push(Artifact::declared(
            trace_blob.clone(),
            ArtifactFormat::TraceIndex,
            1,
            trace.blob_refs(),
        ));
        case.references.push(Reference {
            label: "start".into(),
            kind: ReferenceKind::Resumable,
            artifact: Artifact::self_contained(seed, ArtifactFormat::MessagePack, 1),
            resume_contract: Some(Fake::RESUME_CONTRACT.into()),
            boundary: rows[0].0.clone(),
        });
        case.schedule = schedule;
        let request = Request {
            schema: 1,
            case_id: case.id.clone(),
            initial_reference: "start".into(),
            observation_contract: Fake::OBSERVATION_CONTRACT.into(),
            expected_trace: trace_blob,
            max_observations: 128,
            max_polls: 1000,
            max_completed_ticks: 64,
        };
        Ok(Self {
            directory,
            store,
            case,
            request,
        })
    }
    fn prepare(&self) -> Result<Prepared> {
        self.store.publish_case(&self.case)?;
        Prepared::load::<Fake>(&self.store, self.request.clone())
    }
}

#[derive(Clone)]
enum Action {
    Tick,
    Partial,
    HiddenTick,
    Jump,
    Service,
    Failure,
}
struct Fake {
    tick: u64,
    rng: u32,
    mismatch_at: Option<u64>,
    actions: VecDeque<Action>,
}
impl Fake {
    fn new(actions: impl IntoIterator<Item = Action>) -> Self {
        Self {
            tick: 10,
            rng: 7,
            mismatch_at: None,
            actions: actions.into_iter().collect(),
        }
    }
}
impl Driver for Fake {
    const RESUME_CONTRACT: &'static str = "authored-protocol-state-v1";
    const OBSERVATION_CONTRACT: &'static str = "authored-protocol-observation-v1";
    fn tick(&self) -> u64 {
        self.tick
    }
    fn observe(&self) -> Result<Value> {
        Ok(serde_json::json!({"tick":self.tick,"rng":self.rng}))
    }
    fn input(&mut self, input: &InputEvent) -> Result<Boundary> {
        if input.name == "hidden_tick" {
            self.tick += 1;
        } else {
            ensure!(input.name == "noop", "unknown input");
        }
        Ok(Boundary::Event {
            name: input.name.clone(),
            tick: Some(self.tick),
        })
    }
    fn poll(&mut self) -> Result<Poll> {
        match self.actions.pop_front().context("no fake actions left")? {
            Action::Tick => {
                self.tick += 1;
                if self.mismatch_at == Some(self.tick) {
                    self.rng += 1;
                }
                Ok(Poll::CompletedTick)
            }
            Action::Partial => Ok(Poll::PartialPass),
            Action::HiddenTick => {
                self.tick += 1;
                Ok(Poll::PartialPass)
            }
            Action::Jump => {
                self.tick += 2;
                Ok(Poll::CompletedTick)
            }
            Action::Service => Ok(Poll::Boundary(service())),
            Action::Failure => {
                self.rng = 99;
                anyhow::bail!("actual missing service")
            }
        }
    }
}
fn start() -> Boundary {
    Boundary::Event {
        name: "start".into(),
        tick: Some(10),
    }
}
fn service() -> Boundary {
    Boundary::Phase {
        name: "service".into(),
        tick: Some(10),
    }
}
fn value(tick: u64) -> Value {
    serde_json::json!({"tick":tick,"rng":7})
}
fn rows(count: u64) -> Vec<(Boundary, Value)> {
    std::iter::once((start(), value(10)))
        .chain((1..=count).map(|n| (Boundary::CompletedTick { tick: 10 + n }, value(10 + n))))
        .collect()
}
fn wait(count: u64) -> Vec<ScheduleStep> {
    vec![ScheduleStep::WaitCompletedTicks { count }]
}

#[test]
fn thirty_two_completed_ticks_exclude_partial_passes_and_baseline() -> Result<()> {
    let mut expected = rows(32);
    expected[0].0 = Boundary::CompletedTick { tick: 10 };
    let f = Fixture::new(wait(32), expected)?;
    let prepared = f.prepare()?;
    let actions = (0..32).flat_map(|_| [Action::Partial, Action::Partial, Action::Tick]);
    let result = prepared.run(|_| Ok(Fake::new(actions)))?;
    assert_eq!(result.report.outcome, Outcome::Agreement);
    assert_eq!(result.report.compared_observations, 33);
    assert_eq!(result.report.completed_ticks, 32);
    assert_eq!(result.report.matched_completed_ticks, 32);
    assert_eq!(result.report.polls, 96);
    assert_eq!(result.driver.unwrap().tick, 42);
    Ok(())
}

#[test]
fn first_divergence_stops_and_retains_actual_failed_driver() -> Result<()> {
    let f = Fixture::new(wait(32), rows(32))?;
    let result = f.prepare()?.run(|_| {
        let mut d = Fake::new((0..32).map(|_| Action::Tick));
        d.mismatch_at = Some(18);
        Ok(d)
    })?;
    assert_eq!(result.report.outcome, Outcome::Diverged);
    assert_eq!(result.report.completed_ticks, 8);
    assert_eq!(result.report.matched_completed_ticks, 7);
    assert_eq!(result.report.difference.unwrap().path, "/rng");
    assert_eq!(result.report.actual_at_stop.unwrap()["rng"], 8);
    assert_eq!(result.driver.unwrap().tick, 18);
    Ok(())
}

#[test]
fn observation_is_not_accepted_as_a_checkpoint() -> Result<()> {
    let mut f = Fixture::new(wait(1), rows(1))?;
    f.case.references[0].kind = ReferenceKind::Observed;
    f.case.references[0].resume_contract = None;
    assert!(
        f.prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("not a qualified resumable")
    );
    Ok(())
}

#[test]
fn wrong_resume_contract_and_ambiguous_labels_are_rejected() -> Result<()> {
    let mut f = Fixture::new(wait(1), rows(1))?;
    f.case.references[0].resume_contract = Some("other".into());
    assert!(
        f.prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("resume contract")
    );
    let mut f = Fixture::new(wait(1), rows(1))?;
    f.case.references.push(f.case.references[0].clone());
    assert!(f.prepare().err().unwrap().to_string().contains("ambiguous"));
    Ok(())
}

#[test]
fn corrupt_reference_and_undeclared_trace_fail_before_runtime_restore() -> Result<()> {
    let f = Fixture::new(wait(1), rows(1))?;
    f.store.publish_case(&f.case)?;
    let path = f.store.blob_path(&f.request.expected_trace)?;
    let mut bytes = std::fs::read(&path)?;
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(path, bytes)?;
    assert!(Prepared::load::<Fake>(&f.store, f.request.clone()).is_err());
    let mut f = Fixture::new(wait(1), rows(1))?;
    f.request.expected_trace = f.case.source.blob.clone();
    assert!(
        f.prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("not a declared")
    );
    Ok(())
}

#[test]
fn reference_tick_gaps_are_not_converted_into_waits() -> Result<()> {
    let mut expected = rows(2);
    expected[2] = (Boundary::CompletedTick { tick: 13 }, value(13));
    let f = Fixture::new(wait(2), expected)?;
    assert!(f.prepare().err().unwrap().to_string().contains("tick gap"));
    Ok(())
}

#[test]
fn backend_clock_misreporting_is_incomplete_with_state_retained() -> Result<()> {
    for action in [Action::HiddenTick, Action::Jump] {
        let f = Fixture::new(wait(1), rows(1))?;
        let result = f.prepare()?.run(|_| Ok(Fake::new([action])))?;
        assert_eq!(result.report.outcome, Outcome::Incomplete);
        assert_eq!(result.report.completed_ticks, 0);
        assert!(result.report.stop.is_some());
        assert!(result.driver.unwrap().tick > 10);
    }
    Ok(())
}

#[test]
fn input_may_not_secretly_advance_time() -> Result<()> {
    let f = Fixture::new(
        vec![ScheduleStep::Input {
            input: InputEvent {
                name: "hidden_tick".into(),
                payload: Value::Null,
            },
        }],
        rows(1),
    )?;
    let result = f.prepare()?.run(|_| Ok(Fake::new([])))?;
    assert_eq!(result.report.outcome, Outcome::Incomplete);
    assert_eq!(result.report.completed_ticks, 0);
    assert!(result.report.stop.unwrap().contains("input advanced time"));
    assert_eq!(result.driver.unwrap().tick, 11);
    Ok(())
}

#[test]
fn missing_schedule_and_exhausted_poll_bound_are_incomplete() -> Result<()> {
    let f = Fixture::new(vec![], rows(1))?;
    let result = f.prepare()?.run(|_| Ok(Fake::new([Action::Tick])))?;
    assert_eq!(result.report.outcome, Outcome::Incomplete);
    assert!(result.report.stop.unwrap().contains("schedule ended"));
    let mut f = Fixture::new(wait(1), rows(1))?;
    f.request.max_polls = 2;
    let result = f
        .prepare()?
        .run(|_| Ok(Fake::new([Action::Partial, Action::Partial, Action::Tick])))?;
    assert_eq!(result.report.outcome, Outcome::Incomplete);
    assert_eq!(result.report.polls, 2);
    assert_eq!(result.driver.unwrap().tick, 10);
    Ok(())
}

#[test]
fn unsupported_service_is_retained_without_acknowledgement() -> Result<()> {
    let f = Fixture::new(wait(1), rows(1))?;
    let result = f
        .prepare()?
        .run(|_| Ok(Fake::new([Action::Service, Action::Tick])))?;
    assert_eq!(result.report.outcome, Outcome::Incomplete);
    assert_eq!(result.report.boundary, service());
    assert_eq!(result.driver.unwrap().actions.len(), 1);
    let f = Fixture::new(wait(1), rows(1))?;
    let result = f.prepare()?.run(|_| Ok(Fake::new([Action::Failure])))?;
    assert_eq!(result.report.actual_at_stop.unwrap()["rng"], 99);
    assert_eq!(result.report.stop.unwrap(), "actual missing service");
    Ok(())
}

#[test]
fn explicit_service_boundary_and_input_do_not_count_as_completed_ticks() -> Result<()> {
    let expected = vec![
        (start(), value(10)),
        (service(), value(10)),
        (
            Boundary::Event {
                name: "noop".into(),
                tick: Some(10),
            },
            value(10),
        ),
    ];
    let f = Fixture::new(
        vec![
            ScheduleStep::WaitBoundary {
                boundary: service(),
            },
            ScheduleStep::Input {
                input: InputEvent {
                    name: "noop".into(),
                    payload: Value::Null,
                },
            },
        ],
        expected,
    )?;
    let result = f
        .prepare()?
        .run(|_| Ok(Fake::new([Action::Partial, Action::Service])))?;
    assert_eq!(result.report.outcome, Outcome::Agreement);
    assert_eq!(result.report.completed_ticks, 0);
    assert_eq!(result.report.matched_completed_ticks, 0);
    Ok(())
}

#[test]
fn restore_failure_does_not_report_a_running_driver_or_tick() -> Result<()> {
    let f = Fixture::new(wait(1), rows(1))?;
    let result: Execution<Fake> = f
        .prepare()?
        .run(|_| anyhow::bail!("missing actual start owner"))?;
    assert_eq!(result.report.outcome, Outcome::Incomplete);
    assert_eq!(result.report.completed_ticks, 0);
    assert!(result.driver.is_none());
    assert!(
        result
            .report
            .stop
            .unwrap()
            .contains("missing actual start owner")
    );
    Ok(())
}

#[test]
fn first_difference_preserves_missing_null_array_and_escaped_key_distinctions() {
    let d = first_difference(&serde_json::json!({"a/b~":null}), &serde_json::json!({})).unwrap();
    assert_eq!(d.path, "/a~1b~0");
    assert_eq!(d.expected, Some(Value::Null));
    assert_eq!(d.actual, None);
    assert_eq!(
        first_difference(&serde_json::json!([1, 2]), &serde_json::json!([1]))
            .unwrap()
            .path,
        "/1"
    );
}

#[test]
#[ignore = "requires the private imported Newbie original-observation corpus"]
fn imported_newbie_observations_are_not_misrepresented_as_a_resumable_runtime() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let store = Store::open(root.join("fixtures/private/corpus"))?;
    let case = store.load_case("newbie-native-observations-v1")?;
    let trace = case
        .artifacts
        .iter()
        .find(|a| a.format == ArtifactFormat::TraceIndex)
        .context("imported trace absent")?;
    let request = Request {
        schema: 1,
        case_id: case.id.clone(),
        initial_reference: "Original observed baseline".into(),
        observation_contract: SessionDriver::OBSERVATION_CONTRACT.into(),
        expected_trace: trace.blob.clone(),
        max_observations: 33,
        max_polls: 100_000,
        max_completed_ticks: 32,
    };
    let error = Prepared::load::<SessionDriver>(&store, request)
        .err()
        .context("observed source was incorrectly accepted as resumable")?;
    ensure!(
        error
            .to_string()
            .contains("not a qualified resumable state"),
        "wrong initial boundary: {error:#}"
    );
    eprintln!("actual Newbie source refused before any runtime restore: {error:#}");
    Ok(())
}
