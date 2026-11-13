//! Replay a qualified native household in-process. Qualification is immutable
//! reference data, not a command or an unchecked "passed" flag. Full original
//! source requalification remains the independently exported acceptance gate.
use crate::{
    fixture_replay::{Difference, first_difference},
    original_session::{
        Progress, Saved, Session,
        completed_replay::{self, ProgramInputs},
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_fixtures::{ArtifactFormat, BlobRef, CaseManifest, ScheduleStep, Store};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

pub mod package;
pub mod sequence;
pub const FORMAT: &str = "sims-runtime/qualified-native-replay-v1+msgpack";
pub const CONTRACT: &str = "goth-completed88448-88449-qualified-v1";
pub const QUALIFICATION: &str = "dfc926bcbcd3403657c06516a13f67f5d2181023ccb7bb67d07d130f34c74fdd";
pub const SOURCE_INVENTORY: &str =
    "9a7f5b3f782f514e29820c8585353a78f0b38e5c9a7137e76dc85dcf1741714c";
pub const SOURCE_REPORT: &str = "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Checks {
    #[default]
    Replay,
    Cold,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub tick: u64,
    pub state: BlobRef,
    pub native_trace: BlobRef,
    pub reference_trace: BlobRef,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColdCut {
    pub state: BlobRef,
    pub suffix_events: usize,
    pub boundary: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: u32,
    pub contract: String,
    pub qualification: BlobRef,
    pub source_inventory: BlobRef,
    pub qualification_witnesses: BTreeMap<String, BlobRef>,
    pub program_config: String,
    pub program_files: BTreeMap<String, BlobRef>,
    pub baseline: BlobRef,
    pub frames: Vec<Frame>,
    pub cold_cuts: Vec<ColdCut>,
}
impl Request {
    pub fn dependencies(&self) -> Vec<BlobRef> {
        let mut refs = vec![
            self.qualification.clone(),
            self.source_inventory.clone(),
            self.baseline.clone(),
        ];
        refs.extend(self.qualification_witnesses.values().cloned());
        refs.extend(self.program_files.values().cloned());
        for f in &self.frames {
            refs.extend([
                f.state.clone(),
                f.native_trace.clone(),
                f.reference_trace.clone(),
            ]);
        }
        refs.extend(self.cold_cuts.iter().map(|c| c.state.clone()));
        refs.sort();
        refs.dedup();
        refs
    }
}
fn raw_json(store: &Store, reference: &BlobRef) -> Result<Value> {
    Ok(serde_json::from_slice(
        &store.read_bytes(reference, 32 << 20)?,
    )?)
}
fn validate_qualification(request: &Request, proof: &Value) -> Result<()> {
    ensure!(
        request.schema == 1 && request.contract == CONTRACT,
        "unsupported native replay contract"
    );
    ensure!(
        request.qualification.sha256 == QUALIFICATION
            && request.source_inventory.sha256 == SOURCE_INVENTORY,
        "native source qualification identity differs"
    );
    ensure!(
        proof["status"] == "strict_native_completed_tick_passed"
            && proof["complete_native_Session_admitted"] == true
            && proof["native_parity_ticks"] == 1
            && proof["source_report_sha256"] == SOURCE_REPORT
            && proof["independent_checks"] == 79
            && proof["unknown_gameplay_owners"] == json!([]),
        "native source qualification is incomplete"
    );
    ensure!(
        proof["baseline_sha256"] == request.baseline.sha256
            && proof["completed_boundary"] == json!([88448, 88449])
            && request.frames.len() == 1
            && request.frames[0].tick == 88449
            && proof["endpoint_sha256"] == request.frames[0].state.sha256,
        "native reference boundaries differ from qualification"
    );
    ensure!(
        proof["resolved_owner_rows"]
            .as_array()
            .is_some_and(|r| r.len() == 366)
            && proof["resolved_Resident_native_values"]
                .as_array()
                .is_some_and(|r| r.len() == 42),
        "native owner coverage is incomplete"
    );
    let witnesses = proof["witnesses"]
        .as_object()
        .context("native qualification witnesses absent")?;
    ensure!(
        witnesses.len() == request.qualification_witnesses.len()
            && witnesses.iter().all(|(name, r)| request
                .qualification_witnesses
                .get(name)
                .is_some_and(|b| r["sha256"] == b.sha256)),
        "native qualification witness closure differs"
    );
    let cuts = proof["independent_suspended_resumes"]
        .as_array()
        .context("qualified suspended resumes absent")?;
    ensure!(
        cuts.len() == 6
            && request.cold_cuts.len() == 6
            && cuts
                .iter()
                .zip(&request.cold_cuts)
                .all(|(a, b)| a["checkpoint"]["sha256"] == b.state.sha256
                    && a["checkpoint"]["bytes"] == b.state.bytes
                    && a["cut"] == b.boundary
                    && a["trace_suffix_events"] == b.suffix_events),
        "suspended source checkpoints differ from qualification"
    );
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ReplayReport {
    pub schema: u32,
    pub contract: String,
    pub qualification_sha256: String,
    pub checks: Checks,
    pub matched_completed_ticks: u64,
    pub completed_ticks: u64,
    pub difference: Option<Difference>,
    pub boundaries: Vec<Value>,
    pub cold_restores: Vec<Value>,
    pub timings_ms: BTreeMap<String, f64>,
    pub preparation_ms: BTreeMap<String, f64>,
    pub qualification_reused: bool,
    pub source_requalification_executed: bool,
    /// Failed subject state is diagnostic evidence, never a promoted reference.
    #[serde(skip)]
    pub diagnostic_state: Option<Box<Saved>>,
}
pub struct Prepared {
    request: Request,
    qualification: Value,
    initial: Saved,
    expected: Vec<Saved>,
    index: sims_content::Index,
    program: sims_vm::Program,
    headline_program: Option<sims_vm::Program>,
    native: Vec<Value>,
    traces: Vec<Vec<sims_vm::Trace>>,
    cuts: Vec<Saved>,
    source_root: PathBuf,
    immutable_root: PathBuf,
    pub preparation_ms: BTreeMap<String, f64>,
}
fn elapsed(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
fn measured<T>(
    timings: &mut BTreeMap<String, f64>,
    label: &str,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let start = Instant::now();
    let value = f();
    *timings.entry(label.into()).or_default() += elapsed(start);
    value
}

impl Prepared {
    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn qualification(&self) -> &Value {
        &self.qualification
    }

    pub fn load(
        store: &Store,
        case: &CaseManifest,
        immutable: &Path,
        house: &str,
    ) -> Result<Option<Self>> {
        let start = Instant::now();
        let mut timings = BTreeMap::new();
        let artifacts: Vec<_> = case
            .artifacts
            .iter()
            .filter(|a| a.format == ArtifactFormat::Other(FORMAT.into()))
            .collect();
        if artifacts.is_empty() {
            return Ok(None);
        }
        ensure!(
            artifacts.len() == 1 && artifacts[0].schema == 1,
            "ambiguous/unknown native replay request"
        );
        let request: Request = store.get_value(&artifacts[0].blob)?;
        let mut declared = artifacts[0].dependencies.clone();
        declared.sort();
        declared.dedup();
        ensure!(
            declared == request.dependencies(),
            "native replay dependency closure differs"
        );
        let qualification = raw_json(store, &request.qualification)?;
        validate_qualification(&request, &qualification)?;
        ensure!(
            case.schedule == vec![ScheduleStep::WaitCompletedTicks { count: 1 }],
            "native replay schedule differs from observed no-command interval"
        );
        let inventory: sims_fixtures::archive::ArchiveInventory =
            store.get_value(&request.source_inventory)?;
        inventory.validate()?;
        for (name, blob) in &request.program_files {
            ensure!(
                inventory
                    .files
                    .iter()
                    .any(|r| r.relative_path == *name && r.blob == *blob),
                "Program input not in qualified original source inventory: {name}"
            );
        }
        for (i, frame) in request.frames.iter().enumerate() {
            ensure!(
                inventory.files.iter().any(|r| r
                    .relative_path
                    .ends_with("/exact-cold-second/native-bytecode.json")
                    && r.blob == frame.native_trace)
                    && inventory.files.iter().any(|r| r.relative_path.ends_with(
                        "/goth-label-owners-20261029-first/session-first/rust-trace.json"
                    ) && r.blob == frame.reference_trace),
                "frame{i} trace source is not qualified"
            );
        }
        let mut values = BTreeMap::new();
        for (name, blob) in &request.program_files {
            values.insert(name.clone(), raw_json(store, blob)?);
        }
        let config = values
            .get(&request.program_config)
            .context("native Program config missing")?;
        let inputs = ProgramInputs::read(config, |name| {
            values
                .get(name)
                .cloned()
                .with_context(|| format!("qualified Program source absent: {name}"))
        })?;
        let initial: Saved = store.get_value(&request.baseline)?;
        ensure!(
            initial.source == house && initial.simulation.state.vm.tick == 88448,
            "native baseline house/tick differs"
        );
        let expected: Vec<Saved> = request
            .frames
            .iter()
            .map(|f| store.get_value(&f.state))
            .collect::<Result<_>>()?;
        let cuts: Vec<Saved> = request
            .cold_cuts
            .iter()
            .map(|f| store.get_value(&f.state))
            .collect::<Result<_>>()?;
        let native = request
            .frames
            .iter()
            .map(|f| raw_json(store, &f.native_trace))
            .collect::<Result<Vec<_>>>()?;
        let traces = request
            .frames
            .iter()
            .map(|f| {
                Ok(serde_json::from_value(raw_json(
                    store,
                    &f.reference_trace,
                )?)?)
            })
            .collect::<Result<Vec<Vec<sims_vm::Trace>>>>()?;
        timings.insert("fixture_decode_and_qualification".into(), elapsed(start));
        let index = measured(&mut timings, "immutable_index", || {
            sims_content::Index::scan(immutable)
        })?;
        completed_replay::require_immutable_index(&index)?;
        let (program, aliases, _, _) = measured(&mut timings, "independent_program_link", || {
            inputs.link(
                &initial
                    .managed
                    .as_ref()
                    .context("owned baseline absent")?
                    .owned,
                &index,
            )
        })?;
        ensure!(
            initial.view.resource_aliases.as_ref() == Some(&aliases),
            "independently rebuilt native provider aliases differ"
        );
        let headline_program = if initial.headline_catalog
            || expected
                .iter()
                .chain(cuts.iter())
                .any(|s| s.headline_catalog)
        {
            let mut p = program.clone();
            sims_content::vm::load_headline_catalog(
                &initial.managed.as_ref().unwrap().owned.mount(&index)?,
                &mut p,
            )?;
            Some(p)
        } else {
            None
        };
        Ok(Some(Self {
            request,
            qualification,
            initial,
            expected,
            index,
            program,
            headline_program,
            native,
            traces,
            cuts,
            source_root: store.root().canonicalize()?,
            immutable_root: immutable.canonicalize()?,
            preparation_ms: timings,
        }))
    }
    fn restore(&self, saved: Saved) -> Result<Session> {
        let program = if saved.headline_catalog {
            self.headline_program
                .as_ref()
                .context("qualified headline Program absent")?
        } else {
            &self.program
        };
        Session::restore_completed_owned_with_program(&self.index, saved, program.clone())
    }
    pub fn replay(&self, checks: Checks) -> Result<ReplayReport> {
        let mut report = ReplayReport {
            schema: 1,
            contract: self.request.contract.clone(),
            qualification_sha256: self.request.qualification.sha256.clone(),
            checks,
            matched_completed_ticks: 0,
            completed_ticks: 0,
            difference: None,
            boundaries: vec![],
            cold_restores: vec![],
            timings_ms: BTreeMap::new(),
            preparation_ms: self.preparation_ms.clone(),
            qualification_reused: true,
            source_requalification_executed: false,
            diagnostic_state: None,
        };
        let mut session = measured(&mut report.timings_ms, "baseline_restore", || {
            self.restore(self.initial.clone())
        })?;
        let baseline = measured(&mut report.timings_ms, "capture", || session.save_managed())?;
        report.difference = measured(&mut report.timings_ms, "state_compare", || {
            compare_saved(&self.initial, &baseline, "/baseline")
        })?;
        if report.difference.is_some() {
            report.diagnostic_state = Some(Box::new(baseline));
            return Ok(report);
        }
        let mut whole_trace = vec![];
        for (i, (frame, expected)) in self.request.frames.iter().zip(&self.expected).enumerate() {
            let mut trace = vec![];
            let before = session.save().simulation.state.vm.tick;
            let progress = measured(&mut report.timings_ms, "simulation", || {
                completed_replay::advance_with_retained_clock(&mut session, &mut trace, None)
            });
            let progress = match progress {
                Ok(p) => p,
                Err(error) => {
                    report.difference = Some(Difference {
                        path: format!("/boundaries/{}/service", frame.tick),
                        expected: Some(json!("completed_tick")),
                        actual: Some(json!({"error":format!("{error:#}"),"trace":trace})),
                    });
                    report.diagnostic_state = Some(Box::new(
                        session.save_managed().unwrap_or_else(|_| session.save()),
                    ));
                    return Ok(report);
                }
            };
            let actual = measured(&mut report.timings_ms, "capture", || session.save_managed())?;
            if progress != Progress::Tick
                || actual.simulation.state.vm.tick != before + 1
                || actual.simulation.state.vm.tick != frame.tick
            {
                report.difference = Some(Difference {
                    path: format!("/boundaries/{}/tick", frame.tick),
                    expected: Some(json!(frame.tick)),
                    actual: Some(
                        json!({"tick":actual.simulation.state.vm.tick,"progress":format!("{progress:?}")}),
                    ),
                });
                report.diagnostic_state = Some(Box::new(actual));
                break;
            }
            report.completed_ticks += 1;
            report.difference = measured(&mut report.timings_ms, "state_compare", || {
                compare_saved(expected, &actual, &format!("/boundaries/{}", frame.tick))
            })?;
            let trace_result = measured(&mut report.timings_ms, "trace_compare", || {
                completed_replay::trace::compare(&self.native[i], &trace, before)
            });
            if report.difference.is_none() {
                if let Err(error) = trace_result {
                    report.difference = Some(Difference {
                        path: format!("/boundaries/{}/native_trace", frame.tick),
                        expected: Some(self.native[i].clone()),
                        actual: Some(json!({"error":error.to_string(),"trace":trace})),
                    });
                }
            }
            if report.difference.is_none() && trace != self.traces[i] {
                report.difference = first_difference(
                    &serde_json::to_value(&self.traces[i])?,
                    &serde_json::to_value(&trace)?,
                );
            }
            whole_trace.extend(trace);
            report.boundaries.push(json!({"tick":frame.tick,"expected_state":frame.state,"matched":report.difference.is_none(),"rng":{"global":actual.simulation.state.vm.rng_state,"lot":actual.simulation.state.vm.lot_rng_state,"CRT":actual.simulation.state.vm.crt_rng_state}}));
            if report.difference.is_some() {
                report.diagnostic_state = Some(Box::new(actual));
                break;
            }
            report.matched_completed_ticks += 1;
        }
        if checks == Checks::Cold && report.difference.is_none() {
            let expected = self.expected.last().context("native endpoint absent")?;
            let endpoint = measured(&mut report.timings_ms, "endpoint_restore", || {
                self.restore(expected.clone())
            })?;
            ensure!(
                compare_saved(expected, &endpoint.save_managed()?, "/cold/endpoint")?.is_none(),
                "endpoint cold roundtrip differs"
            );
            report
                .cold_restores
                .push(json!({"boundary":"endpoint","matched":true}));
            for (cut, saved) in self.request.cold_cuts.iter().zip(&self.cuts) {
                let mut resumed = measured(&mut report.timings_ms, "suspended_restore", || {
                    self.restore(saved.clone())
                })?;
                let mut trace = vec![];
                let progress = measured(&mut report.timings_ms, "suspended_simulation", || {
                    completed_replay::advance_with_retained_clock(&mut resumed, &mut trace, None)
                })?;
                let actual =
                    measured(&mut report.timings_ms, "capture", || resumed.save_managed())?;
                report.difference = measured(&mut report.timings_ms, "state_compare", || {
                    compare_saved(expected, &actual, "/cold/suspended")
                })?;
                if progress != Progress::Tick
                    || trace.len() != cut.suffix_events
                    || !whole_trace.ends_with(&trace)
                {
                    report.difference = Some(Difference {
                        path: "/cold/trace_suffix".into(),
                        expected: Some(json!({"events":cut.suffix_events})),
                        actual: Some(serde_json::to_value(&trace)?),
                    });
                }
                report.cold_restores.push(json!({"boundary":cut.boundary,"matched":report.difference.is_none(),"trace_suffix_events":trace.len()}));
                if report.difference.is_some() {
                    report.diagnostic_state = Some(Box::new(actual));
                    break;
                }
            }
        }
        Ok(report)
    }
}

fn compare_saved(expected: &Saved, actual: &Saved, prefix: &str) -> Result<Option<Difference>> {
    let a = postcard::to_allocvec(expected)?;
    let b = postcard::to_allocvec(actual)?;
    if a == b {
        return Ok(None);
    }
    if let Some(mut d) = first_difference(
        &serde_json::to_value(expected)?,
        &serde_json::to_value(actual)?,
    ) {
        d.path = format!("{prefix}{}", d.path);
        return Ok(Some(d));
    }
    Ok(Some(Difference {
        path: format!("{prefix}/@binary_float_bits"),
        expected: Some(
            json!({"sha256":sims_fixtures::digest(&a),"first_byte":a.iter().zip(&b).position(|(a,b)|a!=b)}),
        ),
        actual: Some(json!({"sha256":sims_fixtures::digest(&b)})),
    }))
}

pub fn run(
    store: &Store,
    case: &CaseManifest,
    immutable: &Path,
    house: &str,
    checks: Checks,
    output: &Path,
) -> Result<super::AdapterResult> {
    ensure!(
        !output
            .canonicalize()?
            .starts_with(immutable.canonicalize()?)
            && !output
                .canonicalize()?
                .starts_with(store.root().canonicalize()?),
        "native worker output overlaps immutable/source data"
    );
    let Some(prepared) = Prepared::load(store, case, immutable, house)? else {
        return Ok(super::AdapterResult::stopped(
            super::Status::Unsupported,
            super::Scope::OriginalHousehold,
            "fixture has no qualified native replay request",
        ));
    };
    let report = prepared.replay(checks)?;
    let mut artifacts = vec!["native-replay-report.json".into()];
    if let Some(actual) = &report.diagnostic_state {
        let diagnostics = Store::create(output.join("diagnostic-state"))?;
        let reference = diagnostics.put_value(actual)?;
        fs::write(
            output.join("diagnostic-state.json"),
            serde_json::to_vec_pretty(
                &json!({"state":reference,"qualified_resumable_reference":false,"scope":"failed subject state; diagnostic only"}),
            )?,
        )?;
        artifacts.push("diagnostic-state.json".into());
    }
    fs::write(
        output.join("native-replay-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let passed = report.difference.is_none()
        && report.matched_completed_ticks == prepared.request.frames.len() as u64;
    Ok(super::AdapterResult {
        status: if passed {
            super::Status::Passed
        } else {
            super::Status::Diverged
        },
        scope: super::Scope::OriginalHousehold,
        contract: prepared.request.contract.clone(),
        observations: report.boundaries.len() as u64 + 1,
        completed_ticks: report.completed_ticks,
        matched_completed_ticks: if passed {
            report.matched_completed_ticks
        } else {
            0
        },
        difference: report.difference,
        detail: Some(format!(
            "Source-qualified native mapped-state replay; qualification {} reused; checks {checks:?}; full source requalification remains separate.",
            prepared.request.qualification.sha256
        )),
        artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_is_explicit_and_unknown_contracts_cannot_claim_native_ticks() {
        let b = BlobRef {
            sha256: "0".repeat(64),
            bytes: 0,
        };
        let r = Request {
            schema: 1,
            contract: "unknown".into(),
            qualification: b.clone(),
            source_inventory: b.clone(),
            qualification_witnesses: BTreeMap::new(),
            program_config: String::new(),
            program_files: BTreeMap::new(),
            baseline: b,
            frames: vec![],
            cold_cuts: vec![],
        };
        assert!(
            validate_qualification(&r, &json!({"status":"strict_native_completed_tick_passed"}))
                .is_err()
        );
        assert_eq!(
            serde_json::from_value::<Checks>(json!("replay")).unwrap(),
            Checks::Replay
        );
        assert!(serde_json::from_value::<Checks>(json!("skip_checks")).is_err());
    }
    #[test]
    #[ignore = "private qualified native replay fixture and immutable assets"]
    fn qualified_replay_rejects_rng_trace_missing_proof_and_dependency_mutants() -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let store = Store::open(root.join("fixtures/private/player-action-regressions-20261025"))?;
        let case = store.load_case("native-goth-qualified-replay-20261029-v1")?;
        let immutable = root.join("cache/managed-independent-20261022-third/immutable-assets");
        let mut bad = case.clone();
        bad.artifacts[0].dependencies.pop();
        assert!(Prepared::load(&store, &bad, &immutable, "UserData/Houses/House05.iff").is_err());
        let mut prepared =
            Prepared::load(&store, &case, &immutable, "UserData/Houses/House05.iff")?.unwrap();
        let proof_before = serde_json::to_vec(&prepared.qualification)?;
        let mut missing_caller = prepared.restore(prepared.initial.clone())?;
        missing_caller.host.presentation.clock = None;
        let missing_before = postcard::to_allocvec(&missing_caller.save())?;
        let mut trace = vec![];
        let mut observed = false;
        let mut observer = |_: &Session| {
            observed = true;
            Ok(())
        };
        let error = completed_replay::advance_with_retained_clock(
            &mut missing_caller,
            &mut trace,
            Some(&mut observer),
        )
        .unwrap_err();
        assert!(error.to_string().contains("retained elapsed caller"));
        assert!(!observed && trace.is_empty());
        assert_eq!(
            postcard::to_allocvec(&missing_caller.save())?,
            missing_before
        );
        let mut r = prepared.request.clone();
        r.qualification_witnesses.pop_first();
        assert!(validate_qualification(&r, &prepared.qualification).is_err());
        let mut r = prepared.request.clone();
        r.cold_cuts.pop();
        assert!(validate_qualification(&r, &prepared.qualification).is_err());
        let positive = prepared.replay(Checks::Replay)?;
        assert!(positive.difference.is_none());
        assert_eq!(positive.matched_completed_ticks, 1);
        let old = prepared.expected[0].simulation.state.vm.rng_state;
        prepared.expected[0].simulation.state.vm.rng_state ^= 1;
        let bad = prepared.replay(Checks::Replay)?;
        assert!(
            bad.difference
                .as_ref()
                .is_some_and(|d| d.path.contains("rng_state"))
        );
        assert_eq!(bad.matched_completed_ticks, 0);
        assert!(bad.diagnostic_state.is_some());
        prepared.expected[0].simulation.state.vm.rng_state = old;
        prepared.native[0][0]["primitive_result_raw"] = 1.into();
        let bad = prepared.replay(Checks::Replay)?;
        assert!(
            bad.difference
                .as_ref()
                .is_some_and(|d| d.path.ends_with("native_trace"))
        );
        assert_eq!(bad.matched_completed_ticks, 0);
        assert_eq!(serde_json::to_vec(&prepared.qualification)?, proof_before);
        Ok(())
    }
}
