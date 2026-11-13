//! Compute the next segment and its original alternate alternate side.
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
    end: &serde_json::Value,
    controls: &sims_vm::interface::state::State,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b, controls))?;
    let (mut restored, rs, rv, rvm, rb, rc): Saved = store.get_value(&blob)?;
    let mut warm = p.clone();
    let mut steps = 0;
    for _ in 0..8192 {
        if warm.world_paint().unwrap().at_tile_cut(end) {
            break;
        }
        let a = warm.advance_world_geometry_child_with_controls(vm, controls)?;
        let z = restored.advance_world_geometry_child_with_controls(&rvm, &rc)?;
        ensure!(
            a == z && warm == restored,
            "alternate side save/resume differs"
        );
        ensure!(a, "alternate side cold continuation stalled");
        steps += 1;
    }
    ensure!(
        warm.world_paint().unwrap().at_tile_cut(end),
        "alternate side cold endpoint absent"
    );
    ensure!(
        *s == rs
            && *v == rv
            && *b == rb
            && *controls == rc
            && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "alternate side cold immutable owners differ"
    );
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = sims_fixtures::BlobRef {
        sha256: "477f0d8b92242db39c18abc4e4a58913fbb9e31f78b552f84473535338470e10".into(),
        bytes: 9744593,
    };
    let (mut p, s, v, vm, b, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-geometry-active-20261101-first/cold-second.store")
            }),
    )?
    .get_value(&input)?;
    let native = std::fs::read(
        std::env::var_os("SIMS_DIAG_NATIVE").context("alternate side trace absent")?,
    )?;
    ensure!(
        sims_fixtures::digest(&native)
            == "739c70896cb1adb8a8bbe130cde66c752ea0f171e2d1983261a689bd0cb69f3a",
        "alternate side trace differs"
    );
    let side_source = std::fs::read(
        root.join("fixtures/private/goth-side-alt-inputs-20261101-first/depth-third.json"),
    )?;
    let color = std::fs::read(
        root.join("fixtures/private/goth-side-alt-inputs-20261101-first/coverage-first.json"),
    )?;
    let mask = std::fs::read(
        root.join("fixtures/private/goth-side-alt-inputs-20261101-first/color-second.json"),
    )?;
    p.bind_world_geometry_side_alternate(&side_source, &color, &mask)?;
    let mut rows = vec![];
    let mut started = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53ed7b && !started {
            let raw = sims_runtime::original_session::completed_animations::bytes(
                q["geometry_prefix"]["frame"]["raw"]
                    .as_str()
                    .context("alternate parent frame absent")?,
            )?;
            if raw[0x178 - 0x1c..0x178 - 0x19] == [22, 20, 1] {
                started = true;
            }
        }
        if !started {
            continue;
        }
        if matches!(pc, 0x4b4513 | 0x492b67) {
            let raw = sims_runtime::original_session::completed_animations::bytes(
                q["visible_geometry"]["stack"]["raw"]
                    .as_str()
                    .context("alternate helper stack absent")?,
            )?;
            let ret = u32::from_le_bytes(raw[..4].try_into()?);
            if !matches!(ret, 0x4c8cf2 | 0x4c8c62 | 0x4c8ce8) {
                continue;
            }
        }
        if matches!(
            pc,
            0x53ed7b
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
                | 0x53e1f7
                | 0x53e1fc
                | 0x53e23e
                | 0x53e243
                | 0x53e246
                | 0x53e24b
                | 0x53e24d
                | 0x53e2f5
                | 0x53e2fa
                | 0x53e35b
                | 0x53e360
                | 0x53e362
                | 0x4c8b1f
                | 0x4c8e0d
                | 0x4c8b4e
                | 0x4c8b5a
                | 0x4c19b1
                | 0x4c8b78
                | 0x4c8b84
                | 0x4c8bab
                | 0x4c7fdb
                | 0x4c76d1
                | 0x4b954a
                | 0x4c77a1
                | 0x4c8bb0
                | 0x4c8be7
                | 0x4c77bc
                | 0x4c8bec
                | 0x4c8c5d
                | 0x492b67
                | 0x4c8c62
                | 0x4c8c69
                | 0x4c8c6e
                | 0x4c8cc5
                | 0x4c8ccb
                | 0x4b444f
                | 0x4c8cd2
                | 0x4b4456
                | 0x4c8cdd
                | 0x4c8ce8
                | 0x4b4513
                | 0x4c8cf2
                | 0x4c8d32
                | 0x49ecbc
                | 0x4c8d37
                | 0x53e3b9
        ) {
            rows.push(q);
        }
        if pc == 0x53e3b9 {
            break;
        }
    }
    let end = rows.last().context("alternate side observations absent")?;
    ensure!(end["pc"] == 0x53e3b9u32, "alternate side endpoint differs");
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("alternate side output absent")?,
    )?;
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?);
    let (mut cuts, mut checks, mut steps, mut current) = (vec![], vec![], 0, 0);
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..8192 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child_with_controls(&vm, &controls)?,
                    "alternate side stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            if current == 0x53deb2 {
                p.start_world_geometry_bound_side()?;
            }
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(
                current,
                0x53e1f7 | 0x4c8b1f | 0x49ecbc | 0x4c8d37 | 0x53e3b9
            ) {
                cuts.push(cold(&store, &p, &s, &v, &vm, &b, end, &controls)?);
            }
            if current == 0x49ecbc {
                for (n, label) in [
                    (2, "after_first_depth_store"),
                    (1, "after_first_color_store"),
                ] {
                    for _ in 0..n {
                        ensure!(
                            p.advance_world_geometry_child_with_controls(&vm, &controls)?,
                            "alternate store suspension stalled"
                        );
                        steps += 1;
                    }
                    let mut cut = cold(&store, &p, &s, &v, &vm, &b, end, &controls)?;
                    cut["label"] = label.into();
                    cuts.push(cut);
                }
            }
        }
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?),
            "alternate side changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"completed_side_renderer":result.is_ok(),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b,&controls))?),"observed_call_arguments_used_as_inputs":false,"source_admission":false,"native_parity_credit":0,"scope":"Computed next descriptor and alternate side5120/6144 frame9 through both actual target planes. Further model2 segments, full vector/Session/CPU/device and32 completed ticks remain pending."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("alternate side report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
