//! Resume the computed Goth scan through its natural return and level callback.
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
            "base scan cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *view == cv && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "base scan cold immutable owners differ"
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
    let out = base.join("fixtures/private/goth-base-scan-20261101-first");
    let old = base.join("fixtures/private/goth-floor-rows64-20261101-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-second.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        j["cold_cuts"]
            .as_array()
            .context("prior cuts absent")?
            .last()
            .unwrap()["blob"]
            .clone(),
    )?;
    ensure!(
        input.sha256 == "c2a289dc9739c0d7c2a8688afa76d41595cc74b48a7dd072757156eba373a881",
        "base scan input differs"
    );
    let input_path = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("cold-second.store"));
    let input_store = sims_fixtures::Store::open(input_path)?;
    let (mut p, mut scene, view, vm, mut profiler): Saved = input_store.get_value(&input)?;
    let source = std::fs::read(
        base.join("fixtures/private/goth-window-dispatch-20261031-first/scan-source-first.json"),
    )?;
    p.bind_world_level_geometry(&source)?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let native_path=std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).unwrap_or_else(||base.join("fixtures/private/goth-base-scan-witness-20261101-second/world-classification-trace.jsonl.gz"));
    let native = std::fs::read(native_path)?;
    let pins: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SIMS_DIAG_PINS")
            .map(PathBuf::from)
            .unwrap_or_else(|| out.join("native-pins.json")),
    )?)?;
    ensure!(
        sims_fixtures::digest(&native) == pins["trace_sha256"],
        "base scan trace differs"
    );
    let mut rows = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let r: serde_json::Value = serde_json::from_str(&line?)?;
        let stop = r["trace_stop"].as_u64().unwrap_or(0);
        if stop >= pins["first_trace_stop"].as_u64().unwrap()
            && stop <= pins["last_trace_stop"].as_u64().unwrap()
            && (r["kind"] == "window_tile_scan"
                || r["kind"] == "world_drawing_setup" && r["post_base"].is_object())
        {
            rows.push(r);
        }
    }
    ensure!(
        rows.len() == 122
            && rows.first().unwrap()["pc"] == 0x4ad88fu32
            && rows.last().unwrap()["pc"] == 0x54045bu32,
        "base scan observation range differs"
    );
    let path = std::env::var_os("SIMS_DIAG_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let (mut checks, mut cuts, mut steps, mut tails, mut current) = (vec![], vec![], 0, 64, 0);
    let result = (|| -> Result<()> {
        for (ordinal, r) in rows.iter().enumerate() {
            current = r["pc"].as_u64().context("base scan PC absent")? as u32;
            for _ in 0..64 {
                if p.world_paint().unwrap().at_tile_cut(r) {
                    break;
                }
                ensure!(
                    p.world_paint().unwrap().tile_row().is_none(),
                    "base scan requires an unexecuted row child"
                );
                ensure!(
                    p.advance_world_paint(&mut scene, &mut profiler)?,
                    "base scan stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(r)?;
            p.compare_world_shared_graphics(r)?;
            if ordinal != 0 && current == 0x4ad88f {
                tails += 1;
            }
            checks.push(serde_json::json!({"pc":current,"trace_stop":r["trace_stop"],"legacy_scan_tail_visits":tails,"full_planes_observed":r["buffers"].is_array()}));
            eprintln!("BASE_MATCH {current:x} tails{tails}");
            if ordinal == 0
                || tails == 84 && current == 0x4ad88f
                || matches!(current, 0x4ad89b | 0x4ad2a1 | 0x4ad2af | 0x54045b)
            {
                let (restored, cut) = cold_check(
                    &store,
                    &p,
                    &scene,
                    &view,
                    &vm,
                    &profiler,
                    &format!("{current:x}-tails{tails}"),
                )?;
                p = restored;
                cuts.push(cut);
            }
        }
        ensure!(
            tails == 103
                && unchanged
                    == sims_fixtures::digest(&serde_json::to_vec(&(
                        &scene, &view, &vm, &profiler
                    ))?),
            "base scan changed enclosing VM/RNG/scene/profiler"
        );
        ensure!(
            matches!(
                p.world_paint().unwrap().pending(),
                Some(
                    sims_runtime::original_session::draw_parent::PaintRequest::LevelGeometry {
                        level: 1,
                        ..
                    }
                )
            ),
            "base scan lost its actual geometry child"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"additional_B_source_sha256":sims_fixtures::digest(&source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"legacy_scan_tail_visits":tails,"actual_row_callbacks_returned":62,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"Computed scan-tail64 through natural first base scan return and first level geometry callback entry. Original context, scan/projection inputs and full observed color/depth planes compared. Level model pointer is bound from B and checked against the original callback. No supplied child reply. The geometry body, ordinary selector, outer window/WorldView/Source/Session returns and32 completed ticks remain pending. Dead child locals, unused parent words and full CPU/x87 state remain unadmitted."});
    let output = std::env::var_os("SIMS_DIAG_REPORT").context("base scan report output absent")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"boundaries":checks.len(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
