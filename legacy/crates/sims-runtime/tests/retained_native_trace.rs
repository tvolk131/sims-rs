//! Original raw-event provenance and strict comparison of stored Rust execution.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::BufRead, path::PathBuf};

fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("native bytes absent")?;
    ensure!(
        s.is_ascii() && s.len() % 2 == 0,
        "native byte encoding differs"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
        .collect()
}
fn word(b: &[u8], i: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4)
            .context("native CPU word outside packet")?
            .try_into()?,
    ))
}
fn checked(row: &Value) -> Result<()> {
    let registers = &row["registers"];
    let cpu = bytes(&row["raw_g"])?;
    for (name, offset) in [("eax", 0), ("esp", 16), ("eip", 32)] {
        ensure!(
            registers[name] == word(&cpu, offset)?,
            "native register projection differs"
        );
    }
    let pc = registers["eip"].as_u64().context("native PC absent")?;
    let reads = row["reads"]
        .as_array()
        .context("native stopped-epoch reads absent")?;
    for r in reads {
        let raw = bytes(&r["raw"])?;
        ensure!(
            r["bytes"] == raw.len() && r["sha256"] == sims_fixtures::digest(&raw),
            "native read receipt differs"
        );
    }
    let code = bytes(&row["code"]["raw"])?;
    ensure!(
        row["code"]["address"] == pc
            && reads
                .iter()
                .any(|r| r["address"] == pc && r["raw"] == row["code"]["raw"]),
        "native instruction/read epoch differs"
    );
    ensure!(!code.is_empty(), "empty native instruction window");
    Ok(())
}
fn observed_blob(row: &Value, key: &str, address: u64) -> Result<Vec<u8>> {
    let blob = &row[key];
    let raw = bytes(&blob["raw"])?;
    ensure!(
        blob["address"] == address
            && blob["bytes"] == raw.len()
            && blob["sha256"] == sims_fixtures::digest(&raw)
            && row["reads"]
                .as_array()
                .context("frame reads absent")?
                .iter()
                .any(|r| r["address"] == address && r["raw"] == blob["raw"]),
        "native frame blob/read differs"
    );
    Ok(raw)
}

