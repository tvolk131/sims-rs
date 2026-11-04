use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::{
    Index, character_geometry::MeshPool, graphic_initialization::Environment,
    graphic_loading::Registry,
};
use sims_sim::{
    lot_creation::{OffLot, Request},
    lot_links::Links,
    pose::{animator::Settings, graphic_transform::Source, transform},
};
use sims_vm::{Snapshot, Vm, World, construction::ObjectDate};

#[test]
fn unavailable_profile_keeps_factory_before_date_and_init() -> Result<()> {
    let index = Index {
        root: Default::default(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    };
    let mut words = vec![0; 108];
    words[9] = 4;
    words[14] = 1;
    words[19] = 200;
    words[102] = 1;
    let module = sims_vm::Module {
        source: "missing profile".into(),
        object_definition_words: std::collections::BTreeMap::from([(1, words)]),
        ..Default::default()
    };
    let p = sims_vm::Program {
        globals: module.source.clone(),
        modules: std::collections::BTreeMap::from([(module.source.clone(), module)]),
        ..Default::default()
    };
    let mut vm = Vm::new(
        World {
            person: Some(Default::default()),
            refresh: Some(Default::default()),
            ..Default::default()
        },
        7,
    );
    let mut links = Links {
        size: 2,
        heads: vec![vec![0; 4]; 2],
        next: Default::default(),
    };
    let mut factory = OffLot::begin_resource_with_graphics(
        &mut vm,
        &p,
        &mut links,
        1,
        ObjectDate {
            year: 1997,
            month: 3,
            day: 12,
        },
        [13, 25],
    )
    .map_err(anyhow::Error::msg)?;
    let mut registry = Registry::default();
    registry.begin(&vm, &p, 1)?;
    let mut crt = 123;
    let mut pool = MeshPool::default();
    let saved_vm = vm.clone();
    let advance = |registry: &mut Registry, crt: &mut u32, pool: &mut MeshPool| {
        registry.advance(
            &index,
            &saved_vm,
            &p,
            1,
            crt,
            Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            pool,
            Environment {
                owner_appearance_index: 0,
                source: Source {
                    position: [-16, -16],
                    facing: 0,
                    level: 1,
                },
                view: transform::View {
                    rotation: 0,
                    floor_vector: [0.; 3],
                },
                terrain: transform::Terrain {
                    size: 2,
                    corners: &[[0; 4]; 4],
                },
                viewport: [0, 0, 800, 600],
            },
        )
    };
    ensure!(!advance(&mut registry, &mut crt, &mut pool)?);
    ensure!(
        advance(&mut registry, &mut crt, &mut pool)
            .unwrap_err()
            .to_string()
            .contains("not mounted")
    );
    let failed = registry.snapshot();
    registry = Registry::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&failed)?)?,
    )?;
    ensure!(advance(&mut registry, &mut crt, &mut pool).is_err());
    ensure!(
        registry.snapshot() == failed
            && registry.graphics.is_empty()
            && crt == 123
            && pool == MeshPool::default()
    );
    ensure!(registry.begin(&vm, &p, 1).is_err());
    ensure!(matches!(
        factory
            .advance(&mut vm, &p, 100)
            .map_err(anyhow::Error::msg)?,
        Some(Request::Graphic(1))
    ));
    ensure!(vm == saved_vm && vm.object_threads.is_empty());
    vm.world.objects.remove(&1);
    ensure!(registry.validate(&vm).is_err());
    Ok(())
}

