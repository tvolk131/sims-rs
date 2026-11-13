use super::*;
use clap::Parser;
use sims_sim::{
    Room, Simulation,
    service_observation::{CreationMoment, CreationPhase, ContinuationMoment, FindLocationPhase},
};
use sims_vm::{Definition, Module, Object, Program, Routine, Start, Vm, World};
use std::collections::BTreeMap;

fn application() -> Application {
    let (viewer, _) = Viewer::new(Args::parse_from(["sims-room", "--paused"]));
    Application {
        viewer,
        epoch: Instant::now(),
        millis: 0,
        paths: Default::default(),
        progress_burst_ticks: 0,
        camera_pointer: Default::default(),
    }
}

fn cut() -> ServiceCut {
    creation_event(
        1,
        9,
        ObjectLifetime {
            object: 3,
            object_thread: Some(3),
        },
    )
}

fn creation_event(thread: u64, request: u64, object: ObjectLifetime) -> ServiceCut {
    ServiceCut::Creation {
        moment: CreationMoment::Registered,
        phase: CreationPhase::Factory,
        thread,
        request,
        object,
    }
}

fn request(app: &Application) -> Request {
    Request::new(app.capture_frame(), Selection::Service(cut()))
}

fn selection_fixture() -> Result<(Opening, Simulation)> {
    let source = "authored/archive::MagicStarterKit.iff";
    let mut world = World {
        scheduler_storage: Some(Default::default()),
        ..Default::default()
    };
    for identifier in 1..=3 {
        let mut object = Object {
            attributes: vec![0; 8],
            definition: Some(Definition {
                source: source.into(),
                object: 1,
                guid: if identifier == 3 { 77 } else { 1 },
                slots: 0,
            }),
            ..Default::default()
        };
        world
            .register_scheduler_object(identifier, &mut object)
            .map_err(anyhow::Error::msg)?;
        world.objects.insert(identifier, object);
    }
    world.actions = Some(Default::default());
    world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .entry(1)
        .or_default()
        .active = Some(sims_vm::action::Action {
        id: 5,
        target: 2,
        routine: 4097,
        ..Default::default()
    });
    let program = Program {
        modules: BTreeMap::from([(
            source.into(),
            Module {
                source: source.into(),
                routines: BTreeMap::from([(
                    4097,
                    Routine {
                        name: "authored selector control".into(),
                        body: sims_formats::behavior::Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![sims_formats::behavior::Instruction {
                                opcode: 2,
                                operands: [0; 8],
                                on_true: 254,
                                on_false: 255,
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut vm = Vm::new(world, 17);
    for object in 1..=3 {
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: source,
                    bhav: 4097,
                    caller: object,
                    callee: object,
                    stack_object: object,
                    args: vec![],
                    check: false,
                },
            )
            .map_err(anyhow::Error::msg)?;
        vm.object_threads.insert(object, thread);
    }
    vm.threads.get_mut(&1).unwrap().status = Status::AwaitingCreation(sims_vm::create::Request {
        id: 9,
        origin: sims_vm::Location {
            source: source.into(),
            bhav: 4097,
            instruction: 0,
        },
        caller: 1,
        target: 2,
        operands: [77, 0, 0, 0, 6, 0, 0, 0],
        created: Some(3),
        placement: None,
    });
    let identity = |object| Identity {
        object,
        handle: vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(object)
            .unwrap(),
    };
    let selection = Opening {
        cut: OpeningCut::Creation,
        actor: identity(1),
        package: identity(2),
        serial: 5,
        source: source.into(),
        contents: vec![77],
        continuation: json!({"serial":5,"package_dialog_seen":true}),
    };
    let simulation = Simulation::from_placements(
        vm,
        Room {
            width: 48,
            height: 48,
            obstacles: vec![],
        },
        BTreeMap::new(),
        (1..=3).collect(),
        None,
    )
    .map_err(anyhow::Error::msg)?;
    Ok((selection, simulation))
}

#[test]
fn local_macro_covers_every_field_and_rebases_the_actual_epoch() -> Result<()> {
    let mut app = application();
    app.camera_pointer =
        serde_json::from_value(json!({"left":[12.0,34.0],"right":true,"cancelled":true}))?;
    app.viewer.viewport_drag = serde_json::from_value(json!({
        "origin":[1.0,2.0],"endpoint":[3.0,4.0],"intent":[5.0,6.0],"camera_owned":true,"revision":7
    }))?;
    let expected = json!({
        "diagnostics":true,"options_open":true,"display_size":[321,654],"tutorial_highlight":17,
        "resident":4,"selected":5,"floor":2,"zoom":0.75,"playing":true,"speed":2.5,
        "last_ms":7,"clock_origin":[9,123],"accumulator":0.25,"pan":[-3,7],"shift":true,
        "viewport_enter_requested":true,"move_inputs":["Enter",{"Motion":[1.0,2.0]}],
        "status":"callback status","error":"callback error","menu_error":"menu error","millis":45,
        "camera_pointer":serde_json::to_value(&app.camera_pointer)?,
        "viewport_drag":serde_json::to_value(&app.viewer.viewport_drag)?,"application_revision":101,
    });
    app.apply_local(serde_json::from_value(expected.clone())?);
    assert_eq!(serde_json::to_value(app.local())?, expected);
    let frame = app.capture_frame();
    let viewer = &app.viewer;
    let sampled = local_state!(viewer, frame.epoch, frame.millis, frame.camera_pointer);
    assert_eq!(serde_json::to_value(sampled)?, expected);
    assert!(app.viewer.service_capture.is_none());
    Ok(())
}

#[test]
fn dispatch_refreshes_owned_frame_inputs_immediately_before_update() -> Result<()> {
    let mut app = application();
    app.viewer.service_capture = Some(request(&app));
    app.millis = 81;
    app.epoch += Duration::from_millis(3);
    app.dispatch(Message::Shift(true))?;
    let frame = &app.viewer.service_capture.as_ref().unwrap().frame;
    assert_eq!(frame.epoch, app.epoch);
    assert_eq!(frame.millis, 81);
    assert_eq!(
        serde_json::to_value(&frame.camera_pointer)?,
        serde_json::to_value(&app.camera_pointer)?
    );
    assert!(app.viewer.shift);
    Ok(())
}

#[test]
fn local_sample_does_not_freeze_pre_update_or_post_return_fields() -> Result<()> {
    let mut app = application();
    let before = serde_json::to_value(app.local())?;
    app.viewer.resident = 7;
    app.viewer.accumulator = 0.5;
    app.viewer.status = "at invocation".into();
    app.millis = 40;
    let frame = app.capture_frame();
    let viewer = &app.viewer;
    let at_call = local_state!(viewer, frame.epoch, frame.millis, frame.camera_pointer);
    app.viewer.status = "after invocation".into();
    app.viewer.resident = 8;
    let after = serde_json::to_value(app.local())?;
    let sampled = serde_json::to_value(at_call)?;
    assert_ne!(sampled, before);
    assert_ne!(sampled, after);
    assert_eq!(sampled["status"], "at invocation");
    assert_eq!(sampled["resident"], 7);
    assert_eq!(sampled["accumulator"], 0.5);
    assert_eq!(sampled["millis"], 40);
    Ok(())
}

#[test]
fn failed_serialization_does_not_publish_and_first_complete_capture_wins() -> Result<()> {
    let app = application();
    let mut request = request(&app);
    assert!(
        request
            .retain(
                cut(),
                json!({"partial":true}),
                Value::Null,
                || anyhow::bail!("authored serializer failure")
            )
            .is_err()
    );
    assert!(request.captured.is_none());
    let bytes = b"complete owned bytes".to_vec();
    request.retain(cut(), json!({"first":true}), json!({"serial":5}), || {
        Ok(bytes.clone())
    })?;
    request.retain(cut(), json!({"second":true}), Value::Null, || {
        panic!("first capture was replaced")
    })?;
    let captured = request.captured.unwrap();
    assert_eq!(captured.checkpoint, bytes);
    assert_eq!(captured.boundary, json!({"first":true}));
    assert_eq!(captured.continuation, json!({"serial":5}));
    Ok(())
}

#[test]
fn command_error_clears_request_but_retains_a_complete_capture() -> Result<()> {
    let mut app = application();
    let request = request(&app);
    let mut polls = 0;
    let outcome = app.execute_capturing(
        &Command::Playing { value: false },
        &Limits::default(),
        request,
        &mut |app| {
            polls += 1;
            app.viewer.service_capture.as_mut().unwrap().retain(
                cut(),
                Value::Null,
                Value::Null,
                || Ok(b"owned capture".to_vec()),
            )?;
            anyhow::bail!("authored later external observer failure")
        },
    )?;
    assert!(outcome.execution.is_err());
    assert_eq!(polls, 1);
    assert_eq!(outcome.captured.unwrap().checkpoint, b"owned capture");
    assert!(app.viewer.service_capture.is_none());
    Ok(())
}

#[test]
fn capture_error_clears_request_without_publishing_partial_bytes() -> Result<()> {
    let mut app = application();
    let request = request(&app);
    let outcome = app.execute_capturing(
        &Command::Playing { value: false },
        &Limits::default(),
        request,
        &mut |app| {
            app.viewer.service_capture.as_mut().unwrap().retain(
                cut(),
                Value::Null,
                Value::Null,
                || anyhow::bail!("authored encode failure"),
            )
        },
    )?;
    assert!(outcome.execution.is_err());
    assert!(outcome.captured.is_none());
    assert!(app.viewer.service_capture.is_none());
    Ok(())
}

#[test]
fn nested_command_capture_is_rejected_without_replacing_the_installed_request() {
    let mut app = application();
    app.viewer.service_capture = Some(request(&app));
    let second = request(&app);
    assert!(
        app.execute_capturing(&Command::Poll, &Limits::default(), second, &mut |_| Ok(()))
            .is_err()
    );
    assert!(app.viewer.service_capture.is_some());
}

#[test]
fn ordinary_creation_selection_uses_event_bindings_not_native_creation_map_membership() -> Result<()>
{
    let (selection, simulation) = selection_fixture()?;
    assert!(simulation.native_creations.is_empty());
    let before = serde_json::to_vec(&simulation)?;
    let boundary = selection.boundary(cut(), &simulation)?.unwrap();
    assert_eq!(boundary["request"]["id"], 9);
    assert!(boundary["creation"].is_null());
    assert_eq!(serde_json::to_vec(&simulation)?, before);
    Ok(())
}

#[test]
fn registered_creation_preserves_truthful_absence_of_a_child_thread() -> Result<()> {
    let (selection, mut simulation) = selection_fixture()?;
    simulation.vm.object_threads.remove(&3);
    let event = creation_event(
        1,
        9,
        ObjectLifetime {
            object: 3,
            object_thread: None,
        },
    );
    assert!(selection.boundary(event, &simulation)?.is_some());
    assert!(selection.boundary(cut(), &simulation)?.is_none());
    Ok(())
}

#[test]
fn queued_only_or_wrong_active_action_cannot_match() -> Result<()> {
    let (selection, initial) = selection_fixture()?;
    for changed in [0, 1, 2, 3] {
        let mut simulation = initial.clone();
        let actor = simulation
            .vm
            .world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .get_mut(&1)
            .unwrap();
        match changed {
            0 => {
                let active = actor.active.take().unwrap();
                actor.queue.pending.push_back(active);
            }
            1 => actor.active.as_mut().unwrap().id += 1,
            2 => actor.active.as_mut().unwrap().routine += 1,
            _ => actor.active.as_mut().unwrap().target = 3,
        }
        assert!(selection.boundary(cut(), &simulation)?.is_none());
    }
    Ok(())
}

#[test]
fn wrong_event_thread_request_or_object_lifetime_cannot_match() -> Result<()> {
    let (selection, simulation) = selection_fixture()?;
    for event in [
        creation_event(
            2,
            9,
            ObjectLifetime {
                object: 3,
                object_thread: Some(3),
            },
        ),
        creation_event(
            1,
            10,
            ObjectLifetime {
                object: 3,
                object_thread: Some(3),
            },
        ),
        creation_event(
            1,
            9,
            ObjectLifetime {
                object: 2,
                object_thread: Some(2),
            },
        ),
        creation_event(
            1,
            9,
            ObjectLifetime {
                object: 3,
                object_thread: None,
            },
        ),
        creation_event(
            1,
            9,
            ObjectLifetime {
                object: 3,
                object_thread: Some(2),
            },
        ),
    ] {
        assert!(selection.boundary(event, &simulation)?.is_none());
    }
    Ok(())
}

#[test]
fn wrong_request_source_caller_content_or_handle_cannot_match() -> Result<()> {
    let (selection, initial) = selection_fixture()?;
    for changed in 0..6 {
        let mut simulation = initial.clone();
        let Status::AwaitingCreation(wait) = &mut simulation.vm.threads.get_mut(&1).unwrap().status
        else {
            unreachable!()
        };
        match changed {
            0 => wait.origin.source = "authored/archive::Other.iff".into(),
            1 => wait.caller = 2,
            2 => wait.operands[0] = 78,
            3 => wait.created = Some(2),
            4 => {
                simulation
                    .vm
                    .world
                    .objects
                    .get_mut(&3)
                    .unwrap()
                    .definition
                    .as_mut()
                    .unwrap()
                    .guid = 78
            }
            _ => simulation.vm.world.objects.get_mut(&2).unwrap().attributes[7] = 1,
        }
        assert!(selection.boundary(cut(), &simulation)?.is_none());
    }
    let mut wrong_handle = selection;
    wrong_handle.actor.handle += 1;
    assert!(wrong_handle.boundary(cut(), &initial).is_err());
    wrong_handle.actor.handle -= 1;
    wrong_handle.package.handle += 1;
    assert!(wrong_handle.boundary(cut(), &initial).is_err());
    Ok(())
}

#[test]
fn awaiting_animation_without_retained_resident_yield_is_not_a_hit() -> Result<()> {
    let (mut selection, mut simulation) = selection_fixture()?;
    selection.cut = OpeningCut::Animation;
    simulation.vm.threads.get_mut(&1).unwrap().status =
        Status::AwaitingAnimation(sims_vm::AnimationRequest {
            id: 9,
            origin: sims_vm::Location {
                source: selection.source.clone(),
                bhav: 4100,
                instruction: 1,
            },
            caller: 1,
            operands: [0; 8],
        });
    let event = ServiceCut::AwaitingAnimation {
        thread: 1,
        request: 9,
        object: ObjectLifetime {
            object: 1,
            object_thread: Some(1),
        },
    };
    assert!(selection.boundary(event, &simulation)?.is_none());
    Ok(())
}

#[test]
fn placement_without_its_retained_native_lot_owner_is_not_a_hit() -> Result<()> {
    let (mut selection, mut simulation) = selection_fixture()?;
    selection.cut = OpeningCut::Placement;
    simulation.vm.threads.get_mut(&1).unwrap().status =
        Status::AwaitingPlacement(sims_vm::placement::Request {
            id: 9,
            origin: sims_vm::Location {
                source: selection.source.clone(),
                bhav: 4097,
                instruction: 1,
            },
            caller: 1,
            target: 3,
            reference: 2,
            operands: [0; 8],
        });
    let event = ServiceCut::FindLocation {
        moment: ContinuationMoment::Installed,
        phase: FindLocationPhase::Search,
        thread: 1,
        request: 9,
        object: ObjectLifetime {
            object: 3,
            object_thread: Some(3),
        },
    };
    assert!(simulation.native_lot.is_none());
    assert!(selection.boundary(event, &simulation)?.is_none());
    Ok(())
}
