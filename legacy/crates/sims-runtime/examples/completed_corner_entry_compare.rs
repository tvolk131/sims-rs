//! Compare the newly observed original corner-call ABI with the retained
//! computed first-vector continuation. No original answer is a runtime input.
use anyhow::{ensure, Context, Result};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::{BufRead,BufReader},path::PathBuf};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer,sims_vm::interface::state::State);
fn main()->Result<()> {
 let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
 let input=sims_fixtures::BlobRef{sha256:"1ae981f655a69ca894fe605472bd821d96097bf21ba005a2b1f787ed33c6b239".into(),bytes:10301912};
 let input_store=sims_fixtures::Store::open(std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-diagonal-endpoint-20261101-first/cold-third.store")))?;
 let(p,scene,view,vm,profiler,controls):Saved=input_store.get_value(&input)?;
 let before=serde_json::to_vec(&vm)?;
 let native=std::fs::read(std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-corner-witness-20261101-first/first-vector-trace.jsonl.gz")))?;
 ensure!(sims_fixtures::digest(&native)=="cffb6c2fd08f8d7955cc3836cecfd2e89896aa743e38acdc1ca76ac94c749ada","corner original trace differs");
 let mut entries=vec![];let mut corner_entries=vec![];
 for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines(){let q:serde_json::Value=serde_json::from_str(&line?)?;if q["kind"]=="world_drawing_setup"&&q["pc"]==0x53c227{if q["corner_ordinal"]==0{entries.push(q.clone());}corner_entries.push(q);}}
 ensure!(entries.len()==1,"original first corner entry is not unique");let q=&entries[0];
 ensure!(p.world_paint().context("paint absent")?.at_tile_cut(q),"retained iterator did not compute the actual second-vector call");
 p.compare_world_tiles(q)?;p.compare_world_shared_graphics(q)?;
 let geometry_source=std::fs::read(root.join("fixtures/private/goth-level-geometry-20261101-first/geometry-source-first.json"))?;
 ensure!(sims_fixtures::digest(&geometry_source)=="e72a81a206221a6d4f64474ce15f464ae343a8c893c1b817ca2532b0465953b0","B geometry ownership differs");
 let geometry:serde_json::Value=serde_json::from_slice(&geometry_source)?;
 let group=geometry["geometry"]["groups"].as_array().context("B geometry groups absent")?.iter().find(|g|g["model_offset"]==8).context("B corner vector absent")?;
 let records=group["records"].as_array().context("B corner records absent")?;
 let floor=&scene.graph.state.floors[0];ensure!(records.len()==25&&floor.corners.len()==25&&corner_entries.len()==25,"corner owner extent differs");
 let mut corner_differences=vec![];
 for (ordinal,entry)in corner_entries.iter().enumerate(){
  ensure!(entry["corner_ordinal"]==ordinal&&entry["level_geometry_child"]["model"]==records[ordinal]["pointer"],"corner model identity/order differs");
  let raw=entry["level_geometry_child"]["raw"].as_str().context("corner observed bytes absent")?;
  let n=(0..raw.len()).step_by(2).map(|i|u8::from_str_radix(&raw[i..i+2],16)).collect::<std::result::Result<Vec<_>,_>>()?;
  let id=floor.corners[ordinal];let c=&scene.graph.state.corners[id];
  let fields=[("point",scene.graph.geometry.positions[id].map(|v|v as u8).to_vec(),n[..3].to_vec()),("base",vec![c.base],vec![n[8]]),("masks",c.masks.to_vec(),n[9..13].to_vec()),("valid",c.valid.map(u8::from).to_vec(),n[0x24..0x26].to_vec()),("cap0",c.caps[0].to_vec(),n[0x28..0x3c].to_vec()),("cap1",c.caps[1].to_vec(),n[0x3c..0x50].to_vec())];
  for(field,rust,original)in fields{if rust!=original{corner_differences.push(serde_json::json!({"ordinal":ordinal,"field":field,"rust":rust,"original":original}));}}
 }

 let store=sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("output store absent")?)?;
 let reference=store.put_value(&(&p,&scene,&view,&vm,&profiler,&controls))?;
 ensure!(reference==input,"nonmutating corner comparison changed the retained input");
 let(mut restored,rs,rv,rvm,rb,rc):Saved=store.get_value(&reference)?;
 restored.compare_world_tiles(q)?;restored.compare_world_shared_graphics(q)?;
 ensure!(p==restored&&scene==rs&&view==rv&&profiler==rb&&controls==rc&&before==serde_json::to_vec(&rvm)?,"corner cold owners differ");
 let mut warm=p.clone();let a=warm.advance_world_geometry_child_with_scene(&vm,&controls,&scene);let b=restored.advance_world_geometry_child_with_scene(&rvm,&rc,&rs);
 let(Err(a),Err(b))=(a,b)else{anyhow::bail!("unimplemented corner body was fabricated")};
 ensure!(format!("{a:#}")==format!("{b:#}")&&warm==restored,"corner pending save/resume differs");
 ensure!(before==serde_json::to_vec(&vm)?,"corner comparison changed VM/RNG");
 let report=serde_json::json!({"schema":1,"success":true,"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"native_trace_stop":q["trace_stop"],"scene_corner_audit":{"checked_models":25,"success":corner_differences.is_empty(),"differences":corner_differences},"pc":q["pc"],"corner_ordinal":0,"original_call_ABI_context_rectangle_iterator_and_coupled_targets_match":true,"saved_reference":reference,"exact_cold_comparison_and_suspended_service_equal":true,"next_pending_service":format!("{a:#}"),"VM_and_RNG_unchanged":true,"observed_call_arguments_used_as_runtime_inputs":false,"source_admission":false,"native_parity_credit":0,"additional_completed_ticks":0,"scope":"Original corner entry now matches the retained Rust first-vector handoff. Corner body execution remains unsupported; no completed-tick admission."});
 let path=std::env::var_os("SIMS_DIAG_REPORT").context("report absent")?;use std::io::Write;std::fs::OpenOptions::new().create_new(true).write(true).open(path)?.write_all(&serde_json::to_vec_pretty(&report)?)?;println!("{}",report);Ok(())
}
