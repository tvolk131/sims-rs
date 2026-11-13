//!Original ordinary-row caller cuts; expected values never supply replies.
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
    mid: bool,
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
        "ordinary cold state differs"
    );
    let mut wp = p.clone();
    let (mut steps, mut intermediate) = (0, None);
    for _ in 0..256 {
        let a = wp.advance_world_row_prefix(vm);
        let z = rp.advance_world_row_prefix(&rvm);
        ensure!(wp == rp, "ordinary continuation differs");
        match (a, z) {
            (Ok(a), Ok(z)) => {
                ensure!(a == z, "ordinary progress differs");
                if !a {
                    break;
                }
                steps += 1;
                if mid && steps == 3 {
                    let blob = store.put_value(&(&wp, s, v, vm, b, c))?;
                    let (restored, ss, vv, vvm, bb, cc): Saved = store.get_value(&blob)?;
                    ensure!(
                        wp == restored
                            && s == &ss
                            && v == &vv
                            && b == &bb
                            && c == &cc
                            && serde_json::to_vec(vm)? == serde_json::to_vec(&vvm)?,
                        "ordinary child save differs"
                    );
                    rp = restored;
                    intermediate =
                        Some(serde_json::json!({"blob":blob,"computed_child_steps":steps}));
                }
            }
            (Err(a), Err(z)) => {
                ensure!(
                    format!("{a:#}") == format!("{z:#}"),
                    "ordinary service errors differ"
                );
                break;
            }
            _ => anyhow::bail!("ordinary reply differs"),
        }
    }
    ensure!(p.world_paint().is_some(),"ordinary paint owner absent");
    ensure!(wp.world_paint().and_then(|p|p.row_prefix()).is_some_and(|r|r.complete()),"ordinary cold suffix did not reach the actual row return");
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"exact_state_and_service_result_equal":true,"intermediate":intermediate}),
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
            sha256: "cd19b973136d85603f1c2130b08c448162d079d0763af85564d542feb627c82c".into(),
            bytes: 10302446,
        });
    let (mut p, scene, view, vm, profiler, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-post-level-20261101-first/full-final.store")
            }),
    )?
    .get_value(&input)?;
    let start = std::env::var("SIMS_DIAG_START_STOP")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    if start == 0 {
        p.bind_world_row_prefix(&std::fs::read(root.join(
            "fixtures/private/goth-row-context-20261031-first/row-source-first.json",
        ))?)?;
        p.bind_world_row_ordinary(&std::fs::read(
            root.join("fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json"),
        )?)?;
    }
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-ordinary-row-witness-20261101-second/post-level-trace.jsonl.gz",
    ))?;
    ensure!(
        sims_fixtures::digest(&trace)
            == "f7a2c85b3e8110499bf6f5288d132d79ccbfe7c0fd4ce075f49eeec0ce093efe",
        "ordinary original trace differs"
    );
    let mut rows = vec![];
    let mut untyped = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["observer"] != "ordinary_row" || q["trace_stop"].as_u64().is_none_or(|n| n < start) {
            continue;
        }
        let pc = q["pc"].as_u64().context("ordinary PC absent")?;
        if matches!(pc, 0x4ad8b1 | 0x4ad8b6) {
            untyped.push(q["trace_stop"].clone());
        } else if (0x4ad8ac..=0x4aec31).contains(&pc) || pc == 0x4ad6ab {
            rows.push(q)
        }
    }
    ensure!(
        rows.last().is_some_and(|q| q["pc"] == 0x4ad6abu32),
        "ordinary trace has no real row return"
    );
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("ordinary output store absent")?,
    )?;
    let before = serde_json::to_vec(&vm)?;
    let (mut steps, mut pc) = (0, 0);
    let mut checks = vec![];
    let mut saved = vec![];
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().unwrap() as u32;
            steps += p.advance_world_row_prefix_to(&vm, q, 2048)?;
            p.compare_world_row_prefix(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":pc,"trace_stop":q["trace_stop"]}));
            if matches!(
                pc,
                0x4ad8ac | 0x4adaa9 | 0x4adac9 | 0x4ae650 | 0x4aec23 | 0x4ad6ab
            ) {
                let mut cut = cold(
                    &store,
                    &p,
                    &scene,
                    &view,
                    &vm,
                    &profiler,
                    &controls,
                    pc == 0x4adac9,
                )?;
                cut["pc"] = pc.into();
                cut["trace_stop"] = q["trace_stop"].clone();
                saved.push(cut);
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&vm)?,
        "ordinary renderer mutated VM or RNG"
    );
    let frontier = cold(&store, &p, &scene, &view, &vm, &profiler, &controls, false)?;
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"comparison_boundaries":checks,"computed_microsteps":steps,"untyped_SEH_observations_retained":untyped,"saved_cuts":saved,"frontier_save_resume":frontier,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"observed_arguments_used_as_runtime_replies":false,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Original ordinary empty-content caller and real return, borrowed shared light/placement/raster owners, selected suspended saves. Walls, upper floors, split lighting, objects and pool children remain explicit pending work. No full CPU/device or32-tick admission."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("ordinary report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
