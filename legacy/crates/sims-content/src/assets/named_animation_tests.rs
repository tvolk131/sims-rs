use super::*;
use crate::Resource;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn catalog(clips: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in [0u32, 0, clips.len() as u32] {
        bytes.extend(word.to_le_bytes());
    }
    for &(name, skill, frames) in clips {
        for text in [name, skill] {
            bytes.push(text.len() as u8);
            bytes.extend(text.as_bytes());
        }
        let translation = u32::from(frames != 0);
        for word in [33f32.to_bits(), 0, 0, translation, 0, 1] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.push(4);
        bytes.extend(b"ROOT");
        for word in [frames, 33f32.to_bits(), translation, 0, 0, 0, 0, 0] {
            bytes.extend(word.to_le_bytes());
        }
    }
    bytes
}

struct Fixture {
    root: PathBuf,
    index: Index,
}

impl Fixture {
    fn new(entries: &[(&str, Vec<u8>)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sims-named-animations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let mut resources = Vec::new();
        for (locator, bytes) in entries {
            let source = root.join(locator);
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(&source, bytes).unwrap();
            resources.push(Resource {
                locator: (*locator).into(),
                name: locator.rsplit('/').next().unwrap().into(),
                source,
                offset: 0,
                size: bytes.len() as u64,
                unpacked_size: bytes.len() as u64,
                archived: false,
                owned: None,
            });
        }
        let index = Index {
            root: root.clone(),
            resources,
            archives: 0,
            diagnostics: Vec::new(),
        };
        Self { root, index }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn existing_asset() -> AnimationAsset {
    AnimationAsset {
        source: "explicit/catalog".into(),
        content_hash: "explicit-content-hash".into(),
        samples_source: "explicit/samples".into(),
        samples_hash: "explicit-samples-hash".into(),
        clip: read_bcf(&catalog(&[("Explicit", "samples", 1)]))
            .unwrap()
            .remove(0),
        samples: read_cfp(&[126, 126, 126], 1, 0).unwrap(),
    }
}

struct Observation {
    result: std::result::Result<Value, String>,
    program: Program,
    reads: Vec<String>,
}

fn outcome(result: Result<Report>) -> std::result::Result<Value, String> {
    result
        .map(|report| serde_json::to_value(report).unwrap())
        .map_err(|error| format!("{error:#}"))
}

fn compare(
    index: &Index,
    requested: &BTreeSet<String>,
    retain_empty: bool,
    initial: &Program,
    read: impl Fn(&Resource) -> Result<Vec<u8>>,
) -> Observation {
    let mut expected = initial.clone();
    let mut expected_reads = Vec::new();
    let reference = outcome(reference(
        index,
        &mut expected,
        requested,
        retain_empty,
        |resource| {
            expected_reads.push(resource.locator.clone());
            read(resource)
        },
    ));
    let mut actual = initial.clone();
    let mut reads = Vec::new();
    let result = outcome(load_named_animations_read(
        index,
        &mut actual,
        requested,
        retain_empty,
        |resource| {
            reads.push(resource.locator.clone());
            read(resource)
        },
    ));
    assert_eq!(result, reference, "complete report or error chain changed");
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "complete Program, including partial animation mutation, changed"
    );
    assert_eq!(reads, expected_reads, "primary read callback order changed");
    let mut catalogs = AnimationCatalogs::default();
    for pass in ["cold", "warm"] {
        let mut cached = initial.clone();
        let mut cached_reads = Vec::new();
        let cached_result = outcome(catalogs.load_read(
            index,
            &mut cached,
            requested,
            retain_empty,
            |resource| {
                cached_reads.push(resource.locator.clone());
                read(resource)
            },
        ));
        assert_eq!(cached_result, reference, "{pass}: report/error chain changed");
        assert_eq!(
            serde_json::to_value(&cached).unwrap(),
            serde_json::to_value(&expected).unwrap(),
            "{pass}: complete or partially mutated Program changed"
        );
        assert_eq!(cached_reads, expected_reads, "{pass}: read order changed");
        assert_eq!(format!("{cached:?}"), format!("{expected:?}"));
    }
    Observation {
        result,
        program: actual,
        reads,
    }
}

#[test]
fn discovery_counts_all_unique_normalized_names_without_loading_unrequested_clips() {
    let fixture = Fixture::new(&[
        ("ignored.iff", vec![0; 9]),
        (
            "z/first.CMX.BCF",
            catalog(&[
                ("Wanted", "samples", 1),
                ("Idle", "absent", 1),
                ("iDLE", "also-absent", 1),
            ]),
        ),
        (
            "a/unrequested.cmx.bcf",
            catalog(&[("Other", "absent", 1), ("IDLE", "absent", 1)]),
        ),
        ("z/samples.cfp", vec![126; 3]),
        ("appearance.cmx.bcf", vec![1; 8]),
        ("empty.cmx.bcf", catalog(&[])),
    ]);
    let observed = compare(
        &fixture.index,
        &names(&["wanted"]),
        false,
        &Program::default(),
        Resource::read,
    );
    assert_eq!(
        observed.result.unwrap(),
        json!({"catalogs":3,"discovered_clips":3,"loaded_clips":1,
            "catalog_errors":[],"unresolved":[]})
    );
    assert_eq!(
        observed.reads,
        [
            "z/first.CMX.BCF",
            "a/unrequested.cmx.bcf",
            "appearance.cmx.bcf",
            "empty.cmx.bcf"
        ]
    );
    let asset = &observed.program.animations["wanted"];
    assert_eq!(asset.source, "z/first.CMX.BCF");
    assert_eq!(asset.samples_source, "z/samples.cfp");
    assert_eq!(asset.clip.name, "Wanted");
    assert_eq!(asset.samples.translations, [[0.0; 3]]);
    assert_eq!(
        asset.content_hash,
        format!(
            "{:x}",
            Sha256::digest(fixture.index.resources[1].read().unwrap())
        )
    );
    assert_eq!(
        asset.samples_hash,
        format!("{:x}", Sha256::digest([126; 3]))
    );
}

#[test]
fn caller_case_missing_names_and_exact_existing_keys_remain_distinct() {
    let fixture = Fixture::new(&[
        ("clips.cmx.bcf", catalog(&[("WaNtEd", "samples", 1)])),
        ("samples.cfp", vec![126; 3]),
    ]);
    for (requested, unresolved, loaded) in [
        (vec!["WANTED", "Wanted"], vec!["WANTED", "Wanted"], 0),
        (vec!["WaNtEd", "wanted"], vec!["WaNtEd"], 1),
        (vec!["wanted", "absent"], vec!["absent"], 1),
        (vec![], vec![], 0),
    ] {
        let observed = compare(
            &fixture.index,
            &names(&requested),
            false,
            &Program::default(),
            Resource::read,
        );
        let report = observed.result.unwrap();
        assert_eq!(report["unresolved"], json!(unresolved));
        assert_eq!(report["loaded_clips"], loaded);
        assert_eq!(report["discovered_clips"], 1);
        assert_eq!(observed.reads, ["clips.cmx.bcf"]);
    }
    let initial = Program {
        animations: BTreeMap::from([("WaNtEd".into(), existing_asset())]),
        ..Program::default()
    };
    let observed = compare(
        &fixture.index,
        &names(&["WaNtEd", "wanted"]),
        false,
        &initial,
        Resource::read,
    );
    assert_eq!(observed.result.unwrap()["loaded_clips"], 1);
    assert_eq!(observed.program.animations.len(), 2);
    assert_eq!(
        observed.program.animations["WaNtEd"].source,
        "explicit/catalog"
    );
}

#[test]
fn already_loaded_guard_keeps_explicit_assets_but_still_reads_all_catalogs() {
    let fixture = Fixture::new(&[
        ("already.cmx.bcf", catalog(&[("Already", "no-cfp", 1)])),
        ("unrequested.cmx.bcf", catalog(&[("Other", "no-cfp", 1)])),
    ]);
    let initial = Program {
        animations: BTreeMap::from([("already".into(), existing_asset())]),
        ..Program::default()
    };
    let observed = compare(
        &fixture.index,
        &names(&["already", "missing"]),
        false,
        &initial,
        Resource::read,
    );
    assert_eq!(
        observed.result.unwrap(),
        json!({"catalogs":2,"discovered_clips":2,"loaded_clips":0,
            "catalog_errors":[],"unresolved":["missing"]})
    );
    assert_eq!(observed.reads, ["already.cmx.bcf", "unrequested.cmx.bcf"]);
    assert_eq!(
        serde_json::to_value(observed.program).unwrap(),
        serde_json::to_value(initial).unwrap()
    );
}

#[test]
fn identical_duplicates_keep_first_resource_provenance_and_local_samples() {
    let fixture = Fixture::new(&[
        (
            "z/first.cmx.bcf",
            catalog(&[("Wanted", "samples", 1), ("Wanted", "samples", 1)]),
        ),
        ("a/second.cmx.bcf", catalog(&[("Wanted", "samples", 1)])),
        ("a/samples.cfp", vec![126; 3]),
        ("z/samples.cfp", vec![126; 3]),
    ]);
    let observed = compare(
        &fixture.index,
        &names(&["wanted"]),
        false,
        &Program::default(),
        Resource::read,
    );
    assert_eq!(observed.result.unwrap()["loaded_clips"], 1);
    assert_eq!(observed.reads, ["z/first.cmx.bcf", "a/second.cmx.bcf"]);
    let asset = &observed.program.animations["wanted"];
    assert_eq!(asset.source, "z/first.cmx.bcf");
    assert_eq!(asset.samples_source, "z/samples.cfp");
}

#[test]
fn conflicting_clip_or_sample_providers_preserve_error_order_and_partial_mutation() {
    for (second_name, samples) in [("Wanted", vec![127, 126, 126]), ("WANTED", vec![126; 3])] {
        let fixture = Fixture::new(&[
            (
                "z/first.cmx.bcf",
                catalog(&[
                    ("Alpha", "samples", 1),
                    ("Wanted", "samples", 1),
                    ("Zeta", "samples", 1),
                ]),
            ),
            ("a/second.cmx.bcf", catalog(&[(second_name, "samples", 1)])),
            ("z/samples.cfp", vec![126; 3]),
            ("a/samples.cfp", samples),
        ]);
        let observed = compare(
            &fixture.index,
            &names(&["alpha", "wanted", "zeta"]),
            false,
            &Program::default(),
            Resource::read,
        );
        assert_eq!(
            observed.result.unwrap_err(),
            "conflicting animation wanted: z/first.cmx.bcf and a/second.cmx.bcf; select an explicit provider"
        );
        assert_eq!(
            observed
                .program
                .animations
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["alpha"]
        );
    }
}

#[test]
fn providers_inside_one_catalog_are_decoded_in_clip_order() {
    for skills in [
        ["broken", "missing"],
        ["missing", "broken"],
        ["samples", "missing"],
    ] {
        let fixture = Fixture::new(&[
            (
                "clips.cmx.bcf",
                catalog(&[("Wanted", skills[0], 1), ("Wanted", skills[1], 1)]),
            ),
            ("broken.cfp", vec![255]),
            ("samples.cfp", vec![126; 3]),
        ]);
        let observed = compare(
            &fixture.index,
            &names(&["wanted"]),
            false,
            &Program::default(),
            Resource::read,
        );
        let error = observed.result.unwrap_err();
        if skills[0] == "broken" {
            assert!(error.starts_with("broken.cfp:"), "{error}");
        } else {
            assert_eq!(error, "resource not found: missing.cfp");
        }
        assert!(observed.program.animations.is_empty());
    }
}

#[test]
fn malformed_unrequested_catalogs_are_reported_in_order_without_partial_discovery() {
    let mut trailing = catalog(&[("Ghost", "absent", 1)]);
    trailing.push(7);
    let mut truncated = catalog(&[
        ("NotPublished", "absent", 1),
        ("AlsoNotPublished", "absent", 1),
    ]);
    truncated.pop();
    let fixture = Fixture::new(&[
        ("bad.cmx.bcf", vec![0; 9]),
        ("trailing.cmx.bcf", trailing),
        ("partial.cmx.bcf", truncated),
        ("good.cmx.bcf", catalog(&[("Wanted", "samples", 1)])),
        ("samples.cfp", vec![126; 3]),
    ]);
    for requested in [vec![], vec!["ghost", "notpublished", "wanted"]] {
        let observed = compare(
            &fixture.index,
            &names(&requested),
            false,
            &Program::default(),
            Resource::read,
        );
        let report = observed.result.unwrap();
        assert_eq!(report["catalogs"], 1);
        assert_eq!(report["discovered_clips"], 1);
        let errors = report["catalog_errors"].as_array().unwrap();
        assert_eq!(errors.len(), 3);
        for (error, locator) in
            errors
                .iter()
                .zip(["bad.cmx.bcf:", "trailing.cmx.bcf:", "partial.cmx.bcf:"])
        {
            assert!(error.as_str().unwrap().starts_with(locator));
        }
        assert!(errors[1].as_str().unwrap().contains("trailing bytes"));
        assert_eq!(
            report["unresolved"],
            if requested.is_empty() {
                json!([])
            } else {
                json!(["ghost", "notpublished"])
            }
        );
        assert_eq!(
            observed.reads,
            [
                "bad.cmx.bcf",
                "trailing.cmx.bcf",
                "partial.cmx.bcf",
                "good.cmx.bcf"
            ]
        );
    }
}

#[test]
fn primary_read_failures_stop_in_resource_order_even_for_empty_or_loaded_requests() {
    let mut fixture = Fixture::new(&[
        ("ignored.txt", vec![]),
        ("z-first.cmx.bcf", catalog(&[("Other", "absent", 1)])),
        ("a-bad.cmx.bcf", vec![0; 9]),
        ("appearance.cmx.bcf", vec![1; 8]),
        ("last.cmx.bcf", catalog(&[("Wanted", "absent", 1)])),
    ]);
    let order = [
        "z-first.cmx.bcf",
        "a-bad.cmx.bcf",
        "appearance.cmx.bcf",
        "last.cmx.bcf",
    ];
    for requested in [vec![], vec!["wanted"], vec!["already"]] {
        let initial = Program {
            animations: BTreeMap::from([("already".into(), existing_asset())]),
            ..Program::default()
        };
        for (failure, locator) in order.iter().enumerate() {
            let observed = compare(
                &fixture.index,
                &names(&requested),
                false,
                &initial,
                |resource| {
                    if resource.locator == *locator {
                        bail!("injected primary read failure: {}", resource.locator);
                    }
                    resource.read()
                },
            );
            assert_eq!(
                observed.result.unwrap_err(),
                format!("injected primary read failure: {locator}")
            );
            assert_eq!(observed.reads, order[..=failure]);
            assert_eq!(
                serde_json::to_value(observed.program).unwrap(),
                serde_json::to_value(&initial).unwrap()
            );
        }
    }
    fixture.index.resources[1].size += 1;
    fixture.index.resources[1].unpacked_size += 1;
    for requested in [vec![], vec!["wanted"]] {
        let observed = compare(
            &fixture.index,
            &names(&requested),
            false,
            &Program::default(),
            Resource::read,
        );
        assert_eq!(
            observed.result.unwrap_err(),
            "resource outside source file: z-first.cmx.bcf"
        );
        assert_eq!(observed.reads, ["z-first.cmx.bcf"]);
        assert!(observed.program.animations.is_empty());
    }
}

#[test]
fn native_empty_motion_records_do_not_broaden_the_ordinary_parser() {
    let fixture = Fixture::new(&[
        (
            "mixed.cmx.bcf",
            catalog(&[("Wanted", "samples", 1), ("Empty", "empty", 0)]),
        ),
        ("samples.cfp", vec![126; 3]),
        ("empty.cfp", vec![]),
    ]);
    for retain_empty in [false, true] {
        for requested in [
            vec![],
            vec!["wanted"],
            vec!["empty"],
            vec!["empty", "wanted"],
        ] {
            let observed = compare(
                &fixture.index,
                &names(&requested),
                retain_empty,
                &Program::default(),
                Resource::read,
            );
            let report = observed.result.unwrap();
            assert_eq!(observed.reads, ["mixed.cmx.bcf"]);
            if retain_empty {
                assert_eq!(report["catalogs"], 1);
                assert_eq!(report["discovered_clips"], 2);
                assert_eq!(report["loaded_clips"], requested.len());
                assert_eq!(report["catalog_errors"], json!([]));
                if let Some(asset) = observed.program.animations.get("empty") {
                    assert_eq!(asset.clip.frames, 0);
                    assert_eq!(asset.clip.motions[0].frames, 0);
                    assert!(asset.samples.translations.is_empty());
                }
            } else {
                assert_eq!(report["catalogs"], 0);
                assert_eq!(report["discovered_clips"], 0);
                assert!(
                    report["catalog_errors"][0]
                        .as_str()
                        .unwrap()
                        .contains("empty motion channels")
                );
                assert_eq!(report["unresolved"], json!(requested));
                assert!(observed.program.animations.is_empty());
            }
        }
    }
}

#[test]
fn direct_dependency_failures_preserve_precedence_and_prior_program_mutation() {
    for (include_samples, missing_samples, malformed_samples, missing_source) in [
        (false, false, false, false),
        (true, true, false, false),
        (true, false, true, false),
        (true, false, false, true),
        (true, false, true, true),
        (true, true, false, true),
    ] {
        let original = catalog(&[("Wanted", "samples", 1)]);
        let mut entries = vec![
            ("alpha.cmx.bcf", catalog(&[("Alpha", "alpha", 1)])),
            ("alpha.cfp", vec![126; 3]),
            ("wanted.cmx.bcf", original.clone()),
        ];
        if include_samples {
            entries.push((
                "samples.cfp",
                if malformed_samples {
                    vec![255]
                } else {
                    vec![126; 3]
                },
            ));
        }
        let mut fixture = Fixture::new(&entries);
        if missing_samples {
            fixture.index.resources[3].source = fixture.root.join("missing-samples");
        }
        if missing_source {
            fixture.index.resources[2].source = fixture.root.join("missing-catalog");
        }
        let observed = compare(
            &fixture.index,
            &names(&["alpha", "wanted", "zeta"]),
            false,
            &Program::default(),
            |resource| {
                if resource.locator == "wanted.cmx.bcf" {
                    Ok(original.clone())
                } else {
                    resource.read()
                }
            },
        );
        let error = observed.result.unwrap_err();
        if !include_samples {
            assert_eq!(error, "resource not found: samples.cfp");
        } else if missing_samples {
            assert!(
                error.starts_with(&format!(
                    "open {}",
                    fixture.root.join("missing-samples").display()
                )),
                "{error}"
            );
        } else if missing_source {
            assert!(
                error.starts_with(&format!(
                    "open {}",
                    fixture.root.join("missing-catalog").display()
                )),
                "{error}"
            );
        } else {
            assert!(error.starts_with("samples.cfp:"), "{error}");
        }
        assert_eq!(observed.reads, ["alpha.cmx.bcf", "wanted.cmx.bcf"]);
        assert_eq!(
            observed
                .program
                .animations
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["alpha"]
        );
    }
}

#[test]
fn source_hash_and_samples_use_direct_reads_not_the_primary_callback_payload() {
    let original = catalog(&[("Wanted", "samples", 1)]);
    let physical = catalog(&[("Hidden", "samples", 1)]);
    let fixture = Fixture::new(&[
        ("catalog.cmx.bcf", physical.clone()),
        ("samples.cfp", vec![127, 126, 126]),
    ]);
    let observed = compare(
        &fixture.index,
        &names(&["wanted"]),
        false,
        &Program::default(),
        |resource| {
            assert_eq!(resource.locator, "catalog.cmx.bcf");
            Ok(original.clone())
        },
    );
    assert_eq!(observed.result.unwrap()["loaded_clips"], 1);
    assert_eq!(observed.reads, ["catalog.cmx.bcf"]);
    let asset = &observed.program.animations["wanted"];
    assert_eq!(asset.clip.name, "Wanted");
    assert_eq!(
        asset.content_hash,
        format!("{:x}", Sha256::digest(physical))
    );
    assert_eq!(
        asset.samples_hash,
        format!("{:x}", Sha256::digest([127, 126, 126]))
    );
    assert_ne!(asset.samples.translations, [[0.0; 3]]);
}

fn compare_retained(
    index: &Index,
    requested: &BTreeSet<String>,
    retain_empty: bool,
    catalogs: &mut AnimationCatalogs,
) -> Observation {
    let mut expected = Program::default();
    let mut expected_reads = Vec::new();
    let expected_result = outcome(reference(
        index,
        &mut expected,
        requested,
        retain_empty,
        |resource| {
            expected_reads.push(resource.locator.clone());
            resource.read()
        },
    ));
    let mut program = Program::default();
    let mut reads = Vec::new();
    let result = outcome(catalogs.load_read(
        index,
        &mut program,
        requested,
        retain_empty,
        |resource| {
            reads.push(resource.locator.clone());
            resource.read()
        },
    ));
    assert_eq!(result, expected_result, "retained report/error changed");
    assert_eq!(
        serde_json::to_value(&program).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "retained complete or partially mutated Program changed"
    );
    assert_eq!(reads, expected_reads, "retained read order changed");
    assert_eq!(format!("{program:?}"), format!("{expected:?}"));
    Observation {
        result,
        program,
        reads,
    }
}

#[test]
fn retained_catalogs_revalidate_changed_clips_samples_errors_and_restored_inputs() {
    let original = catalog(&[("Wanted", "samples", 1)]);
    let fixture = Fixture::new(&[
        ("catalog.cmx.bcf", original.clone()),
        ("samples.cfp", vec![126; 3]),
    ]);
    let requested = names(&["wanted"]);
    let mut catalogs = AnimationCatalogs::default();
    let first = compare_retained(&fixture.index, &requested, true, &mut catalogs);
    assert_eq!(first.result.unwrap()["loaded_clips"], 1);
    let original_asset = &first.program.animations["wanted"];
    let source = fixture.root.join("catalog.cmx.bcf");
    let changed = catalog(&[("WANTED", "samples", 1)]);
    assert_eq!(changed.len(), original.len());
    fs::write(&source, &changed).unwrap();
    let updated = compare_retained(&fixture.index, &requested, true, &mut catalogs);
    assert_ne!(updated.program.animations["wanted"].clip, original_asset.clip);
    fs::write(fixture.root.join("samples.cfp"), [127, 126, 126]).unwrap();
    let resampled = compare_retained(&fixture.index, &requested, true, &mut catalogs);
    assert_ne!(
        resampled.program.animations["wanted"].samples,
        updated.program.animations["wanted"].samples
    );
    fs::write(fixture.root.join("samples.cfp"), [126, 126]).unwrap();
    assert!(
        compare_retained(&fixture.index, &requested, true, &mut catalogs)
            .result
            .is_err()
    );
    fs::write(fixture.root.join("samples.cfp"), [126; 3]).unwrap();
    let mut malformed = changed;
    malformed[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&source, malformed).unwrap();
    let rejected = compare_retained(&fixture.index, &requested, true, &mut catalogs);
    let report = rejected.result.unwrap();
    assert_eq!(report["catalog_errors"].as_array().unwrap().len(), 1);
    assert_eq!(report["unresolved"], json!(["wanted"]));
    let hidden = source.with_extension("hidden");
    fs::rename(&source, &hidden).unwrap();
    assert!(
        compare_retained(&fixture.index, &requested, true, &mut catalogs)
            .result
            .is_err()
    );
    fs::rename(hidden, &source).unwrap();
    fs::write(&source, &original).unwrap();
    let restored = compare_retained(&fixture.index, &requested, true, &mut catalogs);
    assert_eq!(
        serde_json::to_value(&restored.program.animations["wanted"]).unwrap(),
        serde_json::to_value(original_asset).unwrap()
    );
}

#[test]
fn retained_parser_modes_do_not_convert_native_empty_clips_into_legacy_assets() {
    let fixture = Fixture::new(&[
        ("catalog.cmx.bcf", catalog(&[("Empty", "samples", 0)])),
        ("samples.cfp", vec![]),
    ]);
    let requested = names(&["empty"]);
    let mut catalogs = AnimationCatalogs::default();
    for retain_empty in [false, true, false, true] {
        let observed = compare_retained(&fixture.index, &requested, retain_empty, &mut catalogs);
        assert_eq!(observed.program.animations.contains_key("empty"), retain_empty);
    }
}

fn reference(
    index: &Index,
    program: &mut Program,
    names: &BTreeSet<String>,
    retain_empty: bool,
    mut read: impl FnMut(&crate::Resource) -> Result<Vec<u8>>,
) -> Result<Report> {
    let mut candidates: BTreeMap<String, Vec<(usize, Animation)>> = BTreeMap::new();
    let mut report = Report::default();
    for (id, resource) in index.resources.iter().enumerate() {
        if !resource.name.to_ascii_lowercase().ends_with(".cmx.bcf") {
            continue;
        }
        let bytes = read(resource)?;
        // The separate avatar parser owns skeleton/appearance catalogs.
        if bytes.get(..8) != Some(&[0; 8]) {
            continue;
        }
        let clips = match if retain_empty {
            sims_formats::animation::read_bcf_native(&bytes)
        } else {
            read_bcf(&bytes)
        } {
            Ok(clips) => clips,
            Err(e) => {
                report
                    .catalog_errors
                    .push(format!("{}: {e}", resource.locator));
                continue;
            }
        };
        for clip in clips {
            candidates
                .entry(clip.name.to_ascii_lowercase())
                .or_default()
                .push((id, clip));
        }
        report.catalogs += 1;
    }
    report.discovered_clips = candidates.len();
    for name in names {
        // Explicit locomotion/idle selections remain authoritative until the
        // native content manager's search order has been recovered.
        if program.animations.contains_key(name) {
            continue;
        }
        let Some(providers) = candidates.get(name) else {
            report.unresolved.push(name.clone());
            continue;
        };
        let mut decoded: Option<AnimationAsset> = None;
        for (id, clip) in providers {
            let resource = &index.resources[*id];
            let cfp_name = format!("{}.cfp", clip.skill);
            let cfp_id = sibling(index, *id, &cfp_name)?;
            let cfp = &index.resources[cfp_id];
            let data = cfp.read()?;
            let asset = AnimationAsset {
                source: resource.locator.clone(),
                content_hash: format!("{:x}", Sha256::digest(resource.read()?)),
                samples_source: cfp.locator.clone(),
                samples_hash: format!("{:x}", Sha256::digest(&data)),
                samples: read_cfp(&data, clip.translation_count, clip.rotation_count)
                    .with_context(|| cfp.locator.clone())?,
                clip: clip.clone(),
            };
            if let Some(previous) = &decoded {
                if previous.clip != asset.clip || previous.samples != asset.samples {
                    bail!(
                        "conflicting animation {name}: {} and {}; select an explicit provider",
                        previous.source,
                        asset.source
                    );
                }
            } else {
                decoded = Some(asset);
            }
        }
        program.animations.insert(name.clone(), decoded.unwrap());
        report.loaded_clips += 1;
    }
    Ok(report)
}
