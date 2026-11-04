//! Advance the retained draw into remaining original models. Missing services
//! are explicit frontiers; partial observations never earn completed-tick credit.
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
const NATIVE: &str = "d4c67f9b8b5b7daed141d9459d05086be72cc401e17c1319610a4749e3ea613e";
fn selected(pc: u32) -> bool {
    matches!(
        pc,
        0x53c227
            | 0x53db67
            | 0x540480
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
            | 0x53e3b9
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
            | 0x53ed47
            | 0x4c79ca
            | 0x53ed4c
            | 0x53ed5d
            | 0x53ed7b
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
            | 0x4c8878
            | 0x4c8884
            | 0x4c88a0
            | 0x4c88ac
            | 0x4c88d4
            | 0x4c8910
            | 0x4c89f9
            | 0x4c9e38
            | 0x4c8a79
            | 0x4c8aab
            | 0x49f2a3
            | 0x4c8afa
    )
}
fn cold_entry(
    store: &sims_fixtures::Store,
    p: &State,
    s: &WorldScene,
    v: &WorldComposition,
    vm: &sims_sim::Simulation,
    b: &Buffer,
    c: &sims_vm::interface::state::State,
) -> Result<serde_json::Value> {
    let blob = store.put_value(&(p, s, v, vm, b, c))?;
    let (mut restored, rs, rv, rvm, rb, rc): Saved = store.get_value(&blob)?;
    ensure!(
        p == &restored
            && *s == rs
            && *v == rv
            && *b == rb
            && *c == rc
            && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "catalog cold owners differ"
    );
    let mut warm = p.clone();
    let mut steps = 0;
    let mut stopped = false;
    for _ in 0..128 {
        let a = warm.advance_world_geometry_child_with_scene(vm, c, s);
        let z = restored.advance_world_geometry_child_with_scene(&rvm, &rc, &rs);
        ensure!(
            warm == restored,
            "catalog suspended state processing differs"
        );
        match (a, z) {
            (Ok(x), Ok(y)) => {
                ensure!(x == y, "catalog continuation progress differs");
                if !x {
                    stopped = true;
                    break;
                }
                steps += 1
            }
            (Err(x), Err(y)) => {
                ensure!(
                    format!("{x:#}") == format!("{y:#}"),
                    "catalog explicit pending service differs"
                );
                stopped = true;
                break;
            }
            _ => anyhow::bail!("catalog save/resume result differs"),
        }
    }

    Ok(
        serde_json::json!({"blob":blob,"resumed_microsteps":steps,"exact_state_and_service_result_equal":true,"stopped_at_explicit_service":stopped}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = sims_fixtures::BlobRef {
        sha256: "ab73530fa1c727b58571b99bf48336b3d91db7d2d23162ce62144c5009f082ee".into(),
        bytes: 10302667,
    };
    let (mut p, mut scene, view, vm, mut profiler, controls): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join("fixtures/private/goth-portal-selector-20261101-first/cold-first.store")
            }),
    )?
    .get_value(&input)?;
    let work = root.join("fixtures/private/goth-portal-selector-20261101-first");
    let meta = std::fs::read(work.join("catalog-third.json"))?;
    let raw = std::fs::read(work.join("catalog-third.mem"))?;
    for change_meta in [true, false] {
        let mut bad_meta = meta.clone();
        let mut bad_raw = raw.clone();
        if change_meta {
            bad_meta[0] ^= 1
        } else {
            bad_raw[0] ^= 1
        };
        let mut copy = p.clone();
        ensure!(
            copy.bind_world_geometry_catalog(&bad_meta, &bad_raw)
                .is_err()
                && copy == p,
            "catalog tampering changed retained state"
        );
    }
    let source = std::fs::read(root.join(
        "fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json",
    ))?;
    let native = std::fs::read(
        std::env::var_os("SIMS_DIAG_NATIVE").context("remaining native trace absent")?,
    )?;
    ensure!(
        sims_fixtures::digest(&native) == NATIVE,
        "remaining native trace differs"
    );
    let mut rows = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup"
            || q["trace_stop"].as_u64().is_none_or(|i| i < 5885)
            || (q["pc"]!=0x53c227&&q["ordinal"].as_u64().is_none_or(|i| i < 21))
        {
            continue;
        }
        let Some(pc) = q["pc"].as_u64().map(|v| v as u32) else {
            continue;
        };
        if !selected(pc) {
            continue;
        }
        if pc == 0x466fcc
            && !q["geometry_prefix"]["stack"]["raw"]
                .as_str()
                .is_some_and(|s| s.starts_with("d2db5300"))
        {
            continue;
        }
        if pc == 0x688d82c0
            && !q["geometry_prefix"]["stack"]["raw"]
                .as_str()
                .is_some_and(|s| s.starts_with("c2e85300"))
        {
            continue;
        }
        rows.push(q);
    }
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("remaining output absent")?,
    )?;
    let mut checks = vec![];
    let mut endpoint_checks=vec![];
    let (mut steps, mut current, mut ordinal, mut returned) = (0, 0, None, 0);
    let mut cuts = vec![];
    let vm_before = serde_json::to_vec(&vm)?;
    let result = (|| -> Result<()> {
        for q in &rows {
            current = q["pc"].as_u64().unwrap() as u32;
            ordinal = q["ordinal"].as_u64();
            for _ in 0..32768 {
                if p.world_paint().unwrap().at_tile_cut(q) {
                    break;
                }
                let progress = if matches!(current,0x53db67|0x53c227) {
                    p.advance_world_paint(&mut scene, &mut profiler)?
                } else {
                    p.advance_world_geometry_child_with_scene(&vm, &controls, &scene)?
                };
                ensure!(progress, "remaining model{ordinal:?} stalled at{current:x}");
                steps += 1;
            }
            if current == 0x53db67 {
                p.bind_world_geometry_child(&source)?;
            }
            if current == 0x53dcc4 {
                p.bind_world_geometry_segments_from_scene(&source, &scene)?;
            }
            if current == 0x53e8f8 {
                p.bind_world_geometry_visible(&source)?;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            if current==0x53e84d&&matches!(ordinal,Some(21)|Some(22)) {
                let raw=q["geometry_prefix"]["frame"]["raw"].as_str().context("endpoint frame absent")?;
                let mut bytes=(0..raw.len()).step_by(2).map(|i|u8::from_str_radix(&raw[i..i+2],16)).collect::<std::result::Result<Vec<_>,_>>()?;
                let native_mask=bytes[0x178-0xa1];bytes[0x178-0xa1]^=1;
                let mut bad=q.clone();bad["geometry_prefix"]["frame"]["raw"]=bytes.iter().map(|b|format!("{b:02x}")).collect::<String>().into();
                let error=p.compare_world_tiles(&bad).expect_err("wrong endpoint helper result was accepted");
                ensure!(format!("{error:#}").contains("geometry diagonal endpoint helper result differs"),"endpoint tampering failed at an unrelated field");
                endpoint_checks.push(serde_json::json!({"ordinal":ordinal,"native_endpoint_mask":native_mask,"wrong_mask_rejected":true}));
            }
            checks.push(
                serde_json::json!({"pc":current,"ordinal":ordinal,"trace_stop":q["trace_stop"]}),
            );
            if current == 0x53db67 {
                let mut cut = cold_entry(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
                cut["pc"] = current.into();
                cut["ordinal"] = serde_json::to_value(ordinal)?;
                cuts.push(cut);
            }
            if matches!(current, 0x49f2a3 | 0x53e3b9 | 0x53e84d) {
                let mut cut = cold_entry(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
                cut["pc"] = current.into();
                cut["ordinal"] = serde_json::to_value(ordinal)?;
                cuts.push(cut);
            }
            if matches!(current, 0x53ed4c | 0x540480) {
                cuts.push(serde_json::json!({"pc":current,"ordinal":ordinal,"blob":store.put_value(&(&p,&scene,&view,&vm,&profiler,&controls))?}));
            }
            if current == 0x540480 {
                returned += 1;
            }
        }
        ensure!(returned == 7, "remaining geometry models not all returned");
        ensure!(current==0x53c227,"first-vector iterator did not reach the second-vector call");
        cuts.push(serde_json::json!({"pc":current,"ordinal":null,"blob":store.put_value(&(&p,&scene,&view,&vm,&profiler,&controls))?}));
        let before=p.clone();let mut copy=p.clone();
        ensure!(copy.bind_world_geometry_child(&source).is_err()&&copy==before,"second-vector import fabricated a first-vector child");
        Ok(())
    })();
    ensure!(
        vm_before == serde_json::to_vec(&vm)?,
        "geometry diagnostic changed VM/RNG"
    );
    let frontier_save_resume = cold_entry(&store, &p, &scene, &view, &vm, &profiler, &controls)?;
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"endpoint_helper_checks":endpoint_checks,"second_vector_entry_compared":current==0x53c227&&result.is_ok(),"computed_microsteps":steps,"returned_models":returned,"saved_cuts":cuts,"frontier_save_resume":frontier_save_resume,"catalog_tampering_rejected_without_state_changes":true,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"ordinal":ordinal,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"observed_call_arguments_used_as_inputs":false,"scope":"Remaining first-vector projection with original captured model/segment/caller and coupled target-plane checks. Unsupported services remain explicit. The remaining first-vector children and iterator handoff are checked; second-vector execution remains unsupported. This is not complete CPU/Session admission or32 completed ticks."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("remaining report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"comparisons":report["comparison_boundaries"].as_array().unwrap().len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
