//!Original registry lookup and row continuation to the actual object loop.
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
    let (mut rp, rs, rv, rvm, rb, rc): Saved = store.get_value(&blob)?;
    ensure!(
        p == &rp
            && s == &rs
            && v == &rv
            && b == &rb
            && c == &rc
            && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "object cold state differs"
    );
    let mut wp = p.clone();
    let mut steps = 0;
    for _ in 0..256 {
        let a = wp.advance_world_row_prefix(vm);
        let z = rp.advance_world_row_prefix(&rvm);
        ensure!(wp == rp, "object warm/cold continuation differs");
        match (a, z) {
            (Ok(a), Ok(z)) => {
                ensure!(a == z, "object warm/cold progress differs");
                if !a {
                    break;
                }
                steps += 1
            }
            (Err(a), Err(z)) => {
                ensure!(format!("{a:#}") == format!("{z:#}"), "object errors differ");
                break;
            }
            _ => anyhow::bail!("object results differ"),
        }
    }
    ensure!(
        format!(
            "{:?}",
            wp.world_paint()
                .and_then(|p| p.row_prefix())
                .and_then(|p| p.pending())
        ) == "Some(Ordinary(ObjectDraw))",
        "object suffix did not reach its explicit draw child"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_service_result_equal":true,"pending_draw_retained":true}),
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
            sha256: "ceb70590dc62e32aa6a8bad2142b050328983a2430f74ccd534f5993dfdb7b80".into(),
            bytes: 10304270,
        });
    let (mut p, scene, view, vm, profiler, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-ordinary-rows-20261101-second/first.store")
            }),
    )?
    .get_value(&input)?;
    let start = std::env::var("SIMS_DIAG_START_STOP")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    if start == 0 {
        p.bind_world_row_ordinary_objects(&std::fs::read(
            root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"),
        )?)?;
    }
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-ordinary-object-witness-20261101-first/post-level-trace.jsonl.gz",
    ))?;
    let mut rows = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["observer"] == "ordinary_object"
            && q["trace_stop"].as_u64().is_some_and(|n| n >= start)
            && q["pc"].as_u64().is_some_and(|pc| {
                matches!(
                    pc,
                    0x4adace
                        | 0x4adb03
                        | 0x4adb08
                        | 0x4adb25
                        | 0x4adb48
                        | 0x4adb4d
                        | 0x4ae5fd
                        | 0x4ae63b
                        | 0x4ae64b
                        | 0x4ae650
                        | 0x4ae83e
                        | 0x4ae9e4
                        | 0x4ae9fe
                )
            })
        {
            rows.push(q);
            if rows.last().unwrap()["pc"] == 0x4ae9feu32 {
                break;
            }
        }
    }
    ensure!(
        rows.last().is_some_and(|q| q["pc"] == 0x4ae9feu32),
        "object trace lacks first draw loop"
    );
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("object output store absent")?,
    )?;
    let before = serde_json::to_vec(&vm)?;
    let (mut steps, mut pc) = (0, 0);
    let mut checks = vec![];
    let mut saved = vec![];
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().unwrap() as u32;
            steps += p.advance_world_row_prefix_to(&vm, q, 1024)?;
            p.compare_world_row_prefix(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":pc,"trace_stop":q["trace_stop"]}));
            if matches!(pc, 0x4adace | 0x4adb08 | 0x4ae9fe) {
                let mut c = cold(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
                c["trace_stop"] = q["trace_stop"].clone();
                c["pc"] = pc.into();
                saved.push(c);
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&vm)?,
        "object lookup mutated VM/RNG"
    );
    let endpoint = cold(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"saved_cuts":saved,"frontier_save_resume":endpoint,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Original signed object lookup and directory identity, subsequent ordinary row gates and object-loop entry. A real object draw remains pending. No supplied native return or completed-tick credit."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("object report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
