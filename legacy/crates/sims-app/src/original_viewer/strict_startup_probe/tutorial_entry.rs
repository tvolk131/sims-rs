use super::*;
use sims_vm::dialog::{Record, State};

pub(super) const HELP: &str = "Private simulated/headless one-response tutorial-entry diagnostic only. Use SIMS_STRICT_STARTUP_ROOT with the existing parent-pinned readonly snapshot and a fresh exclusive SIMS_STRICT_STARTUP_OUTPUT under this checkout's ignored cache. Run only original_viewer::strict_startup_probe::tutorial_entry::ordinary_strict_newbie_tutorial_entry with --exact --ignored --nocapture --test-threads=1 under a parent-owned whole-process timeout below 300 seconds with child cleanup. Actual Ready/install and production Toggle/Frame/DialogKey handlers only; at most one validated offered opening response. Production budgets and 600 Frame/16ms bounds unchanged. No native/physical input, Save, Load, Continue or tutorial completion. Every qualification remains false.";
const SCHEMA: &str = "sims-strict-tutorial-entry-probe-v1";
const SCOPE: &str = "actual Ready installed in production Viewer; ordinary Toggle and at most 600 simulated/headless Frame inputs at 16ms offsets, with at most one validated Message::DialogKey(13); acknowledgment requires the same retained record's result and modal release, not key submission; stop at first native tick or next modal/error/blocker; not tutorial, native-window, performance or alpha acceptance";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    id: u64,
    caller: sims_vm::ObjectId,
    source: String,
    origin: sims_vm::Location,
    operands: [u8; 8],
    stack_object: sims_vm::ObjectId,
    offered_results: Vec<u32>,
}

impl Identity {
    fn observed(record: &Record) -> Self {
        Self {
            id: record.id,
            caller: record.caller,
            source: record.source.clone(),
            origin: record.origin.clone(),
            operands: record.operands,
            stack_object: record.stack_object,
            offered_results: record.buttons.iter().map(|button| button.result).collect(),
        }
    }
}

#[derive(Default)]
pub(super) struct OneResponse {
    identity: Option<Identity>,
    acknowledged: bool,
    resume_requested: bool,
}

fn current_record(dialogs: &State) -> std::result::Result<&Record, &'static str> {
    let id = dialogs.modal.ok_or("opening_modal_absent")?;
    let mut records = dialogs.records.iter().filter(|record| record.id == id);
    let record = records.next().ok_or("opening_modal_record_absent")?;
    if records.next().is_some() {
        return Err("opening_modal_record_ambiguous");
    }
    Ok(record)
}

fn validate_enter(record: &Record) -> std::result::Result<(), &'static str> {
    if record.key_result(13) != Some(2) {
        return Err("opening_enter_not_offered");
    }
    Ok(())
}

impl OneResponse {
    pub(super) fn acknowledged(&self) -> bool {
        self.acknowledged
    }

    fn reserve(&mut self, dialogs: &State) -> std::result::Result<Identity, &'static str> {
        if let Some(identity) = &self.identity {
            return Err(if dialogs.modal.is_some_and(|id| id != identity.id) {
                "next_modal_requires_player_input"
            } else {
                "opening_response_already_submitted"
            });
        }
        let record = current_record(dialogs)?;
        if record.status != 1 {
            return Err("opening_modal_not_pending");
        }
        if !record.modal() {
            return Err("opening_record_not_modal");
        }
        if record.source != record.origin.source {
            return Err("opening_source_origin_mismatch");
        }
        let resource_name = record
            .source
            .rsplit(['/', '\\'])
            .next()
            .and_then(|component| component.rsplit("::").next());
        if resource_name != Some("Tutorial.iff") {
            return Err("opening_source_not_tutorial");
        }
        if record.origin.bhav != 4097 {
            return Err("opening_bhav_mismatch");
        }
        if record.origin.instruction != 2 {
            return Err("opening_instruction_mismatch");
        }
        if record.buttons.len() != 1 || record.buttons[0].result != 2 {
            return Err("opening_offered_result_mismatch");
        }
        validate_enter(record)?;
        let identity = Identity::observed(record);
        self.identity = Some(identity.clone());
        Ok(identity)
    }

    fn observe_acknowledgment(&mut self, dialogs: &State) -> std::result::Result<(), &'static str> {
        let identity = self
            .identity
            .as_ref()
            .ok_or("opening_response_not_submitted")?;
        if self.acknowledged {
            return Err("opening_response_already_acknowledged");
        }
        let mut matching = dialogs
            .records
            .iter()
            .filter(|record| record.id == identity.id);
        let record = matching.next().ok_or("opening_response_record_absent")?;
        if matching.next().is_some() {
            return Err("opening_response_record_ambiguous");
        }
        if Identity::observed(record) != *identity {
            return Err("opening_response_identity_changed");
        }
        if record.status == 1 {
            return Err("opening_response_not_acknowledged");
        }
        if record.status != 2 {
            return Err("opening_response_result_not_observed");
        }
        if dialogs.modal == Some(identity.id) {
            return Err("opening_response_modal_not_released");
        }
        self.acknowledged = true;
        Ok(())
    }
}

