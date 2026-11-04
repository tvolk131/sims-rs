use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    graphic_initialization,
    graphic_loading::Registry,
    graphic_materials::Backend,
    graphic_update::{Command, Environment, Request, Step},
};
use sims_sim::{
    lot_creation::OffLot,
    lot_links::Links,
    pose::{
        animator::Settings,
        graphic_frame,
        graphic_transform::Source,
        model::{Bound, Matrix},
        transform,
    },
};
use sims_vm::{ObjectId, Program, Vm, World, construction::ObjectDate};

#[path = "../../sims-sim/tests/common/native_graphic_bound.rs"]
mod native_graphic_bound;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "requires private original 3D indicator assets"]
fn ordinary_animator_retirement_releases_real_bindings_and_restores() -> Result<()> {
    use sims_sim::pose::graphic_destruction::Request as Destroy;
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    for name in ["PlumbBob.iff", "DateBob.iff", "StarBob.iff"] {
        let (vm, p, mut registry, mut pool, id) = create(&index, name)?;
        drive(
            &index,
            &p,
            &mut registry,
            &mut pool,
            id,
            Command::Reload { force: true },
            0,
            false,
            false,
        )?;
        let initial = registry.snapshot();
        let mut results = vec![];
        for resume in [false, true] {
            let mut registry = Registry::restore(&index, initial.clone())?;
            let mut pool = pool.clone();
            ensure!(!registry.graphics[&id].ready.avatar.avatar.parts.is_empty());
            registry.begin_retirement(id, true)?;
            ensure!(registry.begin_retirement(id, true).is_err());
            let mut focus = true;
            let mut events = vec![];
            let mut done = false;
            let mut steps = 0;
            let mut failed_notification = false;
            for i in 0..80 {
                if resume {
                    restore(&index, &mut registry, &mut pool)?;
                }
                let before = registry.snapshot();
                let before_pool = pool.clone();
                let result = registry.advance_retirement(
                    &index,
                    id,
                    &mut pool,
                    vm.tick as i32,
                    Settings {
                        fade_enabled: true,
                        fade_duration: 200,
                        sampling_config: 1,
                    },
                    focus,
                    |request| {
                        if matches!(request, Destroy::DestroyModel(_)) && !failed_notification {
                            failed_notification = true;
                            anyhow::bail!("injected presentation failure");
                        }
                        if request == Destroy::ClearFocus {
                            focus = false;
                        }
                        events.push(request);
                        Ok(())
                    },
                );
                if result.is_err() {
                    ensure!(result.unwrap_err().to_string() == "injected presentation failure");
                    ensure!(
                        registry.snapshot() == before && pool == before_pool,
                        "failed ordinary destructor service changed its transaction"
                    );
                    continue;
                }
                let complete = result?;
                steps = i + 1;
                if complete {
                    done = true;
                    break;
                }
            }
            ensure!(
                done && failed_notification
                    && steps > 8
                    && registry.graphics.is_empty()
                    && registry.retirements.is_empty()
            );
            ensure!(events == [Destroy::DestroyModel(id as u64), Destroy::ClearFocus]);
            results.push((registry.snapshot(), pool, events, steps));
        }
        ensure!(
            results[0] == results[1],
            "ordinary destructor restore changed resources"
        );
    }
    Ok(())
}
fn placement() -> graphic_initialization::Environment<'static> {
    graphic_initialization::Environment {
        owner_appearance_index: 0,
        source: Source {
            position: [160, 320],
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
        viewport: [-10, -20, 640, 480],
    }
}
fn env(tick: i32, outside: bool) -> Environment<'static> {
    Environment {
        placement: placement(),
        tick,
        outside,
        visibility_enabled: true,
        camera: Matrix::IDENTITY,
        materials: Backend {
            mode: 0,
            flags: 0,
            references: 1,
            ramp_size: 32,
        },
        normal_material_mode: 0,
    }
}
fn create(index: &Index, name: &str) -> Result<(Vm, Program, Registry, MeshPool, ObjectId)> {
    let p = sims_content::vm::load_program(index, name, "Global.iff")?;
    let source = &index.resources[index.find(name)?].locator;
    let words = p.modules[source]
        .object_definition_words
        .values()
        .next()
        .context("definition absent")?;
    let guid = u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16);
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
    let factory = OffLot::begin_resource_with_graphics(
        &mut vm,
        &p,
        &mut links,
        guid,
        ObjectDate {
            year: 1997,
            month: 3,
            day: 12,
        },
        [13, 25],
    )
    .map_err(anyhow::Error::msg)?;
    let id = factory.object().context("allocation failed")?;
    let mut registry = Registry::default();
    registry.begin(&vm, &p, id)?;
    let mut pool = MeshPool::default();
    let mut crt = 1;
    let mut count = 0;
    while !registry.advance(
        index,
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
        placement(),
    )? {
        count += 1;
        ensure!(count < 30);
    }
    Ok((vm, p, registry, pool, id))
}
fn restore(index: &Index, registry: &mut Registry, pool: &mut MeshPool) -> Result<()> {
    let saved = registry.snapshot();
    *registry = Registry::restore(index, serde_json::from_slice(&serde_json::to_vec(&saved)?)?)?;
    ensure!(registry.snapshot() == saved);
    let saved = pool.clone();
    *pool = serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
    ensure!(*pool == saved);
    Ok(())
}
fn drive(
    index: &Index,
    p: &Program,
    registry: &mut Registry,
    pool: &mut MeshPool,
    id: ObjectId,
    command: Command,
    tick: i32,
    outside: bool,
    resume: bool,
) -> Result<Vec<Request>> {
    registry
        .graphics
        .get_mut(&id)
        .unwrap()
        .begin_update(command.clone())?;
    ensure!(
        registry
            .graphics
            .get_mut(&id)
            .unwrap()
            .begin_update(command)
            .is_err()
    );
    let mut callbacks = vec![];
    for _ in 0..100 {
        if resume {
            restore(index, registry, pool)?;
        }
        let step = registry.graphics.get_mut(&id).unwrap().advance_update(
            index,
            p,
            pool,
            env(tick, outside),
        )?;
        match step {
            Step::Complete => return Ok(callbacks),
            Step::Progress => (),
            Step::Pending(request) => {
                let before = registry.snapshot();
                let before_pool = pool.clone();
                ensure!(
                    registry.graphics.get_mut(&id).unwrap().advance_update(
                        index,
                        p,
                        pool,
                        env(tick + 99, !outside)
                    )? == Step::Pending(request.clone())
                );
                ensure!(
                    registry.snapshot() == before && *pool == before_pool,
                    "observing a callback advanced state"
                );
                if resume {
                    restore(index, registry, pool)?;
                }
                match &request {
                    // Supplied world service, exactly as in the native root
                    // fixture. The base cXObject +78 is an actual native no-op.
                    Request::Invalidate { object } | Request::Position { object, .. } => {
                        ensure!(*object == id)
                    }
                    other => anyhow::bail!("unexpected source marker callback {other:?}"),
                }
                let g = registry.graphics.get_mut(&id).unwrap();
                ensure!(
                    g.acknowledge_update(&Request::Invalidate { object: id + 1 })
                        .is_err()
                );
                g.acknowledge_update(&request)?;
                ensure!(g.acknowledge_update(&request).is_err());
                callbacks.push(request);
            }
        }
    }
    anyhow::bail!("ordinary update exceeded test service budget")
}

