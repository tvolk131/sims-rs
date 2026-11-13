//! Resume the computed level entry into its first original geometry child.
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
    let (mut cold, mut cs, cv, cvm, mut cp): Saved = store.get_value(&blob)?;
    let restored = cold.clone();
    let (mut warm, mut ws, mut wp) = (p.clone(), scene.clone(), profiler.clone());
    let mut n = 0;
    for _ in 0..16 {
        ensure!(
            warm.world_paint().unwrap().tile_row().is_none(),
            "unexpected row child in clipped continuation"
        );
        let a = warm.advance_world_paint(&mut ws, &mut wp)?;
        let b = cold.advance_world_paint(&mut cs, &mut cp)?;
        ensure!(
            a == b && warm == cold && ws == cs && wp == cp,
            "geometry iterator cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *view == cv && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "geometry iterator cold immutable owners differ"
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
    let out = base.join("fixtures/private/goth-level-geometry-20261101-first");
    let old = base.join("fixtures/private/goth-base-scan-20261101-first");
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
        input.sha256 == "a2011dd2646dbfd4e0b8154a39ba0731043761750e94f46f3a62baee38fa72b7",
        "geometry input differs"
    );
    let path = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("cold-first.store"));
    let (mut p, mut scene, view, vm, mut profiler): Saved =
        sims_fixtures::Store::open(path)?.get_value(&input)?;
    let source = std::fs::read(out.join("geometry-source-first.json"))?;
    p.bind_world_geometry_body(&source)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let native=std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).unwrap_or_else(||base.join("fixtures/private/goth-level-geometry-witness-20261101-first/world-classification-trace.jsonl.gz")))?;
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
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let r: serde_json::Value = serde_json::from_str(&line?)?;
        if r["kind"] == "world_drawing_setup"
            && r["trace_stop"].as_u64().unwrap_or(0) >= pins["first_trace_stop"].as_u64().unwrap()
            && r["trace_stop"].as_u64().unwrap_or(0) <= pins["last_trace_stop"].as_u64().unwrap()
        {
            rows.push(r);
        }
    }
    ensure!(
        rows.len() == 2 && rows[0]["pc"] == 0x54045bu32 && rows[1]["pc"] == 0x53db67u32,
        "geometry observation range differs"
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
                    p.advance_world_paint(&mut scene, &mut profiler)?,
                    "geometry iterator stalled at{current:x}"
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
            "geometry iterator lost actual child"
        );
        ensure!(
            unchanged
                == sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?),
            "geometry iterator changed enclosing VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"additional_B_source_sha256":sims_fixtures::digest(&source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"Computed first level geometry entry through first original child call. Iterator slot/header/element ownership, original arguments and shared context compared. No child drawing or reply supplied. Changed second-vector model records are not imported. Geometry child body, subsequent window/WorldView/Source/Session returns and32 completed Goth ticks remain pending."});
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
