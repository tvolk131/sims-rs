//! Original-house component fixture with a synthetic actor Update host. This
//! exercises physical graph/service handoff, not original household scheduling
//! or playable activation. Use original_scheduler.rs for registry-pass checks.
#![recursion_limit = "256"]
use serde_json::json;
use sims_sim::{Simulation, Snapshot, StepStop, lot_runtime};
use std::{fs, path::PathBuf};
#[path = "support/route_clearance.rs"]
mod route_clearance;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "requires original installed homes; records live scheduler boundaries"]
fn original_houses_keep_physical_graph_through_scheduler_handoff() {
    run_houses(sims_sim::lot_route::path::Policy::Strict);
}

#[test]
#[ignore = "requires original installed homes; exercises explicitly inferred routing"]
fn original_houses_walk_with_documented_inferences() {
    run_houses(sims_sim::lot_route::path::Policy::InferredRectangles);
}

fn run_houses(policy: sims_sim::lot_route::path::Policy) {
    let tick_limit = std::env::var("SIMS_ORIGINAL_HOUSE_TICKS")
        .map(|s| s.parse::<u64>().expect("integer original-house tick limit"))
        .unwrap_or(1000);
    assert!((1..=1_000_000).contains(&tick_limit));
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut report = vec![];
    let house_filter = std::env::var("SIMS_ORIGINAL_HOUSE_FILTER").ok();
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .filter(|source| {
        house_filter
            .as_ref()
            .is_none_or(|filter| source.contains(filter))
    }) {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut loaded = house
            .load_frozen(
                &index,
                sims_vm::loading::HouseContext {
                    simulator: sims_vm::loading::SimulatorContext {
                        game_mode: 0,
                        retained_global25: 0,
                        global30: 1,
                    },
                    installed_expansions: 0xff,
                    performance_class: 2,
                },
            )
            .unwrap();
        loaded.initialize_indicators(&index).unwrap();
        let shared = &loaded.execution.vm.world.resource_storage;
        shared.validate(&loaded.program).unwrap();
        let help = shared
            .records
            .iter()
            .find(|r| r.guid == 0x8963d7c1)
            .unwrap();
        assert_eq!(help.values.len(), 24);
        assert_eq!(
            help.values[13],
            if source.starts_with("UserData2/") {
                5
            } else {
                0
            }
        );
        assert_eq!(
            help.values[14],
            if source.starts_with("UserData2/") {
                1
            } else {
                0
            }
        );
        assert_eq!(
            loaded
                .execution
                .vm
                .world
                .person
                .as_ref()
                .unwrap()
                .controlled_family,
            house.household.as_ref().unwrap().id as i16,
            "loaded manager family handle is stale"
        );
        // Scope 24 selects the neighborhood record, not the live object ID.
        // Every original resident must belong to the selected household.
        let mut query_vm = loaded.execution.vm.clone();
        let query_thread = *query_vm
            .threads
            .iter()
            .find(|(_, t)| !t.stack.is_empty())
            .unwrap()
            .0;
        for resident in &house.household.as_ref().unwrap().residents {
            query_vm
                .threads
                .get_mut(&query_thread)
                .unwrap()
                .stack
                .last_mut()
                .unwrap()
                .stack_object = resident.neighbor;
            assert_eq!(
                query_vm
                    .inspect_memory(&loaded.program, query_thread, 24, 1, false)
                    .unwrap()
                    .value,
                1,
                "original resident #{} is not recognized by the manager family",
                resident.object
            );
        }
        // Raw extracted neighborhoods omit LotZoning.iff. This test explicitly
        // supplies the shipped master; installer migration remains separate.
        let zoning = sims_content::zoning::Zoning::load(
            &index,
            "GameData_ALL_Recursive/MasterLotZoning.iff",
            0xff,
        )
        .unwrap();
        loaded
            .execution
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .lot_zoning = zoning.query_table();
        let mut assets = vec![];
        let clock = loaded.execution.lot_clock(&house).unwrap();
        let mut simulation = Simulation::from_loaded_lot(
            loaded.execution.vm.clone(),
            loaded.architecture.clone(),
            lot_runtime::State {
                placement: loaded.placement.clone(),
                membership: loaded.membership.clone(),
                indicators: loaded.indicators.clone(),
                observers: loaded.observers.clone(),
                diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None,
                },
                route: None,
                shoo: None,
                movement: None,
                snap: None,
                find_location: None,
                removal: None,
                snap_offsets: Default::default(),
                completion: None,
                route_paths: Default::default(),
                selected_person: None,
                room_effects: vec![],
                routing_policy: policy,
            },
        )
        .unwrap();
        simulation.removals = loaded.execution.removals.clone();
        let program = &mut loaded.program;
        if policy == sims_sim::lot_route::path::Policy::InferredRectangles {
            sims_content::vm::load_locomotion_animations(&index, program).unwrap();
        }
        assert_eq!(simulation.vm, loaded.execution.vm, "handoff changed VM");
        // CRT rand is process state, not present in SIMI. This diagnostic
        // chooses the same explicit seed as the native factory oracle; it does
        // not claim to reconstruct the original installation's startup draws.
        simulation.vm.crt_rng_state = Some(0x51a5cafe);
        simulation.enable_household(program, clock).unwrap();
        simulation
            .enable_native_residential_autonomy(false)
            .unwrap();
        assert!(simulation.vm.world.person.as_ref().unwrap().free_will);
        let start = simulation.vm.tick;
        let snapshot = Snapshot::capture(program, &simulation);
        assert_eq!(snapshot.clone().restore(program).unwrap(), simulation);
        let mut restored = snapshot.restore(program).unwrap();
        let mut traces = vec![];
        let mut saved_autonomy = false;
        let mut saved_named_callbacks = std::collections::BTreeSet::new();
        let mut constructor_callback_saved = false;
        let mut slices = 0usize;
        let mut planned_routes = std::collections::BTreeMap::new();
        let mut movement_samples = vec![];
        let mut saved_walking = false;
        let mut saved_portals = std::collections::BTreeSet::new();
        let mut saved_seated_services = std::collections::BTreeSet::new();
        let mut seat_events = vec![];
        let mut saved_standing_services = std::collections::BTreeSet::new();
        let mut stand_events = vec![];
        let mut route_completion_events = vec![];
        let mut accessory_states = std::collections::BTreeSet::new();
        let mut seated_attachments = std::collections::BTreeSet::new();
        let mut seated_states = vec![];
        let mut portal_events = vec![];
        let mut saved_snaps = std::collections::BTreeSet::new();
        let mut saved_person_snaps = std::collections::BTreeSet::new();
        let mut snap_events = vec![];
        let mut saved_locations = std::collections::BTreeSet::new();
        let mut saved_removals = std::collections::BTreeSet::new();
        let mut saved_creations = std::collections::BTreeSet::new();
        let mut caller_slot_creations = std::collections::BTreeMap::new();
        let mut creation_events = vec![];
        let mut saved_transfers = std::collections::BTreeSet::new();
        let mut transfer_events = vec![];
        let mut completed_factories = std::collections::BTreeMap::new();
        let mut saved_visitors = std::collections::BTreeSet::new();
        let mut visitor_events = vec![];
        let mut saved_person_resets = std::collections::BTreeSet::new();
        let mut saved_person_reset_states = std::collections::BTreeMap::new();
        let mut saved_shoo_states = std::collections::BTreeMap::new();
        let mut shoo_events = vec![];
        let mut removal_events = vec![];
        let mut location_events = vec![];
        let mut detached_states = vec![];
        let mut rejected_walking_events = vec![];
        let mut saved_departing_foregrounds = std::collections::BTreeMap::new();
        let mut maximum_service_depth = 0;
        let mut dialog_events = vec![];
        let boundary = loop {
            slices += 1;
            if slices % 1000 == 0 {
                eprintln!(
                    "{source}: slice {slices}, tick +{}, native route {:?}",
                    simulation.vm.tick - start,
                    simulation
                        .native_lot
                        .as_ref()
                        .and_then(|n| n.route.as_ref())
                        .map(|r| (r.thread, r.request, &r.obstacles))
                );
            }
            assert!(
                (slices as u64) < 20_000 + tick_limit * 64,
                "original house scheduler prefix did not reach its next host boundary"
            );
            let budget = if simulation.native_lot.as_ref().is_some_and(|n| n.shoo.is_some()) || matches!(simulation.vm.tick - start, 3744 | 3953 | 4401 | 4807) || simulation.vm.threads.values().any(|t| matches!(&t.status, sims_vm::Status::AwaitingCreation(r) if r.operands[4] == 5 && r.operands[5] & 4 != 0)) { 1 } else { 16 };
            let result = simulation.run_fixture_tick(program, budget);
            assert_eq!(restored.run_fixture_tick(program, budget), result);
            assert_eq!(restored, simulation, "resume diverged before host boundary");
            if let Some(e) = simulation.native_lot.as_ref().and_then(|n| n.shoo.as_ref()) {
                let state = serde_json::to_value(e).unwrap();
                let callbacks: Vec<_> = simulation
                    .vm
                    .threads
                    .iter()
                    .filter(|(_, t)| t.callback.is_some())
                    .map(|(id, t)| {
                        (
                            *id,
                            t.stack
                                .last()
                                .map(|f| (f.location.bhav, f.location.instruction)),
                        )
                    })
                    .collect();
                let key = format!(
                    "{}:{}:{}:{callbacks:?}",
                    e.route.id, state["selection"]["pending"], state["attempt"]["phase"]
                );
                if saved_shoo_states.insert(key, state).is_none() {
                    restored = serde_json::from_slice::<Snapshot>(
                        &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                    )
                    .unwrap()
                    .restore(program)
                    .unwrap();
                    assert_eq!(restored, simulation);
                    assert!(restored.advance_fixture_tick().is_err());
                }
            }
            for (&id, playback) in &simulation.animations {
                if simulation.unplaced.contains(&id)
                    && !playback.completed
                    && !saved_departing_foregrounds.contains_key(&id)
                {
                    saved_departing_foregrounds.insert(id, playback.clone());
                    restored = serde_json::from_slice::<Snapshot>(
                        &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                    )
                    .unwrap()
                    .restore(program)
                    .unwrap();
                    assert_eq!(restored, simulation);
                }
            }
            if let Ok(step) = &result {
                shoo_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("original shoo"))
                        .cloned(),
                );
                if step
                    .events
                    .iter()
                    .any(|e| e.detail.starts_with("original-lot rejected walking step:"))
                {
                    rejected_walking_events.extend(
                        step.events
                            .iter()
                            .filter(|e| e.detail.starts_with("original-lot rejected walking step:"))
                            .cloned(),
                    );
                    restored = serde_json::from_slice::<Snapshot>(
                        &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                    )
                    .unwrap()
                    .restore(program)
                    .unwrap();
                    assert_eq!(restored, simulation);
                }
            }
            for (&id, factory) in &simulation.person_factories {
                if factory.visitor.as_ref().is_some_and(|v| !v.complete)
                    && saved_visitors.insert(id)
                {
                    let snapshot = Snapshot::capture(program, &simulation);
                    restored =
                        serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&snapshot).unwrap())
                            .unwrap()
                            .restore(program)
                            .unwrap();
                    assert!(restored.advance_fixture_tick().is_err());
                    let mut invalid = snapshot;
                    invalid
                        .state
                        .person_factories
                        .get_mut(&id)
                        .unwrap()
                        .visitor
                        .as_mut()
                        .unwrap()
                        .thread = u64::MAX;
                    assert!(
                        invalid.restore(program).is_err(),
                        "unbound visitor accepted"
                    );
                }
                if factory.active
                    && factory.finalized
                    && factory.placement.as_ref().is_some_and(|e| e.complete)
                {
                    completed_factories.entry(id).or_insert_with(|| {
                        (factory.clone(), simulation.vm.world.objects[&id].clone())
                    });
                }
            }
            if let Ok(step) = &result {
                visitor_events.extend(
                    step.events
                        .iter()
                        .filter(|e| {
                            e.detail.contains("init visitor")
                                || e.detail.contains("wrapper completed after placement")
                        })
                        .cloned(),
                );
            }
            for d in &simulation.deletion {
                if d.person_reset.is_some() && saved_person_resets.insert(d.target) {
                    let actor = &simulation.vm.world.actions.as_ref().unwrap().actors[&d.target];
                    saved_person_reset_states.insert(d.target, json!({
                        "tick":simulation.vm.tick, "reset":d.person_reset,
                        "action":actor,
                        "thread":simulation.vm.threads[&simulation.vm.object_threads[&d.target]],
                        "previous_target_use_count":actor.previous.as_ref().and_then(|a| simulation.vm.world.objects.get(&a.target)).and_then(|o|o.data.get(&62)),
                    }));
                    restored = serde_json::from_slice::<Snapshot>(
                        &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                    )
                    .unwrap()
                    .restore(program)
                    .unwrap();
                    assert_eq!(restored, simulation);
                    assert!(restored.advance_fixture_tick().is_err());
                }
            }
            let mut new_named_callback = false;
            for call in &simulation.vm.named_calls {
                new_named_callback |= saved_named_callbacks.insert((
                    call.requester,
                    call.recipient,
                    call.owner.clone(),
                    call.bhav,
                ));
            }
            if new_named_callback {
                restored = Snapshot::capture(program, &simulation)
                    .restore(program)
                    .unwrap();
                let mut bad = Snapshot::capture(program, &simulation);
                bad.state.vm.named_calls.clear();
                assert!(bad.restore(program).is_err());
            }
            if let Some(snap) = simulation.native_lot.as_ref().unwrap().snap.as_ref() {
                if saved_snaps.insert((snap.thread, snap.request.route.id)) {
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    assert!(restored.advance_fixture_tick().is_err());
                    assert_eq!(restored, simulation);
                }
            }
            if let Some(location) = simulation
                .native_lot
                .as_ref()
                .unwrap()
                .find_location
                .as_ref()
            {
                if saved_locations.insert((location.binding().0, location.binding().1.id)) {
                    let snapshot = Snapshot::capture(program, &simulation);
                    restored = if location.binding().1.operands[0] == 1 {
                        serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&snapshot).unwrap())
                            .unwrap()
                            .restore(program)
                            .unwrap()
                    } else {
                        snapshot.restore(program).unwrap()
                    };
                    assert!(restored.advance_fixture_tick().is_err());
                    assert_eq!(restored, simulation);
                }
            }
            // A just-completed Snap can be followed by a missing animation
            // asset in the same slice. Err does not return that slice's events;
            // the committed native animator reset still proves completion.
            let mut new_person_snap = false;
            for id in simulation.native_lot.as_ref().unwrap().snap_offsets.keys() {
                new_person_snap |= saved_person_snaps.insert(*id);
            }
            if new_person_snap {
                restored = Snapshot::capture(program, &simulation)
                    .restore(program)
                    .unwrap();
            }
            if let Some(removal) = simulation.native_lot.as_ref().unwrap().removal.as_ref() {
                if saved_removals.insert(removal.target()) {
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    assert!(restored.advance_fixture_tick().is_err());
                }
            }
            for &request in simulation.native_creations.keys() {
                if saved_creations.insert(request) {
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    assert!(restored.advance_fixture_tick().is_err());
                    assert_eq!(restored, simulation);
                }
            }
            for t in simulation.vm.threads.values() {
                if let sims_vm::Status::AwaitingCreation(r) = &t.status {
                    if r.operands[4] == 2 && r.created.is_some() {
                        caller_slot_creations
                            .entry(r.id)
                            .or_insert_with(|| r.clone());
                    }
                }
            }
            for &request in simulation.native_transfers.keys() {
                if saved_transfers.insert(request) {
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    assert!(restored.advance_fixture_tick().is_err());
                    assert_eq!(restored, simulation);
                }
            }
            if let Ok(step) = &result {
                for e in &step.events {
                    let Some(target) = e
                        .detail
                        .strip_prefix("native Find Location detached #")
                        .and_then(|s| s.strip_suffix(" from lot"))
                        .and_then(|s| s.parse::<i16>().ok())
                    else {
                        continue;
                    };
                    // The one-instruction cleanup slice exposes physical
                    // detachment before the script's subsequent destruction.
                    if simulation.vm.tick - start == 3744 {
                        assert!(simulation.vm.world.objects.contains_key(&target));
                        assert!(simulation.unplaced.contains(&target));
                        assert!(!simulation.removals.iter().any(|q| q.target == target));
                        assert!(!simulation.deletion.iter().any(|q| q.target == target));
                        detached_states.push(json!({"target":target,"tick":simulation.vm.tick,
                            "position":simulation.vm.world.positions[&target],
                            "object":simulation.vm.world.objects[&target]}));
                        restored = serde_json::from_slice::<Snapshot>(
                            &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                        )
                        .unwrap()
                        .restore(program)
                        .unwrap();
                        assert_eq!(restored, simulation);
                    }
                }
                transfer_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("native transfer #"))
                        .cloned(),
                );
                creation_events.extend(
                    step.events
                        .iter()
                        .filter(|e| {
                            e.detail.starts_with("created #")
                                || e.detail.starts_with("factory completed #")
                        })
                        .cloned(),
                );
                removal_events.extend(
                    step.events
                        .iter()
                        .filter(|e| {
                            e.detail.starts_with("original physical removal #")
                                || e.detail.starts_with("deleted #")
                        })
                        .cloned(),
                );
                location_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("native Find Location"))
                        .cloned(),
                );
                snap_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("native Snap #"))
                        .cloned(),
                );
                portal_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.contains("portal"))
                        .cloned(),
                );
                route_completion_events.extend(
                    step.events
                        .iter()
                        .filter(|e| {
                            e.detail
                                .starts_with("original-lot native route cleanup completed")
                        })
                        .cloned(),
                );
                stand_events.extend(
                    step.events
                        .iter()
                        .filter(|e| {
                            e.detail.starts_with("original-lot route invokes Stand")
                                || e.detail.starts_with("original Stand #")
                        })
                        .cloned(),
                );
                seat_events.extend(
                    step.events
                        .iter()
                        .filter(|e| e.detail.starts_with("original-lot route invokes Sit"))
                        .cloned(),
                );
            }
            for (&id, object) in &simulation.vm.world.objects {
                for name in &object.accessories {
                    accessory_states.insert((id, name.clone()));
                }
            }
            let mut new_portal = false;
            let mut new_seat = false;
            let mut new_stand = false;
            for (&id, state) in &simulation.route_control {
                if let Some(portal) = state.last_portal {
                    new_portal |= saved_portals.insert((id, portal));
                }
                if let Some(seat) = state.original.as_ref().and_then(|o| o.standing) {
                    new_stand |= saved_standing_services.insert((id, state.caller, seat));
                }
                if let Some(seat) = state.original.as_ref().and_then(|o| o.seated) {
                    new_seat |= saved_seated_services.insert((id, state.caller, seat));
                }
            }
            if new_stand {
                let snapshot = Snapshot::capture(program, &simulation);
                restored =
                    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&snapshot).unwrap())
                        .unwrap()
                        .restore(program)
                        .unwrap();
                let mut bad = snapshot;
                bad.state
                    .route_control
                    .values_mut()
                    .find(|c| c.original.as_ref().is_some_and(|o| o.standing.is_some()))
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .standing = Some(i16::MAX);
                assert!(bad.restore(program).is_err());
            }
            if new_seat {
                let snapshot = Snapshot::capture(program, &simulation);
                restored =
                    serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&snapshot).unwrap())
                        .unwrap()
                        .restore(program)
                        .unwrap();
                let mut bad = snapshot;
                bad.state
                    .route_control
                    .values_mut()
                    .find(|c| c.original.as_ref().is_some_and(|o| o.seated.is_some()))
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .seated = Some(i16::MAX);
                assert!(bad.restore(program).is_err());
            }
            let mut new_attachment = false;
            for (&id, o) in &simulation.vm.world.objects {
                if o.person.is_some()
                    && let Some((parent, slot)) = o.container
                    && seated_attachments.insert((id, parent))
                {
                    new_attachment = true;
                    seated_states.push(json!({"tick":simulation.vm.tick-start,"person":id,
                        "parent":parent,"slot":slot,"position":simulation.vm.world.positions[&id],
                        "posture":o.person.as_ref().unwrap()[&0],"animation":simulation.animations.get(&id),
                        "snap_offsets":simulation.native_lot.as_ref().unwrap().snap_offsets.get(&id)}));
                }
            }
            if new_attachment {
                restored = Snapshot::capture(program, &simulation)
                    .restore(program)
                    .unwrap();
            }
            for thread in simulation.vm.threads.values() {
                maximum_service_depth = maximum_service_depth
                    .max(thread.service.as_ref().map_or(0, |s| s.chain().count()));
            }
            if new_portal {
                restored = Snapshot::capture(program, &simulation)
                    .restore(program)
                    .unwrap();
                let mut bad = Snapshot::capture(program, &simulation);
                let c = bad
                    .state
                    .route_control
                    .values_mut()
                    .find(|c| c.last_portal.is_some())
                    .unwrap();
                c.original = None;
                assert_eq!(
                    bad.restore(program).unwrap_err(),
                    "route controller host mismatch"
                );
            }
            for route in simulation.routes.values() {
                planned_routes
                    .entry(route.request)
                    .or_insert_with(|| route.clone());
            }
            if !simulation.routes.is_empty()
                && (simulation.vm.tick - start) % 20 == 0
                && !result.as_ref().is_ok_and(|s| s.stop == StepStop::Budget)
            {
                movement_samples.push(json!({"tick":simulation.vm.tick-start,"people":simulation.routes.values().map(|r|json!({"caller":r.caller,"position":simulation.vm.world.positions[&r.caller],"elapsed":r.locomotion.elapsed})).collect::<Vec<_>>()}));
                restored = Snapshot::capture(program, &simulation)
                    .restore(program)
                    .unwrap();
                saved_walking = true;
            }
            if result.is_err() || result.as_ref().is_ok_and(|s| s.stop == StepStop::Animation) {
                let names = simulation.vm.pending_accessory_assets(program);
                if !names.is_empty() {
                    match sims_content::avatar::bind_named_accessories(&index, program, &names) {
                        Ok(report) if report.unresolved_labels.is_empty() => {}
                        Ok(report) => {
                            break format!(
                                "required accessories unresolved: {:?}",
                                report.unresolved_labels
                            );
                        }
                        Err(error) => break format!("required accessory loading: {error:#}"),
                    }
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    continue;
                }
                match simulation.pending_animation_assets(program) {
                    Err(error) => break error,
                    Ok(names) if !names.is_empty() => {
                        let report = match sims_content::assets::load_named_animations(
                            &index, program, &names,
                        ) {
                            Ok(report) => report,
                            Err(error) => break format!("required animation loading: {error:#}"),
                        };
                        if !report.unresolved.is_empty() {
                            break format!(
                                "required animation names unresolved: {:?}",
                                report.unresolved
                            );
                        }
                        assets.push(report);
                        restored = Snapshot::capture(program, &simulation)
                            .restore(program)
                            .unwrap();
                        continue;
                    }
                    _ => {}
                }
                let guids: std::collections::BTreeSet<_> = simulation
                    .vm
                    .threads
                    .values()
                    .filter_map(|t| match &t.status {
                        sims_vm::Status::AwaitingCreation(r) => r
                            .created
                            .map(|id| {
                                simulation.vm.world.objects[&id]
                                    .definition
                                    .as_ref()
                                    .unwrap()
                                    .guid
                            })
                            .or_else(|| simulation.vm.creation_guid(r).ok().flatten()),
                        _ => None,
                    })
                    .filter(|guid| {
                        !program.modules.values().any(|m| {
                            m.object_definition_words.values().any(|w| {
                                w.len() > 15
                                    && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16))
                                        == *guid
                            })
                        })
                    })
                    .collect();
                if !guids.is_empty() {
                    if let Err(error) = house.link_creation_resources(&index, program, &guids) {
                        break format!("required object loading: {error:#}");
                    }
                    simulation
                        .architecture
                        .as_mut()
                        .unwrap()
                        .bind_footprints(program)
                        .unwrap();
                    restored = Snapshot::capture(program, &simulation)
                        .restore(program)
                        .unwrap();
                    continue;
                }
            }
            match result {
                Err(error) => break error,
                Ok(step) => {
                    traces.extend(step.trace);
                    if step.stop == StepStop::Budget {
                        let constructor_boundary = !constructor_callback_saved
                            && !simulation.person_factories.is_empty()
                            && simulation
                                .vm
                                .threads
                                .values()
                                .any(|t| t.callback.as_ref().is_some_and(|b| b.external));
                        let autonomy_boundary = !saved_autonomy
                            && simulation.vm.threads.values().any(|t| {
                                t.autonomy.as_ref().is_some_and(|q| q.waiting.is_some())
                                    && t.callback.is_some()
                            });
                        if constructor_boundary {
                            let mut unbound = Snapshot::capture(program, &simulation);
                            unbound.state.person_factories.clear();
                            assert_eq!(
                                unbound.restore(program).unwrap_err(),
                                "unbound immediate execution"
                            );
                            constructor_callback_saved = true;
                        }
                        // Both machines still execute and compare every slice.
                        // Save at explicit resumable boundaries plus periodic
                        // samples, without repeatedly hashing all loaded clips
                        // just because a completed factory remains registered.
                        if constructor_boundary || autonomy_boundary || slices % 64 == 0 {
                            restored = Snapshot::capture(program, &simulation)
                                .restore(program)
                                .unwrap();
                            saved_autonomy |= autonomy_boundary;
                        }
                        continue;
                    }
                    if step.stop == StepStop::Dialog {
                        let id = simulation
                            .vm
                            .world
                            .dialogs
                            .modal
                            .expect("Dialog stop without modal request");
                        let record = simulation
                            .vm
                            .world
                            .dialogs
                            .records
                            .iter()
                            .find(|r| r.id == id)
                            .unwrap()
                            .clone();
                        dialog_events
                            .push(serde_json::json!({"tick":simulation.vm.tick,"record":record}));
                        restored = serde_json::from_slice::<Snapshot>(
                            &serde_json::to_vec(&Snapshot::capture(program, &simulation)).unwrap(),
                        )
                        .unwrap()
                        .restore(program)
                        .unwrap();
                        assert_eq!(restored, simulation);
                        let before = simulation.clone();
                        assert_eq!(
                            simulation.run_fixture_tick(program, 1000).unwrap().stop,
                            StepStop::Dialog
                        );
                        assert_eq!(simulation, before, "waiting for input advanced simulation");
                        assert!(simulation.advance_fixture_tick().is_err());
                        assert_eq!(
                            simulation, before,
                            "rejected clock advance mutated simulation"
                        );
                        // Explicit diagnostic player input: acknowledge single-button
                        // notifications only. Choice dialogs remain paused, after
                        // verifying that saving/redrawing/waiting consumes no input.
                        if record.operands[5] & 127 != 0
                            || record.buttons.len() != 1
                            || record.buttons[0].result != 2
                        {
                            break format!(
                                "original dialog requires player choice: {}",
                                record.title
                            );
                        }
                        simulation.vm.answer_dialog(id, 2).unwrap();
                        restored.vm.answer_dialog(id, 2).unwrap();
                        assert!(simulation.vm.answer_dialog(id, 2).is_err());
                        continue;
                    }
                    if matches!(
                        step.stop,
                        StepStop::Faulted | StepStop::Animation | StepStop::Complete
                    ) {
                        break format!(
                            "{:?}: {:?}",
                            step.stop,
                            simulation
                                .vm
                                .threads
                                .iter()
                                .filter(|(_, t)| !matches!(
                                    t.status,
                                    sims_vm::Status::Sleeping { .. } | sims_vm::Status::Returned(_)
                                ))
                                .map(|(id, t)| (id, &t.status))
                                .collect::<Vec<_>>()
                        );
                    }
                    simulation.advance_fixture_tick().unwrap();
                    restored.advance_fixture_tick().unwrap();
                    if simulation.vm.tick - start >= tick_limit {
                        break format!("{tick_limit} ticks reached");
                    }
                }
            }
        };
        println!("{source} +{} ticks: {boundary}", simulation.vm.tick - start);
        assert_eq!(
            Snapshot::capture(program, &simulation)
                .restore(program)
                .unwrap(),
            simulation,
            "boundary continuation cannot be restored: {boundary}"
        );
        assert!(
            simulation.vm.tick - start > 6,
            "runtime prefix regressed: {boundary}"
        );
        assert!(!completed_factories.is_empty());
        assert!(
            constructor_callback_saved,
            "did not save inside the native second Init callback"
        );
        let (&created, (factory, created_object)) = completed_factories
            .iter()
            .find(|(_, (f, _))| f.visitor.is_none())
            .unwrap();
        assert!(factory.active && factory.wrapper_finished);
        assert_eq!(factory.scores.len(), 9);
        assert_eq!(created_object.data[&42], 259);
        if simulation.vm.world.objects.contains_key(&created) {
            assert!(!simulation.unplaced.contains(&created));
        } else {
            assert!(!simulation.person_factories.contains_key(&created));
            assert!(
                !simulation
                    .household
                    .as_ref()
                    .unwrap()
                    .people
                    .contains_key(&created)
            );
            assert!(
                !simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .unwrap()
                    .person_order
                    .contains(&created)
            );
        }
        assert!(factory.placement.as_ref().unwrap().complete);
        assert!(
            factory
                .acceptance
                .as_ref()
                .unwrap()
                .outcome()
                .unwrap()
                .accepted
        );
        assert_eq!(
            simulation
                .native_lot
                .as_ref()
                .unwrap()
                .observers
                .iter()
                .filter(
                    |e| matches!(e,sims_sim::lot_placement::Request::ViewPlaced(id) if *id==created)
                )
                .count(),
            1
        );
        assert_eq!(simulation.native_lot.as_ref().unwrap().observers.iter().filter(|event|
            matches!(event,sims_sim::lot_placement::Request::Signal{event:0xe2,value} if *value==created)).count(),1);
        // Investigate the exact next native placement query without accepting
        // it, inventing cached blocker rectangles, or running its callbacks.
        let target = factory
            .placement
            .as_ref()
            .unwrap()
            .operation
            .binding()
            .1
            .unwrap();
        let diagnostic_actor = if simulation.vm.world.objects.contains_key(&created) {
            created
        } else {
            *simulation
                .bodies
                .keys()
                .find(|id| simulation.vm.world.objects[id].person.is_some())
                .unwrap()
        };
        let input = sims_sim::lot_acceptance::Input::for_person(
            &simulation.vm.world,
            &simulation.native_lot.as_ref().unwrap().placement,
            simulation.architecture.as_ref().unwrap(),
            diagnostic_actor,
            target.position,
            target.level,
        )
        .unwrap();
        let mut floor_query = sims_sim::lot_acceptance::Floor::new(input.clone(), 91).unwrap();
        assert_eq!(
            floor_query.next(),
            Some(sims_sim::lot_acceptance::Request::Collision)
        );
        let mut query_world = simulation.vm.world.clone();
        let insets = simulation.architecture.as_ref().unwrap().footprints[&query_world.objects
            [&diagnostic_actor]
            .definition
            .as_ref()
            .unwrap()
            .guid]
            .insets;
        let mut collision = sims_sim::lot_acceptance::person::Collision::begin(
            &mut query_world,
            &simulation.native_lot.as_ref().unwrap().placement.links,
            diagnostic_actor,
            input.position,
            input.level,
            insets,
            sims_sim::lot_acceptance::person::Diagnostics {
                error: 91,
                blocker: None,
            },
        )
        .unwrap();
        let first_collision_service = collision
            .next(
                &query_world,
                &simulation.native_lot.as_ref().unwrap().placement.links,
                &Default::default(),
            )
            .unwrap();
        assert_eq!(
            query_world, simulation.vm.world,
            "placement diagnostic mutated the house"
        );
        println!(
            "  native floor environment passed; collision service: {first_collision_service:?}"
        );
        // Explicit supplied collision result for the isolated native outer-
        // checker comparison. This is not a live-house collision acceptance.
        let mut floor_input = serde_json::to_value(&input).unwrap();
        floor_input.as_object_mut().unwrap().extend(
            json!({"initial_error":91,
            "active_level":1,"wall_fit":true,"wall_error":7,"collision":true,
            "collision_error":11})
            .as_object()
            .unwrap()
            .clone(),
        );
        let placement_diagnostic = json!({"input":floor_input,"floor":floor_query,
            "collision":collision,"first_service":first_collision_service});
        // An unsupported service must leave the native graph serializable and
        // resumable, never silently replace it with the authored fixture graph.
        assert_eq!(
            Snapshot::capture(program, &simulation)
                .restore(program)
                .unwrap(),
            simulation
        );
        let pending_creation: Vec<_> = simulation
            .vm
            .threads
            .iter()
            .filter_map(|(id, t)| {
                if let sims_vm::Status::AwaitingCreation(request) = &t.status {
                    Some(json!({"thread":id,"request":request,"frame":t.stack.last()}))
                } else {
                    None
                }
            })
            .collect();
        let route_diagnostics: Vec<_> = simulation
            .vm
            .threads
            .values()
            .filter_map(|t| {
                if let sims_vm::Status::AwaitingRoute(r) = &t.status {
                    Some(route_clearance::inspect(program, &simulation, r))
                } else {
                    None
                }
            })
            .collect();
        for result in &route_diagnostics {
            match result {
                Ok(report) => println!(
                    "  original point clearance: {:?}; {} callback instructions",
                    report["outcome"], report["callback_instructions"]
                ),
                Err(error) => println!("  point clearance boundary: {error}"),
            }
        }
        let live = simulation.native_lot.as_ref().unwrap();
        if policy == sims_sim::lot_route::path::Policy::Strict {
            assert!(boundary.starts_with("original-lot path construction required"));
            assert_eq!(
                route_diagnostics.len(),
                1,
                "unexpected original route boundary"
            );
            let cleared = route_diagnostics[0]
                .as_ref()
                .expect("exact-point clearance must complete");
            let prefix = live
                .route
                .as_ref()
                .expect("native point route not prepared");
            // This independent diagnostic executes the candidate core. The
            // live host also runs its wrapper's pre-scan facing restoration.
            let expected: sims_sim::routing::CandidateSet =
                serde_json::from_value(cleared["candidates"].clone()).unwrap();
            let actual = prefix.candidates.as_ref().unwrap();
            assert_eq!(actual.candidates, expected.candidates);
            assert_eq!(
                (actual.maximum, actual.result),
                (expected.maximum, expected.result)
            );
            assert_eq!(
                prefix.point.as_ref().unwrap().actor_facing,
                expected.actor_facing
            );
            assert_eq!(
                i16::from(actual.actor_facing),
                simulation.vm.world.objects[&prefix.caller].data[&1]
            );
            assert_eq!(
                live.diagnostics,
                sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None
                }
            );
            assert_eq!(cleared["outcome"]["classification"], 6);
        } else {
            if source == "UserData/Houses/House07.iff" && tick_limit > 1006 {
                assert!(
                    simulation.vm.tick - start > 1006,
                    "native ordinary removal regressed: {boundary}"
                );
                assert!(
                    removal_events
                        .iter()
                        .any(|e| e.detail == "deleted #103 after lot object updates"
                            && e.tick > start),
                    "Come and See controller destruction was not observed (its ID may be reused)"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 1242 {
                assert!(
                    simulation.vm.tick - start > 1242,
                    "native person departure regressed: {boundary}"
                );
                assert!(
                    !simulation.person_factories.contains_key(&created),
                    "departed visitor factory remained registered"
                );
                assert!(
                    removal_events.iter().any(|e| e.detail
                        == format!("deleted #{created} after lot object updates")
                        && e.tick > start),
                    "visitor destruction was not observed"
                );
            }
            let fridge_creation = match source {
                "UserData/Houses/House07.iff" => Some((1494, 112)),
                "UserData2/Houses/House09.iff" => Some((523, 177)),
                _ => None,
            };
            if source == "UserData/Houses/House07.iff" && tick_limit > 1695 {
                assert!(
                    simulation.vm.tick - start > 1695,
                    "native Grab/carry regressed: {boundary}"
                );
                assert!(
                    transfer_events.iter().any(|e| e
                        .detail
                        .starts_with("native transfer #103 into Some(137) slot 0")),
                    "original food was not grabbed by person #137"
                );
                assert!(
                    program.animations.contains_key("a2o-rarm-carry-loop"),
                    "automatic carry asset was not discovered"
                );
            }
            if source == "UserData2/Houses/House09.iff" && tick_limit > 631 {
                assert!(
                    traces.iter().any(|t| t.opcode == 2
                        && t.location.source.ends_with("::CarPortal.iff")
                        && t.operands[2..4] == [12, 0]
                        && t.operands[7] == 33
                        && t.effect.contains("rhs 18; branch true")),
                    "original carpool career field did not execute: {boundary}"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 2275 {
                assert!(
                    simulation.vm.tick - start > 2275,
                    "original selected-seat dispatch regressed: {boundary}"
                );
                assert!(
                    saved_seated_services
                        .iter()
                        .any(|(_, person, seat)| *person == 137 && *seat == 128),
                    "original dining-chair Sit was not saved"
                );
                assert!(
                    seat_events
                        .iter()
                        .any(|e| e.detail
                            == "original-lot route invokes Sit on #128; entry flags 16")
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 2354 {
                assert!(
                    simulation.vm.tick - start > 2354,
                    "native contained Snap regressed: {boundary}"
                );
                assert!(
                    seated_attachments.contains(&(137, 128)),
                    "original dining-chair attachment was not observed"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 2489 {
                assert!(
                    simulation.vm.tick - start > 2489,
                    "native seated route cleanup regressed: {boundary}"
                );
                // A later asset pause can return Err after cleanup, without
                // returning that slice's diagnostic events. The original food
                // transfer is the durable downstream behavior we require.
                assert!(
                    transfer_events.iter().any(|e| e
                        .detail
                        .starts_with("native transfer #103 into Some(125) slot 4")),
                    "original seated dining did not reach table placement: {boundary}"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 2705 {
                assert!(
                    simulation.vm.tick - start > 2705,
                    "original eating accessory load regressed: {boundary}"
                );
                assert!(
                    accessory_states.contains(&(137, "suit-butterknife".into())),
                    "original eating script did not add its knife"
                );
                assert!(!program.accessories["suit-butterknife"].is_empty());
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3201 {
                assert!(
                    simulation.vm.tick - start > 3201,
                    "native NPC neighborhood enumeration regressed: {boundary}"
                );
                assert!(
                    traces.iter().any(|t| t.opcode == 31
                        && t.operands[4] & 127 == 7
                        && t.location.source.ends_with("::NPCController.iff")
                        && t.effect == "Set To Next 7: 0 → 25; branch true"),
                    "original NPC controller did not find neighborhood record #25"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3401 {
                assert!(
                    simulation.vm.tick - start > 3401,
                    "original Stand did not advance: {boundary}"
                );
                assert!(
                    saved_standing_services
                        .iter()
                        .any(|(_, person, seat)| (*person, *seat) == (137, 128)),
                    "original dining-chair Stand was not saved"
                );
                assert!(
                    stand_events
                        .iter()
                        .any(|e| e.detail == "original Stand #128 returned true; posture 0"),
                    "original dining-chair exit did not clear posture"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3652 {
                assert!(
                    simulation.vm.tick - start > 3652,
                    "pedestrian construction regressed: {boundary}"
                );
                let (&visitor, (factory, person)) = completed_factories
                    .iter()
                    .find(|(_, (f, _))| f.visitor.is_some())
                    .expect("pedestrian visitor factory did not finish");
                assert!(factory.finalized && factory.visitor.as_ref().unwrap().complete);
                assert_eq!(
                    person.definition.as_ref().unwrap().guid,
                    simulation.vm.world.person.as_ref().unwrap().neighbors[&25].guid
                );
                assert!(
                    saved_visitors.contains(&visitor),
                    "no save inside the pedestrian's init visitor"
                );
                for detail in [
                    format!(
                        "created person #{visitor}: original init visitor begins after placement"
                    ),
                    format!("created person #{visitor}: original init visitor returned"),
                ] {
                    assert_eq!(
                        visitor_events.iter().filter(|e| e.detail == detail).count(),
                        1,
                        "visitor callback repeated or missing"
                    );
                }
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3744 {
                assert!(
                    simulation.vm.tick - start > 3744,
                    "original sink cleanup detachment regressed: {boundary}"
                );
                assert!(
                    detached_states.iter().any(|s| s["target"] == 103),
                    "original dish was not saved between detachment and deletion"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3953 {
                assert!(
                    simulation.vm.tick - start > 3953,
                    "original caller-slot creation regressed: {boundary}"
                );
                assert!(
                    caller_slot_creations.values().any(|r| r
                        .origin
                        .source
                        .ends_with("People\\PaperCarrierF1.iff")
                        && r.placement
                            .as_ref()
                            .is_some_and(|at| at.target == r.caller && at.room == -5)
                        && saved_creations.contains(&r.id)),
                    "paper carrier's slot-zero factory was not saved"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 4401 {
                assert!(
                    simulation.vm.tick - start > 4401,
                    "native blocked-walk handoff regressed: {boundary}"
                );
                assert!(
                    rejected_walking_events
                        .iter()
                        .any(|e| e.detail.contains("native return state 6")),
                    "blocked step did not reach the native retry controller"
                );
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 4807 {
                assert_eq!(saved_departing_foregrounds[&138].clip, "c2o-standing-loop");
                assert!(
                    removal_events.iter().any(|e| e.detail
                        == "original physical removal #138 detached; queued end-of-tick deletion"),
                    "paper carrier's active-pose detachment regressed: {boundary}"
                );
                assert!(
                    simulation.vm.tick - start > 4807,
                    "paper carrier Reset regressed: {boundary}"
                );
                assert!(
                    saved_person_resets.contains(&138),
                    "carrier Reset was not saved"
                );
                let saved = &saved_person_reset_states[&138];
                assert_eq!(saved["tick"], start + 4807);
                assert!(saved["action"]["active"].is_null());
                assert_eq!(saved["action"]["previous"]["id"], 15);
                assert_eq!(saved["action"]["previous"]["target"], 203);
                assert_eq!(saved["previous_target_use_count"], 0);
                assert!(
                    removal_events.iter().any(|e| e.tick == start + 4807
                        && e.detail == "deleted #138 after lot object updates"),
                    "carrier registry deletion absent: {boundary}"
                );
                // Native registry allocation can reuse #138 later in the run;
                // final existence does not identify this departed person.
            }
            if source == "UserData/Houses/House07.iff" && tick_limit > 3652 {
                for (instruction, operands) in [
                    (3, [0, 0, 0, 0, 0, 2, 24, 7]),
                    (7, [0, 0, 1, 0, 0, 5, 25, 24]),
                ] {
                    assert!(
                        traces.iter().any(|t| t.opcode == 2
                            && t.location.source.ends_with("::PedPortal.iff")
                            && t.location.bhav == 4104
                            && t.location.instruction == instruction
                            && t.operands == operands
                            && !t.effect.starts_with("FAULT:")),
                        "pedestrian neighbor lookup {instruction} regressed: {boundary}"
                    );
                }
            }
            if let Some((tick, fridge)) = fridge_creation.filter(|(tick, _)| tick_limit > *tick) {
                assert!(
                    simulation.vm.tick - start > tick,
                    "native contained creation regressed: {boundary}"
                );
                assert!(
                    !saved_creations.is_empty(),
                    "no contained factory save exercised"
                );
                assert!(
                    creation_events
                        .iter()
                        .any(|e| e.detail.starts_with("created #")
                            && e.detail.ends_with(&format!("in #{fridge} slot 0"))),
                    "original fridge food placement was not observed"
                );
            }
            for regression in [
                "autonomy spatial context unavailable",
                "autonomy enumeration of person join interactions is unsupported",
                "unsupported advertisement personality modifier",
                "callback argument read exceeds supplied buffer",
                "missing BHAV 0x1010 for ExpansionShared/ExpansionShared.far::Gift_Gnome.iff",
            ] {
                assert!(
                    !boundary.contains(regression),
                    "post-walk regression: {boundary}"
                );
            }
            assert!(
                traces.iter().any(|t| t.opcode == 3
                    && t.effect == "Find Best Action advanced native residential eligibility scan"),
                "original autonomy eligibility did not execute: {boundary}"
            );
            assert!(!planned_routes.is_empty(), "no route started: {boundary}");
            assert!(saved_walking, "no walking save exercised: {boundary}");
            if source.contains("House05.iff") {
                if tick_limit > 1778 {
                    let help = dialog_events
                        .iter()
                        .find(|e| e["record"]["bitmap_icon"] == 5068)
                        .expect("original help-system choice was not presented");
                    assert_eq!(
                        help["record"]["source"],
                        "GameData_ALL_Recursive/Objects/Objects.far::HelpSystem.iff"
                    );
                    assert_eq!(
                        help["record"]["operands"],
                        json!([0, 68, 92, 0, 0, 1, 1, 6])
                    );
                    let state = simulation
                        .vm
                        .world
                        .resource_storage
                        .records
                        .iter()
                        .find(|r| r.guid == 0x8963d7c1)
                        .unwrap();
                    assert_eq!(state.values[13], 1);
                    assert!(
                        boundary.starts_with("original dialog requires player choice:"),
                        "{boundary}"
                    );
                }
                if tick_limit > 901 {
                    assert!(
                        simulation.vm.tick - start > 901,
                        "original Car Pool dialog regressed: {boundary}"
                    );
                    assert!(
                        !dialog_events.is_empty(),
                        "original Car Pool prompt was never presented"
                    );
                    let record = &dialog_events[0]["record"];
                    let person = house
                        .household
                        .as_ref()
                        .unwrap()
                        .residents
                        .iter()
                        .find(|r| i64::from(r.object) == record["stack_object"].as_i64().unwrap())
                        .unwrap();
                    assert!(record["body"].as_str().unwrap().contains(&person.name));
                    assert!(!record["body"].as_str().unwrap().contains("$Object"));
                }
                if tick_limit > 902 {
                    assert!(
                        simulation.vm.tick - start > 902,
                        "School Bus dialog regressed: {boundary}"
                    );
                    let school = dialog_events
                        .iter()
                        .find(|e| e["record"]["title"] == "School Bus")
                        .expect("original School Bus prompt was never presented");
                    assert_eq!(school["record"]["job_icon"], -1);
                    assert_eq!(school["record"]["icon_object"], serde_json::Value::Null);
                    assert!(
                        !school["record"]["body"]
                            .as_str()
                            .unwrap()
                            .contains("$Local")
                    );
                }
                assert!(
                    saved_person_snaps.contains(&16),
                    "Cassandra's native Snap did not commit: {boundary}"
                );
            }
            if source.starts_with("UserData2/") {
                assert!(
                    snap_events.iter().any(|e| e.detail.ends_with(": true")),
                    "original floor Snap did not complete: {boundary}"
                );
            }
            assert!(
                saved_autonomy,
                "no original autonomy callback save exercised: {boundary}"
            );
            if !source.starts_with("UserData2/") {
                assert!(
                    !saved_named_callbacks.is_empty(),
                    "no cross-object named callback saved: {boundary}"
                );
                assert!(
                    !saved_portals.is_empty(),
                    "no original door service saved: {boundary}"
                );
                assert!(maximum_service_depth > 0);
            }
            assert!(
                movement_samples
                    .iter()
                    .any(|s| s["people"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|p| p["elapsed"].as_u64().unwrap() > 0)),
                "no native movement: {boundary}"
            );
        }
        let car_groups: Vec<_> = simulation
            .vm
            .world
            .objects
            .iter()
            .filter(|(id, o)| {
                o.multitile.as_ref().is_some_and(|g| g.head.is_none())
                    && sims_vm::multitile::is_type(&simulation.vm.world, program, **id, 0x4b71ecda)
                        .unwrap_or(false)
            })
            .map(|(&id, _)| {
                let parts = sims_vm::multitile::chain(&simulation.vm.world, id).unwrap();
                json!({"head":id,"parts":parts.iter().map(|id| json!({
                    "id":id,"object":simulation.vm.world.objects[id],
                    "position":simulation.vm.world.positions[id],
                    "lifecycle":simulation.vm.lifecycles[id],
                })).collect::<Vec<_>>()})
            })
            .collect();
        if source == "UserData2/Houses/House09.iff"
            && policy == sims_sim::lot_route::path::Policy::InferredRectangles
            && tick_limit > 631
        {
            assert!(
                simulation.vm.tick - start > 631,
                "directional Find Location did not place the car: {boundary}"
            );
            assert_eq!(
                car_groups.len(),
                1,
                "original car factory did not complete: {boundary}"
            );
            assert!(car_groups[0]["parts"].as_array().unwrap().len() > 1);
            assert!(
                traces.iter().any(|t| t.opcode == 16
                    && t.location.source.ends_with("::CarPortal.iff")
                    && t.location.bhav == 4106
                    && t.location.instruction == 7
                    && t.operands == [3, 1, 3, 0, 0, 0, 0, 0]),
                "car factory did not return to original Find Location: {boundary}"
            );
            assert!(simulation.native_creations.is_empty());
            assert!(
                location_events
                    .iter()
                    .any(|e| e.detail
                        == "native Find Location #216 -> Some([376, 72]) floor 1: true"),
                "original directional car placement was not observed: {boundary}"
            );
            if tick_limit > 695 {
                assert!(
                    traces.iter().any(|t| t.opcode == 1
                        && t.operands[0..2] == [12, 0]
                        && t.location.source.ends_with("::CarJunk.iff")
                        && t.effect.contains("native camera tile-depth")),
                    "original car entry did not execute tile depth: {boundary}"
                );
            }
        }
        report.push(json!({"source":source,"zoning":zoning,"assets":assets,"car_groups":car_groups,
            "saved_rng":{"lot":house.simulator.lot_rng,"global":house.simulator.global_rng},
            "runtime_rng":{"lot":simulation.vm.lot_rng_state,"global":simulation.vm.rng_state,
                "crt":simulation.vm.crt_rng_state},
            "animation_names":program.animations.keys().collect::<Vec<_>>(),
            "pending_creation":pending_creation,
            "person_factories":simulation.person_factories,
            "completed_factories":completed_factories,"saved_person_resets":saved_person_resets,
            "saved_person_reset_states":saved_person_reset_states,
            "saved_shoo_states":saved_shoo_states,"shoo_events":shoo_events,
            "saved_visitors":saved_visitors,"visitor_events":visitor_events,
            "saved_creations":saved_creations,"creation_events":creation_events,
            "caller_slot_creations":caller_slot_creations,
            "saved_transfers":saved_transfers,"transfer_events":transfer_events,
            "placement_diagnostic":placement_diagnostic,
            "route_diagnostics":route_diagnostics,
            "live_route":simulation.native_lot.as_ref().unwrap().route,
            "route_controllers":simulation.route_control,
            "movement":simulation.native_lot.as_ref().unwrap().movement,
            "planned_routes":planned_routes,"movement_samples":movement_samples,
            "room_effects":simulation.native_lot.as_ref().unwrap().room_effects,
            "saved_autonomy_callback":saved_autonomy,
            "saved_named_callbacks":saved_named_callbacks,
            "saved_portals":saved_portals,"maximum_service_depth":maximum_service_depth,
            "saved_seated_services":saved_seated_services,"seat_events":seat_events,
            "saved_standing_services":saved_standing_services,"stand_events":stand_events,
            "route_completion_events":route_completion_events,"accessory_states":accessory_states,
            "seated_attachments":seated_attachments,"seated_states":seated_states,
            "portal_events":portal_events,
            "saved_snaps":saved_snaps,"snap_events":snap_events,
            "rejected_walking_events":rejected_walking_events,
            "saved_departing_foregrounds":saved_departing_foregrounds,
            "saved_person_snaps":saved_person_snaps,"snap_offsets":live.snap_offsets,
            "saved_locations":saved_locations,"location_events":location_events,
            "find_location":live.find_location,
            "find_location_target_state":live.find_location.as_ref().map(|e| {
                let (thread,r)=e.binding();
                json!({"request":r,"object":simulation.vm.world.objects.get(&r.target),
                    "position":simulation.vm.world.positions.get(&r.target),
                    "action":simulation.vm.world.actions.as_ref().and_then(|a|a.actors.get(&r.target)),
                    "animation":simulation.animations.get(&r.target),"background":simulation.backgrounds.get(&r.target),
                    "carry":simulation.carry.get(&r.target),"thread":simulation.vm.threads.get(&thread)})
            }),
            "detached_states":detached_states,
            "saved_removals":saved_removals,"removal_events":removal_events,
            "removal":live.removal,
            "deletion":simulation.deletion,
            "deletion_target_states":simulation.deletion.iter().map(|d| json!({
                "target":d.target,"object":simulation.vm.world.objects.get(&d.target),
                "position":simulation.vm.world.positions.get(&d.target),
                "thread":simulation.vm.object_threads.get(&d.target).and_then(|t| simulation.vm.threads.get(t)),
                "action":simulation.vm.world.actions.as_ref().and_then(|a| a.actors.get(&d.target)),
                "animation":simulation.animations.get(&d.target),"carry":simulation.carry.get(&d.target)
            })).collect::<Vec<_>>(),
            "removal_target_state":simulation.vm.threads.values().find_map(|t| match &t.status {
                sims_vm::Status::AwaitingRemoval(r)=>Some(json!({"request":r,"object":simulation.vm.world.objects.get(&r.target),
                    "action":simulation.vm.world.actions.as_ref().and_then(|a|a.actors.get(&r.target)),
                    "animation":simulation.animations.get(&r.target),"background":simulation.backgrounds.get(&r.target),
                    "carry":simulation.carry.get(&r.target),"thread":t})),_=>None}),
            "pending_threads":simulation.vm.threads.iter().map(|(id,t)| json!({"id":id,"status":t.status})).collect::<Vec<_>>(),
            "resource_storage":simulation.vm.world.resource_storage,
            "dialogs":dialog_events,"dialog_input_policy":"explicit test acknowledgment of single-button notifications only",
            "ticks":simulation.vm.tick-start,"boundary":boundary,"trace":traces}));
    }
    assert!(
        !report.is_empty(),
        "original-house filter matched no fixtures"
    );
    let base = if policy == sims_sim::lot_route::path::Policy::Strict {
        "cache/original-house-runtime-prefix"
    } else {
        "cache/original-house-inferred-walking"
    };
    fs::write(
        repo().join(format!(
            "{base}{}.json",
            if house_filter.is_some() {
                "-filtered"
            } else {
                ""
            }
        )),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