#[cfg(feature = "private-fixtures")]
#[test]
#[ignore = "requires private original indicator assets"]
fn retained_graphic_frame_adapter_matches_native_root_prefixes() -> Result<()> {
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    let (_, p, initial, _, id) = create(&index, "PlumbBob.iff")?;
    let initial = initial.snapshot();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../sims-sim/tests/fixtures/native-graphic-root-frames.json"
    ))?;
    let mut frames = 0;
    for sequence in fixture["sequences"].as_array().unwrap() {
        let mut registry = Registry::restore(&index, initial.clone())?;
        let mut pool = MeshPool::default();
        let input = &sequence["input"];
        let mut crt = input["crt_state"].as_u64().unwrap() as u32;
        let g = registry.graphics.get_mut(&id).unwrap();
        g.ready.bound = Bound::root(&mut crt).map_err(anyhow::Error::msg)?;
        g.ready
            .bound
            .model
            .set_scale(serde_json::from_value(input["scale"].clone())?)
            .map_err(anyhow::Error::msg)?;
        let matrix = serde_json::from_value(input["matrix"].clone())?;
        g.ready.bound.publish(matrix).map_err(anyhow::Error::msg)?;
        g.ready.frame = serde_json::from_value(sequence["initial"]["frame"].clone())?;
        g.ready.geometry = CharacterGeometry::new(1);
        native_graphic_bound::compare(&g.ready.bound, &sequence["initial"]["bound"], 0.);
        // These prefixes use visible=true, matching native dirty visibility.
        // Later fixture frames deliberately stub visibility=false despite +d9;
        // those are covered by the lower controller oracle, not claimed here.
        for step in sequence["steps"].as_array().unwrap().iter().take(3) {
            let a = &step["action"];
            drive(
                &index,
                &p,
                &mut registry,
                &mut pool,
                id,
                Command::Frame {
                    entry: serde_json::from_value(a["entry"].clone())?,
                    only_visible: a["only_visible"].as_bool().unwrap(),
                },
                a["tick"].as_i64().unwrap() as i32,
                a["outside"].as_bool().unwrap(),
                true,
            )?;
            let g = &registry.graphics[&id];
            ensure!(g.ready.frame == serde_json::from_value(step["frame"].clone())?);
            native_graphic_bound::compare(&g.ready.bound, &step["bound"], 0.);
            frames += 1;
        }
    }
    ensure!(frames == 24);
    Ok(())
}

