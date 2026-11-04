//! Execute both original active-wall passes from the computed C3 continuation.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::{interface::state::State as Controls, scheduler::clock::profiler::Buffer};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
};
type Base = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
);
type Saved = (
    State,
    WorldScene,
    WorldComposition,
    sims_sim::Simulation,
    Buffer,
    Controls,
);
fn selected(pc: u32) -> bool {
    matches!(
        pc,
        0x53e968
            | 0x53ea46
            | 0x53ea4b
            | 0x53eaba
            | 0x53eabf
            | 0x53ebfb
            | 0x53ecac
            | 0x53ecb1
            | 0x53ecdf
            | 0x53ece4
            | 0x53ed40
            | 0x53ed45
            | 0x53ed47
            | 0x53ed4c
            | 0x53ed5d
            | 0x53ed7b
            | 0x53ebb5
            | 0x53ebba
            | 0x4c79ca
            | 0x4c8e0d
            | 0x4c7a0e
            | 0x4c7a1a
            | 0x4c19b1
            | 0x4c7a41
            | 0x4c7a4d
            | 0x4c7fdb
            | 0x4c76d1
            | 0x4b954a
            | 0x4c77a1
            | 0x4c7a79
            | 0x4c77bc
            | 0x4c7ae9
            | 0x4c7b98
            | 0x4c78c5
            | 0x4c7f75
            | 0x4c4907
            | 0x4c7f7a
    )
}
fn cold(
    store: &sims_fixtures::Store,
    p: &State,
    s: &WorldScene,
    v: &WorldComposition,
    vm: &sims_sim::Simulation,
    b: &Buffer,
    c: &Controls,
    end: &serde_json::Value,
    label: &str,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b, c))?;
    let (mut restored, rs, rv, rvm, rb, rc): Saved = store.get_value(&blob)?;
    let mut warm = p.clone();
    let mut steps = 0;
    for _ in 0..16384 {
        if warm.world_paint().unwrap().at_tile_cut(end) {
            break;
        }
        let a = warm.advance_world_geometry_child_with_controls(vm, c)?;
        let z = restored.advance_world_geometry_child_with_controls(&rvm, &rc)?;
        ensure!(
            a == z && warm == restored,
            "active wall suspended continuation differs"
        );
        ensure!(a, "active wall suspended continuation stalled");
        steps += 1;
    }
    ensure!(
        warm.world_paint().unwrap().at_tile_cut(end),
        "active wall cold endpoint absent"
    );
    ensure!(
        *s == rs
            && *v == rv
            && *b == rb
            && *c == rc
            && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "active wall cold shared owners differ"
    );
    Ok(serde_json::json!({"label":label,"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let work = root.join("fixtures/private/goth-geometry-active-20261101-first");
    let input = sims_fixtures::BlobRef {
        sha256: "5011bc8d7bcf1683bad6c0008df00ad5d1e3e793bed62b619b10baf20135435b".into(),
        bytes: 9723727,
    };
    let (mut p, s, v, vm, b): Base = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-geometry-side-20261101-first/tail-second.store")
            }),
    )?
    .get_value(&input)?;
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("active wall trace absent")?)?;
    ensure!(
        sims_fixtures::digest(&native)
            == "600994910da9cdac23bf0cb9fc89ef531853f71876cfcd8e4775d997a7931384",
        "active wall trace differs"
    );
    let baseline=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00");
    let state_raw = std::fs::read(baseline.join("observation.json.gz"))?;
    ensure!(
        sims_fixtures::digest(&state_raw)
            == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
        "active controls baseline differs"
    );
    let state: serde_json::Value =
        serde_json::from_reader(flate2::read::GzDecoder::new(state_raw.as_slice()))?;
    let memory = std::fs::read(baseline.join("acknowledged.mem"))?;
    let controls = sims_runtime::original_session::completed_controls::captured::import(
        &state,
        &sims_fixtures::readset::Readset::parse(&memory)?,
    )?
    .controls;
    for (source, material) in [
        ("raster-edge-first.json", "material-edge-first.json"),
        ("raster-cover-second.json", "material-cover-second.json"),
        ("raster-overlay-third.json", "material-overlay-third.json"),
    ] {
        p.bind_world_geometry_raster(
            &std::fs::read(work.join(source))?,
            &std::fs::read(work.join(material))?,
        )?;
    }
    let material = std::fs::read(root.join(
        "fixtures/private/goth-geometry-junction-20261101-first/material-source-first.json",
    ))?;
    let mut rows = vec![];
    let mut started = false;
    let mut returns = 0;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53e968 {
            let f = sims_runtime::original_session::completed_animations::bytes(
                q["geometry_prefix"]["frame"]["raw"]
                    .as_str()
                    .context("active parent frame absent")?,
            )?;
            if f[0x178 - 0x1c..0x178 - 0x19] == [22, 20, 1] {
                started = true;
            }
        }
        if !started {
            continue;
        }
        if pc == 0x53ed4c {
            returns += 1;
        }
        if selected(pc as u32) {
            rows.push(q);
        }
        if pc == 0x53ed7b {
            break;
        }
    }
    ensure!(
        returns == 2 && rows.last().is_some_and(|q| q["pc"] == 0x53ed7bu32),
        "original active wall did not complete both passes"
    );
    let end = rows.last().unwrap();
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("active wall output store absent")?,
    )?;
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?);
    let (mut checks, mut cuts, mut steps, mut current, mut renderers) = (vec![], vec![], 0, 0, 0);
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..16384 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child_with_controls(&vm, &controls)?,
                    "active wall stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            if current == 0x53eaba {
                p.bind_world_geometry_materials(&material)?;
            }
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(current, 0x4c79ca | 0x53ed4c | 0x53ebb5 | 0x53ed7b) {
                cuts.push(cold(
                    &store,
                    &p,
                    &s,
                    &v,
                    &vm,
                    &b,
                    &controls,
                    end,
                    &format!("pass{renderers}-at{current:x}"),
                )?);
            }
            if current == 0x4c4907 && renderers == 0 {
                for (n, label) in [
                    (2, "after_first_depth_store"),
                    (1, "after_first_color_store"),
                ] {
                    for _ in 0..n {
                        ensure!(
                            p.advance_world_geometry_child_with_controls(&vm, &controls)?,
                            "active wall store suspension stalled"
                        );
                        steps += 1;
                    }
                    cuts.push(cold(&store, &p, &s, &v, &vm, &b, &controls, end, label)?);
                }
            }
            if current == 0x53ed4c {
                renderers += 1;
                println!("ACTIVE_RENDERER_RETURN {renderers}");
            }
        }
        ensure!(renderers == 2, "Rust active wall did not return twice");
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?),
            "active wall mutated enclosing VM/RNG/scene/profiler/controls"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"controls_B_source_sha256":sims_fixtures::digest(&state_raw),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"renderers_returned":renderers,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"observed_call_arguments_used_as_inputs":false,"shared_VM_RNG_scene_profiler_controls_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b,&controls))?),"source_admission":false,"native_parity_credit":0,"scope":"Computed both original active-wall material passes, their C4 children and coupled pixel planes. Later segments, full model/vector/Session/CPU/device and32 completed ticks remain pending."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("active wall report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
