//! Real subprocess integration: isolated parallel workers, immutable sources,
//! first divergence, honest unsupported/evidence outcomes and exclusive outputs.
use sims_fixtures::{Artifact, ArtifactFormat, CaseManifest, Compression, Coverage, Store};
use sims_runtime::conformance::{
    Adapter, CaseSpec, Fixture, Scope, Status, Suite, runner::SuiteReport,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sims-conformance-cli-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entry(id: &str, adapter: Adapter, fixture: Option<Fixture>, required: bool) -> CaseSpec {
    CaseSpec {
        id: id.into(),
        adapter,
        fixture,
        required,
        timeout_seconds: 60,
        minimum_matched_ticks: 0,
        estimated_memory_mib: 16,
    }
}
fn manifest(store: &Store, id: &str, bytes: &[u8], contract: Option<&str>) {
    let blob = store.put_bytes(bytes, Compression::Gzip).unwrap();
    let mut case = CaseManifest::minimal_import(
        id,
        "CLI regression input",
        Artifact::self_contained(blob, ArtifactFormat::Json, 1),
        vec![],
    );
    if let Some(contract) = contract {
        case.coverage = Coverage::Component {
            scope: contract.into(),
            limitations: vec![
                "Original-derived component data; test may deliberately poison expectations".into(),
            ],
        };
    }
    store.publish_case(&case).unwrap();
}
fn execute(
    suite: &Suite,
    root: &Path,
    name: &str,
    extra: &[&str],
) -> (std::process::Output, PathBuf) {
    let input = root.join(format!("{name}.json"));
    fs::write(&input, serde_json::to_vec(suite).unwrap()).unwrap();
    let output = root.join(name);
    let result = Command::new(env!("CARGO_BIN_EXE_sims-conformance"))
        .args(["run", "--suite"])
        .arg(input)
        .args(["--jobs", "2", "--memory-mib", "32", "--output"])
        .arg(&output)
        .args(extra)
        .output()
        .unwrap();
    (result, output)
}

#[test]
fn parallel_workers_keep_unsupported_and_evidence_distinct_and_source_immutable() {
    let temp = Temp::new();
    let source = temp.0.join("source");
    let store = Store::create(&source).unwrap();
    manifest(&store, "observation", b"{}", None);
    let path = source.join("cases/observation/manifest.json");
    let before = fs::read(&path).unwrap();
    let suite = Suite {
        schema: 1,
        id: "original".into(),
        cases: vec![
            entry(
                "goth",
                Adapter::NativeHousehold {
                    house: "UserData/Houses/House05.iff".into(),
                immutable_content: None, checks: Default::default(),
                },
                None,
                true,
            ),
            entry(
                "photo",
                Adapter::EvidenceOnly {
                    reason: "not a checkpoint".into(),
                },
                Some(Fixture {
                    store: source,
                    case_id: "observation".into(),
                }),
                false,
            ),
        ],
    };
    let (result, output) = execute(&suite, &temp.0, "unsupported", &[]);
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: SuiteReport =
        serde_json::from_slice(&fs::read(output.join("suite-report.json")).unwrap()).unwrap();
    assert!(!report.success);
    assert_eq!(report.peak_admitted_workers, 2);
    assert_eq!(report.peak_admitted_memory_mib, 32);
    assert_eq!(report.cases[0].worker.result.status, Status::Unsupported);
    assert_eq!(report.cases[1].worker.result.status, Status::EvidenceOnly);
    assert_eq!(fs::read(path).unwrap(), before);
    Store::open(output.join("reports-store"))
        .unwrap()
        .verify_all()
        .unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn real_component_replay_reports_first_poisoned_difference_and_single_case_rerun() {
    let temp = Temp::new();
    let source = temp.0.join("source");
    let store = Store::create(&source).unwrap();
    let bytes = include_bytes!("../../sims-vm/tests/fixtures/saved-rng.json");
    manifest(&store, "good", bytes, Some("native-saved-rng-v1"));
    let mut bad: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    bad["cases"][0]["original"]["after"] = serde_json::json!(1);
    manifest(
        &store,
        "poisoned",
        &serde_json::to_vec(&bad).unwrap(),
        Some("native-saved-rng-v1"),
    );
    let suite = Suite {
        schema: 1,
        id: "components".into(),
        cases: ["good", "poisoned"]
            .into_iter()
            .map(|id| {
                entry(
                    id,
                    Adapter::SavedRng,
                    Some(Fixture {
                        store: source.clone(),
                        case_id: id.into(),
                    }),
                    true,
                )
            })
            .collect(),
    };
    let (result, output) = execute(&suite, &temp.0, "mixed", &[]);
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: SuiteReport =
        serde_json::from_slice(&fs::read(output.join("suite-report.json")).unwrap()).unwrap();
    assert_eq!(report.cases[0].worker.result.status, Status::Passed);
    let failed = &report.cases[1].worker.result;
    assert_eq!(failed.status, Status::Diverged);
    assert_eq!(failed.scope, Scope::OriginalComponent);
    assert!(failed.difference.is_some());
    assert_eq!(failed.completed_ticks, 0);
    assert!(!failed.artifacts.is_empty());
    for artifact in &failed.artifacts {
        assert!(output.join("cases/poisoned").join(artifact).exists());
    }
    let (result, output) = execute(&suite, &temp.0, "single", &["--case", "good"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: SuiteReport =
        serde_json::from_slice(&fs::read(output.join("suite-report.json")).unwrap()).unwrap();
    assert!(report.success);
    assert_eq!(report.cases.len(), 1);
}

#[test]
fn configuration_errors_do_not_create_output_or_run_workers() {
    let temp = Temp::new();
    let suite = Suite {
        schema: 1,
        id: "original".into(),
        cases: vec![entry(
            "goth",
            Adapter::NativeHousehold {
                house: "house".into(),
                immutable_content: None, checks: Default::default(),
            },
            None,
            true,
        )],
    };
    let (result, output) = execute(&suite, &temp.0, "unknown", &["--case", "absent"]);
    assert!(!result.status.success());
    assert!(!output.exists());
}

#[test]
fn parent_rejects_output_inside_a_reference_store_before_any_worker_files() {
    let temp = Temp::new();
    let source = temp.0.join("source");
    let store = Store::create(&source).unwrap();
    manifest(&store, "observation", b"{}", None);
    let suite = Suite {
        schema: 1,
        id: "audit".into(),
        cases: vec![entry(
            "evidence",
            Adapter::EvidenceOnly {
                reason: "phase evidence".into(),
            },
            Some(Fixture {
                store: source.clone(),
                case_id: "observation".into(),
            }),
            false,
        )],
    };
    let input = temp.0.join("suite.json");
    fs::write(&input, serde_json::to_vec(&suite).unwrap()).unwrap();
    let output = source.join("must-not-create");
    let result = Command::new(env!("CARGO_BIN_EXE_sims-conformance"))
        .args(["run", "--suite"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!output.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("inside a source fixture store"));
    assert_eq!(fs::read_dir(source.join("runs")).unwrap().count(), 0);
}

#[test]
fn valid_case_names_cannot_collide_with_suite_report_or_store_paths() {
    let temp = Temp::new();
    let suite = Suite {
        schema: 1,
        id: "namespaces".into(),
        cases: ["resolved-suite.json", "suite-report.json", "reports-store"]
            .into_iter()
            .map(|id| {
                entry(
                    id,
                    Adapter::NativeHousehold {
                        house: "unavailable".into(),
                immutable_content: None, checks: Default::default(),
                    },
                    None,
                    true,
                )
            })
            .collect(),
    };
    let (result, output) = execute(&suite, &temp.0, "namespaces", &[]);
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: SuiteReport =
        serde_json::from_slice(&fs::read(output.join("suite-report.json")).unwrap()).unwrap();
    assert_eq!(report.cases.len(), 3);
    assert!(
        report
            .cases
            .iter()
            .all(|c| c.worker.result.status == Status::Unsupported)
    );
    for case in &report.cases {
        assert!(
            output
                .join("cases")
                .join(&case.id)
                .join("result.json")
                .exists()
        );
    }
    Store::open(output.join("reports-store"))
        .unwrap()
        .verify_all()
        .unwrap();
}
