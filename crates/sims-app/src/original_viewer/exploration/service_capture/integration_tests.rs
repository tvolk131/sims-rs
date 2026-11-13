use super::*;
use sims_sim::service_observation::{CreationMoment, CreationPhase};

#[path = "authored_fixture.rs"]
mod authored_fixture;

fn bytes(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

fn state(session: &Session) -> Result<Vec<u8>> {
    bytes(&(
        session.save(),
        &session.house,
        &session.program,
        &session.index().resources,
        session
            .index()
            .resources
            .iter()
            .map(|resource| (&resource.locator, resource.owned.as_deref()))
            .collect::<Vec<_>>(),
    ))
}

fn assert_transient(cut: ServiceCut, session: &Session) -> Result<()> {
    let ServiceCut::Creation {
        moment,
        phase,
        thread,
        request,
        object,
    } = cut
    else {
        anyhow::bail!("expected real native registry Creation, got {cut:?}");
    };
    assert_eq!(moment, CreationMoment::Registered);
    assert_eq!(phase, CreationPhase::Factory);
    assert_eq!(object.object, 5);
    assert_eq!(
        object.object_thread,
        session.simulation.vm.object_threads.get(&5).copied()
    );
    assert!(matches!(
        &session.simulation.vm.threads[&thread].status,
        Status::AwaitingCreation(owner) if owner.id == request && owner.created == Some(5)
    ));
    session
        .simulation
        .native_scheduler
        .as_ref()
        .context("native registry absent")?
        .validate(&session.simulation)
        .map_err(anyhow::Error::msg)?;
    assert!(session.simulation.fixture_driver.is_none());
    Ok(())
}

fn expected_cut(application: &Application, local: &Local) -> Result<(ServiceCut, Value, Value)> {
    let mut session = application.session()?.fork_for_exploration()?;
    let client = DialogClient::restore_managed(
        &session,
        application
            .viewer
            .client
            .as_ref()
            .context("client absent")?
            .save(application.session()?)?,
    )?;
    let device = application
        .viewer
        .device
        .as_ref()
        .map(|device| CameraDevice::restore(&session, device.save(application.session()?)?))
        .transpose()?;
    let mut captured = None;
    session.step_with_service_observer(&mut |cut, view| {
        assert_transient(cut, view)?;
        ensure!(captured.is_none(), "authored fixture produced a second cut");
        let before = state(view)?;
        let ordinary = capture_envelope(view, &client, device.as_ref(), local, false)?;
        let managed = capture_envelope(view, &client, device.as_ref(), local, true)?;
        assert_eq!(state(view)?, before);
        captured = Some((cut, ordinary, managed));
        Ok(())
    })?;
    captured.context("authored registry did not invoke its real service callback")
}

fn assert_shape(ordinary: &Value, managed: &Value) {
    let keys = [
        "application",
        "camera_device",
        "dialog_client",
        "schema",
        "session",
    ];
    assert_eq!(
        ordinary
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        keys
    );
    assert_eq!(
        managed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        keys
    );
    assert_eq!(ordinary["schema"], 1);
    assert_eq!(managed["schema"], 1);
    for key in ["application", "camera_device", "dialog_client"] {
        assert_eq!(ordinary[key], managed[key], "{key}");
    }
    assert_eq!(ordinary["session"]["version"], 1);
    assert!(ordinary["session"].get("managed").is_none());
    assert_eq!(managed["session"]["version"], 2);
    assert!(managed["session"]["managed"].is_object());
    let mut normalized = managed["session"].clone();
    normalized["version"] = json!(1);
    normalized.as_object_mut().unwrap().remove("managed");
    assert_eq!(ordinary["session"], normalized);
}

fn restore_after_unwind(application: &Application, checkpoint: &[u8]) -> Result<Application> {
    let restored = Application::restore_checkpoint(
        &application.viewer.args,
        application.session()?.index(),
        application.viewer.icons.as_ref().context("icons absent")?,
        application.epoch + Duration::from_secs(17),
        checkpoint,
    )?;
    assert_eq!(restored.checkpoint()?, checkpoint);
    assert!(restored.viewer.service_capture.is_none());
    Ok(restored)
}

fn callback_local(application: &Application, message: &Message) -> Result<Local> {
    let mut reference = restore_after_unwind(application, &application.checkpoint()?)?;
    reference.viewer.prepare_viewport_drag(message);
    Ok(reference.local())
}

fn missing_renderer(result: Result<()>) -> Result<String> {
    let error =
        result.expect_err("asset-free application unexpectedly acquired adult renderer assets");
    let message = format!("{error:#}");
    ensure!(
        message.contains("adult-skeleton.cmx.bcf"),
        "unexpected application failure instead of the unavailable renderer binding: {message}"
    );
    Ok(message)
}

#[test]
fn authored_full_envelope_enters_only_through_validated_restore_and_real_codecs() -> Result<()> {
    let application = authored_fixture::application()?;
    let session = application.session()?;
    assert!(
        session
            .index()
            .resources
            .iter()
            .all(|resource| resource.owned.is_some())
    );
    assert!(session.host.presentation.clock.is_none());
    assert!(session.host.presentation.application_parent.is_none());
    assert_eq!(
        session.options.routing,
        sims_sim::lot_route::path::Policy::Strict
    );
    assert_eq!(
        session
            .simulation
            .native_lot
            .as_ref()
            .unwrap()
            .routing_policy,
        sims_sim::lot_route::path::Policy::Strict
    );
    assert!(application.viewer.device.is_none());
    let checkpoint = application.checkpoint()?;
    let restored = restore_after_unwind(&application, &checkpoint)?;
    assert_eq!(state(restored.session()?)?, state(session)?);
    let value: Value = serde_json::from_slice(&checkpoint)?;
    let client = application.viewer.client.as_ref().unwrap();
    assert_eq!(
        value["dialog_client"],
        serde_json::to_value(client.save(session)?)?
    );
    assert!(value["camera_device"].is_null());
    Ok(())
}

#[test]
fn actual_step_callback_samples_full_envelope_before_transient_owner_unwinds() -> Result<()> {
    let mut application = authored_fixture::application()?;
    let before = application.local();
    let mut callback = callback_local(&application, &Message::Step)?;
    callback.playing = false;
    let (cut, ordinary, expected) = expected_cut(&application, &callback)?;
    assert_shape(&ordinary, &expected);
    application.viewer.service_capture = Some(Request::new(
        application.capture_frame(),
        Selection::Service(cut),
    ));
    let execution = application.dispatch(Message::Step);
    let request = application
        .viewer
        .service_capture
        .take()
        .context("request disappeared")?;
    let publication_error = missing_renderer(execution)?;
    let captured = request
        .captured
        .context("Step did not reach real callback")?;
    assert_eq!(captured.cut, cut);
    assert_eq!(captured.boundary, json!({"service": cut}));
    assert_eq!(captured.continuation, Value::Null);
    assert_eq!(captured.checkpoint, bytes(&expected)?);
    assert_ne!(expected["application"], serde_json::to_value(before)?);
    assert_eq!(expected["application"], serde_json::to_value(callback)?);
    assert_ne!(captured.checkpoint, application.checkpoint()?);
    assert_ne!(
        expected["session"]["simulation"],
        serde_json::to_value(application.session()?.save().simulation)?
    );
    let saved: Saved = serde_json::from_value(expected["session"].clone())?;
    assert_transient(
        cut,
        &Session::restore(application.session()?.index(), saved)?,
    )?;
    let mut resumed = restore_after_unwind(&application, &captured.checkpoint)?;
    assert_eq!(
        missing_renderer(resumed.dispatch(Message::Step))?,
        publication_error
    );
    assert_eq!(state(resumed.session()?)?, state(application.session()?)?);
    assert_eq!(resumed.checkpoint()?, application.checkpoint()?);
    assert_eq!(resumed.session()?.simulation.vm.world.objects.len(), 5);
    Ok(())
}

#[test]
fn actual_frame_callback_distinguishes_preupdate_callback_and_return_local() -> Result<()> {
    let mut application = authored_fixture::application()?;
    let before = application.local();
    let mut callback = callback_local(
        &application,
        &Message::Frame(application.epoch + Duration::from_millis(53)),
    )?;
    callback.millis += 40;
    callback.last_ms = Some(callback.millis);
    callback.accumulator = (callback.accumulator + 0.04_f32 * callback.speed).min(0.32);
    let (cut, ordinary, expected) = expected_cut(&application, &callback)?;
    assert_shape(&ordinary, &expected);
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let mut polls = 0;
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| {
            polls += 1;
            Ok(())
        },
    )?;
    missing_renderer(outcome.execution)?;
    assert_eq!(polls, 0);
    assert!(application.viewer.service_capture.is_none());
    let captured = outcome
        .captured
        .context("Frame did not reach real callback")?;
    assert_eq!(captured.checkpoint, bytes(&expected)?);
    let during = &expected["application"];
    let returned = serde_json::to_value(application.local())?;
    assert_ne!(*during, serde_json::to_value(before)?);
    assert_ne!(*during, returned);
    assert_eq!(during["millis"], 53);
    assert_eq!(during["last_ms"], 53);
    assert!(during["accumulator"].as_f64().unwrap() > returned["accumulator"].as_f64().unwrap());
    assert_eq!(during["camera_pointer"], returned["camera_pointer"]);
    assert!(restore_after_unwind(&application, &captured.checkpoint).is_ok());
    Ok(())
}

