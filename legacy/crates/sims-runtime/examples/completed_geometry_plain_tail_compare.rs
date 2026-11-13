//! Retain the ordinary wall return, clip publications and next side-pass frontier.
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
        ensure!(a == z && warm == restored, "plain tail save/resume differs");
        ensure!(a, "plain tail cold continuation stalled");
        steps += 1;
    }
    ensure!(
        warm.world_paint().unwrap().at_tile_cut(end),
        "plain tail cold endpoint absent"
    );
    ensure!(
        *s == rs && *v == rv && *b == rb && serde_json::to_vec(vm)? == serde_json::to_vec(&rvm)?,
        "plain tail cold immutable owners differ"
    );
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let work = root.join("fixtures/private/goth-geometry-junction-20261101-first");
    let input = sims_fixtures::BlobRef {
        sha256: "fff3c9a9c639c217b5b4938e6c996e29d0900be41634d4eb2ba56e2318e4b7fc".into(),
        bytes: 9793148,
    };
    let (mut p, s, v, vm, b): Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE")
            .map(PathBuf::from)
            .unwrap_or_else(|| work.join("cold-second.store")),
    )?
    .get_value(&input)?;
    let native =
        std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("plain tail trace absent")?)?;
    ensure!(
        sims_fixtures::digest(&native)
            == "600994910da9cdac23bf0cb9fc89ef531853f71876cfcd8e4775d997a7931384",
        "plain tail trace differs"
    );
    let mut rows = vec![];
    let mut started = false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if q["kind"] != "world_drawing_setup" || q["ordinal"] != 2 {
            continue;
        }
        let Some(pc) = q["pc"].as_u64() else { continue };
        if pc == 0x53ed4c {
            started = true
        }
        if started
            && matches!(
                pc,
                0x53ed4c
                    | 0x53ed7b
                    | 0x53dd65
                    | 0x53dd6a
                    | 0x53dd8e
                    | 0x53dd93
                    | 0x53de18
                    | 0x53de1d
                    | 0x53de78
                    | 0x53de7d
                    | 0x53de8a
                    | 0x53de8f
                    | 0x53dead
                    | 0x53deb2
            )
        {
            rows.push(q)
        }
        if started && pc == 0x53deb2 {
            break;
        }
    }
    let end = rows.last().context("plain tail observations absent")?;
    ensure!(end["pc"] == 0x53deb2u32, "plain tail endpoint differs");
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("plain tail output absent")?,
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
                    "plain tail stalled at{current:x}"
                );
                steps += 1;
            }
            p.compare_world_tiles(q)?;
            p.compare_world_shared_graphics(q)?;
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            if matches!(current, 0x53ed4c | 0x53dd93 | 0x53deb2) {
                cuts.push(cold(&store, &p, &s, &v, &vm, &b, end)?)
            }
        }
        let mut pending = p.clone();
        let error = pending
            .advance_world_geometry_child(&vm)
            .expect_err("the unimplemented side child must stay explicit");
        ensure!(
            error.to_string()=="geometry side polygon branch is pending",
            "unexpected next side frontier: {error:#}"
        );
        ensure!(
            immutable == sims_fixtures::digest(&serde_json::to_vec(&(&s, &v, &vm, &b))?),
            "plain tail changed VM/RNG/scene/profiler"
        );
        Ok(())
    })();
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"computed_microsteps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"pending_original_pc":0x53e1f7u32,"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"source_admission":false,"native_parity_credit":0,"scope":"Computed wall return, ordered clip restoration and next descriptor/classification only. Nonzero side rendering remains pending; this is not a model/vector/Session or completed-tick proof."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("plain tail report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]})
    );
    result
}
