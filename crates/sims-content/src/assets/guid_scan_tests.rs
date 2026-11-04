//! GUID-discovery differentials against the frozen direct-reader lookup.
//! Synthetic resources use real IFF bytes, real file ranges and real VM linking.
use super::*;
use crate::Resource;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const A: u32 = 0x10203040;
const B: u32 = 0x50607080;

fn iff(objects: &[(u16, u32)]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for &(id, guid) in objects {
        let mut data = vec![0; 216];
        data[..4].copy_from_slice(&138u32.to_le_bytes());
        data[28..32].copy_from_slice(&guid.to_le_bytes());
        bytes.extend(b"OBJD");
        bytes.extend((76u32 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend([0; 66]);
        bytes.extend(data);
    }
    bytes
}
struct Fixture {
    root: PathBuf,
    archive: PathBuf,
    bytes: Vec<u8>,
    index: Index,
}
impl Fixture {
    fn new(entries: &[(&str, Vec<u8>)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sims-guid-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let archive = root.join("ranges.far");
        let mut bytes = vec![];
        let mut resources = vec![];
        // Resource views model actual FAR entry reads; this fixture does not
        // pretend the concatenation is a FAR directory parsed by Index::scan.
        for (name, data) in entries {
            resources.push(Resource {
                locator: format!("ranges.far::{name}"),
                name: (*name).into(),
                source: archive.clone(),
                offset: bytes.len() as u64,
                size: data.len() as u64,
                unpacked_size: data.len() as u64,
                archived: true,
                owned: None,
            });
            bytes.extend(data);
        }
        fs::write(&archive, &bytes).unwrap();
        let global = iff(&[]);
        let source = root.join("Global.iff");
        fs::write(&source, &global).unwrap();
        resources.push(Resource {
            locator: "Global.iff".into(),
            name: "Global.iff".into(),
            source,
            offset: 0,
            size: global.len() as u64,
            unpacked_size: global.len() as u64,
            archived: false,
            owned: None,
        });
        let index = Index {
            root: root.clone(),
            resources,
            archives: 1,
            diagnostics: vec![],
        };
        Self {
            root,
            archive,
            bytes,
            index,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn outcome(r: Result<BTreeSet<u32>>) -> std::result::Result<BTreeSet<u32>, String> {
    r.map_err(|e| format!("{e:#}"))
}
fn compare(
    index: &Index,
    guids: &[u32],
    required: bool,
) -> (std::result::Result<BTreeSet<u32>, String>, Program) {
    let mut actual = Program {
        globals: "Global.iff".into(),
        text_language: Some(1),
        ..Program::default()
    };
    let mut expected = actual.clone();
    let reference = outcome(direct_reference(index, &mut expected, guids, required));
    let found = outcome(load_object_guids_inner(index, &mut actual, guids, required));
    assert_eq!(
        found, reference,
        "lookup result or complete error chain changed"
    );
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "complete Program including partial failure mutation changed"
    );
    assert_eq!(
        normalized(&actual), normalized(&expected),
        "serde-skipped normalized-definition provenance changed"
    );
    (found, actual)
}
#[test]
fn guid_scan_repeated_guids_and_multiple_definitions_keep_complete_linked_program() {
    let f = Fixture::new(&[("Objects.IFF", iff(&[(128, A), (129, B)]))]);
    let (result, program) = compare(&f.index, &[B, A, A, B], true);
    assert_eq!(result.unwrap(), BTreeSet::from([A, B]));
    assert_eq!(program.modules.len(), 2);
    assert_eq!(
        program.modules["ranges.far::Objects.IFF"]
            .object_definition_words
            .len(),
        2
    );
    assert!(program.modules["ranges.far::Objects.IFF"].normalized_object_definitions);
}
#[test]
fn guid_scan_duplicate_providers_fail_after_the_same_full_discovery() {
    let f = Fixture::new(&[
        ("first.iff", iff(&[(128, A)])),
        ("second.iff", iff(&[(129, A)])),
    ]);
    for required in [false, true] {
        let (result, program) = compare(&f.index, &[A, A], required);
        let error = result.unwrap_err();
        assert!(error.contains("ambiguous factory GUID 10203040"));
        assert!(error.contains("first.iff") && error.contains("second.iff"));
        assert!(program.modules.is_empty());
    }
}
#[test]
fn guid_scan_malformed_unrelated_iff_precedes_missing_or_ambiguous_lookup() {
    for entries in [
        vec![
            ("unrelated.iff", vec![0; 64]),
            ("first.iff", iff(&[(128, A)])),
        ],
        vec![
            ("first.iff", iff(&[(128, A)])),
            ("second.iff", iff(&[(129, A)])),
            ("unrelated.iff", vec![0; 64]),
        ],
    ] {
        let f = Fixture::new(&entries);
        for ids in [vec![A], vec![B], vec![]] {
            for required in [false, true] {
                let (result, program) = compare(&f.index, &ids, required);
                assert!(result.unwrap_err().contains("invalid Maxis IFF signature"));
                assert!(program.modules.is_empty());
            }
        }
    }
}
#[test]
fn guid_scan_missing_optional_required_and_partial_program_failure_remain_distinct() {
    let f = Fixture::new(&[("first.iff", iff(&[(128, A)]))]);
    assert!(compare(&f.index, &[B], false).0.unwrap().is_empty());
    assert!(
        compare(&f.index, &[B], true)
            .0
            .unwrap_err()
            .contains("no source for factory GUID 50607080")
    );
    assert!(compare(&f.index, &[], true).0.unwrap().is_empty());
    let (result, program) = compare(&f.index, &[B, A, A], true);
    assert!(result.unwrap_err().contains("50607080"));
    assert_eq!(
        program.modules.len(),
        2,
        "earlier sorted GUID still links before later failure"
    );
}
#[test]
fn guid_scan_owned_and_loose_reads_keep_storage_and_selected_parse_errors() {
    let mut f = Fixture::new(&[("first.iff", iff(&[(128, A)]))]);
    let r = &mut f.index.resources[0];
    r.owned = Some(Arc::from(f.bytes.clone()));
    r.source = f.root.join("not-opened");
    assert_eq!(
        compare(&f.index, &[A], true).0.unwrap(),
        BTreeSet::from([A])
    );
    f.index.resources[0].owned = Some(Arc::from(&b"bad-length"[..]));
    assert!(
        compare(&f.index, &[A], true)
            .0
            .unwrap_err()
            .contains("owned resource size differs")
    );
    f.index.resources[0].owned = None;
    f.index.resources[0].source = f.archive.clone();
    f.index.resources[0].archived = false;
    assert_eq!(
        compare(&f.index, &[A], true).0.unwrap(),
        BTreeSet::from([A])
    );
    let mut unsupported = iff(&[(128, A)]);
    unsupported[140..144].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&f.archive, &unsupported).unwrap();
    assert!(
        compare(&f.index, &[A], true)
            .0
            .unwrap_err()
            .contains("unsupported OBJD version")
    );
    // An unrelated unsupported definition is not selected for strict linking.
    assert!(compare(&f.index, &[B], false).0.unwrap().is_empty());
}
#[test]
fn guid_scan_replacement_in_place_write_truncation_and_recovery_have_no_query_cache() {
    let f = Fixture::new(&[
        ("first.iff", iff(&[(128, A)])),
        ("second.iff", iff(&[(129, B)])),
    ]);
    assert_eq!(
        compare(&f.index, &[A, B], true).0.unwrap(),
        BTreeSet::from([A, B])
    );
    let updated = [iff(&[(128, B)]), iff(&[(129, A)])].concat();
    let replacement = f.root.join("replacement");
    fs::write(&replacement, &updated).unwrap();
    fs::rename(&replacement, &f.archive).unwrap();
    let (result, program) = compare(&f.index, &[A, B], true);
    result.unwrap();
    assert_eq!(
        program.modules["ranges.far::first.iff"].object_definition_words[&128][14] as u16,
        B as u16
    );
    fs::write(&f.archive, &f.bytes).unwrap();
    compare(&f.index, &[A, B], true).0.unwrap();
    fs::write(&f.archive, &f.bytes[..f.bytes.len() - 1]).unwrap();
    assert!(
        compare(&f.index, &[A, B], true)
            .0
            .unwrap_err()
            .contains("resource outside source file")
    );
    fs::write(&f.archive, &f.bytes).unwrap();
    compare(&f.index, &[A, B], true).0.unwrap();
    fs::remove_file(&f.archive).unwrap();
    assert!(
        compare(&f.index, &[A], true)
            .0
            .unwrap_err()
            .contains("open ")
    );
    fs::write(&f.archive, &f.bytes).unwrap();
    compare(&f.index, &[A], true).0.unwrap();
}
#[test]
fn guid_scan_same_scan_revalidates_the_open_archive_between_resource_reads() {
    let f = Fixture::new(&[
        ("first.iff", iff(&[(128, A)])),
        ("second.iff", iff(&[(129, B)])),
    ]);
    let mut scan = crate::resource_scan::Scan::default();
    assert_eq!(
        scan.read(&f.index.resources[0]).unwrap(),
        f.index.resources[0].read().unwrap()
    );
    let swapped = [iff(&[(128, B)]), iff(&[(129, A)])].concat();
    let path = f.root.join("swap");
    fs::write(&path, &swapped).unwrap();
    fs::rename(&path, &f.archive).unwrap();
    assert_eq!(
        scan.read(&f.index.resources[1]).unwrap(),
        f.index.resources[1].read().unwrap()
    );
    fs::write(&f.archive, &f.bytes).unwrap();
    assert_eq!(
        scan.read(&f.index.resources[0]).unwrap(),
        f.index.resources[0].read().unwrap()
    );
    fs::write(&f.archive, &f.bytes[..64]).unwrap();
    let actual = scan
        .read(&f.index.resources[1])
        .map_err(|e| format!("{e:#}"));
    let expected = f.index.resources[1].read().map_err(|e| format!("{e:#}"));
    assert_eq!(actual, expected);
    assert!(actual.is_err());
}
#[test]
fn guid_scan_skips_non_iff_and_preserves_bad_storage_priority() {
    let mut f = Fixture::new(&[("first.iff", iff(&[(128, A)]))]);
    let mut skipped = f.index.resources[0].clone();
    skipped.name = "broken.txt".into();
    skipped.locator = "broken.txt".into();
    skipped.source = f.root.join("missing");
    skipped.size = u64::MAX;
    skipped.unpacked_size = 0;
    f.index.resources.insert(0, skipped);
    compare(&f.index, &[A], true).0.unwrap();
    f.index.resources[1].unpacked_size += 1;
    assert!(
        compare(&f.index, &[A], true)
            .0
            .unwrap_err()
            .contains("compressed FAR entry is unsupported")
    );
}

// Exact frozen pre-experiment implementation follows. Its SHA is retained in
// the experiment manifest; only its function name differs from the source.
fn direct_reference(
    index: &Index,
    program: &mut Program,
    guids: &[u32],
    required: bool,
) -> Result<BTreeSet<u32>> {
    let wanted: BTreeSet<_> = guids.iter().copied().collect();
    let mut providers: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    for resource in &index.resources {
        if !resource.name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let iff = sims_formats::iff::Iff::parse(resource.read()?)?;
        for chunk in iff.all(b"OBJD") {
            let data = iff.data(chunk);
            // Unrelated unsupported historical OBJD records don't participate in
            // this lookup; selected records still go through the strict loader.
            if data.len() < 32 {
                continue;
            }
            let guid = u32::from_le_bytes(data[28..32].try_into().unwrap());
            if wanted.contains(&guid) {
                providers
                    .entry(guid)
                    .or_default()
                    .insert(resource.locator.clone());
            }
        }
    }
    let mut loaded = BTreeSet::new();
    for guid in wanted {
        if !required && !providers.contains_key(&guid) {
            continue;
        }
        let sources = providers
            .get(&guid)
            .with_context(|| format!("no source for factory GUID {guid:08X}"))?;
        anyhow::ensure!(
            sources.len() == 1,
            "ambiguous factory GUID {guid:08X}: {sources:?}"
        );
        for (name, module) in
            crate::vm::load_program(index, sources.first().unwrap(), &program.globals)?.modules
        {
            program.modules.entry(name).or_insert(module);
        }
        loaded.insert(guid);
    }
    Ok(loaded)
}

/// The empty GUID set intentionally isolates full-discovery scan policy. Both
/// implementations still read/parse every IFF, but neither links providers.
/// This is not a representative controller-load or whole-Live benchmark.
#[test]
#[ignore = "private immutable installation; root-coordinated quiet paired scan-policy attribution only"]
fn installed_guid_discovery_direct_vs_scan_abba() -> Result<()> {
    use std::time::Instant;
    let root = PathBuf::from(std::env::var("SIMS_GUID_SCAN_ROOT")?);
    let out = PathBuf::from(std::env::var("SIMS_GUID_SCAN_OUTPUT")?);
    fs::create_dir(&out)?;
    let index = Index::scan(&root)?;
    let identity = ordered_bytes(&index)?;
    let new_program = || Program {
        globals: "Global.iff".into(),
        text_language: Some(1),
        ..Program::default()
    };
    let expected = serde_json::to_value(new_program())?;
    let mut rows = vec![];
    for round in 0..11 {
        for (position, direct) in [true, false, false, true].into_iter().enumerate() {
            // Program creation and result/state verification are outside time.
            let mut program = new_program();
            let start = Instant::now();
            let result = if direct {
                direct_reference(&index, &mut program, &[], false)
            } else {
                load_object_guids_inner(&index, &mut program, &[], false)
            };
            let elapsed_ns = start.elapsed().as_nanos();
            assert!(result?.is_empty());
            assert_eq!(serde_json::to_value(&program)?, expected);
            rows.push(serde_json::json!({"round":round,"position":position,
                "policy":if direct{"direct"}else{"scan"},"elapsed_ns":elapsed_ns,
                "warmup":round==0,"empty_result":true,"program_unchanged":true}));
        }
    }
    let after = ordered_bytes(&index)?;
    assert_eq!(
        after, identity,
        "immutable source bytes/Index identity changed during benchmark"
    );
    fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true,"rounds":10,"warmup_rounds":1,"pattern":"ABBA/direct-scan-scan-direct",
            "requested_guids":[],"required":false,"before":identity,"after":after,"rows":rows,
            "scope":"Full-discovery read/parse scan-policy only. Empty requested GUIDs do not model controller binding/provider loading or whole Live latency.",
            "checks":"Same frozen ordered Index, direct/Scan per-resource byte equality outside timing, actual IFF parse success, source digest unchanged after all rounds, exact empty result and complete Program unchanged for every timed lookup.",
            "timing":"Source/Index preparation, byte checks, Program creation, state/result verification and JSON output are outside lookup intervals; no portable latency threshold. Quiet coordination documented separately."
        }))?,
    )?;
    Ok(())
}
fn ordered_bytes(index: &Index) -> Result<serde_json::Value> {
    let mut signature = Sha256::new();
    signature.update(serde_json::to_vec(&index.resources)?);
    let mut scan = crate::resource_scan::Scan::default();
    let mut resources = 0usize;
    let mut bytes = 0usize;
    for r in &index.resources {
        if !r.name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let expected = r.read()?;
        let actual = scan.read(r)?;
        assert_eq!(
            actual, expected,
            "read policy changed bytes for {}",
            r.locator
        );
        signature.update((r.locator.len() as u64).to_le_bytes());
        signature.update(r.locator.as_bytes());
        signature.update((actual.len() as u64).to_le_bytes());
        signature.update(&actual);
        sims_formats::iff::Iff::parse(actual)?;
        resources += 1;
        bytes += expected.len();
    }
    Ok(
        serde_json::json!({"resources":resources,"bytes":bytes,"ordered_index_and_iff_bytes_sha256":format!("{:x}",signature.finalize())}),
    )
}