fn record_observation(record: &Record) -> Value {
    json!({
        "id": record.id, "caller": record.caller, "source": record.source,
        "origin": record.origin, "status": record.status, "modal": record.modal(),
        "operands": record.operands, "stack_object": record.stack_object,
        "offered_results": record.buttons.iter().map(|button| button.result).collect::<Vec<_>>(),
        "offered_keys": sims_vm::dialog::presentation::keys(record.operands[5]).into_iter()
            .filter_map(|(key, _)| record.key_result(key).map(|result| json!({"key": key, "result": result})))
            .collect::<Vec<_>>(),
        "enter_result": record.key_result(13)
    })
}

pub(super) fn capture_owners(viewer: &Viewer, state: &mut Value, failures: &mut Vec<Value>) {
    let dialogs = viewer.session.as_ref().map(|session| {
        let dialogs = &session.simulation.vm.world.dialogs;
        json!({
            "modal": dialogs.modal, "modal_busy": dialogs.modal_busy,
            "special_busy": dialogs.special_busy, "input_present": !dialogs.input.is_empty(),
            "protocol": dialogs.protocol,
            "records": dialogs.records.iter().map(record_observation).collect::<Vec<_>>()
        })
    });
    capture_viewer_owners_with_dialogs(viewer, state, failures, Ok(json!(dialogs)));
}

pub(super) fn boundary(
    initial_tick: u64,
    state: &Value,
    play_activated: bool,
    frames: u32,
    response: &OneResponse,
) -> Option<&'static str> {
    let boundary = first_tick_boundary(initial_tick, state, play_activated, frames);
    if matches!(
        boundary,
        Some(
            "viewer_error"
                | "viewer_menu_error"
                | "viewer_session_absent"
                | "vm_tick_regressed"
                | "tick_advanced_before_player_frame"
                | "tick_advanced_without_strict_native_lot"
        )
    ) {
        return boundary;
    }
    if response.acknowledged() && !state["flags"]["modal"].is_null() {
        return Some("next_modal_requires_player_input");
    }
    if boundary == Some("first_native_tick") && !response.acknowledged() {
        return Some("native_tick_before_opening_response");
    }
    boundary
}

