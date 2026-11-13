//! Inspect existing independently saved classification/visibility continuations.
//! This executes no VM, renderer or original-engine instruction.
use anyhow::{Context,Result,ensure};
use sims_runtime::original_session::{draw_parent::{State,WorldClassification},character_visibility};
type Cut=(State,WorldClassification,sims_sim::Simulation,character_visibility::Source);
type Saved=(State,sims_runtime::original_session::draw_parent::WorldScene,sims_runtime::original_session::draw_parent::WorldComposition,sims_sim::Simulation,sims_vm::scheduler::clock::profiler::Buffer,sims_vm::interface::state::State,WorldClassification);
fn differences(a:&serde_json::Value,b:&serde_json::Value,path:&str,out:&mut Vec<String>){
    if a==b{return;}match(a,b){
        (serde_json::Value::Object(a),serde_json::Value::Object(b))=>{let keys:std::collections::BTreeSet<_>=a.keys().chain(b.keys()).collect();for k in keys{differences(a.get(k).unwrap_or(&serde_json::Value::Null),b.get(k).unwrap_or(&serde_json::Value::Null),&format!("{path}.{k}"),out);}},
        (serde_json::Value::Array(a),serde_json::Value::Array(b))if a.len()==b.len()=>for(i,(a,b))in a.iter().zip(b).enumerate(){differences(a,b,&format!("{path}[{i}]"),out);},
        _=>out.push(path.to_string()),
    }
}
fn main()->Result<()>{
    let refs:serde_json::Value=serde_json::from_slice(&std::fs::read(std::env::var_os("SIMS_HANDOFF_REFS").context("references absent")?)?)?;
    let store=sims_fixtures::Store::open(refs["store"].as_str().context("store absent")?)?;
    let mut rows=vec![];let mut best=None;let mut best_index=0;
    for r in refs["references"].as_array().context("references absent")?{
        let reference:sims_fixtures::BlobRef=serde_json::from_value(r.clone())?;let c:Cut=store.get_value(&reference)?;
        let p=serde_json::to_value(&c.0)?;let k=serde_json::to_value(&c.1)?;
        let index=p["world"]["people"]["index"].as_u64().context("person cursor absent")?;
        let row=serde_json::json!({"reference":reference,"classify_complete":c.1.complete(),"classification_stage":k["stage"],"lists":c.1.lists,"parent_phase":p["world"]["phase"],"people_stage":p["world"]["people"]["stage"],"people_index":index,"tick":c.2.vm.tick});
        if best.is_none()||index>=best_index{best=Some((reference,c));best_index=index;}rows.push(row);
    }
    let(reference,c)=best.context("no candidate")?;ensure!(c.1.complete(),"executed classification candidate incomplete");
    let baseline:sims_fixtures::BlobRef=serde_json::from_str("{\"sha256\":\"0693a3ec36c30dffbeb0d781f41e4b8412751d17b21d0b6e78117ef501ddc832\",\"bytes\":17330480}")?;
    let s:Saved=sims_fixtures::Store::open("fixtures/private/goth-remaining-window-20261102-first/end-window-final.store")?.get_value(&baseline)?;
    let before=serde_json::to_value(&c.2)?;let after=serde_json::to_value(&s.3)?;let mut vm_differences=vec![];differences(&before,&after,"simulation",&mut vm_differences);
    let executed=serde_json::to_value(&c.1)?;let auxiliary=serde_json::to_value(&s.6)?;
    let fields=["source","view","module","definitions","pointers","force_dynamic","lists","vectors","cache"].map(|k|(k,executed[k]==auxiliary[k])).into_iter().collect::<std::collections::BTreeMap<_,_>>();
    let rect_differences=executed["rectangles"].as_object().context("rectangles absent")?.keys().filter(|k|executed["rectangles"][*k]!=auxiliary["rectangles"][*k]).collect::<Vec<_>>();
    let mut mismatch=executed.clone();let id=mismatch["rectangles"].as_object().unwrap().keys().next().unwrap().clone();
    let x=mismatch["rectangles"][&id][0][0].as_i64().context("rectangle coordinate absent")?as i32;
    mismatch["rectangles"][&id][0][0]=serde_json::json!(x.wrapping_add(1));
    let bad:WorldClassification=serde_json::from_value(mismatch)?;let mut rejected=s.6.clone();let original=rejected.clone();
    ensure!(rejected.join_completed(bad).is_err()&&rejected==original,"handoff clobbered a changed rectangle cache");
    ensure!(rejected.join_completed(s.6.clone()).is_err()&&rejected==original,"handoff accepted incomplete execution");
    let mut joined=s.6.clone();joined.join_completed(c.1.clone())?;ensure!(joined.complete()&&joined.lists==c.1.lists,"handoff lost the executed lists");
    let output=serde_json::json!({"scope":"Existing actual Rust-executed classification/visibility cuts; checked owner handoff, no VM or renderer execution","rows":rows,"candidate":reference,"candidate_people_index":best_index,"same_classification_fields":fields,"auxiliary_stage":auxiliary["stage"],"rectangle_difference_ids":rect_differences,"simulation_differences":vm_differences,"same_VM_rng":c.2.vm.rng_state==s.3.vm.rng_state,"same_lot_rng":c.2.vm.lot_rng_state==s.3.vm.lot_rng_state,"modified_cache_rejected_without_mutation":true,"incomplete_execution_rejected_without_mutation":true,"actual_completed_handoff_succeeded":true,"source_admission":false,"fresh_tick_credit":0});
    std::fs::write(std::env::var_os("SIMS_HANDOFF_REPORT").context("report absent")?,serde_json::to_vec_pretty(&output)?)?;println!("{}",output["candidate"]);Ok(())
}