// Frozen immediate pre-candidate implementation, using the SAME Scan read
// policy. Only its function name differs; benchmark must not mix parser and
// earlier direct-file/Scan policy changes.
fn scan_reference(
    index: &Index,
    program: &mut Program,
    guids: &[u32],
    required: bool,
) -> Result<BTreeSet<u32>> {
    let wanted: BTreeSet<_> = guids.iter().copied().collect();
    let mut providers: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut scan = crate::resource_scan::Scan::default();
    for resource in &index.resources {
        if !resource.name.to_ascii_lowercase().ends_with(".iff") {
            continue;
        }
        let iff = sims_formats::iff::Iff::parse(scan.read(resource)?)?;
        for chunk in iff.all(b"OBJD") {
            let data = iff.data(chunk);
            // Unrelated unsupported historical OBJD records don't participate in
            // this lookup; selected records still go through the strict loader.
            if data.len() < 32 {
                continue;
            }
            let guid = u32::from_le_bytes(data[28..32].try_into().unwrap());
            if wanted.contains(&guid) {
                providers
                    .entry(guid)
                    .or_default()
                    .insert(resource.locator.clone());
            }
        }
    }
    let mut loaded = BTreeSet::new();
    for guid in wanted {
        if !required && !providers.contains_key(&guid) {
            continue;
        }
        let sources = providers
            .get(&guid)
            .with_context(|| format!("no source for factory GUID {guid:08X}"))?;
        anyhow::ensure!(
            sources.len() == 1,
            "ambiguous factory GUID {guid:08X}: {sources:?}"
        );
        for (name, module) in
            crate::vm::load_program(index, sources.first().unwrap(), &program.globals)?.modules
        {
            program.modules.entry(name).or_insert(module);
        }
        loaded.insert(guid);
    }
    Ok(loaded)
}

