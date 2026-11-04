use super::*;
use sims_fixtures::{Artifact, Compression};
use std::sync::atomic::{AtomicU64, Ordering};

const SAVED: &[u8] = include_bytes!("../../../../sims-vm/tests/fixtures/saved-rng.json");
const CLOCK: &[u8] = include_bytes!("../../../../sims-vm/tests/fixtures/scheduler-clock-gameplay.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sims-component-{}-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn output(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir(&p).unwrap();
        p
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn case(store: &Store, id: &str, contract: &str, bytes: &[u8]) -> CaseManifest {
    let blob = store.put_bytes(bytes, Compression::Gzip).unwrap();
    let mut case = CaseManifest::minimal_import(id, id,
        Artifact::self_contained(blob, ArtifactFormat::Json, 1), vec![]);
    case.coverage = Coverage::Component { scope: contract.into(), limitations: vec!["component only; no household ticks".into()] };
    store.publish_case(&case).unwrap();
    case
}

fn files(root: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() { visit(root, &p, out); }
            else { out.insert(p.strip_prefix(root).unwrap().into(), sims_fixtures::digest(&fs::read(p).unwrap())); }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn stored_original_components_pass_without_source_store_writes() {
    let temp = Sandbox::new();
    let store = Store::create(temp.0.join("store")).unwrap();
    let rng = case(&store, "rng", SAVED_RNG_CONTRACT, SAVED);
    let clock = case(&store, "clock", SCHEDULER_CLOCK_CONTRACT, CLOCK);
    let before = files(store.root());
    for (case, adapter, count) in [(&rng, Adapter::SavedRng, 162), (&clock, Adapter::SchedulerClock, 239)] {
        let output = temp.output(&case.id);
        let report = run(&store, case, &adapter, &output).unwrap();
        assert_eq!(report.status, Status::Passed);
        assert_eq!(report.scope, Scope::OriginalComponent);
        assert_eq!(report.observations, count);
        assert_eq!((report.completed_ticks, report.matched_completed_ticks), (0, 0));
        assert!(report.difference.is_none());
        assert_eq!(report.artifacts.len(), 2);
        for path in report.artifacts { assert!(output.join(path).is_file()); }
    }
    assert_eq!(before, files(store.root()));
}

#[test]
fn changed_expected_field_is_preserved_as_real_first_divergence() {
    let temp = Sandbox::new();
    let store = Store::create(temp.0.join("store")).unwrap();
    for (name, bytes, contract, adapter, field, path) in [
        ("rng-negative", SAVED, SAVED_RNG_CONTRACT, Adapter::SavedRng, "/cases/0/original/after", "/cases/0/original/after"),
        ("clock-negative", CLOCK, SCHEDULER_CLOCK_CONTRACT, Adapter::SchedulerClock, "/cases/0/original/state/inputs/now", "/cases/0/original/state/inputs/now"),
    ] {
        let mut source: Value = serde_json::from_slice(bytes).unwrap();
        let actual = source.pointer(field).unwrap().clone();
        *source.pointer_mut(field).unwrap() = json!(actual.as_u64().unwrap()+1);
        let expected = source.pointer(field).unwrap().clone();
        let c = case(&store, name, contract, &serde_json::to_vec(&source).unwrap());
        let output = temp.output(name);
        let report = run(&store, &c, &adapter, &output).unwrap();
        assert_eq!(report.status, Status::Diverged);
        let difference = report.difference.unwrap();
        assert_eq!(difference.path, path);
        assert_eq!(difference.expected, Some(expected));
        assert_eq!(difference.actual, Some(actual));
        let retained: Value = serde_json::from_slice(&fs::read(output.join("component-comparison.json")).unwrap()).unwrap();
        assert_eq!(retained["status"], "diverged");
        assert_eq!(retained["difference"]["path"], path);
        assert_eq!(retained["completed_ticks"], 0);
    }
}

#[test]
fn unknown_contract_formats_and_mutation_paths_reject() {
    let temp = Sandbox::new();
    let store = Store::create(temp.0.join("store")).unwrap();
    let mut c = case(&store, "rng", SAVED_RNG_CONTRACT, SAVED);
    c.coverage = Coverage::Component { scope: "guessed-v2".into(), limitations: vec![] };
    assert!(run(&store, &c, &Adapter::SavedRng, &temp.output("unknown")).is_err());
    c.coverage = Coverage::Component { scope: SAVED_RNG_CONTRACT.into(), limitations: vec![] };
    c.source.format = ArtifactFormat::MessagePack;
    assert!(run(&store, &c, &Adapter::SavedRng, &temp.output("format")).is_err());
    let mut source: Value = serde_json::from_slice(CLOCK).unwrap();
    source["cases"][0]["input"]["actions"]["ApplicationState"] = json!([{"mutation":{"fields":{"misspelled":1}}}]);
    assert!(scheduler_clock(&serde_json::to_vec(&source).unwrap()).is_err());
    let oversized = vec![b' '; MAX_SOURCE_BYTES as usize+1];
    let too_big = case(&store, "too-big", SAVED_RNG_CONTRACT, &oversized);
    assert!(run(&store, &too_big, &Adapter::SavedRng, &temp.output("oversized")).is_err());
}

#[test]
fn worker_cannot_write_into_source_store_or_replace_existing_output() {
    let temp = Sandbox::new();
    let store = Store::create(temp.0.join("store")).unwrap();
    let c = case(&store, "rng", SAVED_RNG_CONTRACT, SAVED);
    assert!(run(&store, &c, &Adapter::SavedRng, store.root()).is_err());
    let output = temp.output("existing");
    fs::write(output.join("component-actual.json"), b"keep").unwrap();
    assert!(run(&store, &c, &Adapter::SavedRng, &output).is_err());
    assert_eq!(fs::read(output.join("component-actual.json")).unwrap(), b"keep");
}
