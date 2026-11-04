//!Replay successive original ordinary rows, stopping at the first unavailable
//!child. No observed result becomes an implementation input.
use anyhow::{ensure,Context,Result};
use sims_runtime::original_session::draw_parent::{State,WorldScene,WorldComposition};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::{BufRead,BufReader,Write},path::PathBuf};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer,sims_vm::interface::state::State);
fn main()->Result<()>{
 let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
 let input=sims_fixtures::BlobRef{sha256:"cd19b973136d85603f1c2130b08c448162d079d0763af85564d542feb627c82c".into(),bytes:10302446};
 let(mut p,mut scene,view,vm,mut profiler,controls):Saved=sims_fixtures::Store::open(root.join("fixtures/private/goth-post-level-20261101-first/full-final.store"))?.get_value(&input)?;
 let prefix=std::fs::read(root.join("fixtures/private/goth-row-context-20261031-first/row-source-first.json"))?;let source=std::fs::read(root.join("fixtures/private/goth-ordinary-row-20261101-first/ordinary-source.json"))?;
 let trace=std::fs::read(root.join("fixtures/private/goth-ordinary-rows-witness-20261101-first/post-level-trace.jsonl.gz"))?;
 let mut rows=vec![];for line in BufReader::new(flate2::read::GzDecoder::new(trace.as_slice())).lines(){let q:serde_json::Value=serde_json::from_str(&line?)?;if q["observer"]=="ordinary_rows"&&q["pc"].as_u64().is_some_and(|pc|matches!(pc,0x4ad8ac|0x4ad6ab|0x4ad368)){rows.push(q)}}ensure!(!rows.is_empty(),"ordinary row span evidence absent");
 let store=sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("span store absent")?)?;let before=serde_json::to_vec(&vm)?;let(mut steps,mut ordinal,mut pc,mut returned)=(0,0,0,0);let mut checks=vec![];let mut snapshots=vec![];
 let result=(||->Result<()>{for q in &rows{
  pc=q["pc"].as_u64().unwrap()as u32;ordinal=q["row_number"].as_u64().context("span row number absent")?;
  if pc==0x4ad8ac||pc==0x4ad368{
   if p.world_paint().and_then(|w|w.row_prefix()).is_some_and(|r|r.complete()&&!r.acknowledged()){p.return_world_row()?;}
   for _ in 0..8192{if p.world_paint().context("span paint absent")?.at_tile_cut(q){break}ensure!(p.advance_world_paint(&mut scene,&mut profiler)?,"span caller stopped before row{ordinal}");steps+=1;}
   p.compare_world_tiles(q)?;
   if pc==0x4ad8ac{p.bind_world_row_prefix(&prefix)?;p.bind_world_row_ordinary(&source)?;p.compare_world_row_prefix(q)?;}
  }else{
   steps+=p.advance_world_row_prefix_to(&vm,q,65536)?;p.compare_world_row_prefix(q)?;returned+=1;
  }
  p.compare_world_shared_graphics(q)?;checks.push(serde_json::json!({"row_number":ordinal,"pc":pc,"trace_stop":q["trace_stop"]}));
  if pc==0x4ad6ab&&matches!(ordinal,1|4|8|16|32){let blob=store.put_value(&(&p,&scene,&view,&vm,&profiler,&controls))?;let(rp,rs,rv,rvm,rb,rc):Saved=store.get_value(&blob)?;ensure!(p==rp&&scene==rs&&view==rv&&profiler==rb&&controls==rc&&serde_json::to_vec(&vm)?==serde_json::to_vec(&rvm)?,"span checkpoint differs");snapshots.push(serde_json::json!({"row_number":ordinal,"blob":blob}));}
 }Ok(())})();
 ensure!(before==serde_json::to_vec(&vm)?,"row span mutated VM/RNG");let frontier=store.put_value(&(&p,&scene,&view,&vm,&profiler,&controls))?;
 let pending=p.world_paint().and_then(|w|w.row_prefix()).and_then(|r|r.pending());
 let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"native_trace_sha256":sims_fixtures::digest(&trace),"comparison_boundaries":checks,"computed_microsteps":steps,"returned_rows":returned,"saved_cuts":snapshots,"frontier":frontier,"first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"row_number":ordinal,"pc":pc,"message":format!("{e:#}"),"pending":format!("{pending:?}")})),"VM_and_RNG_unchanged":true,"source_admission":false,"native_parity_credit":0,"additional_completed_fresh_ticks":0,"scope":"Selected natural ordinary-row entries/returns and coupled state; first unsupported child is preserved, never acknowledged by a stub. No completed-tick admission."});
 std::fs::OpenOptions::new().create_new(true).write(true).open(std::env::var_os("SIMS_DIAG_REPORT").context("span report absent")?)?.write_all(&serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::json!({"success":result.is_ok(),"returned_rows":returned,"checks":checks.len(),"first_unavailable":report["first_unavailable"]}));result
}
