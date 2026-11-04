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
type Prefix = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    sims_vm::interface::state::State,
);
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
        ) == "Some(Ordinary(SpriteFrame))",
        "sprite suffix frontier differs"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"state_and_service_results_equal":true,"sprite_frame_child_pending":true}),
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
            sha256: "ceb70590dc62e32aa6a8bad2142b050328983a2430f74ccd534f5993dfdb7b80".into(),
            bytes: 10304270,
        });
    let input_store = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-ordinary-rows-20261101-second/first.store")
            }),
    )?;
    let mut state: Saved = if start == 0 {
        let (mut p, s, v, sim, b, c): Prefix = input_store.get_value(&input)?;
        let d = std::fs::read(
            root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"),
        )?;
        p.bind_world_row_ordinary_objects(&d)?;
        p.bind_world_object_sprites(
            &std::fs::read(
                root.join("fixtures/private/goth-object-sprite-20261101-first/sprite-source.json"),
            )?,
            &d,
            &sim,
        )?;
        // A component input, not a claim to have executed classification here.
        // The complete retained caller must pass its already-owned instance.
        let k=WorldClassification::import(&std::fs::read(root.join("fixtures/private/goth-world-classification-20261031-first/classification-source-third.json"))?,&sim)?;
        (p, s, v, sim, b, c, k)
    } else {
        input_store.get_value(&input)?
    };
    let trace=std::fs::read(root.join("fixtures/private/goth-ordinary-object-draw-witness-20261101-first/post-level-trace.jsonl.gz"))?;
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
                    | 0x4aea7a
                    | 0x4aea7f
                    | 0x4aea83
                    | 0x4aea88
                    | 0x4aeaa8
                    | 0x4aeaad
                    | 0x4aebb8
                    | 0x4378bc
                    | 0x4378d3
                    | 0x4378d8
                    | 0x4378f5
                    | 0x437907
                    | 0x437922
                    | 0x437927
                    | 0x43793d
                    | 0x437942
                    | 0x4379b7
                    | 0x4379d5
                    | 0x4379da
                    | 0x4379ea
                    | 0x4379ef
                    | 0x437a0b
            )
        {
            rows.push(q);
            if pc == 0x437a0b {
                break;
            }
        }
    }
    ensure!(
        rows.last().is_some_and(|q| q["pc"] == 0x437a0bu32),
        "draw trace lacks frame-count call"
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
            if matches!(pc, 0x4aea83 | 0x4378d8 | 0x437942 | 0x4379da) {
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
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"trace_start_stop":start,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"saved_cuts":saved,"frontier_save_resume":endpoint,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Resident B DGRP selection, actual live VM selector/direction/position, caller placement, shared rectangle clear and draw prefix to the sprite frame child. Classification owner is a scoped B input here; its prior callbacks are not claimed. No supplied native return or completed-tick credit."});
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
