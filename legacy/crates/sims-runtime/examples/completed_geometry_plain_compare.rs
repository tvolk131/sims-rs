//! Continue the computed first-model return into the next ordinary wall and palette call.
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
fn selected(pc: u32) -> bool {
    matches!(
        pc,
        0x4b954a
            | 0x4c77a1
            | 0x53db67
            | 0x53bfb5
            | 0x53db9e
            | 0x53dba9
            | 0x466fcc
            | 0x53dbd2
            | 0x53dbf2
            | 0x53dc67
            | 0x53dc6a
            | 0x53dc78
            | 0x53dc7d
            | 0x53dcbf
            | 0x53dcc4
            | 0x4c79ca
            | 0x4c8e0d
            | 0x4c7a0e
            | 0x4c7a1a
            | 0x4c19b1
            | 0x4c7a41
            | 0x4c7a4d
            | 0x4c7fdb
            | 0x4c76d1
            | 0x4c7a79
            | 0x4c77bc
            | 0x4c7ae9
            | 0x4c7b98
            | 0x4c78c5
            | 0x4c7f75
            | 0x4c4907
            | 0x4c7f7a
            | 0x53ed47
            | 0x53ed4c
            | 0x53ed7b
            | 0x540480
            | 0x53dd65
            | 0x53dd6a
            | 0x53dd8e
            | 0x53dd93
            | 0x53de18
            | 0x53de1d
            | 0x53de3a
            | 0x53de3f
            | 0x53de78
            | 0x53de7d
            | 0x53de8a
            | 0x53de8f
            | 0x53dead
            | 0x53deb2
            | 0x53e84d
            | 0x53e852
            | 0x53e861
            | 0x53e866
            | 0x53e8bc
            | 0x2f12874
            | 0x688d82c0
            | 0x53e8c2
            | 0x53e8f2
            | 0x53e8f8
            | 0x53e934
            | 0x53e939
            | 0x53e953
            | 0x53e958
            | 0x53e95c
            | 0x53e961
            | 0x53e963
            | 0x53e968
            | 0x53ea46
            | 0x53ea4b
            | 0x53eaba
            | 0x53eabf
            | 0x53eaf1
            | 0x53eaf6
            | 0x53ebfb
            | 0x53ecac
            | 0x53ecb1
            | 0x53ecdf
            | 0x53ece4
            | 0x53ed40
            | 0x53ed45
    )
}
fn cold(
    store: &sims_fixtures::Store,
    p: &State,
    s: &WorldScene,
    v: &WorldComposition,
    vm: &sims_sim::Simulation,
    b: &Buffer,
    label: &str,
    endpoint: &serde_json::Value,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b))?;
    let (mut restored, rs, rv, rvm, rb): Saved = store.get_value(&blob)?;
    let mut warm = p.clone();
    let mut steps = 0;
    for _ in 0..8192 {
        if warm.world_paint().unwrap().at_tile_cut(endpoint) {
            break;
        }
        let a = warm.advance_world_geometry_child(vm)?;
        let z = restored.advance_world_geometry_child(&rvm)?;
        ensure!(
            a == z && warm == restored,
            "renderer cold continuation differs"
        );
        if !a {
            break;
        }
        steps += 1;
    }
    ensure!(
        *s == rs && *v == rv && *b == rb && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "renderer cold immutable owners differ"
    );
    Ok(serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let prior = root.join("fixtures/private/goth-geometry-renderer-20261101-first");
    let work = root.join("fixtures/private/goth-geometry-junction-20261101-first");
    let old: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prior.join("joined-fourth.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(
        old["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone(),
    )?;
    ensure!(
        input.sha256 == "f9260a4ffef334303f0a261feebc0e9e5a9649e5b641276bd53e699804512ff7",
        "renderer input frontier differs"
    );
    let (mut p, mut s, v, vm, mut b): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| prior.join("cold-fourth.store")),
    )?
    .get_value(&input)?;
    let material = std::fs::read(work.join("material-source-first.json"))?;
    let raster = std::fs::read(work.join("raster-source-second.json"))?;
    let model = std::fs::read(root.join(
        "fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json",
    ))?;
    p.bind_world_geometry_raster(&raster, &material)?;
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("renderer trace absent")?)?;
    ensure!(
        sims_fixtures::digest(&native)
            == "600994910da9cdac23bf0cb9fc89ef531853f71876cfcd8e4775d997a7931384",
        "renderer trace differs"
    );
    let mut observations = vec![];
    let mut started = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53db67 {
            started = true
        }
        //The same API also runs inside retained placement. Only this return
        //address belongs to the segment's protected OffsetRect dispatch.
        let segment_api = pc != 0x688d82c0
            || q["geometry_prefix"]["stack"]["raw"]
                .as_str()
                .is_some_and(|s| s.starts_with("c2e85300"));
        if started && selected(pc as u32) && segment_api {
            observations.push(q)
        }
        if started && pc == 0x53ed4c {
            break;
        }
    }
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("renderer output store absent")?,
    )?;
    let endpoint = observations.last().context("plain wall endpoint absent")?;
    ensure!(
        endpoint["pc"] == 0x53ed4cu32,
        "plain wall renderer return absent"
    );
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?);
    let (mut checks, mut cuts, mut steps, mut current, mut renderers) = (vec![], vec![], 0, 0, 0);
    let result = (|| -> Result<()> {
        for q in &observations {
            current = q["pc"].as_u64().unwrap() as u32;
            if current == 0x53db67 {
                for _ in 0..8 {
                    if p.world_paint().unwrap().at_tile_cut(q) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut s, &mut b)?,
                        "next model iterator stalled"
                    );
                    steps += 1;
                }
                p.bind_world_geometry_child(&model)?;
            }
            for _ in 0..8192 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child(&vm)?,
                    "renderer stalled at{current:x}"
                );
                steps += 1;
            }
            if current == 0x53dcc4 {
                p.bind_world_geometry_segments_from_scene(&model, &s)?;
            }
            if current == 0x53e8f8 {
                p.bind_world_geometry_visible(&model)?;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            if current == 0x53eaba {
                p.bind_world_geometry_materials(&material)?;
            }
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(
                current,
                0x4c79ca | 0x4b954a | 0x4c77a1 | 0x4c4907 | 0x53ed4c
            ) {
                cuts.push(cold(
                    &store,
                    &p,
                    &s,
                    &v,
                    &vm,
                    &b,
                    &format!("at{current:x}"),
                    endpoint,
                )?);
            }
            if current == 0x4c4907 {
                for (n, label) in [
                    (2, "after_first_depth_store"),
                    (1, "after_first_color_store"),
                    (304, "suspended_stores"),
                ] {
                    for _ in 0..n {
                        ensure!(
                            p.advance_world_geometry_child(&vm)?,
                            "renderer store suspension stalled"
                        );
                        steps += 1;
                    }
                    cuts.push(cold(&store, &p, &s, &v, &vm, &b, label, endpoint)?);
                }
            }
            if current == 0x53ed4c {
                renderers += 1;
                println!(
                    "RENDERER_RETURN {renderers} at original stop{}",
                    q["trace_stop"]
                );
            }
        }
        ensure!(
            current == 0x53ed4c && renderers == 1,
            "visible child did not really return"
        );
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?),
            "renderer changed enclosing VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"raster_source_sha256":sims_fixtures::digest(&raster),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"renderers_returned":renderers,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"completed_visible_child":false,"completed_plain_wall_renderer":result.is_ok(),"observed_call_arguments_used_as_inputs":false,"source_admission":false,"native_parity_credit":0,"scope":"Rust visible caller, shared resource/scratch/plane owners, ordinary C4 renderer return for the first ordinary wall of model2 from the computed model1 return. The remaining model2 segments are pending. Full first vector, enclosing Session/CPU/device and32 completed ticks remain pending. Suspended stores check Rust functional resumption, not a native mid-instruction cut."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("renderer report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"renderers_returned":renderers,"first_divergence":report["first_divergence"]})
    );
    result
}
