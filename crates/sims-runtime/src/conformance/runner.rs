//! Bounded subprocess orchestration. Suite files select compiled adapters,
//! never shell commands from fixture provenance.
use super::*;
use anyhow::{Context, Result, ensure};
use sims_fixtures::{Artifact, ArtifactFormat, CaseManifest, Coverage, RunReport, Store};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const MAX_JSON: u64 = 8 << 20;
const MAX_LOG: u64 = 4 << 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerRequest {
    pub schema: u32,
    pub case: CaseSpec,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerReport {
    pub schema: u32,
    pub id: String,
    pub fixture_manifest_sha256: Option<String>,
    pub result: AdapterResult,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaseReport {
    pub id: String,
    pub required: bool,
    pub elapsed_ms: u128,
    pub worker_exit: Option<i32>,
    pub worker: WorkerReport,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SuiteReport {
    pub schema: u32,
    pub suite: String,
    pub suite_sha256: String,
    pub runner_sha256: String,
    pub jobs: usize,
    pub estimated_memory_budget_mib: u32,
    pub peak_admitted_workers: usize,
    pub peak_admitted_memory_mib: u32,
    pub success: bool,
    pub counts: BTreeMap<String, usize>,
    pub cases: Vec<CaseReport>,
    pub qualification: String,
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&read_small(path)?)?)
}

fn read_small(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_JSON, "JSON input exceeds 8 MiB");
    Ok(bytes)
}

