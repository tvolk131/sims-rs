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
fn render_scene_publication_keeps_original_scene_and_a_coherent_cached_frame() -> Result<()> {
    run(render_scene_publication)
}

fn render_scene_publication() -> Result<()> {
    let mut viewer = loading();
    let mut source = sims_render::software::rgba_scene(4, 4, vec![137; 4 * 4 * 4])?;
    source.censors.push(sims_render::censor::Region {
        rect: [0, 0, 4, 4],
        depth: 0.9,
        seed: 17,
    });
    let source = Arc::new(source);
    let pixels = sims_render::software::render(&source).into_raw();
    viewer.scene = Some(source.clone());
    let mut owner = Engine::new(viewer);
    let first = owner.publish().map_err(anyhow::Error::msg)?;
    assert!(Arc::ptr_eq(first.scene.as_ref().unwrap(), &source));
    let prepared = first.render_scene.as_ref().unwrap();
    assert!(prepared.censors.is_empty());
    assert_eq!(prepared.textures[0].rgba.as_ref(), pixels);
    let repeated = owner.publish().map_err(anyhow::Error::msg)?;
    assert!(Arc::ptr_eq(
        prepared,
        repeated.render_scene.as_ref().unwrap()
    ));
    owner.viewer.scene = None;
    let cleared = owner.publish().map_err(anyhow::Error::msg)?;
    assert!(cleared.scene.is_none() && cleared.render_scene.is_none());
    assert_eq!(prepared.textures[0].rgba.as_ref(), pixels);
    assert!(Arc::ptr_eq(first.scene.as_ref().unwrap(), &source));
    Ok(())
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
fn save_feedback_distinguishes_accepted_work_from_each_real_completion() -> Result<()> {
    run(save_feedback_completions)
}

fn save_feedback_completions() -> Result<()> {
    use std::time::Duration;
    let mut viewer = loading();
    let output = std::env::temp_dir().join(format!(
        "sims-unloaded-save-feedback-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    assert!(!output.exists());
    viewer.args.save = output.clone();
    let mut host = Host::from_viewer_for_test(viewer);
    let before = host.frontend.data.clone();
    let (first_entered, first_resume) = host.gate_next_for_test();
    input(&mut host, Message::Save);
    first_entered.recv_timeout(Duration::from_secs(5))?;
    assert!(host.saving());
    assert_eq!(host.save_feedback.requested, 1);
    assert!(Arc::ptr_eq(&before, &host.frontend.data));
    assert!(!output.exists());
    let (second_entered, second_resume) = host.gate_next_for_test();
    input(&mut host, Message::Save);
    let feedback = host.save_feedback.observation(host.saving());
    assert_eq!(feedback["requested"], 2);
    assert_eq!(feedback["submitted"]["request"], 1);
    assert!(feedback["completed"].is_null());
    assert_eq!(host.queued.len(), 1);
    for _ in 0..10 {
        drop(host.update(Event::Frame(Instant::now())));
        drop(host.view());
    }
    assert_eq!(host.queued.len(), 1);
    assert_eq!(host.save_feedback.observation(true), feedback);
    assert!(Arc::ptr_eq(&before, &host.frontend.data));
    first_resume.send(())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.save_feedback.observation(host.saving())["completed"]["request"].as_u64() != Some(1)
    {
        drop(host.update(Event::Completion));
        ensure!(Instant::now() < deadline, "first Save did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
    second_entered.recv_timeout(Duration::from_secs(5))?;
    let feedback = host.save_feedback.observation(host.saving());
    assert!(host.saving());
    assert_eq!(feedback["submitted"]["request"], 2);
    assert_eq!(feedback["completed"]["request"], 1);
    assert!(host.frontend.error.is_some());
    assert!(!host.frontend.status.starts_with("Saved "));
    assert!(!output.exists());
    second_resume.send(())?;
    host.settle_for_test()?;
    assert!(!host.saving());
    assert_eq!(
        host.save_feedback.observation(false)["completed"]["request"],
        2
    );
    assert!(!output.exists());
    let owner = host.recover_for_test();
    assert!(owner.session.is_none() && owner.last.is_none());
    assert!(owner.error.is_some());
    Ok(())
}

#[test]
fn rejected_save_inputs_do_not_publish_an_accepted_request() -> Result<()> {
    run(rejected_save_feedback)
}

#[test]
fn save_feedback_retires_aborted_queue_but_keeps_a_running_save_pending() -> Result<()> {
    run(aborted_save_queue)
}

fn aborted_save_queue() -> Result<()> {
    use std::time::Duration;
    for saving_first in [false, true] {
        let mut host = Host::from_viewer_for_test(loading());
        let (entered, resume) = host.gate_next_for_test();
        input(
            &mut host,
            if saving_first {
                Message::Save
            } else {
                Message::Shift(false)
            },
        );
        entered.recv_timeout(Duration::from_secs(5))?;
        input(&mut host, Message::Save);
        assert!(host.saving());
        for _ in 0..256 {
            host.enqueue(0, Message::Shift(false));
        }
        assert!(host.error.as_deref().unwrap().contains("queue is full"));
        assert_eq!(host.saving(), saving_first);
        let requested = host.save_feedback.requested;
        assert_eq!(requested, if saving_first { 2 } else { 1 });
        resume.send(())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.worker.as_ref().unwrap().busy() {
            drop(host.poll());
            ensure!(Instant::now() < deadline, "blocked owner did not return");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!host.saving());
        assert_eq!(host.queued.len(), 256);
        assert_eq!(host.save_feedback.requested, requested);
        let observation = host.save_feedback.observation(host.saving());
        if saving_first {
            assert_eq!(observation["completed"]["request"], 1);
        } else {
            assert!(observation["submitted"].is_null());
            assert!(observation["completed"].is_null());
        }
        let owner = host.recover_for_test();
        assert!(owner.session.is_none() && owner.last.is_none());
    }
    Ok(())
}

#[test]
fn submitted_save_disconnect_retires_pending_without_a_completion() -> Result<()> {
    run(disconnected_save)
}

fn disconnected_save() -> Result<()> {
    use std::time::Duration;
    let mut host = Host::from_viewer_for_test(loading());
    let publication = host.frontend.data.clone();
    let (entered, resume) = host.gate_next_for_test();
    host.next_gate.as_mut().unwrap().disconnect = true;
    input(&mut host, Message::Save);
    entered.recv_timeout(Duration::from_secs(5))?;
    assert!(host.saving());
    let before = host.save_feedback.observation(true);
    assert_eq!(before["submitted"]["request"], 1);
    resume.send(())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while host.error.is_none() {
        drop(host.poll());
        ensure!(Instant::now() < deadline, "owner did not disconnect");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(host.error.as_deref().unwrap().contains("disconnected"));
    assert!(!host.saving());
    let after = host.save_feedback.observation(false);
    assert_eq!(after["requested"], 1);
    assert!(after["submitted"].is_null() && after["completed"].is_null());
    assert!(Arc::ptr_eq(&publication, &host.frontend.data));
    input(&mut host, Message::Save);
    assert_eq!(host.save_feedback.requested, 1);
    assert!(host.worker.take().unwrap().recover().is_err());
    Ok(())
}

fn rejected_save_feedback() -> Result<()> {
    let mut host = Host::from_viewer_for_test(loading());
    drop(host.update(Event::Widget {
        generation: 1,
        message: Message::Save,
    }));
    assert_eq!(host.save_feedback.requested, 0);
    host.replacing = true;
    input(&mut host, Message::Save);
    assert_eq!(host.save_feedback.requested, 0);
    host.replacing = false;
    for _ in 0..256 {
        host.enqueue(0, Message::Shift(false));
    }
    input(&mut host, Message::Save);
    assert_eq!(host.queued.len(), 256);
    assert_eq!(host.save_feedback.requested, 0);
    assert!(!host.saving());
    assert!(host.error.as_deref().unwrap().contains("queue is full"));
    let owner = host.recover_for_test();
    assert!(owner.session.is_none() && owner.last.is_none());
    let mut host = Host::from_viewer_for_test(loading());
    host.save_feedback.requested = u64::MAX;
    input(&mut host, Message::Save);
    assert!(host.queued.is_empty());
    assert_eq!(host.save_feedback.requested, u64::MAX);
    assert!(!host.saving());
    assert!(
        host.error
            .as_deref()
            .unwrap()
            .contains("Save request counter exhausted")
    );
    let owner = host.recover_for_test();
    assert!(owner.session.is_none() && owner.last.is_none());
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
        disconnect: false,
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
