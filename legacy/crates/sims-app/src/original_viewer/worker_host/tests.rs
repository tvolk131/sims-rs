use super::*;
use clap::Parser;

fn loading() -> Viewer {
    let args = Args::parse_from(["sims-room", "--original", "UserData2/Houses/House09.iff"]);
    let (viewer, task) = Viewer::new(args);
    drop(task);
    viewer
}
fn input(host: &mut Host, message: Message) {
    let generation = host.generation();
    drop(host.update(Event::Widget {
        generation,
        message,
    }));
}
fn run(f: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)?
        .join()
        .unwrap()
}

#[test]
fn shell_controls_do_not_wait_for_or_borrow_the_owner() -> Result<()> {
    run(shell_controls)
}
fn shell_controls() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    let before = host.frontend.data.clone();
    input(&mut host, Message::Options);
    assert!(host.frontend.options_open);
    assert!(Arc::ptr_eq(&before, &host.frontend.data));
    assert!(!host.worker.as_ref().unwrap().busy());
    input(&mut host, Message::Diagnostics);
    assert!(host.frontend.diagnostics && !host.frontend.options_open);
    input(&mut host, Message::FocusLost);
    host.settle_for_test()?;
    assert_eq!(host.frontend.widget_epoch, 1);
    assert!(host.frontend.diagnostics);
    let owner = host.recover_for_test();
    assert!(owner.diagnostics && !owner.options_open);
    assert_eq!(owner.widget_epoch, 1);
    assert!(owner.session.is_none());
    Ok(())
}

#[test]
fn stale_widget_and_task_effect_generations_do_not_reach_replaced_owner() -> Result<()> {
    run(stale_generations)
}
fn stale_generations() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    input(&mut host, Message::Restart);
    host.settle_for_test()?;
    assert_eq!(host.generation(), 1);
    assert_eq!(host.frontend.widget_epoch, 1);
    drop(host.update(Event::Widget {
        generation: 0,
        message: Message::Shift(true),
    }));
    drop(host.update(Event::Effect {
        generation: 0,
        message: Message::Shift(true),
    }));
    let owner = host.recover_for_test();
    assert!(!owner.shift && owner.session.is_none());
    Ok(())
}

#[test]
fn consecutive_motion_coalesces_without_crossing_transition_or_shell_boundaries() -> Result<()> {
    run(motion)
}
fn motion() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    let motion = |n| Message::ViewportMotion {
        pixel: Some([n, 2.]),
        pan: [0, 0],
    };
    host.enqueue(0, motion(1.));
    host.enqueue(0, motion(2.));
    assert_eq!(host.queued.len(), 1);
    host.enqueue(0, Message::ViewportLeave);
    host.enqueue(0, motion(3.));
    assert_eq!(host.queued.len(), 3);
    host.frontend.options_open = true;
    host.enqueue(0, motion(4.));
    assert_eq!(host.queued.len(), 4);
    // Returning to the same flags still crossed two real shell boundaries.
    input(&mut host, Message::Options);
    input(&mut host, Message::Options);
    host.enqueue(0, motion(5.));
    assert_eq!(host.queued.len(), 5);
    let _ = host.recover_for_test();
    Ok(())
}

#[test]
fn input_retention_is_bounded_and_overflow_is_explicit() -> Result<()> {
    run(bounded)
}
fn bounded() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    for _ in 0..256 {
        host.enqueue(0, Message::Shift(false));
    }
    assert_eq!(host.queued.len(), 256);
    assert!(host.error.is_none());
    host.enqueue(0, Message::Shift(true));
    assert_eq!(host.queued.len(), 256);
    assert!(host.error.as_deref().unwrap().contains("queue is full"));
    let owner = host.recover_for_test();
    assert!(!owner.shift);
    Ok(())
}

