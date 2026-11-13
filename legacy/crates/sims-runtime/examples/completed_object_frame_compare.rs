//! B-resident original object drawing metadata, shared caller placement/cache,
//! and suspended continuation. This component does not admit a complete tick.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
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
    WorldClassification,
);
fn cold(store: &sims_fixtures::Store, state: &Saved) -> Result<serde_json::Value> {
    let blob = store.put_value(state)?;
    let mut restored: Saved = store.get_value(&blob)?;
    ensure!(
        serde_json::to_vec(state)? == serde_json::to_vec(&restored)?,
        "sprite cold state differs"
    );
    let mut warm = state.clone();
    let mut steps = 0;
    for _ in 0..2048 {
        if format!("{:?}",warm.0.world_paint().and_then(|p|p.row_prefix()).and_then(|p|p.pending()))=="Some(Ordinary(ObjectDrawReturn))"{break;}
        let a = warm.0.advance_world_row_objects(&warm.3, &mut warm.6)?;
        let b = restored
            .0
            .advance_world_row_objects(&restored.3, &mut restored.6)?;
        ensure!(
            a == b
                && warm.0 == restored.0
                && warm.1 == restored.1
                && warm.2 == restored.2
                && warm.4 == restored.4
                && warm.5 == restored.5
                && warm.6 == restored.6,
            "sprite warm/cold continuation differs"
        );
        if !a {
            break;
        }
        steps += 1;
    }
    ensure!(
        format!(
            "{:?}",
            warm.0
                .world_paint()
                .and_then(|p| p.row_prefix())
                .and_then(|p| p.pending())
        ) == "Some(Ordinary(ObjectDrawReturn))",
        "sprite suffix frontier differs"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_service_results_equal":true,"object_draw_return_retained":true}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let start = std::env::var("SIMS_DIAG_START_STOP")
        .ok()
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let input: sims_fixtures::BlobRef = std::env::var("SIMS_DIAG_INPUT_REF")
        .ok()
        .map(|v| serde_json::from_str(&v))
        .transpose()?
        .unwrap_or(sims_fixtures::BlobRef {
            sha256: "81454902df4a2a40943e2f073cbcb90aec233e0b4086ed94148d755f0fc8f6f8".into(),
            bytes: 10360111,
        });
    let input_store = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-object-sprite-20261101-first/full.store")
            }),
    )?;
    let mut state: Saved = input_store.get_value(&input)?;
    if start == 0 {
        state
            .0
            .bind_world_object_sprite_frames(&std::fs::read(root.join(
                "fixtures/private/goth-object-sprite-frame-20261101-first/frame-source-third.json",
            ))?)?;
    }
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-object-sprite-witness-20261101-first/post-level-trace.jsonl.gz",
    ))?;
    let mut rows = Vec::new();
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        let pc = q["pc"].as_u64().unwrap_or(0);
        if q["row_number"] == 15
            && matches!(
                q["observer"].as_str(),
                Some("ordinary_object" | "ordinary_object_draw")
            )
            && q["trace_stop"].as_u64().is_some_and(|s| s >= start)
            && matches!(
                pc,
                0x437a0b
                    | 0x437a10
                    | 0x437a76
                    | 0x437a7b
                    | 0x437ab3
                    | 0x437ab8
                    | 0x437abd
                    | 0x437ac2
                    | 0x437ad3
                    | 0x437ad8
                    | 0x437af2
                    | 0x437b50
                    | 0x437b55
                    | 0x437b68
                    | 0x437b6d
                    | 0x437c1c
                    | 0x437c25
                    | 0x437c2a
                    | 0x437c56
                    | 0x437d3d
                    | 0x437d40
                    | 0x437d53
                    | 0x437d58
                    | 0x437d73
                    | 0x437d78
                    | 0x437f6a
                    | 0x4aebbb
            )
        {
            rows.push(q);
            if pc == 0x4aebbb {
                break;
            }
        }
    }
    ensure!(
        rows.last().is_some_and(|q| q["pc"] == 0x4aebbbu32),
        "draw trace lacks actual virtual return"
    );
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("sprite output store absent")?,
    )?;
    let before = serde_json::to_vec(&state.3)?;
    let (mut steps, mut pc) = (0, 0);
    let mut checks = Vec::new();
    let mut saved = Vec::new();
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..2048 {
                if state
                    .0
                    .world_paint()
                    .and_then(|p| p.row_prefix())
                    .context("row absent")?
                    .at_native_observation(q)?
                {
                    break;
                }
                if !state.0.advance_world_row_objects(&state.3, &mut state.6)? {
                    break;
                }
                steps += 1;
            }
            state.0.compare_world_row_prefix(q)?;
            state.0.compare_world_shared_graphics(q)?;
            state.6.compare_draw_cache(230, q)?;
            checks.push(serde_json::json!({"pc":pc,"trace_stop":q["trace_stop"]}));
            if matches!(pc, 0x437a7b | 0x437b50 | 0x437b6d | 0x437d73) {
                let mut cut = cold(&store, &state)?;
                cut["pc"] = pc.into();
                cut["trace_stop"] = q["trace_stop"].clone();
                saved.push(cut);
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&state.3)?,
        "drawing metadata altered VM/RNG"
    );
    let endpoint = if result.is_ok() {
        Some(cold(&store, &state)?)
    } else {
        None
    };
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"saved_cuts":saved,"frontier_save_resume":endpoint,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"B-resident frame metadata, computed fully clipped SPR2 bounding rectangle, native layer debug guard, shared bounds publication and actual draw return. Classification remains the retained component owner. No native return injection or completed-tick credit."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("sprite report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"cuts":checks.len(),"steps":steps,"first_unavailable":report["first_unavailable"]})
    );
    result
}
