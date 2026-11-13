use super::super::{
    Args, Message, Viewer,
    worker_host::{Event, Host},
};
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

fn recorder(capacity: usize) -> Recorder {
    Recorder::enabled(true, capacity).unwrap()
}

fn ready(generation: u64) -> Observation {
    Observation {
        generation,
        session_present: true,
        owner_drained: true,
        tick: Some(10),
        clock_minute: Some(600),
        ..Observation::default()
    }
}

fn start(recorder: &Recorder, needs_settled: bool) -> Option<Id> {
    recorder.start(
        None,
        Scope::UiDispatch,
        Operation::OtherUi,
        Role::PlayerPath,
        Some(ready(0)),
        needs_settled,
    )
}

#[test]
fn disabled_recorder_has_no_buffer_clock_or_artifact_path() {
    let disabled = Recorder::enabled(false, usize::MAX);
    assert!(disabled.is_none());
    let touched = Cell::new(false);
    let artifact = disabled.as_ref().map(|recorder| {
        touched.set(true);
        recorder.artifact(true)
    });
    assert!(!touched.get());
    assert!(artifact.is_none());
}

#[test]
fn disabled_loader_constructor_matches_existing_loading_path_without_polling() {
    let args = Args::parse_from(["sims-room", "--original", "UserData2/Houses/House09.iff"]);
    let (existing, task) = Viewer::new(args.clone());
    let (disabled, disabled_task) = Viewer::new_inner(args, false, None);
    assert_eq!(task.units(), disabled_task.units());
    drop((task, disabled_task));
    assert!(existing.session.is_none() && disabled.session.is_none());
    assert_eq!(existing.playing, disabled.playing);
    assert_eq!(existing.status, disabled.status);
    assert_eq!(existing.error, disabled.error);
    assert_eq!(existing.menu_error, disabled.menu_error);
}

struct Payload(Rc<Cell<usize>>);

impl Drop for Payload {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn loader_success_uses_one_load_shared_epoch_and_explicit_request_correlation() {
    let recorder = recorder(8);
    let parent = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    let probe = recorder.loader_probe(parent);
    assert_eq!(probe.recorder.epoch, recorder.epoch);
    let calls = Cell::new(0);
    let drops = Rc::new(Cell::new(0));
    let loaded = probe
        .measure(|| {
            calls.set(calls.get() + 1);
            Ok::<_, &str>(Payload(drops.clone()))
        })
        .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(drops.get(), 0);
    recorder.loaded_effect(0, 0);
    let artifact = recorder.artifact(false);
    let loader = &artifact["rows"][1];
    assert_eq!(loader["scope"], "loader_execution");
    assert_eq!(loader["outcome"], "completed");
    assert_eq!(loader["parent_id"], json!(parent));
    assert_eq!(loader["loader_request_id"], json!(parent));
    assert_eq!(loader["expected_generation"], 0);
    assert_eq!(loader["observed_generation"], 0);
    assert!(loader["end_ns"].as_u64().unwrap() >= loader["start_ns"].as_u64().unwrap());
    assert!(loader["shell_end_ns"].is_null());
    assert!(loader["drained_ns"].is_null());
    assert!(loader["settled_observed_ns"].is_null());
    drop(loaded);
    assert_eq!(drops.get(), 1);
}

#[test]
fn loader_error_is_recorded_once_before_error_conversion() {
    let recorder = recorder(8);
    let parent = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    let calls = Cell::new(0);
    let result = recorder.loader_probe(parent).measure(|| {
        calls.set(calls.get() + 1);
        Err::<(), _>("original loader error")
    });
    assert_eq!(result, Err("original loader error"));
    assert_eq!(calls.get(), 1);
    let artifact = recorder.artifact(false);
    assert_eq!(artifact["rows"][1]["outcome"], "error");
    assert!(artifact["rows"][1]["elapsed_ns"].is_u64());
    assert_eq!(artifact["complete"], false);
}

#[test]
fn stale_loader_generation_never_receives_completion_credit() {
    let recorder = recorder(8);
    let parent = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    recorder
        .loader_probe(parent)
        .measure(|| Ok::<_, ()>(()))
        .unwrap();
    recorder.loaded_effect(0, 1);
    recorder.drive_return(parent, true);
    recorder.observe_settled(ready(1));
    let artifact = recorder.artifact(true);
    assert_eq!(artifact["rows"][0]["outcome"], "superseded");
    assert_eq!(artifact["rows"][1]["outcome"], "superseded");
    assert!(artifact["rows"][0]["settled_observed_ns"].is_null());
    assert_eq!(artifact["complete"], false);
}

#[test]
fn construction_to_settled_overlaps_play_until_an_existing_ready_checkpoint() {
    let recorder = recorder(8);
    let fresh = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    recorder
        .loader_probe(fresh)
        .measure(|| Ok::<_, ()>(()))
        .unwrap();
    recorder.loaded_effect(0, 0);
    recorder.drive_return(fresh, true);
    recorder.observe_settled(Observation {
        pending: true,
        ..ready(0)
    });
    let play = recorder.start(
        None,
        Scope::Play,
        Operation::InitialPlay,
        Role::PlayerPath,
        Some(ready(0)),
        false,
    );
    let before_ready = recorder.artifact(false);
    assert!(before_ready["rows"][0]["end_ns"].is_null());
    assert!(before_ready["rows"][0]["settled_observed_ns"].is_null());
    recorder.finish(play, Outcome::Completed, Some(ready(0)), None);
    recorder.observe_settled(ready(0));
    let artifact = recorder.artifact(true);
    assert_eq!(
        artifact["rows"][0]["end_ns"],
        artifact["rows"][0]["settled_observed_ns"]
    );
    assert!(
        artifact["rows"][2]["start_ns"].as_u64().unwrap()
            <= artifact["rows"][0]["end_ns"].as_u64().unwrap()
    );
    assert_eq!(artifact["complete"], true);
}

#[test]
fn later_generations_do_not_relabel_an_already_correlated_fresh_loader() {
    let recorder = recorder(8);
    let fresh = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    recorder
        .loader_probe(fresh)
        .measure(|| Ok::<_, ()>(()))
        .unwrap();
    recorder.loaded_effect(0, 0);
    recorder.drive_return(fresh, true);
    recorder.observe_settled(ready(0));
    recorder.loaded_effect(1, 1);
    recorder.observe_settled(ready(1));
    let artifact = recorder.artifact(true);
    assert_eq!(artifact["rows"][0]["outcome"], "completed");
    assert_eq!(artifact["rows"][1]["observed_generation"], 0);
    assert_eq!(artifact["complete"], true);
}

#[test]
fn shell_return_precedes_gated_worker_completion_without_duration_threshold() -> anyhow::Result<()>
{
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| -> anyhow::Result<()> {
            let args =
                Args::parse_from(["sims-room", "--original", "UserData2/Houses/House09.iff"]);
            let (viewer, task) = Viewer::new(args);
            drop(task);
            let mut host = Host::from_viewer_for_test(viewer);
            let recorder = recorder(8);
            let id = start(&recorder, false);
            let (entered, resume) = host.gate_next_for_test();
            let generation = host.current_generation_for_test();
            let task = recorder.dispatch(id, || {
                host.update(Event::Widget {
                    generation,
                    message: Message::Shift(true),
                })
            });
            entered.recv_timeout(Duration::from_secs(5))?;
            let before_release = recorder.artifact(false);
            assert!(before_release["rows"][0]["shell_end_ns"].is_u64());
            assert!(before_release["rows"][0]["drained_ns"].is_null());
            resume.send(())?;
            drop(task);
            assert!(host.settle_for_test()?.is_empty());
            recorder.drive_return(id, true);
            recorder.finish(id, Outcome::Completed, None, None);
            let after_release = recorder.artifact(true);
            let row = &after_release["rows"][0];
            assert!(row["shell_end_ns"].as_u64().unwrap() <= row["drained_ns"].as_u64().unwrap());
            assert_eq!(row["outcome"], "completed");
            drop(host.recover_for_test());
            Ok(())
        })?
        .join()
        .unwrap()
}