#[test]
fn real_host_paints_and_retains_ordered_input_while_its_owner_is_blocked() -> Result<()> {
    run(blocked_owner)
}
fn blocked_owner() -> Result<()> {
    use std::sync::mpsc;
    use std::time::Duration;
    let mut host = Host::from_viewer_for_test(loading());
    let original = host.frontend.data.clone();
    let (started, entered) = mpsc::sync_channel(1);
    let (resume, release) = mpsc::sync_channel(1);
    host.next_gate = Some(TestGate {
        started,
        resume: release,
    });
    input(&mut host, Message::Shift(true));
    entered.recv_timeout(Duration::from_secs(5))?;
    assert!(host.worker.as_ref().unwrap().busy());
    // The worker owns the actual Viewer and cannot complete until released.
    // Additional shell paints must neither wait for it nor add VM visits.
    for _ in 0..100 {
        let task = host.update(Event::Frame(Instant::now()));
        assert_eq!(task.units(), 0);
        drop(host.view());
        assert!(host.queued.is_empty());
        assert!(Arc::ptr_eq(&original, &host.frontend.data));
    }
    input(&mut host, Message::Options);
    assert!(host.frontend.options_open);
    input(&mut host, Message::Shift(false));
    input(&mut host, Message::FocusLost);
    assert_eq!(host.queued.len(), 2);
    resume.send(())?;
    assert!(host.settle_for_test()?.is_empty());
    let owner = host.recover_for_test();
    assert!(!owner.shift && owner.options_open);
    assert_eq!(owner.widget_epoch, 1);
    assert!(
        owner.last.is_none(),
        "painting added a Frame call to the engine owner"
    );
    Ok(())
}

#[test]
fn closing_drains_accepted_owner_work_before_emitting_one_close_effect() -> Result<()> {
    run(closing)
}
fn closing() -> Result<()> {
    use std::time::Duration;
    let mut host = Host::from_viewer_for_test(loading());
    let (entered, resume) = host.gate_next_for_test();
    input(&mut host, Message::Shift(true));
    entered.recv_timeout(Duration::from_secs(5))?;
    input(&mut host, Message::Shift(false));
    input(&mut host, Message::FocusLost);
    let id = window::Id::unique();
    assert_eq!(host.update(Event::Close(id)).units(), 0);
    assert_eq!(host.closing, Some(id));
    // No more gameplay, frames or duplicate close effects may be queued.
    input(&mut host, Message::Shift(true));
    assert_eq!(host.queued.len(), 2);
    for _ in 0..20 {
        assert_eq!(host.update(Event::Frame(Instant::now())).units(), 0);
    }
    assert_eq!(host.queued.len(), 2);
    resume.send(())?;
    assert!(host.settle_for_test()?.is_empty());
    assert_eq!(host.update(Event::Frame(Instant::now())).units(), 1);
    assert!(host.close_sent && host.closing.is_none());
    assert_eq!(host.update(Event::Close(id)).units(), 0);
    assert_eq!(host.update(Event::Frame(Instant::now())).units(), 0);
    assert!(!host.worker.as_ref().unwrap().busy());
    let owner = host.recover_for_test();
    assert!(!owner.shift && owner.last.is_none());
    assert_eq!(owner.widget_epoch, 1);
    Ok(())
}

