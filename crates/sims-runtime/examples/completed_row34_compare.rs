//! Check original dynamic-object skipping before any drawing-source demand.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
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
fn cold_shadow(
    store: &sims_fixtures::Store,
    state: &Saved,
    native_return: &serde_json::Value,
) -> Result<serde_json::Value> {
    let blob = store.put_value(state)?;
    let mut cold: Saved = store.get_value(&blob)?;
    ensure!(state == &cold, "shadow cold reconstruction differs");
    let mut warm = state.clone();
    let mut steps = 0;
    for _ in 0..8192 {
        if warm
            .0
            .world_paint()
            .and_then(|p| p.row_prefix())
            .context("shadow row absent")?
            .at_native_observation(native_return)?
        {
            break;
        }
        let a = warm.0.advance_world_row_objects(&warm.3, &mut warm.6)?;
        let b = cold.0.advance_world_row_objects(&cold.3, &mut cold.6)?;
        ensure!(
            a && a == b && warm == cold,
            "shadow cold continuation differs"
        );
        steps += 1;
    }
    warm.0.compare_world_row_prefix(native_return)?;
    warm.0.compare_world_shared_graphics(native_return)?;
    ensure!(
        state.3 == warm.3 && warm == cold,
        "shadow restore changed VM/RNG"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_services_equal":true,"actual_shadow_return_and_planes_match":true}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let start = std::env::var("SIMS_DIAG_START_STOP")
        .ok()
        .map(|v| v.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let input = std::env::var("SIMS_DIAG_INPUT_REF")
        .ok()
        .map(|v| serde_json::from_str::<sims_fixtures::BlobRef>(&v))
        .transpose()?
        .unwrap_or(sims_fixtures::BlobRef {
            sha256: "4377d412e4603c8b312785ef70e3fe8fab9197a39cd4915f4f80580febc38056".into(),
            bytes: 15721643,
        });
    let (mut p, mut s, v, vm, mut b, c, mut k): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join(
                    "fixtures/private/goth-object-shadow-20261101-first/row32-final.store",
                )
            }),
    )?
    .get_value(&input)?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("fixtures/private/goth-object-sprite-frame-20261101-first/row-full.json"),
    )?)?;
    let mut pcs: BTreeSet<u64> = report["comparison_boundaries"]
        .as_array()
        .context("prior cuts absent")?
        .iter()
        .filter_map(|q| q["pc"].as_u64())
        .collect();
    pcs.extend([
        0x4ad8ac, 0x4ad6ab, 0x4bc606, 0x4c1ee5, 0x4ae83e, 0x4ae85f, 0x4ae864, 0x4ae86f, 0x4ae874,
        0x4ae88c, 0x4ae891, 0x4ae8e8, 0x4ae8eb, 0x4ae8f2, 0x4ae8f7, 0x4ae908, 0x4ae90d, 0x4ae916,
        0x4ae91b, 0x4ae91d, 0x4ae922, 0x4ae927, 0x4ae92c, 0x4ae950, 0x4ae955, 0x4ae95b, 0x4ae960,
        0x4ae971, 0x4ae976, 0x4ae981, 0x4ae986, 0x4ae995, 0x4ae99a, 0x4ae9a6, 0x4ae9ab, 0x4ae9d3,
        0x4b10c4, 0x4ae9d8, 0x4ae9e4,
    ]);
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-object-row34-witness-20261101-first/post-level-trace.jsonl.gz",
    ))?;
    let mut rows = Vec::new();
    let mut unselected = Vec::new();
    let mut captured = 0;
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["observer"] == "ordinary_object_draw" && q["row_number"] == 34 {
            captured += 1;
            if q["pc"].as_u64().is_some_and(|pc| pcs.contains(&pc)) {
                if q["trace_stop"].as_u64().is_some_and(|n| n >= start) {
                    rows.push(q);
                }
            } else {
                unselected.push(serde_json::json!({"pc":q["pc"],"stop":q["trace_stop"]}));
            }
        }
    }
    let prior=std::fs::read(root.join("fixtures/private/goth-ordinary-scan-witness-20261101-second/post-level-trace.jsonl.gz"))?;
    let mut prefix=Vec::new();
    for line in BufReader::new(flate2::read::GzDecoder::new(prior.as_slice())).lines(){let q:serde_json::Value=serde_json::from_str(&line?)?;if q["observer"]=="ordinary_rows"&&q["row_number"]==33&&matches!(q["pc"].as_u64(),Some(0x4ad8ac|0x4ad6ab)){prefix.push(q);}}
    ensure!(prefix.len()==2,"row33 entry/return absent");prefix.extend(rows);let rows=prefix;
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("row32 store absent")?,
    )?;
    let native_shadow_return = rows
        .iter()
        .find(|q| q["pc"] == 0x4ae9d8u32 && q["object_lookup"]["head"] == 23)
        .cloned();
    let dynamic_return=rows.iter().find(|q|q["row_number"]==34&&q["pc"]==0x4ad6abu32).cloned().context("row34 return absent")?;
    let mut dynamic_cold=Vec::new();
    let before = serde_json::to_vec(&vm)?;
    let mut saved = Vec::new();
    let mut partial = Vec::new();
    let mut checks = vec![];
    let mut steps = 0;
    let mut pc = 0;
    let mut stop = 0;
    let mut phase = "setup";
    let result = (|| -> Result<()> {
        if start == 0 {
            for _ in 0..65536 {
                if p.world_paint()
                    .and_then(|w| w.row_prefix())
                    .context("row absent")?
                    .complete()
                {
                    break;
                }
                ensure!(
                    p.advance_world_row_objects(&vm, &mut k)?,
                    "prior row did not complete"
                );
                steps += 1;
            }
            p.return_world_row()?;
        }
        for q in &rows {
            pc = q["pc"].as_u64().unwrap() as u32;
            stop = q["trace_stop"].as_u64().unwrap();
            if pc == 0x4ad8ac {
                if p.world_paint().and_then(|w|w.row_prefix()).is_some_and(|r|r.complete()&&!r.acknowledged()){p.return_world_row()?;}
                for _ in 0..8192 {
                    if p.world_paint().context("paint absent")?.at_tile_cut(q) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut s, &mut b)?,
                        "caller did not reach row32"
                    );
                    steps += 1;
                }
                p.compare_world_tiles(q)?;
                p.bind_world_row_prefix(&std::fs::read(root.join(
                    "fixtures/private/goth-row-context-20261031-first/row-source-first.json",
                ))?)?;
                p.bind_world_row_ordinary(&std::fs::read(root.join(
                    "fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json",
                ))?)?;
                p.bind_world_row_ordinary_objects(&std::fs::read(root.join(
                    "fixtures/private/goth-ordinary-object-20261101-first/object-source.json",
                ))?)?;
            } else {
                for _ in 0..65536 {
                    if p.world_paint()
                        .and_then(|w| w.row_prefix())
                        .context("row32 absent")?
                        .at_native_observation(q)?
                    {
                        break;
                    }
                    if !p.advance_world_row_objects(&vm, &mut k)? {
                        break;
                    }
                    steps += 1;
                }
            }
            phase = "row/context";
            p.compare_world_row_prefix(q)?;
            phase = "graphics/planes";
            p.compare_world_shared_graphics(q)?;
            if matches!(pc, 0x4bc606 | 0x4c1ee5) {
                phase = "pixel arguments/scratch";
                p.world_paint()
                    .and_then(|p| p.row_prefix())
                    .context("row absent")?
                    .compare_object_raster(q)?;
            }
            if let Some(id) = q["object_lookup"]["head"].as_i64() {
                phase = "object cache";
                k.compare_draw_cache(id as i16, q)?;
            }
            checks.push(serde_json::json!({"pc":pc,"stop":stop}));
            if q["object_lookup"]["head"]==41 && matches!(pc,0x4ae9fe|0x4aebde){let state=(p.clone(),s.clone(),v.clone(),vm.clone(),b.clone(),c.clone(),k.clone());let mut cut=cold_shadow(&store,&state,&dynamic_return)?;cut["pc"]=pc.into();cut["stop"]=stop.into();dynamic_cold.push(cut);}

            if q["object_lookup"]["head"] == 23
                && matches!(pc, 0x4ae891 | 0x4ae91b | 0x4b10c4 | 0x4ae9d8)
            {
                let state = (
                    p.clone(),
                    s.clone(),
                    v.clone(),
                    vm.clone(),
                    b.clone(),
                    c.clone(),
                    k.clone(),
                );
                let n = native_shadow_return
                    .as_ref()
                    .context("shadow return absent")?;
                let mut cut = cold_shadow(&store, &state, n)?;
                cut["pc"] = pc.into();
                cut["stop"] = stop.into();
                saved.push(cut);
                if pc == 0x4b10c4 {
                    let mut probe = state;
                    ensure!(
                        probe.0.advance_world_row_objects(&probe.3, &mut probe.6)?,
                        "shadow plan did not progress"
                    );
                    let (at, total) = probe
                        .0
                        .world_paint()
                        .and_then(|p| p.row_prefix())
                        .and_then(|r| r.object_shadow_store_progress())
                        .context("shadow drawing absent")?;
                    ensure!(at == 0 && total > 2, "visible shadow has no stores");
                    for desired in [1, total / 2] {
                        while probe
                            .0
                            .world_paint()
                            .and_then(|p| p.row_prefix())
                            .and_then(|r| r.object_shadow_store_progress())
                            .context("shadow drawing ended")?
                            .0
                            < desired
                        {
                            ensure!(
                                probe.0.advance_world_row_objects(&probe.3, &mut probe.6)?,
                                "shadow store stopped"
                            );
                        }
                        let mut cut = cold_shadow(&store, &probe, n)?;
                        cut["computed_stores_applied"] = desired.into();
                        cut["computed_stores_total"] = total.into();
                        cut["resume_native_stop"] = n["trace_stop"].clone();
                        cut["original_inner_cpu_cut_observed"] = false.into();
                        partial.push(cut);
                    }
                }
            }
        }
        Ok(())
    })();
    ensure!(before == serde_json::to_vec(&vm)?, "row32 changed VM/RNG");
    let blob = store.put_value(&(&p, &s, &v, &vm, &b, &c, &k))?;
    let raw = serde_json::to_value(p.world_paint().and_then(|w| w.row_prefix()))?;
    let cursor = &raw["ordinary"];
    let diag = serde_json::json!({"head":cursor["head"],"point":cursor["point"],"index":cursor["index"],"stage":cursor["stage"],"drawing":cursor["draw_cursor"],"raster_palette":cursor["draw_source"]["frames"]});
    let output = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"frontier":blob,"checks":checks,"steps":steps,"dynamic_skip_cold_cuts":dynamic_cold,"saved_shadow_cuts":saved,"partial_shadow_cuts":partial,"trace_start_stop":start,"captured_row34_cuts":captured,"unselected_native_cuts":unselected,"native_trace_sha256":sims_fixtures::digest(&trace),"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"stop":stop,"phase":phase,"message":format!("{e:#}")})),"diagnostic":diag,"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("row32 report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&output)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":output["first_unavailable"]})
    );
    result
}