#[test]
fn drained_pending_or_modal_publication_is_not_settled() {
    let recorder = recorder(8);
    let id = start(&recorder, true);
    recorder.drive_return(id, true);
    recorder.finish(id, Outcome::Completed, Some(ready(0)), None);
    let mut observation = ready(0);
    observation.pending = true;
    recorder.observe_settled(observation.clone());
    observation.pending = false;
    observation.modal = true;
    recorder.observe_settled(observation.clone());
    let artifact = recorder.artifact(true);
    assert!(artifact["rows"][0]["drained_ns"].is_u64());
    assert!(artifact["rows"][0]["settled_observed_ns"].is_null());
    assert_eq!(artifact["rows"][0]["outcome"], "censored");
    assert_eq!(artifact["complete"], false);
    observation.modal = false;
    recorder.observe_settled(observation);
    assert_eq!(recorder.artifact(true)["complete"], true);
}

#[test]
fn late_settlement_retains_its_actual_ready_observation_without_relabeling_drive_return() {
    let recorder = recorder(4);
    let id = start(&recorder, true);
    let mut drained = ready(0);
    drained.pending = true;
    recorder.drive_return(id, true);
    recorder.finish(id, Outcome::Completed, Some(drained), None);
    let before = recorder.artifact(true);
    let returned = before["rows"][0]["end_ns"].clone();
    assert_eq!(before["rows"][0]["after"]["pending"], true);
    assert!(before["rows"][0].get("settled_observation").is_none());
    let mut settled = ready(0);
    settled.commands = 7;
    recorder.observe_settled(settled);
    let after = recorder.artifact(true);
    assert_eq!(after["complete"], true);
    assert_eq!(after["rows"][0]["end_ns"], returned);
    assert_eq!(after["rows"][0]["after"]["pending"], true);
    assert_eq!(after["rows"][0]["settled_observation"]["pending"], false);
    assert_eq!(
        after["rows"][0]["settled_observation"]["owner_drained"],
        true
    );
    assert_eq!(after["rows"][0]["settled_observation"]["commands"], 7);
    assert!(
        after["rows"][0]["settled_observed_ns"].as_u64().unwrap() >= returned.as_u64().unwrap()
    );
}

#[test]
fn readiness_requires_expected_generation_session_error_free_owner_and_driver_drain() {
    for observation in [
        Observation {
            generation: 1,
            ..ready(0)
        },
        Observation {
            session_present: false,
            ..ready(0)
        },
        Observation {
            application_error: true,
            ..ready(0)
        },
        Observation {
            menu_error: true,
            ..ready(0)
        },
        Observation {
            owner_drained: false,
            ..ready(0)
        },
    ] {
        let recorder = recorder(4);
        let id = start(&recorder, true);
        recorder.drive_return(id, true);
        recorder.finish(id, Outcome::Completed, Some(ready(0)), None);
        recorder.observe_settled(observation);
        let artifact = recorder.artifact(true);
        assert!(artifact["rows"][0]["settled_observed_ns"].is_null());
        assert_eq!(artifact["complete"], false);
    }
}

