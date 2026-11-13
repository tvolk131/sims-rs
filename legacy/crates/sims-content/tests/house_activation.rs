#[path = "support/catalog.rs"]
mod historical_catalog;
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Move through the live native registration list, stopping at unsupported
/// services. This is a prefix diagnostic, not playable import.
/// UI/drawing invalidations are recorded; room notifications are consumed.
#[test]
#[ignore = "requires original homes; diagnoses ordered activation with recorded graphics/light boundaries"]
fn probe_ordered_activation_prefix() {
    use sims_sim::lot_placement;
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let context = sims_vm::loading::HouseContext {
            simulator: sims_vm::loading::SimulatorContext {
                game_mode: 0,
                retained_global25: 0,
                global30: 1,
            },
            installed_expansions: 0xff,
            performance_class: 2,
        };
        let mut serviced = vec![];
        let stopped =
            house.load_frozen_with_animators(&index, context, &mut |effect, execution, _, _, _| {
                assert_eq!(execution.vm.tick, u64::from(house.simulator.ticks));
                serviced.push(effect);
                anyhow::bail!("test host: initialized character unavailable");
            });
        assert!(
            format!("{:#}", stopped.err().unwrap()).contains("initialized character unavailable")
        );
        assert_eq!(
            serviced.len(),
            1,
            "an unfinished animator service must stop later callbacks"
        );
        assert!(matches!(
            serviced[0],
            sims_content::house_loading::AnimatorEffect::Reset(_)
        ));
        let mut frozen = house
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
        for resident in &house.household.as_ref().unwrap().residents {
            use sims_content::house_loading::AnimatorEffect;
            let id = resident.object as i16;
            let services: Vec<_> = frozen
                .animator_effects
                .iter()
                .copied()
                .filter(|e| match e {
                    AnimatorEffect::Reset(i)
                    | AnimatorEffect::ReloadAppearance(i)
                    | AnimatorEffect::ResetPlacementCache(i)
                    | AnimatorEffect::Rebuild(i) => *i == id,
                })
                .collect();
            assert_eq!(
                services,
                [
                    AnimatorEffect::Reset(id),
                    AnimatorEffect::ReloadAppearance(id),
                    AnimatorEffect::ResetPlacementCache(id),
                    AnimatorEffect::Rebuild(id)
                ]
            );
        }
        let before = frozen.execution.vm.world.objects.len();
        let tick = frozen.execution.vm.tick;
        frozen.initialize_indicators(&index).unwrap();
        assert_eq!(frozen.execution.vm.tick, tick);
        assert_eq!(frozen.execution.vm.world.objects.len(), before + 2);
        for id in [frozen.indicators.primary, frozen.indicators.secondary] {
            let id = id.unwrap();
            assert_eq!(
                frozen.execution.vm.world.objects[&id].data[&8] & 0x4000,
                0x4000
            );
            assert_eq!(frozen.placement.links.next[&id], 0);
        }
        let once = frozen.execution.vm.clone();
        frozen.initialize_indicators(&index).unwrap();
        assert_eq!(frozen.execution.vm, once);
        let sims_content::house_loading::FrozenLot {
            program: p,
            execution: e,
            placement,
            membership,
            visits,
            observers,
            resident_tails,
            initial_visit_count,
            indicators,
            animator_effects,
            ..
        } = frozen;
        let boundary = "native selected-person/indicator attachment";
        // This is a frozen pre-activation checkpoint, not a playable save.
        // Validate the shared VM and physical graph after every load/reset.
        let snapshot = sims_vm::Snapshot::capture(&p, &e.vm);
        let restored = snapshot.clone().restore(&p).unwrap();
        assert_eq!(restored, e.vm);
        let restored_links: lot_placement::State =
            serde_json::from_value(json!(placement)).unwrap();
        assert_eq!(restored_links, placement);
        fs::create_dir_all(repo().join("cache/original-activation")).unwrap();
        let name = source.replace('/', "-");
        fs::write(repo().join(format!("cache/original-activation/{name}.json")), serde_json::to_vec(&json!({
            "boundary":boundary,"snapshot":snapshot,"placement":placement,"membership":membership,
            "removals":e.removals,"resident_tails":resident_tails,"indicators":indicators,"animator_effects":animator_effects
        })).unwrap()).unwrap();
        let visited: std::collections::BTreeSet<_> = visits.iter().map(|v| v.id).collect();
        assert_eq!(visited.len(), visits.len(), "an object was visited twice");
        for resident in &house.household.as_ref().unwrap().residents {
            assert!(
                visited.contains(&(resident.object as i16)),
                "resident omitted from callback pass"
            );
        }
        println!(
            "{source}: {} ordered prefix visits; {boundary:?}",
            visits.len()
        );
        report.push(
            json!({"source":source,"visits":visits,"boundary":boundary,"observers":observers,"resident_tails":resident_tails,"membership":membership,"initial_visit_count":initial_visit_count}),
        );
    }
    fs::write(
        repo().join("cache/original-ordered-activation-prefix.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for (row, expected_people) in report.iter().zip([2, 3, 2]) {
        assert_eq!(
            row["visits"].as_array().unwrap().len() as u64,
            row["initial_visit_count"].as_u64().unwrap(),
            "{row}"
        );
        assert!(
            row["boundary"]
                .as_str()
                .unwrap()
                .starts_with("native selected-person/indicator attachment")
        );
        assert_eq!(
            row["resident_tails"].as_array().unwrap().len(),
            expected_people
        );
    }
}

