//! Original ordered instruction/branch/yield comparison, including the qualified
//! one-tick and observed partial-continuation shapes. Unknown shapes stop.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
pub fn compare(native: &Value, rust: &[sims_vm::Trace], tick: u64) -> Result<Value> {
    compare_fields(native, rust, tick, true)
}

/// Captured dispatcher stops prove ordered entry, primitive results and yields.
/// A call result is the entry helper's AL status, not a later BHAV exit value.
/// Older projections may omit it; report that gap without inventing a status.
pub fn compare_dispatches(native: &Value, rust: &[sims_vm::Trace], tick: u64) -> Result<Value> {
    let mut rows = native
        .as_array()
        .context("native dispatcher rows absent")?
        .clone();
    for row in &mut rows {
        ensure!(
            row.get("visit").is_none(),
            "dispatcher visit already supplied"
        );
        row["visit"] = row["stop_index"].clone();
    }
    let calls: Vec<_> = rows.iter().filter(|r| r["opcode"].as_u64().is_some_and(|n| n >= 256)).collect();
    let observed = calls.iter().filter(|r| r["call_result_raw"].as_u64().is_some()).count();
    let complete = observed == calls.len();
    let count = calls.len();
    let mut result = compare_fields(&Value::Array(rows), rust, tick, complete)?;
    // Preserve the historical report key, with its precise ABI stated below.
    result["native_call_return_values_compared"] = complete.into();
    result["native_call_entry_statuses"] = json!({"calls":count,"observed":observed,"complete":complete,
        "abi":"entry helper AL; later BHAV exit values are separate"});
    result["native_parity_credit"] = 0.into();
    Ok(result)
}

