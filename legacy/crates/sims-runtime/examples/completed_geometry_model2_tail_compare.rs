//! Continue the retained original model2 through its remaining three segments.
use anyhow::{ensure, Context, Result};
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
    for _ in 0..32768 {
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
fn point(q: &serde_json::Value) -> Result<[u8; 3]> {
    let f = sims_runtime::original_session::completed_animations::bytes(
        q["geometry_prefix"]["frame"]["raw"]
            .as_str()
            .context("model2 frame absent")?,
    )?;
    Ok(f[0x178 - 0x1c..0x178 - 0x19].try_into()?)
}
fn selected(pc: u32) -> bool {
    matches!(
        pc,
        0x492b67
            | 0x49ecbc
            | 0x49f2a3
            | 0x4b444f
            | 0x4b4456
            | 0x4b4513
            | 0x4b954a
            | 0x4c19b1
            | 0x4c4907
            | 0x4c76d1
            | 0x4c77a1
            | 0x4c77bc
            | 0x4c78c5
            | 0x4c79ca
            | 0x4c7a0e
            | 0x4c7a1a
            | 0x4c7a41
            | 0x4c7a4d
            | 0x4c7a79
            | 0x4c7ae9
            | 0x4c7b98
            | 0x4c7f75
            | 0x4c7f7a
            | 0x4c7fdb
            | 0x4c8849
            | 0x4c8878
            | 0x4c8884
            | 0x4c88a0
            | 0x4c88ac
            | 0x4c88d4
            | 0x4c8910
            | 0x4c89f9
            | 0x4c8a79
            | 0x4c8aab
            | 0x4c8afa
            | 0x4c8b1f
            | 0x4c8b4e
            | 0x4c8b5a
            | 0x4c8b78
            | 0x4c8b84
            | 0x4c8bab
            | 0x4c8bb0
            | 0x4c8be7
            | 0x4c8bec
            | 0x4c8c5d
            | 0x4c8c62
            | 0x4c8c69
            | 0x4c8c6e
            | 0x4c8cc5
            | 0x4c8ccb
            | 0x4c8cd2
            | 0x4c8cdd
            | 0x4c8ce8
            | 0x4c8cf2
            | 0x4c8d32
            | 0x4c8d37
            | 0x4c8e0d
            | 0x4c9e38
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
            | 0x53e3ad
            | 0x53e3b4
            | 0x53e3b9
            | 0x53e84d
            | 0x53e852
            | 0x53e861
            | 0x53e866
            | 0x53e8bc
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
            | 0x53eb1b
            | 0x53ebb5
            | 0x53ebba
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
            | 0x540480
            | 0x2f12874
            | 0x688d82c0
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = sims_fixtures::BlobRef {
        sha256: "f6b7d506db615534f5d32cd5a40bb5859b7f17ea8be0d20b6be20721e77a76da".into(),
        bytes: 9752528,
    };
    let (mut p, s, v, vm, b, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-side-alt-20261101-first/cold-third.store")
            }),
    )?
    .get_value(&input)?;
    for image in [3, 14, 18, 1788] {
        let base = root.join("fixtures/private/goth-side-alt-inputs-20261101-first");
        p.bind_world_geometry_raster(
            &std::fs::read(base.join(format!("next-raster-{image}-first.json")))?,
            &std::fs::read(base.join(format!("next-material-{image}-second.json")))?,
        )?;
    }
    let model = std::fs::read(root.join(
        "fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json",
    ))?;
    let material = std::fs::read(root.join(
        "fixtures/private/goth-geometry-junction-20261101-first/material-source-first.json",
    ))?;
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("model2 native trace absent")?)?;
    ensure!(
        sims_fixtures::digest(&native)
            == "600994910da9cdac23bf0cb9fc89ef531853f71876cfcd8e4775d997a7931384",
        "model2 trace differs"
    );
    let mut rows = vec![];
    let mut started = false;
    let mut in_side = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64().map(|v| v as u32) else {
            continue;
        };
        if pc == 0x53e3b9 && point(&q)? == [23, 20, 1] {
            started = true
        }
        if pc == 0x53e1f7 {
            in_side = true;
        }
        if pc == 0x53e3b9 {
            in_side = false;
        }
        //This older witness has caller cuts but no S4 frame interiors.
        if in_side && !(0x53e1f7..=0x53e3b9).contains(&pc) {
            continue;
        }
        if pc == 0x688d82c0
            && !q["geometry_prefix"]["stack"]["raw"]
                .as_str()
                .is_some_and(|s| s.starts_with("c2e85300"))
        {
            continue;
        }
        if started && selected(pc) {
            rows.push(q)
        }
        if started && pc == 0x540480 {
            break;
        }
    }
    ensure!(
        rows.first().is_some_and(|q| q["pc"] == 0x53e3b9u32)
            && rows.last().is_some_and(|q| q["pc"] == 0x540480u32),
        "model2 boundaries absent"
    );
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("model2 output absent")?,
    )?;
    let immutable = sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?);
    let (mut checks, mut cuts, mut steps, mut current, mut renderers) = (vec![], vec![], 0, 0, 0);
    let result = (|| -> Result<()> {
        for (index, q) in rows.iter().enumerate() {
            current = q["pc"].as_u64().unwrap() as u32;
            for _ in 0..32768 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                ensure!(
                    p.advance_world_geometry_child_with_controls(&vm, &controls)?,
                    "model2 stalled at{current:x}"
                );
                steps += 1;
            }
            if current == 0x53e8f8 {
                p.bind_world_geometry_visible(&model)?;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            if current == 0x53de8f {
                p.start_world_geometry_last_side_if_pending()?;
            }
            if current == 0x53deb2 {
                p.start_world_geometry_bound_side()?;
            }
            if current == 0x53eaba {
                p.bind_world_geometry_materials(&material)?;
            }
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if current == 0x53ed4c {
                renderers += 1;
            }
            let endpoint = if current == 0x53e1f7 {
                rows[index..].iter().find(|e| e["pc"] == 0x53e3b9u32)
            } else if matches!(current, 0x4c79ca | 0x53ed4c) {
                rows[index..].iter().find(|e| e["pc"] == 0x53ed7bu32)
            } else if current == 0x540480 {
                Some(q)
            } else {
                None
            };
            if let Some(end) = endpoint {
                let mut cut = cold(&store, &p, &s, &v, &vm, &b, end, &controls)?;
                cut["pc"] = current.into();
                cut["trace_stop"] = q["trace_stop"].clone();
                cuts.push(cut);
            }
        }
        ensure!(
            renderers == 4,
            "model2 did not complete four remaining material passes"
        );
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b, &controls))?),
            "model2 changed VM/RNG/scene/profiler/controls"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"renderers_returned":renderers,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b,&controls))?),"observed_call_arguments_used_as_inputs":false,"source_admission":false,"native_parity_credit":0,"scope":"Remaining model2 segments from the computed first S4 return, four C4 returns with coupled original target planes, shared controls and selected cold continuations. Later S4 interiors are not directly sampled here; the completed vector/Session/CPU/device and32 ticks remain pending."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("model2 report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
