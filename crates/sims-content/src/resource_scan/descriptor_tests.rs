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

fn legacy_definitions(
    index: &Index,
    neighborhood: &str,
    guids: &BTreeSet<u32>,
    mut read: impl FnMut(&Resource) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<std::collections::BTreeMap<u32, crate::saves::DefinitionSource>> {
    use crate::saves::DefinitionSource;
    use anyhow::{Context, ensure};
    use sha2::{Digest, Sha256};
    use sims_formats::iff::Iff;
    use std::collections::BTreeMap;
    let character_prefix = format!("{neighborhood}/characters/").to_ascii_lowercase();
    let mut candidates = BTreeMap::<u32, Vec<(DefinitionSource, bool)>>::new();
    for resource in &index.resources {
        if !resource.is_iff() {
            continue;
        }
        let location = resource.locator.to_ascii_lowercase();
        let is_local = location.starts_with(&character_prefix);
        if location.contains("/characters/") && !is_local {
            continue;
        }
        let iff = Iff::parse(read(resource)?)?;
        let mut sha256 = None;
        for chunk in iff.all(b"OBJD") {
            let payload = iff.data(chunk);
            if payload.len() < 32 {
                continue;
            }
            let guid = u32::from_le_bytes(payload[28..32].try_into().unwrap());
            if !guids.contains(&guid) {
                continue;
            }
            let def = DefinitionSource {
                source: resource.locator.clone(),
                sha256: sha256
                    .get_or_insert_with(|| format!("{:x}", Sha256::digest(&iff.bytes)))
                    .clone(),
                chunk: chunk.id,
                kind: u16::from_le_bytes(payload[18..20].try_into().unwrap()),
                multipart: u16::from_le_bytes(payload[20..22].try_into().unwrap()) != 0,
            };
            candidates.entry(guid).or_default().push((def, is_local));
        }
    }
    let mut result = BTreeMap::new();
    for guid in guids {
        let mut choices = candidates
            .remove(guid)
            .with_context(|| format!("missing original object GUID {guid:08x}"))?;
        if choices.iter().any(|choice| choice.1) {
            choices.retain(|choice| choice.1);
        }
        ensure!(
            choices
                .iter()
                .all(|choice| choice.0.sha256 == choices[0].0.sha256
                    && choice.0.chunk == choices[0].0.chunk),
            "ambiguous original object {guid:08x}: {:?}",
            choices
                .iter()
                .map(|choice| &choice.0.source)
                .collect::<Vec<_>>()
        );
        result.insert(*guid, choices.remove(0).0);
    }
    Ok(result)
}

fn object_iff(guid: u32, identifier: u16, kind: u16, multipart: u16) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    bytes.extend(b"OBJD");
    bytes.extend(108u32.to_be_bytes());
    bytes.extend(identifier.to_be_bytes());
    bytes.extend(0u16.to_be_bytes());
    bytes.extend([0; 64]);
    let mut payload = vec![0; 32];
    payload[18..20].copy_from_slice(&kind.to_le_bytes());
    payload[20..22].copy_from_slice(&multipart.to_le_bytes());
    payload[28..32].copy_from_slice(&guid.to_le_bytes());
    bytes.extend(payload);
    bytes
}

fn definition_query(index: &Index, neighborhood: &str, requested: &[u32]) -> serde_json::Value {
    let guids = requested.iter().copied().collect();
    let mut legacy_order = Vec::new();
    let mut scan_order = Vec::new();
    let expected = legacy_definitions(index, neighborhood, &guids, |resource| {
        legacy_order.push(resource.locator.clone());
        resource.read()
    })
    .map(|value| serde_json::to_value(value).unwrap())
    .map_err(|error| format!("{error:#}"));
    let mut scan = Scan::default();
    let actual = crate::saves::definitions_read(index, neighborhood, &guids, |resource| {
        scan_order.push(resource.locator.clone());
        scan.read(resource)
    })
    .map(|value| serde_json::to_value(value).unwrap())
    .map_err(|error| format!("{error:#}"));
    let public = crate::saves::definitions(index, neighborhood, &guids)
        .map(|value| serde_json::to_value(value).unwrap())
        .map_err(|error| format!("{error:#}"));
    assert_eq!(legacy_order, scan_order);
    assert_eq!(actual, expected);
    assert_eq!(public, expected);
    serde_json::json!({"result": actual, "read_order": scan_order})
}

