//! Original marker resource/binding integration, before factory/placement and
//! ordinary Animator integration. This is not an activated household marker.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    graphic_materials::{Backend, GraphicMaterials},
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_formats::{iff::Iff, strings::StringTable};
use sims_sim::pose::{
    appearance::{Execution, Input},
    appearance_selection::descriptor,
    graphic_appearance::{Model, Reload, Reply, Request, State},
    model::{Bound, Matrix},
};

#[test]
#[ignore = "requires private original indicator assets"]
fn original_indicator_profiles_bind_replace_and_restore_real_meshes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for (name, kind) in [("PlumbBob.iff", 1), ("DateBob.iff", 2), ("StarBob.iff", 3)] {
        let resource = &index.resources[index.find(name)?];
        let program = sims_content::vm::load_program(&index, name, "Global.iff")?;
        let module = &program.modules[&resource.locator];
        let words = module
            .object_definition_words
            .values()
            .next()
            .context("definition absent")?;
        ensure!(
            words[9] == 4 && words[102] & 1 != 0,
            "ordinary 3D graphic definition required"
        );
        let iff = Iff::parse(resource.read()?)?;
        let profile = StringTable::parse(
            iff.data(
                iff.get(b"STR#", words[19] as u16)
                    .context("profile table absent")?,
            ),
        )?;
        let skeleton = profile.get(0, 1).context("skeleton absent")?;
        let text = profile.get(1, 1).context("generic descriptor absent")?;
        let mut avatar = BoundAvatar::empty(&index, skeleton, kind)?;
        let mut crt = 1;
        ensure!(skeleton.eq_ignore_ascii_case("root"));
        let mut bound = Bound::root(&mut crt).map_err(anyhow::Error::msg)?;
        let mut geometry = CharacterGeometry::new(avatar.avatar.skeleton.bones.len());
        let mut pool = MeshPool::default();
        let mut state = State::constructed();
        let mut previous = None;
        let mut service_count = 0;
        for _ in 0..2 {
            let mut reload = Reload::new(true);
            loop {
                let pending = reload
                    .advance(
                        &mut state,
                        0,
                        Some(Model {
                            model: kind,
                            skin: kind,
                        }),
                    )
                    .map_err(anyhow::Error::msg)?;
                // Full ownership is restored before each exposed service. Asset
                // rehydration must not rebind or change cache generations.
                let saved = (
                    reload.clone(),
                    state.clone(),
                    bound.clone(),
                    geometry.clone(),
                    pool.clone(),
                    avatar.snapshot(),
                );
                let restored: (
                    Reload,
                    State,
                    Bound,
                    CharacterGeometry,
                    MeshPool,
                    BindingState,
                ) = serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
                ensure!(saved == restored);
                let bindings = restored.5;
                (reload, state, bound, geometry, pool) =
                    (restored.0, restored.1, restored.2, restored.3, restored.4);
                avatar = BoundAvatar::restore(&index, skeleton, kind, &bindings)?;
                ensure!(avatar.snapshot() == bindings);
                let Some(request) = pending else {
                    break;
                };
                let reply = match &request {
                    Request::Descriptor => Reply::Descriptor(descriptor(text, false)),
                    Request::Bind { skin, descriptor } => {
                        ensure!(*skin == kind);
                        let record = avatar.records.generic;
                        geometry.apply_record(
                            &index,
                            &mut avatar,
                            record,
                            Execution::bind(Input::Replace {
                                name: descriptor.name.clone(),
                                texture: descriptor.texture.clone(),
                            }),
                            &mut bound,
                            &mut pool,
                        )?;
                        Reply::Done
                    }
                    Request::Tint { model, tint } => {
                        ensure!(*model == kind && *tint == state.tint);
                        // Explicit CPU backend inputs; original built-in colors
                        // and actual 5a1a8f control/publication semantics.
                        let mut materials = GraphicMaterials::from_builtin_geometry(
                            &geometry,
                            &avatar,
                            [0; 3],
                            Backend {
                                mode: 0,
                                flags: 0,
                                references: 1,
                                ramp_size: 32,
                            },
                        )?;
                        materials.apply(*tint, 0)?;
                        ensure!(materials.state.controls == *tint);
                        ensure!(!materials.published.is_empty());
                        Reply::Done
                    }
                };
                reload
                    .acknowledge(&request, reply)
                    .map_err(anyhow::Error::msg)?;
                service_count += 1;
            }
            ensure!(
                reload.complete()
                    && state.marker
                    && state.kind == kind as u32
                    && !state.ordinary_material
            );
            let binding = avatar.records.entries[avatar.records.generic]
                .binding
                .context("generic appearance absent")?;
            ensure!(
                Some(binding.id) != previous
                    && avatar.part_ranges.len() == 1
                    && geometry.appearances.len() == 1
            );
            if let Some(previous) = previous {
                ensure!(!avatar.part_ranges.contains_key(&previous));
            }
            previous = Some(binding.id);
        }
        // An explicit neutral pose for inspection, not a claim to execute the
        // still-unintegrated base Animator refresh/ordinary update routines.
        bound
            .publish(Matrix::IDENTITY)
            .map_err(anyhow::Error::msg)?;
        bound
            .skeleton
            .hierarchy
            .update(&mut bound.skeleton.skin, false)
            .map_err(anyhow::Error::msg)?;
        geometry
            .state
            .refresh(&mut bound, &mut pool.meshes, false)
            .map_err(anyhow::Error::msg)?;
        let notifications = geometry.update_loaded_vertices(&avatar, &bound, false)?;
        ensure!(!notifications.is_empty() && !avatar.avatar.parts.is_empty());
        let actual = geometry
            .vertices
            .values()
            .map(|v| v.vertices.len())
            .sum::<usize>();
        ensure!(
            actual > 0
                && bound
                    .local_bounds
                    .minimum
                    .iter()
                    .zip(bound.local_bounds.maximum)
                    .all(|(a, b)| *a <= b)
        );
        report.push(json!({"source":resource.locator,"skeleton":skeleton,"descriptor":descriptor(text,false),
            "state":state,"bound":bound,"geometry":geometry,"mesh_pool":pool,"asset_state":avatar.snapshot(),
            "services":service_count,"vertices":actual,"notifications":notifications,"sources":avatar.avatar.sources}));
    }
    std::fs::write(
        root.join("cache/original-indicator-bindings.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}