#[test]
fn actual_callback_observation_preserves_full_state_trace_rng_and_bounded_polls() -> Result<()> {
    for service_polls in [1, 8] {
        let mut ordinary = authored_fixture::application()?;
        let mut observed = authored_fixture::application()?;
        let (cut, _, _) = expected_cut(&ordinary, &ordinary.local())?;
        let limits = Limits {
            service_polls,
            ..Default::default()
        };
        let before_rng = ordinary.session()?.simulation.vm.lot_rng_state;
        let mut ordinary_polls = 0;
        let ordinary_execution =
            ordinary.execute_observed(&Command::Advance { millis: 40 }, &limits, &mut |_| {
                ordinary_polls += 1;
                Ok(())
            });
        let mut observed_polls = 0;
        let request = Request::new(observed.capture_frame(), Selection::Service(cut));
        let outcome = observed.execute_capturing(
            &Command::Advance { millis: 40 },
            &limits,
            request,
            &mut |_| {
                observed_polls += 1;
                Ok(())
            },
        )?;
        assert_eq!(
            missing_renderer(outcome.execution)?,
            missing_renderer(ordinary_execution)?
        );
        assert!(outcome.captured.is_some());
        assert_eq!(ordinary_polls, observed_polls);
        assert_eq!(ordinary_polls, 0);
        assert_eq!(ordinary.checkpoint()?, observed.checkpoint()?);
        assert_eq!(ordinary.paths, observed.paths);
        assert_ne!(before_rng, observed.session()?.simulation.vm.lot_rng_state);
        let trace_application = authored_fixture::application()?;
        let initial = trace_application.session()?;
        let mut reference = initial.fork_for_exploration()?;
        let mut instrumented = initial.fork_for_exploration()?;
        let mut expected_trace = vec![];
        let mut actual_trace = vec![];
        let expected_progress = reference.step_with_trace(&mut expected_trace)?;
        let local = observed.local();
        let client = DialogClient::restore_managed(
            &instrumented,
            trace_application
                .viewer
                .client
                .as_ref()
                .unwrap()
                .save(initial)?,
        )?;
        let mut calls = 0;
        let actual_progress = instrumented.step_with_trace_and_service_observer(
            &mut actual_trace,
            &mut |event, view| {
                assert_transient(event, view)?;
                calls += 1;
                let before = state(view)?;
                capture_envelope(view, &client, None, &local, true)?;
                assert_eq!(state(view)?, before);
                Ok(())
            },
        )?;
        assert_eq!(calls, 1);
        assert!(!expected_trace.is_empty());
        assert_eq!(actual_trace, expected_trace);
        assert_eq!(actual_progress, expected_progress);
        assert_eq!(state(&instrumented)?, state(&reference)?);
    }
    Ok(())
}

