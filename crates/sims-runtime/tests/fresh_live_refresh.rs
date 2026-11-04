use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session, interface::Progress};
use sims_sim::house_valuation as value;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "private frozen Goth session, actual original baseline and immutable installation"]
fn fresh_live_refreshes_goth_before_mask_publication_without_running_scripts() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("fresh Live refresh test panicked")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn saved_bytes(s: &Session) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&s.save())?)
}
fn unchanged_execution(before: &sims_sim::Simulation, after: &sims_sim::Simulation) -> Result<()> {
    let mut vm = after.vm.clone();
    // Publication changes world-owned globals/family/room state; all VM-owned
    // streams, threads, callbacks, effects and instruction continuations stay.
    vm.world = before.vm.world.clone();
    ensure!(
        vm == before.vm,
        "refresh advanced execution, a stream or VM work"
    );
    Ok(())
}
fn cold(
    index: &Index,
    s: &Session,
    reports: &mut Vec<serde_json::Value>,
    label: &str,
) -> Result<Session> {
    let encoded = serde_json::to_vec(&s.save_managed()?)?;
    let restored = Session::restore(index, serde_json::from_slice(&encoded)?)?;
    let before = saved_bytes(s)?;
    ensure!(
        before == saved_bytes(&restored)?,
        "{label}: independent cold restore differs"
    );
    reports.push(
        serde_json::json!({"boundary":label,"research_save_sha256":digest(&before),
        "managed_save_sha256":digest(&encoded),"bytes":encoded.len(),"equal":true}),
    );
    Ok(restored)
}
fn guarded_error(s: &mut Session, expected: &str) -> Result<()> {
    let before = saved_bytes(s)?;
    let error = s.begin_live_interface_update().unwrap_err().to_string();
    ensure!(error == expected, "guard error {error:?} != {expected:?}");
    ensure!(before == saved_bytes(s)?, "guard changed state: {expected}");
    Ok(())
}
fn run() -> Result<()> {
    let repo = std::env::var_os("SIMS_REFRESH_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let repo = repo.canonicalize()?;
    let output = repo.join(std::env::var("SIMS_LIVE_REFRESH_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let fixture = repo.join(
        "cache/native-parity-guest-20261024-sixth/zero-replay-first/opened-managed-session.json",
    );
    let fixture_bytes = std::fs::read(&fixture)?;
    let mut saved: Saved = serde_json::from_slice(&fixture_bytes)?;
    ensure!(saved.version == 2 && saved.simulation.state.vm.tick == 88444);
    ensure!(
        saved
            .view
            .interface_update
            .as_ref()
            .is_some_and(|s| !s.pending())
    );
    ensure!(saved.simulation.state.vm.world.globals[&37] == 0);
    ensure!(
        saved
            .simulation
            .state
            .household
            .as_ref()
            .unwrap()
            .valuation
            .is_none()
    );
    // A supplied caller-entry fixture: the original historical session has
    // already completed the OLD partial interface publication. Remove only
    // that completed owner to exercise the fresh caller again, with its real
    // unchanged Goth VM/object state. This is not an actual native boundary.
    saved.view.interface_update = None;
    let index =
        Index::scan(&repo.join("cache/native-parity-guest-20261023-second/assets-immutable"))?;
    let mut actual = Session::restore(&index, saved)?;
    let before = actual.simulation.clone();
    let input = saved_bytes(&actual)?;
    let contributors: Vec<_> = actual
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .unwrap()
        .object_order
        .iter()
        .filter_map(|id| {
            let o = &actual.simulation.vm.world.objects[id];
            (o.data.get(&59) == Some(&4)).then_some((*id, o.definition.as_ref().unwrap().guid))
        })
        .collect();
    // Actual original Goth baseline88444: d476ccd23911f8ba3a9b9ea3b91376e18808f8a99b205dc061e509432aff6edd.
    // These nine category4 contributors produce global37 bit0 via444cb9.
    let mut sorted = contributors.clone();
    sorted.sort();
    ensure!(
        sorted
            == vec![
                (124, 1176208024),
                (125, 2975863671),
                (126, 1256020925),
                (127, 2774183876),
                (128, 2746028808),
                (129, 1256158234),
                (211, 1329959665),
                (212, 1390929335),
                (213, 2105705220)
            ],
        "Goth capability contributors changed: {sorted:?}"
    );
    let mut reference = actual.fork_for_exploration()?;
    let publication = reference.refresh_original_house()?;
    ensure!(
        publication.globals[2] == 1,
        "shared native valuation capability differs"
    );
    actual.begin_live_interface_update()?;
    ensure!(
        actual.simulation.vm.world.globals[&37] == 1,
        "fresh Live omitted original valuation/capability publication"
    );
    ensure!(
        actual.simulation == reference.simulation,
        "fresh Live differs from shared416534 effects"
    );
    let mut view = actual.host.presentation.clone();
    view.interface_update = None;
    ensure!(
        view == reference.host.presentation,
        "fresh Live changed unexpected presentation state"
    );
    ensure!(
        actual.host.snapshot() == reference.host.snapshot(),
        "fresh Live changed resident assets"
    );
    ensure!(
        actual
            .host
            .presentation
            .interface_update
            .as_ref()
            .is_some_and(|s| s.pending())
    );
    unchanged_execution(&before, &actual.simulation)?;

    // Independent complete expected Simulation: native publication stores plus
    // room and route invalidation, without merely comparing two caller paths.
    let mut expected = before.clone();
    let family = expected.vm.world.person.as_ref().unwrap().controlled_family;
    expected
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .family_metadata
        .get_mut(&family)
        .unwrap()
        .flags = publication.family_flags;
    expected.household.as_mut().unwrap().valuation = Some(value::Retained {
        objects_value: publication.objects_value,
        architecture_value: publication.architecture_value,
    });
    for (key, n) in [23, 22, 37].into_iter().zip(publication.globals) {
        expected.vm.world.globals.insert(key, n);
    }
    let active: Vec<_> = expected
        .room_environment
        .as_ref()
        .unwrap()
        .manager
        .rooms
        .iter()
        .filter_map(|(&id, room)| room.active.then_some(id))
        .collect();
    let refresh = expected.vm.world.refresh.as_mut().unwrap();
    for id in active {
        refresh.rooms.get_mut(&id).unwrap().dirty = true;
    }
    refresh.dirty_categories = 3;
    let people = &expected.vm.world.person.as_ref().unwrap().person_order;
    for control in expected
        .route_control
        .values_mut()
        .filter(|c| people.contains(&c.caller))
    {
        if let Some(c) = &mut control.original {
            c.cache_valid = false;
        }
    }
    for cache in expected.portal_cache.values_mut() {
        for cost in cache.costs.values_mut() {
            *cost = 10001.;
        }
    }
    ensure!(
        actual.simulation == expected,
        "fresh Live changed simulation beyond complete native refresh effects"
    );

    // Existing entry guards stay before the new call and do not create an
    // owner or mutate state when entry is rejected.
    let mut duplicate = actual.fork_for_exploration()?;
    guarded_error(&mut duplicate, "fresh interface publication already bound")?;
    let mut blocked = reference.fork_for_exploration()?;
    blocked.host.presentation.awaiting_house_open = true;
    blocked
        .host
        .presentation
        .presenter
        .as_mut()
        .unwrap()
        .camera
        .as_mut()
        .unwrap()
        .settings
        .application_mode = 0;
    blocked.simulation.room_environment = None;
    guarded_error(
        &mut blocked,
        "fresh interface update overlaps synchronous work",
    )?;
    let mut mode = reference.fork_for_exploration()?;
    mode.host
        .presentation
        .presenter
        .as_mut()
        .unwrap()
        .camera
        .as_mut()
        .unwrap()
        .settings
        .application_mode = 0;
    mode.simulation.room_environment = None;
    guarded_error(
        &mut mode,
        "fresh interface requires actual Live camera mode",
    )?;
    let mut no_rooms = reference.fork_for_exploration()?;
    no_rooms.simulation.room_environment = None;
    guarded_error(&mut no_rooms, "house refresh room manager absent")?;

    // Failure in a later shared refresh child retains its earlier room/route
    // writes but must not install the subsequent interface owner.
    let mut faulty = reference.fork_for_exploration()?;
    faulty.simulation.architecture = None;
    let refresh = faulty.simulation.vm.world.refresh.as_mut().unwrap();
    refresh.dirty_categories = 0;
    for room in refresh.rooms.values_mut() {
        room.dirty = false;
    }
    let faulty_before = faulty.simulation.clone();
    let mut faulty_reference = faulty.fork_for_exploration()?;
    let expected_error = faulty_reference
        .refresh_original_house()
        .unwrap_err()
        .to_string();
    ensure!(expected_error == "valuation architecture absent");
    ensure!(
        faulty_reference.simulation != faulty_before,
        "later child fault lost earlier room writes"
    );
    ensure!(
        faulty
            .begin_live_interface_update()
            .unwrap_err()
            .to_string()
            == expected_error
    );
    ensure!(
        faulty.simulation == faulty_reference.simulation
            && faulty.host.presentation.interface_update.is_none()
    );

    let mut restores = vec![];
    let mut restored = cold(&index, &actual, &mut restores, "after_refresh_before_mask")?;
    let mut services = vec![];
    for ordinal in 0..16 {
        let a = actual.advance_interface_update(1)?;
        let b = restored.advance_interface_update(1)?;
        ensure!(
            a == b && saved_bytes(&actual)? == saved_bytes(&restored)?,
            "interface stage{ordinal} differs after cold restore"
        );
        unchanged_execution(&before, &actual.simulation)?;
        let mut stage_expected = expected.clone();
        stage_expected
            .vm
            .world
            .globals
            .insert(19, actual.simulation.vm.world.globals[&19]);
        ensure!(
            actual.simulation == stage_expected,
            "interface stage changed world beyond native Live-mode global19"
        );
        restored = cold(&index, &actual, &mut restores, &format!("stage{ordinal}"))?;
        match a {
            Progress::Complete => break,
            Progress::Pending => {}
            Progress::Service(request) => {
                services.push(request.clone());
                actual.acknowledge_interface_service(&request)?;
                restored.acknowledge_interface_service(&request)?;
                ensure!(saved_bytes(&actual)? == saved_bytes(&restored)?);
            }
        }
    }
    ensure!(
        !actual.interface_update_pending(),
        "interface did not finish bounded publication"
    );
    ensure!(actual.simulation.vm.world.globals[&37] == 1);
    std::fs::write(output.join("caller-entry-research.json"), &input)?;
    std::fs::write(
        output.join("final-managed.json"),
        serde_json::to_vec(&actual.save_managed()?)?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"Supplied-entry Rust Goth caller regression against native-derived shared416534 subsystem; not a fresh native whole-household comparison.",
            "fixture":fixture,"fixture_sha256":digest(&fixture_bytes),"caller_fixture_transform":"remove only completed old interface owner",
            "tick":actual.simulation.vm.tick,"contributors":sorted,"publication":publication,
            "full_simulation_expected_effects_equal":true,"shared_refresh_equal":true,"execution_rng_threads_unchanged":true,
            "guards_preserved":4,"later_child_error_preserves_prior_writes_without_interface_owner":true,
            "services":services,"cold_boundaries":restores,"strict_original_ticks_added":0
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private historical inferred-route Newbie checkpoint; no native game execution"]
fn fresh_live_refresh_invalidates_an_existing_walk_without_resuming_it() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_walk)?
        .join()
        .expect("fresh Live walk refresh test panicked")
}
fn run_walk() -> Result<()> {
    let repo = std::env::var_os("SIMS_REFRESH_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let repo = repo.canonicalize()?;
    let output = repo.join(std::env::var("SIMS_LIVE_REFRESH_WALK_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let fixture =
        repo.join("cache/verified-input-routing-20261021/replay-inferred/final-session.json");
    let bytes = std::fs::read(&fixture)?;
    let mut saved: Saved = serde_json::from_slice(&bytes)?;
    ensure!(
        saved
            .view
            .interface_update
            .as_ref()
            .is_some_and(|s| !s.pending())
    );
    saved.view.interface_update = None;
    let source_index = Index::scan(&repo.join("cache/wine-parity-import-1/assets"))?;
    let mut actual = Session::restore(&source_index, saved)?;
    let before = actual.simulation.clone();
    let walk = before.characters[&105].native_walk.as_ref().unwrap();
    ensure!(walk.request == 60 && walk.thread == 283);
    ensure!(
        before.route_control[&60]
            .original
            .as_ref()
            .unwrap()
            .cache_valid
    );
    let mut reference = actual.fork_for_exploration()?;
    reference.refresh_original_house()?;
    actual.begin_live_interface_update()?;
    ensure!(
        !actual.simulation.route_control[&60]
            .original
            .as_ref()
            .unwrap()
            .cache_valid,
        "fresh Live omitted retained walk-context invalidation"
    );
    ensure!(actual.simulation == reference.simulation);
    ensure!(actual.simulation.characters[&105].native_walk.as_ref() == Some(walk));
    ensure!(actual.simulation.vm.world.positions == before.vm.world.positions);
    ensure!(actual.simulation.vm.world.actions == before.vm.world.actions);
    unchanged_execution(&before, &actual.simulation)?;
    let immutable =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let mut restores = vec![];
    let _restored = cold(
        &immutable,
        &actual,
        &mut restores,
        "walk_refresh_before_mask",
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "scope":"Historical inferred-route Rust Newbie continuation, supplied caller entry; not original gameplay parity.",
            "fixture":fixture,"fixture_sha256":digest(&bytes),"tick":actual.simulation.vm.tick,
            "actor":105,"request":60,"thread":283,"cache_invalidated":true,
            "full_shared_refresh_equal":true,"walk_positions_actions_execution_rng_unchanged":true,
            "cold_boundaries":restores,"strict_original_ticks_added":0
        }))?,
    )?;
    Ok(())
}
