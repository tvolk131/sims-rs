//! Differential descriptor queries through the former per-resource reader and
//! the bounded scan reader. The exact Program/report/error survive failures.
use crate::{Index, Resource, assets, avatar, resource_scan::Scan};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    directory: PathBuf,
    index: Index,
}
impl Fixture {
    fn new(entries: Vec<(&str, Vec<u8>)>) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "sims-descriptor-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("payload");
        let mut bytes = vec![];
        let mut resources = vec![];
        for (locator, data) in entries {
            let offset = bytes.len() as u64;
            bytes.extend(&data);
            resources.push(Resource {
                locator: locator.into(),
                name: locator.split("::").last().unwrap().into(),
                source: source.clone(),
                offset,
                size: data.len() as u64,
                unpacked_size: data.len() as u64,
                archived: true,
                owned: None,
            });
        }
        fs::write(&source, bytes).unwrap();
        Self {
            index: Index {
                root: directory.clone(),
                resources,
                archives: 1,
                diagnostics: vec![],
            },
            directory,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn word(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}
fn string(b: &mut Vec<u8>, s: &str) {
    b.push(s.len() as u8);
    b.extend(s.as_bytes());
}
fn animation(name: &str, frames: u32) -> Vec<u8> {
    let mut b = vec![];
    for v in [0, 0, 1] {
        word(&mut b, v);
    }
    string(&mut b, name);
    string(&mut b, "samples");
    for v in [1000f32.to_bits(), 0, 0, 1, 0, 1] {
        word(&mut b, v);
    }
    string(&mut b, "ROOT");
    for v in [frames, 1000f32.to_bits(), 1, 0, 0, u32::MAX, 0, 0] {
        word(&mut b, v);
    }
    b
}
fn samples(x: f32) -> Vec<u8> {
    let mut b = vec![];
    for v in [x, 0., 0.] {
        b.push(255);
        b.extend(v.to_le_bytes());
    }
    b
}
fn appearance(name: &str, kind: u32) -> Vec<u8> {
    let mut b = vec![];
    word(&mut b, 0);
    word(&mut b, 1);
    string(&mut b, name);
    for v in [kind, 0, 0, 0] {
        word(&mut b, v);
    }
    b
}
fn animation_query(
    index: &Index,
    names: &[&str],
    native: bool,
    initial: &sims_vm::Program,
) -> serde_json::Value {
    let names: BTreeSet<_> = names.iter().map(|s| s.to_string()).collect();
    let mut direct = initial.clone();
    let mut cached = initial.clone();
    let mut original_order = vec![];
    let mut scan_order = vec![];
    let mut scan = Scan::default();
    let expected = assets::load_named_animations_read(index, &mut direct, &names, native, |r| {
        original_order.push(r.locator.clone());
        r.read()
    })
    .map(|r| serde_json::to_value(r).unwrap())
    .map_err(|e| format!("{e:#}"));
    let actual = assets::load_named_animations_read(index, &mut cached, &names, native, |r| {
        scan_order.push(r.locator.clone());
        scan.read(r)
    })
    .map(|r| serde_json::to_value(r).unwrap())
    .map_err(|e| format!("{e:#}"));
    assert_eq!(original_order, scan_order);
    assert_eq!(actual, expected);
    assert_eq!(
        serde_json::to_value(&cached).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );
    serde_json::json!({"result":actual,"program":cached})
}
fn appearance_query(index: &Index, name: &str) -> Result<Option<usize>, String> {
    let mut scan = Scan::default();
    let mut original_order = vec![];
    let mut scan_order = vec![];
    let expected = avatar::appearance_catalog_read(index, name, |r| {
        original_order.push(r.locator.clone());
        r.read()
    })
    .map_err(|e| format!("{e:#}"));
    let actual = avatar::appearance_catalog_read(index, name, |r| {
        scan_order.push(r.locator.clone());
        scan.read(r)
    })
    .map_err(|e| format!("{e:#}"));
    assert_eq!(original_order, scan_order);
    assert_eq!(actual, expected);
    actual
}

#[test]
fn animation_scan_keeps_case_duplicates_empty_modes_and_reports() {
    for frames in [0, 1] {
        let fixture = Fixture::new(vec![
            ("first.far::one.cmx.bcf", animation("Walk", frames)),
            ("first.far::samples.cfp", samples(1.)),
            ("second.far::two.cmx.bcf", animation("Walk", frames)),
            ("second.far::samples.cfp", samples(1.)),
            ("bad.far::bad.cmx.bcf", vec![0; 10]),
        ]);
        for native in [false, true] {
            let output = animation_query(
                &fixture.index,
                &["walk", "missing"],
                native,
                &Default::default(),
            );
            if frames > 0 || native {
                assert_eq!(output["result"]["Ok"]["loaded_clips"], 1);
                assert_eq!(
                    output["program"]["animations"]["walk"]["source"],
                    "first.far::one.cmx.bcf"
                );
                assert_eq!(
                    output["result"]["Ok"]["catalog_errors"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
            } else {
                assert!(
                    output["program"]["animations"]
                        .as_object()
                        .unwrap()
                        .is_empty()
                );
                assert_eq!(
                    output["result"]["Ok"]["catalog_errors"]
                        .as_array()
                        .unwrap()
                        .len(),
                    3
                );
            }
        }
    }
}

#[test]
fn animation_scan_keeps_conflicts_read_error_order_and_partial_publication() {
    let mut fixture = Fixture::new(vec![
        ("first.far::one.cmx.bcf", animation("walk", 1)),
        ("first.far::samples.cfp", samples(1.)),
        ("second.far::two.cmx.bcf", animation("walk", 1)),
        ("second.far::samples.cfp", samples(2.)),
    ]);
    assert!(
        animation_query(&fixture.index, &["walk"], true, &Default::default())["result"]["Err"]
            .as_str()
            .unwrap()
            .contains("conflicting animation walk")
    );
    // A preinstalled clip is authoritative after the same full discovery pass.
    let one = Fixture::new(vec![
        ("first.far::one.cmx.bcf", animation("walk", 1)),
        ("first.far::samples.cfp", samples(1.)),
    ]);
    let mut program = sims_vm::Program::default();
    assets::load_named_native_animations(&one.index, &mut program, &["walk".into()].into())
        .unwrap();
    let output = animation_query(&fixture.index, &["walk"], true, &program);
    assert_eq!(output["result"]["Ok"]["loaded_clips"], 0);
    fixture.index.resources[0].source = fixture.directory.join("absent-first");
    fixture.index.resources[2].source = fixture.directory.join("absent-second");
    let error = animation_query(&fixture.index, &["walk"], true, &program)["result"]["Err"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(error.contains("absent-first"));
    assert!(!error.contains("absent-second"));
    let partial = Fixture::new(vec![
        ("first.far::a.cmx.bcf", animation("a", 1)),
        ("first.far::samples.cfp", samples(1.)),
        ("second.far::b.cmx.bcf", animation("b", 1)),
        ("second.far::samples.cfp", vec![255]),
    ]);
    let output = animation_query(&partial.index, &["a", "b"], true, &Default::default());
    assert!(output["result"]["Err"].is_string());
    assert!(output["program"]["animations"]["a"].is_object());
    assert!(output["program"]["animations"]["b"].is_null());
}

#[test]
fn appearance_scan_keeps_whole_catalog_conflicts_filtering_and_failure_order() {
    let identical = appearance("suit", 0);
    let mut fixture = Fixture::new(vec![
        ("a.far::one.cmx.bcf", identical.clone()),
        ("b.far::two.cmx.bcf", identical),
        (
            "Creator_Files/invalid.cmx",
            b"invalid creator descriptor".to_vec(),
        ),
    ]);
    assert_eq!(appearance_query(&fixture.index, "SuIt"), Ok(Some(0)));
    assert_eq!(appearance_query(&fixture.index, "missing"), Ok(None));
    fixture.index.resources.swap(0, 1);
    assert_eq!(appearance_query(&fixture.index, "suit"), Ok(Some(0)));
    // New membership must be visited on the next call; no persistent ID cache.
    let different = Fixture::new(vec![
        ("other.far::three.cmx.bcf", appearance("suit", 1)),
        ("late.far::bad.cmx", b"invalid".to_vec()),
    ]);
    fixture
        .index
        .resources
        .extend(different.index.resources.clone());
    let error = appearance_query(&fixture.index, "suit").unwrap_err();
    assert!(error.contains("conflicting appearance suit"));
    assert!(!error.contains("late.far"));
    let missing_error = appearance_query(&fixture.index, "missing").unwrap_err();
    assert!(missing_error.contains("read appearance catalog late.far::bad.cmx"));
    // A prior read failure must not be cached as absence after repair.
    fixture.index.resources[0].source = fixture.directory.join("not-yet-present");
    assert!(
        appearance_query(&fixture.index, "suit")
            .unwrap_err()
            .contains("not-yet-present")
    );
    fixture.index.resources = different.index.resources[..1].to_vec();
    assert_eq!(appearance_query(&fixture.index, "suit"), Ok(Some(0)));
}
