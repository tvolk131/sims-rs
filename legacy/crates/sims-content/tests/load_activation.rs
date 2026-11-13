#[path = "support/catalog.rs"]
mod historical_catalog;
use serde_json::json;
use sims_formats::iff::Iff;
use std::{collections::BTreeMap, fs, path::PathBuf};
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[test]
#[ignore = "requires original extracted resources"]
fn original_global_controller_inventory() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut rows = BTreeMap::new();
    for resource in &index.resources {
        if !resource.is_iff()
            || resource
                .locator
                .to_ascii_lowercase()
                .contains("/characters/")
        {
            continue;
        }
        let iff = Iff::parse(resource.read().unwrap()).unwrap();
        for c in iff.all(b"OBJD") {
            let b = iff.data(c);
            if b.len() < 0x56 {
                continue;
            }
            let word = |o| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
            if matches!(word(0x12), 7 | 12) && word(0x54) != 0 {
                let guid = u32::from_le_bytes(b[28..32].try_into().unwrap());
                rows.entry(guid).or_insert_with(Vec::new).push(
                    json!({"source":resource.locator,"id":c.id,"kind":word(0x12),"label":c.label}),
                );
            }
        }
    }
    let mut houses = vec![];
    assert_eq!(catalog.len(), rows.len());
    assert_eq!(catalog.len(), 141);
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let h = sims_content::saves::House::load(&index, source).unwrap();
        let mut program = h.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut program, &catalog).unwrap();
        for c in &catalog {
            let m = &program.modules[&c.definition.source];
            let callbacks = &m.object_functions[&c.definition.chunk];
            assert_ne!(callbacks[1].action, 0, "{} missing Main", c.label);
        }
        let missing: Vec<_> = rows
            .keys()
            .filter(|g| !h.objects.instances.values().any(|o| o.guid == **g))
            .collect();
        println!("{source}: {} controller types missing", missing.len());
        let mut segments = BTreeMap::<u8, usize>::new();
        for wall in h.floors.iter().flat_map(|f| &f.walls) {
            *segments.entry(wall.segments).or_default() += 1;
        }
        println!("{source}: wall segments {segments:?}");
        houses.push(json!({"source":source,"missing":missing,"modules":program.modules.len(), "wall_segments":segments}));
    }
    fs::write(
        repo().join("cache/original-global-controllers.json"),
        serde_json::to_vec_pretty(&json!({"catalog":rows,"houses":houses})).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires original extracted houses and controller scripts"]
fn original_house_controller_activation() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::global_controllers(&index).unwrap();
    let mut reports = vec![];
    let mut geometry_reports = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = sims_content::saves::House::load(&index, source).unwrap();
        let mut p = house.link_program(&index).unwrap();
        sims_content::catalog::link_controllers(&index, &mut p, &catalog).unwrap();
        let mut execution = house.prepare_execution(&p).unwrap();
        execution
            .load_simulator_context(
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
        let original_threads = execution.vm.threads.clone();
        let ticks = execution.vm.tick;
        let result = execution.reconcile_global_controllers(&p, &catalog);
        let grid = execution.repair_object_grid(&house).unwrap();
        let geometry = house.lot_geometry(&grid.floors).unwrap();
        let architecture =
            sims_sim::architecture::Architecture::from_original(geometry.clone(), &p).unwrap();
        for resident in &house.household.as_ref().unwrap().residents {
            let saved = &house.objects.instances[&resident.object];
            let room = architecture.room_at(
                saved.position[2],
                sims_sim::Point::new(saved.position[0], saved.position[1]),
            );
            assert!(
                room >= 0,
                "resident {} has no recovered room",
                resident.name
            );
        }
        for (i, floor) in house.floors.iter().enumerate() {
            geometry_reports.push(json!({
                "source":source,
                "input":{
                    "size":house.width,"level":i+1,
                    "walls":floor.walls.iter().map(|w|w.segments).collect::<Vec<_>>(),
                    "styles":floor.walls.iter().map(|w|w.styles).collect::<Vec<_>>(),
                    "floors":floor.coverings,"flags":floor.flags,
                    "heights":geometry.heights,"ground_regions":geometry.pool,
                    "object_heads":grid.floors[i],
                    "below":if i==0 {vec![0;house.width*house.height]} else {geometry.floors[i-1].flags.flags.clone()},
                },
                "rust":geometry.floors[i],
            }));
        }
        println!(
            "{source}: controllers {}, deferred removals {}, grid repairs {}",
            result.as_ref().map_or(0, |r| r.created.len()),
            execution.removals.len(),
            grid.repairs.len()
        );
        reports.push(json!({"source":source,"result":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),"objects":execution.vm.world.objects.len(),"grid_repairs":grid.repairs}));
        fs::write(
            repo().join("cache/original-global-activation.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
        fs::write(
            repo().join("cache/original-house-flags.json"),
            serde_json::to_vec_pretty(&geometry_reports).unwrap(),
        )
        .unwrap();
        let result = result.unwrap();
        assert_eq!(
            grid.floors[1], house.floors[1].object_heads,
            "native repair does not alter upstairs grid"
        );
        // Saved heads can name existing objects at a different tile as well as
        // absent IDs. The Goth save has both kinds; parsing only reports absent IDs.
        for missing in &house.missing_tile_heads {
            if missing.floor == 1 {
                assert_eq!(grid.floors[0][missing.x * house.height + missing.y], 0);
            }
        }
        for repair in &grid.repairs {
            assert_eq!(
                grid.floors[0][repair.tile[0] * house.height + repair.tile[1]],
                0
            );
            assert_eq!(
                house.floors[0].object_heads[repair.tile[0] * house.height + repair.tile[1]],
                repair.object,
                "source remains intact"
            );
        }
        assert_eq!(execution.vm.tick, ticks);
        for (id, thread) in original_threads {
            assert_eq!(execution.vm.threads[&id], thread);
        }
        assert_eq!(
            result.created.len(),
            catalog
                .iter()
                .filter(|c| !house.objects.instances.values().any(|o| o.guid == c.guid))
                .count()
        );
    }
}