#[test]
#[ignore = "requires private original indicator assets"]
fn original_marker_updates_retain_assets_materials_and_every_service_boundary() -> Result<()> {
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    let mut rows = vec![];
    for (name, kind) in [("PlumbBob.iff", 1), ("DateBob.iff", 2), ("StarBob.iff", 3)] {
        let (vm, p, initial, pool, id) = create(&index, name)?;
        let baseline_vm = vm.clone();
        let initial = initial.snapshot();
        let mut results = vec![];
        for resume in [false, true] {
            let mut registry = Registry::restore(&index, initial.clone())?;
            let mut pool = pool.clone();
            let mut matrix = Matrix::IDENTITY;
            matrix.values[12..15].copy_from_slice(&[60., 3., -40.]);
            matrix.kind = 2;
            // Non-unit scale catches accidentally copying the skeleton matrix
            // back to Animator +38 and applying its scale a second time.
            registry
                .graphics
                .get_mut(&id)
                .unwrap()
                .ready
                .bound
                .model
                .set_scale([2., 3., 4.])
                .map_err(anyhow::Error::msg)?;
            registry
                .graphics
                .get_mut(&id)
                .unwrap()
                .ready
                .appearance
                .tint = [2, 127, 1];
            let mut callbacks = vec![];
            for command in [
                Command::BindTransform(matrix),
                Command::TransformControl(true),
                Command::Reload { force: true },
            ] {
                callbacks.extend(drive(
                    &index,
                    &p,
                    &mut registry,
                    &mut pool,
                    id,
                    command,
                    0,
                    false,
                    resume,
                )?);
            }
            ensure!(registry.graphics[&id].ready.frame.matrix == matrix);
            ensure!(registry.graphics[&id].ready.bound.matrix != matrix);
            for (tick, entry, only_visible, outside, visible_byte) in [
                (0, graphic_frame::Entry::Refresh, false, false, 1),
                (1, graphic_frame::Entry::Ordinary, true, false, 1),
                (2, graphic_frame::Entry::Refresh, true, false, 0),
                (2, graphic_frame::Entry::Refresh, false, false, 0),
                (3, graphic_frame::Entry::Ordinary, true, true, 1),
                (4, graphic_frame::Entry::Ordinary, false, false, 1),
            ] {
                registry.graphics.get_mut(&id).unwrap().ready.frame.visible = visible_byte;
                callbacks.extend(drive(
                    &index,
                    &p,
                    &mut registry,
                    &mut pool,
                    id,
                    Command::Frame {
                        entry,
                        only_visible,
                    },
                    tick,
                    outside,
                    resume,
                )?);
            }
            let g = registry.graphics.get_mut(&id).unwrap();
            ensure!(g.ready.appearance.kind == kind && g.ready.appearance.marker);
            ensure!(
                !g.materials
                    .as_ref()
                    .context("materials absent")?
                    .published
                    .is_empty()
            );
            g.ready
                .geometry
                .update_loaded_vertices(&g.ready.avatar, &g.ready.bound, false)?;
            ensure!(!g.ready.geometry.vertices.is_empty());
            ensure!(
                callbacks
                    .iter()
                    .filter(|c| matches!(c, Request::Invalidate { .. }))
                    .count()
                    == 4
            );
            registry.validate(&vm)?;
            ensure!(vm == baseline_vm, "asset services changed VM execution");
            results.push((registry.snapshot(), pool, callbacks));
        }
        ensure!(
            results[0] == results[1],
            "save/resume changed ordinary services"
        );
        let registry = Registry::restore(&index, results.remove(1).0)?;
        let g = &registry.graphics[&id];
        rows.push(json!({"owner": id, "source": g.ready.definition.source, "skeleton": g.ready.avatar.avatar.skeleton.name,
            "bound": g.ready.bound, "geometry": g.ready.geometry, "asset_state": g.ready.avatar.snapshot(),
            "state": g.ready.appearance, "frame": g.ready.frame, "materials": g.materials,
            "scope": "Retained factory owner, copied transform, explicit transform control, original mesh reload, frame services and material publication. World invalidation supplied; no live selection or head attachment claimed."}));
    }
    std::fs::write(
        root().join("cache/original-retained-graphic-updates.json"),
        serde_json::to_vec(&rows)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original indicator assets"]
fn failed_marker_bind_remains_pending_and_retries_after_restoring_assets() -> Result<()> {
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    let (vm, p, mut registry, mut pool, id) = create(&index, "PlumbBob.iff")?;
    let saved_vm = vm.clone();
    let missing = Index {
        root: Default::default(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    };
    registry
        .graphics
        .get_mut(&id)
        .unwrap()
        .begin_update(Command::Reload { force: true })?;
    ensure!(
        registry.graphics.get_mut(&id).unwrap().advance_update(
            &index,
            &p,
            &mut pool,
            env(0, false)
        )? == Step::Progress
    );
    ensure!(
        registry
            .graphics
            .get_mut(&id)
            .unwrap()
            .advance_update(&missing, &p, &mut pool, env(0, false))
            .is_err()
    );
    let failed = registry.snapshot();
    let failed_pool = pool.clone();
    restore(&index, &mut registry, &mut pool)?;
    ensure!(
        registry
            .graphics
            .get_mut(&id)
            .unwrap()
            .advance_update(&missing, &p, &mut pool, env(0, false))
            .is_err()
    );
    ensure!(registry.snapshot() == failed && pool == failed_pool);
    let mut complete = false;
    for _ in 0..20 {
        restore(&index, &mut registry, &mut pool)?;
        if registry.graphics.get_mut(&id).unwrap().advance_update(
            &index,
            &p,
            &mut pool,
            env(0, false),
        )? == Step::Complete
        {
            complete = true;
            break;
        }
    }
    ensure!(complete && !registry.graphics[&id].ready.avatar.avatar.parts.is_empty());
    ensure!(vm == saved_vm);
    Ok(())
}

#[test]
#[ignore = "requires private original household and indicator assets"]
fn live_household_graphic_services_restore_without_running_scripts_or_teleporting() -> Result<()> {
    use sims_content::character_loading::live::graphic::View;
    use sims_runtime::original_session::{Options, Session};
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    let initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?.save();
    let mut results = vec![];
    for resume in [false, true] {
        let mut s = Session::restore(&index, initial.clone())?;
        let id = s
            .simulation
            .native_lot
            .as_ref()
            .unwrap()
            .indicators
            .primary
            .context("primary absent")?;
        let mut expected_vm = s.simulation.vm.clone();
        expected_vm
            .refresh_object(id, id, 0, 0)
            .map_err(anyhow::Error::msg)?;
        let mut matrix = Matrix::IDENTITY;
        matrix.values[12..15].copy_from_slice(&[60., 3., -40.]);
        matrix.kind = 2;
        let mut total = 0;
        for command in [
            Command::BindTransform(matrix),
            Command::TransformControl(true),
            Command::Reload { force: true },
            Command::Frame {
                entry: graphic_frame::Entry::Refresh,
                only_visible: false,
            },
        ] {
            s.host
                .ordinary
                .graphics
                .get_mut(&id)
                .unwrap()
                .begin_update(command)?;
            ensure!(s.host.ordinary.updates_pending());
            let before = s.simulation.clone();
            ensure!(
                s.step()
                    .unwrap_err()
                    .to_string()
                    .contains("graphic service must finish")
            );
            ensure!(
                s.simulation == before,
                "scheduler advanced while graphics were suspended"
            );
            let mut complete = false;
            for _ in 0..100 {
                if resume && [0, 3, 6, 10].contains(&total) {
                    let saved = s.save();
                    let assets = saved.assets.clone();
                    let before = s.simulation.clone();
                    s = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )?;
                    ensure!(s.simulation == before && s.host.snapshot() == assets);
                }
                total += 1;
                let step = s.host.service_graphic(
                    &mut s.simulation,
                    &s.program,
                    id,
                    View {
                        viewport: s.host.presentation.viewport,
                        outside: true,
                        visibility_enabled: true,
                        camera: Matrix::IDENTITY,
                        materials: Backend {
                            mode: 0,
                            flags: 0,
                            references: 1,
                            ramp_size: 32,
                        },
                        normal_material_mode: 0,
                    },
                )?;
                match step {
                    Step::Complete => {
                        complete = true;
                        break;
                    }
                    Step::Progress => (),
                    Step::Pending(request) => {
                        anyhow::bail!("unserviced live marker request {request:?}")
                    }
                }
            }
            ensure!(complete && !s.host.ordinary.updates_pending());
        }
        ensure!(
            s.simulation.vm == expected_vm,
            "ordinary refresh changed VM beyond native drawing invalidation"
        );
        ensure!(
            s.simulation.vm.world.positions[&id]
                == initial.simulation.state.vm.world.positions[&id]
        );
        results.push((s.simulation, s.host.snapshot(), total));
    }
    ensure!(results[0] == results[1]);
    Ok(())
}
