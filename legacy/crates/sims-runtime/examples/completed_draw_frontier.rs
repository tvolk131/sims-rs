//! Inspect a retained drawing frontier without advancing any owner.
use anyhow::{Context, Result};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    sims_vm::interface::state::State,
    WorldClassification,
);
fn main() -> Result<()> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (store, r) = if let (Some(store), Some(r)) = (
        std::env::var_os("SIMS_DIAG_INPUT_STORE"),
        std::env::var_os("SIMS_DIAG_INPUT_REF"),
    ) {
        (
            std::path::PathBuf::from(store),
            serde_json::from_str::<sims_fixtures::BlobRef>(&r.to_string_lossy())?,
        )
    } else {
        let folder = root.join("fixtures/private/goth-spr2-mmx-20261101-first");
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("full-scan-batch.json"))?)?;
        (
            folder.join("full-scan-batch.store"),
            serde_json::from_value(report["frontier"].clone())?,
        )
    };
    let state: Saved = sims_fixtures::Store::open(store)?.get_value(&r)?;
    let row = serde_json::to_value(state.0.world_paint().and_then(|w| w.row_prefix()))?;
    let c = &row["ordinary"];
    let id = std::env::var("SIMS_DIAG_OBJECT_ID")
        .ok()
        .map(|s| s.parse::<i16>())
        .transpose()?
        .or_else(|| {
            c["selected_id"]
                .as_i64()
                .or_else(|| c["head"].as_i64())
                .map(|n| n as i16)
        })
        .unwrap_or(223);
    let obj = state
        .3
        .vm
        .world
        .objects
        .get(&id)
        .context("frontier object absent")?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"schema":1,"input":r,"object":id,"slots":obj.slots,"definition":obj.definition,"draw_state":state.3.vm.world.refresh.as_ref().and_then(|v|v.draw.get(&id)),"stage":c["stage"],"selected":c["selected"],"selected_id":c["selected_id"],"tile_head":c["head"],"rotated":c["rotated"],"draw_source_id":c["draw_source"]["id"],"draw_source_definition":c["draw_source"]["definition"],"draw_bounds":c["draw_source"]["bounds"],"draw_stage":c["draw_cursor"]["stage"],"sprite":c["draw_cursor"]["sprite"],"context":state.0.world_context().map(|c|c.words()),"frame_owner":c["draw_source"]["frames"],"scope":"Read-only Rust-derived frontier, no original reply or parity credit","source_admission":false,"native_parity_credit":0})
        )?
    );
    Ok(())
}
