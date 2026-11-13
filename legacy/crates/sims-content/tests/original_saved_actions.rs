#[path = "support/catalog.rs"]
mod historical_catalog;
use sha2::{Digest, Sha256};
use sims_content::{Index, saves::House};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires the preserved private Wine Newbie save and coherent asset tree"]
fn empty_neighborhood_owner_runs_controllers_clears_and_imports_in_one_vm() {
    use sims_sim::lot_geometry::{FloorSource, Lot};
    use sims_vm::loading::{HouseContext, SimulatorContext};
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/wine-parity-import-1/assets");
    let index = Index::scan(&root).unwrap();
    let house = House::load(&index, "UserData/Houses/House07.iff").unwrap();
    let mut catalog = sims_content::object_catalog::Catalog::household(
        std::sync::Arc::new(index.clone()),
        &house.neighborhood_source,
        Default::default(),
    )
    .unwrap();
    let controllers = catalog.global_controllers(&index).unwrap();
    let mut program = house.link_program(&index).unwrap();
    sims_content::catalog::link_controllers(&index, &mut program, &controllers).unwrap();
    program.resource_storage = Some(catalog.resource_storage(&index).unwrap());
    let mut world = house.prepare_neighborhood(&program).unwrap();
    // Process-retained compatibility: a matching ObjT entry clears its bit,
    // but a registered controller omitted from this save must retain it.
    let listed = house
        .compatibility(&program)
        .unwrap()
        .into_iter()
        .find(|c| !c.changed)
        .unwrap()
        .guid;
    let unlisted = controllers
        .iter()
        .find(|c| !house.types.entries.iter().any(|t| t.guid == c.guid))
        .unwrap()
        .guid;
    world.changed_resources.extend([listed, unlisted]);
    world.globals = sims_vm::loading::house_resize_globals(
        &house.simulator.globals,
        HouseContext {
            simulator: SimulatorContext {
                game_mode: 1,
                retained_global25: 0,
                global30: 1,
            },
            installed_expansions: 255,
            performance_class: 1,
        },
    )
    .unwrap();
    world.person.as_mut().unwrap().lot_zoning = sims_content::zoning::Zoning::load(
        &index,
        "GameData_ALL_Recursive/MasterLotZoning.iff",
        255,
    )
    .unwrap()
    .query_table();
    world.magic_history = Some(sims_vm::magic_history::Context::constructed(0, true));
    world.refresh = Some(Default::default());
    world.budget = Some(sims_vm::budget::State {
        balance: house
            .simulator
            .funds
            .wrapping_add(house.simulator.pending_funds),
        expenses: [0; 9],
        notifications: vec![],
    });
    let requested_map_size = match world.globals[&23] {
        0 => 64,
        n => i32::from(n),
    };
    world.map_size = Some(sims_vm::world_map::State { active_size: 64 });
    let mut vm = sims_vm::Vm::new(world, u64::from(house.simulator.global_rng));
    vm.tick = u64::from(house.simulator.ticks);
    vm.lot_rng_state = Some(house.simulator.lot_rng);
    vm.crt_rng_state = Some(123456);
    // An explicit empty test world, not a reconstruction of a captured process.
    let floor = FloorSource {
        coverings: vec![0; 4096],
        walls: vec![Default::default(); 4096],
        object_heads: vec![0; 4096],
        flags: vec![0; 4096],
    };
    let incoming = sims_content::house_startup::Incoming {
        vm,
        lot: Lot::rebuild(
            64,
            vec![[0; 4]; 4096],
            vec![255; 4096],
            vec![255; 4096],
            vec![floor.clone(), floor],
        )
        .unwrap(),
        indicators: Default::default(),
        pools: Default::default(),
        room_settings: sims_sim::lot_environment::manager::Settings {
            daylight_multiplier: 1.,
            light_range: 12.,
            rooms_enabled: true,
            models: std::array::from_fn(|i| Some(i as u8)),
        },
    };
    let with_assets = incoming.clone();
    let mut phases = vec![];
    let vm = incoming
        .prepare_read(
            &house,
            &program,
            &controllers,
            |phase, vm, _| {
                assert_eq!(vm.world.changed_resources, [listed, unlisted].into());
                assert_eq!(
                    vm.world.map_size.unwrap().active_size,
                    if phase == "resize_entry" {
                        64
                    } else {
                        requested_map_size
                    }
                );
                phases.push((phase.to_string(), vm.world.objects.len(), vm.rng_state));
                Ok(())
            },
            &mut |_, _| Err("unexpected pending asset service".into()),
        )
        .unwrap();
    // The same empty/non-graphic fixture also traverses the real Reader asset
    // seam. This is an authored startup regression, not a native hot import.
    let mut reader = sims_content::character_loading::Reader::new(
        &index,
        &house,
        sims_sim::pose::animator::Settings {
            fade_enabled: true,
            fade_duration: 200,
            sampling_config: 1,
        },
        [0.; 3],
        with_assets.vm.crt_rng_state.unwrap(),
        sims_runtime::original_session::View::default(),
    )
    .unwrap();
    let mut owned_phases = vec![];
    let owned = with_assets
        .prepare_read_with_assets(
            &house,
            &program,
            &controllers,
            |phase, vm, _| {
                owned_phases.push((phase.to_string(), vm.world.objects.len(), vm.rng_state));
                Ok(())
            },
            &mut reader,
        )
        .unwrap();
    assert_eq!(owned.vm, vm.vm);
    assert_eq!(owned.removals, vm.removals);
    assert_eq!(owned_phases, phases);
    assert!(reader.ordinary.graphics.is_empty() && reader.ordinary.retirements.is_empty());
    assert_eq!(reader.crt_state, vm.vm.crt_rng_state.unwrap());
    assert_eq!(
        phases
            .iter()
            .map(|p| (p.0.as_str(), p.1))
            .collect::<Vec<_>>(),
        [
            ("resize_entry", 0),
            ("resize_return", 141),
            ("clear_entry", 141),
            ("clear_return", 0),
            ("tatt_return", 0),
            ("objm_read_entry", 0)
        ]
    );
    assert_ne!(vm.vm.rng_state, house.simulator.global_rng);
    let before_import = (
        vm.vm.tick,
        vm.vm.rng_state,
        vm.vm.lot_rng_state,
        vm.vm.crt_rng_state,
    );
    let pending = vm.removals.clone();
    let imported = house.import_saved_execution(&program, vm).unwrap();
    assert!(!pending.is_empty());
    assert_eq!(
        imported.vm.world.map_size.unwrap().active_size,
        requested_map_size
    );
    let restored: sims_vm::Vm =
        serde_json::from_slice(&serde_json::to_vec(&imported.vm).unwrap()).unwrap();
    assert_eq!(restored.world.map_size, imported.vm.world.map_size);
    assert_eq!(imported.removals, pending);
    assert!(!imported.vm.world.changed_resources.contains(&listed));
    assert!(imported.vm.world.changed_resources.contains(&unlisted));
    assert_eq!(imported.vm.world.objects.len(), 245);
    assert_eq!(
        (
            imported.vm.tick,
            imported.vm.rng_state,
            imported.vm.lot_rng_state,
            imported.vm.crt_rng_state
        ),
        before_import
    );
}