#[test]
fn play_wall_span_includes_modal_response_children_and_reports_clock_tick_deltas() {
    let recorder = recorder(8);
    let mut before = ready(0);
    before.modal = true;
    let play = recorder.start(
        None,
        Scope::Play,
        Operation::InitialPlay,
        Role::PlayerPath,
        Some(before),
        false,
    );
    let response = recorder.start(
        play,
        Scope::UiDispatch,
        Operation::OtherUi,
        Role::PlayerPath,
        Some(ready(0)),
        true,
    );
    recorder.dispatch(response, || ());
    recorder.drive_return(response, true);
    recorder.finish(response, Outcome::Completed, Some(ready(0)), None);
    let after = Observation {
        tick: Some(42),
        clock_minute: Some(601),
        frames: 17,
        playing: true,
        ..ready(0)
    };
    recorder.finish(play, Outcome::Completed, Some(after.clone()), None);
    recorder.observe_settled(after);
    let artifact = recorder.artifact(true);
    assert_eq!(artifact["rows"][0]["before"]["modal"], true);
    assert_eq!(artifact["rows"][1]["parent_id"], json!(play));
    assert_eq!(artifact["rows"][0]["deltas"]["tick"], 32);
    assert_eq!(artifact["rows"][0]["deltas"]["clock_minute"], 1);
    assert_eq!(artifact["rows"][0]["deltas"]["frames"], 17);
    assert!(artifact.get("fps").is_none());
    assert_eq!(artifact["complete"], true);
}

#[test]
fn save_load_roles_keep_no_frame_order_and_checked_managed_generation_distinct_from_fresh_zero() {
    let recorder = recorder(8);
    for (operation, role, request, expected) in [
        (Operation::ProductionSave, Role::PrimarySave, 0, 0),
        (Operation::ProductionSave, Role::ControlSave, 0, 0),
        (Operation::ManagedLoad, Role::ManagedRestore, 0, 1),
        (Operation::ProductionSave, Role::ValidationResave, 1, 1),
    ] {
        let id = recorder.start(
            None,
            Scope::UiDispatch,
            operation,
            role,
            Some(ready(request)),
            true,
        );
        recorder.expect_generation(id, request, Some(expected));
        recorder.dispatch(id, || ());
        recorder.drive_return(id, true);
        recorder.finish(id, Outcome::Completed, Some(ready(expected)), None);
        recorder.observe_settled(ready(expected));
    }
    let artifact = recorder.artifact(true);
    let rows = artifact["rows"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["role"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("primary_save"),
            json!("control_save"),
            json!("managed_restore"),
            json!("validation_resave")
        ]
    );
    assert_eq!(rows[2]["request_generation"], 0);
    assert_eq!(rows[2]["expected_generation"], 1);
    assert!(rows.iter().all(|row| row["deltas"]["frames"] == 0));
    assert_eq!(artifact["complete"], true);
}

#[test]
fn native_proof_preserves_dispatch_drain_and_serial_absent_play_checkpoint_boundaries() {
    let recorder = recorder(8);
    let proof = recorder.start(
        None,
        Scope::NativeProof,
        Operation::Cancellation,
        Role::PlayerPath,
        Some(ready(0)),
        true,
    );
    let dispatch = recorder.start(
        proof,
        Scope::UiDispatch,
        Operation::Cancellation,
        Role::PlayerPath,
        Some(ready(0)),
        true,
    );
    recorder.dispatch(dispatch, || ());
    recorder.drive_return(dispatch, true);
    recorder.finish(dispatch, Outcome::Completed, Some(ready(0)), None);
    let play = recorder.start(
        proof,
        Scope::Play,
        Operation::CancelContinuation,
        Role::PlayerPath,
        Some(ready(0)),
        false,
    );
    let end = recorder.timestamp();
    recorder.finish_checkpoint(play, end, ready(0));
    recorder.observe_settled(ready(0));
    let artifact = recorder.artifact(true);
    let rows = artifact["rows"].as_array().unwrap();
    assert_eq!(rows[0]["start_ns"], rows[1]["start_ns"]);
    assert_eq!(rows[0]["shell_end_ns"], rows[1]["shell_end_ns"]);
    assert_eq!(rows[0]["drained_ns"], rows[1]["drained_ns"]);
    assert_eq!(rows[0]["end_ns"], rows[2]["end_ns"]);
    assert!(rows[0]["settled_observed_ns"].is_u64());
    assert_eq!(artifact["complete"], true);
}

#[test]
fn truncation_is_bounded_and_denies_complete_timing_qualification() {
    let recorder = recorder(2);
    for _ in 0..4 {
        let id = start(&recorder, false);
        recorder.finish(id, Outcome::Completed, None, None);
    }
    let artifact = recorder.artifact(true);
    assert_eq!(artifact["rows"].as_array().unwrap().len(), 2);
    assert_eq!(artifact["truncated"], true);
    assert_eq!(artifact["complete"], false);
}

