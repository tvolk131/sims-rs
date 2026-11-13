//! Resume the computed level entry into its first original geometry child through the ordinary mode and tile-point preparation.
//! Original observations are comparison outputs, never callback replies.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);

fn cold_check(
    store: &sims_fixtures::Store,
    p: &State,
    scene: &WorldScene,
    view: &WorldComposition,
    vm: &sims_sim::Simulation,
    profiler: &Buffer,
    label: &str,
) -> Result<(State, serde_json::Value)> {
    let blob = store.put_value(&(p, scene, view, vm, profiler))?;
    let (mut cold, cs, cv, cvm, cp): Saved = store.get_value(&blob)?;
    let restored = cold.clone();
    let (mut warm, ws, wp) = (p.clone(), scene.clone(), profiler.clone());
    let mut n = 0;
    for _ in 0..16 {
        ensure!(
            warm.world_paint().unwrap().tile_row().is_none(),
            "unexpected row child in clipped continuation"
        );
        let a = warm.advance_world_geometry_child(vm)?;
        let b = cold.advance_world_geometry_child(&cvm)?;
        ensure!(
            a == b && warm == cold && ws == cs && wp == cp,
            "geometry setup cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *view == cv && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "geometry setup cold immutable owners differ"
    );
    Ok((
        restored,
        serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":n}),
    ))
}
fn main() -> Result<()> {
    let base = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-geometry-first-child-20261101-first");
    let old = base.join("fixtures/private/goth-level-geometry-20261101-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        j["cold_cuts"]
            .as_array()
            .context("prior cuts absent")?
            .last()
            .unwrap()["blob"]
            .clone(),
    )?;
    ensure!(
        input.sha256 == "3b1b82c2d6e7804eaaa6ea39a9fd70ecf03754513ef9b7b82a8f6043a2ca153d",
        "geometry input differs"
    );
    let path = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("cold-first.store"));
    let (mut p, scene, view, vm, profiler): Saved =
        sims_fixtures::Store::open(path)?.get_value(&input)?;
    let source = std::fs::read(out.join("mode-source-first.json"))?;
    p.bind_world_geometry_child(&source)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let native=std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).unwrap_or_else(||base.join("fixtures/private/goth-geometry-first-child-witness-20261101-first/world-classification-trace.jsonl.gz")))?;
    let pins: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SIMS_DIAG_PINS")
            .map(PathBuf::from)
            .unwrap_or_else(|| out.join("native-pins.json")),
    )?)?;
    ensure!(
        sims_fixtures::digest(&native) == pins["trace_sha256"],
        "geometry trace differs"
    );
    let mut rows = vec![];
    let mut mode_seen = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let r: serde_json::Value = serde_json::from_str(&line?)?;
        if r["kind"] != "world_drawing_setup" {
            continue;
        }
        let pc = r["pc"].as_u64().context("mode observation PC absent")? as u32;
        let selected = matches!(
            pc,
            0x53db67
                | 0x53bfb5
                | 0x53db9e
                | 0x53dba9
                | 0x466fcc
                | 0x53dbd2
                | 0x53dbf2
                | 0x53dc67
                | 0x53dc6a
                | 0x53dc78
                | 0x53dc7d
                | 0x53dcbf
                | 0x53dcc4
        ) || (pc == 0x4688bf && !mode_seen);
        if pc == 0x53dbf2 {
            mode_seen = true;
        }
        if selected {
            rows.push(r);
        }
        if pc == 0x53dcc4 {
            break;
        }
    }
    ensure!(
        rows.len() == 14 && rows[0]["pc"] == 0x53db67u32 && rows[13]["pc"] == 0x53dcc4u32,
        "geometry mode observation range differs"
    );
    let path = std::env::var_os("SIMS_DIAG_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let (mut checks, mut cuts, mut steps, mut current) = (vec![], vec![], 0, 0);
    let result = (|| -> Result<()> {
        for r in &rows {
            current = r["pc"].as_u64().context("geometry PC absent")? as u32;
            for _ in 0..64 {
                if p.world_paint().unwrap().at_tile_cut(r) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child(&vm)?,
                    "geometry setup stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(r)?;
            p.compare_world_shared_graphics(r)?;
            checks.push(serde_json::json!({"pc":current,"trace_stop":r["trace_stop"]}));
            let (restored, cut) = cold_check(
                &store,
                &p,
                &scene,
                &view,
                &vm,
                &profiler,
                &format!("{current:x}"),
            )?;
            p = restored;
            cuts.push(cut);
            eprintln!("GEOMETRY_MATCH {current:x}");
        }
        ensure!(
            matches!(
                p.world_paint().unwrap().pending(),
                Some(
                    sims_runtime::original_session::draw_parent::PaintRequest::LevelGeometryChild {
                        function: 0x53db67,
                        level: 1,
                        ..
                    }
                )
            ),
            "geometry setup lost actual child"
        );
        ensure!(
            unchanged
                == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
            "geometry setup changed enclosing VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"additional_B_source_sha256":sims_fixtures::digest(&source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"Computed first child setup and ordinary mode through current view zoom, current model room light and wrapping tile-point conversion. B supplies consumed model/room identities; current Rust owners supply service values. The x87 return is retained pending the original store; its resulting f32 local is compared at the point call. No full CPU/x87 agreement or child drawing/result acknowledgement. Remaining drawing/window/WorldView/Source/Session and32 completed ticks are pending."});
    let output = std::env::var_os("SIMS_DIAG_REPORT").context("geometry report output absent")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