#[test]
fn definition_scan_preserves_neighborhood_precedence_and_definition_fields() {
    let shared = object_iff(0x1020, 7, 4, 0);
    let fixture = Fixture::new(vec![
        ("objects.far::global.iff", shared.clone()),
        ("UserData2/Characters/foreign.iff", vec![0; 12]),
        (
            "UserData/Characters/local.iff",
            object_iff(0x1020, 11, 2, 1),
        ),
        ("objects.far::another.iff", object_iff(0x3040, 19, 8, 2)),
        ("objects.far::ignored.txt", vec![0; 12]),
    ]);
    let output = definition_query(&fixture.index, "USERDATA", &[0x1020, 0x3040]);
    assert_eq!(
        output["result"]["Ok"]["4128"]["source"],
        "UserData/Characters/local.iff"
    );
    assert_eq!(output["result"]["Ok"]["4128"]["chunk"], 11);
    assert_eq!(output["result"]["Ok"]["4128"]["kind"], 2);
    assert_eq!(output["result"]["Ok"]["4128"]["multipart"], true);
    assert_eq!(output["read_order"].as_array().unwrap().len(), 3);
    let duplicates = Fixture::new(vec![
        ("objects.far::first.iff", shared.clone()),
        ("objects.far::second.iff", shared),
    ]);
    assert_eq!(
        definition_query(&duplicates.index, "UserData", &[0x1020])["result"]["Ok"]["4128"]["source"],
        "objects.far::first.iff"
    );
}

#[test]
fn definition_scan_preserves_ambiguity_missing_guids_and_first_read_error() {
    let mut fixture = Fixture::new(vec![
        ("objects.far::first.iff", object_iff(9, 1, 0, 0)),
        ("objects.far::second.iff", object_iff(9, 2, 0, 0)),
    ]);
    assert!(
        definition_query(&fixture.index, "UserData", &[9])["result"]["Err"]
            .as_str()
            .unwrap()
            .contains("ambiguous original object 00000009")
    );
    assert!(
        definition_query(&fixture.index, "UserData", &[99])["result"]["Err"]
            .as_str()
            .unwrap()
            .contains("missing original object GUID 00000063")
    );
    fixture.index.resources[0].source = fixture.directory.join("first-missing");
    fixture.index.resources[1].source = fixture.directory.join("second-missing");
    let output = definition_query(&fixture.index, "UserData", &[9]);
    let error = output["result"]["Err"].as_str().unwrap();
    assert!(error.contains("first-missing"));
    assert!(!error.contains("second-missing"));
}

#[test]
fn definition_scan_validates_every_iff_before_selection_or_empty_result() {
    let valid = object_iff(9, 1, 0, 0);
    let mut unrelated = object_iff(0x8000ffff, 2, 0xffff, 0x8000);
    let mut junk = object_iff(77, 3, 0, 0);
    junk[64..68].copy_from_slice(b"JUNK");
    unrelated.extend_from_slice(&junk[64..]);
    unrelated.extend_from_slice(&object_iff(u32::MAX, 4, 6, 1)[64..]);
    unrelated.extend(b"BAD");
    let mut fixture = Fixture::new(vec![
        ("objects.far::selected.iff", valid),
        ("objects.far::unrelated.iff", unrelated.clone()),
    ]);
    for length in 0..=unrelated.len() {
        let resource = &mut fixture.index.resources[1];
        resource.size = length as u64;
        resource.unpacked_size = length as u64;
        resource.owned = Some(std::sync::Arc::from(&unrelated[..length]));
        for requested in [
            &[][..],
            &[9][..],
            &[0x8000ffff][..],
            &[u32::MAX][..],
            &[77][..],
        ] {
            let output = definition_query(&fixture.index, "UserData", requested);
            if length < 64 || (length - 64) % 108 != 0 {
                assert!(
                    output["result"]["Err"].is_string(),
                    "prefix length {length}"
                );
            } else if requested.is_empty() {
                assert_eq!(output["result"]["Ok"], serde_json::json!({}));
            }
        }
    }
}

#[test]
fn definition_scan_keeps_physical_duplicates_and_first_discovery_fields() {
    let guid = 0x8000ffff;
    let mut bytes = object_iff(guid, 300, 0xffff, 0x8000);
    bytes[74..76].copy_from_slice(&4u16.to_be_bytes());
    bytes[140..144].copy_from_slice(&u32::MAX.to_le_bytes());
    let duplicate_start = bytes.len();
    bytes.extend_from_slice(&object_iff(guid, 300, 2, 0)[64..]);
    let fixture = Fixture::new(vec![("objects.far::duplicates.iff", bytes.clone())]);
    let output = definition_query(&fixture.index, "UserData", &[guid]);
    let definition = &output["result"]["Ok"][guid.to_string()];
    assert_eq!(definition["chunk"], 300);
    assert_eq!(definition["kind"], 0xffff);
    assert_eq!(definition["multipart"], true);
    bytes[duplicate_start + 8..duplicate_start + 10].copy_from_slice(&301u16.to_be_bytes());
    let fixture = Fixture::new(vec![("objects.far::duplicates.iff", bytes)]);
    let output = definition_query(&fixture.index, "UserData", &[guid]);
    assert_eq!(
        output["result"]["Err"],
        "ambiguous original object 8000ffff: [\"objects.far::duplicates.iff\", \"objects.far::duplicates.iff\"]"
    );
}