#[test]
fn censored_and_controlled_error_serialization_preserves_null_endpoints_and_bounded_text() {
    let recorder = recorder(8);
    start(&recorder, true);
    let failed = start(&recorder, true);
    recorder.dispatch(failed, || ());
    recorder.drive_return(failed, false);
    recorder.finish(
        failed,
        Outcome::Error,
        Some(Observation {
            application_error: true,
            ..ready(0)
        }),
        Some(&"failure".repeat(200)),
    );
    let artifact = recorder.artifact(false);
    assert_eq!(artifact["schema"], "sims-host-journey-timing-v1");
    assert_eq!(artifact["clock"], "process-local-std-instant");
    let encoded = serde_json::to_string(&artifact).unwrap();
    let decoded: Value = serde_json::from_str(&encoded).unwrap();
    for field in [
        "end_ns",
        "elapsed_ns",
        "shell_end_ns",
        "drained_ns",
        "settled_observed_ns",
    ] {
        assert!(decoded["rows"][0].get(field).is_some());
        assert!(decoded["rows"][0][field].is_null());
    }
    assert_eq!(decoded["rows"][0]["outcome"], "censored");
    assert_eq!(decoded["rows"][1]["outcome"], "error");
    assert!(decoded["rows"][1]["drained_ns"].is_null());
    assert!(
        decoded["rows"][1]["error"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 512
    );
    assert!(decoded["rows"][1].get("engine_ns").is_none());
    assert_eq!(decoded["complete"], false);
}

#[test]
fn terminal_failure_censors_completion_without_extending_closed_measurements() {
    let recorder = recorder(4);
    let id = start(&recorder, false);
    recorder.finish(id, Outcome::Completed, Some(ready(0)), None);
    let successful = recorder.final_artifact(true, true);
    assert_eq!(successful["complete"], true);
    for (journey_completed, terminal_complete) in [(false, true), (true, false), (false, false)] {
        let failed = recorder.final_artifact(journey_completed, terminal_complete);
        assert_eq!(failed["complete"], false);
        assert_eq!(failed["rows"], successful["rows"]);
    }
}

#[test]
fn managed_operation_parent_expects_checked_successor_without_relabeling_fresh_zero() {
    let recorder = recorder(4);
    let managed = recorder.start(
        None,
        Scope::JourneyOperation,
        Operation::ManagedSaveLoad,
        Role::PlayerPath,
        Some(ready(7)),
        false,
    );
    recorder.finish(managed, Outcome::Completed, Some(ready(8)), None);
    let fresh = recorder.start(
        None,
        Scope::ConstructionToSettled,
        Operation::FreshLoad,
        Role::PlayerPath,
        None,
        true,
    );
    let _probe = recorder.loader_probe(fresh);
    let overflow = recorder.start(
        None,
        Scope::JourneyOperation,
        Operation::ManagedSaveLoad,
        Role::PlayerPath,
        Some(ready(u64::MAX)),
        false,
    );
    let artifact = recorder.artifact(false);
    assert_eq!(
        artifact["rows"][managed.unwrap() as usize]["request_generation"],
        7
    );
    assert_eq!(
        artifact["rows"][managed.unwrap() as usize]["expected_generation"],
        8
    );
    assert_eq!(
        artifact["rows"][managed.unwrap() as usize]["observed_generation"],
        8
    );
    assert_eq!(
        artifact["rows"][fresh.unwrap() as usize]["expected_generation"],
        0
    );
    assert!(
        artifact["rows"][overflow.unwrap() as usize]
            .get("expected_generation")
            .is_none()
    );
}


fn stage_fixture(capacity: usize, ticks: &[u64]) -> (Recorder, Option<Id>, Arc<std::sync::atomic::AtomicUsize>) {
    let mut recorder = recorder(capacity);
    let parent = recorder.start(None, Scope::ConstructionToSettled, Operation::FreshLoad, Role::PlayerPath, None, true);
    if let Some(row) = recorder.buffer.lock().unwrap().rows.first_mut() {
        row.start_ns = 0;
    }
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let ticks = Mutex::new(std::collections::VecDeque::from(ticks.to_vec()));
    recorder.loader_clock = Some(Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ticks.lock().map_err(|_| "fake clock poisoned")?.pop_front().ok_or("fake clock exhausted")
    }));
    (recorder, parent, calls)
}

fn enter_fresh(stages: &mut LoaderStages) {
    for phase in LoaderPhase::ALL {
        LoaderStages::enter_optional(&mut Some(&mut *stages), phase);
    }
}

fn fresh_stage_artifact(recorder: &Recorder) -> Value {
    let buffer = recorder.buffer.lock().unwrap();
    let parent = buffer.rows.first().unwrap().id;
    drop(buffer);
    recorder.loaded_effect(0, 0);
    let mut buffer = recorder.buffer.lock().unwrap();
    let request = buffer.rows.iter_mut().find(|row| row.id == parent).unwrap();
    request.drained_ns = Some(300);
    request.settled_observed_ns = Some(300);
    request.end_ns = Some(300);
    request.elapsed_ns = Some(300);
    request.after = Some(ready(0));
    request.settled_observation = Some(ready(0));
    request.outcome = Outcome::Completed;
    drop(buffer);
    recorder.try_final_artifact(true, true).unwrap()
}

