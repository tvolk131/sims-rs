//! Authored component controls, UNRUN. These inspect the ledger itself; they
//! do not manufacture journal events, external save identity or eligibility.
use super::*;
fn count(account: &Account) -> u64 { account.inspect_for_test()["transition_ordinal"].as_u64().unwrap() }
#[test]
fn every_actual_slot_change_is_counted_even_without_draws_and_none_to_none_is_not() {
    let account = Account::default(); let registration = account.register();
    let work = registration.work();
    registration.transition(false, false, None); assert_eq!(count(&account), 0);
    registration.transition(false, true, None); assert_eq!(count(&account), 1);
    registration.transition(true, true, None); assert_eq!(count(&account), 2);
    registration.transition(true, false, None); assert_eq!(count(&account), 3);
    registration.transition(false, true, None); assert_eq!(count(&account), 4);
    work.complete(true);
    assert_eq!(account.inspect_for_test()["eligible"], false);
}
#[test]
fn new_lifetimes_do_not_reset_ordinals_and_overlap_is_sticky_unknown() {
    let account = Account::default();
    let first = account.register(); let work = first.work();
    first.transition(false, true, None); work.complete(true); drop(first);
    let second = account.register();
    assert_eq!(account.inspect_for_test()["viewer_lifetime_ordinal"], 2);
    assert_eq!(count(&account), 1);
    let overlapping = account.register(); drop(overlapping); drop(second);
    let snapshot = account.terminal();
    assert!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|r| r == "overlapping_viewer_registration"));
    assert_eq!(snapshot["eligible"], false);
}
#[test]
fn nested_scopes_and_terminal_close_never_claim_a_current_owner_or_quiescence() {
    let account = Account::default(); let registration = account.register();
    let outer = registration.work(); let inner = registration.work();
    registration.transition(false, true, None); inner.complete(true);
    let snapshot = account.terminal();
    assert_eq!(snapshot["unresolved_in_flight"], true);
    assert_eq!(snapshot["current_owner_asserted"], false);
    assert_eq!(snapshot["active_work"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["worker_finalized"], false);
    assert_eq!(snapshot["quiescence_proven"], false);
    let count = snapshot["transition_ordinal"].clone();
    outer.complete(true);
    let late = registration.work(); registration.transition(true, false, None); late.complete(true);
    let _late_registration = account.register();
    assert_eq!(account.inspect_for_test()["transition_ordinal"], count);
    assert_eq!(account.inspect_for_test()["active_work"], snapshot["active_work"]);
    assert_eq!(account.inspect_for_test()["eligible"], false);
}
#[test]
fn dropped_scope_poison_and_all_three_counter_overflows_fail_closed() {
    let account = Account::default(); let registration = account.register();
    drop(registration.work());
    assert_eq!(account.inspect_for_test()["ownership_status"], "unknown_invalid");
    for field in 0..3 {
        let account = Account::default(); let registration = account.register();
        { let mut ledger = account.lock(); match field {
            0 => ledger.transition_ordinal = u64::MAX,
            1 => ledger.work_ordinal = u64::MAX,
            2 => ledger.lifetime_ordinal = u64::MAX,
            _ => unreachable!(),
        }}
        let work = registration.work(); registration.transition(false, true, None); work.complete(true);
        if field == 2 { drop(registration); drop(account.register()); }
        let snapshot = account.terminal();
        assert_eq!(snapshot["eligible"], false);
        assert_eq!(snapshot["ownership_status"], "unknown_invalid");
        let reason = ["transition_overflow", "work_scope_overflow", "viewer_lifetime_overflow"][field];
        assert!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|value| value == reason));
    }
    let account = Account::default(); let poisoned = account.clone();
    let _ = std::panic::catch_unwind(move || { let _ledger = poisoned.ledger.lock().unwrap(); panic!("authored poison"); });
    assert_eq!(account.terminal()["ownership_status"], "unknown_invalid");
}
#[test]
fn a_zero_open_count_does_not_prove_quiescence_or_worker_finalization() {
    let account = Account::default(); let registration = account.register();
    let work = registration.work(); work.complete(true);
    let terminal = account.terminal();
    assert!(terminal["active_work"].as_array().unwrap().is_empty());
    assert_eq!(terminal["snapshot_only"], true);
    assert_eq!(terminal["worker_finalized"], false);
    assert_eq!(terminal["quiescence_proven"], false);
    assert_eq!(terminal["eligible"], false);
}