#[test]
fn completion_tasks_drain_and_close_without_a_window_input_or_frame_pulse() -> Result<()> {
    run(no_pulse_close)
}
fn task_action(task: Task<Event>) -> Result<iced_runtime::Action<Event>> {
    use iced::futures::StreamExt;
    let mut stream = iced_runtime::task::into_stream(task).context("task stream absent")?;
    // Watchdog thread is test-only. Production uses one oneshot per accepted
    // job on Iced's existing executor, with no polling/thread-per-frame timer.
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let waiter = std::thread::spawn(move || {
        let action = iced::futures::executor::block_on(stream.next());
        let _ = tx.send(action);
    });
    let action = rx
        .recv_timeout(std::time::Duration::from_secs(5))?
        .context("empty task")?;
    waiter.join().unwrap();
    Ok(action)
}
fn no_pulse_close() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    let (entered, resume) = host.gate_next_for_test();
    let task = host.update(Event::Widget {
        generation: 0,
        message: Message::Shift(true),
    });
    entered.recv_timeout(std::time::Duration::from_secs(5))?;
    input(&mut host, Message::FocusLost);
    let id = window::Id::unique();
    assert_eq!(host.update(Event::Close(id)).units(), 0);
    resume.send(())?;
    // Only completion outputs are delivered after release. The second wake
    // drains the queued FocusLost, then emits the one actual close effect.
    let mut task = task;
    for _ in 0..2 {
        let iced_runtime::Action::Output(Event::Completion) = task_action(task)? else {
            anyhow::bail!("missing completion wake");
        };
        task = host.update(Event::Completion);
    }
    assert!(
        matches!(task_action(task)?,iced_runtime::Action::Window(iced_runtime::window::Action::Close(actual)) if actual==id)
    );
    assert!(host.close_sent);
    let owner = host.recover_for_test();
    assert!(!owner.shift && owner.last.is_none());
    assert_eq!(owner.widget_epoch, 1);
    Ok(())
}

fn settings_host() -> Host {
    let mut host = Host::from_viewer_for_test(loading());
    Arc::make_mut(&mut host.frontend.data).session_present = true;
    input(&mut host, Message::Options);
    host
}

#[test]
fn pending_native_transport_waits_for_frame_and_preserves_input_order() -> Result<()> {
    run(pending_transport)
}
fn pending_transport() -> Result<()> {
    for message in [Message::Toggle, Message::NativeSpeed(760)] {
        let mut host = Host::from_viewer_for_test(loading());
        let publication = Arc::make_mut(&mut host.frontend.data);
        publication.native_transport = true;
        publication.pending = true;
        let (entered, resume) = host.gate_next_for_test();
        input(&mut host, message);
        let (following_entered, following_resume) = host.gate_next_for_test();
        input(&mut host, Message::Shift(true));
        assert!(host.transport_waiting());
        assert_eq!(host.queued.len(), 2);
        assert!(!host.worker.as_ref().unwrap().busy());
        assert!(entered.try_recv().is_err());
        assert_eq!(host.update(Event::Completion).units(), 0);
        assert_eq!(host.update(Event::Frame(Instant::now())).units(), 1);
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while entered.try_recv().is_err() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "transport did not follow settling frame"
            );
            drop(host.update(Event::Completion));
            std::thread::yield_now();
        }
        assert_eq!(host.queued.len(), 1);
        assert!(matches!(host.queued[0].work.message, Message::Shift(true)));
        assert!(!host.frontend.pending);
        assert!(following_entered.try_recv().is_err());
        resume.send(())?;
        while following_entered.try_recv().is_err() {
            anyhow::ensure!(Instant::now() < deadline, "following input did not run");
            drop(host.update(Event::Completion));
            std::thread::yield_now();
        }
        following_resume.send(())?;
        assert!(host.settle_for_test()?.is_empty());
        assert!(host.queued.is_empty());
        assert!(!host.worker.as_ref().unwrap().busy());
    }
    Ok(())
}

#[test]
fn modal_and_close_do_not_deadlock_deferred_transport() -> Result<()> {
    run(transport_boundaries)
}
fn transport_boundaries() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    let publication = Arc::make_mut(&mut host.frontend.data);
    publication.native_transport = true;
    publication.pending = true;
    publication.modal = true;
    input(&mut host, Message::Toggle);
    assert!(!host.transport_waiting());
    assert!(host.settle_for_test()?.is_empty());
    let publication = Arc::make_mut(&mut host.frontend.data);
    publication.native_transport = true;
    publication.pending = true;
    input(&mut host, Message::Toggle);
    assert!(host.transport_waiting());
    let id = window::Id::unique();
    let task = host.update(Event::Close(id));
    let iced_runtime::Action::Output(Event::Completion) = task_action(task)? else {
        anyhow::bail!("missing transport completion wake");
    };
    assert_eq!(host.update(Event::Completion).units(), 1);
    assert!(host.close_sent);
    assert!(host.queued.is_empty());
    Ok(())
}

