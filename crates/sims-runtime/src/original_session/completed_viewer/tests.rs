use super::*;
use std::{fs, path::Path};
fn input(path: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )?)?)
}
#[test]
#[ignore = "private frozen viewer/lot source evidence"]
fn retained_completed_viewer_preserves_active_extent_cache_and_listener_order() -> Result<()> {
    let source = input("fixtures/private/goth-viewer-owners-20261028-first/viewer-fourth.json")?;
    let lot = input("fixtures/private/goth-lot-owners-20261028-first/lot-fifth.json")?;
    let imported = import(&source, &lot)?;
    assert_eq!(imported.camera.world_size, Some(52));
    assert_eq!(imported.camera.settings.bounds, [0, 51, 0, 51]);
    assert_eq!(imported.registry.generation, 49);
    assert_eq!(imported.camera.settings.clock, 343485);
    assert_eq!(imported.view.floor.history.capacity, 3);
    assert_eq!(imported.view.floor.history.rooms, vec![0]);
    assert_eq!(imported.camera.settings.motion.rate_bits, 1701969920);
    assert_eq!(imported.camera.graph.state.corners.len(), 36);
    assert_eq!(imported.camera.graph.state.edges.len(), 39);
    assert_eq!(imported.camera.rooms.len(), 11);
    assert_eq!(imported.person_selection_override, 0);
    let encoded = serde_json::to_vec(&imported)?;
    let restored: Imported = serde_json::from_slice(&encoded)?;
    assert_eq!(
        postcard::to_allocvec(&imported)?,
        postcard::to_allocvec(&restored)?
    );
    // A retained active52 world cannot silently become an authored64 profile.
    let mut changed = restored.clone();
    changed.camera.world_size = None;
    assert!(changed.camera.validate(&changed.view).is_err());
    let reentry = input(
        "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
    )?;
    imported.runtime_owners(&reentry["source_closure"])?;
    let mut wrong = reentry["source_closure"].clone();
    wrong["source_frame_unwind"]["returns"][0] = json!("different");
    assert!(imported.runtime_owners(&wrong).is_err());
    Ok(())
}
#[test]
#[ignore = "private frozen viewer/lot source evidence"]
fn retained_viewer_rejects_scalar_alias_and_listener_mutation() -> Result<()> {
    let source = input("fixtures/private/goth-viewer-owners-20261028-first/viewer-fourth.json")?;
    let lot = input("fixtures/private/goth-lot-owners-20261028-first/lot-fifth.json")?;
    let mut wrong = source.clone();
    wrong["scalars"]["person_selection_override"] = json!("01000000");
    assert!(import(&wrong, &lot).is_err());
    let mut wrong = source.clone();
    wrong["commander"]["nodes"][0]["receiver"] = json!("Viewer");
    assert!(import(&wrong, &lot).is_err());
    let mut wrong = source;
    wrong["history"]["pointer"] = json!(0);
    assert!(import(&wrong, &lot).is_err());
    Ok(())
}
