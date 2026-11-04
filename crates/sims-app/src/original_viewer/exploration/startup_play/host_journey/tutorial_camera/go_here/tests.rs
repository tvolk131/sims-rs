use super::super::super::super::super::super::ui_publication::Choice;
use super::*;

fn sample() -> JourneyGroundPixel {
    JourneyGroundPixel {
        pixel: [80., 80.],
        picked: None,
        point: Some([160, 176]),
        error: None,
    }
}

fn viewport() -> Rectangle {
    Rectangle::new(Point::ORIGIN, iced::Size::new(400., 320.))
}

fn action() -> Action {
    Action {
        target: 210,
        icon_owner: 210,
        menu_id: 6,
        priority: 50,
        flags: 12,
        routine: 4096,
        ..Default::default()
    }
}

fn owner() -> PopupOwner {
    PopupOwner {
        kind: sims_runtime::original_session::live_menu::Kind::Ground,
        tick: 7,
        actor: (105, 1001),
        target: (210, 2001),
        clicked: None,
        command: None,
    }
}

fn ground_input() -> JourneyGroundInput {
    JourneyGroundInput {
        tick: 7,
        actor: 105,
        point: [160, 176],
        floor: 1,
        target: Some(210),
        target_lifetime: Some(2001),
        ready: true,
        complete: false,
        actions: vec![action()],
    }
}

fn menu() -> Menu {
    Menu {
        title: "Go Here".into(),
        diagnostic_title: "Move here".into(),
        preview: false,
        ready: true,
        diagnostic: vec![],
        popup: vec![Choice {
            label: "Go Here".into(),
            intent: Some(Intent::GroundChoice(0)),
        }],
        cancel: Some(Intent::GroundCancel),
        error: None,
        popup_visible: true,
    }
}

fn offer() -> Offer {
    Offer { cut: Cut { generation: 7, host_widget_epoch: 9, publication_widget_epoch: 11, popup_epoch: 13 },
        owner: serde_json::from_value(json!({"kind":"Ground","tick":7,"actor":[105,1001],
            "target":[210,2001],"clicked":null,"saved_poll_messages":0,"saved_stopping":0,"command":null})).unwrap(),
        input: ground_input(), index: 0, action: action(), serial: 30,
        previous: vec![Action { id: 29, ..action() }], actor_lifetime: 1001 }
}

fn native() -> JourneyGround {
    JourneyGround {
        scene: Arc::new(Scene {
            vertices: vec![],
            draws: vec![],
            censors: vec![],
            textures: Arc::new(vec![]),
            width: 200,
            height: 160,
        }),
        display_size: [400, 400],
        widget_epoch: 11,
        tick: 7,
        actor: 105,
        actor_lifetime: 1001,
        floor: 1,
        camera: Input {
            viewport: [0, 0, 200, 160],
            center: [0, 0],
            zoom: 2,
            rotation: 0,
        },
        lot: Arc::new(sims_sim::lot_geometry::Lot {
            size: 64,
            active_size: None,
            heights: vec![],
            pool: vec![],
            water: vec![],
            floors: vec![],
        }),
        lot_size: 64,
        input: None,
        object_input: false,
        serial: 31,
        queue: vec![Action { id: 31, ..action() }],
        queue_targets: BTreeMap::from([(210, Some(2001))]),
    }
}

fn instruction_record() -> Record {
    serde_json::from_value(json!({"id":91,"caller":322,"stack_object":48,"source":SOURCE,
        "origin":{"source":SOURCE,"bhav":4125,"instruction":3},"operands":[0,1,14,0,0,4,0,135],
        "title":"Tutorial","body":"synthetic instruction boundary","buttons":[{"label":"OK","result":2}],
        "status":1,"bitmap_icon":5001,"icon_object":null,"job_icon":null,"global_bitmap_icon":null,"portrait":null,
        "presentation":{"anchored":true,"hide_title":true,"large":false}})).unwrap()
}

#[test]
fn tutorial_go_here_discards_empty_wake_streams_and_preserves_real_outputs() {
    assert!(active_task(Task::none()).is_none());
    let empty: Task<Event> = Task::batch([Task::none(), Task::batch([Task::none(), Task::none()])]);
    assert_eq!(empty.units(), 0);
    assert!(iced_runtime::task::into_stream(empty).is_some());
    assert!(active_task(Task::batch([Task::none(), Task::none()])).is_none());
    let real = active_task(Task::batch([Task::none(), Task::done(Event::Completion)])).unwrap();
    let actions = iced::futures::executor::block_on(
        iced_runtime::task::into_stream(real)
            .unwrap()
            .collect::<Vec<_>>(),
    );
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions.first(),
        Some(iced_runtime::Action::Output(Event::Completion))
    ));
}