#[test]
fn full_input_queue_still_allows_the_frame_that_releases_transport() -> Result<()> {
    run(full_transport_queue)
}
fn full_transport_queue() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    let publication = Arc::make_mut(&mut host.frontend.data);
    publication.native_transport = true;
    publication.pending = true;
    let (entered, resume) = host.gate_next_for_test();
    input(&mut host, Message::Toggle);
    for _ in 1..256 {
        input(&mut host, Message::Shift(true));
    }
    assert!(host.transport_waiting());
    assert_eq!(host.queued.len(), 256);
    assert_eq!(host.update(Event::Frame(Instant::now())).units(), 1);
    assert!(host.error.is_none());
    assert_eq!(host.queued.len(), 256);
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while entered.try_recv().is_err() {
        anyhow::ensure!(
            Instant::now() < deadline,
            "full queue blocked settling frame"
        );
        drop(host.update(Event::Completion));
        std::thread::yield_now();
    }
    assert_eq!(host.queued.len(), 255);
    assert!(host.error.is_none());
    resume.send(())?;
    assert!(host.settle_for_test()?.is_empty());
    assert!(host.queued.is_empty());
    Ok(())
}

#[test]
fn application_confirmation_cancel_is_shell_only_and_does_not_submit_owner_work() -> Result<()> {
    run(settings_cancel)
}
fn settings_cancel() -> Result<()> {
    let mut host = settings_host();
    let published = host.frontend.data.clone();
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::InstalledHouses),
    );
    let confirmation = host.application_confirmation.unwrap();
    for _ in 0..5 {
        assert_eq!(host.update(Event::Frame(Instant::now())).units(), 0);
    }
    input(&mut host, Message::Toggle);
    input(&mut host, Message::CancelApplicationChoice(confirmation));
    assert!(host.application_confirmation.is_none());
    assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
    assert!(Arc::ptr_eq(&published, &host.frontend.data));
    assert_eq!(host.generation(), 0);
    let owner = host.recover_for_test();
    assert!(owner.session.is_none() && owner.client.is_none() && owner.device.is_none());
    assert!(owner.last.is_none() && owner.args.neighborhood_policy.is_none());
    assert!(!owner.args.buy_preview && !owner.args.inferred_routing);
    Ok(())
}

#[test]
fn configured_restart_is_a_real_owner_replacement_and_fences_stale_config_input() -> Result<()> {
    run(settings_restart)
}
fn settings_restart() -> Result<()> {
    let mut host = settings_host();
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::BuyPreview),
    );
    let stale = host.application_confirmation.unwrap();
    input(&mut host, Message::CancelApplicationChoice(stale));
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::InferredRouting),
    );
    let current = host.application_confirmation.unwrap();
    input(&mut host, Message::ConfirmApplicationChoice(stale));
    input(&mut host, Message::CancelApplicationChoice(stale));
    assert_eq!(host.application_confirmation, Some(current));
    assert_eq!(host.generation(), 0);
    assert!(!host.worker.as_ref().unwrap().busy());
    input(&mut host, Message::ConfirmApplicationChoice(current));
    assert!(host.replacing);
    input(&mut host, Message::Shift(true));
    assert_eq!(host.settle_for_test()?.len(), 1);
    assert_eq!(host.generation(), 1);
    assert!(host.replacing && host.application_confirmation.is_none());
    for message in [
        Message::ConfirmApplicationChoice(current),
        Message::ConfiguredRestart(neighborhood_usability::Choice::InstalledHouses),
    ] {
        drop(host.update(Event::Widget {
            generation: 0,
            message,
        }));
    }
    let owner = host.recover_for_test();
    assert!(owner.args.inferred_routing && !owner.args.buy_preview);
    assert!(owner.args.neighborhood_policy.is_none());
    assert!(!owner.shift && owner.session.is_none());
    assert!(owner.args.load.is_none() && owner.args.at_tick == 0);
    Ok(())
}

