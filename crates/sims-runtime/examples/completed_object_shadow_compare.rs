//! Original shadow query, resident bitmap selection and compositing, shared caller placement/cache,
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
        if warm
            .0
            .world_paint()
            .and_then(|p| p.row_prefix())
            .is_some_and(|r| {
                r.at_native_observation(&serde_json::json!({"pc":0x4ae9e4u32}))
                    .unwrap_or(false)
            })
        {
            break;
        }
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
        warm.0
            .world_paint()
            .and_then(|p| p.row_prefix())
            .context("shadow row absent")?
            .at_native_observation(&serde_json::json!({"pc":0x4ae9e4u32}))?,
        "shadow completion frontier differs"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_service_results_equal":true,"shadow_return_retained":true}),
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
            sha256: "2f6dc8a917d2c81095b4fda13d7703c8a723d45c6429857fd9f377865ab5bab7".into(),
            bytes: 15616588,
        });
    let input_store = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-visible-sprite-20261101-first/verified-span.store")
            }),
    )?;
    let mut state: Saved = input_store.get_value(&input)?;
    if start == 0 {
        state.0.bind_world_object_shadow(&std::fs::read(root.join(
            "fixtures/private/goth-object-shadow-20261101-first/shadow-source-first.json",
        ))?)?;
    }
    let trace = std::fs::read(root.join(
        "fixtures/private/goth-object-shadow-witness-row21-20261101-second/post-level-trace.jsonl.gz",
    ))?;
    let mut rows = Vec::new();
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        let pc = q["pc"].as_u64().unwrap_or(0);
        if q["row_number"] == 21
            && matches!(
                q["observer"].as_str(),
                Some("ordinary_object" | "ordinary_object_draw")
            )
            && q["trace_stop"].as_u64().is_some_and(|s| s >= start)
            && q["object_lookup"]["head"] == 241
            && matches!(
                pc,
                0x4ae83e
                    | 0x4ae85f
                    | 0x4ae864
                    | 0x4ae86f
                    | 0x4ae874
                    | 0x4ae88c
                    | 0x4ae891
                    | 0x4ae8e8
                    | 0x4ae8eb
                    | 0x4ae8f2
                    | 0x4ae8f7
                    | 0x4ae908
                    | 0x4ae90d
                    | 0x4ae916
                    | 0x4ae91b
                    | 0x4ae91d
                    | 0x4ae922
                    | 0x4ae927
                    | 0x4ae92c
                    | 0x4ae950
                    | 0x4ae955
                    | 0x4ae95b
                    | 0x4ae960
                    | 0x4ae971
                    | 0x4ae976
                    | 0x4ae981
                    | 0x4ae986
                    | 0x4ae995
                    | 0x4ae99a
                    | 0x4ae9a6
                    | 0x4ae9ab
                    | 0x4ae9d3
                    | 0x4b10c4
                    | 0x4ae9d8
                    | 0x4ae9e4
            )
        {
            rows.push(q);
            if pc == 0x4ae9e4 {
                break;
            }
        }
    }
    ensure!(
        rows.last().is_some_and(|q| q["pc"] == 0x4ae9e4u32),
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
            state.6.compare_draw_cache(241, q)?;
            checks.push(serde_json::json!({"pc":pc,"trace_stop":q["trace_stop"]}));
            if matches!(
                pc,
                0x4ae891 | 0x4ae91b | 0x4ae986 | 0x4ae9d3 | 0x4b10c4 | 0x4ae9d8
            ) {
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
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"saved_cuts":saved,"frontier_save_resume":endpoint,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Original shadow query, resident bitmap selection, depth/intensity arguments, color-only composite and actual shadow return. Classification remains the retained component owner. No native return injection or completed-tick credit."});
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
