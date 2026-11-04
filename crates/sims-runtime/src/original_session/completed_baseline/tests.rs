//! Authored structural composition controls. The private managed-state test is
//! explicitly opt-in and is not an original-engine capture or parity vector.
use super::*;
use sims_content::{Resource, owned_resources::Entry};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn resource(locator: &str, bytes: &[u8], owned: bool) -> Resource {
    Resource {
        locator: locator.into(),
        name: locator.rsplit('/').next().unwrap().into(),
        source: PathBuf::from("/absent-completed-composition-test").join(locator),
        offset: 0,
        size: bytes.len() as u64,
        unpacked_size: bytes.len() as u64,
        archived: false,
        owned: owned.then(|| Arc::from(bytes)),
    }
}
fn index() -> Index {
    Index {
        root: PathBuf::from("/absent-completed-composition-test"),
        resources: vec![resource("Global.iff", b"authored immutable module", true)],
        archives: 0,
        diagnostics: vec![],
    }
}
fn entry(locator: &str, bytes: &[u8]) -> Entry {
    Entry {
        locator: locator.into(),
        name: locator.rsplit('/').next().unwrap().into(),
        sha256: digest(bytes),
        bytes: bytes.to_vec(),
    }
}
fn owned() -> Neighborhood {
    Neighborhood {
        directory: "UserData".into(),
        entries: vec![
            entry(
                "UserData/Neighborhood.iff",
                b"current neighborhood resource",
            ),
            entry("UserData/Houses/House07.iff", b"current house resource"),
            entry(
                "UserData/Characters/User00000.iff",
                b"loaded mutable module",
            ),
        ],
    }
}
fn program() -> Program {
    Program {
        globals: "Global.iff".into(),
        modules: BTreeMap::from([
            (
                "Global.iff".into(),
                sims_vm::Module {
                    source: "Global.iff".into(),
                    content_hash: digest(b"authored immutable module"),
                    ..Default::default()
                },
            ),
            (
                "UserData/Characters/User00000.iff".into(),
                sims_vm::Module {
                    source: "UserData/Characters/User00000.iff".into(),
                    content_hash: digest(b"loaded mutable module"),
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    }
}

#[test]
fn retained_provider_aliases_rebind_to_current_owned_bytes() -> Result<()> {
    let aliases = BTreeMap::from([(
        "native-provider:00abc123".into(),
        "UserData/Characters/User00000.iff".into(),
    )]);
    let base = index();
    let first = bind_aliases(owned().mount(&base)?, Some(&aliases))?;
    assert_eq!(
        first.resources[first.find("User00000.iff")?].locator,
        "UserData/Characters/User00000.iff"
    );
    assert_eq!(
        first.resources.iter().filter(|r| r.is_iff()).count(),
        owned()
            .mount(&base)?
            .resources
            .iter()
            .filter(|r| r.is_iff())
            .count()
    );
    assert_eq!(
        first.resources[first.find("native-provider:00abc123")?].read()?,
        b"loaded mutable module"
    );
    let mut later = owned();
    later.entries[2] = entry(
        "UserData/Characters/User00000.iff",
        b"later current owned module",
    );
    // A remounted namespace must not retain the old alias's copied Arc.
    let second = bind_aliases(later.mount(&first)?, Some(&aliases))?;
    assert_eq!(
        second.resources[second.find("native-provider:00abc123")?].read()?,
        b"later current owned module"
    );
    assert_eq!(
        first.resources[first.find("native-provider:00abc123")?].read()?,
        b"loaded mutable module"
    );
    Ok(())
}

#[test]
fn retained_aliases_reject_unowned_mutable_and_noncanonical_targets() -> Result<()> {
    let mut base = index();
    base.resources.push(resource(
        "UserData/Characters/User00000.iff",
        b"absent",
        false,
    ));
    for (name, target) in [
        (
            "native-provider:00abc123",
            "UserData/Characters/User00000.iff",
        ),
        ("native-provider:00abc123", "User00000.iff"),
        ("native-provider:00abc123", "native-provider:00000000"),
        ("wrong-provider:00abc123", "Global.iff"),
        ("native-provider:garbage!", "Global.iff"),
    ] {
        assert!(
            bind_aliases(
                base.clone(),
                Some(&BTreeMap::from([(name.into(), target.into())]))
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn explicit_current_owned_subset_resolves_without_starting_file_fallback() -> Result<()> {
    let base = index();
    immutable_index(&base)?;
    assert!(base.find("UserData/Neighborhood.iff").is_err());
    let current = owned();
    let mounted = current.mount(&base)?;
    resources(
        &mounted,
        "UserData/Houses/House07.iff",
        "UserData/Neighborhood.iff",
        &program(),
    )?;
    // Only three explicitly supplied files are owned; no unused neighborhood
    // files, household constructors, or disk-backed starting files are imported.
    assert_eq!(current.entries.len(), 3);
    assert!(mounted.find("UserData/Houses/House05.iff").is_err());
    for e in &current.entries {
        assert_eq!(owned_resource(&mounted, &e.locator)?, e.sha256);
    }
    Ok(())
}

#[test]
fn parsed_house_metadata_geometry_keeps_managed_restore_limits() {
    for size in [3, 32, 64] {
        assert!(house_geometry(size, size, 2).is_ok());
    }
    for (width, height, floors) in [
        (2, 2, 2),
        (65, 65, 2),
        (32, 33, 2),
        (32, 32, 1),
        (32, 32, 3),
    ] {
        assert!(house_geometry(width, height, floors).is_err());
    }
}

#[test]
fn missing_owned_closure_hash_mismatch_and_resident_version_ambiguity_stop() -> Result<()> {
    let mut disk = index();
    disk.resources.push(resource(
        "UserData/Neighborhood.iff",
        b"never opened",
        false,
    ));
    assert!(
        immutable_index(&disk)
            .unwrap_err()
            .to_string()
            .contains("mutable starting-save")
    );
    assert!(
        owned_resource(&disk, "UserData/Neighborhood.iff")
            .unwrap_err()
            .to_string()
            .contains("no owned bytes")
    );
    let mut source = owned();
    source.entries.pop();
    let mounted = source.mount(&index())?;
    assert!(
        resources(
            &mounted,
            "UserData/Houses/House07.iff",
            "UserData/Neighborhood.iff",
            &program()
        )
        .is_err()
    );
    let mounted = owned().mount(&index())?;
    let mut p = program();
    p.modules.get_mut("Global.iff").unwrap().content_hash = digest(b"different immutable source");
    assert!(
        resources(
            &mounted,
            "UserData/Houses/House07.iff",
            "UserData/Neighborhood.iff",
            &p
        )
        .unwrap_err()
        .to_string()
        .contains("immutable linked module")
    );
    let mut source = owned();
    source.entries[2] = entry(
        "UserData/Characters/User00000.iff",
        b"new current bytes after the resident module was loaded",
    );
    let mounted = source.mount(&index())?;
    assert!(
        resources(
            &mounted,
            "UserData/Houses/House07.iff",
            "UserData/Neighborhood.iff",
            &program()
        )
        .unwrap_err()
        .to_string()
        .contains("resident mutable module/current resource version mapping unsupported")
    );
    let mounted = owned().mount(&index())?;
    let mut p = program();
    p.modules
        .get_mut("Global.iff")
        .unwrap()
        .content_hash
        .clear();
    assert!(
        resources(
            &mounted,
            "UserData/Houses/House07.iff",
            "UserData/Neighborhood.iff",
            &p
        )
        .unwrap_err()
        .to_string()
        .contains("source hash is unknown")
    );
    Ok(())
}

fn authored_simulation() -> sims_sim::Simulation {
    // Test-only construction of authored empty components, not a native input
    // normalizer. The production composition method invokes none of these.
    let mut vm = sims_vm::Vm::new(
        sims_vm::World {
            scheduler_storage: Some(sims_vm::scheduler::storage::Storage::empty_known()),
            ..Default::default()
        },
        0x12345678,
    );
    vm.tick = 123;
    vm.rng_state = 0x12345678;
    vm.lot_rng_state = Some(0x456789ab);
    vm.crt_rng_state = Some(0x89abcdef);
    let mut s = sims_sim::Simulation::new(
        vm,
        sims_sim::Room {
            width: 16,
            height: 16,
            obstacles: vec![],
        },
        BTreeMap::new(),
    )
    .unwrap();
    s.native_scheduler = Some(
        serde_json::from_value(serde_json::json!({"cursor":0,"pass":null,"tick":null})).unwrap(),
    );
    s
}

#[test]
fn supplied_snapshot_version_driver_and_all_state_validation_remain_unchanged() -> Result<()> {
    let p = Program::default();
    let state = authored_simulation();
    let snapshot = Snapshot::capture(&p, &state);
    let before = postcard::to_allocvec(&state)?;
    let restored = simulation(&p, snapshot.clone())?;
    assert_eq!(postcard::to_allocvec(&restored)?, before);
    let cold: Snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot)?)?;
    assert_eq!(postcard::to_allocvec(&simulation(&p, cold)?)?, before);
    let mut bad = snapshot.clone();
    bad.version += 100;
    assert!(simulation(&p, bad).is_err());
    let mut bad = snapshot.clone();
    bad.program_hash = "unqualified program".into();
    assert!(simulation(&p, bad).is_err());
    let mut bad = snapshot.clone();
    bad.state.native_scheduler = None;
    assert!(
        simulation(&p, bad)
            .unwrap_err()
            .to_string()
            .contains("original registry driver")
    );
    let mut bad = snapshot;
    bad.state.room.width = 0;
    assert!(simulation(&p, bad).is_err());
    assert_eq!(postcard::to_allocvec(&state)?, before);
    Ok(())
}

#[derive(serde::Deserialize)]
struct PrivatePlayerCheckpoint {
    session: super::super::Saved,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ColdOwners {
    mutable: Neighborhood,
    house: House,
    simulation: Snapshot,
    assets: live::Snapshot,
    view: View,
    options: Options,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn private_input() -> Result<(Input, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let immutable = Arc::new(Index::scan(
        &root.join("cache/managed-independent-20261022-third/immutable-assets"),
    )?);
    immutable_index(&immutable)?;
    let store = sims_fixtures::Store::open(
        root.join("fixtures/private/player-newbie-carrier-writeback-20261026-second"),
    )?;
    let source = store.load_case("after-actual-carrier-removal")?.source.blob;
    ensure!(
        source.sha256 == "2460f2b71cd9b7e8559858c3c0c24a1e8d8a8211261e2da3dfefd67668dd9f12",
        "private authored starting checkpoint differs"
    );
    let saved: PrivatePlayerCheckpoint = store.get_value(&source)?;
    ensure!(
        saved.session.version == 2,
        "managed explicit source required"
    );
    let mutable = saved
        .session
        .managed
        .as_ref()
        .context("owned namespace absent")?
        .owned
        .clone();
    // Existing managed restoration is used only to obtain a qualified authored
    // live input for this test. The new seam itself never calls Session::restore.
    let s = Session::restore(&immutable, saved.session)?;
    ensure!(
        !s.simulation.characters.is_empty(),
        "actual live Character required"
    );
    let before = postcard::to_allocvec(&s.save())?;
    let mut used = std::collections::BTreeSet::from([
        s.house.source.clone(),
        s.house.neighborhood_source.clone(),
    ]);
    used.extend(
        s.program
            .modules
            .keys()
            .filter(|n| mutable_locator(n))
            .cloned(),
    );
    let mutable = Neighborhood {
        directory: mutable.directory,
        entries: mutable
            .entries
            .into_iter()
            .filter(|e| used.contains(&e.locator))
            .collect(),
    };
    let snapshot = Snapshot::capture(&s.program, &s.simulation);
    let input = Input {
        immutable,
        mutable,
        house: s.house,
        program: s.program,
        simulation: snapshot,
        assets: s.host.snapshot(),
        view: s.host.presentation,
        options: s.options,
    };
    Ok((input, before))
}

#[test]
#[ignore = "private authored Newbie managed live state; read-only no steps; not native baseline parity"]
fn actual_owned_session_composes_and_cold_restores_exactly_without_startup() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| -> Result<()> {
            let (input, before) = private_input()?;
            ensure!(
                input.mutable.entries.len() < 35,
                "unused full neighborhood namespace must not be required"
            );
            let mut missing = input.mutable.clone();
            missing.entries.retain(|e| e.locator != input.house.source);
            ensure!(
                resources(
                    &missing.mount(&input.immutable)?,
                    &input.house.source,
                    &input.house.neighborhood_source,
                    &input.program
                )
                .is_err(),
                "missing used source silently fell back"
            );
            let saved_input_assets = postcard::to_allocvec(&input.assets)?;
            let immutable = input.immutable.clone();
            let owned = input.mutable.clone();
            let s = Session::from_completed_owned(input)?;
            ensure!(
                postcard::to_allocvec(&s.save())? == before,
                "complete Session changed during composition"
            );
            ensure!(
                postcard::to_allocvec(&s.host.snapshot())? == saved_input_assets,
                "live asset owner changed"
            );
            // A cold composition consumes independently decoded live owners.
            // It does not relink via v1 starting-house loading or run v2's
            // optional animation-table completion after validating the state.
            // Current snapshots use omitted map fields; MessagePack is the
            // existing self-describing fixture protocol. Postcard above is a
            // bit-exact comparison encoding, not a positional save decoder.
            let path = std::env::temp_dir()
                .join(format!("sims-completed-owned-cold-{}", std::process::id()));
            let store = sims_fixtures::Store::create(&path)?;
            let _scratch = Scratch(path);
            let blob = store.put_value(&ColdOwners {
                mutable: owned,
                house: s.house.clone(),
                simulation: Snapshot::capture(&s.program, &s.simulation),
                assets: s.host.snapshot(),
                view: s.host.presentation.clone(),
                options: s.options.clone(),
            })?;
            let cold: ColdOwners = store.get_value(&blob)?;
            store.verify_all()?;
            let cold = Input {
                immutable,
                mutable: cold.mutable,
                house: cold.house,
                program: s.program.clone(),
                simulation: cold.simulation,
                assets: cold.assets,
                view: cold.view,
                options: cold.options,
            };
            let restored = Session::from_completed_owned(cold)?;
            ensure!(
                postcard::to_allocvec(&restored.save())? == before,
                "independent cold composition changed complete state/RNG"
            );
            // Same supplied body/rig/geometry and all clock/routing/neighborhood
            // owners; no VM instruction, draw or UI input is performed by this test.
            let (mut mismatched, unchanged) = private_input()?;
            mismatched.options.routing = sims_sim::lot_route::path::Policy::Strict;
            ensure!(
                Session::from_completed_owned(mismatched).is_err(),
                "routing profile mismatch was repaired"
            );
            let (mut mismatched, _) = private_input()?;
            let actor = *mismatched
                .simulation
                .state
                .characters
                .keys()
                .next()
                .context("live Character absent")?;
            mismatched.view.selected = if mismatched.view.selected == Some(actor) {
                None
            } else {
                Some(actor)
            };
            ensure!(
                Session::from_completed_owned(mismatched).is_err(),
                "presentation/Character context mismatch was repaired"
            );
            ensure!(
                unchanged == before,
                "fixture changed between independent source reads"
            );
            Ok(())
        })?
        .join()
        .expect("private completed composition worker panicked")
}