#[test]
fn unconfirmed_configured_restart_and_quarantined_confirmation_are_rejected() -> Result<()> {
    run(settings_unconfirmed)
}
fn settings_unconfirmed() -> Result<()> {
    let mut host = settings_host();
    input(
        &mut host,
        Message::ConfiguredRestart(neighborhood_usability::Choice::BuyPreview),
    );
    assert!(!host.worker.as_ref().unwrap().busy());
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::BuyPreview),
    );
    let confirmation = host.application_confirmation.unwrap();
    host.error = Some("Household owner requires application restart.".into());
    input(&mut host, Message::ConfirmApplicationChoice(confirmation));
    assert!(!host.worker.as_ref().unwrap().busy());
    assert_eq!(host.generation(), 0);
    host.error = None;
    assert!(!host.recover_for_test().args.buy_preview);
    Ok(())
}

#[test]
fn configured_restart_drains_already_accepted_work_before_replacement() -> Result<()> {
    run(settings_drain)
}
fn settings_drain() -> Result<()> {
    let mut host = settings_host();
    let (entered, resume) = host.gate_next_for_test();
    input(&mut host, Message::Shift(true));
    entered.recv_timeout(std::time::Duration::from_secs(5))?;
    input(&mut host, Message::FocusLost);
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::InstalledHouses),
    );
    let confirmation = host.application_confirmation.unwrap();
    input(&mut host, Message::ConfirmApplicationChoice(confirmation));
    assert_eq!(host.queued.len(), 2);
    assert!(host.replacing);
    resume.send(())?;
    assert_eq!(host.settle_for_test()?.len(), 1);
    assert_eq!(host.generation(), 1);
    let owner = host.recover_for_test();
    assert!(!owner.shift);
    assert_eq!(owner.widget_epoch, 1);
    assert_eq!(
        owner.args.neighborhood_policy.as_deref(),
        Some(neighborhood_usability::POLICY)
    );
    Ok(())
}

#[test]
fn failed_restart_releases_replacement_fence_but_not_quarantine() -> Result<()> {
    run(restart_failure)
}
fn restart_failure() -> Result<()> {
    let mut host = settings_host();
    input(
        &mut host,
        Message::RequestApplicationChoice(neighborhood_usability::Choice::BuyPreview),
    );
    let confirmation = host.application_confirmation.unwrap();
    input(&mut host, Message::ConfirmApplicationChoice(confirmation));
    assert_eq!(host.settle_for_test()?.len(), 1);
    let generation = host.generation();
    assert!(host.replacing && host.error.is_none());
    drop(host.update(Event::Effect {
        generation: generation - 1,
        message: Message::Loaded(Err("stale load failure".into())),
    }));
    assert!(host.settle_for_test()?.is_empty());
    assert!(host.replacing && host.frontend.error.is_none());
    drop(host.update(Event::Effect {
        generation,
        message: Message::Loaded(Err("failed configured restart".into())),
    }));
    assert!(host.settle_for_test()?.is_empty());
    assert_eq!(
        host.frontend.error.as_deref(),
        Some("failed configured restart")
    );
    assert!(host.error.is_none() && !host.replacing);
    input(&mut host, Message::Restart);
    assert_eq!(host.settle_for_test()?.len(), 1);
    assert_eq!(host.generation(), generation + 1);
    assert!(host.replacing && host.frontend.error.is_none());
    drop(host.update(Event::Effect {
        generation,
        message: Message::Loaded(Err("stale retry failure".into())),
    }));
    assert!(host.settle_for_test()?.is_empty());
    assert!(host.replacing && host.frontend.error.is_none());
    drop(host.update(Event::Effect {
        generation: host.generation(),
        message: Message::Loaded(Err("failed retry".into())),
    }));
    assert!(host.settle_for_test()?.is_empty());
    assert!(!host.replacing && host.error.is_none());
    host.error = Some("quarantined owner".into());
    input(&mut host, Message::Restart);
    assert!(!host.worker.as_ref().unwrap().busy() && host.queued.is_empty());
    assert_eq!(host.generation(), generation + 1);
    let owner = host.recover_for_test();
    assert!(owner.args.buy_preview && owner.session.is_none());
    assert_eq!(owner.error.as_deref(), Some("failed retry"));
    Ok(())
}

