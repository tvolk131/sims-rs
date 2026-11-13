//! Retained post-window continuation. Native rows compare; no engine replies.
use anyhow::{ensure,Context,Result};
use sims_runtime::original_session::draw_parent::{State,WorldScene,WorldComposition,WorldClassification};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{path::PathBuf,io::{BufRead,BufReader}};
type Saved=(State,WorldScene,WorldComposition,sims_sim::Simulation,Buffer,sims_vm::interface::state::State,WorldClassification);
fn step(s:&mut Saved,clocks:&serde_json::Value,baseline:&sims_sim::Simulation)->Result<()>{
    if let Some((ordinal,name))=s.0.world_paint().and_then(|p|p.post_clock_request()){
        let q=clocks["events"].as_array().context("clock events absent")?.get(ordinal).context("derived clock inputs exhausted")?;
        ensure!(q["ordinal"]==ordinal&&q["name"]==name&&q["argument"]==0&&q["rdtsc_pc"]==0x5d5336u32,"RDTSC request differs");
        s.0.acknowledge_world_timestamp(q["clock"].as_u64().context("clock word absent")?,&mut s.4)?;
    }else{
        ensure!(s.0.advance_world_paint(&mut s.1,&mut s.4)?,"post-window stopped at {:?}",s.0.world_post_window_request().or_else(||s.0.world_paint().and_then(|p|p.pending())));
    }
    ensure!(&s.3==baseline,"post-window changed VM/RNG");Ok(())
}
fn ready(s:&Saved,q:&serde_json::Value)->bool{s.0.world_paint().is_some_and(|p|p.at_post_window_cut(q["pc"].as_u64().unwrap()as u32))}
fn compare(s:&Saved,q:&serde_json::Value)->Result<()>{s.0.compare_world_post_window(q,&s.4)?;s.0.compare_world_shared_graphics(q)}
fn main()->Result<()>{
    let root=std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from).unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let input:sims_fixtures::BlobRef=serde_json::from_str(&std::env::var("SIMS_DIAG_INPUT_REF").unwrap_or_else(|_|"{\"sha256\":\"0693a3ec36c30dffbeb0d781f41e4b8412751d17b21d0b6e78117ef501ddc832\",\"bytes\":17330480}".into()))?;
    let source_store=std::env::var_os("SIMS_DIAG_INPUT_STORE").map(PathBuf::from).unwrap_or_else(||root.join("fixtures/private/goth-remaining-window-20261102-first/end-window-final.store"));
    let mut s:Saved=sims_fixtures::Store::open(source_store)?.get_value(&input)?;let baseline=s.3.clone();
    let source=std::fs::read(root.join("fixtures/private/goth-after-window-20261102-first/source-first.json"))?;s.0.bind_world_post_window(&source)?;
    let encoded_clocks=std::fs::read(root.join("fixtures/private/goth-after-window-20261102-first/direct-inputs-first.json"))?;
    ensure!(sims_fixtures::digest(&encoded_clocks)=="eb5ac13549838e729acfade2029a95de420c394da380d82ccdfb511c053b6dcc","captured RDTSC inputs differ");
    let clocks:serde_json::Value=serde_json::from_slice(&encoded_clocks)?;
    let native=std::fs::read(root.join("fixtures/private/goth-after-window-copy-witness-20261102-first/world-classification-trace.jsonl.gz"))?;
    ensure!(sims_fixtures::digest(&native)=="381c6eef0317b5c24841b29d810dde5674522365bfe5dd8d906ada76d2cadaeb","post-window trace differs");
    let rows=BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines().map(|l|Ok(serde_json::from_str::<serde_json::Value>(&l?)?)).collect::<Result<Vec<_>>>()?.into_iter().filter(|q|matches!(q["pc"].as_u64(),Some(0x4a92a2|0x4a92f8|0x4a9310|0x4a934c|0x4a9371|0x4a93b6|0x4a93ce|0x4a93f1|0x4a93ff|0x4a941d|0x4a946a|0x4a8deb))).collect::<Vec<_>>();
    let out=sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("output store absent")?)?;
    let mut checks=vec![];let mut cold_cuts=vec![];let mut steps=0;let mut pc=0;
    let cold=std::env::var_os("SIMS_DIAG_COLD").is_some();
    let result=(||->Result<()>{
        for(index,q)in rows.iter().enumerate(){pc=q["pc"].as_u64().unwrap()as u32;
            for _ in 0..262144{if ready(&s,q){break;}step(&mut s,&clocks,&baseline)?;steps+=1;}
            ensure!(ready(&s,q),"post-window native cut not reached");compare(&s,q)?;
            checks.push(serde_json::json!({"pc":pc,"stop":q["trace_stop"]}));println!("{}",serde_json::json!({"checked":checks.len(),"pc":pc}));
            if cold&&index+1<rows.len(){
                let blob=out.put_value(&s)?;let mut a=s.clone();let mut b:Saved=out.get_value(&blob)?;ensure!(a==b,"cold entry differs");
                let target=&rows[index+1];let mut count=0;
                let continued=(||->Result<()>{for _ in 0..262144{if ready(&a,target){ensure!(ready(&b,target),"cold boundary differs");compare(&a,target)?;compare(&b,target)?;return Ok(());}step(&mut a,&clocks,&baseline)?;step(&mut b,&clocks,&baseline)?;ensure!(a==b,"complete cold Saved tuple differs");count+=1;}anyhow::bail!("cold iteration bound")})();
                cold_cuts.push(serde_json::json!({"blob":blob,"from_pc":pc,"to_pc":target["pc"],"steps":count,"complete_state_equal_after_each_step":a==b,"success":continued.is_ok(),"first_unavailable":continued.as_ref().err().map(|e|format!("{e:#}"))}));
                //Unsupported native children remain a failed continuation; the
                //primary execution must independently reach the same frontier.
            }
        }Ok(())
    })();
    let report=serde_json::json!({"schema":1,"success":result.is_ok(),"input":input,"checks":checks,"steps":steps,"cold_cuts":cold_cuts,"frontier":out.put_value(&s)?,"VM_and_RNG_unchanged":s.3==baseline,
        "first_unavailable":result.as_ref().err().map(|e|serde_json::json!({"target_pc":pc,"message":format!("{e:#}"),"computed_request":s.0.world_post_window_request()})),
        "clock_input_sha256":sims_fixtures::digest(&encoded_clocks),"clock_provenance":clocks["scope"],"preceding_profiler_timestamps_controlled":true,"actual_RDTSC_instruction_boundary_captured":true,"source_admission":false,"additional_completed_fresh_ticks":0});
    std::fs::write(std::env::var_os("SIMS_DIAG_REPORT").context("report absent")?,serde_json::to_vec_pretty(&report)?)?;println!("{}",serde_json::json!({"success":result.is_ok(),"checks":checks.len()}));result
}
