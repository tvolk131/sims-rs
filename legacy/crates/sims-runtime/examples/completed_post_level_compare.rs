//!Replay only derived retained state against the original post-geometry caller.
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
    sims_vm::interface::state::State,
);
fn cold(
    store: &sims_fixtures::Store,
    p: &State,
    s: &WorldScene,
    v: &WorldComposition,
    vm: &sims_sim::Simulation,
    b: &Buffer,
    c: &sims_vm::interface::state::State,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b, c))?;
    let (mut rp, mut rs, rv, rvm, mut rb, rc): Saved = store.get_value(&blob)?;
    ensure!(
        p == &rp
            && s == &rs
            && v == &rv
            && b == &rb
            && c == &rc
            && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "post-level cold initial state differs"
    );
    let (mut wp, mut ws, mut wb) = (p.clone(), s.clone(), b.clone());
    let mut steps = 0;
    for _ in 0..128 {
        let a = wp.advance_world_paint(&mut ws, &mut wb);
        let z = rp.advance_world_paint(&mut rs, &mut rb);
        ensure!(
            wp == rp && ws == rs && wb == rb,
            "post-level warm/cold continuation state differs"
        );
        match (a, z) {
            (Ok(a), Ok(z)) => {
                ensure!(a == z, "post-level progress differs");
                if !a {
                    break;
                }
                steps += 1;
            }
            (Err(a), Err(z)) => {
                ensure!(
                    format!("{a:#}") == format!("{z:#}"),
                    "post-level errors differ"
                );
                break;
            }
            _ => anyhow::bail!("post-level warm/cold reply differs"),
        }
    }
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_service_result_equal":true}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = std::env::var("SIMS_DIAG_INPUT_REF")
        .ok()
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or(sims_fixtures::BlobRef {
            sha256: "677e1bfdb39821dddc48a6372718525154440be8e8e9a99125a2f2b8abfc518c".into(),
            bytes: 10302150,
        });
    let (mut p, mut scene, view, vm, mut profiler, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-visible-cap-20261101-second/full.store")
            }),
    )?
    .get_value(&input)?;
    let start = std::env::var("SIMS_DIAG_START_STOP")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-post-level-witness-20261101-first/post-level-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&trace)
            == "1bc20abb2f383c6f29740048f479a5c2bc6e230df930fac306ce7a61f04271ec",
        "post-level original witness differs"
    );
    let mut rows = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] == "world_drawing_setup"
            && q["pc"].as_u64().is_some_and(|pc| {
                matches!(
                    pc,
                    0x4ad2d9
                        | 0x4ad2df
                        | 0x4ad304
                        | 0x4ad309
                        | 0x4ad30c
                        | 0x4ad345
                        | 0x4ad34f
                        | 0x4ad363
                        | 0x4ad4bd
                        | 0x4ad4df
                        | 0x4ad548
                        | 0x4ad55b
                        | 0x4ad8ac
                )
            })
            && q["trace_stop"].as_u64().is_some_and(|n| n >= start)
        {
            rows.push(q)
        }
    }
    ensure!(!rows.is_empty(), "post-level witness cuts absent");
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("post-level output store absent")?,
    )?;
    let before = serde_json::to_vec(&vm)?;
    let (mut steps, mut current) = (0, 0);
    let mut checks = vec![];
    let mut saved = vec![];
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..1024 {
                if p.world_paint().context("paint absent")?.at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_paint(&mut scene, &mut profiler)?,
                    "post-level caller stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(
                current,
                0x4ad2d9 | 0x4ad309 | 0x4ad363 | 0x4ad4df | 0x4ad8ac
            ) {
                let mut cut = cold(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
                cut["trace_stop"] = q["trace_stop"].clone();
                cut["pc"] = current.into();
                saved.push(cut);
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&vm)?,
        "post-level paint mutated VM/RNG"
    );
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"comparison_boundaries":checks,"computed_microsteps":steps,"saved_cuts":saved,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"observed_arguments_used_as_runtime_inputs":false,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Retained NULL explicit-clip Window region caller, ordered clip writes, copied rectangle, complete above ABI word, reused scanner and first ordinary-row request; selected scopes, not full CPU/device or tick admission."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("post-level report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
