//! Replay the actual window region handoff, with complete cold reconstructions.
use anyhow::{Context,Result,ensure};
use sims_runtime::original_session::draw_parent::{State,WorldScene,WorldComposition,WorldClassification};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{path::{Path,PathBuf},io::{BufRead,BufReader,Write}};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer,sims_vm::interface::state::State,WorldClassification);
fn advance(s:&mut Saved)->Result<bool>{s.0.advance_world_paint(&mut s.1,&mut s.4)}
fn compare(s:&Saved,q:&serde_json::Value)->Result<()>{s.0.compare_world_tiles(q)?;s.0.compare_world_shared_graphics(q)}
fn cold(store:&sims_fixtures::Store,state:&Saved,rows:&[&serde_json::Value])->Result<serde_json::Value>{
 let blob=store.put_value(state)?;let mut restored:Saved=store.get_value(&blob)?;ensure!(&restored==state,"region cold reconstruction differs");let mut warm=state.clone();let before=state.3.clone();let mut steps=0;
 for q in rows{
  for _ in 0..1024{if warm.0.world_paint().context("cold paint absent")?.at_tile_cut(q){break;}let a=advance(&mut warm)?;let b=advance(&mut restored)?;ensure!(a&&a==b&&warm.3==before&&restored.3==before,"region cold progress/VM/RNG differs");steps+=1;}
  ensure!(warm==restored,"full region state differs at native boundary");compare(&warm,q)?;
 }
 ensure!(warm==restored,"cold region endpoint differs");Ok(serde_json::json!({"blob":blob,"continuation_steps":steps,"full_state_native_boundaries":rows.len(),"VM_and_RNG_checked_every_step":true,"state_and_services_equal":true}))
}
fn main()->Result<()>{
 let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
 let input:sims_fixtures::BlobRef=serde_json::from_str(&std::env::var("SIMS_DIAG_INPUT_REF").unwrap_or_else(|_|"{\"sha256\":\"b7bf89cee3e53556c8f054b1e856dc43482b2abb744d21e72d5171ce52f9be6d\",\"bytes\":17287928}".into()))?;
 let input_store=std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-ordinary-split-20261102-first/following-release.store"));
 let mut state:Saved=sims_fixtures::Store::open(input_store)?.get_value(&input)?;
 state.0.bind_world_region_loop(&std::fs::read(root.join("fixtures/private/goth-region-loop-20261102-first/region-loop-source.json"))?)?;
 let witness=std::env::var_os("SIMS_DIAG_WITNESS").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-window-after-ordinary-witness-20261102-second/post-level-trace.jsonl.gz"));let encoded=std::fs::read(witness)?;let mut rows=vec![];
 for l in BufReader::new(flate2::read::GzDecoder::new(encoded.as_slice())).lines(){let q:serde_json::Value=serde_json::from_str(&l?)?;if q["observer"]=="ordinary_rows"&&(q["window_tail"].is_object()||q["pc"]==0x4ad368u32){rows.push(q);}}
 ensure!(rows.len()==44&&rows.last().unwrap()["pc"]==0x4ad29cu32,"original region evidence scope differs");
 let store=sims_fixtures::Store::create(Path::new(&std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("region output store absent")?))?;let initial=state.3.clone();let (mut checks,mut cuts,mut steps)=(vec![],vec![],0);let mut current=0;
 let result=(||->Result<()>{for(i,q)in rows.iter().enumerate(){current=q["pc"].as_u64().context("PC absent")?as u32;
  for _ in 0..1024{if state.0.world_paint().context("paint absent")?.at_tile_cut(q){break;}ensure!(advance(&mut state)?,"region handoff stalled before{current:x}");ensure!(state.3==initial,"region handoff changed VM/RNG");steps+=1;}
  compare(&state,q)?;checks.push(serde_json::json!({"pc":current,"stop":q["trace_stop"]}));
  if std::env::var_os("SIMS_DIAG_NO_COLD_RECHECK").is_none()&&i>0{let remaining:Vec<_>=rows[i+1..].iter().collect();let mut cut=cold(&store,&state,&remaining)?;cut["pc"]=current.into();cut["stop"]=q["trace_stop"].clone();println!("{}",serde_json::json!({"cold_pc":current,"boundaries":remaining.len()}));cuts.push(cut);}
 }Ok(())})();
 let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"frontier":store.put_value(&state)?,"checks":checks,"saved_suspended_cuts":cuts,"steps":steps,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"pc":current,"message":format!("{e:#}")})),"VM_and_RNG_unchanged":state.3==initial,"native_trace_sha256":sims_fixtures::digest(&encoded),"source_admission":false,"additional_completed_fresh_ticks":0,"scope":"Original null-clip next-region handoff and next base call; no drawing child or completed tick is acknowledged."});
 std::fs::File::create(std::env::var_os("SIMS_DIAG_REPORT").context("report path absent")?)?.write_all(&serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::json!({"success":result.is_ok(),"checks":checks.len(),"first_unavailable":report["first_unavailable"]}));result
}
