//! First interior rectangle/clip and third fast-row return, from a computed cut.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{Read, Write},
    path::PathBuf,
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
fn main() -> Result<()> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let old = base.join("fixtures/private/goth-terrain-interior-20261031-first");
    let root = base.join("fixtures/private/goth-terrain-clip-20261031-first");
    let previous: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(previous["cold_cuts"][7]["blob"].clone())?;
    ensure!(
        input.sha256 == "a7aaf5de1e2f91791a30b5b9660de8abf58f3c339a34e688d563c07b8aee5eed",
        "interior input differs"
    );
    let old_store = sims_fixtures::Store::open(old.join("preparation-cold-first.store"))?;
    let (mut parent, scene, view, vm, profiler): Saved = old_store.get_value(&input)?;
    parent.bind_world_row_contents(&std::fs::read(old.join("terrain-source-first.json"))?)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = root.join("clip-cold-first.store");
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let native=std::fs::read(base.join("fixtures/private/goth-interior-terrain-witness-20261031-first/world-classification-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&native)
            == "842c6c2dcf392b77dc49721f608cad9f962fab30929b246478ef85ee6293e5e1",
        "native trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let start = rows
        .iter()
        .position(|r| {
            r["kind"] == "window_tile_scan" && r["pc"] == 0x4c4388u32 && r["trace_stop"] == 310
        })
        .context("interior clip start absent")?;
    let mut cuts = vec![];
    let mut compared = 0;
    let mut returned = false;
    for row in rows[start..]
        .iter()
        .filter(|r| r["kind"] == "window_tile_scan")
    {
        let pc = row["pc"].as_u64().context("native PC absent")? as u32;
        if pc == 0x4ad680 {
            parent.return_world_row()?;
        }
        for _ in 0..64 {
            if parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .at_native_observation(row)?
            {
                break;
            }
            ensure!(
                parent.advance_world_row_prefix(&vm)?,
                "third row stalled at{pc:x}"
            );
        }
        parent.compare_world_row_prefix(row)?;
        if pc == 0x4ad680 {
            parent.compare_world_tiles(row)?;
            returned = true;
        }
        compared += 1;
        if matches!(
            pc,
            0x4c4388 | 0x49d24c | 0x4c4622 | 0x4b000b | 0x4b0111 | 0x4b0138 | 0x4b0155 | 0x4ad680
        ) {
            let blob = store.put_value(&(&parent, &scene, &view, &vm, &profiler))?;
            let (mut cold, cold_scene, cold_view, cold_vm, cold_profiler): Saved =
                store.get_value(&blob)?;
            let mut warm = parent.clone();
            ensure!(
                warm.advance_world_row_prefix(&vm)? == cold.advance_world_row_prefix(&cold_vm)?
                    && warm == cold
                    && scene == cold_scene
                    && view == cold_view
                    && profiler == cold_profiler
                    && serde_json::to_vec(&vm)? == serde_json::to_vec(&cold_vm)?,
                "third row cold owner differs"
            );
            cuts.push(serde_json::json!({"index":compared-1,"pc":pc,"blob":blob}));
        }
        if returned {
            break;
        }
    }
    ensure!(
        returned
            && parent
                .world_paint()
                .unwrap()
                .row_prefix()
                .unwrap()
                .acknowledged(),
        "third row did not return"
    );
    ensure!(
        unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
        "row changed VM/RNG/room/profiler"
    );
    let row = serde_json::to_value(parent.world_paint().unwrap().row_prefix().unwrap())?;
    let report = serde_json::json!({"schema":1,"input":input,"native_clip_and_row_boundaries":compared,"cold_cuts":cuts,"next":parent.world_paint().unwrap().row_prefix().unwrap().pending(),"local_end":row["tail"],"request":row["request"],"acknowledged":true,"persistent_owners_retained":true,"retained_vm_and_profiler_unchanged":true,"native_parity_credit":0,"source_admission":false,"expected_state_injected":false,"qualification":"Computed first interior rectangle, clipped return, dry-water/absent-overlay gates and third row return. Overlapping raster work, later rows and full Draw/Source/Session remain pending; no strict-tick credit."});
    if let Some(path) = std::env::var_os("SIMS_TERRAIN_CLIP_REPORT") {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