#[test]
fn loader_stages_exact_integer_boundaries_containment_and_separate_residuals() {
    let (recorder, parent, calls) = stage_fixture(8, &[90, 100, 120, 140, 160, 180, 200, 220, 260, 280]);
    recorder.loader_probe(parent).measure_staged(|stages| {
        enter_fresh(stages);
        stages.finish(true);
        Ok::<_, ()>(())
    }).unwrap();
    let artifact = fresh_stage_artifact(&recorder);
    let aggregate = &artifact["rows"][1];
    let loader = &aggregate["loader"];
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 10);
    assert_eq!(aggregate["start_ns"], 90);
    assert_eq!(aggregate["end_ns"], 280);
    assert_eq!(aggregate["elapsed_ns"], 190);
    assert_eq!(loader["schema"], "sims-ready-loader-phases-v1");
    assert_eq!(loader["aggregate_parent_id"], aggregate["id"]);
    assert_eq!(loader["stage_total_ns"], 160);
    assert_eq!(loader["entry_residual_ns"], 10);
    assert_eq!(loader["exit_residual_ns"], 20);
    let phases = loader["phases"].as_array().unwrap();
    assert_eq!(phases.len(), 7);
    for (position, phase) in phases.iter().enumerate() {
        assert_eq!(phase["phase_id"], serde_json::to_value(LoaderPhase::ALL[position]).unwrap());
        assert_eq!(phase["start_ns"], 100 + position as u64 * 20);
        assert_eq!(phase["end_ns"], if position == 6 { 260 } else { 120 + position as u64 * 20 });
        assert_eq!(phase["elapsed_ns"], if position == 6 { 40 } else { 20 });
        assert_eq!(phase["outcome"], "completed");
        assert!(phase.get("drained_ns").is_none());
        assert!(phase.get("settled_observed_ns").is_none());
    }
    assert_eq!(artifact["complete"], true);
}

#[test]
fn loader_stages_cold_restore_has_only_scan_and_wrapping() {
    let (recorder, parent, _) = stage_fixture(8, &[10, 12, 20, 30, 35]);
    recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        stages.finish(true);
        Ok::<_, ()>(())
    }).unwrap();
    let artifact = fresh_stage_artifact(&recorder);
    let phases = artifact["rows"][1]["loader"]["phases"].as_array().unwrap();
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0]["phase_id"], "index_scan_route");
    assert_eq!(phases[1]["phase_id"], "icons_ready_wrap_cleanup");
    assert_eq!(artifact["rows"][1]["loader"]["stage_total_ns"], 18);
}

#[test]
fn loader_stages_each_phase_error_keeps_original_error_and_only_entered_prefix() {
    for failed in 0..LOADER_STAGE_CAPACITY {
        let ticks: Vec<_> = (0..failed as u64 + 4).map(|tick| tick * 10).collect();
        let (recorder, parent, _) = stage_fixture(8, &ticks);
        let original = anyhow::anyhow!("phase-{failed}").context("original game context");
        let identity = original.root_cause() as *const dyn std::error::Error;
        let result = recorder.loader_probe(parent).measure_staged(|stages| {
            for phase in &LoaderPhase::ALL[..=failed] { stages.enter(*phase); }
            stages.finish(false);
            Err::<(), _>(original)
        }).unwrap_err();
        assert!(std::ptr::eq(result.root_cause() as *const dyn std::error::Error, identity));
        assert_eq!(format!("{result:#}"), format!("original game context: phase-{failed}"));
        let artifact = recorder.try_final_artifact(false, false).unwrap();
        let phases = artifact["rows"][1]["loader"]["phases"].as_array().unwrap();
        assert_eq!(phases.len(), failed + 1);
        assert_eq!(phases[failed]["outcome"], "error");
        assert!(phases[..failed].iter().all(|phase| phase["outcome"] == "completed"));
        assert_eq!(artifact["complete"], false);
    }
}

#[test]
fn loader_stages_one_load_retains_payload_until_caller_drop() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30, 40]);
    let calls = Cell::new(0);
    let drops = Rc::new(Cell::new(0));
    let payload = recorder.loader_probe(parent).measure_staged(|stages| {
        calls.set(calls.get() + 1);
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        stages.finish(true);
        Ok::<_, ()>(Payload(drops.clone()))
    }).unwrap();
    assert_eq!((calls.get(), drops.get()), (1, 0));
    assert_eq!(recorder.try_final_artifact(false, false).unwrap()["rows"][1]["end_ns"], 40);
    drop(payload);
    assert_eq!(drops.get(), 1);
}

#[test]
fn loader_stages_disabled_optional_collector_never_touches_fake_clock_or_recorder() {
    let (recorder, _, calls) = stage_fixture(0, &[]);
    let before = recorder.buffer.lock().unwrap().rows.len();
    let mut absent = None;
    for phase in LoaderPhase::ALL { LoaderStages::enter_optional(&mut absent, phase); }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let buffer = recorder.buffer.lock().unwrap();
    assert_eq!(buffer.rows.len(), before);
    assert!(buffer.loader_failure.is_none());
}

#[test]
fn loader_stages_unclosed_tail_is_censored_without_drained_or_settled_credit() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30]);
    recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::SessionLoad);
        Ok::<_, ()>(())
    }).unwrap();
    let artifact = recorder.try_final_artifact(true, true).unwrap();
    let aggregate = &artifact["rows"][1];
    let loader = &aggregate["loader"];
    assert_eq!(loader["phases"][0]["outcome"], "completed");
    assert_eq!(loader["phases"][1]["outcome"], "censored");
    assert!(loader["phases"][1]["end_ns"].is_null());
    assert!(loader["phases"][1]["elapsed_ns"].is_null());
    assert!(loader["exit_residual_ns"].is_null());
    assert_eq!(loader["outcome"], "censored");
    assert!(aggregate["drained_ns"].is_null());
    assert!(aggregate["settled_observed_ns"].is_null());
    assert_eq!(artifact["complete"], false);
}