#[test]
fn tutorial_go_here_task_limit_rejects_before_dispatch() {
    let dispatched = std::cell::Cell::new(false);
    let rejected = Usage {
        tasks: LIMITS.tasks,
        ..Default::default()
    }
    .run(
        Usage {
            commands: 1,
            effect_polls: 1,
            tasks: 1,
            ..Default::default()
        },
        || dispatched.set(true),
    );
    assert!(rejected.is_err());
    assert!(!dispatched.get());
    Usage {
        tasks: LIMITS.tasks - 1,
        ..Default::default()
    }
    .run(
        Usage {
            tasks: 1,
            ..Default::default()
        },
        || dispatched.set(true),
    )
    .unwrap();
    assert!(dispatched.get());
}

#[test]
fn tutorial_go_here_pixel_limit_rejects_before_native_pick() {
    let probes = std::cell::Cell::new(0);
    let delta = Usage {
        pixel_scans: 1,
        ..Default::default()
    };
    Usage {
        pixel_scans: LIMITS.pixel_scans - 1,
        ..Default::default()
    }
    .run(delta, || probes.set(probes.get() + 1))
    .unwrap();
    assert_eq!(probes.get(), 1);
    assert!(
        Usage {
            pixel_scans: LIMITS.pixel_scans,
            ..Default::default()
        }
        .run(delta, || probes.set(probes.get() + 1))
        .is_err()
    );
    assert_eq!(probes.get(), 1);
}

#[test]
fn tutorial_go_here_point_requires_finite_native_interior_and_current_floor() {
    let qualify = |sample: &JourneyGroundPixel, floor, bounds| {
        point(sample, floor, 1, [200, 160], 64, bounds, &[])
    };
    let (position, native) = qualify(&sample(), 1, viewport()).unwrap();
    assert_eq!([position.x, position.y], [160., 160.]);
    assert_eq!(native, [160, 176]);
    for pixel in [
        [f32::NAN, 80.],
        [80., f32::INFINITY],
        [-1., 80.],
        [200., 80.],
        [80., 160.],
        [19., 80.],
        [181., 80.],
    ] {
        assert!(qualify(&JourneyGroundPixel { pixel, ..sample() }, 1, viewport()).is_err());
    }
    for native in [[-1, 176], [0, 176], [1008, 176], [160, i32::MAX]] {
        assert!(
            qualify(
                &JourneyGroundPixel {
                    point: Some(native),
                    ..sample()
                },
                1,
                viewport()
            )
            .is_err()
        );
    }
    assert!(
        qualify(
            &JourneyGroundPixel {
                point: None,
                ..sample()
            },
            1,
            viewport()
        )
        .is_err()
    );
    assert!(
        qualify(
            &JourneyGroundPixel {
                picked: Some(105),
                ..sample()
            },
            1,
            viewport()
        )
        .is_err()
    );
    assert!(
        qualify(
            &JourneyGroundPixel {
                error: Some("native floor absent".into()),
                ..sample()
            },
            1,
            viewport()
        )
        .is_err()
    );
    assert!(qualify(&sample(), 2, viewport()).is_err());
    assert!(qualify(&sample(), 0, viewport()).is_err());
    assert!(
        qualify(
            &sample(),
            1,
            Rectangle {
                width: f32::NAN,
                ..viewport()
            }
        )
        .is_err()
    );
    assert!(
        qualify(
            &sample(),
            1,
            Rectangle {
                height: 0.,
                ..viewport()
            }
        )
        .is_err()
    );
    assert!(point(&sample(), 1, 1, [0, 160], 64, viewport(), &[]).is_err());
}

#[test]
fn tutorial_go_here_point_rejects_hud_overlay_and_letterbox_margins() {
    let covered = Rectangle::new(Point::new(150., 150.), iced::Size::new(30., 30.));
    assert!(point(&sample(), 1, 1, [200, 160], 64, viewport(), &[covered]).is_err());
    let letterboxed = Rectangle::new(Point::new(10., 20.), iced::Size::new(500., 200.));
    let (position, _) = point(&sample(), 1, 1, [200, 160], 64, letterboxed, &[]).unwrap();
    assert_eq!([position.x, position.y], [235., 120.]);
    let margin = Rectangle::new(Point::new(10., 20.), iced::Size::new(120., 200.));
    assert!(!margin.contains(position));
    assert!(
        point(
            &JourneyGroundPixel {
                pixel: [-10., 80.],
                ..sample()
            },
            1,
            1,
            [200, 160],
            64,
            letterboxed,
            &[]
        )
        .is_err()
    );
    assert!(point(&sample(), 1, 1, [200, 160], 64, letterboxed, &[letterboxed]).is_err());
    assert!(
        point(
            &sample(),
            1,
            1,
            [200, 160],
            64,
            viewport(),
            &[Rectangle {
                x: f32::NAN,
                ..covered
            }]
        )
        .is_err()
    );
}

