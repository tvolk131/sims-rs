//! Storage/owner protocol checks, not original-engine parity.
use super::*;
use crate::original_session::completed_baseline::Input;
use sims_content::Resource;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
fn entry(source: &str, bytes: &[u8]) -> Entry {
    Entry {
        locator: source.into(),
        name: source.rsplit('/').next().unwrap().into(),
        sha256: digest(bytes),
        bytes: bytes.into(),
    }
}
fn namespace() -> Neighborhood {
    Neighborhood {
        directory: "UserData".into(),
        entries: vec![
            entry("UserData/Neighborhood.iff", b"owned neighborhood"),
            entry("UserData/Houses/House07.iff", b"current house"),
            entry("UserData/Portraits/Face.bmp", b"owned non-IFF input"),
        ],
    }
}
fn external(source: &str) -> Resource {
    Resource {
        locator: source.into(),
        name: source.rsplit('/').next().unwrap().into(),
        source: PathBuf::from("/must-not-read-an-external-starting-save"),
        offset: 0,
        size: 12,
        unpacked_size: 12,
        archived: false,
        owned: None,
    }
}
fn index() -> Index {
    Index {
        root: PathBuf::from("/denied-starting-save-installation"),
        resources: vec![
            external("UserData/Houses/House07.iff"),
            external("UserData2/Other.iff"),
        ],
        archives: 0,
        diagnostics: vec![],
    }
}
#[test]
fn all_owned_extensions_survive_and_unowned_capture_rejects_before_read() -> Result<()> {
    let source = namespace();
    let mut mounted = mount(&index(), &source)?;
    let captured = capture(&mounted, "UserData/Neighborhood.iff")?;
    assert_eq!(captured.hashes(), source.hashes());
    assert!(captured.entries.iter().any(|e| e.locator.ends_with(".bmp")));
    mounted.resources.push(external("UserData/Unowned.iff"));
    assert!(
        capture(&mounted, "UserData/Neighborhood.iff")
            .unwrap_err()
            .to_string()
            .contains("unowned")
    );
    Ok(())
}
#[test]
fn missing_owned_resource_cannot_fall_back_to_any_external_neighborhood() -> Result<()> {
    let mut source = namespace();
    source.entries.retain(|e| !e.locator.contains("House07"));
    let mounted = mount(&index(), &source)?;
    assert!(mounted.find("UserData/Houses/House07.iff").is_err());
    assert!(mounted.find("UserData2/Other.iff").is_err());
    assert!(owned(&mounted, "UserData/Houses/House07.iff").is_err());
    assert!(mounted.resources.iter().all(|r| r.owned.is_some()));
    Ok(())
}
#[test]
fn current_and_immutable_version_mismatches_reject_without_hash_repair() -> Result<()> {
    let mut mounted = mount(&index(), &namespace())?;
    let mut immutable = external("Global.iff");
    immutable.owned = Some(Arc::from(b"immutable asset".as_slice()));
    immutable.size = 15;
    immutable.unpacked_size = 15;
    mounted.resources.push(immutable);
    let required = BTreeMap::from([("Global.iff".into(), digest(b"changed asset"))]);
    assert!(
        validate_hashes(&mounted, &required)
            .unwrap_err()
            .to_string()
            .contains("current source version differs")
    );
    let required = BTreeMap::from([(
        "UserData/Houses/House07.iff".into(),
        digest(b"historical house"),
    )]);
    assert!(validate_hashes(&mounted, &required).is_err());
    let mut p = Program::default();
    p.modules.insert(
        "Global.iff".into(),
        sims_vm::Module {
            content_hash: digest(b"resident version"),
            ..Default::default()
        },
    );
    let before = p.fingerprint();
    assert!(
        validate_modules(
            &p,
            &BTreeMap::from([("Global.iff".into(), digest(b"current version"))])
        )
        .is_err()
    );
    assert_eq!(p.fingerprint(), before);
    Ok(())
}
#[derive(Deserialize)]
struct PrivateCheckpoint {
    session: Saved,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn private_source() -> Result<(Arc<Index>, Saved)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let immutable = Arc::new(Index::scan(
        &root.join("cache/managed-independent-20261022-third/immutable-assets"),
    )?);
    completed_baseline::immutable_index(&immutable)?;
    let store = sims_fixtures::Store::open(
        root.join("fixtures/private/player-newbie-carrier-writeback-20261026-second"),
    )?;
    let source = store.load_case("after-actual-carrier-removal")?.source.blob;
    ensure!(
        source.sha256 == "2460f2b71cd9b7e8559858c3c0c24a1e8d8a8211261e2da3dfefd67668dd9f12",
        "private fixture changed"
    );
    let checkpoint: PrivateCheckpoint = store.get_value(&source)?;
    Ok((immutable, checkpoint.session))
}
fn mark_unlinked_house(source: &mut Neighborhood, house: &str) -> Result<String> {
    let entry = source
        .entries
        .iter_mut()
        .find(|e| e.locator == house)
        .context("owned house absent")?;
    let old = entry.sha256.clone();
    // A valid authored unknown IFF chunk, never a claimed native game write.
    // Existing parsed historical House/live state are deliberately unchanged.
    let iff = sims_formats::iff::Iff::parse(entry.bytes.clone())?;
    ensure!(
        iff.get(b"NOTE", 0x7ffd).is_none(),
        "authored test marker already present"
    );
    entry.bytes.extend_from_slice(b"NOTE");
    entry.bytes.extend_from_slice(&76u32.to_be_bytes());
    entry.bytes.extend_from_slice(&0x7ffdu16.to_be_bytes());
    entry.bytes.extend_from_slice(&0u16.to_be_bytes());
    let mut label = [0; 64];
    let text = b"AUTHORED current-owned storage provenance control";
    label[..text.len()].copy_from_slice(text);
    entry.bytes.extend_from_slice(&label);
    sims_formats::iff::Iff::parse(entry.bytes.clone())?;
    entry.sha256 = digest(&entry.bytes);
    ensure!(entry.sha256 != old, "source version did not change");
    Ok(old)
}
#[test]
#[ignore = "private live Newbie state/assets; actual managed Store roundtrip, no VM steps/native execution"]
fn actual_completed_owned_default_save_store_restore_and_resave_preserve_policy_and_state()
-> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| -> Result<()> {
        let (immutable, saved) = private_source()?;
        ensure!(saved.managed.as_ref().unwrap().schema == 1, "expected historical schema1 source");
        let mut current = saved.managed.as_ref().unwrap().owned.clone();
        let mut original = Session::restore(&immutable, saved)?; // Existing schema1 branch remains exercised.
        ensure!(original.save_managed()?.managed.unwrap().schema == 1, "legacy save policy changed");
        let historical = mark_unlinked_house(&mut current, &original.house.source)?;
        ensure!(original.house.sources[&original.house.source] == historical, "historical receipt differs");
        ensure!(!original.program.modules.contains_key(&original.house.source), "house must not be a resident module in this authored control");
        let old_index = original.index.clone();
        original.index = Arc::new(current.mount(&immutable)?);
        ensure!(original.save_managed().is_err(), "schema1 historical source-change detection weakened");
        original.index = old_index;
        // Pin an actually present optional immutable cache entry as absent in
        // this declared catalog; it is not a native observation. Native cache
        // completion must not re-add it after validating schema2's fingerprint.
        let globals = original.program.globals.clone();
        let table = [152, 153, 158, 159, 160].into_iter().find(|id| original.program.modules[&globals].strings.contains_key(id))
            .context("private optional native table absent")?;
        original.program.modules.get_mut(&globals).unwrap().strings.remove(&table);
        let expected_program = original.program.fingerprint();
        let expected = postcard::to_allocvec(&original.save())?;
        let expected_rng = (original.simulation.vm.rng_state, original.simulation.vm.lot_rng_state, original.simulation.vm.crt_rng_state);
        let expected_tick = original.simulation.vm.tick;
        let snapshot = sims_sim::Snapshot::capture(&original.program, &original.simulation);
        let input = Input { immutable: immutable.clone(), mutable: current, house: original.house,
            program: original.program, simulation: snapshot,
            assets: original.host.snapshot(), view: original.host.presentation, options: original.options };
        let session = Session::from_completed_owned(input)?;
        ensure!(postcard::to_allocvec(&session.save())? == expected, "completed composition changed supplied state");
        let saved = session.save_managed()?; ensure!(saved.managed.as_ref().unwrap().schema == 2, "completed default save policy missing");
        let fork = session.fork_for_exploration()?;
        ensure!(fork.save_managed()?.managed.unwrap().schema == 2, "fork lost completed save policy");
        drop(fork);
        let path = std::env::temp_dir().join(format!("sims-completed-managed-cold-{}", std::process::id()));
        let _scratch = Scratch(path.clone()); let store = sims_fixtures::Store::create(path)?;
        let blob = store.put_value(&saved)?; let decoded: Saved = store.get_value(&blob)?; store.verify_all()?;
        // Even a caller Index carrying denied external starting paths cannot
        // influence the schema2 resolver or repair a missing embedded entry.
        let mut full = (*immutable).clone(); full.resources.push(external(&session.house.source));
        full.resources.push(external("UserData2/Other.iff"));
        let restored = Session::restore(&full, decoded)?;
        ensure!(postcard::to_allocvec(&restored.save())? == expected, "actual managed cold restore changed complete state/RNG");
        ensure!(restored.program.fingerprint() == expected_program, "catalog changed after exact validation");
        ensure!(!restored.program.modules[&globals].strings.contains_key(&table), "optional native table was repaired");
        ensure!((restored.simulation.vm.rng_state, restored.simulation.vm.lot_rng_state, restored.simulation.vm.crt_rng_state) == expected_rng
            && restored.simulation.vm.tick == expected_tick, "restore advanced clock/RNG");
        ensure!(postcard::to_allocvec(&restored.save_managed()?)? == postcard::to_allocvec(&saved)?, "default resave lost schema2 policy/current sources");
        let mut missing: Saved = store.get_value(&blob)?;
        missing.managed.as_mut().unwrap().owned.entries.retain(|e| e.locator != session.house.source);
        ensure!(Session::restore(&full, missing).is_err(), "missing owned source fell back to original path");
        let mut bad: Saved = store.get_value(&blob)?;
        bad.managed.as_mut().unwrap().owned.entries[0].bytes.push(0);
        ensure!(Session::restore(&full, bad).is_err(), "owned hash corruption accepted");
        let mut changed_immutable = full.clone();
        let resource = &mut changed_immutable.resources[full.find(&globals)?];
        resource.owned = Some(Arc::from(b"wrong immutable resource".as_slice()));
        resource.size = 24; resource.unpacked_size = 24; resource.archived = false; resource.offset = 0;
        let cold: Saved = store.get_value(&blob)?;
        ensure!(Session::restore(&changed_immutable, cold).is_err(), "immutable source mismatch accepted");
        println!("completed managed schema2: tick={expected_tick} rng={expected_rng:?} catalog_table_absent={table} blob={} source_house_history={historical}; no steps/native parity", blob.sha256);
        Ok(())
    })?.join().expect("completed managed test panicked")
}
