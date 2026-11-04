//! Original assets exercise the host's masked attachments. The independent
//! native-censor fixture establishes selection/lifecycle semantics; this test
//! covers asset decoding, shared prototype identity and snapshot reconstruction.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_sim::pose::initialize::Initialization;

#[test]
#[ignore = "requires private original assets and populated initialization report"]
fn masked_original_appearances_keep_identity_order_and_mesh_prototypes() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let rows: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-character-populated-initialization.json"),
    )?)?;
    let row = rows
        .iter()
        .find(|r| r["source"] == "UserData2/Houses/House09.iff" && r["object"] == 195)
        .context("initialized Mashuga adult absent")?;
    let mut initialized: Initialization = serde_json::from_value(row["initialization"].clone())?;
    let mut geometry: CharacterGeometry = serde_json::from_value(row["geometry"].clone())?;
    let mut pool: MeshPool = serde_json::from_value(row["mesh_pool"].clone())?;
    let state: BindingState = serde_json::from_value(row["asset_state"].clone())?;
    let mut avatar = BoundAvatar::restore(&index, &initialized.source.name, 195, &state)?;
    let original_parts = avatar.avatar.parts.len();
    let original_appearances = geometry.appearances.len();
    let original_prototypes = pool.meshes.len();
    let mut prototypes = std::collections::BTreeMap::new();

    // Reordered single-bit selections must reuse the descriptor's original
    // ordinal, not slot zero in each filtered output. Include an empty live
    // instance between populated ones and a full seven-part appearance.
    for (mask, expected_bones) in [
        (8, vec!["L_HAND"]),
        (16, vec!["R_HAND"]),
        (256, vec![]),
        (8, vec!["L_HAND"]),
        (
            127,
            vec![
                "PELVIS", "SPINE2", "R_HAND", "L_HAND", "HEAD", "R_FOOT", "L_FOOT",
            ],
        ),
    ] {
        let id = geometry.bind_auxiliary(
            &index,
            &mut avatar,
            "adult-needs",
            mask,
            &mut initialized.bound,
            &mut pool,
        )?;
        let range = avatar.part_ranges[&id].clone();
        ensure!(range == (original_parts..original_parts + expected_bones.len()));
        let parts = &avatar.avatar.parts[range];
        ensure!(parts.iter().all(|p| p.censor));
        ensure!(
            parts
                .iter()
                .map(|p| p.mesh.bones[0].as_str())
                .collect::<Vec<_>>()
                == expected_bones
        );
        let selected = &geometry.state.appearances[geometry.appearances[&id]];
        ensure!(selected.len() == expected_bones.len());
        for (&binding, bone) in selected.iter().zip(expected_bones) {
            let mesh = geometry.state.bindings[binding].mesh;
            if let Some(&previous) = prototypes.get(bone) {
                ensure!(
                    mesh == previous,
                    "masked selection changed prototype identity"
                );
            } else {
                ensure!(
                    !prototypes.values().any(|&other| other == mesh),
                    "different masked descriptors aliased one prototype"
                );
                prototypes.insert(bone, mesh);
            }
        }

        let saved = avatar.snapshot();
        let encoded = serde_json::to_vec(&(&saved, &geometry, &initialized, &pool))?;
        let (saved, restored_geometry, restored_initialization, restored_pool): (
            BindingState,
            CharacterGeometry,
            Initialization,
            MeshPool,
        ) = serde_json::from_slice(&encoded)?;
        let restored = BoundAvatar::restore(&index, &initialized.source.name, 195, &saved)?;
        ensure!(restored.snapshot() == avatar.snapshot());
        ensure!(restored.part_ranges == avatar.part_ranges);
        ensure!(restored.avatar.parts.len() == avatar.avatar.parts.len());
        ensure!(
            restored_geometry == geometry
                && restored_initialization == initialized
                && restored_pool == pool
        );
        avatar = restored;
        geometry = restored_geometry;
        initialized = restored_initialization;
        pool = restored_pool;

        geometry.detach_auxiliary(&index, &mut avatar, id, &mut initialized.bound, &mut pool)?;
        ensure!(avatar.avatar.parts.len() == original_parts);
        ensure!(geometry.appearances.len() == original_appearances);
        ensure!(avatar.auxiliary().is_empty());
    }
    ensure!(pool.meshes.len() == original_prototypes + 7);

    // Empty instances have identical geometry offsets but distinct order and
    // lifetime. Rebuilding or deleting one must not reorder its neighbours.
    let mut ids = vec![];
    for mask in [256, 512, 1] {
        ids.push(geometry.bind_auxiliary(
            &index,
            &mut avatar,
            "adult-needs",
            mask,
            &mut initialized.bound,
            &mut pool,
        )?);
    }
    geometry.detach_auxiliary(
        &index,
        &mut avatar,
        ids[1],
        &mut initialized.bound,
        &mut pool,
    )?;
    let saved = avatar.snapshot();
    ensure!(saved.order.ends_with(&[ids[0], ids[2]]));
    let restored = BoundAvatar::restore(&index, &initialized.source.name, 195, &saved)?;
    ensure!(restored.snapshot() == saved && restored.part_ranges == avatar.part_ranges);
    ensure!(avatar.part_ranges[&ids[0]].is_empty());
    ensure!(avatar.part_ranges[&ids[2]].len() == 1);
    Ok(())
}