#[test]
fn loader_stages_zero_and_exhausted_recorder_capacity_deny_export_without_retry() {
    for capacity in [0, 1] {
        let (recorder, parent, _) = stage_fixture(capacity, &[0, 20]);
        let calls = Cell::new(0);
        let original = recorder.loader_probe(parent).measure_staged(|stages| {
            calls.set(calls.get() + 1);
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.finish(false);
            Err::<(), _>("original game error")
        });
        assert_eq!(original, Err("original game error"));
        assert_eq!(calls.get(), 1);
        assert!(recorder.try_final_artifact(true, true).is_err());
    }
}

#[test]
fn loader_stages_fixed_local_capacity_overflow_denies_qualification() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 20]);
    let result = recorder.loader_probe(parent).measure_staged(|stages| {
        stages.len = LOADER_STAGE_CAPACITY;
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        Ok::<_, ()>(())
    });
    assert_eq!(result, Ok(()));
    assert!(recorder.try_final_artifact(true, true).is_err());
}

#[test]
fn loader_stages_missing_aggregate_or_request_parent_never_manufactures_complete_evidence() {
    for remove_parent in [false, true] {
        let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30, 40]);
        recorder.loader_probe(parent).measure_staged(|stages| {
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.enter(LoaderPhase::IconsReadyWrapCleanup);
            stages.finish(true);
            let mut buffer = stages.recorder.buffer.lock().unwrap();
            buffer.rows.remove(if remove_parent { 0 } else { 1 });
            Ok::<_, ()>(())
        }).unwrap();
        assert!(recorder.try_final_artifact(true, true).is_err());
    }
}

#[test]
fn loader_stages_poisoned_parent_preserves_game_error_without_new_recorder_panic() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10]);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = recorder.buffer.lock().unwrap();
        panic!("deterministic recorder poison");
    }));
    let result = recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        Err::<(), _>("original game error")
    });
    assert_eq!(result, Err("original game error"));
    assert!(recorder.try_final_artifact(true, true).is_err());
}

#[test]
fn loader_stages_invalid_order_and_reversed_clock_are_fail_closed() {
    for ticks in [&[0, 10, 20, 30][..], &[0, 20, 10, 30][..]] {
        let (recorder, parent, _) = stage_fixture(8, ticks);
        let result = recorder.loader_probe(parent).measure_staged(|stages| {
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.enter(if ticks[1] == 10 { LoaderPhase::NeighborhoodBinding } else { LoaderPhase::SessionLoad });
            Err::<(), _>("unchanged game result")
        });
        assert_eq!(result, Err("unchanged game result"));
        assert!(recorder.try_final_artifact(true, true).is_err());
    }
}

#[test]
fn loader_stages_checked_totals_reject_overflow_and_accept_exact_u64_limit() {
    let spans = [LoaderSpan { elapsed_ns: Some(u64::MAX), ..LoaderSpan::EMPTY }, LoaderSpan { elapsed_ns: Some(1), ..LoaderSpan::EMPTY }];
    assert_eq!(checked_stage_total(&spans[..1]), Ok(u64::MAX));
    assert_eq!(checked_stage_total(&spans), Err("loader phase total overflow"));
}

#[test]
fn loader_stages_export_rejects_containment_endpoint_and_arithmetic_corruption() {
    for corruption in 0..4 {
        let (recorder, parent, _) = stage_fixture(8, &[90, 100, 120, 140, 160]);
        recorder.loader_probe(parent).measure_staged(|stages| {
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.enter(LoaderPhase::IconsReadyWrapCleanup);
            stages.finish(true);
            Ok::<_, ()>(())
        }).unwrap();
        let mut buffer = recorder.buffer.lock().unwrap();
        let row = &mut buffer.rows[1];
        let payload = row.loader.as_mut().unwrap();
        match corruption {
            0 => payload.spans[0].start_ns = 89,
            1 => payload.spans[1].end_ns = Some(161),
            2 => payload.spans[0].elapsed_ns = Some(u64::MAX),
            _ => row.end_ns = Some(89),
        }
        drop(buffer);
        assert!(recorder.try_final_artifact(true, true).is_err());
    }
}

#[test]
fn loader_stages_request_identity_and_generations_are_inherited_not_reset() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30, 40]);
    recorder.expect_generation(parent, 7, Some(8));
    recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        stages.finish(true);
        Ok::<_, ()>(())
    }).unwrap();
    recorder.loaded_effect(8, 8);
    let artifact = recorder.try_final_artifact(false, false).unwrap();
    let loader = &artifact["rows"][1]["loader"];
    assert_eq!(loader["request_id"], json!(parent));
    assert_eq!(loader["request_generation"], 7);
    assert_eq!(loader["expected_generation"], 8);
    assert_eq!(loader["observed_generation"], 8);
}

#[test]
fn loader_stages_superseded_request_stays_stale_and_never_relabels_later() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30, 40]);
    recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        stages.finish(true);
        Ok::<_, ()>(())
    }).unwrap();
    recorder.loaded_effect(0, 1);
    let before = recorder.try_final_artifact(true, true).unwrap();
    recorder.loaded_effect(1, 1);
    let after = recorder.try_final_artifact(true, true).unwrap();
    assert_eq!(before["rows"], after["rows"]);
    assert_eq!(after["rows"][1]["loader"]["outcome"], "superseded");
    assert_eq!(after["rows"][1]["loader"]["complete"], false);
    assert!(after["rows"][0]["settled_observed_ns"].is_null());
    assert_eq!(after["complete"], false);
}