fn normalized(program: &Program) -> BTreeMap<String, bool> {
    program.modules.iter().map(|(name, module)|
        (name.clone(), module.normalized_object_definitions)).collect()
}

#[test]
fn guid_header_discovery_preserves_preexisting_program_and_mutable_index() {
    let mut fixture = Fixture::new(&[("first.iff", iff(&[(128, A)])),
        ("second.iff", iff(&[(129, B)]))]);
    let mut initial = Program { globals: "Global.iff".into(), text_language: Some(1),
        ..Program::default() };
    initial.modules.insert("ranges.far::first.iff".into(), sims_vm::Module {
        source: "supplied-existing-module".into(), content_hash: "supplied-old-hash".into(),
        normalized_object_definitions: false, ..sims_vm::Module::default() });
    for order in [false, true] {
        if order { fixture.index.resources.swap(0, 1); }
        for required in [false, true] {
            let mut expected = initial.clone();
            let mut actual = initial.clone();
            assert_eq!(outcome(load_object_guids_inner(&fixture.index, &mut actual, &[B,A], required)),
                outcome(scan_reference(&fixture.index, &mut expected, &[B,A], required)));
            assert_eq!(serde_json::to_value(&actual).unwrap(), serde_json::to_value(&expected).unwrap());
            assert_eq!(normalized(&actual), normalized(&expected));
            assert_eq!(actual.modules["ranges.far::first.iff"].content_hash, "supplied-old-hash");
            assert!(!actual.modules["ranges.far::first.iff"].normalized_object_definitions);
        }
    }
    // Metadata changes between calls are current input, not a cached resource ID.
    fixture.index.resources[0].name = "now-skipped.txt".into();
    compare(&fixture.index, &[B], false).0.unwrap();
}