#[test]
fn tutorial_go_here_popup_rejects_preview_heading_wrong_owner_and_stale_action() {
    let qualify = |menu: &Menu, input: &JourneyGroundInput, owner: &PopupOwner| {
        offered_index(menu, input, owner, 1001, [160, 176], 1)
    };
    assert_eq!(qualify(&menu(), &ground_input(), &owner()).unwrap(), 0);
    for changed in [
        Menu {
            preview: true,
            ..menu()
        },
        Menu {
            ready: false,
            ..menu()
        },
        Menu {
            popup_visible: false,
            ..menu()
        },
        Menu {
            popup: vec![],
            ..menu()
        },
        Menu {
            popup: vec![Choice {
                label: "Go Here".into(),
                intent: None,
            }],
            ..menu()
        },
        Menu {
            popup: vec![Choice {
                label: "Go Here".into(),
                intent: Some(Intent::ObjectChoice(0)),
            }],
            ..menu()
        },
        Menu {
            popup: vec![Choice {
                label: "Sit".into(),
                intent: Some(Intent::GroundChoice(0)),
            }],
            ..menu()
        },
        Menu {
            popup: vec![Choice {
                label: "Go Here".into(),
                intent: Some(Intent::GroundChoice(1)),
            }],
            ..menu()
        },
    ] {
        assert!(qualify(&changed, &ground_input(), &owner()).is_err());
    }
    for changed in [
        PopupOwner {
            actor: (106, 1001),
            ..owner()
        },
        PopupOwner {
            actor: (105, 1002),
            ..owner()
        },
        PopupOwner {
            target: (211, 2001),
            ..owner()
        },
        PopupOwner {
            target: (210, 2002),
            ..owner()
        },
        PopupOwner { tick: 8, ..owner() },
        PopupOwner {
            clicked: Some((210, 2001)),
            ..owner()
        },
        PopupOwner {
            command: Some(sims_runtime::original_session::live_menu::Command::Choose(
                0,
            )),
            ..owner()
        },
        PopupOwner {
            kind: sims_runtime::original_session::live_menu::Kind::Object,
            ..owner()
        },
    ] {
        assert!(qualify(&menu(), &ground_input(), &changed).is_err());
    }
    for changed in [
        JourneyGroundInput {
            floor: 2,
            ..ground_input()
        },
        JourneyGroundInput {
            point: [176, 160],
            ..ground_input()
        },
        JourneyGroundInput {
            actor: 106,
            ..ground_input()
        },
        JourneyGroundInput {
            ready: false,
            ..ground_input()
        },
        JourneyGroundInput {
            complete: true,
            ..ground_input()
        },
        JourneyGroundInput {
            actions: vec![],
            ..ground_input()
        },
        JourneyGroundInput {
            actions: vec![Action { id: 29, ..action() }],
            ..ground_input()
        },
        JourneyGroundInput {
            actions: vec![Action {
                target: 211,
                ..action()
            }],
            ..ground_input()
        },
    ] {
        assert!(qualify(&menu(), &changed, &owner()).is_err());
    }
    same_offered_action(&offer(), 0, &ground_input()).unwrap();
    assert!(same_offered_action(&offer(), 1, &ground_input()).is_err());
    assert!(
        same_offered_action(
            &offer(),
            0,
            &JourneyGroundInput {
                actions: vec![Action {
                    routine: 4097,
                    ..action()
                }],
                ..ground_input()
            }
        )
        .is_err()
    );
}

#[test]
fn tutorial_go_here_enqueue_requires_fresh_serial_exact_action_target_lifetime_and_closure() {
    assert_eq!(fresh_enqueue(&offer(), &native(), true).unwrap(), Some(31));
    assert_eq!(fresh_enqueue(&offer(), &native(), false).unwrap(), None);
    assert_eq!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                input: Some(ground_input()),
                ..native()
            },
            true
        )
        .unwrap(),
        None
    );
    assert_eq!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                object_input: true,
                ..native()
            },
            true
        )
        .unwrap(),
        None
    );
    for queued in [
        Action { id: 30, ..action() },
        Action { id: 29, ..action() },
        Action {
            id: 31,
            target: 211,
            ..action()
        },
        Action {
            id: 31,
            menu_id: 7,
            ..action()
        },
        Action {
            id: 31,
            routine: 4097,
            ..action()
        },
        Action {
            id: 31,
            args: [1, 0, 0, 0],
            ..action()
        },
        Action {
            id: 31,
            label: Some("Go Here".into()),
            ..action()
        },
    ] {
        assert_eq!(
            fresh_enqueue(
                &offer(),
                &JourneyGround {
                    queue: vec![queued],
                    ..native()
                },
                true
            )
            .unwrap(),
            None
        );
    }
    assert_eq!(
        fresh_enqueue(
            &Offer {
                previous: vec![Action { id: 31, ..action() }],
                ..offer()
            },
            &native(),
            true
        )
        .unwrap(),
        None
    );
    assert_eq!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                queue: vec![],
                ..native()
            },
            true
        )
        .unwrap(),
        None
    );
    assert_eq!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                queue_targets: BTreeMap::from([(210, Some(2002))]),
                ..native()
            },
            true
        )
        .unwrap(),
        None
    );
    assert_eq!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                serial: 30,
                ..native()
            },
            true
        )
        .unwrap(),
        None
    );
    for changed in [
        JourneyGround {
            actor: 106,
            ..native()
        },
        JourneyGround {
            actor_lifetime: 1002,
            ..native()
        },
        JourneyGround {
            floor: 2,
            ..native()
        },
    ] {
        assert!(fresh_enqueue(&offer(), &changed, true).is_err());
    }
    assert!(
        fresh_enqueue(
            &offer(),
            &JourneyGround {
                serial: 32,
                queue: vec![Action { id: 31, ..action() }, Action { id: 32, ..action() }],
                ..native()
            },
            true
        )
        .is_err()
    );
}

