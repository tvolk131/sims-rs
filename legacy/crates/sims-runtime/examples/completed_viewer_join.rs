//! Retained viewer/cache import without camera construction or F6 execution.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::completed_viewer;
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 3,
        "usage: completed_viewer_join VIEWER_JSON LOT_JSON NEW_DIR"
    );
    let source: Value = serde_json::from_slice(&fs::read(&a[0])?)?;
    let lot: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let imported = completed_viewer::import(&source, &lot)?;
    let out = Path::new(&a[2]);
    fs::create_dir(out)?;
    let store = Store::create(out.join("store"))?;
    let blob = store.put_value(&imported)?;
    let cold: completed_viewer::Imported = store.get_value(&blob)?;
    ensure!(
        postcard::to_allocvec(&imported)? == postcard::to_allocvec(&cold)?,
        "cold viewer owners differ"
    );
    let report = json!({"schema":1,"tick":88448,"native_parity_ticks":0,"executed_ticks":0,"complete_native_Session_admitted":false,"current_world_size":imported.camera.world_size,"world_bounds":imported.camera.settings.bounds,"clock":imported.camera.settings.clock,"history":imported.view.floor.history,"level":imported.view.floor.level,"cutaway":imported.view.floor.visibility.enabled,"wall_corners":imported.camera.graph.state.corners.len(),"wall_edges":imported.camera.graph.state.edges.len(),"room_masks":imported.camera.rooms.len(),"commander_generation":imported.registry.generation,"source":imported.source,"cold_restore_identical":true});
    fs::write(
        out.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"viewer":blob}))?,
    )?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