#[test]
fn loader_stages_clock_recording_failure_returns_the_same_game_error() {
    let (recorder, parent, calls) = stage_fixture(8, &[0, 10]);
    let result = recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::SessionLoad);
        stages.finish(false);
        Err::<(), _>("original game failure")
    });
    assert_eq!(result, Err("original game failure"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(recorder.try_final_artifact(false, false).is_err());
}

#[test]
fn loader_stages_phase_publication_occurs_only_after_aggregate_end_sample() {
    let (recorder, parent, _) = stage_fixture(8, &[0, 10, 20, 30, 40]);
    recorder.loader_probe(parent).measure_staged(|stages| {
        stages.enter(LoaderPhase::IndexScanRoute);
        stages.enter(LoaderPhase::IconsReadyWrapCleanup);
        stages.finish(true);
        let buffer = stages.recorder.buffer.lock().unwrap();
        assert!(buffer.rows[1].end_ns.is_none());
        assert!(buffer.rows[1].loader.is_none());
        Ok::<_, ()>(())
    }).unwrap();
    let buffer = recorder.buffer.lock().unwrap();
    assert_eq!(buffer.rows[1].end_ns, Some(40));
    assert!(buffer.rows[1].loader.is_some());
}

fn assert_loader_stage_source_wiring(source: &str) {
    let body = source.split("    fn load_inner(").nth(1).unwrap().split("\n#[derive(Debug, Clone)]\nenum Message").next().unwrap();
    let phases = ["IndexScanRoute", "IconsReadyWrapCleanup", "SessionLoad", "CameraOpenCompletion",
                  "NeighborhoodBinding", "DialogsResidentOpening", "ClockTransportBuyPreview", "IconsReadyWrapCleanup"];
    let expected = phases.map(|phase| format!("player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::{phase});"));
    let actual = body.lines().map(str::trim).filter(|line| line.starts_with("player_timing::LoaderStages::enter_optional(")).collect::<Vec<_>>();
    assert_eq!(actual, expected.iter().map(String::as_str).collect::<Vec<_>>());
    for (phase, following) in [
        ("IndexScanRoute", "let index = Index::scan(&args.root)?;"),
        ("SessionLoad", "let mut session = Session::load("),
        ("CameraOpenCompletion", "let mut device = if args.load.is_none() {"),
        ("NeighborhoodBinding", "neighborhood_usability::bind_policy(&mut session, args, true)?;"),
        ("DialogsResidentOpening", "let mut client = if args.load.is_none() {"),
        ("ClockTransportBuyPreview", "if args.load.is_none() {"),
        ("IconsReadyWrapCleanup", "Ok(Self {"),
    ] {
        assert!(body.contains(&format!("player_timing::LoaderPhase::{phase});\n            {following}")), "checkpoint placement changed: {phase}");
    }
    let cold = body.split("            if args.load.is_some() {").nth(1).unwrap()
        .split("            #[cfg(test)]\n            player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::SessionLoad);").next().unwrap();
    assert_eq!(cold.matches("enter_optional").count(), 1);
    assert!(cold.contains("player_timing::LoaderPhase::IconsReadyWrapCleanup);"));
    assert!(cold.contains("return Ok(Self {\n                    icons: Arc::new(dialog_view::Icons::load(&index)?),\n                    restore_index: Some(index),\n                    session: Mutex::new(None),\n                    client: Mutex::new(None),\n                    device: Mutex::new(None),\n                });\n            }\n"));
    assert_eq!(body.matches("Index::scan(&args.root)?").count(), 1);
    assert_eq!(body.matches("let mut session = Session::load(").count(), 1);
    assert!(body.contains("restore_index: None,\n                client: Mutex::new(client),\n                session: Mutex::new(Some(session)),\n                device: Mutex::new(device),\n                icons: Arc::new(dialog_view::Icons::load(&index)?),"));
    assert_eq!(body.matches("stages.finish(").count(), 1);
    assert!(body.contains("\n        })();\n        #[cfg(test)]\n        if let Some(stages) = stages {\n            stages.finish(result.is_ok());\n        }\n        result\n    }\n}\n"));
    assert!(source.contains(".measure_staged(|stages| Ready::load_inner(&work, Some(stages)))"));
    let restart = source.split("Reloading original household…").nth(1).unwrap().split("let frame = matches!").next().unwrap();
    assert!(restart.contains("Ready::load(&work)"));
    assert!(!restart.contains("probe") && !restart.contains("load_inner"));
}

#[test]
fn loader_stages_shared_body_keeps_icons_field_order_and_restart_uninstrumented() {
    let source = include_str!("../../original_viewer.rs");
    assert_loader_stage_source_wiring(source);
    let marker = "player_timing::LoaderStages::enter_optional(&mut stages, player_timing::LoaderPhase::";
    for (position, _) in source.match_indices(marker) {
        let length = source[position..].find(';').unwrap() + 1;
        let mut changed = source.to_owned();
        changed.replace_range(position..position + length, "");
        assert!(std::panic::catch_unwind(|| assert_loader_stage_source_wiring(&changed)).is_err(), "removed checkpoint escaped guard");
    }
    let changed = source.replacen("            stages.finish(result.is_ok());", "", 1);
    assert!(std::panic::catch_unwind(|| assert_loader_stage_source_wiring(&changed)).is_err(), "removed finalization escaped guard");
}


#[test]
fn loader_stages_unwind_preserves_completed_prefix_and_null_interrupted_endpoints() {
    let (recorder, parent, calls) = stage_fixture(8, &[0, 10, 20, 30]);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recorder.loader_probe(parent).measure_staged::<(), ()>(|stages| {
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.enter(LoaderPhase::SessionLoad);
            std::panic::resume_unwind(Box::new("original interruption"));
        })
    })).unwrap_err();
    assert_eq!(interrupted.downcast_ref::<&str>(), Some(&"original interruption"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    let artifact = recorder.try_final_artifact(true, true).unwrap();
    let aggregate = &artifact["rows"][1];
    let loader = &aggregate["loader"];
    assert!(aggregate["end_ns"].is_null());
    assert!(aggregate["elapsed_ns"].is_null());
    assert_eq!(aggregate["outcome"], "censored");
    assert_eq!(loader["phases"][0]["end_ns"], 20);
    assert_eq!(loader["phases"][0]["outcome"], "completed");
    assert!(loader["phases"][1]["end_ns"].is_null());
    assert!(loader["phases"][1]["elapsed_ns"].is_null());
    assert_eq!(loader["phases"][1]["outcome"], "censored");
    assert!(loader["exit_residual_ns"].is_null());
    assert_eq!(loader["complete"], false);
    assert_eq!(artifact["complete"], false);
}

#[test]
fn loader_stages_body_cleanup_precedes_final_phase_close_and_payload_drop() {
    struct OrderedDrop(&'static str, Rc<std::cell::RefCell<Vec<&'static str>>>);
    impl Drop for OrderedDrop {
        fn drop(&mut self) { self.1.borrow_mut().push(self.0); }
    }
    for succeeds in [true, false] {
        let (recorder, parent, calls) = stage_fixture(8, &[0, 10, 20, 30, 40]);
        let order = Rc::new(std::cell::RefCell::new(Vec::new()));
        let invocations = Cell::new(0);
        let result = recorder.loader_probe(parent).measure_staged(|stages| {
            invocations.set(invocations.get() + 1);
            let result = (|| {
                stages.enter(LoaderPhase::IndexScanRoute);
                let _index = OrderedDrop("index", order.clone());
                stages.enter(LoaderPhase::IconsReadyWrapCleanup);
                let _session = OrderedDrop("session", order.clone());
                let _device = OrderedDrop("device", order.clone());
                if succeeds { Ok(OrderedDrop("ready", order.clone())) } else { Err("original game error") }
            })();
            assert_eq!(&*order.borrow(), &["device", "session", "index"]);
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
            stages.finish(result.is_ok());
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
            result
        });
        assert_eq!(invocations.get(), 1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 5);
        assert_eq!(&*order.borrow(), &["device", "session", "index"]);
        if !succeeds { assert_eq!(result.as_ref().err(), Some(&"original game error")); }
        drop(result);
        assert_eq!(&*order.borrow(), if succeeds { &["device", "session", "index", "ready"][..] } else { &["device", "session", "index"][..] });
    }
}

fn panicking_stage_fixture(fail_at: usize) -> (Recorder, Option<Id>, Arc<std::sync::atomic::AtomicUsize>) {
    let (mut recorder, parent, calls) = stage_fixture(8, &[]);
    let observed = calls.clone();
    recorder.loader_clock = Some(Arc::new(move || {
        let sample = observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if sample == fail_at {
            std::panic::resume_unwind(Box::new("injected diagnostic clock panic"));
        }
        Ok(sample as u64 * 10)
    }));
    (recorder, parent, calls)
}

#[test]
fn loader_stages_clock_panics_preserve_returned_game_error_and_stop_sampling() {
    for fail_at in 0..4 {
        let (recorder, parent, calls) = panicking_stage_fixture(fail_at);
        let original = Box::new(173);
        let invocations = Cell::new(0);
        let result = recorder.loader_probe(parent).measure_staged(|stages| {
            invocations.set(invocations.get() + 1);
            stages.enter(LoaderPhase::IndexScanRoute);
            stages.finish(false);
            Err::<(), _>(&*original)
        });
        assert!(std::ptr::eq(result.unwrap_err(), &*original));
        assert_eq!(invocations.get(), 1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), fail_at + 1);
        assert!(recorder.try_final_artifact(false, false).is_err());
        assert_eq!(recorder.try_loader_now(), Err("loader diagnostic clock panicked"));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), fail_at + 1);
    }
}