#[test]
#[ignore = "diagnostic base placement callbacks with recorded observer boundaries; not full activation"]
fn probe_original_placement_callbacks_in_isolation() {
    probe_placement_callbacks(false, false);
}
#[test]
#[ignore = "diagnostic virtual placement callbacks with recorded observers; not full activation"]
fn probe_original_virtual_placement_callbacks_in_isolation() {
    probe_placement_callbacks(true, false);
}
#[test]
#[ignore = "diagnostic isolated non-person loading reset; records remaining graphics boundaries"]
fn probe_original_reconstruction_reset_in_isolation() {
    probe_placement_callbacks(true, true);
}
fn probe_placement_callbacks(group: bool, reset: bool) {
    use sims_sim::lot_placement::{Operation, State, VirtualOperation};
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut report = vec![];
    let mut failures = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
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
        e.reconcile_global_controllers(&p, &catalog).unwrap();
        let grid = e.repair_object_grid(&house).unwrap();
        e.restore_placement_links(&house, &grid).unwrap();
        e.select_loaded_household(&house).unwrap();
        let a = sims_sim::architecture::Architecture::from_original(
            house.lot_geometry(&grid.floors).unwrap(),
            &p,
        )
        .unwrap();
        a.publish_room_context(&mut e.vm.world);
        let initial = e.vm.clone();
        let initial_placement = State::new(e.placement_links.clone().unwrap(), &a).unwrap();
        let mut rows = vec![];
        let mut errors = BTreeMap::new();
        let mut completed = 0;
        for &raw in &house.objects.order {
            let id = raw as i16;
            if initial.world.objects[&id].person.is_some() {
                continue;
            }
            e.vm = initial.clone();
            let mut state = initial_placement.clone();
            let mut observers = vec![];
            let result = (|| -> Result<(), String> {
                e.restore_object_position(&a, id)
                    .map_err(|e| e.to_string())?;
                if let Some(thread) = e.vm.begin_loaded_function(&p, id, 2)? {
                    e.vm.run_slice(&p, thread, 100_000)?;
                    if e.vm.threads[&thread].callback.is_some()
                        || e.vm.threads[&thread].status != sims_vm::Status::Ready
                    {
                        return Err(format!("Load: {:?}", e.vm.threads[&thread].status));
                    }
                }
                if reset {
                    let mut operation = sims_sim::lot_reset::Reset::new(id);
                    let now = sims_vm::construction::ObjectDate {
                        year: e.vm.world.globals[&8],
                        month: e.vm.world.globals[&7],
                        day: e.vm.world.globals[&1],
                    };
                    while let Some(request) =
                        operation.advance(&mut e.vm, &p, &a, &mut state, now, false)?
                    {
                        match request {
                            sims_sim::lot_reset::Request::Placement(request) => {
                                observers.push(request)
                            }
                            sims_sim::lot_reset::Request::Host {
                                thread,
                                status: sims_vm::Status::AwaitingCreation(_),
                            } => {
                                let mut creation = e
                                    .begin_off_lot_creation(
                                        &index,
                                        &mut p,
                                        &mut state.links,
                                        thread,
                                    )
                                    .map_err(|e| e.to_string())?;
                                while let Some(request) =
                                    creation.advance(&mut e.vm, &p, 100_000)?
                                {
                                    match request {
                                        sims_sim::lot_creation::Request::Created(id) => {
                                            // Explicit observer boundary in this isolated
                                            // diagnostic; no native UI is attached.
                                            observers.push(
                                                sims_sim::lot_placement::Request::Signal {
                                                    event: 0xe2,
                                                    value: id,
                                                },
                                            );
                                        }
                                        other => {
                                            return Err(format!("created host service: {other:?}"));
                                        }
                                    }
                                    creation = serde_json::from_value(json!(creation)).unwrap();
                                }
                            }
                            other => return Err(format!("reset host service: {other:?}")),
                        }
                        operation = serde_json::from_value(json!(operation)).unwrap();
                    }
                    return Ok(());
                }
                let saved = state.links.capture(&mut e.vm.world, id)?;
                let mut remove = if group {
                    VirtualOperation::remove(&e.vm.world, &state.links, id, true)?
                } else {
                    VirtualOperation::Base(Operation::remove(id, true))
                };
                while let Some(request) = remove.advance_loaded(&mut e.vm, &p, &a, &mut state)? {
                    observers.push(request);
                }
                if saved.on_lot {
                    let mut restore = if group {
                        VirtualOperation::set(&e.vm.world, &state.links, id, saved, true)?
                    } else {
                        VirtualOperation::Base(Operation::set(id, saved, true))
                    };
                    while let Some(request) =
                        restore.advance_loaded(&mut e.vm, &p, &a, &mut state)?
                    {
                        observers.push(request);
                    }
                }
                Ok(())
            })();
            if let Err(error) = &result {
                *errors.entry(error.clone()).or_insert(0) += 1;
            } else {
                completed += 1;
            }
            rows.push(json!({"id":id,"observers":observers,"error":result.err()}));
        }
        println!(
            "{source}: {completed} placement script paths (group={group},reset={reset}); errors {errors:?}"
        );
        if !errors.is_empty() {
            failures.push((source, errors.clone()));
        }
        report.push(json!({"source":source,"completed":completed,"errors":errors,"objects":rows}));
    }
    fs::write(
        repo().join(if reset {
            "cache/original-reconstruction-reset-callbacks.json"
        } else if group {
            "cache/original-virtual-placement-callbacks.json"
        } else {
            "cache/original-base-placement-callbacks.json"
        }),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        failures.is_empty(),
        "placement/reset callback failures: {failures:?}"
    );
    assert_eq!(
        report
            .iter()
            .map(|r| r["completed"].as_u64().unwrap())
            .sum::<u64>(),
        625
    );
}

