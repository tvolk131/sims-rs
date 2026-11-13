use super::super::super::worker_host::{Event, Host};
use super::presentation_latency::{distribution, frozen_load};
use super::*;
use iced::futures::StreamExt;

const REPETITIONS: usize = 7;

fn call(
    reference: &mut Application,
    host: &mut Host,
    message: Message,
    updates: &mut Vec<Value>,
) -> Result<()> {
    let label = format!("{message:?}");
    let event = match message {
        Message::Frame(now) => Event::Frame(now),
        _ => host.ui_event_for_test(host.publication_for_test(), message.clone()),
    };
    let started = Instant::now();
    let task = host.update(event);
    let shell_us = started.elapsed().as_secs_f64() * 1e6;
    let wakes = task.units();
    ensure!(wakes <= 1, "unexpected shell task count: {wakes}");
    let mut completion_us = None;
    if wakes == 1 {
        let mut stream =
            iced_runtime::task::into_stream(task).context("completion stream absent")?;
        let event =
            iced::futures::executor::block_on(stream.next()).context("completion absent")?;
        ensure!(
            matches!(event, iced_runtime::Action::Output(Event::Completion)),
            "unexpected task effect"
        );
        let completion_started = Instant::now();
        ensure!(
            host.update(Event::Completion).units() == 0,
            "unexpected completion effect"
        );
        completion_us = Some(completion_started.elapsed().as_secs_f64() * 1e6);
    }
    let dispatch_through_completion_us = started.elapsed().as_secs_f64() * 1e6;
    ensure!(
        reference.viewer.update(message).units() == 0,
        "unexpected inline reference effect"
    );
    reference.check_errors()?;
    ensure!(
        reference.viewer.presentation_pending() == host.publication_for_test().pending,
        "worker and reference pending flags differ"
    );
    updates.push(json!({"message":label,"shell_us":shell_us,
        "dispatch_through_completion_us":dispatch_through_completion_us,
        "completion_handler_us":completion_us,"completion_wakes":wakes,
        "pending":host.publication_for_test().pending}));
    Ok(())
}

fn finish(reference: &mut Application, host: &mut Host, updates: &mut Vec<Value>) -> Result<()> {
    let now = reference.epoch + Duration::from_millis(reference.millis);
    for _ in 0..Limits::default().service_polls {
        if !host.publication_for_test().pending {
            return Ok(());
        }
        call(reference, host, Message::Frame(now), updates)?;
    }
    anyhow::bail!("ordinary presentation continuation exceeded service budget")
}

fn operation(
    reference: &mut Application,
    host: &mut Host,
    label: &str,
    repetition: usize,
    message: Message,
    operations: &mut Vec<Value>,
) -> Result<()> {
    let mut updates = vec![];
    call(reference, host, message, &mut updates)?;
    finish(reference, host, &mut updates)?;
    operations.push(json!({"label":label,"repetition":repetition,
        "dispatch_through_completion_us":updates.iter()
            .map(|row| row["dispatch_through_completion_us"].as_f64().unwrap()).sum::<f64>(),
        "updates":updates}));
    Ok(())
}

fn summary(operations: &[Value]) -> Value {
    let mut groups: std::collections::BTreeMap<&str, Vec<&Value>> = Default::default();
    for operation in operations {
        groups
            .entry(operation["label"].as_str().unwrap())
            .or_default()
            .push(operation);
    }
    groups
        .into_iter()
        .map(|(label, rows)| {
            let total = |rows: &[&Value]| {
                distribution(
                    rows.iter()
                        .map(|row| row["dispatch_through_completion_us"].as_f64().unwrap())
                        .collect(),
                )
            };
            let shell = distribution(
                rows.iter()
                    .map(|row| row["updates"][0]["shell_us"].as_f64().unwrap())
                    .collect(),
            );
            let mut value = json!({"shell_dispatch":shell,"all_dispatch_to_completion":total(&rows),
            "first_dispatch_to_completion_us":rows[0]["dispatch_through_completion_us"]});
            if rows.len() > 1 {
                value["warm_dispatch_to_completion"] = total(&rows[1..]);
            }
            (label.to_owned(), value)
        })
        .collect()
}

#[test]
#[ignore = "private paused Mashuga and Goth modal; real production Host completion tasks, bounded latency and full recovery equality"]
fn production_host_gui_handlers_and_modal_latency() -> Result<()> {
    run_worker(campaign)
}