#[test]
fn tutorial_go_here_first_new_instruction_never_implies_enqueue_or_allows_a_response() {
    let expected = instruction_record();
    let mut next = expected.clone();
    next.id += 1;
    assert!(!first_instruction(&expected, Some(&expected), false, false).unwrap());
    assert!(first_instruction(&expected, Some(&next), false, false).is_err());
    assert!(first_instruction(&expected, Some(&expected), true, false).is_err());
    assert!(first_instruction(&expected, None, false, false).is_err());
    assert!(first_instruction(&expected, Some(&next), false, true).unwrap());
    assert!(first_instruction(&expected, Some(&expected), true, true).unwrap());
    assert!(!first_instruction(&expected, None, false, true).unwrap());
    assert!(!first_instruction(&expected, Some(&expected), false, true).unwrap());
}

#[test]
fn tutorial_go_here_budget_reserves_the_next_poll_task_command_and_pixel_before_work() {
    for (at_limit, delta) in [
        (
            Usage {
                effect_polls: LIMITS.effect_polls,
                ..Default::default()
            },
            Usage {
                effect_polls: 1,
                ..Default::default()
            },
        ),
        (
            Usage {
                tasks: LIMITS.tasks,
                ..Default::default()
            },
            Usage {
                tasks: 1,
                ..Default::default()
            },
        ),
        (
            Usage {
                commands: LIMITS.commands,
                ..Default::default()
            },
            Usage {
                commands: 1,
                ..Default::default()
            },
        ),
        (
            Usage {
                pixel_scans: LIMITS.pixel_scans,
                ..Default::default()
            },
            Usage {
                pixel_scans: 1,
                ..Default::default()
            },
        ),
    ] {
        at_limit.check().unwrap();
        assert!(at_limit.reserve(delta).is_err());
    }
    assert!(
        Usage {
            frames: LIMITS.frames - 1,
            ..Default::default()
        }
        .reserve(Usage {
            service_polls: 2,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn tutorial_walking_disabled_preserves_legacy_observe_before_submit() {
    let mut state = (Vec::new(), 1_usize);
    let result: Result<()> = walking::keep_then_observe(
        false,
        &mut state,
        Task::done(Event::Completion),
        |state, task| {
            state.0.push("register");
            state.1 += usize::from(active_task(task).is_some());
            Ok(())
        },
        |state| {
            state.0.push("observe");
            anyhow::bail!("existing Go Here boundary");
        },
    );
    assert!(result.is_err());
    assert_eq!(state, (vec!["observe"], 1));
}

#[test]
fn tutorial_walking_registers_returned_task_before_boundary_and_retains_outstanding_streams() {
    let mut state = (Vec::new(), 1_usize);
    let result: Result<()> = walking::keep_then_observe(
        true,
        &mut state,
        Task::done(Event::Completion),
        |state, task| {
            state.0.push("register");
            state.1 += usize::from(active_task(task).is_some());
            Ok(())
        },
        |state| {
            state.0.push("observe");
            anyhow::bail!("newly published boundary");
        },
    );
    assert!(result.is_err());
    assert_eq!(state, (vec!["register", "observe"], 2));
}

#[test]
fn tutorial_walking_zero_work_does_not_register_and_submit_error_still_observes() {
    for task in [Task::none(), Task::batch([Task::none(), Task::none()])] {
        let mut state = (0_usize, false);
        let result: Result<()> = walking::keep_then_observe(
            true,
            &mut state,
            task,
            |state, task| {
                state.0 += usize::from(active_task(task).is_some());
                anyhow::bail!("post-registration capacity check");
            },
            |state| {
                state.1 = true;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(state, (0, true));
    }
}
