//! Replacement request domains and explicit native Thread equality obligations.
use super::{Result, Saved, Value, fact, json, obligation};
use sims_vm::Thread;
pub fn evaluate(saved: &Saved) -> Result<Value> {
    let vm = &saved.simulation.state.vm;
    let mut rows = vec![];
    fact(
        "scheduled_object_visit",
        "unpublished_replacement_visit",
        vm.scheduled_object_visit.is_none(),
        &mut rows,
    );
    fact(
        "player_menu",
        "unpublished_replacement_query",
        vm.player_menu.is_none(),
        &mut rows,
    );
    fact(
        "lot_rng_scope",
        "completed_native_stream_scope",
        vm.lot_rng_scope.is_none(),
        &mut rows,
    );
    fact(
        "native_accessory_host",
        "replacement_native_asset_dispatch",
        vm.native_accessory_host,
        &mut rows,
    );
    for (field, comparison) in [
        (
            "world",
            "complete_World_contract_and_nested_native_equality",
        ),
        ("tick", "core/tick"),
        ("rng_state", "core/rng/global"),
        ("lot_rng_state", "core/rng/lot"),
        ("crt_rng_state", "core/rng/CRT"),
        (
            "threads",
            "native_thread_fields_and_unpublished_call_contract",
        ),
        ("object_threads", "retained_epoch/object_thread_bindings"),
        ("lifecycles", "retained_epoch/explicit_lifecycle_bindings"),
    ] {
        obligation(field, comparison, &mut rows);
    }
    fact(
        "named_calls",
        "unpublished_replacement_named_calls",
        vm.named_calls.is_empty(),
        &mut rows,
    );
    fact(
        "sounds",
        "qualified_inactive_original_effects",
        vm.sounds.is_empty(),
        &mut rows,
    );
    fact(
        "effects",
        "qualified_inactive_original_effects",
        vm.effects.is_empty(),
        &mut rows,
    );
    let (next_thread, next_request) = vm.completed_delivery_counters();
    fact(
        "next_thread",
        "replacement_identity_epoch",
        next_thread > vm.threads.keys().copied().max().unwrap_or(0),
        &mut rows,
    );
    rows.last_mut().unwrap()["value"] = next_thread.into();
    fact(
        "next_request",
        "replacement_identity_epoch",
        next_request > 0,
        &mut rows,
    );
    rows.last_mut().unwrap()["value"] = next_request.into();
    anyhow::ensure!(rows.len() == 17, "VM field coverage inventory changed");
    let mut threads = vec![];
    for (&id, thread) in &vm.threads {
        let Thread {
            service,
            dispatch,
            autonomy,
            find_best,
            completed_actions,
            last_result,
            callback,
            callback_result,
            stack,
            frame_memory,
            retained_frame_arena,
            check,
            interrupted,
            status,
        } = thread;
        let mut fields = vec![];
        macro_rules! absent { ($($f:ident),+ $(,)?)=>{$(
            fact(stringify!($f),"unpublished_replacement_thread_call",$f.is_none(),&mut fields);
        )+}; }
        absent!(service, dispatch, autonomy, callback, callback_result);
        fact(
            "find_best",
            "unpublished_replacement_query",
            find_best.is_empty(),
            &mut fields,
        );
        fact(
            "completed_actions",
            "unpublished_replacement_results",
            completed_actions.is_empty(),
            &mut fields,
        );
        let _ = (
            last_result,
            stack,
            frame_memory,
            retained_frame_arena,
            check,
            interrupted,
            status,
        );
        for (field, comparison) in [
            ("last_result", "core/last_result"),
            ("stack", "core/frames+native_arena_headers"),
            ("frame_memory", "frame_arena/all_words"),
            ("retained_frame_arena", "retained_epoch/raw_context_owner"),
            ("check", "retained_epoch/thread_controls/check"),
            ("interrupted", "retained_epoch/thread_controls/interrupted"),
            ("status", "retained_epoch/native_registry_reentry"),
        ] {
            obligation(field, comparison, &mut fields);
        }
        anyhow::ensure!(
            fields.len() == 14,
            "Thread field coverage inventory changed"
        );
        threads.push(json!({"thread":id,"fields":fields}));
    }
    let differences = rows
        .iter()
        .chain(threads.iter().flat_map(|t| t["fields"].as_array().unwrap()))
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"threads":threads,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All17 VM root fields and14 fields per Thread classified. Fresh Rust calls and IDs do not erase native pending frames. Fixed original context/control/status equality obligations remain mandatory."}),
    )
}