fn campaign() -> Result<()> {
    let out = output()?;
    let setup = Instant::now();
    let mut reference = frozen_load(&out)?;
    select(&mut reference, 196)?;
    let mut recovered = reference.restore(&reference.checkpoint()?)?;
    let mut host = Host::from_viewer_for_test(recovered.viewer);
    ensure!(
        host.publication_for_test().audit.is_none(),
        "timing refuses full-state audit overhead"
    );
    let setup_us = setup.elapsed().as_secs_f64() * 1e6;
    let before = reference.observe()?;
    let mut ui = Input::new()?;
    let mut operations = vec![];
    for name in ["Options", "Needs"] {
        let (_, message) = sequences::control(&mut ui, &reference, name)?;
        for repetition in 0..REPETITIONS {
            operation(
                &mut reference,
                &mut host,
                name,
                repetition,
                message.clone(),
                &mut operations,
            )?;
            if name == "Options" {
                operation(
                    &mut reference,
                    &mut host,
                    "Options-close",
                    repetition,
                    message.clone(),
                    &mut operations,
                )?;
            }
        }
    }
    let residents: Vec<_> = reference
        .session()?
        .house
        .household
        .as_ref()
        .context("household absent")?
        .residents
        .iter()
        .map(|resident| resident.object as i16)
        .collect();
    ensure!(residents.len() >= 2, "fixture has fewer than two residents");
    for resident in residents.iter().take(2).rev() {
        sequences::control(&mut ui, &reference, &format!("Resident({resident})"))?;
    }
    for repetition in 0..REPETITIONS {
        for (label, resident) in [
            ("Resident-other", residents[1]),
            ("Resident-return", residents[0]),
        ] {
            operation(
                &mut reference,
                &mut host,
                label,
                repetition,
                Message::Resident(resident),
                &mut operations,
            )?;
            ensure!(
                reference.viewer.resident == resident,
                "resident selection did not change"
            );
        }
    }
    let (_, detail) = sequences::control(&mut ui, &reference, "MotiveDetails(0)")?;
    for repetition in 0..REPETITIONS {
        operation(
            &mut reference,
            &mut host,
            "MotiveDetails",
            repetition,
            detail.clone(),
            &mut operations,
        )?;
        ensure!(
            reference
                .viewer
                .client
                .as_ref()
                .and_then(|client| client.live_panel())
                .and_then(|panel| panel.popup())
                == Some(0),
            "motive popup did not open"
        );
        if repetition == 0 {
            sequences::control(&mut ui, &reference, "CloseMotiveDetails")?;
        }
        operation(
            &mut reference,
            &mut host,
            "CloseMotiveDetails",
            repetition,
            Message::CloseMotiveDetails,
            &mut operations,
        )?;
    }
    for repetition in 0..REPETITIONS {
        for (label, message) in [("Buy", Message::Buy), ("Live", Message::Live)] {
            if repetition == 0 {
                sequences::control(&mut ui, &reference, label)?;
            }
            operation(
                &mut reference,
                &mut host,
                label,
                repetition,
                message,
                &mut operations,
            )?;
        }
    }
    let after = reference.observe()?;
    ensure!(
        before.tick == after.tick && before.rng == after.rng,
        "paused GUI commands advanced tick/RNG"
    );
    recovered.viewer = host.recover_for_test();
    ensure!(
        reference.capture(false)? == recovered.capture(false)?,
        "production owner recovery differs from complete inline state/RNG/continuations"
    );
    write_json(
        &out.join("handler-measurements.json"),
        &json!({"passed":true,"setup_us":setup_us,"summary":summary(&operations),
        "operations":operations,"before":before,"after":after,"complete_recovery_equal":true}),
    )?;

    let modal_out = out.join("modal-fixture");
    std::fs::create_dir(&modal_out)?;
    let modal_setup = Instant::now();
    let mut modal = modal_lifetimes::load_car_pool(&modal_out)?;
    let mut modal_recovered = modal.restore(&modal.checkpoint()?)?;
    let mut host = Host::from_viewer_for_test(modal_recovered.viewer);
    let modal_setup_us = modal_setup.elapsed().as_secs_f64() * 1e6;
    let (_, answer) = sequences::control(&mut ui, &modal, "Dialog(149, 2)")?;
    let mut dialog_views = vec![];
    presentation_latency::measure(&modal, &out, "car-pool-modal", &mut dialog_views)?;
    let mut dialog_operations = vec![];
    operation(
        &mut modal,
        &mut host,
        "Dialog(149, 2)",
        0,
        answer,
        &mut dialog_operations,
    )?;
    modal_recovered.viewer = host.recover_for_test();
    ensure!(
        modal.capture(false)? == modal_recovered.capture(false)?,
        "modal response changed complete worker state/RNG/continuations"
    );
    ensure!(
        modal.session()?.simulation.vm.world.dialogs.modal.is_none(),
        "real Car Pool modal did not close"
    );
    write_json(
        &out.join("result.json"),
        &json!({"passed":true,
        "profile":if cfg!(debug_assertions){"debug"}else{"release"},
        "repetitions":REPETITIONS,"setup_us":setup_us,"modal_setup_us":modal_setup_us,
        "summary":summary(&operations),"operations":operations,
        "dialog_summary":summary(&dialog_operations),"dialog_operations":dialog_operations,
        "dialog_views":dialog_views,"complete_recovery_equal":true,
        "before":before,"after":after,"full_state_audit_inside_timing":false,
        "scope":"Paused restored Mashuga GUI commands through production Host and real completion task, plus one retained Goth Car Pool dialog answer. Shell dispatch is distinct from dispatch-through-worker-completion; summed continuation timing excludes the intervening reference calls. Immutable offscreen work is not compositor/window FPS, fresh startup or active-play evidence. No full-state snapshots per update."}),
    )
}