pub(super) fn respond_once(
    viewer: &mut Viewer,
    epoch: Instant,
    observation: &mut Value,
    failures: &mut Vec<Value>,
    response: &mut OneResponse,
) -> Option<&'static str> {
    observation["active_boundary"] = json!("opening_response_validation");
    observation["before_response"] = viewer_observation(viewer, epoch);
    capture_owners(viewer, &mut observation["before_response"], failures);
    let Some(session) = viewer.session.as_ref() else {
        return Some("viewer_session_absent");
    };
    if let Err(blocker) = response.reserve(&session.simulation.vm.world.dialogs) {
        observation["response_validation_blocker"] = json!(blocker);
        return Some(blocker);
    }
    let record = match current_record(&session.simulation.vm.world.dialogs) {
        Ok(record) => record,
        Err(blocker) => return Some(blocker),
    };
    observation["response"] = json!({
        "record": record_observation(record), "current_modal": session.simulation.vm.world.dialogs.modal,
        "message": "Message::DialogKey(13)", "key": 13, "offered_result": 2,
        "frame": observation["frames"], "last_frame_input_ms": observation["last_frame_input_ms"],
        "identity_kind": "observed current modal and caller, never fixed runtime IDs"
    });
    observation["active_boundary"] = json!("viewer_dialog_key");
    observation["response_submitted"] = json!(true);
    let _ = viewer.update(Message::DialogKey(13));
    observation["after_response"] = viewer_observation(viewer, epoch);
    capture_owners(viewer, &mut observation["after_response"], failures);
    let acknowledgment = viewer
        .session
        .as_ref()
        .ok_or("viewer_session_absent")
        .and_then(|session| response.observe_acknowledgment(&session.simulation.vm.world.dialogs));
    observation["response_acknowledged"] = json!(acknowledgment.is_ok());
    observation["response_acknowledgment_blocker"] = json!(acknowledgment.err());
    if viewer.error.is_some() {
        return Some("viewer_error");
    }
    if viewer.menu_error.is_some() {
        return Some("viewer_menu_error");
    }
    acknowledgment.err()
}

pub(super) fn resume_play(
    viewer: &mut Viewer,
    epoch: Instant,
    observation: &mut Value,
    response: &mut OneResponse,
) -> Option<&'static str> {
    if response.resume_requested {
        return Some("playing_stopped_after_response_resume");
    }
    response.resume_requested = true;
    observation["active_boundary"] = json!("viewer_toggle_after_response");
    observation["before_resume"] = viewer_observation(viewer, epoch);
    observation["resume_play_requested"] = json!(true);
    let _ = viewer.update(Message::Toggle);
    observation["after_resume"] = viewer_observation(viewer, epoch);
    if viewer.error.is_some() {
        Some("viewer_error")
    } else if viewer.menu_error.is_some() {
        Some("viewer_menu_error")
    } else if !viewer.playing {
        Some("response_resume_play_not_activated")
    } else {
        None
    }
}

pub(super) fn tag_request(request: &mut Value) {
    request["frame_driver"]["play_input"] = json!(
        "ordinary Toggle after paused presentation, and at most one ordinary resume Toggle only after the same opening response is acknowledged and presentation gates return"
    );
    request["frame_driver"]["opening_response"] = json!({
        "max_responses": 1, "message": "Message::DialogKey(13)",
        "gate": "actual pending current modal, identical source/origin source, Tutorial.iff basename, BHAV 4097 instruction 2, exactly offered result 2 and Record::key_result(13) == Some(2)",
        "acknowledgment": "same observed identity retained with status 2 and modal released; absent/stale/ignored/unsupported result stops without retry",
        "next_modal": "stop without answering; no following prompts or tutorial completion"
    });
}

pub(super) fn tag_receipt(receipt: &mut Value) {
    receipt["schema"] = json!(SCHEMA);
    receipt["scope"] = json!(SCOPE);
    receipt["help"] = json!(HELP);
    for name in [
        "native_qualification",
        "startup_qualified",
        "performance_qualified",
        "alpha_qualified",
        "tutorial_qualified",
        "tutorial_complete",
        "three_days_qualified",
        "continuous_900s_qualified",
        "window_qualified",
        "restart_qualified",
        "package_qualified",
        "physical_input_observed",
        "gui_started",
        "presentation_observed",
        "settlement_observed",
    ] {
        receipt[name] = json!(false);
    }
}

pub(super) fn finish_probe(
    output: &Path,
    request: &Value,
    args: &Args,
    loaded: Result<Ready>,
    timing: Result<Value>,
) -> Result<()> {
    let ready_returned_ok = loaded.is_ok();
    let mut observation = json!({
        "initial_tick": null, "frames": 0, "last_frame_input_ms": null,
        "play_requested": false, "play_activated": false, "frame_observations": [],
        "active_boundary": "ready_load_inner", "response_submitted": false,
        "response_acknowledged": false, "resume_play_requested": false
    });
    let mut failures = Vec::new();
    let outcome = loaded
        .and_then(|ready| drive_tick_probe(args, ready, &mut observation, &mut failures, true));
    finish_tick_observation(
        output,
        request,
        ready_returned_ok,
        observation,
        outcome,
        failures,
        timing,
        true,
    )
}

