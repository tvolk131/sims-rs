//! Continue the retained side frontier through the original C3 renderer.
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
);
fn cold(
    store: &sims_fixtures::Store,
    p: &State,
    s: &WorldScene,
    v: &WorldComposition,
    vm: &sims_sim::Simulation,
    b: &Buffer,
    end: &serde_json::Value,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b))?;
    let (mut restored, rs, rv, rvm, rb): Saved = store.get_value(&blob)?;
    let mut warm = p.clone();
    let mut steps = 0;
    for _ in 0..8192 {
        if warm.world_paint().unwrap().at_tile_cut(end) {
            break;
        }
        let a = warm.advance_world_geometry_child(vm)?;
        let z = restored.advance_world_geometry_child(&rvm)?;
        ensure!(
            a == z && warm == restored,
            "side renderer save/resume differs"
        );
        ensure!(a, "side renderer cold continuation stalled");
        steps += 1;
    }
    ensure!(
        warm.world_paint().unwrap().at_tile_cut(end),
        "side renderer cold endpoint absent"
    );
    ensure!(
        *s == rs && *v == rv && *b == rb && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "side renderer cold immutable owners differ"
    );
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let work = root.join("fixtures/private/goth-geometry-side-20261101-first");
    let input = sims_fixtures::BlobRef {
        sha256: "632dc99bacc5eb7460035e4f3be4b90067587f9a6cbe614b6aa4f44a3308192e".into(),
        bytes: 9714232,
    };
    let (mut p, s, v, vm, b): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join(
                    "fixtures/private/goth-geometry-junction-20261101-first/tail-second.store",
                )
            }),
    )?
    .get_value(&input)?;
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("side renderer trace absent")?)?;
    ensure!(
        sims_fixtures::digest(&native)
            == "6db78028b3c1572de43bb204b9b6c063bcfec3e7a93f2ba5babd17ea637540c5",
        "side renderer trace differs"
    );
    let side_source = std::fs::read(work.join("raster-source-first.json"))?;
    let material = std::fs::read(root.join(
        "fixtures/private/goth-geometry-junction-20261101-first/side-material-source-first.json",
    ))?;
    p.bind_world_geometry_side(&side_source, &material)?;
    let mut rows = vec![];
    let mut started = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53deb2 && q["visible_geometry"].is_null() {
            continue;
        }
        if pc == 0x53e1f7 {
            started = true;
        }
        if started
            && matches!(
                pc,
                0x53e1f7
                    | 0x53e1fc
                    | 0x53e23e
                    | 0x53e243
                    | 0x53e246
                    | 0x53e24b
                    | 0x53e24d
                    | 0x53e2f5
                    | 0x53e2fa
                    | 0x53e3ad
                    | 0x53e3b4
                    | 0x4c8849
                    | 0x4c8e0d
                    | 0x4c8878
                    | 0x4c8884
                    | 0x4c19b1
                    | 0x4c88a0
                    | 0x4c88ac
                    | 0x4c7fdb
                    | 0x4c76d1
                    | 0x4b954a
                    | 0x4c77a1
                    | 0x4c88d4
                    | 0x4c77bc
                    | 0x4c8910
                    | 0x4c89f9
                    | 0x4c9e38
                    | 0x4c8a79
                    | 0x4c8aab
                    | 0x4b444f
                    | 0x4b4456
                    | 0x49f2a3
                    | 0x4c8afa
                    | 0x53e3b9
            )
        {
            rows.push(q)
        }
        if started && pc == 0x53e3b9 {
            break;
        }
    }
    let end = rows.last().context("side renderer observations absent")?;
    ensure!(end["pc"] == 0x53e3b9u32, "side renderer endpoint differs");
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("side renderer output absent")?,
    )?;
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?);
    let (mut cuts, mut checks, mut steps, mut current) = (vec![], vec![], 0, 0);
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..8192 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child(&vm)?,
                    "side renderer stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(current, 0x4c8849 | 0x49f2a3 | 0x4c8afa | 0x53e3b9) {
                cuts.push(cold(&store, &p, &s, &v, &vm, &b, end)?)
            }
            if current == 0x49f2a3 {
                for (n, label) in [
                    (2, "after_first_color_before_depth"),
                    (1, "after_first_depth"),
                    (4, "suspended_pixel_stores"),
                ] {
                    for _ in 0..n {
                        ensure!(
                            p.advance_world_geometry_child(&vm)?,
                            "side store suspension stalled"
                        );
                        steps += 1;
                    }
                    let mut cut = cold(&store, &p, &s, &v, &vm, &b, end)?;
                    cut["label"] = label.into();
                    cuts.push(cut);
                }
            }
        }
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?),
            "side renderer changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"completed_side_renderer":result.is_ok(),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"observed_call_arguments_used_as_inputs":false,"source_admission":false,"native_parity_credit":0,"scope":"Computed side prefix and C3 sprite4096 renderer only. Further model2 cutaway/overlay paths, full vector/Session and32 completed ticks remain pending."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("side renderer report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
