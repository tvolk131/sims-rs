use anyhow::{Context, Result, ensure};
use sims_content::{Index, character_initialization::Prepared, saves::House};
use sims_sim::pose::{
    animator::{Settings, Slot},
    initialize::Request,
    properties, transform,
};
use sims_vm::{Object, Vm, World};

/// Original resource/calibration integration with explicit bare-model services.
/// Profile fields execute the recovered writes; appearance rebuilding is a
/// supplied no-op and geometry stays bare. This is NOT a character activation
/// test. Real house loading must service ReloadProfile with actual bindings.
#[test]
#[ignore = "requires private original game resources"]
fn original_initializer_resources_calibrate_with_explicit_bare_model_services() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let mut program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        for resident in &house
            .household
            .as_ref()
            .context("household absent")?
            .residents
        {
            let id = resident.object as i16;
            let definition = storage.world.objects[&id]
                .definition
                .clone()
                .context("resident definition absent")?;
            let object =
                Object::construct_person(definition, &program, 0).map_err(anyhow::Error::msg)?;
            let mut world = World::default();
            world.objects.insert(id, object);
            let mut vm = Vm::new(world, 1);
            vm.tick = house.simulator.ticks as u64;
            let settings = Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            };
            let mut crt = 0x51a5cafe;
            let mut prepared = Prepared::begin(&index, &mut vm, &program, id, settings, &mut crt)?;
            let context = properties::Context {
                actor: id,
                sound_visible: false,
                selected: false,
                debug_sound: false,
                position_x: -999.,
                appearance_mode: 0,
                snow: false,
                lot_kind: 1,
            };
            let person_before = vm.world.objects[&id].person.clone();
            let terrain = vec![[0; 4]; 64 * 64];
            let mut requests = vec![];
            let mut pool_queries = vec![];
            let mut snapshots = 0;
            loop {
                ensure!(
                    snapshots < 100,
                    "initializer did not finish: {source} {}",
                    resident.name
                );
                let request = prepared.advance(&index, &mut vm, &mut program, context)?;
                let mut restored: Prepared =
                    serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
                ensure!(
                    restored == prepared,
                    "pending initialization did not round trip"
                );
                let mut restored_vm = vm.clone();
                let mut restored_program = program.clone();
                ensure!(
                    restored.advance(&index, &mut restored_vm, &mut restored_program, context)?
                        == request
                );
                ensure!(
                    restored == prepared && restored_vm == vm,
                    "pending service retried with side effects"
                );
                snapshots += 1;
                let Some(request) = request else { break };
                requests.push(request.clone());
                // Continue the restored execution, so every service boundary is
                // exercised after an actual serialization/deserialization.
                prepared = restored;
                match request {
                    Request::ReloadProfile => {
                        sims_vm::person_reset::reload_bound_profile(&mut vm, &program, id)
                            .map_err(anyhow::Error::msg)?;
                        prepared
                            .execution
                            .profile_reloaded()
                            .map_err(anyhow::Error::msg)?;
                    }
                    Request::Placement => {
                        let t = prepared
                            .execution
                            .place(
                                transform::Source {
                                    position: [-16, -16],
                                    facing: 0,
                                    level: 1,
                                    flags: [0, 0],
                                },
                                transform::View {
                                    rotation: 0,
                                    floor_vector: [0.; 3],
                                },
                                transform::Terrain {
                                    size: 64,
                                    corners: &terrain,
                                },
                            )
                            .map_err(anyhow::Error::msg)?;
                        pool_queries.push(t.pool_query);
                    }
                    Request::Property(
                        properties::Request::TraceSound { .. } | properties::Request::Sound { .. },
                    ) => {
                        prepared
                            .execution
                            .property_done(None)
                            .map_err(anyhow::Error::msg)?;
                    }
                    Request::Geometry { force } => {
                        prepared
                            .execution
                            .bound
                            .refresh_geometry(force)
                            .map_err(anyhow::Error::msg)?;
                        prepared
                            .execution
                            .geometry_done()
                            .map_err(anyhow::Error::msg)?;
                    }
                    Request::Viewport => prepared
                        .execution
                        .viewport([0, 0, 800, 600])
                        .map_err(anyhow::Error::msg)?,
                    Request::Visibility { .. } => prepared
                        .execution
                        .visibility(false)
                        .map_err(anyhow::Error::msg)?,
                    other => anyhow::bail!("unhandled real initializer service {other:?}"),
                }
            }
            let initialized = &prepared.execution;
            ensure!(initialized.is_complete() && initialized.scalars.head_target_point.is_none());
            ensure!(
                initialized.scalars.head_clock == 0 && initialized.scalars.head_angles == [0.; 3]
            );
            ensure!(
                initialized
                    .bound
                    .skeleton
                    .skin
                    .animator
                    .slot(Slot::Foreground)
                    .is_none()
            );
            ensure!(
                initialized
                    .bound
                    .skeleton
                    .skin
                    .animator
                    .slot(Slot::Base)
                    .is_some()
            );
            ensure!(
                initialized
                    .scalars
                    .stop_distance
                    .iter()
                    .all(|x| x.is_finite() && *x > 0.)
            );
            ensure!(vm.world.objects[&id].data[&22] == 0);
            reports.push(
                serde_json::json!({"source": source, "object":id, "name":resident.name,
                "tick":vm.tick, "crt_state":crt, "before_person":person_before,
                "person":vm.world.objects[&id].person, "requests":requests,
                "pool_queries":pool_queries, "initialization":initialized}),
            );
        }
    }
    ensure!(reports.len() == 7);
    std::fs::write(
        root.join("cache/original-character-initialization.json"),
        serde_json::to_vec(&reports)?,
    )?;
    Ok(())
}