#[test]
fn loader_stages_drop_clock_panic_keeps_original_game_unwind_payload() {
    let (recorder, parent, calls) = panicking_stage_fixture(2);
    let original = Arc::new(173);
    let observed = original.clone();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recorder.loader_probe(parent).measure_staged::<(), ()>(|stages| {
            stages.enter(LoaderPhase::IndexScanRoute);
            std::panic::resume_unwind(Box::new(observed));
        })
    })).unwrap_err();
    assert!(Arc::ptr_eq(interrupted.downcast_ref::<Arc<i32>>().unwrap(), &original));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert!(recorder.try_final_artifact(false, false).is_err());
    let buffer = recorder.buffer.lock().unwrap();
    assert!(buffer.rows[1].end_ns.is_none());
    assert!(buffer.rows[1].elapsed_ns.is_none());
    assert!(buffer.rows[1].loader.is_none());
}

#[test]
fn loader_stages_known_order_failure_skips_drop_clock_during_game_unwind() {
    let (recorder, parent, calls) = stage_fixture(8, &[0]);
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recorder.loader_probe(parent).measure_staged::<(), ()>(|stages| {
            stages.enter(LoaderPhase::SessionLoad);
            std::panic::resume_unwind(Box::new("original game interruption"));
        })
    })).unwrap_err();
    assert_eq!(interrupted.downcast_ref::<&str>(), Some(&"original game interruption"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(recorder.try_final_artifact(false, false).is_err());
    assert_eq!(recorder.try_loader_now(), Err("loader first phase is not scan"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
