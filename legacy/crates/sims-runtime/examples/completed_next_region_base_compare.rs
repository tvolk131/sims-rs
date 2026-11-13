//! Replay the original next-region base callbacks without supplying child replies.
use anyhow::{Context,Result,ensure};
use sims_runtime::original_session::draw_parent::{State,WorldScene,WorldComposition,WorldClassification};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{path::PathBuf,io::{BufRead,BufReader,Write}};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer,sims_vm::interface::state::State,WorldClassification);
fn caller(s:&Saved,q:&serde_json::Value)->Result<()>{
 let mut shaped=q.clone();shaped["kind"]="world_drawing_setup".into();
 // The sealed observer called its plane list native_planes. Normalize only
 // this field name; all plane/backend bytes still come from acknowledged reads.
 if let Some(planes)=shaped["surface_state"].get("native_planes").cloned(){ensure!(shaped["surface_state"].get("planes").is_none(),"ambiguous native plane-list schema");shaped["surface_state"]["planes"]=planes;}
 s.0.compare_world_tiles(&shaped)
}
fn main()->Result<()>{
 let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
 let input:sims_fixtures::BlobRef=serde_json::from_str(&std::env::var("SIMS_DIAG_INPUT_REF").unwrap_or_else(|_|"{\"sha256\":\"098baedb672bc2dc1fccbc24f640982ffc0b85b6e3851e185ad25f0e8e9dcc54\",\"bytes\":17287154}".into()))?;
 let store_in=std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-region-loop-20261102-first/region-third.store"));let mut s:Saved=sims_fixtures::Store::open(store_in)?.get_value(&input)?;
 if let Some(source)=std::env::var_os("SIMS_DIAG_GRADIENT_SOURCE"){s.0.bind_world_row_floor_sprite(&std::fs::read(root.join(source))?)?;}
 let first_row:u64=std::env::var("SIMS_DIAG_FIRST_ROW").unwrap_or_else(|_|"0".into()).parse()?;
 let witness=std::env::var_os("SIMS_DIAG_WITNESS").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-next-region-base-witness-20261102-fifth/post-level-trace.jsonl.gz"));let encoded=std::fs::read(witness)?;let mut rows=vec![];
 for l in BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice())).lines(){let q:serde_json::Value=serde_json::from_str(&l?)?;if q["observer"]=="window_region_base"&&q["base_row_number"].as_u64().context("base row ordinal absent")?>=first_row{rows.push(q);}}
 ensure!(!rows.is_empty(),"next base evidence absent");let store=sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("base output store absent")?)?;let prefix=std::fs::read(root.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"))?;let before=s.3.clone();let (mut checks,mut steps,mut current,mut returned)=(vec![],0,0,0);
 let result=(||->Result<()>{for q in &rows{current=q["pc"].as_u64().context("base PC absent")?as u32;
  match current {
   0x4ad4bd|0x4aee4c|0x4ad2a1=>{
    if s.0.world_paint().and_then(|w|w.row_prefix()).is_some_and(|r|r.complete()&&!r.acknowledged()){s.0.return_world_row()?;}
    for _ in 0..65536{if s.0.world_paint().context("paint absent")?.at_tile_cut(q){break;}ensure!(s.0.advance_world_paint(&mut s.1,&mut s.4)?,"next base caller stalled before{current:x}");ensure!(s.3==before,"next base caller changed VM/RNG");steps+=1;}
    caller(&s,q)?;
    if current==0x4aee4c{s.0.bind_world_row_prefix(&prefix)?;s.0.compare_world_row_prefix(q)?;}
   }
   0x4ad680|0x4ad88f=>{
    for _ in 0..65536{
     if s.0.world_paint().and_then(|w|w.row_prefix()).is_some_and(|r|r.complete()&&!r.acknowledged()){s.0.return_world_row()?;}
     if s.0.world_paint().and_then(|w|w.row_prefix()).context("base row absent")?.at_native_observation(q)?{break;}
     ensure!(s.0.advance_world_row_objects_mut(&mut s.3,&mut s.6)?,"next base child stalled before{current:x}");ensure!(s.3==before,"next base child changed VM/RNG");steps+=1;
    }
    s.0.compare_world_row_prefix(q)?;returned+=1;
   }
   _=>anyhow::bail!("unexpected base evidence PC{current:x}"),
  }
  s.0.compare_world_shared_graphics(q)?;checks.push(serde_json::json!({"pc":current,"stop":q["trace_stop"],"base_row_number":q["base_row_number"]}));
 }Ok(())})();
 let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"frontier":store.put_value(&s)?,"checks":checks,"steps":steps,"returned_rows":returned,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"message":format!("{e:#}"),"row_pending":format!("{:?}",s.0.world_paint().and_then(|p|p.row_prefix()).and_then(|r|r.pending()))})),"VM_and_RNG_unchanged":s.3==before,"native_trace_sha256":sims_fixtures::digest(&encoded),"source_admission":false,"additional_completed_fresh_ticks":0,"scope":"Actual next-region base scan and full shared state; no missing child reply is manufactured."});std::fs::File::create(std::env::var_os("SIMS_DIAG_REPORT").context("base report path absent")?)?.write_all(&serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"returned_rows":returned,"first_unavailable":report["first_unavailable"]}));result
}
