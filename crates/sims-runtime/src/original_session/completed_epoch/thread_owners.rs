//! Source-bound native frame/context ownership plus replacement delivery state.
//! This closes specified comparison obligations, never whole Session admission.
use super::{Context, Result, Saved, Value, ensure, json, same};
use sims_vm::{
    Status,
    lifecycle::{Lifecycle, Phase},
    retained_frames::{ContextBinding, RetainedFrame, RetainedStack},
};
use std::collections::{BTreeMap, BTreeSet};

pub fn evaluate(saved: &Saved, reentry: Value, source: &Value, bytecode: &Value) -> Result<Value> {
    let vm = &saved.simulation.state.vm;
    ensure!(
        [88448, 88449].contains(&vm.tick)
            && source["schema"] == 1
            && source["tick"] == vm.tick
            && source["executes_target_code"] == false,
        "thread ownership source boundary differs"
    );
    let parts = crate::conformance::completed_reentry::import_parts(reentry)?;
    let records = bytecode
        .as_array()
        .context("native bytecode trace absent")?;
    ensure!(
        records.len() == 8
            && records
                .iter()
                .filter(|r| r["primitive_result_raw"] == 2)
                .count()
                == 4,
        "qualified native execution/yield cohort differs"
    );
    let rows = source["threads"]["objects"]
        .as_array()
        .context("actual native threads absent")?;
    let native: BTreeMap<i16, &Value> = rows
        .iter()
        .map(|r| {
            Ok((
                r["id"]
                    .as_i64()
                    .context("native object ID absent")?
                    .try_into()?,
                r,
            ))
        })
        .collect::<Result<_>>()?;
    ensure!(
        native.len() == rows.len()
            && native.keys().eq(vm.world.objects.keys())
            && native.len() == parts.entries.len(),
        "native thread owner cohort differs"
    );
    let mut comparisons = vec![];
    let next_thread = parts
        .entries
        .iter()
        .map(|e| e.adapter_thread)
        .max()
        .context("empty source threads")?
        + 1;
    let expected_bindings: BTreeMap<_, _> = parts
        .entries
        .iter()
        .map(|e| (e.object.unwrap(), e.adapter_thread))
        .collect();
    same(
        "object_threads",
        &expected_bindings,
        &vm.object_threads,
        &mut comparisons,
    )?;
    let mut lifecycles = BTreeMap::new();
    let mut thread_reports = vec![];
    let mut observed_run = BTreeSet::new();
    let mut next_request = 1;
    for row in records {
        if row["primitive_result_raw"] == 2 && row["opcode"] == 44 {
            next_request += 1;
        }
        observed_run.insert(
            row["object"]
                .as_i64()
                .context("native execution object absent")? as i16,
        );
    }
    ensure!(
        observed_run == BTreeSet::from([16, 18, 21, 278]),
        "native visited cohort changed"
    );
    for e in parts.entries {
        let id = e.object.context("source thread object absent")?;
        let source = &native[&id];
        let original = e
            .stack
            .clone()
            .into_thread(&parts.program, e.controls.clone())
            .map_err(anyhow::Error::msg)?;
        let Status::NativeRegistryReentry(owner) = &original.status else {
            anyhow::bail!("source reentry owner absent")
        };
        let definition = parts.objects[&id]
            .definition
            .as_ref()
            .context("native definition absent")?;
        lifecycles.insert(
            id,
            Lifecycle {
                owner: definition.source.clone(),
                definition: definition.object,
                init: owner.evidence().cached_init,
                main: owner.evidence().cached_main,
                phase: Phase::Main,
                thread: e.adapter_thread,
                run_init: owner.closed_group_init_permission(),
            },
        );
        let context = owner.evidence().source_binding.context;
        ensure!(
            source["resource_context"] == context,
            "native context association changed"
        );
        let mut frames = vec![];
        for f in source["frames"]
            .as_array()
            .context("native frames absent")?
        {
            ensure!(
                f["code_owner_context"] == context && f["continuation_word_raw"] == 0,
                "native invocation requires another source profile"
            );
            let tree: u16 = f["tree_raw"].as_u64().context("tree absent")?.try_into()?;
            ensure!(
                tree < 0x8000,
                "flagged native frame requires another source profile"
            );
            let (code, _) = parts
                .program
                .resolve(&definition.source, tree)
                .map_err(anyhow::Error::msg)?;
            frames.push(RetainedFrame {
                context_object: id,
                frame: sims_vm::Frame {
                    continuation: 0,
                    action_target: None,
                    action_icon: false,
                    code_owner: definition.source.clone(),
                    location: sims_vm::Location {
                        source: code.into(),
                        bhav: tree,
                        instruction: f["node_raw"].as_u64().context("node absent")?.try_into()?,
                    },
                    caller: id,
                    callee: id,
                    stack_object: serde_json::from_value(f["stack_object"].clone())?,
                    args: serde_json::from_value(f["arguments"].clone())?,
                    locals: serde_json::from_value(f["locals"].clone())?,
                    opaque_locals: BTreeSet::new(),
                    storage_offset: f["offset_words"]
                        .as_u64()
                        .context("offset absent")?
                        .try_into()?,
                },
            });
        }
        let arena = &source["arena"];
        let raw = super::super::completed_animations::bytes(
            arena["raw"].as_str().context("arena raw absent")?,
        )?;
        ensure!(
            arena["bytes"] == raw.len() && arena["sha256"] == sims_fixtures::digest(&raw),
            "native arena receipt differs"
        );
        let stack = RetainedStack::from_bytes(
            &parts.program,
            &raw,
            frames,
            vec![ContextBinding {
                object: id,
                definition: definition.clone(),
                native_context: context,
                registry_lifetime: Some(id as u32),
            }],
        )
        .map_err(anyhow::Error::msg)?;
        let mut controls = e.controls;
        controls.last_result = match source["last_result_raw"].as_u64() {
            Some(0) => false,
            Some(1) => true,
            _ => anyhow::bail!("native last-result bool differs"),
        };
        if vm.tick == 88449 && observed_run.contains(&id) {
            let last = records.iter().rev().find(|r| r["object"] == id).unwrap();
            ensure!(
                last["primitive_result_raw"] == 2,
                "visited native thread did not yield"
            );
            controls.status = if last["opcode"] == 44 {
                let request = records
                    .iter()
                    .take_while(|r| !std::ptr::eq(*r, last))
                    .filter(|r| r["opcode"] == 44)
                    .count() as u64
                    + 1;
                Status::AwaitingAnimation(sims_vm::AnimationRequest {
                    id: request,
                    origin: sims_vm::Location {
                        source: last["source"].as_str().context("source absent")?.into(),
                        bhav: serde_json::from_value(last["tree"].clone())?,
                        instruction: serde_json::from_value(last["node"].clone())?,
                    },
                    caller: id,
                    operands: serde_json::from_value(last["operands"].clone())?,
                })
            } else {
                ensure!(last["opcode"] == 0, "unqualified native yield kind");
                let wait = source["countdown_raw"]
                    .as_i64()
                    .context("sleep countdown absent")?;
                ensure!(wait >= 0, "passive Sleep source profile changed");
                Status::Sleeping {
                    wake_tick: 88448 + wait as u64 + 1,
                }
            };
        }
        let expected = stack
            .into_thread(&parts.program, controls)
            .map_err(anyhow::Error::msg)?;
        let actual = &vm.threads[&e.adapter_thread];
        let mut fields = vec![];
        same("complete_thread", &expected, actual, &mut fields)?;
        if expected != *actual {
            let a = serde_json::to_value(&expected)?;
            let b = serde_json::to_value(actual)?;
            fields.last_mut().unwrap()["different_fields"] = json!(
                a.as_object()
                    .unwrap()
                    .keys()
                    .filter(|k| a[*k] != b[*k])
                    .map(|k| json!({"field":k,"native_or_delivery":a[k],"rust":b[k]}))
                    .collect::<Vec<_>>()
            );
        }
        let object = &vm.world.objects[&id];
        let visited = vm.tick == 88449 && observed_run.contains(&id);
        same(
            "object.last_update",
            &visited.then_some(88448),
            &object.last_update,
            &mut fields,
        )?;
        same(
            "idle.last_run",
            &visited.then_some(88448),
            &object.idle.as_ref().context("idle absent")?.last_run,
            &mut fields,
        )?;
        if let Some(headline) = &object.headline {
            same(
                "headline.last_update",
                &visited.then_some(88448),
                &headline.last_update,
                &mut fields,
            )?;
        }
        thread_reports.push(json!({"object":id,"thread":e.adapter_thread,"comparisons":fields}));
    }
    same("lifecycles", &lifecycles, &vm.lifecycles, &mut comparisons)?;
    same(
        "delivery_counters",
        &(next_thread, if vm.tick == 88448 { 1 } else { next_request }),
        &vm.completed_delivery_counters(),
        &mut comparisons,
    )?;
    let differences = comparisons
        .iter()
        .chain(
            thread_reports
                .iter()
                .flat_map(|r| r["comparisons"].as_array().unwrap()),
        )
        .filter(|r| r["matched"] == false)
        .count();
    Ok(
        json!({"schema":1,"tick":vm.tick,"comparisons":comparisons,"threads":thread_reports,
        "difference_count":differences,"complete_native_Session_admitted":false,"native_parity_ticks":0,
        "scope":"Actual frame values, complete retained arena/context metadata, lifecycle binding and explicit fresh Rust delivery state. Replacement handles/timestamps are checked against source ordered yields, never compared to native pointers or historical timestamps. Action/invocation adapter absence still requires the source caller proof; no whole Session admission."}),
    )
}
