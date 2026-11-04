//! Follow current scene-owned geometry to the first explicitly pending child.
//! A successful diagnostic is a qualified frontier, never a completed vector.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::{BufRead, BufReader, Write}, path::PathBuf};
type Saved = (State, WorldScene, WorldComposition, sims_sim::Simulation, Buffer);
fn selected(r: &serde_json::Value) -> bool {
    let Some(pc) = r["pc"].as_u64() else { return false };
    if pc == 0x4688bf {
        return r["geometry_prefix"]["stack"]["raw"].as_str().is_some_and(|s| s.starts_with("ecdb5300"));
    }
    matches!(pc, 0x53db67|0x53bfb5|0x53db9e|0x53dba9|0x466fcc|0x53dbd2|0x53dbf2|
        0x53dc67|0x53dc6a|0x53dc78|0x53dc7d|0x53dcbf|0x53dcc4|
        0x53dd65|0x53dd6a|0x53dd8e|0x53dd93|0x53de18|0x53de1d|0x53de3a|0x53de3f|
        0x53de78|0x53de7d|0x53de8a|0x53de8f|0x53dead|0x53deb2|
        0x53e84d|0x53e852|0x53e861|0x53e866|0x53e8bc|0x2f12874|0x688d82c0|
        0x53e8c2|0x53e8f2|0x53e8f8|0x540480)
}
fn cold(store:&sims_fixtures::Store,p:&State,scene:&WorldScene,view:&WorldComposition,vm:&sims_sim::Simulation,profiler:&Buffer)->Result<serde_json::Value>{
    let blob=store.put_value(&(p,scene,view,vm,profiler))?;
    let(mut c,cs,cv,cvm,cp):Saved=store.get_value(&blob)?;
    let mut w=p.clone();let mut steps=0;
    for _ in 0..512 {
        let a=w.advance_world_geometry_child(vm)?;let b=c.advance_world_geometry_child(&cvm)?;
        ensure!(a==b&&w==c,"vector cold continuation differs");
        if !a {break} steps+=1;
    }
    ensure!(*scene==cs&&*view==cv&&*profiler==cp&&serde_json::to_vec(vm)?==serde_json::to_vec(&cvm)?,"vector cold immutable owners differ");
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main()->Result<()> {
    let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let old=root.join("fixtures/private/goth-level-geometry-20261101-first");
    let j:serde_json::Value=serde_json::from_slice(&std::fs::read(old.join("joined-first.json"))?)?;
    let input:sims_fixtures::BlobRef=serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(input.sha256=="3b1b82c2d6e7804eaaa6ea39a9fd70ecf03754513ef9b7b82a8f6043a2ca153d","vector input differs");
    let(mut p,mut scene,view,vm,mut profiler):Saved=sims_fixtures::Store::open(std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||old.join("cold-first.store")))?.get_value(&input)?;
    let source=std::fs::read(root.join("fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json"))?;
    ensure!(sims_fixtures::digest(&source)=="48bdda26252d0600a22b49fc681cb982a98cff89d1b8279b6aee8731bde22531","vector source differs");
    let native=std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("native trace absent")?)?;
    let pins:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("fixtures/private/goth-geometry-first-vector-20261101-first/native-intake-first.json"))?)?;
    ensure!(pins["captures"][1]["pins"]["trace_sha256"]==sims_fixtures::digest(&native),"vector trace differs");
    let mut rows=vec![];
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines(){
        let r:serde_json::Value=serde_json::from_str(&line?)?;
        if r["kind"]=="world_drawing_setup"&&r["ordinal"].is_number()&&selected(&r){rows.push(r)}
    }
    let path=std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("output store absent")?;
    let store=sims_fixtures::Store::create(path)?;
    let immutable=sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?);
    let(mut checks,mut cuts,mut steps,mut returns,mut current,mut frontier)=(vec![],vec![],0,0,0,None);
    let result=(||->Result<()>{
        for r in &rows {
            current=r["pc"].as_u64().unwrap()as u32;
            if current==0x53db67 {
                for _ in 0..8 {
                    if p.world_paint().unwrap().at_tile_cut(r){break}
                    ensure!(p.advance_world_paint(&mut scene,&mut profiler)?,"iterator stalled before next model");steps+=1;
                }
                p.bind_world_geometry_child(&source)?;
            }
            if current==0x53e8f8 {
                let before=serde_json::to_vec(&p)?;
                ensure!(!p.advance_world_geometry_child(&vm)?&&serde_json::to_vec(&p)?==before,"visible child must stay pending without mutation");
                cuts.push(cold(&store,&p,&scene,&view,&vm,&profiler)?);
                frontier=Some(serde_json::json!({"pc":current,"model":r["model"]["pointer"],"ordinal":r["ordinal"],"trace_stop":r["trace_stop"],"reason":"visible geometry rendering is pending"}));
                break;
            }
            for _ in 0..64 {
                if p.world_paint().unwrap().at_tile_cut(r){break}
                ensure!(p.advance_world_geometry_child(&vm)?,"child stalled at{current:x}");steps+=1;
            }
            if current==0x53dcc4 {
                let before=serde_json::to_vec(&p)?;
                let missing=p.bind_world_geometry_segments(&source).expect_err("vector must require current scene ownership");
                ensure!(missing.to_string().contains("current scene descriptors")&&serde_json::to_vec(&p)?==before,"missing scene owner changed the continuation");
                p.bind_world_geometry_segments_from_scene(&source,&scene)?;
            }
            p.compare_world_tiles(r)?;
            p.compare_world_shared_graphics(r)?;
            checks.push(serde_json::json!({"pc":current,"model":r["model"]["pointer"],"trace_stop":r["trace_stop"]}));
            if matches!(current,0x53dcc4|0x540480|0x53e8f2){cuts.push(cold(&store,&p,&scene,&view,&vm,&profiler)?);}
            if current==0x540480 {returns+=1;}
        }
        ensure!(returns==1&&frontier.is_some(),"expected visible model frontier not reached");
        ensure!(immutable==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"vector changed enclosing VM/RNG/scene/profiler");
        Ok(())
    })();
    let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"additional_B_source_sha256":sims_fixtures::digest(&source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"actual_children_returned":returns,"pending_frontier":frontier,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&scene,&view,&vm,&profiler))?),"completed_vector":false,"source_admission":false,"native_parity_credit":0,"scope":"Current scene descriptor inputs and first child return, followed by the next model's first visible branch. A diagnostic frontier is not completed rendering, full CPU equality or completed-tick parity."});
    std::fs::OpenOptions::new().create_new(true).write(true).open(std::env::var_os("SIMS_DIAG_REPORT").context("report absent")?)?.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::json!({"success":result.is_ok(),"steps":steps,"actual_children_returned":returns,"frontier":report["pending_frontier"],"first_divergence":report["first_divergence"]}));
    result
}
