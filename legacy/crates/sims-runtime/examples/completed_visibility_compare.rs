//! Original row42 wrapper, temporary visibility stores, physical chain and suspended replay.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{
    State, WorldClassification, WorldComposition, WorldScene,
};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
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
fn trace(path: &Path) -> Result<Vec<serde_json::Value>> {
    let encoded = std::fs::read(path)?;
    BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice()))
        .lines()
        .map(|l| Ok(serde_json::from_str(&l?)?))
        .collect()
}
fn compare_contained_cache(state:&Saved,q:&serde_json::Value)->Result<()>{
    if let Some(id)=q["slot_child_cursor"]["id"].as_i64(){
        let mut observed=q.clone();observed["object_cursor"]=q["slot_child_cursor"].clone();state.6.compare_draw_cache(id as i16,&observed)?;
    }Ok(())
}
fn cold(
    store: &sims_fixtures::Store,
    state: &Saved,
    remaining: &[&serde_json::Value],
    final_vm:&sims_sim::Simulation,
) -> Result<serde_json::Value> {
    let blob = store.put_value(state)?;
    let mut restored: Saved = store.get_value(&blob)?;
    ensure!(
        &restored == state,
        "physical-next cold reconstruction differs"
    );
    let mut warm = state.clone();
    let mut steps = 0;
    for q in remaining{
        for _ in 0..65536{
            if warm.0.world_paint().and_then(|w|w.row_prefix()).context("cold row absent")?.at_native_observation(q)?{break;}
            let a=warm.0.advance_world_row_objects_mut(&mut warm.3,&mut warm.6)?;
            let b=restored.0.advance_world_row_objects_mut(&mut restored.3,&mut restored.6)?;
            ensure!(a&&a==b&&warm.3==restored.3,"cold continuation changed VM/RNG or progress");steps+=1;
        }
        ensure!(warm==restored,"full cold state differs at selected native boundary");
        warm.0.compare_world_row_prefix(q)?;warm.0.compare_world_shared_graphics(q)?;compare_contained_cache(&warm,q)?;
        if q["palette_selector"].is_object()||matches!(q["pc"].as_u64(),Some(0x4bc606|0x4c1ee5|0x4bca26|0x4c1d14)){
            warm.0.world_paint().and_then(|w|w.row_prefix()).context("cold raster owner absent")?.compare_object_raster(q)?;
        }
        if q["spr2_owners"].is_object(){warm.0.world_paint().and_then(|w|w.row_prefix()).context("cold batch owner absent")?.compare_object_raster_batch(q)?;}
    }
    ensure!(
        warm == restored && &warm.3 == final_vm,
        "physical-next resume changed VM/RNG"
    );
    Ok(
        serde_json::json!({"blob":blob,"continuation_steps":steps,"actual_row_return_context_planes_and_VM_match":true,"state_and_services_equal":true,"full_state_native_boundaries":remaining.len(),"VM_and_RNG_checked_every_step":true}),
    )
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let target_row=std::env::var("SIMS_DIAG_TARGET_ROW").ok().map(|s|s.parse::<u64>()).transpose()?.unwrap_or(42);
    let input = std::env::var("SIMS_DIAG_INPUT_REF").ok().map(|s|serde_json::from_str::<sims_fixtures::BlobRef>(&s)).transpose()?.unwrap_or(sims_fixtures::BlobRef {
        sha256: "afa744b9de1efef00e34380e6840b2c0dfce74d22a2b7030003d8c662456e1d4".into(),
        bytes: 15731760,
    });
    let mut state: Saved = sims_fixtures::Store::open(
        std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-mirrored-render-20261101-first/row41-third.store")),
    )?
    .get_value(&input)?;
    let supplement=std::env::var_os("SIMS_DIAG_OVERRIDE_CATALOG").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-object-visibility-20261101-first/override-catalog-source.json"));
    if state.0.world_paint().and_then(|p|p.row_prefix()).is_some_and(|r|r.object_catalog_bound())&&std::env::var_os("SIMS_DIAG_OVERRIDE_CATALOG").is_some(){state.0.bind_world_override_catalog(&std::fs::read(&supplement)?,&std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?,&state.3)?;}
    if let Some(file)=std::env::var_os("SIMS_DIAG_RASTER_FAMILY"){state.0.bind_world_raster_family(&std::fs::read(file)?)?;}
    if let Some(file)=std::env::var_os("SIMS_DIAG_SLOT_RECORDS"){state.0.bind_world_object_slot_records(&std::fs::read(file)?,&std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?)?;}
    if let Some(file)=std::env::var_os("SIMS_DIAG_AUXILIARY_RECORDS"){state.0.bind_world_object_auxiliary(&std::fs::read(file)?,&std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?)?;}
    let prior: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("fixtures/private/goth-mirrored-render-20261101-first/row41-third.json"),
    )?)?;
    let mut pcs: BTreeSet<u64> = prior["checks"]
        .as_array()
        .context("prior PC set absent")?
        .iter()
        .filter_map(|q| q["pc"].as_u64())
        .collect();
    pcs.extend([
        0x4ae9fe,0x4aebde,0x4aebe3,
        0x437cc2,0x437cd1,0x437cd6,0x437d3a,
        0x4aea88,0x4aeb88,0x4aeb9a,0x4aeb9e,0x4aeba3,0x4aebc1,0x4aebc6,0x4aebcd,
        0x463f4c,0x463f5f,0x463f64,0x463fc3,0x46409c,0x4640a4,0x4640a5,0x4640a6,0x4640a7,0x4640ac,
        0x432656, 0x432658, 0x43265d, 0x432823, 0x43283b, 0x432841, 0x40743b, 0x407441, 0x407450,
        0x407455, 0x407458, 0x40745c,
    ]);
    // Every implemented base-draw boundary, including the hidden-sprite branch.
    pcs.extend([0x4378bc,0x4378d3,0x4378d8,0x4378f5,0x437907,0x437922,0x437927,0x43793d,0x437942,0x4379b7,0x4379d5,0x4379da,0x4379ea,0x4379ef,0x437a0b,0x437a10,0x437a76,0x437a7b,0x437ab3,0x437ab8,0x437abd,0x437ac2,0x437ad3,0x437ad8,0x437af2,0x437b50,0x437b55,0x437b68,0x437b6d,0x437c1c,0x437c25,0x437c2a,0x437c56,0x437cbd,0x437cc2,0x437cd1,0x437cd6,0x437d3d,0x437d40,0x437d53,0x437d58,0x437d73,0x437d78,0x437f6a,0x463f4c,0x463f5f,0x463f64,0x463fc3,0x46409c,0x4640a4,0x4640ac,0x4aebbb]);
    pcs.extend([0x4b954a,0x4b9564,0x4b9566,0x4b956e,0x4b9577,0x4b9587,0x4b959b,0x4b95ac,0x4b95b2,0x4c1bc7]);
    pcs.extend([0x437fdd,0x438008,0x43800d,0x438018,0x438024,0x438026,0x438044,0x438049,0x438068,0x43806d,0x43807a,0x43807f,0x438082,0x438087,0x438090,0x438093,0x4380a9,0x4380ae,0x4380b0,0x4380c7,0x4380cc,0x4380ce,0x4380d1,0x4380d6,0x4380de,0x4380e0,0x4380e1,0x4380e9,0x4380ea,0x4380eb,0x4380ec,0x4380f0]);
    pcs.extend([0x4ad885,0x467482,0x4674e4,0x467515,0x467690,0x46754e,0x4671f8,0x467574,0x46767f,0x4adaa2,0x4adaa9]);
    pcs.extend([0x437ce8,0x437cee,0x437cf9,0x437cfd,0x437cff,0x437d07,0x437d0c,0x437d1b,0x437d20,0x437c3f,0x437d26,0x437d29,0x437d2b]);
    let witness = std::env::var_os("SIMS_DIAG_WITNESS").map(PathBuf::from).unwrap_or_else(||root.join(format!("fixtures/private/goth-object-visibility-row{target_row}-witness-20261101-first/post-level-trace.jsonl.gz")));
    let raw = std::fs::read(&witness)?;
    let captured: Vec<_> = trace(&witness)?
        .into_iter()
        .filter(|q| q["observer"] == "ordinary_object_draw" && q["row_number"] == target_row)
        .collect();
    ensure!(!captured.is_empty(), "row42 original evidence absent");
    let unselected: Vec<_> = captured
        .iter()
        .filter(|q| !q["pc"].as_u64().is_some_and(|pc| pcs.contains(&pc)))
        .map(|q| serde_json::json!({"pc":q["pc"],"stop":q["trace_stop"]}))
        .collect();
    let selected: Vec<_> = captured
        .iter()
        .filter(|q| q["pc"].as_u64().is_some_and(|pc| pcs.contains(&pc)))
        .cloned()
        .collect();
    let rows = selected;
    let store = sims_fixtures::Store::create(
        std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("row42 output store absent")?,
    )?;
    let before = serde_json::to_vec(&state.3)?;
    let final_vm=state.3.clone();
    let (mut checks, mut saved) = (Vec::new(), Vec::new());
    let mut readonly_cuts=Vec::new();
    let (mut steps, mut pc, mut stop, mut phase) = (0, 0, 0, "setup");
    let result = (|| -> Result<()> {
        for q in &rows {
            pc = q["pc"].as_u64().context("PC absent")? as u32;
            stop = q["trace_stop"].as_u64().context("stop absent")?;
            phase = "advance";
            if pc == 0x4ad8ac {
                if state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .is_some_and(|r| r.complete() && !r.acknowledged())
                {
                    state.0.return_world_row()?;
                }
                for _ in 0..8192 {
                    if state
                        .0
                        .world_paint()
                        .context("paint absent")?
                        .at_tile_cut(q)
                    {
                        break;
                    }
                    ensure!(
                        state.0.advance_world_paint(&mut state.1, &mut state.4)?,
                        "row42 caller stopped"
                    );
                    steps += 1;
                }
                state.0.compare_world_tiles(q)?;
                state.0.bind_world_row_prefix(&std::fs::read(root.join(
                    "fixtures/private/goth-row-context-20261031-first/row-source-first.json",
                ))?)?;
                state.0.bind_world_row_ordinary(&std::fs::read(root.join(
                    "fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json",
                ))?)?;
                state
                    .0
                    .bind_world_row_ordinary_objects(&std::fs::read(root.join(
                        "fixtures/private/goth-ordinary-object-20261101-first/object-source.json",
                    ))?)?;
                state
                    .0
                    .bind_world_row_ordinary_physical(&std::fs::read(root.join(
                        "fixtures/private/goth-ordinary-object-20261101-first/object-source.json",
                    ))?)?;
                state.0.bind_world_override_catalog(&std::fs::read(&supplement)?,&std::fs::read(root.join("fixtures/private/goth-ordinary-object-20261101-first/object-source.json"))?,&state.3)?;
                state.0.bind_world_row_visibility(&state.2)?;
            } else {
                for _ in 0..65536 {
                    if state
                        .0
                        .world_paint()
                        .and_then(|w| w.row_prefix())
                        .context("row42 absent")?
                        .at_native_observation(q)?
                    {
                        break;
                    }
                    ensure!(
                        state.0.advance_world_row_objects_mut(&mut state.3, &mut state.6)?,
                        "row42 child stopped before{pc:x}"
                    );
                    steps += 1;
                }
            }
            phase = "row/context";
            state.0.compare_world_row_prefix(q)?;
            if q["visibility_word"].is_object(){let v=&q["visibility_word"];let id=v["id"].as_i64().context("native visibility ID absent")?as i16;ensure!(state.3.vm.world.objects.get(&id).and_then(|o|o.data.get(&34)).copied().map(i64::from)==v["word"].as_i64(),"retained VM visibility word differs");}
            compare_contained_cache(&state,q)?;
            phase = "graphics/planes";
            state.0.compare_world_shared_graphics(q)?;
            if q["palette_selector"].is_object()||matches!(pc, 0x4bc606 | 0x4c1ee5 | 0x4bca26 | 0x4c1d14) {
                phase = "physical-next arguments/scratch";
                state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .context("row absent")?
                    .compare_object_raster(q)?;
            }
            if q["spr2_owners"].is_object() {
                phase = "physical-next owners";
                state
                    .0
                    .world_paint()
                    .and_then(|w| w.row_prefix())
                    .context("row absent")?
                    .compare_object_raster_batch(q)?;
            }
            if let Some(id) = q["object_cursor"]["id"]
                .as_i64()
                .or_else(|| q["object_lookup"]["head"].as_i64())
            {
                phase = "object cache";
                state.6.compare_draw_cache(id as i16, q)?;
            }
            if matches!(pc,0x4aeb9e|0x4aebc6){let mut readonly=state.clone();ensure!(!readonly.0.advance_world_row_objects(&readonly.3,&mut readonly.6)?&&readonly==state,"read-only replay acknowledged a VM field store");readonly_cuts.push(pc);}
            checks.push(serde_json::json!({"row_number":q["row_number"],"pc":pc,"stop":stop}));
            if (pc==0x4379ef&&q["registers"]["eax"].as_u64().is_some_and(|v|v&255==0)||matches!(pc,0x467482|0x4674e4|0x467515|0x467690|0x46754e|0x4671f8|0x467574|0x46767f|0x4adaa2|0x4adaa9|0x437c3f|0x437cee|0x437cf9|0x437cfd|0x437d26|0x437d29|0x437d2b|0x438024|0x43806d|0x438087|0x438093|0x4380de|0x4380e0|0x4aeb9e|0x4aeba3|0x463f4c|0x4640a5|0x4640a6|0x4aebc6|0x432656)||q["palette_selector"]["input_bits"]==0x3f000000u32&&matches!(pc,0x4b954a|0x4b9577|0x4b95ac|0x4c1bc7))
                && std::env::var_os("SIMS_DIAG_NO_COLD_RECHECK").is_none()
            {
                phase = "cold physical lookup";
                let ret=rows.last().context("row return absent")?;
                ensure!(ret["pc"]==0x4ad6abu32||ret["pc"]==0x4ad885u32,"original row return differs");
                let remaining:Vec<_>=rows.iter().filter(|r|r["trace_stop"].as_u64().is_some_and(|n|n>stop)).collect();
                let mut cut = cold(&store,&state,&remaining,&final_vm)?;
                cut["pc"] = pc.into();
                cut["stop"] = stop.into();
                println!("{}",serde_json::json!({"cold_stop":stop,"continuation_steps":cut["continuation_steps"],"native_boundaries":cut["full_state_native_boundaries"]}));
                saved.push(cut);
            }
        }
        Ok(())
    })();
    let final_unchanged=before==serde_json::to_vec(&state.3)?;
    let mut normalized=state.3.clone();
    let ids:std::collections::BTreeSet<_>=rows.iter().filter_map(|q|q["visibility_word"]["id"].as_i64()).map(|n|n as i16).collect();
    for id in ids{normalized.vm.world.objects.get_mut(&id).context("visibility owner absent")?.data.insert(34,*final_vm.vm.world.objects.get(&id).context("original visibility owner absent")?.data.get(&34).context("original word absent")?);}
    ensure!(normalized==final_vm,"row42 changed VM/RNG beyond its retained temporary word");
    if result.is_ok(){ensure!(final_unchanged,"row42 did not restore its visibility word");}
    let frontier = store.put_value(&state)?;
    let report = serde_json::json!({"schema":1,"row_number":target_row,"success":result.is_ok(),"input":input,"frontier":frontier,"checks":checks,"steps":steps,"saved_suspended_cuts":saved,"readonly_store_cuts":readonly_cuts,"captured_row42_cuts":captured.len(),"unselected_native_cuts":unselected,"native_trace_sha256":sims_fixtures::digest(&raw),"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":pc,"stop":stop,"phase":phase,"message":format!("{e:#}")})),"final_VM_and_RNG_unchanged":final_unchanged,"VM_unchanged_beyond_expected_visibility_word":true,"temporary_VM_visibility_writes_retained":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0});
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(std::env::var_os("SIMS_DIAG_REPORT").context("row42 report absent")?)?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]})
    );
    result
}