#[test]
#[ignore = "requires the preserved private Wine Newbie save and coherent asset tree"]
fn saved_import_retains_pre_read_mutations_and_rng_without_running_init() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/wine-parity-import-1/assets");
    let index = Index::scan(&root).unwrap();
    let house = House::load(&index, "UserData/Houses/House07.iff").unwrap();
    let mut program = house.link_program(&index).unwrap();
    program.resource_storage = Some(historical_catalog::resource_storage(&index).unwrap());
    let mut world = house.prepare_neighborhood(&program).unwrap();
    world.globals.insert(27, 1234);
    world.neighborhood.insert(5, 4321);
    let neighbor = *world
        .person
        .as_ref()
        .unwrap()
        .neighbors
        .keys()
        .next()
        .unwrap();
    world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&neighbor)
        .unwrap()
        .person
        .insert(7, 2345);
    let record = world
        .resource_storage
        .records
        .iter_mut()
        .find(|r| !r.values.is_empty())
        .unwrap();
    record.values[0] = -123;
    let resources = world.resource_storage.clone();
    let cached_guid = house.objects.instances[&house.objects.order[0]].guid;
    world
        .person
        .as_mut()
        .unwrap()
        .resource_instances
        .insert(cached_guid, vec![]);
    world.actions = Some(sims_vm::action::WorldState {
        serial: 1_000_000,
        ..Default::default()
    });
    let mut vm = sims_vm::Vm::new(world, 0x12345678);
    vm.tick = 7654321;
    vm.lot_rng_state = Some(0x87654321);
    vm.crt_rng_state = Some(12345);
    let mut retained = sims_content::save_storage::Execution::from_retained(vm);
    retained.removals.push(sims_vm::removal::Pending {
        target: 143,
        suppress_reset: true,
    });
    let mut e = house.import_saved_execution(&program, retained).unwrap();
    assert_eq!(
        e.removals,
        [sims_vm::removal::Pending {
            target: 143,
            suppress_reset: true
        }]
    );
    assert_eq!(
        (
            e.vm.tick,
            e.vm.rng_state,
            e.vm.lot_rng_state,
            e.vm.crt_rng_state
        ),
        (7654321, 0x12345678, Some(0x87654321), Some(12345))
    );
    assert_eq!(e.vm.world.globals[&27], 1234);
    assert_eq!(e.vm.world.neighborhood[&5], 4321);
    assert_eq!(
        e.vm.world.person.as_ref().unwrap().neighbors[&neighbor].person[&7],
        2345
    );
    assert_eq!(e.vm.world.resource_storage, resources);
    assert_eq!(e.vm.world.actions.as_ref().unwrap().serial, 1_000_000);
    let expected_order: Vec<_> = house
        .objects
        .order
        .iter()
        .filter(|id| house.objects.instances[id].guid == cached_guid)
        .map(|&id| id as i16)
        .collect();
    assert_eq!(
        e.vm.world.person.as_ref().unwrap().resource_instances[&cached_guid],
        expected_order
    );
    assert_eq!(
        e.vm.world.person.as_ref().unwrap().resource_instances.len(),
        1
    );
    let fresh = house.prepare_execution(&program).unwrap();
    assert_eq!(e.vm.threads, fresh.vm.threads);
    assert_eq!(e.vm.world.objects, fresh.vm.world.objects);
    // A second read must fail before replacing any live object/continuation.
    e.vm.world.globals.insert(27, 999);
    assert!(house.import_saved_execution(&program, e).is_err());
}

