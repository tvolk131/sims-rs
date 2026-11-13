//! Continue the qualified visible frontier using retained shared context/cache owners.
use anyhow::{Context,Result,ensure};
use sims_runtime::original_session::draw_parent::{State,WorldScene,WorldComposition};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::{BufRead,BufReader,Write},path::PathBuf};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer);
fn cold(store:&sims_fixtures::Store,p:&State,s:&WorldScene,v:&WorldComposition,vm:&sims_sim::Simulation,b:&Buffer)->Result<serde_json::Value>{
    let blob=store.put_value(&(p,s,v,vm,b))?;let(mut c,cs,cv,cvm,cb):Saved=store.get_value(&blob)?;let mut w=p.clone();let mut steps=0;
    for _ in 0..128 {let a=w.advance_world_geometry_child(vm)?;let z=c.advance_world_geometry_child(&cvm)?;ensure!(a==z&&w==c,"visible cold continuation differs");if !a {break}steps+=1;}
    ensure!(*s==cs&&*v==cv&&*b==cb&&serde_json::to_vec(vm)?==serde_json::to_vec(&cvm)?,"visible immutable owners differ");
    Ok(serde_json::json!({"blob":blob,"resumed_microsteps":steps}))
}
fn main()->Result<()> {
    let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let previous=root.join("fixtures/private/goth-geometry-first-vector-20261101-first");
    let j:serde_json::Value=serde_json::from_slice(&std::fs::read(previous.join("joined-third.json"))?)?;
    let input:sims_fixtures::BlobRef=serde_json::from_value(j["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(input.sha256=="12b262de7a3cdf5e41a9c971a5de22a69ff05e8d96789779682c10e36fbed959","visible input differs");
    let(mut p,s,v,vm,b):Saved=sims_fixtures::Store::open(std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||previous.join("cold-third.store")))?.get_value(&input)?;
    let model_source=std::fs::read(previous.join("first-vector-source-first.json"))?;
    p.bind_world_geometry_visible(&model_source)?;
    let material_source=std::fs::read(root.join("fixtures/private/goth-geometry-visible-20261101-first/material-source-third.json"))?;
    let native=std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").context("visible native trace absent")?)?;
    ensure!(sims_fixtures::digest(&native)=="6aafd73662b39219086a05d048d07b7c907fbd39a5cb9a3e09034f409f2362ac","visible trace differs");
    let mut rows=vec![];let mut started=false;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines(){
        let q:serde_json::Value=serde_json::from_str(&line?)?;
        if q["kind"]!="world_drawing_setup"||q["ordinal"]!=1 {continue}
        let Some(pc)=q["pc"].as_u64()else{continue};
        if pc==0x53e8f8 {started=true}
        if started&&matches!(pc,0x53e8f8|0x53e934|0x53e939|0x53e953|0x53e958|0x53e95c|0x53e961|0x53e963|0x53e968|0x53ea46|0x53ea4b|0x53eaba|0x53eabf|0x53eaf1|0x53eaf6|0x53ebfb|0x53ecac|0x53ecb1|0x53ecdf|0x53ece4|0x53ed40|0x53ed45|0x53ed47){rows.push(q)}
        if started&&pc==0x53ed47{break}
    }
    ensure!(rows.len()==23,"visible observation count differs");
    let store=sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("visible output store absent")?)?;
    let immutable=sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?);
    let(mut cuts,mut checks,mut steps,mut current)=(vec![],vec![],0,0);
    let result=(||->Result<()>{
        for q in &rows {
            current=q["pc"].as_u64().unwrap()as u32;
            for _ in 0..128 {if p.world_paint().unwrap().at_tile_cut(q){break}ensure!(p.advance_world_geometry_child(&vm)?,"visible child stalled at{current:x}");steps+=1;}
            p.compare_world_tiles(q)?;p.compare_world_shared_graphics(q)?;
            if current==0x53eaba{p.bind_world_geometry_materials(&material_source)?;}
            checks.push(serde_json::json!({"pc":current,"trace_stop":q["trace_stop"]}));
            cuts.push(cold(&store,&p,&s,&v,&vm,&b)?);
        }
        ensure!(!p.advance_world_geometry_child(&vm)?,"unimplemented dual-sprite renderer acknowledged");
        ensure!(immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"visible prelude changed enclosing VM/RNG/scene/profiler");Ok(())
    })();
    let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"additional_B_source_sha256":sims_fixtures::digest(&material_source),"native_trace_sha256":sims_fixtures::digest(&native),"comparison_boundaries":checks,"cold_cuts":cuts,"steps":steps,"first_divergence":result.as_ref().err().map(|e|format!("at{current:x}: {e:#}")),"VM_and_RNG_unchanged":immutable==sims_fixtures::digest(&serde_json::to_vec(&(&s,&v,&vm,&b))?),"completed_visible_child":false,"source_admission":false,"native_parity_credit":0,"scope":"Computed visible rotation/center/placement/light, baseline material lookup, image selection, flags, descriptor and palette receiver with current shared caches/context. Dual-sprite drawing, full CPU and32 completed ticks remain pending."});
    std::fs::OpenOptions::new().create_new(true).write(true).open(std::env::var_os("SIMS_DIAG_REPORT").context("visible report absent")?)?.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::json!({"success":result.is_ok(),"steps":steps,"first_divergence":report["first_divergence"]}));result
}
