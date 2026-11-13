use anyhow::{Result, ensure};
use sims_content::{Index, Resource, saved_avatar::BoundAvatar};
use sims_sim::pose::appearance::{Binding, Execution, Input, Record};

fn replace(name: &str) -> Execution {
    Execution::bind(Input::Replace {
        name: name.into(),
        texture: String::new(),
    })
}

#[test]
fn incomplete_installation_and_bad_catalog_are_not_native_misses() -> Result<()> {
    let root = std::env::temp_dir().join(format!("sims-missing-appearance-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let mut index = Index {
        root: root.clone(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    };
    let mut avatar = BoundAvatar::empty(&index, "root", 17)?;
    avatar.records.entries[0] = Record {
        binding: Some(Binding { id: 99, owner: 17 }),
        name: "retained".into(),
        texture: "retained=texture".into(),
    };
    let before = avatar.snapshot();
    ensure!(avatar.apply_record(&index, 0, replace("missing")).is_err());
    ensure!(avatar.snapshot() == before);
    let path = root.join("bad.cmx");
    std::fs::write(&path, b"not a catalog")?;
    index.resources.push(Resource {
        locator: "bad.cmx".into(),
        name: "bad.cmx".into(),
        source: path,
        offset: 0,
        size: 13,
        unpacked_size: 13,
        archived: false,
        owned: None,
    });
    let error = avatar
        .apply_record(&index, 0, replace("missing"))
        .unwrap_err();
    ensure!(error.to_string().contains("read appearance catalog"));
    ensure!(avatar.snapshot() == before);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
#[ignore = "original Complete Collection catalogs and meshes"]
fn original_missing_stars_appearance_preserves_record_and_geometry() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let requested = "suit-chemset-heed-stars104";
    let actual = "suit-chemsuit-heed-stars104";
    ensure!(sims_content::avatar::find_appearance_catalog(&index, requested).is_err());
    let provider = sims_content::avatar::find_appearance_catalog(&index, actual)?;
    ensure!(
        index.resources[provider]
            .locator
            .ends_with("accessory-chemset-heed-stars104.cmx.bcf")
    );
    let mut avatar = BoundAvatar::empty(&index, "adult", 179)?;
    let id = avatar.records.generic;
    avatar.apply_record(&index, id, replace("suit-chemset-heed-stars103"))?;
    ensure!(avatar.records.entries[id].binding.is_some());
    let before = avatar.snapshot();
    let ranges = avatar.part_ranges.clone();
    let sources = avatar.avatar.sources.clone();
    let parts = avatar.avatar.parts.len();
    let dirty = avatar.apply_record(&index, id, replace(requested))?;
    ensure!(dirty.is_empty());
    ensure!(avatar.snapshot() == before);
    ensure!(avatar.part_ranges == ranges && avatar.avatar.parts.len() == parts);
    ensure!(avatar.avatar.sources == sources);
    let empty = avatar.records.entries.len();
    avatar.records.entries.push(Record::default());
    avatar.records.accessories.push(empty);
    let before_empty = avatar.snapshot();
    ensure!(
        avatar
            .apply_record(&index, empty, replace(requested))?
            .is_empty()
    );
    ensure!(avatar.snapshot() == before_empty);
    let restored = BoundAvatar::restore(&index, "adult", 179, &avatar.snapshot())?;
    ensure!(restored.snapshot() == avatar.snapshot());
    ensure!(restored.part_ranges == avatar.part_ranges);
    // A checkpoint promising actual geometry still requires that geometry.
    // The ordinary miss reply must not silently drop a saved binding.
    let mut invalid = avatar.snapshot();
    invalid.records.entries[id].name = requested.into();
    ensure!(BoundAvatar::restore(&index, "adult", 179, &invalid).is_err());
    std::fs::write(
        repo.join("cache/original-missing-appearance.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "requested":requested,"actual_registered_name":actual,"catalog":index.resources[provider].locator,
            "existing_binding_preserved":true,"empty_record_preserved":true,"no_geometry_dirty_callback":true,
        "restored_records_and_ranges_identical":true,"missing_saved_binding_rejected":true,
            "scope":"Missing name proven against complete source-backed startup registry; native record miss is distinct from an unresolved host/decode error."
        }))?,
    )?;
    Ok(())
}
