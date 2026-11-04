//! Continue computed image2 through a successful bounded resident-floor sweep.
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
    let mut warm = p.clone();
    let mut n = 0;
    for _ in 0..16 {
        let a = warm.advance_world_row_prefix(vm)?;
        let b = cold.advance_world_row_prefix(&cvm)?;
        ensure!(
            a == b && warm == cold,
            "clipped warm/cold continuation differs at{label}"
        );
        if !a {
            break;
        }
        n += 1;
    }
    ensure!(
        *scene == cs
            && *view == cv
            && *profiler == cp
            && serde_json::to_vec(vm)? == serde_json::to_vec(&cvm)?,
        "clipped cold owners differ"
    );
    Ok((
        restored,
        serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":n}),
    ))
}
fn main() -> Result<()> {
    let base = std::env::var_os("SIMS_RESIDENT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let out = base.join("fixtures/private/goth-resident-floors-complete-20261031-first");
    let old = base.join("fixtures/private/goth-wall-floor-20261031-first");
    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(old.join("joined-final.json"))?)?;
    let input: sims_fixtures::BlobRef =
        serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(
        input.sha256 == "5310aa7dad91ba6727b4970fb779e9b23d050e1e385676e98770804d131bdb68",
        "clipped input differs"
    );
    let input_store = std::env::var_os("SIMS_RESIDENT_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| old.join("cold-first.store"));
    let store = sims_fixtures::Store::open(input_store)?;
    let (mut p, mut scene, view, vm, mut profiler): Saved = store.get_value(&input)?;
    let native = std::fs::read(base.join("fixtures/private/goth-resident-floor-witness-20261031-fourth/world-classification-trace.jsonl.gz"))?;
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("native-pins.json"))?)?;
    ensure!(
        sims_fixtures::digest(&native) == expected["trace_sha256"],
        "clipped trace differs"
    );
    let mut text = String::new();
    flate2::read::GzDecoder::new(native.as_slice()).read_to_string(&mut text)?;
    let rows = text
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let selected = rows
        .iter()
        .filter(|r| {
            r["kind"] == "window_tile_scan"
                && r["trace_stop"].as_u64().unwrap()
                    >= expected["first_trace_stop"].as_u64().unwrap()
                && matches!(
                    r["pc"].as_u64().unwrap(),
                    0x4aee4c
                        | 0x4ad6b2
                        | 0x4ad548
                        | 0x4ad55b
                        | 0x4ad680
                        | 0x49d142
                        | 0x467690
                        | 0x49d14b
                        | 0x4af453
                        | 0x4aed23
                        | 0x4af479
                        | 0x4af485
                        | 0x4af4da
                        | 0x4af4e7
                        | 0x4afae4
                        | 0x4aff8d
                        | 0x4c6383
                        | 0x4affb1
                        | 0x4c811f
                        | 0x4c8e0d
                        | 0x4c8f52
                        | 0x4c8152
                        | 0x49f42d
                        | 0x49f628
                        | 0x49f949
                        | 0x4b954a
                        | 0x4c82c9
                        | 0x4b0000
                )
        })
        .collect::<Vec<_>>();
    ensure!(
        selected.first().context("clipped native cuts absent")?["pc"] == 0x4b0000u32
            && selected.len() > 1,
        "clipped cut range differs"
    );
    p.bind_world_row_floor_sprite(&std::fs::read(base.join(
        "fixtures/private/goth-resident-floors-complete-20261031-first/residents-first.json",
    ))?)?;
    p.bind_world_row_floor_sprite(&std::fs::read(out.join("shadow-header-first.json"))?)?;
    let row_source = std::fs::read(
        base.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let unchanged = sims_fixtures::digest(&serde_json::to_vec(&(&scene, &view, &vm, &profiler))?);
    let path = std::env::var_os("SIMS_RESIDENT_OUTPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("cold-first.store"));
    let store = if path.exists() {
        sims_fixtures::Store::open(path)?
    } else {
        sims_fixtures::Store::create(path)?
    };
    let mut checks = vec![];
    let mut cuts = vec![];
    let mut steps = 0;
    let mut returned = 41;
    let mut wall_cuts = std::collections::BTreeSet::new();
    let mut current = 0;
    let result = (|| -> Result<()> {
        for r in selected {
            let pc = r["pc"].as_u64().unwrap() as u32;
            current = pc;
            if matches!(pc, 0x4aee4c | 0x4ad6b2 | 0x4ad548 | 0x4ad55b) {
                for _ in 0..64 {
                    if p.world_paint().unwrap().at_tile_cut(r) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut scene, &mut profiler)?,
                        "clipped scan stalled"
                    );
                }
                p.compare_world_tiles(r)?;
                if pc == 0x4aee4c {
                    p.bind_world_row_prefix(&row_source)?;
                }
            } else {
                steps += p.advance_world_row_prefix_to(&vm, r, 1000000)?;
                if pc == 0x4ad680 {
                    ensure!(
                        p.world_paint().unwrap().row_prefix().unwrap().complete(),
                        "clipped row return not reached"
                    );
                    p.return_world_row()?;
                    returned += 1;
                    p.compare_world_tiles(r)?;
                    let state =
                        serde_json::to_value(p.world_paint().unwrap().row_prefix().unwrap())?;
                    let bytes = sims_runtime::original_session::completed_animations::bytes(
                        r["row_frame"]["raw"]
                            .as_str()
                            .context("clipped return locals absent")?,
                    )?;
                    let word = |i| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
                    ensure!(
                        serde_json::json!(
                            bytes[0x214..0x217]
                                .iter()
                                .map(|v| *v as i8)
                                .collect::<Vec<_>>()
                        ) == state["tail"]["point"]
                            && state["tail"]["index"] == word(0x174)
                            && state["tail"]["screen"]
                                == serde_json::json!([word(0x238), word(0x23c)]),
                        "clipped return locals differ"
                    );
                }
                p.compare_world_row_prefix(r)?;
            }
            checks.push(
                serde_json::json!({"pc":pc,"trace_stop":r["trace_stop"],"rows_returned":returned}),
            );
            eprintln!("RESIDENT_MATCH {pc:x} rows{returned}");
            let wall = matches!(pc, 0x467690 | 0x4af4da | 0x4af4e7 | 0x4afae4)
                && r.get("floor_wall_result").is_some()
                || matches!(pc, 0x4af4e7 | 0x4afae4);
            // Sample each first wall phase, an intervening row, and the endpoint.
            let wall = wall
                || matches!(pc, 0x4b954a | 0x4c82c9)
                || pc == 0x467690
                    && r["stack"]["raw"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("daf44a00"));
            let last = r["trace_stop"] == expected["last_trace_stop"];
            if wall && wall_cuts.insert(pc) || pc == 0x4ad680 && matches!(returned, 44 | 48) || last
            {
                let (restored, cut) = cold_check(
                    &store,
                    &p,
                    &scene,
                    &view,
                    &vm,
                    &profiler,
                    &format!("{pc:x}-rows{returned}"),
                )?;
                cuts.push(cut);
                p = restored;
            }
        }
        ensure!(
            !wall_cuts.is_empty()
                && unchanged
                    == sims_fixtures::digest(&serde_json::to_vec(&(
                        &scene, &view, &vm, &profiler
                    ))?),
            "clipped changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"rows_returned":returned,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":unchanged == sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"source_admission":false,"native_parity_credit":0,"scope":"Actual computed first image2 return through the resident-floor sweep endpoint. Shared contexts, consumed wall segment words, resource calls, returned bounds and full color/depth buffers compared. Remaining40 expanded-record bytes are not fabricated or admitted. All nine resident resources and both palette tables are imported from B, with live default/cached palette inputs and result checks; palette rebuild/conversion remains unsupported. No completed WorldView drawing or Source/Session or completed-tick admission."});
    let output =
        std::env::var_os("SIMS_RESIDENT_REPORT").context("clipped report output absent")?;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"boundaries":checks.len(),"rows_returned":returned,"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