/// Actual record/asset services at the initializer's reload boundary. Stops
/// before geometry propagation: asset attachment is not native mesh evaluation.
#[test]
#[ignore = "requires private original household assets"]
fn original_initializer_reload_binds_assets_and_resumes_at_every_service() -> Result<()> {
    initializer_reload(false)
}

#[test]
#[ignore = "requires private original household assets"]
fn original_initializer_with_bound_geometry_completes_and_resumes() -> Result<()> {
    initializer_reload(true)
}

/// Continue complete real-resource initializers through replacement, complete
/// removal and reattachment. This exercises geometry/asset ownership together;
/// saved person deserialization and household scheduling are separate work.
#[test]
#[ignore = "requires private original assets and completed populated initializer report"]
fn original_initialized_appearances_replace_clear_and_restore_geometry() -> Result<()> {
    use serde_json::{Value, json};
    use sims_content::{
        character_geometry::{CharacterGeometry, MeshPool},
        saved_avatar::{BindingState, BoundAvatar},
    };
    use sims_sim::pose::{
        appearance::{Execution, Input},
        initialize::Initialization,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-character-populated-initialization.json"),
    )?)?;
    ensure!(rows.len() == 7);
    let mut reports = vec![];
    for row in rows {
        let id = row["object"].as_u64().context("resident ID absent")?;
        let mut initialized: Initialization =
            serde_json::from_value(row["initialization"].clone())?;
        let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let mut pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
        let asset_state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let mut avatar = BoundAvatar::restore(&index, &initialized.source.name, id, &asset_state)?;
        let mut phases = vec![];
        let order = avatar.records.saved_order();
        let original_count = avatar.avatar.parts.len();
        let mesh_count = pool.meshes.len();
        let mut restored_operations = 0;
        for phase in ["replace", "clear", "rebind"] {
            let mut last_drawable_bounds = initialized.bound.render_bounds;
            for record in order {
                let old = avatar.records.entries[record].binding;
                if phase == "clear" && old.is_some() && geometry.appearances.len() == 1 {
                    last_drawable_bounds = initialized.bound.render_bounds;
                }
                let before_pool = pool.clone();
                let execution = if phase == "clear" {
                    Execution::clear()
                } else if phase == "replace" {
                    let entry = &avatar.records.entries[record];
                    Execution::bind(Input::Replace {
                        name: entry.name.to_ascii_uppercase(),
                        texture: entry.texture.clone(),
                    })
                } else {
                    Execution::bind(Input::Current)
                };
                geometry.apply_record(
                    &index,
                    &mut avatar,
                    record,
                    execution,
                    &mut initialized.bound,
                    &mut pool,
                )?;
                // A new binding of the same canonical prototype must reuse the
                // existing pose cache even when lookup spelling has changed.
                ensure!(
                    pool == before_pool,
                    "replacement recreated mesh prototype caches"
                );
                if let Some(old) = old {
                    ensure!(!geometry.appearances.contains_key(&old.id));
                    ensure!(!avatar.part_ranges.contains_key(&old.id));
                    ensure!(avatar.records.entries[record].binding != Some(old));
                }
                let mut identities: Vec<_> = geometry.appearances.iter().collect();
                identities.sort_by_key(|(_, appearance)| **appearance);
                ensure!(identities.iter().enumerate().all(|(i, (_, a))| i == **a));
                ensure!(
                    identities.iter().map(|&(id, _)| *id).collect::<Vec<_>>()
                        == avatar.snapshot().order,
                    "asset attachment order differs from native geometry"
                );
                geometry
                    .state
                    .refresh(&mut initialized.bound, &mut pool.meshes, false)
                    .map_err(anyhow::Error::msg)?;
                let saved = avatar.snapshot();
                let restored: (Initialization, CharacterGeometry, MeshPool, BindingState) =
                    serde_json::from_slice(&serde_json::to_vec(&(
                        &initialized,
                        &geometry,
                        &pool,
                        &saved,
                    ))?)?;
                ensure!(
                    (&initialized, &geometry, &pool, &saved)
                        == (&restored.0, &restored.1, &restored.2, &restored.3)
                );
                (initialized, geometry, pool, _) = restored;
                let restored_avatar =
                    BoundAvatar::restore(&index, &initialized.source.name, id, &saved)?;
                ensure!(avatar.part_ranges == restored_avatar.part_ranges);
                ensure!(avatar.avatar.parts.len() == restored_avatar.avatar.parts.len());
                for (a, b) in avatar
                    .avatar
                    .parts
                    .iter()
                    .zip(&restored_avatar.avatar.parts)
                {
                    ensure!(format!("{:?}", a.mesh) == format!("{:?}", b.mesh));
                    ensure!(a.texture.rgba == b.texture.rgba);
                }
                avatar = restored_avatar;
                restored_operations += 1;
            }
            ensure!(pool.meshes.len() == mesh_count);
            if phase == "clear" {
                ensure!(geometry.appearances.is_empty() && geometry.state.appearances.is_empty());
                ensure!(avatar.avatar.parts.is_empty());
                // 59934b clears the local box before 5a1103 tests it, so this
                // path retains the last published render box despite having
                // no live drawable. Do not replace it with idealized bounds.
                ensure!(initialized.bound.local_bounds == sims_sim::pose::model::Bounds::EMPTY);
                ensure!(initialized.bound.render_bounds == last_drawable_bounds);
            } else {
                ensure!(
                    avatar.avatar.parts.len() == original_count && geometry.appearances.len() == 4
                );
                let b = initialized.bound.render_bounds;
                ensure!((0..3).all(|i| b.minimum[i] <= b.maximum[i]));
            }
            phases.push(
                json!({"phase":phase,"initialization":initialized,"geometry":geometry,
                "mesh_pool":pool,"asset_state":avatar.snapshot()}),
            );
        }
        ensure!(restored_operations == 15);
        // Exercise the production reload adapter on populated geometry, with
        // initialized profile fields rather than final saved person storage.
        let house = House::load(&index, row["source"].as_str().context("house source")?)?;
        let program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        let object_id = id as i16;
        let definition = storage.world.objects[&object_id]
            .definition
            .clone()
            .context("definition")?;
        let mut object =
            Object::construct_person(definition, &program, 0).map_err(anyhow::Error::msg)?;
        let words: Vec<i16> = serde_json::from_value(row["original"]["person"].clone())?;
        object.person = Some(
            words
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i as i16, v))
                .collect(),
        );
        let mut world = World::default();
        world.objects.insert(object_id, object);
        let mut vm = Vm::new(world, 1);
        vm.tick = house.simulator.ticks as u64;
        let owner = serde_json::from_value(row["prepared"]["appearance_owner"].clone())?;
        let mut service = sims_content::character_appearance::Service::begin(object_id, owner);
        let old_handles: Vec<_> = geometry.appearances.keys().copied().collect();
        let mut notifications = 0;
        while let Some(request) = service.advance_bound(
            &index,
            &mut vm,
            &program,
            &mut avatar,
            &mut geometry,
            &mut initialized.bound,
            &mut initialized.scalars.placement,
            &mut pool,
        )? {
            let saved = avatar.snapshot();
            let restored: (
                sims_content::character_appearance::Service,
                Vm,
                Initialization,
                CharacterGeometry,
                MeshPool,
                BindingState,
            ) = serde_json::from_slice(&serde_json::to_vec(&(
                &service,
                &vm,
                &initialized,
                &geometry,
                &pool,
                &saved,
            ))?)?;
            ensure!(
                (&service, &vm, &initialized, &geometry, &pool, &saved)
                    == (
                        &restored.0,
                        &restored.1,
                        &restored.2,
                        &restored.3,
                        &restored.4,
                        &restored.5
                    )
            );
            (service, vm, initialized, geometry, pool, _) = restored;
            avatar = BoundAvatar::restore(&index, &initialized.source.name, id, &saved)?;
            let before = serde_json::to_vec(&(
                &service,
                &vm,
                &initialized,
                &geometry,
                &pool,
                avatar.snapshot(),
            ))?;
            ensure!(
                service.advance_bound(
                    &index,
                    &mut vm,
                    &program,
                    &mut avatar,
                    &mut geometry,
                    &mut initialized.bound,
                    &mut initialized.scalars.placement,
                    &mut pool
                )? == Some(request.clone())
            );
            ensure!(
                before
                    == serde_json::to_vec(&(
                        &service,
                        &vm,
                        &initialized,
                        &geometry,
                        &pool,
                        avatar.snapshot()
                    ))?
            );
            use sims_content::character_appearance::Request as AppearanceRequest;
            use sims_sim::pose::appearance_selection::Request as SelectionRequest;
            match request {
                AppearanceRequest::Selection(SelectionRequest::TextureNotification { .. }) => {
                    notifications += 1
                }
                AppearanceRequest::Selection(SelectionRequest::Tint(tint)) => {
                    ensure!(tint == [0; 3])
                }
                other => anyhow::bail!("unhandled bound reload service {other:?}"),
            }
            service.acknowledge(&mut avatar.records)?;
        }
        ensure!(service.is_complete() && notifications == 1);
        ensure!(
            old_handles
                .iter()
                .all(|id| !geometry.appearances.contains_key(id))
        );
        ensure!(geometry.appearances.len() == 4 && pool.meshes.len() == mesh_count);
        geometry
            .state
            .refresh(&mut initialized.bound, &mut pool.meshes, false)
            .map_err(anyhow::Error::msg)?;
        phases.push(
            json!({"phase":"reload","initialization":initialized,"geometry":geometry,
            "mesh_pool":pool,"asset_state":avatar.snapshot()}),
        );
        reports
            .push(json!({"name":row["name"],"object":id,"source":row["source"],"phases":phases}));
    }
    std::fs::write(
        root.join("cache/original-appearance-geometry-lifecycle.json"),
        serde_json::to_vec(&reports)?,
    )?;
    Ok(())
}