#[test]
#[ignore = "private original raw trace joined to stored corrected 32-transition Rust replay"]
fn captured_entry_results_and_frame_boundary_have_actual_native_witnesses() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let native = root.join(
        "fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence",
    );
    let flat_raw = std::fs::read(native.join("native-bytecode.json"))?;
    ensure!(
        sims_fixtures::digest(&flat_raw)
            == "7430b5bcddc06cbc4a5be8ab0708da56f18bce00ef3a4bb9bc14a20552f05c44",
        "native flat trace differs"
    );
    let flat: Vec<Value> = serde_json::from_slice(&flat_raw)?;
    let encoded = std::fs::read(native.join("ordered-native-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&encoded)
            == "77ea9e08752623e74dd250eb7e9e59f4ab4a56d0551ffe1cea02aea1d409146c",
        "native raw trace differs"
    );
    let mut dispatches = Vec::new();
    let mut pending: [Vec<(usize, u64)>; 2] = [vec![], vec![]];
    let mut returned = BTreeMap::new();
    let mut frames = Vec::new();
    for line in std::io::BufReader::new(flate2::read::GzDecoder::new(&encoded[..])).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let kind = row["kind"].as_str().context("native row kind absent")?;
        match kind {
            "vm_dispatch" => {
                checked(&row)?;
                let i = dispatches.len();
                let mut expected = flat
                    .get(i)
                    .context("native dispatch exceeds flat trace")?
                    .clone();
                let object = expected
                    .as_object_mut()
                    .context("native flat instruction absent")?;
                object.remove("call_result_raw");
                object.remove("primitive_result_raw");
                ensure!(
                    expected == row["instruction"] && expected["stop_index"] == row["stop_index"],
                    "native dispatch/flat identity differs"
                );
                ensure!(
                    row["tick"]
                        .as_u64()
                        .is_some_and(|n| (89073..89105).contains(&n)),
                    "native dispatcher tick outside interval"
                );
                dispatches.push((
                    row["tick"].as_u64().unwrap(),
                    row["stop_index"].as_u64().unwrap(),
                ));
            }
            "primitive_call" | "subroutine_call" => {
                checked(&row)?;
                let slot = usize::from(kind == "subroutine_call");
                let i: usize = row["dispatch_index"]
                    .as_u64()
                    .context("native call index absent")?
                    .try_into()?;
                ensure!(
                    i + 1 == dispatches.len()
                        && (flat[i]["opcode"].as_u64().context("native opcode absent")? >= 256)
                            == (slot == 1),
                    "native call opcode/index differs"
                );
                pending[slot].push((
                    i,
                    row["registers"]["esp"]
                        .as_u64()
                        .context("native call stack absent")?
                        + if slot == 1 { 12 } else { 8 },
                ));
            }
            "primitive_return" | "subroutine_return" => {
                checked(&row)?;
                let slot = usize::from(kind == "subroutine_return");
                let (i, stack) = pending[slot].pop().context("unpaired native return")?;
                ensure!(
                    row["dispatch_index"] == i
                        && row["registers"]["esp"] == stack
                        && row["tick"] == dispatches[i].0,
                    "native return owner/stack/tick differs"
                );
                let eax: u32 = row["registers"]["eax"]
                    .as_u64()
                    .context("native return EAX absent")?
                    .try_into()?;
                let value = if slot == 1 { eax & 255 } else { eax };
                let field = if slot == 1 {
                    "call_result_raw"
                } else {
                    "primitive_result_raw"
                };
                ensure!(
                    row[field] == value
                        && flat[i][field] == value
                        && returned.insert(i, slot).is_none(),
                    "native return CPU/flat value differs"
                );
            }
            "application_after_clock"
            | "application_outer_return"
            | "application_entry"
            | "clock_entry" => {
                checked(&row)?;
                let now = word(&observed_blob(&row, "global_now", 0x6e54e0)?, 0)?;
                let sim = row["simulator"]
                    .as_u64()
                    .context("frame simulator absent")?;
                let elapsed = observed_blob(&row, "elapsed", sim + 0xb8)?;
                ensure!(elapsed.len() == 41, "frame elapsed owner size differs");
                frames.push(json!({"kind":kind,"stop_index":row["stop_index"],"tick":row["tick"],"pc":row["registers"]["eip"],"now":now,"elapsed_raw":row["elapsed"]["raw"],"instruction_source_profile":row["instruction_source_profile"]}));
            }
            _ => (),
        }
    }
    ensure!(
        dispatches.len() == 398 && returned.len() == 398 && pending.iter().all(Vec::is_empty),
        "native dispatch/call/return coverage incomplete"
    );
    let calls = returned.values().filter(|&&slot| slot == 1).count();
    ensure!(calls == 60, "native call-entry count differs");
    ensure!(
        frames.len() == 4,
        "native application/clock frame count differs"
    );
    for (r, (kind, stop, now)) in frames.iter().zip([
        ("application_after_clock", 666, 584466),
        ("application_outer_return", 667, 584466),
        ("application_entry", 668, 586466),
        ("clock_entry", 669, 586466),
    ]) {
        ensure!(
            r["kind"] == kind && r["stop_index"] == stop && r["tick"] == 89081 && r["now"] == now,
            "native frame ordering/input differs"
        );
    }
    ensure!(
        frames
            .iter()
            .all(|r| r["elapsed_raw"] == frames[0]["elapsed_raw"]),
        "elapsed fields changed before new clock entry"
    );
    let path =
        root.join("fixtures/private/goth-heading-20261031-first/sequence-heading-full-third.json");
    let report_raw = std::fs::read(path)?;
    let report: Value = serde_json::from_slice(&report_raw)?;
    ensure!(
        report["after"]["tick"] == 89105
            && report["boundary_sequence"]
                .as_array()
                .is_some_and(|r| r.len() == 32),
        "corrected completed Rust interval absent"
    );
    let rust: Vec<sims_vm::Trace> = serde_json::from_value(report["ordered_trace"].clone())?;
    let mut comparisons = Vec::new();
    for tick in 89073..89105 {
        let mut n = Vec::new();
        for (i, &(t, stop)) in dispatches.iter().enumerate() {
            if t == tick {
                let mut r = flat[i].clone();
                r["visit"] = stop.into();
                n.push(r);
            }
        }
        let r: Vec<_> = rust.iter().filter(|r| r.tick == tick).cloned().collect();
        let checked =
            sims_runtime::original_session::completed_replay::trace::compare(&json!(n), &r, tick)?;
        comparisons.push(json!({"tick":tick,"instructions":checked["instructions"],"yield_objects":checked["yield_objects"]}));
    }
    let result = json!({"schema":1,"kind":"raw_native_trace_entry_and_frame_witnesses","native_parity_credit":0,
        "native_trace_sha256":sims_fixtures::digest(&encoded),"native_flat_sha256":sims_fixtures::digest(&flat_raw),"rust_report_sha256":sims_fixtures::digest(&report_raw),
        "instructions":398,"primitive_results":338,"call_entry_statuses":60,"frame_events":frames,"comparisons":comparisons,
        "qualification":"All captured primitive and call-entry helper statuses, ordered instruction identities and yields checked. Later BHAV exits are not inferred from AL. Actual application/clock entry ordering and input globals are witnessed; no renderer callsite was captured by these points. Complete Session/source/gameplay admission remains separate."});
    if let Some(path) = std::env::var_os("SIMS_RETAINED_RAW_TRACE_DIAG_OUTPUT") {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        f.write_all(&serde_json::to_vec_pretty(&result)?)?;
    }
    eprintln!(
        "398 dispatches:338 primitive results,60 actual call-entry statuses; four native application/clock witnesses"
    );
    Ok(())
}