#[test]
fn definition_scan_authenticates_header_labels_flags_and_unselected_payloads() {
    use sha2::{Digest, Sha256};
    let mut bytes = object_iff(9, 1, 0, 0);
    let mut junk = object_iff(77, 2, 0, 0);
    junk[64..68].copy_from_slice(b"JUNK");
    bytes.extend_from_slice(&junk[64..]);
    let fixture = Fixture::new(vec![
        ("objects.far::first.iff", bytes.clone()),
        ("objects.far::second.iff", bytes.clone()),
    ]);
    let output = definition_query(&fixture.index, "UserData", &[9]);
    assert_eq!(
        output["result"]["Ok"]["9"]["sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(
        output["result"]["Ok"]["9"]["source"],
        "objects.far::first.iff"
    );
    for offset in [13, 60, 74, 76, 140, bytes.len() - 1] {
        let mut changed = bytes.clone();
        changed[offset] ^= 4;
        let fixture = Fixture::new(vec![
            ("objects.far::first.iff", bytes.clone()),
            ("objects.far::second.iff", changed),
        ]);
        let output = definition_query(&fixture.index, "UserData", &[9]);
        assert_eq!(
            output["result"]["Err"],
            "ambiguous original object 00000009: [\"objects.far::first.iff\", \"objects.far::second.iff\"]",
            "changed byte {offset}"
        );
    }
}

#[test]
fn definition_scan_reports_late_failures_before_ambiguity_or_missing_guids() {
    let mut fixture = Fixture::new(vec![
        ("objects.far::first.iff", object_iff(9, 1, 0, 0)),
        ("objects.far::second.iff", object_iff(9, 2, 0, 0)),
        ("objects.far::malformed.iff", vec![0; 3]),
    ]);
    for requested in [&[][..], &[9][..], &[77][..]] {
        let output = definition_query(&fixture.index, "UserData", requested);
        assert_eq!(
            output["result"]["Err"],
            "at byte 0x0: need 64 bytes; only 3 remain"
        );
        assert_eq!(output["read_order"].as_array().unwrap().len(), 3);
    }
    fixture.index.resources[2].source = fixture.directory.join("late-missing");
    for requested in [&[][..], &[9][..], &[77][..]] {
        let output = definition_query(&fixture.index, "UserData", requested);
        assert!(
            output["result"]["Err"]
                .as_str()
                .unwrap()
                .contains("late-missing")
        );
        assert_eq!(output["read_order"].as_array().unwrap().len(), 3);
    }
}

#[test]
fn definition_scan_reuses_one_archive_without_caching_payloads() {
    let fixture = Fixture::new(
        (1..=64)
            .map(|identifier| {
                (
                    format!("objects.far::object-{identifier}.iff"),
                    object_iff(identifier, identifier as u16, 0, 0),
                )
            })
            .collect::<Vec<_>>()
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect(),
    );
    let guids: BTreeSet<_> = (1..=64).collect();
    let mut scan = Scan::default();
    let result = crate::saves::definitions_read(&fixture.index, "UserData", &guids, |resource| {
        scan.read(resource)
    })
    .unwrap();
    assert_eq!(result.len(), 64);
    #[cfg(unix)]
    assert_eq!(scan.open_count(), 1);
    let path = fixture.directory.join("payload");
    let mut bytes = fs::read(&path).unwrap();
    bytes[64 + 76 + 28..64 + 76 + 32].copy_from_slice(&1000u32.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    let updated =
        crate::saves::definitions_read(&fixture.index, "UserData", &[1000].into(), |resource| {
            scan.read(resource)
        })
        .unwrap();
    assert_eq!(updated[&1000].source, "objects.far::object-1.iff");
    assert!(
        crate::saves::definitions_read(&fixture.index, "UserData", &[1].into(), |resource| scan
            .read(resource))
        .is_err()
    );
}

#[test]
fn definition_scan_retains_short_object_records_and_non_object_validation() {
    let mut short = object_iff(9, 1, 0, 0);
    short.truncate(short.len() - 1);
    short[68..72].copy_from_slice(&107u32.to_be_bytes());
    let mut other = object_iff(9, 1, 0, 0);
    other[64..68].copy_from_slice(b"STR#");
    let fixture = Fixture::new(vec![
        ("objects.far::short.iff", short),
        ("objects.far::not-object.iff", other),
        ("objects.far::valid.iff", object_iff(9, 3, 6, 1)),
    ]);
    let output = definition_query(&fixture.index, "UserData", &[9]);
    assert_eq!(output["result"]["Ok"]["9"]["chunk"], 3);
    assert_eq!(output["read_order"].as_array().unwrap().len(), 3);
    assert_eq!(
        definition_query(&fixture.index, "UserData", &[])["result"]["Ok"],
        serde_json::json!({})
    );
}