fn file_digest(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn write_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn scope(adapter: &Adapter) -> Scope {
    match adapter {
        Adapter::SavedRng | Adapter::SchedulerClock | Adapter::CompletedStacks | Adapter::CompletedAnimations | Adapter::CompletedGroups | Adapter::CompletedPersonWorld | Adapter::CompletedReentry | Adapter::CompletedSkeletons | Adapter::CompletedSkin | Adapter::CompletedResidents | Adapter::CompletedPrototypes | Adapter::CompletedAppearances { .. } | Adapter::CompletedOrdinary { .. } => Scope::OriginalComponent,
        Adapter::ManagedSession { .. } => Scope::RustContinuation,
        Adapter::NativeHousehold { .. } => Scope::OriginalHousehold,
        Adapter::EvidenceOnly { .. } => Scope::ObservationOnly,
    }
}

pub fn validate(suite: &Suite, jobs: usize, memory_mib: u32) -> Result<()> {
    ensure!(suite.schema == 1, "unsupported suite schema");
    sims_fixtures::model::validate_id(&suite.id)?;
    ensure!((1..=8).contains(&jobs), "jobs must be 1..8");
    ensure!(
        (16..=32768).contains(&memory_mib),
        "invalid estimated memory budget"
    );
    ensure!(
        !suite.cases.is_empty() && suite.cases.len() <= 10_000,
        "invalid case count"
    );
    let mut ids = BTreeSet::new();
    for case in &suite.cases {
        sims_fixtures::model::validate_id(&case.id)?;
        ensure!(ids.insert(&case.id), "duplicate suite case id {}", case.id);
        ensure!(
            (1..=3600).contains(&case.timeout_seconds),
            "invalid timeout"
        );
        ensure!(
            (16..=memory_mib).contains(&case.estimated_memory_mib),
            "case exceeds estimated memory budget: {}",
            case.id
        );
        match &case.adapter {
            Adapter::NativeHousehold { house, .. } => {
                ensure!(!house.is_empty(), "native house missing");
                ensure!(
                    case.required,
                    "native household cases cannot silently be optional"
                );
            }
            Adapter::EvidenceOnly { reason } => {
                ensure!(
                    !case.required && !reason.is_empty(),
                    "evidence-only case cannot be required"
                );
                ensure!(case.fixture.is_some(), "evidence-only case has no evidence");
            }
            _ => {
                ensure!(
                    case.required,
                    "executable cases cannot silently be optional"
                );
                ensure!(case.fixture.is_some(), "executable case has no fixture");
            }
        }
        if matches!(
            case.adapter,
            Adapter::SavedRng | Adapter::SchedulerClock | Adapter::CompletedStacks | Adapter::CompletedAnimations | Adapter::CompletedGroups | Adapter::CompletedPersonWorld | Adapter::CompletedReentry | Adapter::CompletedSkeletons | Adapter::CompletedSkin | Adapter::CompletedResidents | Adapter::CompletedPrototypes | Adapter::EvidenceOnly { .. }
        ) {
            ensure!(
                case.minimum_matched_ticks == 0,
                "component/evidence cases cannot claim completed-tick coverage"
            );
        }
        if let Some(fixture) = &case.fixture {
            sims_fixtures::model::validate_id(&fixture.case_id)?;
        }
    }
    Ok(())
}

/// Unsupported native import remains visible; it is never adapted by loading an
/// original starting save or substituting a Rust-produced starting checkpoint.
pub fn run_worker(request: WorkerRequest, output: &Path) -> Result<WorkerReport> {
    ensure!(request.schema == 1, "unsupported worker schema");
    let case = &request.case;
    if let Some(fixture) = &case.fixture {
        if let Ok(source) = fixture.store.canonicalize() {
            ensure!(
                !output.canonicalize()?.starts_with(source),
                "worker output is inside the source fixture store"
            );
        }
    }
    let mut manifest_sha256 = None;
    let result = (|| -> Result<AdapterResult> {
        let loaded = if let Some(fixture) = &case.fixture {
            let store = Store::open(&fixture.store)?;
            let manifest_path = fixture.store.join("cases").join(&fixture.case_id).join("manifest.json");
            let before = read_small(&manifest_path)?;
            let manifest = store.load_case(&fixture.case_id)?;
            ensure!(read_small(&manifest_path)? == before, "fixture manifest changed during validation");
            manifest_sha256 = Some(sims_fixtures::digest(&before));
            Some((store, manifest))
        } else { None };
        match &case.adapter {
            Adapter::NativeHousehold { house, immutable_content:Some(immutable), checks } => {
                let (store,manifest)=loaded.as_ref().context("qualified native fixture missing")?;
                super::native_household::run(store,manifest,immutable,house,*checks,output)
            }
            Adapter::NativeHousehold { house, .. } => Ok(AdapterResult::stopped(Status::Unsupported,Scope::OriginalHousehold,
                format!("{house}: a qualified native replay fixture and immutable content are required; observations alone are not resumable state"))),
            Adapter::EvidenceOnly { reason } => Ok(AdapterResult::stopped(
                Status::EvidenceOnly, Scope::ObservationOnly, reason,
            )),
            Adapter::CompletedAppearances { immutable_content } => {
                let (store,manifest)=loaded.as_ref().context("appearance fixture missing")?;
                super::completed_appearances::run(store,manifest,immutable_content,output)
            }
            Adapter::CompletedOrdinary { immutable_content } => {
                let (store,manifest)=loaded.as_ref().context("ordinary fixture missing")?;
                super::completed_ordinary::run(store,manifest,immutable_content,output)
            }
            Adapter::ManagedSession { immutable_content } => {
                let (store, manifest) = loaded.as_ref().context("managed fixture missing")?;
                super::session::run(store, manifest, immutable_content, output)
            }
            Adapter::SavedRng | Adapter::SchedulerClock | Adapter::CompletedStacks | Adapter::CompletedAnimations | Adapter::CompletedGroups | Adapter::CompletedPersonWorld | Adapter::CompletedReentry | Adapter::CompletedSkeletons | Adapter::CompletedSkin | Adapter::CompletedResidents | Adapter::CompletedPrototypes => {
                let (store, manifest) = loaded.as_ref().context("component fixture missing")?;
                if matches!(case.adapter, Adapter::CompletedStacks) {
                    super::completed_stacks::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedAnimations) {
                    super::completed_animations::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedGroups) {
                    super::completed_groups::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedPersonWorld) {
                    super::completed_person_world::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedReentry) {
                    super::completed_reentry::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedSkeletons) {
                    super::completed_skeletons::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedSkin) {
                    super::completed_skin::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedResidents) {
                    super::completed_residents::run(store, manifest, output)
                } else if matches!(case.adapter, Adapter::CompletedPrototypes) {
                    super::completed_prototypes::run(store, manifest, output)
                } else {
                    super::component::run(store, manifest, &case.adapter, output)
                }
            }
        }
    })().unwrap_or_else(|error| AdapterResult::stopped(Status::Error, scope(&case.adapter), format!("{error:#}")));
    let report = WorkerReport {
        schema: 1,
        id: case.id.clone(),
        fixture_manifest_sha256: manifest_sha256,
        result,
    };
    write_new(&output.join("result.json"), &report)?;
    Ok(report)
}

struct Running {
    index: usize,
    child: Child,
    started: Instant,
    directory: PathBuf,
}
impl Drop for Running {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn suite_success(cases: &[CaseReport]) -> bool {
    cases.iter().any(|c| c.required)
        && cases
            .iter()
            .filter(|c| c.required)
            .all(|c| c.worker.result.status == Status::Passed)
}

fn apply_minimum(result: &mut AdapterResult, minimum: u64) {
    if result.status == Status::Passed && result.matched_completed_ticks < minimum {
        result.status = Status::Incomplete;
        result.detail = Some(format!(
            "matched {} ticks, suite requires {}",
            result.matched_completed_ticks, minimum
        ));
    }
}

pub fn run_suite(
    path: &Path,
    selected: &[String],
    output: &Path,
    jobs: usize,
    memory_mib: u32,
) -> Result<SuiteReport> {
    let raw = read_small(path)?;
    let mut suite: Suite = serde_json::from_slice(&raw)?;
    validate(&suite, jobs, memory_mib)?;
    let base = path
        .canonicalize()?
        .parent()
        .context("suite directory missing")?
        .to_path_buf();
    for id in selected {
        ensure!(
            suite.cases.iter().any(|c| &c.id == id),
            "unknown selected case {id}"
        );
    }
    if !selected.is_empty() {
        suite.cases.retain(|c| selected.contains(&c.id));
    }
    for case in &mut suite.cases {
        if let Some(fixture) = &mut case.fixture {
            if fixture.store.is_relative() {
                fixture.store = base.join(&fixture.store);
            }
        }
        if let Adapter::NativeHousehold { immutable_content:Some(immutable_content), .. } = &mut case.adapter {
            if immutable_content.is_relative(){*immutable_content=base.join(&*immutable_content);}
        }
        if let Adapter::ManagedSession { immutable_content } | Adapter::CompletedAppearances { immutable_content } | Adapter::CompletedOrdinary { immutable_content } = &mut case.adapter {
            if immutable_content.is_relative() {
                *immutable_content = base.join(&*immutable_content);
            }
        }
    }
    let output_parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output_location = output_parent
        .canonicalize()?
        .join(output.file_name().context("output name missing")?);
    for case in &suite.cases {
        if let Adapter::NativeHousehold { immutable_content:Some(content), .. } = &case.adapter {
            if let Ok(source)=content.canonicalize(){ensure!(!output_location.starts_with(source),"suite output is inside immutable content");}
        }
        if let Some(fixture) = &case.fixture {
            if let Ok(source) = fixture.store.canonicalize() {
                ensure!(
                    !output_location.starts_with(source),
                    "suite output is inside a source fixture store"
                );
            }
        }
        if let Adapter::ManagedSession { immutable_content } | Adapter::CompletedAppearances { immutable_content } | Adapter::CompletedOrdinary { immutable_content } = &case.adapter {
            if let Ok(source) = immutable_content.canonicalize() {
                ensure!(
                    !output_location.starts_with(source),
                    "suite output is inside immutable content"
                );
            }
        }
    }
    fs::create_dir(output).context("output must be a new directory with an existing parent")?;
    let executable = std::env::current_exe()?;
    let runner_sha256 = file_digest(&executable)?;
    write_new(&output.join("resolved-suite.json"), &suite)?;
    fs::create_dir(output.join("cases"))?;
    let mut pending: VecDeque<usize> = (0..suite.cases.len()).collect();
    let mut running: Vec<Running> = vec![];
    let mut reports: BTreeMap<usize, CaseReport> = BTreeMap::new();
    let mut peak_workers = 0;
    let mut peak_memory = 0;
    while !pending.is_empty() || !running.is_empty() {
        let mut occupied: u32 = running
            .iter()
            .map(|r| suite.cases[r.index].estimated_memory_mib)
            .sum();
        while running.len() < jobs {
            let Some(position) = pending
                .iter()
                .position(|i| suite.cases[*i].estimated_memory_mib <= memory_mib - occupied)
            else {
                break;
            };
            let index = pending.remove(position).unwrap();
            let case = &suite.cases[index];
            let directory = output.join("cases").join(&case.id);
            fs::create_dir(&directory)?;
            let request = directory.join("request.json");
            write_new(
                &request,
                &WorkerRequest {
                    schema: 1,
                    case: case.clone(),
                },
            )?;
            let stdout = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("stdout.log"))?;
            let stderr = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("stderr.log"))?;
            let child = Command::new(&executable)
                .arg("worker")
                .arg("--request")
                .arg(&request)
                .arg("--output")
                .arg(&directory)
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(stderr)
                .spawn()?;
            occupied += case.estimated_memory_mib;
            running.push(Running {
                index,
                child,
                started: Instant::now(),
                directory,
            });
            peak_workers = peak_workers.max(running.len());
            peak_memory = peak_memory.max(occupied);
        }
        let mut index = 0;
        while index < running.len() {
            let job = &mut running[index];
            let case = &suite.cases[job.index];
            let exited = job.child.try_wait()?;
            let oversized_log = ["stdout.log", "stderr.log"].iter().any(|name| {
                fs::metadata(job.directory.join(name)).is_ok_and(|m| m.len() > MAX_LOG)
            });
            let timed_out = job.started.elapsed() >= Duration::from_secs(case.timeout_seconds);
            if exited.is_none() && !timed_out && !oversized_log {
                index += 1;
                continue;
            }
            let mut job = running.remove(index);
            let elapsed_ms = job.started.elapsed().as_millis();
            let worker_exit = exited.and_then(|s| s.code());
            if exited.is_none() {
                if let Err(error) = job.child.kill() {
                    ensure!(
                        job.child.try_wait()?.is_some(),
                        "owned worker could not be terminated: {error}"
                    );
                }
                job.child.wait()?;
            }
            let fallback = |status, detail| WorkerReport {
                schema: 1,
                id: case.id.clone(),
                fixture_manifest_sha256: None,
                result: AdapterResult::stopped(status, scope(&case.adapter), detail),
            };
            let mut worker = if timed_out || oversized_log {
                fallback(
                    if timed_out {
                        Status::TimedOut
                    } else {
                        Status::Error
                    },
                    if timed_out {
                        "worker deadline exceeded".into()
                    } else {
                        "worker log exceeded 4 MiB".into()
                    },
                )
            } else {
                match read_json::<WorkerReport>(&job.directory.join("result.json")) {
                    Ok(report)
                        if report.schema == 1
                            && report.id == case.id
                            && report.result.scope == scope(&case.adapter)
                            && exited.unwrap().success() =>
                    {
                        report
                    }
                    Ok(_) => fallback(
                        Status::Error,
                        "worker result/identity/scope/exit protocol mismatch".into(),
                    ),
                    Err(error) => fallback(
                        Status::Error,
                        format!("worker did not publish a valid result: {error:#}"),
                    ),
                }
            };
            apply_minimum(&mut worker.result, case.minimum_matched_ticks);
            if worker.result.artifacts.iter().any(|p| {
                p.is_absolute()
                    || p.components().any(|c| {
                        !matches!(
                            c,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    })
                    || !job.directory.join(p).canonicalize().is_ok_and(|p| {
                        job.directory
                            .canonicalize()
                            .is_ok_and(|root| p.starts_with(root))
                    })
            }) {
                worker.result.status = Status::Error;
                worker.result.detail =
                    Some("worker artifact is missing or outside its owned output directory".into());
            }
            let report = CaseReport {
                id: case.id.clone(),
                required: case.required,
                elapsed_ms,
                worker_exit,
                worker,
            };
            write_new(&job.directory.join("parent-result.json"), &report)?;
            eprintln!("{}: {:?}", report.id, report.worker.result.status);
            reports.insert(job.index, report);
            drop(job); // kills/waits only this owned worker if it is still alive
        }
        if !running.is_empty() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let cases: Vec<_> = reports.into_values().collect();
    let mut counts = BTreeMap::new();
    for case in &cases {
        *counts
            .entry(format!("{:?}", case.worker.result.status))
            .or_insert(0) += 1;
    }
    let report = SuiteReport { schema: 1, suite: suite.id, suite_sha256: sims_fixtures::digest(&raw), runner_sha256, jobs,
        estimated_memory_budget_mib: memory_mib, peak_admitted_workers: peak_workers,
        peak_admitted_memory_mib: peak_memory, success: suite_success(&cases), counts, cases,
        qualification: "Adapter-scoped results. Original components, Rust continuation, original households and evidence-only records are reported separately; required unsupported/incomplete cases prevent success. No automatic parity upgrade.".into() };
    write_new(&output.join("suite-report.json"), &report)?;
    // A small durable report store; source evidence and diagnostic artifact
    // locations are explicit metadata, not a claim of embedded resumable state.
    let store = Store::create(output.join("reports-store"))?;
    let blob = store.put_value(&report)?;
    let mut case = CaseManifest::minimal_import(&report.suite, "Headless conformance execution report",
        Artifact::self_contained(blob.clone(), ArtifactFormat::MessagePack, 1),
        vec!["Report metadata only; source case identities and worker-local diagnostic paths are retained explicitly. No native parity certification.".into()]);
    case.coverage = Coverage::Component { scope: "compiled-adapter conformance execution report".into(),
        limitations: vec!["Per-case coverage applies; relative diagnostic paths require the preserved run directory and are not embedded checkpoints in this report store.".into()] };
    store.publish_case(&case)?;
    store.record_run(&RunReport {
        schema: 1,
        id: "execution".into(),
        case_id: report.suite.clone(),
        outcome: if report.success {
            "selected required adapters agreed"
        } else {
            "required conformance remains unsatisfied"
        }
        .into(),
        notes: vec![report.qualification.clone()],
        artifacts: vec![blob],
    })?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn native() -> CaseSpec {
        CaseSpec {
            id: "goth".into(),
            adapter: Adapter::NativeHousehold {
                house: "UserData/Houses/House05.iff".into(),
                immutable_content: None, checks: Default::default(),
            },
            fixture: None,
            required: true,
            timeout_seconds: 10,
            minimum_matched_ticks: 0,
            estimated_memory_mib: 16,
        }
    }
    #[test]
    fn rejects_duplicate_ids_resource_overcommit_and_optional_executable_cases() {
        let mut suite = Suite {
            schema: 1,
            id: "original".into(),
            cases: vec![native(), native()],
        };
        assert!(validate(&suite, 2, 32).is_err());
        suite.cases.pop();
        suite.cases[0].estimated_memory_mib = 64;
        assert!(validate(&suite, 2, 32).is_err());
        suite.cases[0].estimated_memory_mib = 16;
        suite.cases[0].adapter = Adapter::SavedRng;
        suite.cases[0].required = false;
        assert!(validate(&suite, 2, 32).is_err());
    }
    #[test]
    fn required_unsupported_is_not_success_and_evidence_does_not_pass() {
        let report = |required, status| CaseReport {
            id: "case".into(),
            required,
            elapsed_ms: 0,
            worker_exit: Some(0),
            worker: WorkerReport {
                schema: 1,
                id: "case".into(),
                fixture_manifest_sha256: None,
                result: AdapterResult::stopped(status, Scope::OriginalHousehold, "unavailable"),
            },
        };
        assert!(!suite_success(&[report(true, Status::Unsupported)]));
        assert!(!suite_success(&[report(false, Status::EvidenceOnly)]));
        assert!(suite_success(&[
            report(true, Status::Passed),
            report(false, Status::EvidenceOnly)
        ]));
    }
    #[test]
    fn shorter_agreement_cannot_meet_a_larger_tick_requirement() {
        let mut result =
            AdapterResult::stopped(Status::Passed, Scope::RustContinuation, "adapter agreed");
        result.matched_completed_ticks = 32;
        apply_minimum(&mut result, 256);
        assert_eq!(result.status, Status::Incomplete);
        assert_eq!(result.matched_completed_ticks, 32);
    }
    #[test]
    #[cfg(unix)]
    fn owned_worker_is_terminated_and_reaped_on_early_parent_exit() {
        let child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .unwrap();
        let began = Instant::now();
        let job = Running {
            index: 0,
            child,
            started: began,
            directory: PathBuf::new(),
        };
        drop(job);
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}
