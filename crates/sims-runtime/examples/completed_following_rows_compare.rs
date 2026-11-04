//!Replay successive original ordinary rows, stopping at the first unavailable
//!child. No observed result becomes an implementation input.
use anyhow::{Context, Result, ensure};
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
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input = std::env::var("SIMS_DIAG_INPUT_REF").ok().map(|s|serde_json::from_str::<sims_fixtures::BlobRef>(&s)).transpose()?.unwrap_or(sims_fixtures::BlobRef {
        sha256: "e08d186dba4b8c26a2b37d72908824a3241e5e94195dc86febb959354c3b208f".into(),
        bytes: 10360802,
    });
    let after=std::env::var("SIMS_DIAG_AFTER_ROW").ok().map(|s|s.parse::<u64>()).transpose()?.unwrap_or(15);
    let (mut p, mut scene, view, mut vm, mut profiler, controls, mut classify): Saved =
        sims_fixtures::Store::open(
            std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-object-sprite-frame-20261101-first/row-full.store")),
        )?
        .get_value(&input)?;
    if let Some(file)=std::env::var_os("SIMS_DIAG_OVERRIDE_CATALOG"){
        if p.world_paint().and_then(|w|w.row_prefix()).is_some_and(|r|r.object_catalog_bound()){
            p.bind_world_override_catalog(&std::fs::read(file)?,&std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?,&vm)?;
        }
    }
    if let Some(file)=std::env::var_os("SIMS_DIAG_RASTER_FAMILY"){p.bind_world_raster_family(&std::fs::read(file)?)?;}
    let prefix = std::fs::read(
        root.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"),
    )?;
    let source = std::fs::read(
        root.join("fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json"),
    )?;
    let trace = std::fs::read(std::env::var_os("SIMS_DIAG_SPAN_TRACE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-ordinary-rows-witness-20261101-first/post-level-trace.jsonl.gz")))?;
    let through=std::env::var("SIMS_DIAG_THROUGH_ROW").ok().map(|v|v.parse::<u64>()).transpose()?;
    let mut rows = vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines() {
        let q: serde_json::Value = serde_json::from_str(&line?)?;
        if through.is_none_or(|last|q["row_number"].as_u64().is_some_and(|r|r<=last))&&q["observer"] == "ordinary_rows"
            && q["row_number"]
                .as_u64()
                .is_some_and(|r| r > after || r == after && q["pc"] == 0x4ad6abu32)
            && q["pc"]
                .as_u64()
                .is_some_and(|pc| matches!(pc, 0x4ad8ac | 0x4ad6ab | 0x4ad885 | 0x4ad368))
        {
            rows.push(q)
        }
    }
    ensure!(!rows.is_empty(), "ordinary row span evidence absent");
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("span store absent")?,
    )?;
    let before = serde_json::to_vec(&vm)?;
    let (mut steps, mut ordinal, mut pc, mut returned) = (0, 0, 0, 0);
    let mut checks = vec![];
    let mut snapshots = vec![];
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().unwrap() as u32;
            ordinal = q["row_number"].as_u64().context("span row number absent")?;
            if pc == 0x4ad8ac || pc == 0x4ad368 {
                if p.world_paint()
                    .and_then(|w| w.row_prefix())
                    .is_some_and(|r| r.complete() && !r.acknowledged())
                {
                    p.return_world_row()?;
                }
                for _ in 0..8192 {
                    if p.world_paint().context("span paint absent")?.at_tile_cut(q) {
                        break;
                    }
                    ensure!(
                        p.advance_world_paint(&mut scene, &mut profiler)?,
                        "span caller stopped before row{ordinal}"
                    );
                    steps += 1;
                }
                p.compare_world_tiles(q)?;
                if pc == 0x4ad8ac {
                    p.bind_world_row_prefix(&prefix)?;
                    p.bind_world_row_ordinary(&source)?;
                    let directory = std::fs::read(root.join(
                        "fixtures/private/goth-ordinary-object-20261101-first/object-source.json",
                    ))?;
                    p.bind_world_row_ordinary_objects(&directory)?;
                    if std::env::var_os("SIMS_DIAG_WITH_PHYSICAL").is_some(){p.bind_world_row_ordinary_physical(&directory)?;}
                    if !p
                        .world_paint()
                        .and_then(|w| w.row_prefix())
                        .context("row absent")?
                        .object_catalog_bound()
                    {
                        p.bind_world_object_catalog(&std::fs::read(root.join("fixtures/private/goth-following-rows-20261101-first/resident-catalog-source.json"))?,&directory,&vm)?;
                        if std::env::var_os("SIMS_DIAG_WITH_SLOTS").is_some(){p.bind_world_object_slots(&directory)?;}
                        if std::env::var_os("SIMS_DIAG_WITH_OVERRIDES").is_some(){p.bind_world_override_catalog(&std::fs::read(std::env::var_os("SIMS_DIAG_OVERRIDE_CATALOG").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-object-visibility-20261101-first/override-catalog-source.json")))?,&directory,&vm)?;}
                        if std::env::var_os("SIMS_DIAG_WITH_RASTER").is_some(){p.bind_world_object_raster(&std::fs::read(root.join("fixtures/private/goth-visible-sprite-20261101-first/raster-source-second.json"))?)?;}
                        if std::env::var_os("SIMS_DIAG_WITH_SPR2_BATCH").is_some(){p.bind_world_object_raster_batch(&std::fs::read(root.join("fixtures/private/goth-spr2-mmx-20261101-first/batch-source-first.json"))?)?;}
                        if std::env::var_os("SIMS_DIAG_WITH_SHADOW").is_some(){p.bind_world_object_shadow(&std::fs::read(root.join("fixtures/private/goth-object-shadow-20261101-first/shadow-source-first.json"))?)?;}
                    }
                    p.bind_world_row_visibility(&view)?;
                    p.compare_world_row_prefix(q)?;
                }
            } else {
                for _ in 0..65536 {
                    if p.world_paint()
                        .and_then(|w| w.row_prefix())
                        .context("following row absent")?
                        .at_native_observation(q)?
                    {
                        break;
                    }
                    if !p.advance_world_row_objects_mut(&mut vm, &mut classify)? {
                        break;
                    }
                    steps += 1;
                }
                p.compare_world_row_prefix(q)?;
            }
            p.compare_world_shared_graphics(q)?;
            if matches!(pc,0x4ad6ab|0x4ad885){returned+=1;}
            checks.push(
                serde_json::json!({"row_number":ordinal,"pc":pc,"trace_stop":q["trace_stop"]}),
            );
            if pc == 0x4ad6ab && matches!(ordinal, 1 | 4 | 8 | 16 | 32) {
                let blob =
                    store.put_value(&(&p, &scene, &view, &vm, &profiler, &controls, &classify))?;
                let (rp, rs, rv, rvm, rb, rc, rk): Saved = store.get_value(&blob)?;
                ensure!(
                    p == rp
                        && scene == rs
                        && view == rv
                        && profiler == rb
                        && controls == rc
                        && classify == rk
                        && serde_json::to_vec(&vm)? == serde_json::to_vec(&rvm)?,
                    "span checkpoint differs"
                );
                snapshots.push(serde_json::json!({"row_number":ordinal,"blob":blob}));
            }
        }
        Ok(())
    })();
    ensure!(
        before == serde_json::to_vec(&vm)?,
        "row span mutated VM/RNG"
    );
    let frontier = store.put_value(&(&p, &scene, &view, &vm, &profiler, &controls, &classify))?;
    let pending = p
        .world_paint()
        .and_then(|w| w.row_prefix())
        .and_then(|r| r.pending());
    let detail = if result.is_err() {
        let raw = serde_json::to_value(p.world_paint().and_then(|w| w.row_prefix()))?;
        let c = &raw["ordinary"];
        let id = c["selected_id"].as_i64().or_else(||c["head"].as_i64()).map(|n| n as i16);
        Some(
            serde_json::json!({"stage":c["stage"],"draw_stage":c["draw_cursor"]["stage"],"sprite":c["draw_cursor"]["sprite"],"point":c["point"],"rotated":c["rotated"],"index":c["index"],"screen":c["screen"],"head":id,"selected":c["selected"],"definition":id.and_then(|id|vm.vm.world.objects.get(&id)).and_then(|o|o.definition.clone()),"draw_state":id.and_then(|id|vm.vm.world.refresh.as_ref().and_then(|r|r.draw.get(&id))),"slots":id.and_then(|id|vm.vm.world.objects.get(&id)).map(|o|o.slots.len()),"row_request":raw["request"]}),
        )
    } else {
        None
    };
    let report = serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"returned_rows":returned,"saved_cuts":snapshots,"frontier":frontier,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"row_number":ordinal,"pc":pc,"message":format!("{e:#}"),"pending":format!("{pending:?}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"frontier_detail":detail,"scope":"Selected natural ordinary-row entries/returns and coupled state; first unsupported child is preserved, never acknowledged by a stub. No completed-tick admission."});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("span report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"returned_rows":returned,"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
