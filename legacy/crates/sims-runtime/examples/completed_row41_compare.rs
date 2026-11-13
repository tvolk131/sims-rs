//! Original mirrored SPR2 row41 proof with retained dispatch and suspended stores.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    sims_vm::interface::state::State,
    WorldClassification,
);
fn trace(path: &Path) -> Result<Vec<serde_json::Value>> {
    let encoded = std::fs::read(path)?;
    BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice()))
        .lines()
        .map(|l| Ok(serde_json::from_str(&l?)?))
        .collect()
}
fn cold(
    store: &sims_fixtures::Store,
    state: &Saved,
    ret: &serde_json::Value,
) -> Result<serde_json::Value> {
    let blob = store.put_value(state)?;
    let mut restored: Saved = store.get_value(&blob)?;
    ensure!(&restored == state, "SPR2 cold reconstruction differs");
    let mut warm = state.clone();
    let mut steps = 0;
    for _ in 0..65536 {
        if warm
            .0
            .world_paint()
            .and_then(|w| w.row_prefix())
            .context("SPR2 row absent")?
            .at_native_observation(ret)?
        {
            break;
        }
        let a = warm.0.advance_world_row_objects(&warm.3, &mut warm.6)?;
        let b = restored
            .0
            .advance_world_row_objects(&restored.3, &mut restored.6)?;
        ensure!(
            a && a == b && warm == restored,
            "SPR2 suspended continuation differs"
        );
        steps += 1;
    }
    warm.0.compare_world_row_prefix(ret)?;
    warm.0.compare_world_shared_graphics(ret)?;
    warm.0
        .world_paint()
        .and_then(|w| w.row_prefix())
        .context("SPR2 row absent")?
        .compare_object_raster(ret)?;
    ensure!(
        warm == restored && warm.3 == state.3,
        "SPR2 resume changed VM/RNG"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"actual_kernel_return_planes_and_all_scratch_match":true,"state_and_services_equal":true}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = sims_fixtures::BlobRef {
        sha256: "544553ba94a742e626b1cf84647e8833dc654153827d3fa38231dff31b22d306".into(),
        bytes: 15715707,
    };
    let mut state: Saved = sims_fixtures::Store::open(
        root.join("fixtures/private/goth-expanded-render-20261101-first/row39-first.store"),
    )?
    .get_value(&input)?;
    state.0.bind_world_object_raster_mirror(&std::fs::read(
        root.join("fixtures/private/goth-visible-sprite-20261101-first/raster-source-second.json"),
    )?)?;
    let prior: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("fixtures/private/goth-expanded-render-20261101-first/row39-first.json"),
    )?)?;
    let mut pcs: BTreeSet<u64> = prior["checks"]
        .as_array()
        .context("prior PC set absent")?
        .iter()
        .filter_map(|q| q["pc"].as_u64())
        .collect();
    pcs.extend([0x4bca26, 0x4c1d14]);
    let witness = root.join(
        "fixtures/private/goth-mirrored-row41-witness-20261101-second/post-level-trace.jsonl.gz",
    );
    let raw = std::fs::read(&witness)?;
    let captured: Vec<_> = trace(&witness)?
        .into_iter()
        .filter(|q| q["observer"] == "ordinary_object_draw" && q["row_number"] == 41)
        .collect();
    ensure!(!captured.is_empty(), "row41 original evidence absent");
    let unselected: Vec<_> = captured
        .iter()
        .filter(|q| !q["pc"].as_u64().is_some_and(|pc| pcs.contains(&pc)))
        .map(|q| serde_json::json!({"pc":q["pc"],"stop":q["trace_stop"]}))
        .collect();
    let selected: Vec<_> = captured
        .iter()
        .filter(|q| q["pc"].as_u64().is_some_and(|pc| pcs.contains(&pc)))
        .cloned()
        .collect();
    let mut rows: Vec<_> = trace(&root.join(
        "fixtures/private/goth-ordinary-scan-witness-20261101-second/post-level-trace.jsonl.gz",
    ))?
    .into_iter()
    .filter(|q| {
        q["observer"] == "ordinary_rows"
            && q["row_number"].as_u64().is_some_and(|n| n == 40)
            && matches!(q["pc"].as_u64(), Some(0x4ad8ac | 0x4ad6ab))
    })
    .collect();
    ensure!(rows.len() == 2, "preceding rows missing");
    rows.extend(selected);
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("row41 output store absent")?,
    )?;
    let before = serde_json::to_vec(&state.3)?;
    let (mut checks, mut saved, mut partial) = (Vec::new(), Vec::new(), Vec::new());
    let (mut steps, mut pc, mut stop, mut phase) = (0, 0, 0, "setup");
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().context("PC absent")? as u32;
            stop = q["trace_stop"].as_u64().context("stop absent")?;
            phase = "advance";
            if pc == 0x4ad8ac {
                if state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .is_some_and(|r| r.complete() && !r.acknowledged())
                {
                    state.0.return_world_row()?;
                }
                for _ in 0..8192 {
                    if state
                        .0
                        .world_paint()
                        .context("paint absent")?
                        .at_tile_cut(q)
                    {
                        break;
                    }
                    ensure!(
                        state.0.advance_world_paint(&mut state.1, &mut state.4)?,
                        "row41 caller stopped"
                    );
                    steps += 1;
                }
                state.0.compare_world_tiles(q)?;
                state.0.bind_world_row_prefix(&std::fs::read(root.join(
                    "fixtures/private/goth-row-context-20261031-first/row-source-first.json",
                ))?)?;
                state.0.bind_world_row_ordinary(&std::fs::read(root.join(
                    "fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json",
                ))?)?;
                state
                    .0
                    .bind_world_row_ordinary_objects(&std::fs::read(root.join(
                        "fixtures/private/goth-ordinary-object-20261101-first/object-source.json",
                    ))?)?;
            } else {
                for _ in 0..65536 {
                    if state
                        .0
                        .world_paint()
                        .and_then(|w| w.row_prefix())
                        .context("row41 absent")?
                        .at_native_observation(q)?
                    {
                        break;
                    }
                    ensure!(
                        state.0.advance_world_row_objects(&state.3, &mut state.6)?,
                        "row41 child stopped before{pc:x}"
                    );
                    steps += 1;
                }
            }
            phase = "row/context";
            state.0.compare_world_row_prefix(q)?;
            phase = "graphics/planes";
            state.0.compare_world_shared_graphics(q)?;
            if matches!(pc, 0x4bc606 | 0x4c1ee5 | 0x4bca26 | 0x4c1d14) {
                phase = "SPR2 arguments/scratch";
                state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .context("row absent")?
                    .compare_object_raster(q)?;
            }
            if q["spr2_owners"].is_object() {
                phase = "SPR2 owners";
                state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .context("row absent")?
                    .compare_object_raster_batch(q)?;
            }
            if let Some(id) = q["object_lookup"]["head"].as_i64() {
                phase = "object cache";
                state.6.compare_draw_cache(id as i16, q)?;
            }
            checks.push(serde_json::json!({"row_number":q["row_number"],"pc":pc,"stop":stop}));
            if pc == 0x4bca26
                && saved.is_empty()
                && std::env::var_os("SIMS_DIAG_NO_COLD_RECHECK").is_none()
            {
                phase = "cold/SSE store cuts";
                let ret = rows
                    .iter()
                    .find(|r| {
                        r["trace_stop"].as_u64().is_some_and(|n| n > stop) && r["pc"] == 0x4c1d14u32
                    })
                    .context("kernel return absent")?;
                let mut cut = cold(&store, &state, ret)?;
                cut["pc"] = pc.into();
                cut["stop"] = stop.into();
                saved.push(cut);
                let mut probe = state.clone();
                ensure!(
                    probe.0.advance_world_row_objects(&probe.3, &mut probe.6)?,
                    "SPR2 planner stopped"
                );
                let progress = |s: &Saved| {
                    s.0.world_paint()
                        .and_then(|w| w.row_prefix())
                        .and_then(|r| r.object_raster_store_progress())
                        .context("SPR2 plan absent")
                };
                let (at, total) = progress(&probe)?;
                ensure!(at == 0 && total > 2, "SPR2 plan empty");
                for desired in [1, total / 2] {
                    while progress(&probe)?.0 < desired {
                        ensure!(
                            probe.0.advance_world_row_objects(&probe.3, &mut probe.6)?,
                            "SPR2 store stopped"
                        );
                    }
                    let mut cut = cold(&store, &probe, ret)?;
                    cut["computed_stores_applied"] = desired.into();
                    cut["computed_stores_total"] = total.into();
                    cut["original_inner_cpu_cut_observed"] = false.into();
                    partial.push(cut);
                }
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&state.3)?,
        "row41 changed VM/RNG"
    );
    let frontier = store.put_value(&state)?;
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"frontier":frontier,"checks":checks,"steps":steps,"saved_kernel_cuts":saved,"partial_kernel_cuts":partial,"captured_row41_cuts":captured.len(),"unselected_native_cuts":unselected,"native_trace_sha256":sims_fixtures::digest(&raw),"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"stop":stop,"phase":phase,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("row41 report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
