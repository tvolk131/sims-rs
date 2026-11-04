//! Compare computed row51 gradient services, complete planes and cold resumes.
use anyhow::{ensure, Context, Result};
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
fn ready(s: &Saved, q: &serde_json::Value) -> Result<bool> {
    s.0.world_paint()
        .and_then(|p| p.row_prefix())
        .context("gradient row absent")?
        .at_native_observation(q)
}
fn step(s: &mut Saved, before: &sims_sim::Simulation) -> Result<()> {
    if s.0
        .world_paint()
        .and_then(|p| p.row_prefix())
        .is_some_and(|r| r.complete() && !r.acknowledged())
    {
        s.0.return_world_row()?;
    } else {
        ensure!(
            s.0.advance_world_row_objects_mut(&mut s.3, &mut s.6)?,
            "gradient continuation stalled"
        );
    }
    ensure!(&s.3 == before, "gradient microstep changed VM/RNG");
    Ok(())
}
fn compare(s: &Saved, q: &serde_json::Value) -> Result<()> {
    s.0.compare_world_row_prefix(q)?;
    s.0.compare_world_shared_graphics(q)
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input: sims_fixtures::BlobRef = serde_json::from_str(&std::env::var("SIMS_DIAG_INPUT_REF")
        .unwrap_or_else(|_|"{\"sha256\":\"59d433021301d920777fbe8db20770978718611a33d38e2c10c6d1c36a281c38\",\"bytes\":17277765}".into()))?;
    let from = std::env::var_os("SIMS_DIAG_INPUT_STORE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("fixtures/private/goth-region-loop-20261102-first/base-release-first.store")
        });
    let mut s: Saved = sims_fixtures::Store::open(from)?.get_value(&input)?;
    let source = std::fs::read(
        root.join("fixtures/private/goth-floor-gradient-20261031-first/caller-source-first.json"),
    )?;
    if std::env::var_os("SIMS_DIAG_RESUME").is_none() {
        s.0.bind_world_row_floor_sprite(&source)?;
    }
    let witness = root.join("fixtures/private/goth-next-region-gradient-witness-20261102-first/post-level-trace.jsonl.gz");
    let encoded = std::fs::read(witness)?;
    let mut rows = vec![];
    for l in BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&l?)?;
        if q["observer"] == "window_region_base"
            && (q.get("gradient").is_some() || q["base_row_number"] == 51 && q["pc"] == 0x4ad680)
        {
            rows.push(q);
        }
    }
    ensure!(rows.len() == 13, "gradient evidence sequence differs");
    let pixel_oracle_encoded = std::fs::read(root.join("fixtures/private/goth-single-gradient-20261102-first/captured-blit-store-oracle-fifth.json"))?;
    let pixel_oracle: serde_json::Value = serde_json::from_slice(&pixel_oracle_encoded)?;
    ensure!(
        pixel_oracle["success"] == true && pixel_oracle["complete_native_planes_equal"] == true,
        "original pixel-store component replay was not verified"
    );
    let out = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("gradient output store absent")?,
    )?;
    let before = s.3.clone();
    let (mut checks, mut cuts, mut endpoints) = (vec![], vec![], vec![]);
    let (mut steps, mut current, mut micro) = (0, 0, 0);
    let start: usize = std::env::var("SIMS_DIAG_START")
        .unwrap_or_else(|_| "0".into())
        .parse()?;
    let result = (|| -> Result<()> {
        for (i, q) in rows.iter().enumerate().skip(start) {
            current = q["pc"].as_u64().context("gradient PC absent")? as u32;
            for _ in 0..65536 {
                if ready(&s, q)? {
                    break;
                }
                step(&mut s, &before)?;
                steps += 1;
                if current == 0x4c848a {
                    micro += 1;
                    if [1, 2, 3, 17].contains(&micro) {
                        cuts.push((i, format!("pixel-microstep-{micro}"), out.put_value(&s)?));
                    }
                }
            }
            compare(&s, q).with_context(|| format!("gradient comparison at {current:08x}"))?;
            if current == 0x4c848a {
                let cursor = serde_json::to_value(
                    s.0.world_paint()
                        .and_then(|p| p.row_prefix())
                        .context("gradient cursor absent")?,
                )?;
                ensure!(
                    cursor["floor_cursor"]["draw"]["gradient"]["writes"] == pixel_oracle["events"],
                    "ordered original gradient pixel stores differ"
                );
            }
            let saved = out.put_value(&s)?;
            endpoints.push(saved.clone());
            cuts.push((i, format!("native-{current:08x}-{i}"), saved));
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"],"complete_planes":q.get("buffers").is_some()}));
        }
        Ok(())
    })();
    let mut cold = vec![];
    if result.is_ok() && std::env::var_os("SIMS_DIAG_COLD").is_some() {
        for (index, label, blob) in &cuts {
            let mut r: Saved = out.get_value(blob)?;
            let mut later = 0;
            for (i, q) in rows.iter().enumerate().skip(*index) {
                for _ in 0..65536 {
                    if ready(&r, q)? {
                        break;
                    }
                    step(&mut r, &before)?;
                }
                compare(&r, q)?;
                let expected: Saved = out.get_value(&endpoints[i - start])?;
                ensure!(
                    r == expected,
                    "complete cold gradient state differs at {label}, later boundary {i}"
                );
                later += 1;
            }
            cold.push(serde_json::json!({"cut":label,"blob":blob,"later_complete_state_comparisons":later}));
        }
    }
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,
        "frontier":out.put_value(&s)?,"checks":checks,"steps":steps,"cuts":cuts,"cold":cold,
        "VM_and_RNG_unchanged":s.3==before,"native_trace_sha256":sims_fixtures::digest(&encoded),
        "gradient_B_input_sha256":sims_fixtures::digest(&source),
        "ordered_pixel_store_oracle_sha256":sims_fixtures::digest(&pixel_oracle_encoded),
        "ordered_native_pixel_stores":pixel_oracle["events"].as_array().context("pixel stores absent")?.len(),
        "first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"message":format!("{e:#}")})),
        "source_admission":false,"additional_completed_fresh_ticks":0,
        "scope":"Original row51 gradient caller, computed masks/palette/depth/color writes, complete retained-state cold resumes; no observed replies supplied."});
    std::fs::File::create(
        std::env::var_os("SIMS_DIAG_REPORT").context("gradient report path absent")?,
    )?
    .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"steps":steps,"cold_cuts":cold.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
