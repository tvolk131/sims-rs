//! Component comparison against actual 60505f calls; no whole-frame admission.
use anyhow::{ensure, Context, Result};
use sims_vm::scheduler::clock::draw::targets::backend::{Blit, Fields, Request};
use std::{io::{BufRead, BufReader}, path::PathBuf};
type Saved = (Blit, Fields, Fields);
fn bytes(v: &serde_json::Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("native bytes absent")?;
    ensure!(s.len() % 2 == 0, "native hex extent differs");
    (0..s.len()).step_by(2).map(|i| Ok(u8::from_str_radix(&s[i..i+2], 16)?)).collect()
}
fn rectangle(v: &serde_json::Value) -> Result<[i32;4]> {
    let b = bytes(v)?; ensure!(b.len() == 16, "native rectangle extent differs");
    Ok(std::array::from_fn(|i| i32::from_le_bytes(b[4*i..4*i+4].try_into().unwrap())))
}
fn field(q: &serde_json::Value, address: u32) -> Result<Fields> {
    let p = q["after_window_copy"]["planes"].as_array().context("native planes absent")?
        .iter().find(|p| p["backend"] == address).context("backend owner absent")?;
    Ok(Fields { raw: bytes(&p["backend_raw"])? })
}
fn step(s: &mut Saved, input: &serde_json::Value) -> Result<()> {
    if let Some(request) = s.0.pending() {
        let expected = Request::DirectDrawBlit {
            interface: input["interface"].as_u64().context("interface absent")? as u32,
            destination: rectangle(&input["destination_rectangle"] )?,
            source_interface: input["source_interface"].as_u64().context("source interface absent")? as u32,
            source: rectangle(&input["source_rectangle"] )?,
            flags: input["flags"].as_u64().context("flags absent")? as u32,
            effects_address: (input["source_backend"].as_u64().context("source backend absent")? as u32).wrapping_add(0xf4),
            effects: bytes(&input["effects"] )?,
        };
        ensure!(request == &expected, "computed Blt request differs from original");
        s.0.directdraw_return(&s.1, &s.2, input["hresult"].as_u64().context("platform HRESULT absent")? as u32).map_err(anyhow::Error::msg)?;
    } else {
        ensure!(s.0.advance(&s.1, &s.2).map_err(anyhow::Error::msg)?, "copy stopped at {:#x}", s.0.pc());
    }
    Ok(())
}
fn compare(s: &Saved, q: &serde_json::Value, destination: u32, source: u32) -> Result<()> {
    ensure!(q["pc"] == s.0.pc(), "native copy PC differs");
    ensure!(s.1 == field(q,destination)? && s.2 == field(q,source)?, "native backend bytes differ");
    if let Some(result) = s.0.result() {
        ensure!(q["registers"]["eax"].as_u64().context("native EAX absent")? as u8 == result, "computed AL differs");
    }
    Ok(())
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let native = std::fs::read(root.join("fixtures/private/goth-after-window-copy-witness-20261102-first/world-classification-trace.jsonl.gz"))?;
    ensure!(sims_fixtures::digest(&native) == "381c6eef0317b5c24841b29d810dde5674522365bfe5dd8d906ada76d2cadaeb", "native trace differs");
    let encoded = std::fs::read(root.join("fixtures/private/goth-after-window-20261102-first/direct-inputs-first.json"))?;
    ensure!(sims_fixtures::digest(&encoded) == "eb5ac13549838e729acfade2029a95de420c394da380d82ccdfb511c053b6dcc", "platform inputs differ");
    let inputs: serde_json::Value = serde_json::from_slice(&encoded)?;
    let rows = BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines()
        .map(|l| Ok(serde_json::from_str::<serde_json::Value>(&l?)?)).collect::<Result<Vec<_>>>()?
        .into_iter().filter(|q| q.get("after_window_copy").is_some() && q["pc"].as_u64().is_some_and(|p| (0x60505f..=0x605181).contains(&p))).collect::<Vec<_>>();
    let store = sims_fixtures::Store::create(std::env::var_os("SIMS_DIAG_OUTPUT_STORE").context("output store absent")?)?;
    let mut reports = vec![];
    let mut at = 0;
    for input in inputs["external_blts"].as_array().context("Blt inputs absent")? {
        let first = rows.get(at).context("native entry absent")?;
        let destination = first["registers"]["ecx"].as_u64().context("native destination absent")? as u32;
        let stack = bytes(&first["after_window_copy"]["stack"]["raw"])?;
        let source = u32::from_le_bytes(stack.get(4..8).context("native source argument absent")?.try_into().unwrap());
        ensure!(input["destination_backend"] == destination && input["source_backend"] == source, "native source aliases differ");
        let mut s = (Blit::new(source, rectangle(&input["source_rectangle"] )?, rectangle(&input["destination_rectangle"] )?), field(first,destination)?, field(first,source)?);
        let mut checks = vec![];
        loop {
            let q = &rows[at]; compare(&s,q,destination,source)?;
            let blob = store.put_value(&s)?; let mut cold: Saved = store.get_value(&blob)?;
            ensure!(s == cold, "cold copy entry differs");
            let pc = s.0.pc(); step(&mut s,input)?; step(&mut cold,input)?;
            ensure!(s == cold, "complete copy continuation differs");
            checks.push(serde_json::json!({"pc":pc,"stop":q["trace_stop"],"blob":blob,"cold_equal":true})); at += 1;
            if s.0.complete() { break; }
        }
        ensure!(s.0.result() == Some(1), "computed engine copy result differs");
        reports.push(serde_json::json!({"destination":destination,"source":source,"checks":checks,"computed_result":s.0.result(),"frontier":store.put_value(&s)?}));
    }
    ensure!(at == rows.len() && at == 62, "native copy coverage differs");
    let report = serde_json::json!({"schema":1,"success":true,"calls":reports,"native_cuts":at,"cold_continuations":at,"all_backend_bytes_compared":true,"actual_platform_inputs_only":true,"complete_CPU_device_admission":false,"source_admission":false,"additional_completed_fresh_ticks":0});
    std::fs::write(std::env::var_os("SIMS_DIAG_REPORT").context("report absent")?,serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::json!({"success":true,"native_cuts":at,"cold_continuations":at})); Ok(())
}
