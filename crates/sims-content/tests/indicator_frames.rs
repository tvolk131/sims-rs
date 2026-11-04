//! Fresh ordinary-object initialization followed by source marker mesh frames.
//! Definition profile lookup, later attachment, viewport/culling and owner
//! invalidation are explicit inputs; live selection is separate.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use sims_content::{
    Index,
    character_geometry::MeshPool,
    graphic_initialization::{Environment, Prepared},
    graphic_profiles::Cached,
    saved_avatar::BoundAvatar,
};
use sims_formats::{behavior::ObjectDefinition, iff::Iff, strings::StringTable};
use sims_sim::pose::{
    animator::Settings,
    appearance::{Execution, Input},
    appearance_selection::descriptor,
    graphic_appearance::Model,
    graphic_frame as frame, graphic_transform as placement,
    model::Matrix,
    model_frame, transform,
};

fn restore<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    value: &mut T,
) -> Result<()> {
    let restored = serde_json::from_slice(&serde_json::to_vec(value)?)?;
    ensure!(*value == restored, "restored graphic state differs");
    *value = restored;
    Ok(())
}
#[test]
#[ignore = "requires private original indicator assets"]
fn original_markers_initialize_then_run_base_frames_and_restore_at_service_boundaries() -> Result<()>
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut rows = vec![];
    for (name, kind) in [("PlumbBob.iff", 1), ("DateBob.iff", 2), ("StarBob.iff", 3)] {
        let program = sims_content::vm::load_program(&index, name, "Global.iff")?;
        let resource = &index.resources[index.find(name)?];
        let module = &program.modules[&resource.locator];
        let (&object, words) = module
            .object_definition_words
            .iter()
            .next()
            .context("marker definition absent")?;
        ensure!(words[9] == 4 && words[102] & 1 != 0);
        let iff = Iff::parse(resource.read()?)?;
        let original = ObjectDefinition::parse(
            iff.data(iff.get(b"OBJD", object).context("marker OBJD absent")?),
        )?;
        let strings = StringTable::parse(
            iff.data(
                iff.get(b"STR#", words[19] as u16)
                    .context("marker profile absent")?,
            ),
        )?;
        let skeleton = strings.get(0, 1).context("marker skeleton absent")?;
        ensure!(skeleton.eq_ignore_ascii_case("root"));
        let desc = descriptor(
            strings.get(1, 1).context("marker appearance absent")?,
            false,
        );
        let mut profile = Cached::new(
            sims_vm::Definition {
                source: resource.locator.clone(),
                object,
                guid: original.guid,
                slots: original.slots,
            },
            words,
        )?;
        let mut profile_services = 0;
        while !profile.complete() {
            profile = Cached::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&profile)?)?,
            )?;
            profile.advance(&index, &mut |_, _, _| {
                anyhow::bail!("original marker unexpectedly requests profile writeback")
            })?;
            profile_services += 1;
            ensure!(profile_services <= 3);
        }
        ensure!(
            profile
                .strings()?
                .context("original marker profile absent")?
                .first()
                .map(String::as_str)
                == Some(skeleton)
        );
        let warm = profile.clone();
        ensure!(profile.advance(&index, &mut |_, _, _| anyhow::bail!(
            "cached profile requested a write"
        ))?);
        ensure!(profile == warm);
        let mut initial_results = vec![];
        for resume in [false, true] {
            let mut prepared = Prepared::begin_cached(
                kind as i16,
                &profile,
                words,
                0,
                Settings {
                    fade_enabled: true,
                    fade_duration: 200,
                    sampling_config: 0,
                },
            )?;
            let mut crt = 1;
            let mut pool = MeshPool::default();
            let mut services = 0;
            while !prepared.complete() {
                if resume {
                    let saved = prepared.snapshot();
                    prepared = Prepared::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )?;
                    ensure!(prepared.snapshot() == saved);
                }
                prepared.advance(
                    &index,
                    &mut crt,
                    &mut pool,
                    Environment {
                        owner_appearance_index: 0,
                        source: placement::Source {
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
                    },
                )?;
                services += 1;
                ensure!(services <= 20);
            }
            initial_results.push((prepared.snapshot(), crt, services));
        }
        ensure!(
            initial_results[0] == initial_results[1],
            "source initialization differs after resume"
        );
        let initialization = serde_json::to_value(&initial_results[1])?;
        let ready = Prepared::restore(&index, initial_results.remove(1).0)?.into_ready()?;
        ensure!(
            ready.avatar.avatar.parts.is_empty()
                && ready
                    .avatar
                    .records
                    .entries
                    .iter()
                    .all(|r| r.binding.is_none()),
            "fresh Init bound the marker prematurely"
        );
        ensure!(!ready.appearance.marker && ready.appearance.cached == -1);
        let mut avatar = ready.avatar;
        let mut bound = ready.bound;
        let mut geometry = ready.geometry;
        let mut pool = MeshPool::default();
        let record = avatar.records.generic;
        geometry.apply_record(
            &index,
            &mut avatar,
            record,
            Execution::bind(Input::Replace {
                name: desc.name,
                texture: desc.texture,
            }),
            &mut bound,
            &mut pool,
        )?;
        let mut matrix = Matrix::IDENTITY;
        matrix.values[12..15].copy_from_slice(&[60., 3., -40.]);
        matrix.kind = 2;
        bound.publish(matrix).map_err(anyhow::Error::msg)?;
        let mut state = frame::State {
            clock_mode: 0,
            now: 0,
            visible: 1,
            blend: None,
            placement: placement::Placement {
                base: [-999.; 3],
                offset: [0.; 3],
                dirty: true,
                explicit: true,
            },
            matrix,
        };
        let mut steps = vec![];
        let mut services = 0;
        for (tick, entry, only, visible) in [
            (0, frame::Entry::Refresh, true, true),
            (0, frame::Entry::Ordinary, true, true),
            (1, frame::Entry::Ordinary, true, false),
            (2, frame::Entry::Refresh, true, false),
            (3, frame::Entry::Refresh, false, false),
            (4, frame::Entry::Ordinary, false, true),
        ] {
            let mut update = frame::Update::new(entry, only);
            loop {
                restore(&mut update)?;
                restore(&mut state)?;
                restore(&mut bound)?;
                restore(&mut geometry)?;
                restore(&mut pool)?;
                let bindings = avatar.snapshot();
                avatar = BoundAvatar::restore(&index, skeleton, kind, &bindings)?;
                ensure!(avatar.snapshot() == bindings);
                let Some(request) = update
                    .advance(
                        &mut state,
                        tick,
                        Some(Model {
                            model: kind,
                            skin: kind,
                        }),
                    )
                    .map_err(anyhow::Error::msg)?
                else {
                    break;
                };
                let reply = match &request {
                    frame::Request::OutsideLot => frame::Reply::Outside(false),
                    frame::Request::Viewport => frame::Reply::Viewport([-10, -20, 640, 480]),
                    frame::Request::Visibility { .. } => {
                        state.visible = u8::from(visible);
                        frame::Reply::Visible(visible)
                    }
                    frame::Request::Invalidate => frame::Reply::Done,
                    frame::Request::Placement => {
                        let mut pass = placement::Update::default();
                        let request = pass
                            .advance(
                                &mut state.placement,
                                state.matrix,
                                placement::Source {
                                    position: [160, 320],
                                    facing: 0,
                                    level: 1,
                                },
                                transform::View {
                                    rotation: 0,
                                    floor_vector: [0.; 3],
                                },
                                transform::Terrain {
                                    size: 2,
                                    corners: &[[0; 4]; 4],
                                },
                            )
                            .map_err(anyhow::Error::msg)?
                            .context("placement service absent")?;
                        ensure!(
                            matches!(request, placement::Request::Position(_)),
                            "dependent marker unexpectedly published a matrix"
                        );
                        pass.acknowledge(&mut state.placement, &request)
                            .map_err(anyhow::Error::msg)?;
                        frame::Reply::Done
                    }
                    frame::Request::Model {
                        model,
                        matrix,
                        now,
                        visible,
                    } => {
                        ensure!(*model == kind);
                        let mut pass = model_frame::Update::new(*matrix, *now, *visible, true);
                        loop {
                            restore(&mut pass)?;
                            restore(&mut bound)?;
                            let Some(request) = pass
                                .next(&mut bound, &program)
                                .map_err(anyhow::Error::msg)?
                            else {
                                break;
                            };
                            ensure!(
                                request == model_frame::Request::Geometry,
                                "unexpected marker animation callback"
                            );
                            geometry
                                .state
                                .refresh(&mut bound, &mut pool.meshes, false)
                                .map_err(anyhow::Error::msg)?;
                            restore(&mut pass)?;
                            restore(&mut bound)?;
                            restore(&mut geometry)?;
                            restore(&mut pool)?;
                            pass.acknowledge(&mut bound, &request)
                                .map_err(anyhow::Error::msg)?;
                        }
                        ensure!(pass.complete());
                        frame::Reply::Done
                    }
                    frame::Request::Skin { skin, now, sample } => {
                        ensure!(*skin == kind);
                        let mut pass = model_frame::Skin::new(*now, *sample != 0, true);
                        ensure!(
                            pass.next(&mut bound, &program)
                                .map_err(anyhow::Error::msg)?
                                .is_none()
                                && pass.complete()
                        );
                        frame::Reply::Done
                    }
                    frame::Request::Hierarchy { skin } => {
                        ensure!(*skin == kind);
                        bound
                            .skeleton
                            .hierarchy
                            .update(&mut bound.skeleton.skin, false)
                            .map_err(anyhow::Error::msg)?;
                        frame::Reply::Done
                    }
                    frame::Request::Geometry { model } => {
                        ensure!(*model == kind);
                        geometry
                            .state
                            .refresh(&mut bound, &mut pool.meshes, false)
                            .map_err(anyhow::Error::msg)?;
                        frame::Reply::Done
                    }
                    other => anyhow::bail!("unexpected marker service {other:?}"),
                };
                restore(&mut update)?;
                restore(&mut state)?;
                restore(&mut bound)?;
                restore(&mut geometry)?;
                restore(&mut pool)?;
                update
                    .acknowledge(&mut state, &request, reply)
                    .map_err(anyhow::Error::msg)?;
                services += 1;
            }
            ensure!(update.complete());
            steps.push(json!({"tick":tick,"entry":entry,"only_visible":only,"state":state,"model":bound.model,"animator":bound.skeleton.skin.animator}));
        }
        // Vertex publication is an explicit renderer preparation, following
        // the frame's actual pose/geometry. It is not part of 585a46 itself.
        let uploads = geometry.update_loaded_vertices(&avatar, &bound, false)?;
        ensure!(!uploads.is_empty() && !geometry.vertices.is_empty());
        rows.push(json!({"name":name,"skeleton":skeleton,"state":{"kind":kind},"profile_cache":profile,"profile_services":profile_services,"initialization":initialization,"frame":state,"bound":bound,
            "geometry":geometry,"asset_state":avatar.snapshot(),"pool":pool,"steps":steps,"services":services,
            "scope":"fresh ordinary-object Init with explicit definition profile, actual root binding/Reset/accessories/placement and dirty visibility; then explicit mesh attachment and base frames; supplied frame viewport/culling/invalidation; neutral material; no live selection"}));
    }
    std::fs::write(
        root.join("cache/original-indicator-initialized-frames.json"),
        serde_json::to_vec(&rows)?,
    )?;
    Ok(())
}