#[test]
fn actual_service_callback_serializes_real_optional_device_without_replacement_owners() -> Result<()>
{
    for present in [false, true] {
        let application = authored_fixture::application()?;
        let mut session = application.session()?.fork_for_exploration()?;
        let client = DialogClient::restore_managed(
            &session,
            application
                .viewer
                .client
                .as_ref()
                .unwrap()
                .save(application.session()?)?,
        )?;
        let device = present
            .then(|| authored_fixture::device(&session))
            .transpose()?;
        let local = application.local();
        let (cut, _, _) = expected_cut(&application, &local)?;
        let mut request = Request::new(application.capture_frame(), Selection::Service(cut));
        let mut expected = None;
        session.step_with_service_observer(&mut |event, view| {
            assert_transient(event, view)?;
            let ordinary = capture_envelope(view, &client, device.as_ref(), &local, false)?;
            let managed = capture_envelope(view, &client, device.as_ref(), &local, true)?;
            assert_shape(&ordinary, &managed);
            assert_eq!(managed["camera_device"].is_object(), present);
            if let Some(device) = &device {
                assert_eq!(
                    managed["camera_device"],
                    serde_json::to_value(device.save(view)?)?
                );
            }
            request.observe(event, view, &client, device.as_ref(), &local)?;
            expected = Some(bytes(&managed)?);
            Ok(())
        })?;
        let captured = request.captured.context("real callback did not publish")?;
        assert_eq!(Some(&captured.checkpoint), expected.as_ref());
        let resumed = restore_after_unwind(&application, &captured.checkpoint)?;
        assert_eq!(resumed.viewer.device.is_some(), present);
    }
    Ok(())
}

