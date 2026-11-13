//! Recorder-local accounting. No mutex is held across owner work, I/O, channel
//! delivery, publication, serialization or waits. A snapshot is never quiescence.
use sims_runtime::original_session::player_input::Observation;
use serde::Serialize;
use std::{collections::{BTreeMap, BTreeSet}, sync::{Arc, Mutex, MutexGuard}};
pub(crate) const TAG: &str = "sims-window-loaded-input-observation-v2";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Install { Absent, Unattributed, Pending, Installed, Failed, Unknown }
#[derive(Clone, Serialize)]
struct Slot {
    lifetime: Option<u64>,
    present: bool,
    input: Option<Observation>,
    install: Install,
}
impl Default for Slot {
    fn default() -> Self { Self { lifetime: None, present: false, input: None, install: Install::Absent } }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkPhase { Applying, OwnerPrepared }
struct OpenWork {
    lifetime: u64,
    _entry_transition: u64,
    changed: bool,
    phase: WorkPhase,
}
#[derive(Default)]
struct Ledger {
    lifetime_ordinal: u64,
    transition_ordinal: u64,
    work_ordinal: u64,
    live: Option<u64>,
    slot: Slot,
    last_recorded_owner: Option<Slot>,
    open: BTreeMap<u64, OpenWork>,
    ui_retirement_intent_lifetime: Option<u64>,
    request_channel_closed_after_ui_retirement: bool,
    retired_lifetime: Option<u64>,
    terminal_snapshot: Option<Snapshot>,
    invalid: BTreeSet<&'static str>,
    last_operation: Option<&'static str>,
    closed: bool,
}
#[derive(Clone, Default)]
pub(crate) struct Account { ledger: Arc<Mutex<Ledger>> }
#[derive(Clone)]
pub(crate) struct Monitor { account: Account, lifetime: Option<u64> }
/// A unique production Viewer lifetime. Intentionally non-Clone.
pub(crate) struct Registration { account: Account, lifetime: Option<u64> }
/// Owned guard has no Viewer borrow and can span result send and notification.
pub(crate) struct WorkScope {
    account: Account,
    scope: Option<u64>,
    completed: bool,
}
#[derive(Clone, Serialize)]
pub(crate) struct Snapshot {
    extension: &'static str,
    viewer_lifetime_ordinal: u64,
    transition_ordinal: u64,
    work_ordinal: u64,
    live_lifetime: Option<u64>,
    accounted_slot: Slot,
    last_recorded_owner: Option<Slot>,
    active_work: Vec<u64>,
    invalidation_reasons: Vec<&'static str>,
    last_operation: Option<&'static str>,
    closed: bool,
    ownership_status: &'static str,
    unresolved_in_flight: bool,
    owner_preparation_complete: bool,
    owner_mutation_pending: bool,
    publication_delivery_pending: bool,
    capture_boundary: &'static str,
    ui_retirement_intent_lifetime: Option<u64>,
    request_channel_closed_after_ui_retirement: bool,
    retired_lifetime: Option<u64>,
    current_owner_asserted: bool,
    eligible: bool,
    snapshot_only: bool,
    worker_finalized: bool,
    quiescence_proven: bool,
}
impl Account {
    fn lock(&self) -> MutexGuard<'_, Ledger> {
        match self.ledger.lock() {
            Ok(ledger) => ledger,
            Err(poison) => {
                // Recover only to make UNKNOWN sticky and to close snapshots.
                // Poison is never cleared and cannot regain certification.
                let mut ledger = poison.into_inner();
                if !ledger.closed { ledger.invalid.insert("ledger_poisoned"); }
                ledger
            }
        }
    }
    pub(crate) fn register(&self) -> Registration {
        let mut ledger = self.lock();
        let lifetime = if ledger.closed { None }
        else if let Some(next) = ledger.lifetime_ordinal.checked_add(1) {
            if ledger.live.is_some() { ledger.invalid.insert("overlapping_viewer_registration"); }
            if ledger.ui_retirement_intent_lifetime.is_some() {
                ledger.invalid.insert("registration_after_ui_retirement");
            }
            if ledger.open.values().any(|work| work.phase == WorkPhase::OwnerPrepared) {
                ledger.invalid.insert("registration_after_owner_prepared");
            }
            ledger.ui_retirement_intent_lifetime = None;
            ledger.request_channel_closed_after_ui_retirement = false;
            ledger.lifetime_ordinal = next;
            ledger.live = Some(next);
            // Lifetime bookkeeping is not a Session slot transition.
            ledger.slot = Slot { lifetime: Some(next), ..Slot::default() };
            Some(next)
        } else { ledger.invalid.insert("viewer_lifetime_overflow"); None };
        Registration { account: self.clone(), lifetime }
    }
    fn snapshot(&self, close: bool) -> Snapshot {
        let mut ledger = self.lock();
        // The first footer freezes every observed fact, including open phases
        // and channel closure. Subsequent activity cannot retrofit that footer.
        if let Some(snapshot) = &ledger.terminal_snapshot { return snapshot.clone(); }
        let unresolved = !ledger.open.is_empty();
        let applying = ledger.open.values().any(|work| work.phase == WorkPhase::Applying);
        let delivery_pending = ledger.open.values().any(|work| work.phase == WorkPhase::OwnerPrepared);
        let preparation_complete = !applying && (unresolved || ledger.last_operation == Some("success"));
        let boundary = if unresolved && preparation_complete { "owner_prepared_before_read_only_publication" }
            else if preparation_complete { "completed_owner_work" }
            else { "owner_preparation_incomplete" };
        let status = if !ledger.invalid.is_empty() { "unknown_invalid" }
            else if applying { "unresolved_in_flight" }
            else if unresolved { "owner_prepared_delivery_pending" }
            else if ledger.live.is_none() { "no_live_registration" }
            else { "accounted_at_snapshot" };
        // Eligibility describes only this exact owner-prepared input capture.
        // It promises no future publish/send/notify success or worker exit.
        let current_owner = !close && preparation_complete && ledger.invalid.is_empty()
            && ledger.live == ledger.slot.lifetime && ledger.live.is_some();
        let eligible = current_owner && ledger.slot.present && ledger.slot.install == Install::Installed
            && matches!(ledger.last_operation, Some("owner_prepared" | "success"))
            && ledger.slot.input.as_ref().is_some_and(Observation::eligible_input);
        let snapshot = Snapshot {
            extension: TAG, viewer_lifetime_ordinal: ledger.lifetime_ordinal,
            transition_ordinal: ledger.transition_ordinal, work_ordinal: ledger.work_ordinal,
            live_lifetime: ledger.live, accounted_slot: ledger.slot.clone(),
            last_recorded_owner: ledger.last_recorded_owner.clone(),
            active_work: ledger.open.keys().copied().collect(),
            invalidation_reasons: ledger.invalid.iter().copied().collect(),
            last_operation: ledger.last_operation, closed: close,
            ownership_status: status, unresolved_in_flight: unresolved,
            owner_preparation_complete: preparation_complete, owner_mutation_pending: applying,
            publication_delivery_pending: delivery_pending, capture_boundary: boundary,
            ui_retirement_intent_lifetime: ledger.ui_retirement_intent_lifetime,
            request_channel_closed_after_ui_retirement: ledger.request_channel_closed_after_ui_retirement,
            retired_lifetime: ledger.retired_lifetime,
            current_owner_asserted: current_owner, eligible, snapshot_only: true,
            worker_finalized: false, quiescence_proven: false,
        };
        if close { ledger.closed = true; ledger.terminal_snapshot = Some(snapshot.clone()); }
        snapshot
    }
    #[cfg(test)]
    pub(crate) fn inspect_for_test(&self) -> serde_json::Value {
        serde_json::to_value(self.snapshot(false)).unwrap()
    }
    pub(crate) fn terminal(&self) -> serde_json::Value {
        // Serialize after releasing the one ledger mutex.
        serde_json::to_value(self.snapshot(true)).unwrap_or(serde_json::Value::Null)
    }
}
impl Monitor {
    /// UI drain intent only: this neither completes an open guard nor joins.
    pub(crate) fn ui_retirement_intent(&self) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        if self.lifetime.is_none() || ledger.live != self.lifetime {
            ledger.invalid.insert("ui_retirement_wrong_lifetime"); return;
        }
        ledger.ui_retirement_intent_lifetime = self.lifetime;
    }
    pub(crate) fn request_channel_closed(&self) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        if self.lifetime.is_some() && ledger.live == self.lifetime
            && ledger.ui_retirement_intent_lifetime == self.lifetime {
            ledger.request_channel_closed_after_ui_retirement = true;
        } else {
            ledger.invalid.insert("request_channel_disconnected");
            ledger.last_operation = Some("failed");
        }
    }
    pub(crate) fn invalidate(&self, reason: &'static str) {
        let mut ledger = self.account.lock();
        // Closure freezes the snapshot history. Later activity cannot certify.
        if !ledger.closed { ledger.invalid.insert(reason); ledger.last_operation = Some("failed"); }
    }
}
impl Registration {
    pub(crate) fn monitor(&self) -> Monitor { Monitor { account: self.account.clone(), lifetime: self.lifetime } }
    pub(crate) fn work(&self) -> WorkScope {
        let mut ledger = self.account.lock();
        let scope = if ledger.closed { None }
        else if self.lifetime.is_none() || ledger.live != self.lifetime {
            ledger.invalid.insert("unreconciled_viewer_lifetime"); None
        } else if let Some(next) = ledger.work_ordinal.checked_add(1) {
            ledger.work_ordinal = next;
            if ledger.ui_retirement_intent_lifetime == self.lifetime {
                ledger.invalid.insert("work_after_ui_retirement");
            }
            let transition = ledger.transition_ordinal;
            if ledger.open.values().any(|work| work.phase == WorkPhase::OwnerPrepared) {
                ledger.invalid.insert("work_started_after_owner_prepared");
            }
            ledger.open.insert(next, OpenWork { lifetime: self.lifetime.unwrap(),
                _entry_transition: transition, changed: false, phase: WorkPhase::Applying });
            ledger.last_operation = Some("in_flight");
            Some(next)
        } else { ledger.invalid.insert("work_scope_overflow"); None };
        WorkScope { account: self.account.clone(), scope, completed: false }
    }
    /// Called immediately after actual assignment, with no intervening native
    /// or fallible owner work. Caller already holds an open work scope.
    pub(crate) fn transition(&self, old_present: bool, new_present: bool, input: Option<&Observation>) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        if self.lifetime.is_none() || ledger.live != self.lifetime {
            ledger.invalid.insert("transition_wrong_lifetime");
        }
        if ledger.ui_retirement_intent_lifetime.is_some() && ledger.ui_retirement_intent_lifetime == self.lifetime {
            ledger.invalid.insert("transition_after_ui_retirement");
        }
        if ledger.open.values().any(|work| work.phase == WorkPhase::OwnerPrepared) {
            ledger.invalid.insert("transition_after_owner_prepared");
        }
        if !ledger.open.values().any(|work| Some(work.lifetime) == self.lifetime) {
            ledger.invalid.insert("transition_without_work_scope");
        }
        if ledger.slot.present != old_present { ledger.invalid.insert("slot_account_mismatch"); }
        if old_present || new_present {
            // Track the actual replacement independently of counter success.
            for work in ledger.open.values_mut() {
                if Some(work.lifetime) == self.lifetime { work.changed = true; }
            }
            if let Some(next) = ledger.transition_ordinal.checked_add(1) { ledger.transition_ordinal = next; }
            else { ledger.invalid.insert("transition_overflow"); }
        }
        ledger.slot = Slot {
            lifetime: self.lifetime, present: new_present, input: input.cloned(),
            install: if !new_present { Install::Absent } else if input.is_some() { Install::Pending } else { Install::Unattributed },
        };
        if new_present { ledger.last_recorded_owner = Some(ledger.slot.clone()); }
    }
    pub(crate) fn preparation_result(&self, success: bool) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        if ledger.live != self.lifetime || self.lifetime.is_none() {
            ledger.invalid.insert("preparation_wrong_lifetime"); return;
        }
        if ledger.open.values().any(|work| work.phase == WorkPhase::OwnerPrepared) {
            ledger.invalid.insert("preparation_after_owner_prepared");
        }
        // A precommit failure must not relabel an old installed owner.
        if ledger.slot.install == Install::Pending {
            ledger.slot.install = if success { Install::Installed } else { Install::Failed };
            ledger.last_recorded_owner = Some(ledger.slot.clone());
        }
    }
    pub(crate) fn capture(&self, actual_present: bool, actual_input: Option<&Observation>) -> serde_json::Value {
        let snapshot = self.account.snapshot(false);
        let matches = snapshot.live_lifetime == self.lifetime
            && self.lifetime.is_some() && snapshot.accounted_slot.present == actual_present
            && snapshot.accounted_slot.input.as_ref() == actual_input;
        let mut value = serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null);
        value["capture_owner_matches_account"] = matches.into();
        value["current_owner_asserted"] = (snapshot.current_owner_asserted && matches).into();
        value["eligible"] = (snapshot.eligible && matches).into();
        value
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        if self.lifetime.is_some() && ledger.live == self.lifetime {
            ledger.retired_lifetime = self.lifetime;
            ledger.live = None;
            ledger.last_operation = Some("viewer_registration_closed");
        }
        // Closure is explicitly not a Session replacement or worker proof.
    }
}
impl WorkScope {
    /// Marks only this guard, after all mutable owner preparation and before
    /// read-only publication. Children and sticky failures remain untouched.
    pub(crate) fn owner_prepared(&mut self) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        let Some(scope) = self.scope else {
            ledger.invalid.insert("owner_prepared_without_work_scope"); return;
        };
        let Some(work) = ledger.open.get(&scope) else {
            ledger.invalid.insert("owner_prepared_unknown_work_scope"); return;
        };
        if ledger.live != Some(work.lifetime) {
            ledger.invalid.insert("owner_prepared_wrong_lifetime"); return;
        }
        ledger.open.get_mut(&scope).unwrap().phase = WorkPhase::OwnerPrepared;
        if ledger.invalid.is_empty() && ledger.open.values().all(|work| work.phase == WorkPhase::OwnerPrepared) {
            ledger.last_operation = Some("owner_prepared");
        }
    }
    pub(crate) fn complete(mut self, success: bool) {
        self.close(success, false);
        self.completed = true;
    }
    fn close(&self, success: bool, abandoned: bool) {
        let mut ledger = self.account.lock();
        if ledger.closed { return; }
        let Some(scope) = self.scope else {
            if !success {
                ledger.last_operation = Some("failed");
                if ledger.slot.input.is_some() {
                    ledger.slot.install = if ledger.slot.install == Install::Pending { Install::Failed } else { Install::Unknown };
                    ledger.last_recorded_owner = Some(ledger.slot.clone());
                }
            }
            return;
        };
        let Some(work) = ledger.open.remove(&scope) else {
            ledger.invalid.insert("unknown_work_scope"); return;
        };
        if !success {
            ledger.last_operation = Some("failed");
            // Sticky work failure is conservative: later same-owner frames
            // cannot erase publication/delivery/panic uncertainty.
            ledger.invalid.insert(if abandoned { "abandoned_or_panicked_work" } else { "owner_work_failed" });
            if ledger.slot.lifetime == Some(work.lifetime) && work.changed
                && ledger.slot.input.is_some() {
                ledger.slot.install = Install::Failed;
                ledger.last_recorded_owner = Some(ledger.slot.clone());
            }
        } else if ledger.invalid.is_empty() {
            if ledger.open.is_empty() { ledger.last_operation = Some("success"); }
            else if ledger.open.values().all(|work| work.phase == WorkPhase::OwnerPrepared) {
                ledger.last_operation = Some("owner_prepared");
            }
        }
    }
}
impl Drop for WorkScope {
    fn drop(&mut self) { if !self.completed { self.close(false, true); } }
}
pub(crate) fn unregistered(actual_present: bool, origin: Option<&Observation>) -> serde_json::Value {
    serde_json::json!({"extension":TAG,"ownership_status":"unregistered","eligible":false,
        "session_present":actual_present,"candidate_input":origin,
        "owner_preparation_complete":false,"owner_mutation_pending":false,
        "publication_delivery_pending":false,"capture_boundary":"owner_preparation_incomplete",
        "ui_retirement_intent_lifetime":null,"request_channel_closed_after_ui_retirement":false,
        "retired_lifetime":null,"snapshot_only":true,"worker_finalized":false,"quiescence_proven":false})
}
#[cfg(test)]
#[path = "loaded_input/tests.rs"]
mod tests;
