use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    saved_avatar::{BindingState, BoundAvatar},
    saves::House,
};
use sims_sim::pose::{
    appearance_read::{Read, Request},
    appearance_reload::Owner,
    initialize::Initialization,
};

/// Actual modern saved appearance records installed onto complete initialized
/// models. The following animation descriptor/read-tail services remain separate.
#[test]
#[ignore = "requires private original assets and complete populated initializer report"]
fn original_saved_appearances_replace_initialized_geometry_and_resume() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-character-populated-initialization.json"),
    )?)?;
    ensure!(rows.len() == 7);
    let mut report = vec![];
    for row in rows {
        let id = row["object"].as_u64().context("resident object")?;
        let house = House::load(&index, row["source"].as_str().context("house source")?)?;
        let saved = house.objects.instances[&(id as u16)]
            .person
            .as_ref()
            .context("saved person")?;
        let mut initialized: Initialization =
            serde_json::from_value(row["initialization"].clone())?;
        let mut owner: Owner = serde_json::from_value(row["prepared"]["appearance_owner"].clone())?;
        let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let mut pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
        let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let mut avatar = BoundAvatar::restore(&index, &initialized.source.name, id, &state)?;
        let input = json!({"profile":[],"records":avatar.records.entries.iter().map(|r|
            json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})).collect::<Vec<_>>(),
            "cache":owner.selection.cached,"attachment":owner.attachment.is_some(),"attachment_metadata":owner.attachment_metadata,"skip_bind":false,
            "version":house.objects.version,"fixed":saved.appearance.chunks_exact(2).collect::<Vec<_>>(),
            "saved_accessories":saved.accessories.iter().map(|a|[&a.name,&a.binding]).collect::<Vec<_>>()});
        let old_ids: Vec<_> = geometry.appearances.keys().copied().collect();
        let mut read =
            Read::from_saved(&avatar.records, saved, false).map_err(anyhow::Error::msg)?;
        let mut notifications = 0;
        loop {
            let pending = geometry.advance_saved_appearance(
                &index,
                &mut avatar,
                &mut read,
                &mut owner,
                &mut initialized.bound,
                &mut initialized.scalars.placement,
                &mut pool,
            )?;
            let state = avatar.snapshot();
            let restored: (
                Read,
                Owner,
                Initialization,
                CharacterGeometry,
                MeshPool,
                BindingState,
            ) = serde_json::from_slice(&serde_json::to_vec(&(
                &read,
                &owner,
                &initialized,
                &geometry,
                &pool,
                &state,
            ))?)?;
            ensure!(
                (&read, &owner, &initialized, &geometry, &pool, &state)
                    == (
                        &restored.0,
                        &restored.1,
                        &restored.2,
                        &restored.3,
                        &restored.4,
                        &restored.5
                    )
            );
            (read, owner, initialized, geometry, pool, _) = restored;
            avatar = BoundAvatar::restore(&index, &initialized.source.name, id, &state)?;
            let before = serde_json::to_vec(&(
                &read,
                &owner,
                &initialized,
                &geometry,
                &pool,
                avatar.snapshot(),
            ))?;
            ensure!(
                geometry.advance_saved_appearance(
                    &index,
                    &mut avatar,
                    &mut read,
                    &mut owner,
                    &mut initialized.bound,
                    &mut initialized.scalars.placement,
                    &mut pool
                )? == pending
            );
            ensure!(
                before
                    == serde_json::to_vec(&(
                        &read,
                        &owner,
                        &initialized,
                        &geometry,
                        &pool,
                        avatar.snapshot()
                    ))?
            );
            let Some(request) = pending else { break };
            ensure!(
                request == Request::TextureNotification,
                "unhandled saved appearance service {request:?}"
            );
            notifications += 1;
            read.acknowledge(&mut owner, &mut avatar.records)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(read.is_complete() && initialized.is_complete());
        ensure!(
            old_ids
                .iter()
                .all(|id| !geometry.appearances.contains_key(id))
        );
        for (record, pair) in avatar
            .records
            .saved_order()
            .into_iter()
            .zip(saved.appearance.chunks_exact(2))
        {
            ensure!(
                avatar.records.entries[record].name == pair[0]
                    && avatar.records.entries[record].texture == pair[1]
            );
        }
        ensure!(avatar.records.accessories.len() == saved.accessories.len());
        let direct = sims_content::saved_avatar::load(&index, &initialized.source.name, saved)?;
        ensure!(direct.parts.len() == avatar.avatar.parts.len());
        for (a, b) in direct.parts.iter().zip(&avatar.avatar.parts) {
            ensure!(
                format!("{:?}", a.mesh) == format!("{:?}", b.mesh)
                    && a.texture.rgba == b.texture.rgba
            );
        }
        ensure!(geometry.appearances.len() == avatar.part_ranges.len());
        // No geometry refresh is invented between the appearance prefix and
        // descriptor reads. The full native tail will service this dirty state.
        ensure!(initialized.bound.skeleton.geometry.dirty);
        ensure!(initialized.scalars.placement.dirty && owner.selection.dirty);
        report.push(json!({"source":row["source"],"name":row["name"],"object":id,"input":input,
            "initialization":initialized,"geometry":geometry,"mesh_pool":pool,"asset_state":avatar.snapshot(),
            "read":read,"owner":owner,"notifications":notifications,
            "person_before_saved_read":row["person_after_initialization"],"context":row["context"]}));
    }
    std::fs::write(
        root.join("cache/original-saved-appearance-prefix.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

/// Original BMF samples and initialized hierarchy, with supplied renderer
/// metadata. Export inputs/results for the native vertex probe; drawable
/// construction and the enclosing saved-animation tail remain separate.
#[test]
#[ignore = "requires private original assets and saved-appearance prefix report"]
fn original_saved_appearance_vertex_inputs_and_resume() -> Result<()> {
    use sims_sim::pose::{
        mesh::Pose,
        model::Bounds,
        vertices::{Buffer, Vertex},
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-saved-appearance-prefix.json"),
    )?)?;
    ensure!(rows.len() == 7);
    let mut output = vec![];
    for row in rows {
        let initialized: Initialization = serde_json::from_value(row["initialization"].clone())?;
        let asset_state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let avatar = BoundAvatar::restore(
            &index,
            &initialized.source.name,
            row["object"].as_u64().unwrap(),
            &asset_state,
        )?;
        let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let notifications = geometry.update_loaded_vertices(&avatar, &initialized.bound, false)?;
        let order: Vec<_> = geometry
            .state
            .appearances
            .iter()
            .flatten()
            .copied()
            .collect();
        ensure!(notifications == order && notifications.len() == avatar.avatar.parts.len());
        let mut restored: CharacterGeometry =
            serde_json::from_slice(&serde_json::to_vec(&geometry)?)?;
        ensure!(geometry == restored);
        ensure!(
            restored.update_loaded_vertices(&avatar, &initialized.bound, false)? == notifications
        );
        ensure!(
            geometry == restored,
            "loaded appearance repeat changed geometry"
        );
        for (part, asset) in avatar.avatar.parts.iter().enumerate() {
            let mesh = &asset.mesh;
            let poses = mesh
                .bindings
                .iter()
                .map(|b| {
                    let bone = avatar
                        .avatar
                        .skeleton
                        .bones
                        .iter()
                        .position(|v| v.name == mesh.bones[b.bone])
                        .context("vertex bone")?;
                    Ok(Some(
                        Pose::from_absolute(
                            initialized.bound.skeleton.hierarchy.absolute[bone],
                            initialized.bound.skeleton.hierarchy.updates[bone],
                        )
                        .map_err(anyhow::Error::msg)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let vertices: Vec<_> = mesh
                .vertices
                .iter()
                .map(|v| Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                })
                .chain(mesh.blends.iter().map(|v| Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: [0.; 2],
                }))
                .collect();
            let samples: Vec<_> = vertices
                .iter()
                .map(|v| v.position.into_iter().chain(v.normal).collect::<Vec<_>>())
                .collect();
            let initial_vertices: Vec<_> = vertices
                .iter()
                .map(|v| {
                    v.position
                        .into_iter()
                        .chain(v.normal)
                        .chain(v.uv)
                        .collect::<Vec<_>>()
                })
                .collect();
            let bindings: Vec<_> = mesh
                .bindings
                .iter()
                .map(|b| {
                    [
                        b.bone,
                        b.first_vertex,
                        b.vertex_count,
                        b.first_blend,
                        b.blend_count,
                        0,
                    ]
                })
                .collect();
            let blends: Vec<_> = mesh
                .blends
                .iter()
                .map(|b| [(b.weight * 32768.) as usize, b.vertex])
                .collect();
            for rest in [false, true] {
                let mut buffer = Buffer {
                    vertices: vertices.clone(),
                    flags: 6,
                    buffer_flags: 8,
                    range: [0, mesh.vertices.len() as i32 - 1],
                    bounds: Bounds::EMPTY,
                };
                let bound = Bounds {
                    minimum: [-4., -5., -6.],
                    maximum: [7., 8., 9.],
                };
                let notification = buffer
                    .update(mesh, &poses, rest, false, bound)
                    .map_err(anyhow::Error::msg)?;
                let mut restored: Buffer = serde_json::from_slice(&serde_json::to_vec(&buffer)?)?;
                ensure!(buffer == restored);
                ensure!(
                    restored
                        .update(mesh, &poses, rest, false, bound)
                        .map_err(anyhow::Error::msg)?
                        == notification
                );
                ensure!(buffer == restored, "vertex update changed on repeat");
                if !rest {
                    let (&identity, range) = avatar
                        .part_ranges
                        .iter()
                        .find(|(_, range)| range.contains(&part))
                        .context("vertex appearance range")?;
                    let binding = geometry.state.appearances[geometry.appearances[&identity]]
                        [part - range.start];
                    let published = &geometry.vertices[&binding];
                    ensure!(
                        published.vertices == buffer.vertices && published.range == buffer.range
                    );
                    ensure!(published.buffer_flags | 8 == buffer.buffer_flags);
                    ensure!(
                        geometry.state.bindings[binding].drawable.bounds
                            == geometry.state.bindings[binding].bounds
                    );
                }
                output.push(json!({"name":row["name"],"object":row["object"],"part":part,"mesh":mesh.name,
                    "input":{"rest":rest,"cached_drawable":false,"flags":6,"buffer_flags":8,"samples":samples,
                        "initial_vertices":initial_vertices,"bindings":bindings,"bones":poses,"blends":blends},
                    "rust":buffer,"notification":notification}));
            }
        }
    }
    ensure!(
        output.len() == 58,
        "original saved mesh coverage changed: {}",
        output.len()
    );
    std::fs::write(
        root.join("cache/original-saved-appearance-vertices.json"),
        serde_json::to_vec(&output)?,
    )?;
    Ok(())
}

/// Preserve the actual initialized owner when entering the saved animator tail.
/// This stops at its first service; callback-time person storage is separate.
#[test]
#[ignore = "requires private original assets and saved-appearance prefix report"]
fn original_initialized_saved_character_handoff_and_resume() -> Result<()> {
    use sims_content::{character_model, saved_layers};
    use sims_sim::{
        character::Resident,
        pose::{
            animator::Slot,
            layer::Descriptor,
            properties,
            refresh::{Censor, Request as CharacterRequest},
        },
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-saved-appearance-prefix.json"),
    )?)?;
    ensure!(rows.len() == 7);
    let mut report = vec![];
    for row in rows {
        let id = row["object"].as_u64().context("resident object")?;
        let house = House::load(&index, row["source"].as_str().context("house source")?)?;
        let saved = house.objects.instances[&(id as u16)]
            .person
            .as_ref()
            .context("saved person")?;
        let initialized: Initialization = serde_json::from_value(row["initialization"].clone())?;
        let owner: Owner = serde_json::from_value(row["owner"].clone())?;
        let context = properties::Context {
            actor: id as i16,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: -999.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        };
        let mut program = sims_vm::Program::default();
        let names = initialized
            .bound
            .skeleton
            .skin
            .animator
            .layers()
            .iter()
            .map(|owned| owned.layer.name.clone())
            .collect();
        sims_content::assets::load_named_animations(&index, &mut program, &names)?;
        let now = initialized.scalars.clock;
        let (mut resident, mut model) = saved_layers::restore_initialized(
            &index,
            &mut program,
            saved,
            initialized.clone(),
            Censor {
                binding: owner.attachment,
                mask: owner.attachment_metadata as i32,
            },
            house.simulator.ticks as u64,
            context,
            now,
        )?;
        resident
            .character
            .validate(&program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            resident.character.clock == initialized.scalars.clock
                && resident.character.head.last_clock == initialized.scalars.head_clock
        );
        ensure!(resident.character.visible == initialized.visible.unwrap());
        ensure!(resident.character.head.target_point == Some(saved.floats[3..6].try_into()?));
        ensure!(resident.character.placement.offset == saved.floats[6..9]);
        ensure!(resident.character.placement.base == initialized.scalars.placement.base);
        // Live handoff supplies the initialized empty route cache. The saved
        // initializer predates that handoff; all other movement state transfers
        // unchanged. Check the cache too rather than ignoring the path field.
        let mut expected_movement = initialized.movement.clone();
        ensure!(expected_movement.path.is_none());
        expected_movement.path = Some(sims_sim::pose::movement::path::Path::initialized(
            sims_sim::pose::movement::path::Settings::COMPLETE_COLLECTION,
            initialized.scalars.phase_window,
            initialized.scalars.stop_distance,
            initialized.scalars.clip_distances,
        ));
        ensure!(
            resident.controls.movement == expected_movement
                && resident.controls.movement_anchors == initialized.anchors
        );
        ensure!(
            resident.controls.animation_events == saved.animation_events
                && resident.controls.engaged == saved.engaged
        );
        ensure!(
            model.phase_window == initialized.scalars.phase_window
                && model.stop_distance == initialized.scalars.stop_distance
                && model.clip_distances == initialized.scalars.clip_distances
        );
        for (slot, text) in [Slot::Foreground, Slot::Carry, Slot::Base]
            .into_iter()
            .zip(&saved.animations)
        {
            let expected = Descriptor::parse(text).map(|d| d.name);
            let actual = resident.character.skin.animator.slot(slot).map(|id| {
                resident
                    .character
                    .skin
                    .animator
                    .layer(id)
                    .unwrap()
                    .name
                    .clone()
            });
            ensure!(
                actual == expected,
                "saved descriptor lost at resident handoff"
            );
        }
        // The descriptor read changes the animator. Every other initialized
        // model field, including dormant noise and caches, transfers exactly.
        let mut expected = initialized.bound.clone();
        expected.skeleton.skin.animator = resident.character.skin.animator.clone();
        ensure!(model.with_bound(&mut resident, |bound| Ok(bound.clone()))? == expected);
        let before = (resident.clone(), model.clone());
        let error: Result<()> = model.with_bound(&mut resident, |bound| {
            bound.skeleton.geometry.generation = 17;
            bound.local_bounds = sims_sim::pose::model::Bounds::EMPTY;
            anyhow::bail!("deliberate interrupted host service")
        });
        ensure!(error.is_err() && before == (resident.clone(), model.clone()));
        resident.controls.geometry_generation =
            resident.controls.geometry_generation.wrapping_add(1);
        ensure!(
            model.with_bound(&mut resident, |bound| Ok(bound
                .skeleton
                .geometry
                .generation))?
                == resident.controls.geometry_generation
        );
        (resident, model) = before;
        // No person word may be read before this first explicit service.
        let mut fields = std::collections::BTreeMap::new();
        ensure!(
            resident
                .character
                .next(&program, &mut fields, context, 1)
                .map_err(anyhow::Error::msg)?
                == Some(CharacterRequest::Invalidate)
        );
        ensure!(fields.is_empty());
        let (mut resumed, resumed_model): (Resident, character_model::State) =
            serde_json::from_slice(&serde_json::to_vec(&(&resident, &model))?)?;
        ensure!((&resident, &model) == (&resumed, &resumed_model));
        ensure!(
            resumed
                .character
                .next(&program, &mut fields, context, 1)
                .map_err(anyhow::Error::msg)?
                == Some(CharacterRequest::Invalidate)
        );
        ensure!(resident == resumed);
        report.push(json!({"source":row["source"],"name":row["name"],"object":id,
            "resident":resident,"model":model,"skeleton":initialized.source,
            "geometry":row["geometry"],"mesh_pool":row["mesh_pool"],"asset_state":row["asset_state"]}));
    }
    std::fs::write(
        root.join("cache/original-initialized-saved-characters.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

/// Full modern animator read tail, before the subsequent saved person words.
/// Uses actual initialized person storage, saved generic placement and original
/// terrain. Viewport invalidation and silent audio are recorded headless host
/// services. This does not run post-load Reset or ordinary household updates.
#[test]
#[ignore = "requires private original assets and regenerated initializer/appearance reports"]
fn original_initialized_saved_animation_tail_with_geometry_and_resume() -> Result<()> {
    use sims_content::{character_model, saved_layers};
    use sims_sim::{
        character::Resident,
        pose::{
            properties,
            refresh::{Censor, Request as CharacterRequest, Response},
            transform,
        },
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-saved-appearance-prefix.json"),
    )?)?;
    ensure!(rows.len() == 7);
    let mut report = vec![];
    for row in rows {
        let id = row["object"].as_u64().context("resident object")?;
        let house = House::load(&index, row["source"].as_str().context("house source")?)?;
        let object = &house.objects.instances[&(id as u16)];
        let saved = object.person.as_ref().context("saved person")?;
        let initialized: Initialization = serde_json::from_value(row["initialization"].clone())?;
        let owner: Owner = serde_json::from_value(row["owner"].clone())?;
        let asset_state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
        let avatar = BoundAvatar::restore(&index, &initialized.source.name, id, &asset_state)?;
        let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
        let mut pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
        let context: properties::Context = serde_json::from_value(row["context"].clone())?;
        let mut fields: std::collections::BTreeMap<i16, i16> =
            serde_json::from_value(row["person_before_saved_read"].clone())?;
        ensure!(fields.len() == 256);
        let before_fields = fields.clone();
        let mut program = sims_vm::Program::default();
        let names = initialized
            .bound
            .skeleton
            .skin
            .animator
            .layers()
            .iter()
            .map(|owned| owned.layer.name.clone())
            .collect();
        sims_content::assets::load_named_animations(&index, &mut program, &names)?;
        let now = initialized.scalars.clock;
        let (mut resident, mut model) = saved_layers::restore_initialized(
            &index,
            &mut program,
            saved,
            initialized,
            Censor {
                binding: owner.attachment,
                mask: owner.attachment_metadata as i32,
            },
            house.simulator.ticks as u64,
            context,
            now,
        )?;
        let terrain = house.corner_heights()?;
        // Floor-1 inputs make the explicit headless view's floor vector unused.
        ensure!(
            object.position[2] == 1,
            "read-tail corpus needs an upper-floor view profile"
        );
        let mut requests = vec![];
        let mut vertex_passes = 0;
        while let Some(request) = resident
            .character
            .next(&program, &mut fields, context, object.position[2])
            .map_err(anyhow::Error::msg)?
        {
            ensure!(requests.len() < 100, "saved animator tail did not complete");
            requests.push(request.clone());
            let restored: (
                Resident,
                character_model::State,
                CharacterGeometry,
                MeshPool,
                std::collections::BTreeMap<i16, i16>,
            ) = serde_json::from_slice(&serde_json::to_vec(&(
                &resident, &model, &geometry, &pool, &fields,
            ))?)?;
            ensure!(
                (&resident, &model, &geometry, &pool, &fields)
                    == (
                        &restored.0,
                        &restored.1,
                        &restored.2,
                        &restored.3,
                        &restored.4
                    )
            );
            (resident, model, geometry, pool, fields) = restored;
            let before = resident.clone();
            ensure!(
                resident
                    .character
                    .next(&program, &mut fields, context, object.position[2])
                    .map_err(anyhow::Error::msg)?
                    == Some(request.clone())
            );
            ensure!(resident == before);
            if let Some(notifications) =
                geometry.service_resident(&mut model, &mut resident, &avatar, &mut pool)?
            {
                if request == CharacterRequest::LoadedAppearance {
                    ensure!(
                        notifications
                            == geometry
                                .state
                                .appearances
                                .iter()
                                .flatten()
                                .copied()
                                .collect::<Vec<_>>()
                    );
                    vertex_passes += 1;
                }
                continue;
            }
            match request {
                CharacterRequest::Placement => {
                    resident
                        .character
                        .place(
                            transform::Source {
                                position: [object.position[1], object.position[0]],
                                facing: object.data[1],
                                level: object.position[2],
                                flags: [fields[&84] as u8, fields[&68] as u8],
                            },
                            transform::View {
                                rotation: house.view.camera_direction.try_into()?,
                                floor_vector: [0.; 3],
                            },
                            transform::Terrain {
                                size: house.width,
                                corners: &terrain,
                            },
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                CharacterRequest::Invalidate => resident
                    .character
                    .acknowledge(Response::Done)
                    .map_err(anyhow::Error::msg)?,
                CharacterRequest::Property(
                    properties::Request::Sound { .. } | properties::Request::TraceSound { .. },
                ) => resident
                    .character
                    .acknowledge(Response::Done)
                    .map_err(anyhow::Error::msg)?,
                CharacterRequest::RefreshClock => resident
                    .character
                    .acknowledge(Response::Clock(sims_sim::pose::ordinary::clock(
                        house.simulator.ticks as i32,
                        resident.controls.ordinary.clock_mode,
                    )))
                    .map_err(anyhow::Error::msg)?,
                CharacterRequest::CarriedObject => {
                    ensure!(
                        object.slots.iter().skip(1).all(|s| s.object == 0),
                        "resident has a carried-object dependency"
                    );
                    resident
                        .character
                        .acknowledge(Response::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                other => anyhow::bail!(
                    "resident {} pending real saved-tail service {other:?}",
                    row["name"]
                ),
            }
        }
        ensure!(!resident.character.busy() && vertex_passes == 1);
        resident
            .character
            .validate(&program)
            .map_err(anyhow::Error::msg)?;
        report.push(json!({"source":row["source"],"name":row["name"],"object":id,"resident":resident,"model":model,
            "geometry":geometry,"mesh_pool":pool,"asset_state":asset_state,"requests":requests,"person_before":before_fields,"person_after":fields}));
    }
    std::fs::write(
        root.join("cache/original-initialized-saved-animation-tails.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}