#[test]
fn actual_step_with_real_device_captures_before_missing_camera_presenter_rejects_publication()
-> Result<()> {
    let mut application = authored_fixture::application()?;
    application.viewer.device = Some(authored_fixture::device(application.session()?)?);
    let mut reference = authored_fixture::application()?;
    reference.viewer.device = Some(authored_fixture::device(reference.session()?)?);
    reference.viewer.prepare_viewport_drag(&Message::Step);
    reference.viewer.playing = false;
    assert!(reference.viewer.poll_application_ui()?);
    let (cut, ordinary, expected) = expected_cut(&reference, &reference.local())?;
    assert_shape(&ordinary, &expected);
    assert!(expected["camera_device"].is_object());
    application.viewer.service_capture = Some(Request::new(
        application.capture_frame(),
        Selection::Service(cut),
    ));
    let execution = application.dispatch(Message::Step);
    let request = application
        .viewer
        .service_capture
        .take()
        .context("request disappeared")?;
    let error = format!("{:#}", execution.unwrap_err());
    assert!(error.contains("camera presenter absent"), "{error}");
    let captured = request
        .captured
        .context("real device prevented callback capture")?;
    assert_eq!(captured.checkpoint, bytes(&expected)?);
    assert_eq!(application.session()?.simulation.vm.world.globals[&12], 0);
    let restored = restore_after_unwind(&application, &captured.checkpoint)?;
    assert!(restored.viewer.device.is_some());
    assert!(restored.session()?.host.presentation.presenter.is_none());
    Ok(())
}

#[test]
fn actual_managed_serializer_failure_clears_request_without_partial_publication() -> Result<()> {
    let mut application = authored_fixture::application()?;
    let (cut, _, _) = expected_cut(&application, &application.local())?;
    application
        .viewer
        .session
        .as_mut()
        .unwrap()
        .house
        .sources
        .insert(
            authored_fixture::HOUSE.into(),
            engine::digest(b"not the owned authored house"),
        );
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| Ok(()),
    )?;
    assert!(
        outcome
            .execution
            .unwrap_err()
            .to_string()
            .contains("original import source changed")
    );
    assert!(outcome.captured.is_none());
    assert!(application.viewer.service_capture.is_none());
    assert_transient(cut, application.session()?)?;
    Ok(())
}