fn compare_fields(
    native: &Value,
    rust: &[sims_vm::Trace],
    tick: u64,
    raw_calls: bool,
) -> Result<Value> {
    let native = native
        .as_array()
        .context("native instruction trace absent")?;
    ensure!(
        !native.is_empty() && native.len() == rust.len(),
        "native/Rust instruction count differs"
    );
    let mut previous = 0;
    let mut yields = vec![];
    let mut rows = vec![];
    for (i, (a, b)) in native.iter().zip(rust).enumerate() {
        let visit = a["visit"].as_u64().context("native visit absent")?;
        ensure!(visit > previous, "unordered native instruction visits");
        previous = visit;
        let expected = json!({"tick":tick,"thread":a["object"],"depth":a["depth"],"location":{"source":a["source"],"bhav":a["tree"],"instruction":a["node"]},"opcode":a["opcode"],"operands":a["operands"]});
        let actual = serde_json::to_value(b)?;
        ensure!(
            actual.as_object().context("Rust trace absent")?.len() == 7
                && expected
                    .as_object()
                    .unwrap()
                    .iter()
                    .all(|(k, v)| actual[k] == *v),
            "instruction identity/order differs at {i}"
        );
        let opcode = a["opcode"].as_u64().context("native opcode absent")?;
        let result = a["primitive_result_raw"].as_i64();
        let effect = actual["effect"].as_str().context("Rust effect absent")?;
        let outcome = if opcode >= 256 {
            let next = native.get(i + 1).context("native call has no child")?;
            ensure!(
                ((!raw_calls && a["call_result_raw"].is_null()) || a["call_result_raw"] == 1)
                    && next["object"] == a["object"]
                    && next["depth"].as_u64() == a["depth"].as_u64().map(|n| n + 1)
                    && next["tree"] == opcode
                    && next["node"] == 0
                    && effect
                        == format!(
                            "call {} BHAV 0x{opcode:04x} resident:{opcode}",
                            next["source"]
                                .as_str()
                                .context("native callee source absent")?
                        ),
                "subroutine entry differs at {i}"
            );
            "call"
        } else if result == Some(2)
            && ((opcode == 44 && effect == "await animation service response")
                || (opcode == 27 && effect == "await route service response"))
        {
            "yield"
        } else if opcode == 0 && matches!(result, Some(1 | 2)) {
            let outcome = if result == Some(1) { "true" } else { "yield" };
            ensure!(
                effect
                    == if result == Some(1) {
                        "sleep completed; branch true"
                    } else {
                        "sleep yielded"
                    },
                "Sleep result differs at {i}"
            );
            outcome
        } else if opcode == 7 && result == Some(1) {
            let mode = u16::from_le_bytes([b.operands[2], b.operands[3]]);
            ensure!(
                matches!(mode, 0 | 2)
                    && effect == format!("Refresh mode {mode} applied; branch true"),
                "Refresh mode/result differs at {i}"
            );
            "true"
        } else if opcode == 8 && result == Some(1) {
            let body = effect
                .strip_prefix("original RNG modulo ")
                .and_then(|s| s.strip_suffix("; branch true"))
                .context("unqualified RNG effect")?;
            let (range, value) = body
                .split_once(" → ")
                .context("RNG effect lacks bound/draw")?;
            let range: i16 = range.parse().context("RNG effect bound malformed")?;
            let value: u32 = value.parse().context("RNG effect draw malformed")?;
            ensure!(
                range > 0 && value < range as u32,
                "RNG effect outside observed positive-bound profile"
            );
            "true"
        } else if matches!(opcode, 1 | 2 | 31) && matches!(result, Some(0 | 1)) {
            let outcome = if result == Some(1) { "true" } else { "false" };
            let prefix = match opcode {
                1 => "native lot classification ",
                2 => "expression op ",
                31 => "Set To Next ",
                _ => unreachable!(),
            };
            ensure!(
                (effect.starts_with(prefix)
                    || opcode == 2
                        && effect.starts_with("scope ")
                        && effect.contains("; compare with original RHS "))
                    && effect.ends_with(&format!("; branch {outcome}")),
                "primitive branch differs at {i}"
            );
            outcome
        } else {
            bail!("unqualified primitive trace shape at {i}")
        };
        if outcome == "yield" {
            yields.push(a["object"].clone());
        }
        let mut row = expected;
        row["outcome"] = outcome.into();
        rows.push(row);
    }
    Ok(json!({"instructions":rows.len(),"yield_objects":yields,"rows":rows}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> (Value, Vec<sims_vm::Trace>) {
        let native = json!([{ "visit":1,"object":7,"depth":1,"source":"x","tree":4096,"node":0,"opcode":0,"operands":[0,0,0,0,0,0,0,0],"primitive_result_raw":1 },
            { "visit":2,"object":7,"depth":1,"source":"x","tree":4096,"node":1,"opcode":0,"operands":[0,0,0,0,0,0,0,0],"primitive_result_raw":2 }]);
        let rust = (0..2)
            .map(|i| sims_vm::Trace {
                tick: 4,
                thread: 7,
                depth: 1,
                location: sims_vm::Location {
                    source: "x".into(),
                    bhav: 4096,
                    instruction: i,
                },
                opcode: 0,
                operands: [0; 8],
                effect: if i == 0 {
                    "sleep completed; branch true"
                } else {
                    "sleep yielded"
                }
                .into(),
            })
            .collect();
        (native, rust)
    }
    #[test]
    fn preserves_original_order_and_rejects_unknown_or_changed_effects() {
        let (n, r) = sample();
        assert_eq!(compare(&n, &r, 4).unwrap()["yield_objects"], json!([7]));
        let mut bad = r.clone();
        bad.swap(0, 1);
        assert!(compare(&n, &bad, 4).is_err());
        let mut bad = r.clone();
        bad[1].effect = "sleep completed; branch true".into();
        assert!(compare(&n, &bad, 4).is_err());
        let mut bad = n.clone();
        bad[1]["opcode"] = 99.into();
        assert!(compare(&bad, &r, 4).is_err());
        let mut bad = n.clone();
        bad[1]["visit"] = 1.into();
        assert!(compare(&bad, &r, 4).is_err());
        assert!(compare(&n, &r, 5).is_err());
    }

    #[test]
    fn refresh_branch_requires_native_success_and_the_actual_mode() {
        let (mut native, mut rust) = sample();
        native[0]["opcode"] = 7.into();
        rust[0].opcode = 7;
        rust[0].effect = "Refresh mode 0 applied; branch true".into();
        assert!(compare(&native, &rust, 4).is_ok());
        rust[0].effect = "Refresh mode 2 applied; branch true".into();
        assert!(compare(&native, &rust, 4).is_err());
        rust[0].effect = "Refresh mode 0 applied; branch true".into();
        native[0]["primitive_result_raw"] = 0.into();
        assert!(compare(&native, &rust, 4).is_err());
    }

    #[test]
    fn rng_branch_requires_original_success_and_a_valid_bounded_draw() {
        let (mut native, mut rust) = sample();
        native[0]["opcode"] = 8.into();
        rust[0].opcode = 8;
        rust[0].effect = "original RNG modulo 20 → 9; branch true".into();
        assert!(compare(&native, &rust, 4).is_ok());
        native[0]["primitive_result_raw"] = 0.into();
        assert!(compare(&native, &rust, 4).is_err());
        native[0]["primitive_result_raw"] = 1.into();
        rust[0].effect = "original RNG modulo 20 → 20; branch true".into();
        assert!(compare(&native, &rust, 4).is_err());
        rust[0].effect = "unknown RNG; branch true".into();
        assert!(compare(&native, &rust, 4).is_err());
    }

    #[test]
    fn dispatcher_entry_does_not_invent_numeric_call_returns() {
        let (mut native, mut rust) = sample();
        native[0]["opcode"] = 424.into();
        native[1]["depth"] = 2.into();
        native[1]["tree"] = 424.into();
        native[1]["node"] = 0.into();
        rust[0].opcode = 424;
        rust[0].effect = "call x BHAV 0x01a8 resident:424".into();
        rust[1].depth = 2;
        rust[1].location.bhav = 424;
        rust[1].location.instruction = 0;
        assert!(compare(&native, &rust, 4).is_err());
        for row in native.as_array_mut().unwrap() {
            let object = row.as_object_mut().unwrap();
            let visit = object.remove("visit").unwrap();
            object.insert("stop_index".into(), visit);
        }
        assert_eq!(
            compare_dispatches(&native, &rust, 4).unwrap()["native_call_return_values_compared"],
            false
        );
        native[0]["call_result_raw"] = 1.into();
        let observed = compare_dispatches(&native, &rust, 4).unwrap();
        assert_eq!(observed["native_call_return_values_compared"], true);
        assert_eq!(observed["native_call_entry_statuses"]["calls"], 1);
        assert_eq!(observed["native_call_entry_statuses"]["observed"], 1);
        native[0]["call_result_raw"] = 0.into();
        assert!(compare_dispatches(&native, &rust, 4).is_err());
        native[0].as_object_mut().unwrap().remove("call_result_raw");
        rust[1].depth = 1;
        assert!(compare_dispatches(&native, &rust, 4).is_err());
    }

    #[test]
    fn retained_route_yield_is_checked_against_native_result() {
        let (mut native, mut rust) = sample();
        native[0]["opcode"] = 27.into();
        native[0]["primitive_result_raw"] = 2.into();
        rust[0].opcode = 27;
        rust[0].effect = "await route service response".into();
        assert!(compare(&native, &rust, 4).is_ok());
        native[0]["primitive_result_raw"] = 1.into();
        assert!(compare(&native, &rust, 4).is_err());
    }
    #[test]
    fn requires_original_subroutine_entry_and_depth() {
        let (mut n, mut r) = sample();
        n[0]["opcode"] = 424.into();
        n[0]["call_result_raw"] = 1.into();
        n[1]["depth"] = 2.into();
        n[1]["tree"] = 424.into();
        n[1]["node"] = 0.into();
        r[0].opcode = 424;
        r[0].effect = "call x BHAV 0x01a8 resident:424".into();
        r[1].depth = 2;
        r[1].location.bhav = 424;
        r[1].location.instruction = 0;
        assert!(compare(&n, &r, 4).is_ok());
        r[1].depth = 1;
        assert!(compare(&n, &r, 4).is_err());
    }
}