#[test]
fn preparing_one_guard_never_prepares_its_open_child_and_footer_freezes_every_phase() {
    let account = Account::default(); let registration = account.register();
    let mut outer = registration.work(); let inner = registration.work();
    outer.owner_prepared();
    let nested = account.inspect_for_test();
    assert_eq!(nested["owner_preparation_complete"], false);
    assert_eq!(nested["owner_mutation_pending"], true);
    assert_eq!(nested["publication_delivery_pending"], true);
    assert_eq!(nested["unresolved_in_flight"], true);
    assert_eq!(nested["ownership_status"], "unresolved_in_flight");
    assert_eq!(nested["current_owner_asserted"], false);
    inner.complete(true);
    let prepared = account.inspect_for_test();
    assert_eq!(prepared["owner_preparation_complete"], true);
    assert_eq!(prepared["owner_mutation_pending"], false);
    assert_eq!(prepared["publication_delivery_pending"], true);
    assert_eq!(prepared["unresolved_in_flight"], true);
    assert_eq!(prepared["ownership_status"], "owner_prepared_delivery_pending");
    assert_eq!(prepared["capture_boundary"], "owner_prepared_before_read_only_publication");
    assert_eq!(prepared["last_operation"], "owner_prepared");
    let footer = account.terminal();
    assert_eq!(footer["current_owner_asserted"], false);
    assert_eq!(footer["eligible"], false);
    outer.complete(true);
    registration.monitor().ui_retirement_intent();
    registration.monitor().request_channel_closed();
    drop(registration);
    assert_eq!(account.terminal(), footer);
    assert_eq!(account.inspect_for_test(), footer);
}

#[test]
fn preparation_is_lifetime_bound_and_never_clears_failures_or_prepared_drop() {
    let account = Account::default(); let registration = account.register();
    let mut work = registration.work();
    registration.monitor().invalidate("authored_sticky_failure");
    work.owner_prepared(); work.complete(true);
    let snapshot = account.inspect_for_test();
    assert_eq!(snapshot["eligible"], false);
    assert!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "authored_sticky_failure"));
    let mut dropped = registration.work(); dropped.owner_prepared(); drop(dropped);
    assert!(account.inspect_for_test()["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "abandoned_or_panicked_work"));
    let mut stale = registration.work(); drop(registration);
    let _next = account.register(); stale.owner_prepared();
    assert!(account.inspect_for_test()["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "owner_prepared_wrong_lifetime"));
    stale.complete(true);
}

#[test]
fn mutation_and_new_applying_work_after_preparation_fail_closed_without_suppressing_accounting() {
    for action in 0..3 {
        let account = Account::default(); let registration = account.register();
        let mut work = registration.work(); work.owner_prepared();
        let reason = match action {
            0 => { registration.transition(false, true, None); "transition_after_owner_prepared" }
            1 => { registration.preparation_result(true); "preparation_after_owner_prepared" }
            2 => { registration.work().complete(true); "work_started_after_owner_prepared" }
            _ => unreachable!(),
        };
        let snapshot = account.inspect_for_test();
        assert!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == reason));
        assert_eq!(snapshot["eligible"], false);
        if action == 0 { assert_eq!(snapshot["transition_ordinal"], 1); assert_eq!(snapshot["accounted_slot"]["present"], true); }
        work.complete(true);
        assert_eq!(account.inspect_for_test()["eligible"], false);
    }
}

#[test]
fn request_channel_closure_requires_matching_ui_retirement_and_retirement_does_not_complete_work() {
    for known in [false, true] {
        let account = Account::default(); let registration = account.register();
        let monitor = registration.monitor(); let mut work = registration.work(); work.owner_prepared();
        if known { monitor.ui_retirement_intent(); }
        monitor.request_channel_closed();
        let snapshot = account.inspect_for_test();
        assert_eq!(snapshot["request_channel_closed_after_ui_retirement"], known);
        assert_eq!(snapshot["ui_retirement_intent_lifetime"], if known { serde_json::json!(1) } else { serde_json::Value::Null });
        assert_eq!(snapshot["active_work"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["publication_delivery_pending"], true);
        assert_eq!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "request_channel_disconnected"), !known);
        work.complete(true); drop(registration);
        let retired = account.terminal();
        assert_eq!(retired["retired_lifetime"], 1); assert_eq!(retired["live_lifetime"], serde_json::Value::Null);
        assert_eq!(retired["worker_finalized"], false); assert_eq!(retired["quiescence_proven"], false);
    }
    let account = Account::default(); let first = account.register(); let stale = first.monitor();
    stale.ui_retirement_intent(); drop(first); let _next = account.register(); stale.request_channel_closed();
    assert_eq!(account.inspect_for_test()["request_channel_closed_after_ui_retirement"], false);
    assert!(account.inspect_for_test()["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "request_channel_disconnected"));
}

#[test]
fn ui_retirement_intent_cannot_silently_hide_later_mutation_or_a_new_registration() {
    let account = Account::default(); let registration = account.register();
    let monitor = registration.monitor(); monitor.ui_retirement_intent();
    let work = registration.work(); registration.transition(false, true, None); work.complete(true);
    monitor.request_channel_closed();
    let snapshot = account.inspect_for_test();
    assert_eq!(snapshot["request_channel_closed_after_ui_retirement"], true);
    for reason in ["work_after_ui_retirement", "transition_after_ui_retirement"] {
        assert!(snapshot["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == reason));
    }
    assert_eq!(snapshot["eligible"], false);
    drop(registration); let next = account.register();
    assert!(account.inspect_for_test()["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "registration_after_ui_retirement"));
    drop(next);
}
