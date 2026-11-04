//! Follow the computed C3 side return into the next active top-wall prelude.
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
    for _ in 0..128 {
        if warm.world_paint().unwrap().at_tile_cut(end) {
            break;
        }
        let a = warm.advance_world_geometry_child(vm)?;
        let z = restored.advance_world_geometry_child(&rvm)?;
        ensure!(
            a == z && warm == restored,
            "side continuation save/resume differs"
        );
        ensure!(a, "side continuation cold continuation stalled");
        steps += 1;
    }
    ensure!(
        warm.world_paint().unwrap().at_tile_cut(end),
        "side continuation cold endpoint absent"
    );
    ensure!(
        *s == rs && *v == rv && *b == rb && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "side continuation cold immutable owners differ"
    );
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let work = root.join("fixtures/private/goth-geometry-side-20261101-first");
    let input = sims_fixtures::BlobRef {
        sha256: "86412f20bdb94899d460af24f07d2abc573d8438e1fa17edb6ae8d2e06f14576".into(),
        bytes: 9726373,
    };
    let (mut p, s, v, vm, b): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| work.join("cold-second.store")),
    )?
    .get_value(&input)?;
    let native = std::fs::read(
        std::env::var_os("SIMS_DIAG_NATIVE").context("side continuation trace absent")?,
    )?;
    ensure!(
        sims_fixtures::digest(&native)
            == "6db78028b3c1572de43bb204b9b6c063bcfec3e7a93f2ba5babd17ea637540c5",
        "side continuation trace differs"
    );
    let mut rows = vec![];
    let mut started = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53e3b9 {
            started = true
        }
        if started
            && matches!(
                pc,
                0x53e3b9
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
            )
        {
            if pc != 0x688d82c0
                || q["geometry_prefix"]["stack"]["raw"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("c2e85300"))
            {
                rows.push(q)
            }
        }
        if started && pc == 0x53e968 {
            break;
        }
    }
    let end = rows
        .last()
        .context("side continuation observations absent")?;
    ensure!(
        end["pc"] == 0x53e968u32,
        "side continuation endpoint differs"
    );
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("side continuation output absent")?,
    )?;
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?);
    let (mut cuts, mut checks, mut steps, mut current) = (vec![], vec![], 0, 0);
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..128 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child(&vm)?,
                    "side continuation stalled at{current:x}"
                );
                steps += 1;
            }
            if current == 0x53e8f8 {
                p.bind_world_geometry_visible(&std::fs::read(root.join("fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json"))?)?;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(current, 0x53e8f8 | 0x53e968) {
                cuts.push(cold(&store, &p, &s, &v, &vm, &b, end)?)
            }
        }
        let mut pending = p.clone();
        let error = pending
            .advance_world_geometry_child(&vm)
            .expect_err("the unimplemented active material child must stay explicit");
        ensure!(
            error.to_string() == "visible active material setup is pending",
            "unexpected next side frontier: {error:#}"
        );
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?),
            "side continuation changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"pending_original_pc":0x53e9e5u32,"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"observed_call_arguments_used_as_inputs":false,"source_admission":false,"native_parity_credit":0,"scope":"Computed C3 return through the next top-wall placement only. Active build-mode/material-pass services, full model/vector/Session and32 completed ticks remain pending."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("side continuation report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