#[test]
#[ignore = "requires the preserved private Wine Newbie save and coherent asset tree"]
fn captured_newbie_execution_survives_import_without_redispatch() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/wine-parity-import-1/assets");
    let source = "UserData/Houses/House07.iff";
    assert_eq!(
        format!("{:x}", Sha256::digest(fs::read(root.join(source)).unwrap())),
        "38cae6123a00c611fb46d572c1f6a8cc3380e363bd22d93515e47329a7e1002b"
    );
    let index = Index::scan(&root).unwrap();
    let house = House::load(&index, source).unwrap();
    let program = house.link_program(&index).unwrap();
    let execution = house.prepare_execution(&program).unwrap();
    let vm = &execution.vm;
    assert_eq!(vm.tick, 194953);
    assert_eq!(vm.lot_rng_state, Some(house.simulator.lot_rng));
    let mut serial = 0i32;
    for (&id, saved) in &house.objects.instances {
        let object = &vm.world.objects[&(id as i16)];
        assert_eq!(object.data[&62], saved.data[62]);
        let thread = &vm.threads[&vm.object_threads[&(id as i16)]];
        assert_eq!(thread.stack.len(), saved.stack.len());
        let mut callee = id as i16;
        for (i, (frame, original)) in thread.stack.iter().zip(&saved.stack).enumerate() {
            let record = saved.person.as_ref().and_then(|p| {
                p.object_uses
                    .iter()
                    .find(|r| r.stack_depth == (i + 1) as i32)
            });
            if let Some(record) = record {
                callee = record.target;
            }
            assert_eq!(frame.caller, id as i16);
            assert_eq!(frame.callee, callee);
            assert_eq!(frame.action_target, record.map(|r| r.target));
            assert_eq!(frame.action_icon, record.is_some_and(|r| r.unknown != 0));
            assert_eq!(frame.continuation, original.primitive_state as u32);
            assert_eq!(frame.location.bhav, original.tree);
            assert_eq!(frame.location.instruction, original.node as usize);
            assert_eq!(frame.args, original.parameters);
            assert_eq!(frame.locals, original.locals);
        }
        if let Some(person) = &saved.person {
            let actor = &vm.world.actions.as_ref().unwrap().actors[&(id as i16)];
            let active = actor.active.as_ref().unwrap();
            assert_eq!(active.target, person.active.target);
            assert_eq!(active.id, person.active.uid as u32);
            assert_eq!(active.flags, person.active.flags);
            assert_eq!(active.attenuation_bits, person.active.attenuation.to_bits());
            assert_eq!(actor.queue.pending.len(), person.queue.len());
            for action in [&person.active, &person.last]
                .into_iter()
                .chain(&person.queue)
            {
                serial = serial.max(action.uid);
            }
        }
    }
    assert_eq!(vm.world.actions.as_ref().unwrap().serial, serial as u32);
    // This checks serialization of the pre-activation storage. Full session
    // validation and deterministic continuation are separate checks.
    let restored: sims_vm::Vm = serde_json::from_slice(&serde_json::to_vec(vm).unwrap()).unwrap();
    assert_eq!(&restored, vm);
}