#[test]
fn post_update_popup_overflow_invalidates_account_and_retains_old_publication() -> Result<()> {
    // Authored UNRUN: force the bookkeeping comparison to differ, then enter
    // the real Engine apply and generic worker result path. No fake save origin.
    run(|| {
        let account = window_session::loaded_input::Account::default();
        let mut viewer = loading(); viewer.loaded_input = Some(account.register());
        let mut engine = Engine::new(viewer);
        engine.popup_epoch = u64::MAX;
        engine.popup = Some(serde_json::from_value(serde_json::json!({
            "kind":"Ground","tick":0,"actor":[1,1],"target":[2,2],"clicked":null,
            "saved_poll_messages":0,"saved_stopping":0,"command":null
        }))?);
        let mut worker = match Worker::start(engine) { Ok(worker) => worker, Err(_) => anyhow::bail!("start") };
        let old = worker.published().clone();
        let (notify, ready) = std::sync::mpsc::sync_channel(1);
        let work = Work { camera_shortcut: false, save_shortcut: None, message: Message::FocusLost, shell: Shell { diagnostics: false, options_open: false, revision: 0 },
            popup: None, gate: None };
        assert!(worker.submit_notifying(work, Some(Box::new(move || { let _ = notify.send(()); }))).is_ok());
        ready.recv_timeout(std::time::Duration::from_secs(5))?;
        let completion = worker.poll().map_err(|error| anyhow::anyhow!("{error}"))?.context("completion absent")?;
        assert!(completion.reply.err().is_some_and(|error| error.contains("popup lifetime counter exhausted")));
        assert!(completion.quarantined);
        assert!(Arc::ptr_eq(&old, worker.published()));
        let _owner = worker.recover().map_err(|_| anyhow::anyhow!("owner panicked"))?;
        let terminal = account.terminal();
        assert_eq!(terminal["eligible"], false);
        assert!(terminal["invalidation_reasons"].as_array().unwrap().iter().any(|r| r == "owner_work_failed"));
        Ok(())
    })
}

#[test]
fn late_accepted_effect_after_ui_close_keeps_gameplay_and_invalidates_retirement_metadata() -> Result<()> {
    // UNRUN: close intent excludes no future effects; the existing handler
    // still executes this accepted effect and the ledger records contradiction.
    run(|| {
        let account = window_session::loaded_input::Account::default();
        let mut viewer = loading(); viewer.loaded_input = Some(account.register());
        let mut host = Host::from_viewer_for_test(viewer);
        assert_eq!(host.update(Event::Close(window::Id::unique())).units(), 1);
        assert_eq!(account.inspect_for_test()["ui_retirement_intent_lifetime"], 1);
        drop(host.update(Event::Effect { generation: 0, message: Message::FocusLost }));
        host.settle_for_test()?;
        let owner = host.recover_for_test();
        assert_eq!(owner.widget_epoch, 1, "the late accepted effect must still execute");
        let observed = account.inspect_for_test();
        assert_eq!(observed["request_channel_closed_after_ui_retirement"], true);
        assert!(observed["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "work_after_ui_retirement"));
        assert_eq!(observed["eligible"], false);
        drop(owner);
        assert_eq!(account.terminal()["worker_finalized"], false);
        Ok(())
    })
}