#[test]
#[ignore = "Private parent-pinned readonly inputs required; fresh exclusive output; parent whole-process timeout below 300s and child cleanup; one offered opening response only, not tutorial/native qualification"]
fn ordinary_strict_newbie_tutorial_entry() -> Result<()> {
    ordinary_probe_with_response(true, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_vm::{Location, dialog::Button};

    fn authored_record() -> Record {
        Record {
            presentation: None,
            id: 73,
            caller: 7,
            origin: Location {
                source: "authored/Tutorial.iff".into(),
                bhav: 4097,
                instruction: 2,
            },
            source: "authored/Tutorial.iff".into(),
            operands: [0; 8],
            stack_object: 8,
            title: "authored title".into(),
            body: "authored body".into(),
            buttons: vec![Button {
                label: "authored primary".into(),
                result: 2,
            }],
            icon_object: None,
            job_icon: None,
            bitmap_icon: None,
            global_bitmap_icon: None,
            portrait: None,
            status: 1,
        }
    }

    fn authored_dialogs() -> State {
        let record = authored_record();
        State {
            modal: Some(record.id),
            records: vec![record],
            ..State::default()
        }
    }

    fn rejection(record: Record, expected: &str) {
        let dialogs = State {
            modal: Some(record.id),
            records: vec![record],
            ..State::default()
        };
        let mut response = OneResponse::default();
        assert_eq!(response.reserve(&dialogs).unwrap_err(), expected);
        assert!(response.identity.is_none());
        assert!(!response.acknowledged());
    }

    #[test]
    fn authored_identity_binds_actual_ids_not_runtime_constants() {
        for (id, caller, source) in [
            (73, 7, "authored/Tutorial.iff"),
            (900, 19, "different\\Tutorial.iff"),
            (301, 11, "authored/objects.far::Tutorial.iff"),
            (302, 12, "authored\\objects.far::Tutorial.iff"),
        ] {
            let mut record = authored_record();
            record.id = id;
            record.caller = caller;
            record.source = source.into();
            record.origin.source = source.into();
            let dialogs = State {
                modal: Some(id),
                records: vec![record],
                ..State::default()
            };
            let before = serde_json::to_value(&dialogs).unwrap();
            let mut response = OneResponse::default();
            let identity = response.reserve(&dialogs).unwrap();
            assert_eq!((identity.id, identity.caller), (id, caller));
            assert_eq!(identity.source, source);
            assert!(!response.acknowledged());
            assert_eq!(
                response.observe_acknowledgment(&dialogs),
                Err("opening_response_not_acknowledged")
            );
            assert_eq!(serde_json::to_value(&dialogs).unwrap(), before);
        }
    }

    #[test]
    fn authored_source_and_origin_must_match_exact_identity() {
        let mut record = authored_record();
        record.origin.source = "other/Tutorial.iff".into();
        rejection(record, "opening_source_origin_mismatch");
        for source in [
            "authored/NotTutorial.iff",
            "authored/tutorial.iff",
            "authored/Tutorial.iff/other",
            "authored/Other.iff",
            "authored/objects.far::Other.iff",
            "authored/objects.far::Tutorial.iff.extra",
        ] {
            let mut record = authored_record();
            record.source = source.into();
            record.origin.source = source.into();
            rejection(record, "opening_source_not_tutorial");
        }
    }

    #[test]
    fn authored_origin_status_and_modality_are_required() {
        let mut record = authored_record();
        record.origin.bhav = 4098;
        rejection(record, "opening_bhav_mismatch");
        let mut record = authored_record();
        record.origin.instruction = 3;
        rejection(record, "opening_instruction_mismatch");
        for status in [0, 2, 3, 4, 5] {
            let mut record = authored_record();
            record.status = status;
            rejection(record, "opening_modal_not_pending");
        }
        let mut record = authored_record();
        record.operands[7] = 128;
        rejection(record, "opening_record_not_modal");
    }

    #[test]
    fn authored_exactly_one_result_and_real_enter_mapping_are_required() {
        let record = authored_record();
        assert_eq!(record.key_result(13), Some(2));
        assert_eq!(validate_enter(&record), Ok(()));
        let mut missing = record.clone();
        missing.buttons.clear();
        assert_eq!(missing.key_result(13), None);
        assert_eq!(validate_enter(&missing), Err("opening_enter_not_offered"));
        rejection(missing, "opening_offered_result_mismatch");
        for results in [vec![3], vec![2, 2], vec![2, 3]] {
            let mut record = authored_record();
            record.buttons = results
                .into_iter()
                .map(|result| Button {
                    label: "authored choice".into(),
                    result,
                })
                .collect();
            if record.buttons[0].result != 2 {
                assert_eq!(validate_enter(&record), Err("opening_enter_not_offered"));
            }
            rejection(record, "opening_offered_result_mismatch");
        }
    }

    #[test]
    fn authored_stale_missing_and_duplicate_modal_records_are_rejected() {
        let mut dialogs = authored_dialogs();
        dialogs.modal = None;
        assert_eq!(
            OneResponse::default().reserve(&dialogs).unwrap_err(),
            "opening_modal_absent"
        );
        dialogs.modal = Some(91);
        assert_eq!(
            OneResponse::default().reserve(&dialogs).unwrap_err(),
            "opening_modal_record_absent"
        );
        dialogs.modal = Some(dialogs.records[0].id);
        dialogs.records.push(dialogs.records[0].clone());
        assert_eq!(
            OneResponse::default().reserve(&dialogs).unwrap_err(),
            "opening_modal_record_ambiguous"
        );
    }

    #[test]
    fn authored_consumed_response_cannot_answer_again_or_follow_a_second_modal() {
        let mut dialogs = authored_dialogs();
        let mut response = OneResponse::default();
        response.reserve(&dialogs).unwrap();
        assert_eq!(
            response.reserve(&dialogs).unwrap_err(),
            "opening_response_already_submitted"
        );
        dialogs.records[0].status = 2;
        assert_eq!(
            response.observe_acknowledgment(&dialogs),
            Err("opening_response_modal_not_released")
        );
        dialogs.modal = None;
        assert_eq!(response.observe_acknowledgment(&dialogs), Ok(()));
        assert!(response.acknowledged());
        assert_eq!(
            response.observe_acknowledgment(&dialogs),
            Err("opening_response_already_acknowledged")
        );
        assert_eq!(
            response.reserve(&dialogs).unwrap_err(),
            "opening_response_already_submitted"
        );
        let mut next = authored_record();
        next.id = 91;
        dialogs.modal = Some(next.id);
        dialogs.records.push(next);
        let before = serde_json::to_value(&dialogs).unwrap();
        assert_eq!(
            response.reserve(&dialogs).unwrap_err(),
            "next_modal_requires_player_input"
        );
        assert_eq!(serde_json::to_value(&dialogs).unwrap(), before);
    }

    #[test]
    fn authored_acknowledgment_requires_retained_same_record_and_result() {
        for mode in [
            "absent",
            "duplicate",
            "caller",
            "source",
            "origin",
            "operands",
            "stack",
            "offered",
            "pending",
            "wrong_result",
            "immediate_close",
        ] {
            let mut dialogs = authored_dialogs();
            let mut response = OneResponse::default();
            response.reserve(&dialogs).unwrap();
            dialogs.modal = None;
            dialogs.records[0].status = 2;
            let expected = match mode {
                "absent" => {
                    dialogs.records.clear();
                    "opening_response_record_absent"
                }
                "duplicate" => {
                    dialogs.records.push(dialogs.records[0].clone());
                    "opening_response_record_ambiguous"
                }
                "pending" => {
                    dialogs.records[0].status = 1;
                    "opening_response_not_acknowledged"
                }
                "wrong_result" => {
                    dialogs.records[0].status = 3;
                    "opening_response_result_not_observed"
                }
                "immediate_close" => {
                    dialogs.records[0].status = 5;
                    "opening_response_result_not_observed"
                }
                _ => {
                    let record = &mut dialogs.records[0];
                    match mode {
                        "caller" => record.caller += 1,
                        "source" => record.source.push_str(".changed"),
                        "origin" => record.origin.instruction += 1,
                        "operands" => record.operands[0] += 1,
                        "stack" => record.stack_object += 1,
                        "offered" => record.buttons[0].result = 3,
                        _ => unreachable!(),
                    }
                    "opening_response_identity_changed"
                }
            };
            let before = serde_json::to_value(&dialogs).unwrap();
            assert_eq!(response.observe_acknowledgment(&dialogs), Err(expected));
            assert!(!response.acknowledged());
            assert_eq!(serde_json::to_value(&dialogs).unwrap(), before);
        }
        assert_eq!(
            OneResponse::default().observe_acknowledgment(&authored_dialogs()),
            Err("opening_response_not_submitted")
        );
    }

    #[test]
    fn authored_dialog_observations_omit_all_prose() {
        let record = authored_record();
        let observed = record_observation(&record);
        let text = observed.to_string();
        for authored_prose in [&record.title, &record.body, &record.buttons[0].label] {
            assert!(!text.contains(authored_prose.as_str()));
        }
        assert!(observed.get("title").is_none());
        assert!(observed.get("body").is_none());
        assert_eq!(observed["id"], record.id);
        assert_eq!(observed["origin"]["bhav"], 4097);
        assert_eq!(observed["enter_result"], 2);
        assert!(
            observed["offered_keys"]
                .as_array()
                .unwrap()
                .contains(&json!({"key": 13, "result": 2}))
        );
    }

    #[test]
    fn authored_no_response_boundary_remains_unchanged_and_next_modal_stops() {
        let mut state = json!({
            "tick": 41, "error": null, "menu_error": null, "playing": false,
            "flags": {"modal": 73, "running_menu": false},
            "session": {"native_lot_present": true, "native_lot_routing": "Strict"}
        });
        let mut response = OneResponse::default();
        assert_eq!(
            first_tick_boundary(41, &state, true, 3),
            Some("modal_requires_player_input")
        );
        assert_eq!(
            boundary(41, &state, true, 3, &response),
            Some("modal_requires_player_input")
        );
        let mut dialogs = authored_dialogs();
        response.reserve(&dialogs).unwrap();
        dialogs.records[0].status = 2;
        dialogs.modal = None;
        response.observe_acknowledgment(&dialogs).unwrap();
        state["flags"]["modal"] = json!(91);
        state["tick"] = json!(42);
        assert_eq!(
            boundary(41, &state, true, 4, &response),
            Some("next_modal_requires_player_input")
        );
        state["flags"]["modal"] = Value::Null;
        assert_eq!(
            boundary(41, &state, true, 4, &response),
            Some("first_native_tick")
        );
        assert_eq!(
            boundary(41, &state, true, 4, &OneResponse::default()),
            Some("native_tick_before_opening_response")
        );
        state["error"] = json!("authored exact failure");
        assert_eq!(
            boundary(41, &state, true, 4, &response),
            Some("viewer_error")
        );
        assert_eq!(frame_input_millis(FIRST_TICK_MAX_FRAMES), None);
    }

    #[test]
    fn authored_dispatch_is_single_normal_key_without_direct_vm_mutation() {
        let source = include_str!("tutorial_entry.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert_eq!(
            source
                .matches("viewer.update(Message::DialogKey(13))")
                .count(),
            1
        );
        assert_eq!(source.matches("viewer.update(Message::Toggle)").count(), 1);
        for forbidden in [
            ".answer_dialog(",
            "Message::Dialog(",
            "session.as_mut()",
            "vm.world.dialogs =",
            "Message::Save",
            "Message::Load",
            "Message::Continue",
            "Message::Step",
        ] {
            assert!(
                !source.contains(forbidden),
                "forbidden response path: {forbidden}"
            );
        }
        let driver = include_str!("../strict_startup_probe.rs");
        assert!(
            driver.contains("drive_tick_probe(args, ready, observation, capture_failures, false)")
        );
        assert!(driver.contains("ordinary_probe_with_response(first_tick, false)"));
        assert!(
            driver.contains("first_tick_boundary(initial_tick, &state, play_activated, frames)")
        );
    }

    #[test]
    fn authored_receipts_separate_acknowledgment_progress_errors_and_qualification() -> Result<()> {
        for mode in [
            "first_native_tick",
            "next_modal_requires_player_input",
            "viewer_error",
            "ignored_key",
            "owner_failure",
            "timing_failure",
            "load_failure",
            "unacknowledged_tick",
        ] {
            let scratch = Scratch::new()?;
            let output = scratch.output()?;
            let mut observation = json!({
                "active_boundary": "viewer_dialog_key", "initial_tick": 41, "frames": 3,
                "last_frame_input_ms": 32, "response_submitted": mode != "load_failure",
                "response_acknowledged": !matches!(mode, "ignored_key" | "load_failure" | "unacknowledged_tick"),
                "final": {"tick": 42, "error": null, "menu_error": null}
            });
            let mut failures = Vec::new();
            retain_owner_capture(
                &mut observation["final"],
                &mut failures,
                "clock",
                Ok(json!({"now": 32, "authored_partial": true})),
            );
            let timing = if mode == "timing_failure" {
                Err(anyhow::anyhow!("authored timing detail").context("authored timing context"))
            } else {
                Ok(json!({"rows": []}))
            };
            if mode == "owner_failure" {
                retain_owner_capture(
                    &mut observation["final"],
                    &mut failures,
                    "scheduler",
                    Err(anyhow::anyhow!("authored owner detail").context("authored owner context")),
                );
            }
            let outcome = match mode {
                "viewer_error" => {
                    observation["final"]["error"] = json!("authored exact Viewer error");
                    Ok("viewer_error")
                }
                "ignored_key" => {
                    observation["final"]["tick"] = json!(41);
                    Ok("opening_response_not_acknowledged")
                }
                "load_failure" => {
                    observation["active_boundary"] = json!("ready_load_inner");
                    Err(anyhow::anyhow!("authored load detail").context("authored load context"))
                }
                "next_modal_requires_player_input" => Ok("next_modal_requires_player_input"),
                _ => Ok("first_native_tick"),
            };
            let result = finish_tick_observation(
                &output,
                &json!({}),
                mode != "load_failure",
                observation.clone(),
                outcome,
                failures,
                timing,
                true,
            );
            assert_eq!(result.is_ok(), mode == "first_native_tick");
            let receipt: Value = serde_json::from_slice(&fs::read(output.join(RECEIPT))?)?;
            assert_eq!(receipt["schema"], SCHEMA);
            assert_eq!(
                receipt["response_submitted"],
                observation["response_submitted"]
            );
            assert_eq!(
                receipt["response_acknowledged"],
                observation["response_acknowledged"]
            );
            assert_eq!(
                receipt["observation"]["final"]["owners"]["clock"]["authored_partial"],
                true
            );
            for (name, value) in receipt.as_object().unwrap() {
                if name.ends_with("_qualified")
                    || name.ends_with("_qualification")
                    || name == "tutorial_complete"
                    || name == "physical_input_observed"
                {
                    assert_eq!(value, &json!(false), "qualification flag {name}");
                }
            }
            if mode == "viewer_error" {
                assert_eq!(
                    receipt["first_failure"]["message"],
                    "authored exact Viewer error"
                );
            }
            if mode == "next_modal_requires_player_input" {
                assert_eq!(receipt["vm_tick_advanced"], true);
                assert_eq!(receipt["first_tick_success"], false);
                assert_eq!(receipt["success"], false);
            }
            if mode == "ignored_key" {
                assert_eq!(receipt["vm_tick_advanced"], false);
                assert_eq!(receipt["response_submitted"], true);
                assert_eq!(receipt["response_acknowledged"], false);
            }
            for (case, message) in [
                ("owner_failure", "authored owner detail"),
                ("timing_failure", "authored timing detail"),
                ("load_failure", "authored load detail"),
            ] {
                if mode == case {
                    assert!(
                        receipt["failures"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|failure| failure["chain"]
                                .as_array()
                                .is_some_and(|chain| chain.contains(&json!(message))))
                    );
                }
            }
        }
        Ok(())
    }
}