#[test]
#[ignore = "requires extracted original homes; writes the private link oracle corpus"]
fn export_original_house_placement_link_corpus() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut rows = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
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
        let globals = e.reconcile_global_controllers(&p, &catalog).unwrap();
        let grid = e.repair_object_grid(&house).unwrap();
        e.restore_placement_links(&house, &grid).unwrap();
        assert!(
            e.restore_placement_links(&house, &grid).is_err(),
            "cannot reload links after activation begins"
        );
        let links = e.placement_links.as_ref().unwrap();
        links.publish_tile_chains(&mut e.vm.world).unwrap();
        for id in globals.created {
            assert_eq!(links.next[&id], 0);
        }
        let objects: Vec<_> =
            e.vm.world
                .objects
                .iter()
                .map(|(&id, o)| {
                    let q = e.vm.world.positions[&id];
                    let data: BTreeMap<_, _> = [2, 3, 14, 26]
                        .into_iter()
                        .map(|i| (i, o.data[&i]))
                        .collect();
                    let slots: Vec<_> = std::iter::once(links.next[&id])
                        .chain(o.slots.iter().map(|s| s.unwrap_or(0)))
                        .collect();
                    json!({"id":id,"data":data,"slots":slots,"position":[q.x,q.y,q.level]})
                })
                .collect();
        let mut operations = vec![];
        for &id in &house.objects.order {
            operations.extend([
                json!({"kind":"capture","id":id,"remember":true}),
                json!({"kind":"walk","id":id}),
                json!({"kind":"raw","id":id}),
                json!({"kind":"apply","id":id,"descriptor":{"on_lot":false,"position":[-16,-16],"container":null,"slot":-1,"level":1}}),
                json!({"kind":"restore","id":id}),
                json!({"kind":"raw","id":id}),
                json!({"kind":"walk","id":id}),
            ]);
        }
        println!(
            "{source}: {} registered objects, {} saved-object detach/reinsert sequences",
            objects.len(),
            house.objects.order.len()
        );
        rows.push(json!({"source":source,"input":{"size":house.width,"heads":links.heads,"objects":objects,"operations":operations}}));
    }
    fs::write(
        repo().join("cache/research/sims-exe/oracle/house-placement-link-inputs.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "isolated original placement notifications, not full household activation"]
fn original_house_seat_adjacency_callbacks() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut reports = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
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
        e.reconcile_global_controllers(&p, &catalog).unwrap();
        let grid = e.repair_object_grid(&house).unwrap();
        e.restore_placement_links(&house, &grid).unwrap();
        let original = e.vm.clone();
        let mut updates = vec![];
        let mut errors = vec![];
        for perturb in [false, true] {
            for (floor, heads) in grid.floors.iter().enumerate() {
                for (cell, &head) in heads.iter().enumerate().filter(|(_, h)| **h != 0) {
                    e.vm = original.clone();
                    // The unmodified save already has correct adjacency flags.
                    // Flip that bit on surfaces in a second, explicitly synthetic
                    // stimulus so the original placement callbacks actually run.
                    if perturb {
                        for object in e.vm.world.objects.values_mut() {
                            let d = object.definition.as_ref().unwrap();
                            if p.modules[&d.source].object_functions[&d.object]
                                .get(25)
                                .is_some_and(|f| f.action != 0)
                            {
                                *object.data.get_mut(&8).unwrap() ^= 0x100;
                            }
                        }
                    }
                    let tile = [
                        (cell / house.width) as i8,
                        (cell % house.width) as i8,
                        (floor + 1) as i8,
                    ];
                    match e.refresh_loaded_seat_adjacency(&p, tile) {
                    Ok(u) => updates.extend(u),
                    Err(error) => {
                        errors.push(json!({"head":head,"tile":tile,"perturbed":perturb,"error":format!("{error:#}")}))
                    }
                }
                }
            }
        }
        let callbacks = updates.iter().filter(|u| u.callback).count();
        println!(
            "{source}: {} visited objects, {callbacks} notification callbacks, {} errors",
            updates.len(),
            errors.len()
        );
        reports
            .push(json!({"source":source,"updates":updates,"callbacks":callbacks,"errors":errors}));
    }
    fs::write(
        repo().join("cache/original-seat-adjacency-callbacks.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
    for row in reports {
        assert!(
            row["errors"].as_array().unwrap().is_empty(),
            "{}: {}",
            row["source"],
            row["errors"]
        );
    }
}

#[test]
#[ignore = "requires extracted original houses"]
fn original_post_load_callback_inventory() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let p = house.link_program(&index).unwrap();
        let e = house.prepare_execution(&p).unwrap();
        let mut kinds = BTreeMap::new();
        let mut rows = vec![];
        for &id in &house.objects.order {
            let saved = &house.objects.instances[&id];
            let definition = &house.definitions[&saved.guid];
            let functions = &p.modules[&definition.source].object_functions[&definition.chunk];
            let plan = e.post_load[&(id as i16)];
            *kinds
                .entry((definition.kind, plan.reset, functions[3].action != 0))
                .or_insert(0) += 1;
            rows.push(json!({"id":id,"source":definition.source,"definition":definition.chunk,"kind":definition.kind,"position":saved.position,"data":saved.data,"plan":plan,"callbacks":functions.iter().map(|f|f.action).collect::<Vec<_>>()}));
        }
        println!("{source}: (kind, changed, has Reset): {kinds:?}");
        report.push(json!({"source":source,"objects":rows}));
    }
    fs::write(
        repo().join("cache/original-post-load-callbacks.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "diagnostic isolated Load callback probe, not full activation"]
fn probe_original_load_callbacks_in_isolation() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
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
        e.reconcile_global_controllers(&p, &catalog).unwrap();
        let heads = e.repair_object_grid(&house).unwrap();
        let a = sims_sim::architecture::Architecture::from_original(
            house.lot_geometry(&heads.floors).unwrap(),
            &p,
        )
        .unwrap();
        e.select_loaded_household(&house).unwrap();
        let initial = e.vm.clone();
        let mut errors = BTreeMap::new();
        let mut success = 0;
        let mut absent = 0;
        let mut rows = vec![];
        for &raw_id in &house.objects.order {
            let id = raw_id as i16;
            if !e.post_load[&id].load_callback {
                continue;
            }
            // Each callback sees the same supplied starting world, not the
            // preceding probe's side effects. Full native order is separate.
            e.vm = initial.clone();
            let spatial = e.restore_object_position(&a, id);
            let result = (|| -> Result<serde_json::Value, String> {
                spatial.map_err(|err| err.to_string())?;
                let Some(thread) = e.vm.begin_loaded_function(&p, id, 2)? else {
                    absent += 1;
                    return Ok(json!({"absent":true}));
                };
                let mut remaining = 100_000;
                let mut traces = vec![];
                while e.vm.threads[&thread].callback.is_some() {
                    if remaining == 0 {
                        return Err("Load diagnostic budget exhausted".into());
                    }
                    let run = e.vm.run_slice(&p, thread, remaining)?;
                    remaining -= run.trace.len();
                    traces.extend(run.trace);
                    if let sims_vm::Status::Faulted(f) = &e.vm.threads[&thread].status {
                        return Err(f.message.clone());
                    }
                    if e.vm.threads[&thread].status != sims_vm::Status::Ready {
                        return Err(format!(
                            "Load host boundary: {:?}",
                            e.vm.threads[&thread].status
                        ));
                    }
                    if e.vm.threads[&thread].callback.is_some() && remaining == 100_000 {
                        return Err("Load made no instruction progress".into());
                    }
                }
                success += 1;
                Ok(
                    json!({"returned":e.vm.threads[&thread].callback_result,"instructions":100_000-remaining,"trace":traces}),
                )
            })();
            if let Err(error) = &result {
                *errors.entry(error.clone()).or_insert(0) += 1;
            }
            rows.push(json!({"id":id,"source":house.definitions[&house.objects.instances[&raw_id].guid].source,"result":result.as_ref().ok(),"error":result.as_ref().err()}));
        }
        println!(
            "{source}: {success} Load callbacks completed, {absent} absent, errors {errors:?}"
        );
        report.push(json!({"source":source,"completed":success,"absent":absent,"errors":errors,"objects":rows}));
    }
    fs::write(
        repo().join("cache/original-isolated-load-callbacks.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for house in report {
        assert!(
            house["errors"].as_object().unwrap().is_empty(),
            "{}: {}",
            house["source"],
            house["errors"]
        );
    }
}

/// Ordered spatial prefix only. No callback mutations are interleaved: this
/// corpus validates saved positions/rooms/children, not full post-load ordering.
fn position_corpus() -> Vec<serde_json::Value> {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut rows = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let p = house.link_program(&index).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        let heads = e.repair_object_grid(&house).unwrap();
        let a = sims_sim::architecture::Architecture::from_original(
            house.lot_geometry(&heads.floors).unwrap(),
            &p,
        )
        .unwrap();
        let world = &e.vm.world;
        let objects:Vec<_>=house.objects.order.iter().map(|&raw| {
            let id=raw as i16;let o=&world.objects[&id];let shape=&a.footprints[&o.definition.as_ref().unwrap().guid];
            let pos=world.positions[&id];let person=|i|o.person.as_ref().and_then(|p|p.get(&i)).copied().unwrap_or(0);
            json!({"id":id,"kind":shape.kind,"size":shape.size,"insets":shape.insets,"facing":o.data[&1],"light":o.data[&7],"room":o.data[&29],
                "person68":person(68),"person84":person(84),"person73":person(73) as u16,"position":[pos.x,pos.y,pos.level],"children":o.slots})
        }).collect();
        let lot = a.original.as_ref().unwrap();
        let floors:Vec<_>=lot.floors.iter().map(|f|json!({"primary":f.rooms.primary,"walls":f.source.walls.iter().map(|w|w.segments).collect::<Vec<_>>()})).collect();
        let mut visits = vec![];
        for &id in &house.objects.order {
            visits.push(json!({"id":id,"update":e.restore_object_position(&a,id as i16).unwrap()}));
        }
        rows.push(json!({"source":source,"input":{"size":house.width,"floors":floors,"objects":objects,"order":house.objects.order},"rust":visits}));
    }
    rows
}
#[test]
#[ignore = "requires extracted original homes; writes a private native probe corpus"]
fn export_original_house_position_corpus() {
    let rows = position_corpus();
    fs::write(
        repo().join("cache/research/sims-exe/oracle/house-loaded-position-inputs.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
#[test]
#[ignore = "requires original house position oracle"]
fn original_house_position_corpus_matches_native() {
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/house-loaded-positions.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let fresh = position_corpus();
    assert_eq!(fresh.len(), report["cases"].as_array().unwrap().len());
    for (actual, native) in fresh.iter().zip(report["cases"].as_array().unwrap()) {
        assert_eq!(actual["source"], native["source"]);
        assert_eq!(actual["input"], native["input"], "stale oracle inputs");
        assert_eq!(
            actual["rust"].as_array().unwrap().len(),
            native["original"].as_array().unwrap().len()
        );
        for (a, b) in actual["rust"]
            .as_array()
            .unwrap()
            .iter()
            .zip(native["original"].as_array().unwrap())
        {
            assert_eq!(a, b, "{} object {}", actual["source"], a["id"]);
        }
        println!(
            "{}: {} native spatial-prefix visits match",
            actual["source"],
            actual["rust"].as_array().unwrap().len()
        );
    }
}

#[test]
#[ignore = "diagnostic isolated base Reset callback branch, not full activation"]
fn probe_original_base_reset_callbacks_in_isolation() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut e = house.prepare_execution(&p).unwrap();
        e.load_simulator_context(
            &house,
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
        e.reconcile_global_controllers(&p, &catalog).unwrap();
        let heads = e.repair_object_grid(&house).unwrap();
        let a = sims_sim::architecture::Architecture::from_original(
            house.lot_geometry(&heads.floors).unwrap(),
            &p,
        )
        .unwrap();
        e.select_loaded_household(&house).unwrap();
        let initial = e.vm.clone();
        let mut rows = vec![];
        let mut errors = BTreeMap::new();
        let mut success = 0;
        for &raw in &house.objects.order {
            let id = raw as i16;
            let o = &initial.world.objects[&id];
            let d = o.definition.as_ref().unwrap();
            if o.person.is_some()
                || !e.post_load[&id].reset
                || p.modules[&d.source].object_functions[&d.object][3].action == 0
            {
                continue;
            }
            e.vm = initial.clone();
            let result = (|| -> Result<(), String> {
                e.restore_object_position(&a, id)
                    .map_err(|e| e.to_string())?;
                // Native Load runs first on this object. Other objects retain
                // the initial supplied world in this isolated diagnostic.
                let load = e.vm.begin_loaded_function(&p, id, 2)?;
                if let Some(thread) = load {
                    e.vm.run_slice(&p, thread, 100_000)?;
                    if e.vm.threads[&thread].callback.is_some()
                        || e.vm.threads[&thread].status != sims_vm::Status::Ready
                    {
                        return Err(format!("Load boundary: {:?}", e.vm.threads[&thread].status));
                    }
                }
                let reset = e.vm.begin_loaded_base_callback_reset(&p, id)?;
                let thread = reset.thread();
                if e.vm.threads[&thread].callback.is_some() {
                    e.vm.run_slice(&p, thread, 100_000)?;
                }
                if let sims_vm::Status::Faulted(f) = &e.vm.threads[&thread].status {
                    return Err(f.message.clone());
                }
                if e.vm.threads[&thread].callback.is_some() {
                    return Err(format!(
                        "Reset host boundary: {:?}",
                        e.vm.threads[&thread].status
                    ));
                }
                reset.finish(&mut e.vm, &p)?;
                Ok(())
            })();
            if let Err(error) = &result {
                *errors.entry(error.clone()).or_insert(0) += 1;
            } else {
                success += 1;
            }
            rows.push(json!({"id":id,"source":d.source,"error":result.err()}));
        }
        println!("{source}: {success} base Reset callbacks completed; errors {errors:?}");
        report.push(json!({"source":source,"completed":success,"errors":errors,"objects":rows}));
    }
    fs::write(
        repo().join("cache/original-isolated-reset-callbacks.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    for house in report {
        assert!(
            house["errors"].as_object().unwrap().is_empty(),
            "{}: {}",
            house["source"],
            house["errors"]
        );
    }
}