fn initializer_reload(populated: bool) -> Result<()> {
    use serde_json::json;
    use sims_content::character_geometry::{CharacterGeometry, MeshPool};
    use sims_content::{character_appearance, saved_avatar::BoundAvatar};
    use sims_sim::pose::{
        appearance::{Execution, Input},
        appearance_reload::Request as ReloadRequest,
        appearance_selection::Request as SelectionRequest,
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    let mut mesh_reports = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let mut program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        for resident in &house
            .household
            .as_ref()
            .context("household absent")?
            .residents
        {
            let id = resident.object as i16;
            let definition = storage.world.objects[&id]
                .definition
                .clone()
                .context("definition absent")?;
            let module = &program.modules[&definition.source];
            let words = &module.object_definition_words[&definition.object];
            let profile = module.strings[&(words[19] as u16)].clone();
            let tones = program.modules[module.semiglobal.as_ref().unwrap()].strings[&8192].clone();
            let flags = words[0xcc / 2] as u16;
            let object =
                Object::construct_person(definition, &program, 0).map_err(anyhow::Error::msg)?;
            let mut world = World::default();
            world.objects.insert(id, object);
            let mut vm = Vm::new(world, 1);
            vm.tick = house.simulator.ticks as u64;
            let mut prepared = Prepared::begin(
                &index,
                &mut vm,
                &program,
                id,
                Settings {
                    fade_enabled: true,
                    fade_duration: 200,
                    sampling_config: 1,
                },
                &mut 0x51a5cafe,
            )?;
            let context = properties::Context {
                actor: id,
                sound_visible: false,
                selected: false,
                debug_sound: false,
                position_x: -999.,
                appearance_mode: 0,
                snow: false,
                lot_kind: 1,
            };
            ensure!(
                prepared.advance(&index, &mut vm, &mut program, context)?
                    == Some(Request::ReloadProfile)
            );
            let mut avatar = BoundAvatar::empty(&index, &profile[0], id as u64)?;
            let mut geometry = CharacterGeometry::new(avatar.avatar.skeleton.bones.len());
            let mut mesh_pool = MeshPool::default();
            let mut service = prepared.appearance_reload()?;
            let input = json!({"profile":profile,"tones":tones,"flags":flags,"model":true,"attachment":false,
                "attachment_metadata":service.owner.attachment_metadata,"accessories":false,
                "cache":service.owner.selection.cached,"tint":service.owner.selection.tint,
                "person":vm.world.objects[&id].person,
                "records":avatar.records.entries.iter().map(|r|json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})).collect::<Vec<_>>()});
            let mut events = vec![];
            let mut steps = 0;
            while let Some(request) = service.advance(&mut vm, &program, &mut avatar.records)? {
                steps += 1;
                ensure!(steps < 40, "appearance service failed to finish");
                let saved = avatar.snapshot();
                let restored: (
                    character_appearance::Service,
                    sims_content::saved_avatar::BindingState,
                    Vm,
                    CharacterGeometry,
                    MeshPool,
                ) = serde_json::from_slice(&serde_json::to_vec(&(
                    &service, &saved, &vm, &geometry, &mesh_pool,
                ))?)?;
                ensure!(restored.0 == service && restored.1 == saved && restored.2 == vm);
                ensure!(restored.3 == geometry && restored.4 == mesh_pool);
                (service, _, vm, geometry, mesh_pool) = restored;
                avatar = BoundAvatar::restore(&index, &profile[0], id as u64, &saved)?;
                let before = (service.clone(), avatar.records.clone(), vm.clone());
                ensure!(
                    service.advance(&mut vm, &program, &mut avatar.records)?
                        == Some(request.clone())
                );
                ensure!(
                    (service.clone(), avatar.records.clone(), vm.clone()) == before,
                    "reload retry mutated state"
                );
                match request {
                    character_appearance::Request::Record(ReloadRequest::ClearRecord(record)) => {
                        if populated {
                            geometry.apply_record(
                                &index,
                                &mut avatar,
                                record,
                                Execution::clear(),
                                &mut prepared.execution.bound,
                                &mut mesh_pool,
                            )?;
                        } else {
                            let dirty = avatar.apply_record(&index, record, Execution::clear())?;
                            ensure!(dirty.is_empty(), "fresh records unexpectedly had geometry");
                        }
                    }
                    character_appearance::Request::Selection(SelectionRequest::Bind {
                        record,
                        descriptor,
                    }) => {
                        events.push(json!({"bind":{"record":record,"name":descriptor.name,"texture":descriptor.texture}}));
                        let execution = Execution::bind(Input::Replace {
                            name: descriptor.name,
                            texture: descriptor.texture,
                        });
                        if populated {
                            geometry.apply_record(
                                &index,
                                &mut avatar,
                                record,
                                execution,
                                &mut prepared.execution.bound,
                                &mut mesh_pool,
                            )?;
                        } else {
                            ensure!(avatar.apply_record(&index, record, execution)?.is_empty());
                        }
                    }
                    character_appearance::Request::Selection(
                        SelectionRequest::TextureNotification { first },
                    ) => {
                        // Explicit head-cache service for this headless asset test.
                        events.push(json!({"texture_notification":if first {"4eb0cc"} else {"4eb0eb"},"guid":0x12345678u32}));
                    }
                    character_appearance::Request::Selection(SelectionRequest::Tint(tint)) => {
                        ensure!(
                            tint == [0; 3],
                            "test host does not implement tinted materials"
                        );
                        events.push(json!({"tint":tint}));
                    }
                    other => anyhow::bail!("unexpected fresh appearance request {other:?}"),
                }
                service.acknowledge(&mut avatar.records)?;
            }
            ensure!(service.is_complete() && avatar.part_ranges.len() == 4);
            let owner = &service.owner;
            let original = json!({"person":vm.world.objects[&id].person.as_ref().unwrap().values().copied().collect::<Vec<_>>(),
                "cache":owner.selection.cached,"dirty":owner.selection.dirty,"special_dirty":owner.selection.special_dirty,
                "skin_dirty":owner.selection.skin_dirty,"attachment":owner.attachment.is_some(),"attachment_metadata":owner.attachment_metadata,
                "accessories":avatar.records.accessories.len(),"records":avatar.records.entries.iter().map(|r|
                json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})).collect::<Vec<_>>()});
            prepared.appearance_reloaded(service)?;
            ensure!(prepared.execution.pending_request().is_none());
            let mut geometry_requests = vec![];
            let mut geometry_comparisons = vec![];
            if populated {
                let terrain = vec![[0; 4]; 64 * 64];
                while let Some(request) =
                    prepared.advance(&index, &mut vm, &mut program, context)?
                {
                    ensure!(
                        geometry_requests.len() < 100,
                        "populated initializer did not complete"
                    );
                    geometry_requests.push(request.clone());
                    let saved = avatar.snapshot();
                    let restored: (
                        Prepared,
                        Vm,
                        CharacterGeometry,
                        MeshPool,
                        sims_content::saved_avatar::BindingState,
                    ) = serde_json::from_slice(&serde_json::to_vec(&(
                        &prepared, &vm, &geometry, &mesh_pool, &saved,
                    ))?)?;
                    ensure!(
                        (&prepared, &vm, &geometry, &mesh_pool, &saved)
                            == (
                                &restored.0,
                                &restored.1,
                                &restored.2,
                                &restored.3,
                                &restored.4
                            )
                    );
                    (prepared, vm, geometry, mesh_pool, _) = restored;
                    avatar = BoundAvatar::restore(&index, &profile[0], id as u64, &saved)?;
                    let before = (
                        prepared.clone(),
                        vm.clone(),
                        geometry.clone(),
                        mesh_pool.clone(),
                    );
                    ensure!(
                        prepared.advance(&index, &mut vm, &mut program, context)?
                            == Some(request.clone())
                    );
                    ensure!(
                        (
                            prepared.clone(),
                            vm.clone(),
                            geometry.clone(),
                            mesh_pool.clone()
                        ) == before,
                        "pending geometry service mutated on retry"
                    );
                    match request {
                        Request::Placement => {
                            prepared
                                .execution
                                .place(
                                    transform::Source {
                                        position: [-16, -16],
                                        facing: 0,
                                        level: 1,
                                        flags: [0, 0],
                                    },
                                    transform::View {
                                        rotation: 0,
                                        floor_vector: [0.; 3],
                                    },
                                    transform::Terrain {
                                        size: 64,
                                        corners: &terrain,
                                    },
                                )
                                .map_err(anyhow::Error::msg)?;
                        }
                        Request::Geometry { force } => {
                            let input = json!({"skeleton":prepared.execution.source,"bound":prepared.execution.bound,
                                "state":geometry.state,"pool":mesh_pool,"force":force});
                            prepared.refresh_geometry(&mut geometry, &mut mesh_pool)?;
                            geometry_comparisons
                                .push(json!({"input":input,"bound":prepared.execution.bound,
                                "state":geometry.state,"pool":mesh_pool}));
                        }
                        Request::Property(
                            properties::Request::TraceSound { .. }
                            | properties::Request::Sound { .. },
                        ) => prepared
                            .execution
                            .property_done(None)
                            .map_err(anyhow::Error::msg)?,
                        Request::Viewport => prepared
                            .execution
                            .viewport([0, 0, 800, 600])
                            .map_err(anyhow::Error::msg)?,
                        Request::Visibility { .. } => prepared
                            .execution
                            .visibility(false)
                            .map_err(anyhow::Error::msg)?,
                        other => anyhow::bail!("unhandled populated initializer request {other:?}"),
                    }
                }
                ensure!(prepared.execution.is_complete());
                ensure!(
                    geometry.appearances.len() == 4
                        && geometry.state.bindings.len() == avatar.avatar.parts.len()
                );
                ensure!(!mesh_pool.meshes.is_empty());
                let b = prepared.execution.bound.local_bounds;
                ensure!(
                    (0..3).all(|i| b.maximum[i] >= b.minimum[i]),
                    "populated initialization left empty bounds"
                );
            }
            for part in &avatar.avatar.parts {
                let mesh = &part.mesh;
                let local = sims_sim::pose::mesh::LocalBounds::from_mesh(mesh)
                    .map_err(anyhow::Error::msg)?;
                let input = json!({
                    "bindings":mesh.bindings.iter().map(|b|[
                        b.bone,b.first_vertex,b.vertex_count,b.first_blend,b.blend_count
                    ]).collect::<Vec<_>>(),
                    "primary_count":mesh.vertices.len(),
                    "samples":mesh.vertices.iter().map(|v|v.position).chain(mesh.blends.iter().map(|v|v.position)).collect::<Vec<_>>(),
                    "normals":mesh.vertices.iter().map(|v|v.normal).chain(mesh.blends.iter().map(|v|v.normal)).collect::<Vec<_>>()
                });
                mesh_reports.push(json!({"source":source,"object":id,"resident":resident.name,
                    "mesh":mesh.name,"input":input,"rust":local}));
            }
            reports.push(json!({"source":source,"object":id,"name":resident.name,"input":input,"events":events,"original":original,
                "records":avatar.records,"asset_state":avatar.snapshot(),"parts":avatar.avatar.parts.len(),"steps":steps,
                "initialization":prepared.execution,"geometry":geometry,"mesh_pool":mesh_pool,
                "geometry_requests":geometry_requests,"geometry_comparisons":geometry_comparisons,"prepared":prepared,
                "person_after_initialization":vm.world.objects[&id].person,"context":context}));
        }
    }
    ensure!(reports.len() == 7);
    std::fs::write(
        root.join(if populated {
            "cache/original-character-populated-initialization.json"
        } else {
            "cache/original-appearance-reload.json"
        }),
        serde_json::to_vec(&reports)?,
    )?;
    std::fs::write(
        root.join("cache/original-mesh-local-bounds.json"),
        serde_json::to_vec(&mesh_reports)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private original-profile native reload evidence"]
fn original_initializer_reload_matches_native_fields_and_selected_bindings() -> Result<()> {
    use serde_json::Value;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rust: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-appearance-reload.json"),
    )?)?;
    let native: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-original-appearance-reload.json"),
    )?)?;
    ensure!(rust.len() == 7 && native.len() == 7);
    for (r, n) in rust.iter().zip(&native) {
        ensure!(r["input"] == n["input"]);
        ensure!(
            r["original"] == n["original"],
            "{} native reload fields differ",
            r["name"]
        );
        let selection: Vec<_> = n["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e.get("bind").is_some()
                    || e.get("texture_notification").is_some()
                    || e.get("tint").is_some()
            })
            .cloned()
            .collect();
        ensure!(
            r["events"] == serde_json::json!(selection),
            "{} native reload selection differs",
            r["name"]
        );
    }
    Ok(())
}