#[test]
#[ignore = "requires private original indicator assets"]
fn original_graphic_factories_preserve_native_order_and_restore_every_service() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for name in ["PlumbBob.iff", "DateBob.iff", "StarBob.iff"] {
        let p = sims_content::vm::load_program(&index, name, "Global.iff")?;
        let source = &index.resources[index.find(name)?].locator;
        let words = p.modules[source]
            .object_definition_words
            .values()
            .next()
            .context("marker OBJD absent")?;
        let guid = u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16);
        let now = ObjectDate {
            year: 1997,
            month: 3,
            day: 12,
        };
        let time = [13, 25];
        let mut results = vec![];
        for resume in [false, true] {
            let mut vm = Vm::new(
                World {
                    person: Some(Default::default()),
                    refresh: Some(Default::default()),
                    ..Default::default()
                },
                7,
            );
            vm.tick = 1234;
            let mut links = Links {
                size: 2,
                heads: vec![vec![0; 4]; 2],
                next: Default::default(),
            };
            let mut registry = Registry::default();
            let mut pool = MeshPool::default();
            let mut crt = 1;
            let mut services = vec![];
            // The second instance shares the definition-owned warm profile.
            for instance in 0..2 {
                let mut factory =
                    OffLot::begin_resource_with_graphics(&mut vm, &p, &mut links, guid, now, time)
                        .map_err(anyhow::Error::msg)?;
                let id = factory.object().context("marker allocation failed")?;
                let registered = vm.clone();
                ensure!(!vm.object_threads.contains_key(&id) && !vm.lifecycles.contains_key(&id));
                ensure!(
                    [45, 46, 47, 24, 21]
                        .iter()
                        .all(|key| vm.world.objects[&id].data[key] == 0)
                );
                ensure!(links.next[&id] == 0);
                registry.begin(&vm, &p, id)?;
                let mut count = 0;
                let mut created = 0;
                loop {
                    if resume {
                        vm = Snapshot::capture(&p, &vm)
                            .restore(&p)
                            .map_err(anyhow::Error::msg)?;
                        factory = serde_json::from_slice(&serde_json::to_vec(&factory)?)?;
                        let saved = registry.snapshot();
                        registry = Registry::restore(
                            &index,
                            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                        )?;
                        ensure!(registry.snapshot() == saved);
                        pool = serde_json::from_slice(&serde_json::to_vec(&pool)?)?;
                    }
                    let Some(request) = factory
                        .advance(&mut vm, &p, if resume { 1 } else { 100_000 })
                        .map_err(anyhow::Error::msg)?
                    else {
                        break;
                    };
                    match request {
                        Request::Graphic(owner) => {
                            ensure!(
                                owner == id && vm == registered,
                                "factory ran script/date/VM state before graphics"
                            );
                            let object = &vm.world.objects[&id];
                            let position = &vm.world.positions[&id];
                            let complete = registry.advance(
                                &index,
                                &vm,
                                &p,
                                id,
                                &mut crt,
                                Settings {
                                    fade_enabled: true,
                                    fade_duration: 200,
                                    sampling_config: 1,
                                },
                                &mut pool,
                                Environment {
                                    owner_appearance_index: object.data[&68],
                                    source: Source {
                                        position: [position.x, position.y],
                                        facing: object.data[&1],
                                        level: position.level,
                                    },
                                    view: transform::View {
                                        rotation: 0,
                                        floor_vector: [0.; 3],
                                    },
                                    terrain: transform::Terrain {
                                        size: 2,
                                        corners: &[[0; 4]; 4],
                                    },
                                    viewport: [0, 0, 800, 600],
                                },
                            )?;
                            count += 1;
                            ensure!(count < 100);
                            if complete {
                                let ready = &registry.graphics[&id];
                                ensure!(
                                    ready.owner.ready == 1
                                        && ready.owner.graphic == Some(id as u64)
                                );
                                ensure!(
                                    ready.ready.avatar.avatar.parts.is_empty(),
                                    "Init binds skeleton before later appearance selection"
                                );
                                let mut expected = 1;
                                for _ in 0..32 * (instance + 1) {
                                    sims_vm::rng::next_crt(&mut expected);
                                }
                                ensure!(
                                    crt == expected,
                                    "root model constructor CRT position changed"
                                );
                                factory
                                    .complete_graphic(&mut vm, &p, id)
                                    .map_err(anyhow::Error::msg)?;
                                ensure!(vm.object_threads.contains_key(&id));
                                ensure!(
                                    factory.complete_graphic(&mut vm, &p, id).is_err(),
                                    "duplicate graphics acknowledgment accepted"
                                );
                            }
                            if let Some(job) =
                                json!(registry.snapshot())["jobs"].get(id.to_string())
                            {
                                if !job["prepared"].is_null() {
                                    let mut bad = json!(registry.snapshot());
                                    bad["jobs"][id.to_string()]["prepared"]["definition"]["guid"] =
                                        json!(0);
                                    ensure!(
                                        Registry::restore(&index, serde_json::from_value(bad)?)
                                            .is_err(),
                                        "mismatched nested initializer owner accepted"
                                    );
                                }
                            }
                        }
                        Request::Created(owner) => {
                            ensure!(owner == id);
                            ensure!(registry.graphics[&id].owner.ready == 1);
                            created += 1;
                        }
                        Request::Budget => {}
                        other => anyhow::bail!("unexpected marker factory service: {other:?}"),
                    }
                    ensure!(vm.tick == 1234);
                }
                ensure!(created == 1);
                registry.validate(&vm)?;
                services.push(count);
            }
            let saved = registry.snapshot();
            ensure!(json!(saved)["profiles"].as_array().unwrap().len() == 1);
            let mut bad = json!(saved);
            bad["graphics"]["1"]["owner"]["ready"] = json!(0);
            ensure!(Registry::restore(&index, serde_json::from_value(bad)?).is_err());
            results.push((vm, saved, pool, crt, services));
        }
        ensure!(
            results[0] == results[1],
            "{name}: factory/save service ordering differs"
        );
        // Graphics mutate their own assets/CRT stream, not VM construction.
        let mut control = Vm::new(
            World {
                person: Some(Default::default()),
                refresh: Some(Default::default()),
                ..Default::default()
            },
            7,
        );
        control.tick = 1234;
        let mut links = Links {
            size: 2,
            heads: vec![vec![0; 4]; 2],
            next: Default::default(),
        };
        for _ in 0..2 {
            let mut factory = OffLot::begin_resource(&mut control, &p, &mut links, guid, now, time)
                .map_err(anyhow::Error::msg)?;
            while let Some(request) = factory
                .advance(&mut control, &p, 100_000)
                .map_err(anyhow::Error::msg)?
            {
                ensure!(matches!(request, Request::Created(_)));
            }
        }
        ensure!(
            control == results[0].0,
            "{name}: graphics handoff changed VM construction/scripts"
        );
        report.push(json!({"source":source,"guid":guid,"services":results[0].4,"crt":results[0].3,"graphics":results[0].1}));
    }
    std::fs::write(
        root.join(
            std::env::var("SIMS_GRAPHIC_FACTORIES_REPORT")
                .unwrap_or_else(|_| "cache/original-graphic-factories.json".into()),
        ),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