#[test]
#[ignore = "requires private original assets and the v61 censor boundary"]
fn original_pending_censor_bind_retries_atomically_and_restores_assets() -> Result<()> {
    use sims_content::{character_censor, character_loading::live::SavedAssets};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let checkpoint: Value = serde_json::from_slice(&std::fs::read(root.join(
        "cache/neighborhood-input-experiments/mashuga-empty-extended-v61/original-live-character-final.json",
    ))?)?;
    // Extract one pending service as a component fixture. This intentionally
    // does not migrate or restore the old full-program fingerprint.
    let mut resident: sims_sim::character::Resident =
        serde_json::from_value(checkpoint["simulation"]["state"]["characters"]["196"].clone())?;
    let SavedAssets {
        skeleton,
        mut model,
        mut geometry,
        binding,
        mut appearance_owner,
    } = serde_json::from_value(checkpoint["assets"]["characters"]["196"].clone())?;
    let mut pool: MeshPool = serde_json::from_value(checkpoint["assets"]["mesh_pool"].clone())?;
    let mut avatar = BoundAvatar::restore(&index, &skeleton, 196, &binding)?;
    ensure!(
        resident.character.pending_request()
            == Some(&sims_sim::pose::refresh::Request::BindCensor { mask: 127 })
    );
    let before = serde_json::to_vec(&(
        &resident,
        &model,
        &geometry,
        avatar.snapshot(),
        &appearance_owner,
        &pool,
    ))?;
    let mut missing = index.clone();
    missing.resources.retain(|r| !r.name.contains("PELVISBOX"));
    let error = character_censor::service(
        &missing,
        &mut resident,
        &mut model,
        &mut geometry,
        &mut avatar,
        &mut appearance_owner,
        &mut pool,
    )
    .unwrap_err();
    ensure!(
        error.to_string().contains("PELVISBOX"),
        "unexpected failure: {error:#}"
    );
    ensure!(
        before
            == serde_json::to_vec(&(
                &resident,
                &model,
                &geometry,
                avatar.snapshot(),
                &appearance_owner,
                &pool
            ))?,
        "failed bind changed the pending service or an owner"
    );
    ensure!(character_censor::service(
        &index,
        &mut resident,
        &mut model,
        &mut geometry,
        &mut avatar,
        &mut appearance_owner,
        &mut pool
    )?);
    character_censor::validate(&resident, &appearance_owner, &avatar, &geometry)?;
    let id = resident
        .character
        .censor
        .binding
        .context("censor binding absent")?;
    ensure!(avatar.part_ranges[&id].len() == 7 && appearance_owner.attachment == Some(id));
    let saved: BindingState = serde_json::from_slice(&serde_json::to_vec(&avatar.snapshot())?)?;
    let restored = BoundAvatar::restore(&index, &skeleton, 196, &saved)?;
    character_censor::validate(&resident, &appearance_owner, &restored, &geometry)?;
    ensure!(restored.snapshot() == avatar.snapshot() && restored.part_ranges == avatar.part_ranges);
    let mut damaged = appearance_owner.clone();
    damaged.attachment = None;
    ensure!(character_censor::validate(&resident, &damaged, &restored, &geometry).is_err());
    Ok(())
}