#[test]
fn actual_dialog_serializer_failure_clears_request_without_partial_publication() -> Result<()> {
    let mut application = authored_fixture::application()?;
    let (cut, _, _) = expected_cut(&application, &application.local())?;
    let cache = &mut application
        .viewer
        .session
        .as_mut()
        .unwrap()
        .simulation
        .vm
        .world
        .image_cache;
    assert!(!cache.refs.is_empty());
    cache.refs[0] = 0;
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| Ok(()),
    )?;
    assert!(outcome.execution.is_err());
    assert!(outcome.captured.is_none());
    assert!(application.viewer.service_capture.is_none());
    assert_transient(cut, application.session()?)?;
    Ok(())
}

#[test]
fn real_device_clock_mismatch_rejects_at_callback_without_retiming_or_partial_record() -> Result<()>
{
    let mut application = authored_fixture::application()?;
    let mut future = authored_fixture::live_session(application.session()?.index())?;
    let initial_tick = future.simulation.vm.tick;
    future.step()?;
    assert!(future.simulation.vm.tick > initial_tick);
    let device = authored_fixture::device(&future)?;
    let device_before = bytes(&device.save(&future)?)?;
    let local = application.local();
    let (cut, _, _) = expected_cut(&application, &local)?;
    application.viewer.device = Some(device);
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| Ok(()),
    )?;
    let error = outcome.execution.unwrap_err();
    assert!(
        format!("{error:#}").contains("camera surface belongs to another simulation tick"),
        "{error:#}"
    );
    assert!(outcome.captured.is_none());
    assert!(application.viewer.service_capture.is_none());
    assert_eq!(
        bytes(&application.viewer.device.as_ref().unwrap().save(&future)?)?,
        device_before
    );
    assert_transient(cut, application.session()?)?;
    Ok(())
}

#[test]
fn later_real_publication_error_preserves_earlier_complete_callback_record_separately() -> Result<()>
{
    let mut application = authored_fixture::application()?;
    let mut callback = callback_local(
        &application,
        &Message::Frame(application.epoch + Duration::from_millis(53)),
    )?;
    callback.millis += 40;
    callback.last_ms = Some(callback.millis);
    callback.accumulator = (callback.accumulator + 0.04_f32 * callback.speed).min(0.32);
    let (cut, _, expected) = expected_cut(&application, &callback)?;
    let icons = application.viewer.icons.take().context("icons absent")?;
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| Ok(()),
    )?;
    assert!(
        outcome
            .execution
            .unwrap_err()
            .to_string()
            .contains("dialog icons absent")
    );
    assert!(application.viewer.service_capture.is_none());
    let captured = outcome
        .captured
        .context("later error discarded complete earlier record")?;
    assert_eq!(captured.checkpoint, bytes(&expected)?);
    assert!(
        serde_json::from_slice::<Value>(&captured.checkpoint)?["application"]["error"].is_null()
    );
    application.viewer.icons = Some(icons);
    restore_after_unwind(&application, &captured.checkpoint)?;
    Ok(())
}

#[test]
fn absent_real_client_fails_before_callback_and_request_does_not_leak() -> Result<()> {
    let mut application = authored_fixture::application()?;
    let (cut, _, _) = expected_cut(&application, &application.local())?;
    application.viewer.client = None;
    let request = Request::new(application.capture_frame(), Selection::Service(cut));
    let outcome = application.execute_capturing(
        &Command::Advance { millis: 40 },
        &Limits::default(),
        request,
        &mut |_| Ok(()),
    )?;
    assert!(
        outcome
            .execution
            .unwrap_err()
            .to_string()
            .contains("dialog client absent")
    );
    assert!(outcome.captured.is_none());
    assert!(application.viewer.service_capture.is_none());
    assert!(
        !application
            .session()?
            .simulation
            .vm
            .world
            .objects
            .contains_key(&5)
    );
    Ok(())
}