#[test]
#[ignore = "private immutable installation; root-coordinated same-Scan parser ABBA component timing"]
fn installed_guid_header_discovery_same_scan_abba() -> Result<()> {
    use std::time::Instant;
    let root = PathBuf::from(std::env::var("SIMS_GUID_SCAN_ROOT")?);
    let out = PathBuf::from(std::env::var("SIMS_GUID_SCAN_OUTPUT")?);
    fs::create_dir(&out)?;
    let globals = std::env::var("SIMS_GUID_SCAN_GLOBALS")?;
    let specification = std::env::var("SIMS_GUID_SCAN_GUIDS")?;
    let guids = if specification.is_empty() { vec![] } else {
        specification.split(',').map(|word| u32::from_str_radix(word.trim().trim_start_matches("0x"),16))
            .collect::<std::result::Result<Vec<_>,_>>()? };
    let required = match std::env::var("SIMS_GUID_SCAN_REQUIRED")?.as_str() {
        "true" => true, "false" => false,
        _ => anyhow::bail!("SIMS_GUID_SCAN_REQUIRED must be exactly true or false") };
    let index = Index::scan(&root)?;
    let identity = ordered_bytes(&index)?;
    let fresh = Program { globals, text_language: Some(1), ..Program::default() };
    let mut linked = fresh.clone();
    let wanted = scan_reference(&index, &mut linked, &guids, required)?;
    let mut rows = vec![];
    for (profile, initial) in [("fresh-authored-Program",fresh),("already-linked-Program",linked)] {
        let mut expected = initial.clone();
        assert_eq!(scan_reference(&index,&mut expected,&guids,required)?,wanted);
        let expected_bytes = serde_json::to_vec(&expected)?;
        let expected_normalized = normalized(&expected);
        for round in 0..4 {
            for (position, old) in [true,false,false,true].into_iter().enumerate() {
                let mut program = initial.clone();
                let start = Instant::now();
                let actual = if old { scan_reference(&index,&mut program,&guids,required) }
                    else { load_object_guids_inner(&index,&mut program,&guids,required) };
                let elapsed_ns = start.elapsed().as_nanos();
                assert_eq!(actual?, wanted);
                assert_eq!(serde_json::to_vec(&program)?,expected_bytes);
                assert_eq!(normalized(&program),expected_normalized);
                rows.push(serde_json::json!({"profile":profile,"round":round,"position":position,
                    "policy":if old{"full-IFF-metadata"}else{"GUID-projection"},
                    "warmup":round==0,"elapsed_ns":elapsed_ns,"result_and_complete_program_equal":true}));
            }
        }
    }
    let after = ordered_bytes(&index)?;
    assert_eq!(after,identity,"source/Index bytes changed during component timings");
    fs::write(out.join("result.json"),serde_json::to_vec_pretty(&serde_json::json!({
        "passed":true,"pattern":"ABBA/full-projection-projection-full","measured_rounds_per_profile":3,
        "warmup_rounds_per_profile":1,"guids":guids,"required":required,"rows":rows,
        "before":identity,"after":after,"scope":"Complete GUID resource lookup including same Scan creation, exhaustive current reads, complete IFF validation, provider collection and requested module linking. Component only; no application/UI/native-parity claim.",
        "outside_timing":"Index scan, ordered byte verification, authored Program creation/cloning, result/full Program+skipped provenance comparisons, and output. No candidate map or parse work prepared outside lookup timer.",
        "empty_guid_limit":"An explicitly empty GUID list exercises complete discovery but no provider linking; report it separately from nonempty controller lookup."
    }))?)?;
    Ok(())
}
