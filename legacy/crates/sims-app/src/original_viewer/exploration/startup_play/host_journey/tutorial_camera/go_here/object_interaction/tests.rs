use super::super::super::super::super::super::super::ui_publication::{
    Choice, JourneyObjectIdentity,
};
use super::*;

fn action() -> Action {
    Action {
        target: 230,
        icon_owner: 231,
        menu_id: 9,
        priority: 50,
        flags: 12,
        routine: 4100,
        ..Default::default()
    }
}

fn native() -> walking::Native {
    walking::Native {
        tick: 200,
        actor: BOB_OBJECT,
        actor_lifetime: Some(1001),
        position: Some(sims_vm::autonomy::Placement {
            x: 80,
            y: 96,
            level: 1,
        }),
        owner: Some(420),
        owner_lifetime: Some(5001),
        owner_source: Some(SOURCE.into()),
        children_pending: false,
        clock_pending: false,
        input_pending: false,
        serial: 40,
        active: None,
        pending: vec![],
        previous: None,
        targets: BTreeMap::new(),
        dialog_statuses: BTreeMap::new(),
    }
}

fn record() -> Record {
    serde_json::from_value(
        json!({"id":702,"caller":420,"stack_object":48,"source":SOURCE,
        "origin":{"source":SOURCE,"bhav":4125,"instruction":7},"operands":[0,2,17,0,0,4,0,135],
        "title":"Tutorial","body":"synthetic metadata only; not a captured instruction",
        "buttons":[{"label":"OK","result":2}],"status":1,"bitmap_icon":5002,
        "icon_object":null,"job_icon":null,"global_bitmap_icon":null,"portrait":null,
        "presentation":{"anchored":true,"hide_title":true,"large":false}}),
    )
    .unwrap()
}

fn input() -> JourneyObjectInput {
    JourneyObjectInput {
        tick: 200,
        actor: (105, 1001),
        clicked: (229, 2000),
        resolved: (230, 2001),
        ready: true,
        actions: vec![action()],
        action_targets: BTreeMap::from([(230, 2001)]),
    }
}

fn pick() -> Pick {
    Pick {
        pixel: [80., 80.],
        clicked: (229, 2000),
        resolved: (230, 2001),
    }
}

fn object_snapshot() -> JourneyObject {
    JourneyObject {
        progress: None,
        objects: [(229, 2000), (230, 2001), (240, 3000)]
            .into_iter()
            .map(|(object, lifetime)| {
                (
                    object,
                    JourneyObjectIdentity {
                        lifetime: Some(lifetime),
                        person: false,
                        loaded_interaction_table_count: Some(1),
                    },
                )
            })
            .collect(),
        input: None,
        ground_input: false,
        pointer_pending: false,
    }
}

fn owner() -> sims_runtime::original_session::live_menu::State {
    serde_json::from_value(json!({"kind":"Object","tick":200,"actor":[105,1001],
        "target":[230,2001],"clicked":[229,2000],"saved_poll_messages":0,"saved_stopping":0,"command":null})).unwrap()
}

fn menu() -> Menu {
    Menu {
        title: "Synthetic object".into(),
        diagnostic_title: "not evidence".into(),
        preview: false,
        ready: true,
        diagnostic: vec![],
        popup: vec![Choice {
            label: "Offered synthetic leaf".into(),
            intent: Some(Intent::ObjectChoice(0)),
        }],
        cancel: Some(Intent::ObjectCancel),
        error: None,
        popup_visible: true,
    }
}

fn cut() -> Cut {
    Cut {
        generation: 3,
        host_widget_epoch: 4,
        publication_widget_epoch: 5,
        popup_epoch: 6,
    }
}

fn offer() -> Offer {
    Offer {
        cut: cut(),
        owner: owner(),
        input: input(),
        index: 0,
        label: "Offered synthetic leaf".into(),
        action: action(),
        serial: 40,
        prior_ids: BTreeSet::from([35, 39, 40]),
    }
}

#[test]
fn tutorial_object_absent_zero_preserves_the_disabled_seam() {
    for raw in [None, Some("0")] {
        let enabled = env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION", |_| {
            raw.map(str::to_owned).ok_or(std::env::VarError::NotPresent)
        })
        .unwrap();
        let mut retained = json!({"receipt":{"walking":"unchanged"},"events":["walking_stop"],
            "streams":1,"tasks":8,"idle":false,"observations":0,"redraws":0,
            "notifications":0,"commands":0,"frames":0,"responses":0});
        let before = retained.clone();
        let result = object_continuation(enabled, || {
            retained["observations"] = json!(1);
            panic!("disabled seam must not enter continuation")
        })
        .unwrap();
        assert!(result.is_none());
        assert_eq!(retained, before);
    }
}

#[test]
fn tutorial_object_dependency_matrix_is_fresh_house_and_all_flags() {
    for camera in [false, true] {
        for go_here in [false, true] {
            for walking in [false, true] {
                assert_eq!(
                    super::super::super::object_dependencies(
                        camera,
                        go_here,
                        walking,
                        "UserData/Houses/House07.iff"
                    )
                    .is_ok(),
                    camera && go_here && walking
                );
            }
        }
    }
    assert!(
        super::super::super::object_dependencies(true, true, true, "UserData/Houses/House05.iff")
            .is_err()
    );
}

#[test]
fn tutorial_object_exact_entry_rejects_stale_or_altered_pins() {
    let mut prior = record();
    prior.id -= 4;
    entry_pins(&prior, &record(), ENTRY_BODY).unwrap();
    assert!(entry_pins(&prior, &record(), "wrong body hash").is_err());
    let mutations: &[fn(&mut Record)] = &[
        |record| record.id = 1,
        |record| record.caller += 1,
        |record| record.origin.instruction += 1,
        |record| record.origin.bhav += 1,
        |record| record.source.push(' '),
        |record| record.origin.source.push(' '),
        |record| record.stack_object += 1,
        |record| record.status = 2,
        |record| record.bitmap_icon = None,
        |record| record.presentation.as_mut().unwrap().large = true,
        |record| record.presentation.as_mut().unwrap().anchored = false,
        |record| record.buttons[0].result = 3,
        |record| record.buttons[0].label.push(' '),
    ];
    for mutation in mutations {
        let mut changed = record();
        mutation(&mut changed);
        assert!(entry_pins(&prior, &changed, ENTRY_BODY).is_err());
    }
    for index in 0..8 {
        let mut changed = record();
        changed.operands[index] ^= 1;
        assert!(entry_pins(&prior, &changed, ENTRY_BODY).is_err());
    }
}

#[test]
fn tutorial_object_retained_stream_is_not_inferred_from_pending() {
    assert!(!drained(1, false).unwrap());
    assert!(input_gate(1, true, false).is_err());
    assert!(input_gate(0, true, true).is_err());
    input_gate(0, true, false).unwrap();
    let initial = native();
    let mut changed = initial.clone();
    changed.children_pending = true;
    notification_unchanged(&initial, &changed).unwrap();
    changed.tick += 1;
    assert!(notification_unchanged(&initial, &changed).is_err());
    changed = initial.clone();
    changed.position.as_mut().unwrap().x += 1;
    assert!(notification_unchanged(&initial, &changed).is_err());
}

#[test]
fn tutorial_object_registers_returned_task_before_any_stop_observation() {
    struct Ownership {
        order: Vec<&'static str>,
        streams: usize,
        tasks: usize,
    }
    let mut ownership = Ownership {
        order: vec![],
        streams: 1,
        tasks: 8,
    };
    assert!(active_task(Task::<Event>::none()).is_none());
    let returned = Task::done(Event::Frame(Instant::now()));
    let result: Result<()> = walking::keep_then_observe(
        true,
        &mut ownership,
        returned,
        |ownership, task| {
            assert!(task.units() > 0);
            ownership.streams += 1;
            ownership.tasks += 1;
            ownership.order.push("register");
            Ok(())
        },
        |ownership| {
            assert_eq!((ownership.streams, ownership.tasks), (2, 9));
            ownership.order.push("stop");
            anyhow::bail!("new instruction")
        },
    );
    assert!(result.is_err());
    assert_eq!(ownership.order, ["register", "stop"]);
    assert!(walking::notification_event(Event::Frame(Instant::now())).is_err());
}

#[test]
fn tutorial_object_stale_epochs_and_actor_caller_lifetimes_fail() {
    let offer = offer();
    let mut stale = cut();
    stale.popup_epoch += 1;
    assert!(current_payload(&offer, &input(), stale).is_err());
    stale = cut();
    stale.generation += 1;
    assert!(current_payload(&offer, &input(), stale).is_err());
    stale = cut();
    stale.host_widget_epoch += 1;
    assert!(current_payload(&offer, &input(), stale).is_err());
    stale = cut();
    stale.publication_widget_epoch += 1;
    assert!(current_payload(&offer, &input(), stale).is_err());
    let initial = native();
    let mut changed = initial.clone();
    changed.actor_lifetime = Some(1002);
    assert!(persistent_native(&initial, &changed).is_err());
    changed = initial.clone();
    changed.owner_lifetime = Some(5002);
    assert!(persistent_native(&initial, &changed).is_err());
}

#[test]
fn tutorial_object_zero_stream_busy_host_never_synthesizes_a_wake() {
    assert!(drained(0, false).is_err());
    assert!(input_gate(0, false, false).is_err());
    assert!(drained(0, true).unwrap());
}

#[test]
fn tutorial_object_pointer_motion_and_press_are_serviced_before_release() {
    struct Pointer {
        pending: usize,
        order: Vec<&'static str>,
    }
    let mut pointer = Pointer {
        pending: 0,
        order: vec![],
    };
    settle_pointer_with(
        &mut pointer,
        |pointer| Ok(pointer.pending != 0),
        |_| panic!("settled initial handoff must not dispatch a frame"),
    )
    .unwrap();
    for gesture in ["motion", "press"] {
        pointer.order.push(gesture);
        pointer.pending = 2;
        settle_pointer_with(
            &mut pointer,
            |pointer| Ok(pointer.pending != 0),
            |pointer| {
                pointer.order.push("reserved_service_frame");
                pointer.pending -= 1;
                pointer.order.push("registered_task_before_observation");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(pointer.pending, 0);
        pointer.order.push("revalidate_pick");
    }
    pointer.order.push("release");
    assert_eq!(
        pointer.order,
        [
            "motion",
            "reserved_service_frame",
            "registered_task_before_observation",
            "reserved_service_frame",
            "registered_task_before_observation",
            "revalidate_pick",
            "press",
            "reserved_service_frame",
            "registered_task_before_observation",
            "reserved_service_frame",
            "registered_task_before_observation",
            "revalidate_pick",
            "release"
        ]
    );
}

#[test]
fn tutorial_object_pointer_service_admission_failure_stops_before_input() {
    let mut frames = 0;
    let result = settle_pointer_with(
        &mut frames,
        |_| Ok(true),
        |_| anyhow::bail!("pre-dispatch frame budget exhausted"),
    );
    assert!(result.is_err());
    assert_eq!(frames, 0);
    let result = settle_pointer_with(
        &mut frames,
        |_| anyhow::bail!("unexpected owner"),
        |frames| {
            *frames += 1;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(frames, 0);
}

#[test]
fn tutorial_object_each_button_needs_its_current_captured_viewport_message() {
    let pick = pick();
    let press = Message::ViewportButton {
        down: true,
        pixel: pick.pixel,
        picked: Some(pick.clicked.0),
    };
    let release = Message::ViewportButton {
        down: false,
        pixel: pick.pixel,
        picked: Some(pick.clicked.0),
    };
    viewport_button_messages(
        std::slice::from_ref(&press),
        &[iced::event::Status::Captured],
        true,
        &pick,
    )
    .unwrap();
    viewport_button_messages(
        std::slice::from_ref(&release),
        &[iced::event::Status::Captured],
        false,
        &pick,
    )
    .unwrap();
    assert!(
        viewport_button_messages(
            std::slice::from_ref(&press),
            &[iced::event::Status::Captured],
            false,
            &pick
        )
        .is_err()
    );
    assert!(
        viewport_button_messages(
            std::slice::from_ref(&release),
            &[iced::event::Status::Ignored],
            false,
            &pick
        )
        .is_err()
    );
    assert!(
        viewport_button_messages(
            &[press, release],
            &[iced::event::Status::Captured; 2],
            false,
            &pick
        )
        .is_err()
    );
}

#[test]
fn tutorial_object_scene_pick_excludes_ground_person_and_multitile_person() {
    let mut snapshot = object_snapshot();
    let hint = JourneyTarget::Object {
        target: 230,
        table_sit: false,
    };
    classified_target(&snapshot, Some(229), Some(&hint)).unwrap();
    assert!(classified_target(&snapshot, None, Some(&hint)).is_err());
    assert!(classified_target(&snapshot, Some(229), Some(&JourneyTarget::Person(230))).is_err());
    snapshot.objects.get_mut(&230).unwrap().person = true;
    assert!(classified_target(&snapshot, Some(229), Some(&hint)).is_err());
    snapshot.objects.get_mut(&229).unwrap().person = true;
    assert!(classified_target(&snapshot, Some(229), Some(&hint)).is_err());
    let viewport = Rectangle::new(Point::ORIGIN, iced::Size::new(400., 320.));
    assert!(viewport_point([1., 80.], [400, 320], viewport, &[]).is_err());
    assert!(viewport_point([80., 80.], [400, 320], viewport, &[viewport]).is_err());
}

#[test]
fn tutorial_object_search_hint_prefers_a_matching_loaded_table_without_proving_an_offer() {
    let first = pick();
    let mut selected = None;
    let point = Point::new(10., 20.);
    assert!(!prefer_pick(&mut selected, (first.clone(), point), None));
    let candidate = Pick {
        clicked: (240, 3000),
        resolved: (241, 3001),
        ..first.clone()
    };
    for hint in [
        JourneyTarget::Person(241),
        JourneyTarget::Object {
            target: 241,
            table_sit: false,
        },
        JourneyTarget::Object {
            target: 242,
            table_sit: true,
        },
    ] {
        assert!(!prefer_pick(
            &mut selected,
            (candidate.clone(), point),
            Some(&hint)
        ));
        assert_eq!(selected.as_ref().unwrap().0, first);
    }
    assert!(prefer_pick(
        &mut selected,
        (candidate.clone(), point),
        Some(&JourneyTarget::Object {
            target: 241,
            table_sit: true,
        })
    ));
    assert_eq!(selected.as_ref().unwrap().0, candidate);
    assert!(offer_ready(None).is_err());
}

#[test]
fn tutorial_object_closed_native_input_stops_before_more_service_work() {
    let mut service_calls = 0;
    let result = offer_ready(None).map(|ready| {
        if !ready {
            service_calls += 1;
        }
    });
    assert!(result.is_err());
    assert_eq!(service_calls, 0);
    let mut pending = input();
    pending.ready = false;
    pending.actions.clear();
    assert!(!offer_ready(Some(&pending)).unwrap());
    assert!(offer_ready(Some(&input())).unwrap());
}

fn scene() -> sims_render::Scene {
    sims_render::Scene {
        width: 80,
        height: 64,
        vertices: vec![sims_render::Vertex {
            position: [0., 1., 2.],
            uv: [0., 1.],
            tint: [1.; 4],
            depth_scale: 1.,
        }],
        draws: vec![sims_render::Draw {
            vertices: 0..1,
            texture: 0,
            object: Some(229),
        }],
        censors: vec![sims_render::censor::Region {
            rect: [1, 2, 3, 4],
            depth: 0.,
            seed: 7,
        }],
        textures: Arc::new(vec![sims_render::Texture {
            width: 1,
            height: 1,
            rgba: Arc::from([1, 2, 3, 255]),
            depth: Arc::from([4]),
        }]),
    }
}

fn candidate_scene() -> sims_render::Scene {
    let mut scene = scene();
    scene.width = 96;
    scene.height = 96;
    scene.vertices[0].position = [0.; 3];
    scene.vertices = vec![scene.vertices[0]; 3];
    for (vertex, position) in
        scene
            .vertices
            .iter_mut()
            .zip([[-0.5, -0.5, 0.], [0.5, -0.5, 0.], [0., 1., 0.]])
    {
        vertex.position = position;
    }
    scene.draws[0].vertices = 0..3;
    scene
}

#[test]
fn tutorial_object_resolved_loaded_table_hint_controls_discovery_and_pick() {
    let scene = candidate_scene();
    let pixel = [48., 48.];
    let picked = scene.pick(pixel);
    assert_eq!(picked, Some(229));
    let mut snapshot = object_snapshot();
    for table_sit in [false, true] {
        let hint = JourneyTarget::Object {
            target: 230,
            table_sit,
        };
        let targets = BTreeMap::from([(229, Ok(hint))]);
        for clicked_count in [None, Some(0), Some(9)] {
            snapshot
                .objects
                .get_mut(&229)
                .unwrap()
                .loaded_interaction_table_count = clicked_count;
            for resolved_count in [None, Some(0), Some(2)] {
                snapshot
                    .objects
                    .get_mut(&230)
                    .unwrap()
                    .loaded_interaction_table_count = resolved_count;
                let expected = resolved_count == Some(2);
                let candidates = scene_candidates(&scene, &snapshot, &targets).unwrap();
                assert_eq!(candidates.len(), usize::from(expected));
                let classified = classified_target(&snapshot, picked, Some(&hint));
                assert_eq!(classified.is_ok(), expected);
                if expected {
                    assert_eq!(classified.unwrap(), ((229, 2000), (230, 2001)));
                    assert_eq!(candidates[0].clicked, 229);
                    assert_eq!(candidates[0].table_sit, table_sit);
                    assert_eq!(candidates[0].pixels, [pixel]);
                }
            }
        }
    }
}

#[test]
fn tutorial_object_loaded_table_hints_exclude_person_and_unknown_identities() {
    let scene = candidate_scene();
    let snapshot = object_snapshot();
    let hint = JourneyTarget::Object {
        target: 230,
        table_sit: true,
    };
    let rejected = |snapshot: &JourneyObject, hint: Option<JourneyTarget>| {
        assert!(classified_target(snapshot, Some(229), hint.as_ref()).is_err());
        let targets = hint
            .map(|hint| BTreeMap::from([(229, Ok(hint))]))
            .unwrap_or_default();
        assert!(
            scene_candidates(&scene, snapshot, &targets)
                .unwrap()
                .is_empty()
        );
    };
    rejected(&snapshot, None);
    rejected(&snapshot, Some(JourneyTarget::Person(230)));
    rejected(
        &snapshot,
        Some(JourneyTarget::Object {
            target: 999,
            table_sit: true,
        }),
    );
    for object in [229, 230] {
        let mut changed = snapshot.clone();
        changed.objects.get_mut(&object).unwrap().person = true;
        rejected(&changed, Some(hint));
        changed = snapshot.clone();
        changed.objects.remove(&object);
        rejected(&changed, Some(hint));
        changed = snapshot.clone();
        changed.objects.get_mut(&object).unwrap().lifetime = None;
        rejected(&changed, Some(hint));
    }
    let targets = BTreeMap::from([(229, Err("synthetic unknown resolved target".into()))]);
    assert!(
        scene_candidates(&scene, &snapshot, &targets)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tutorial_object_nonempty_loaded_table_and_sit_hints_give_no_offer_or_enqueue_credit() {
    let scene = candidate_scene();
    let mut snapshot = object_snapshot();
    let hint = JourneyTarget::Object {
        target: 230,
        table_sit: true,
    };
    let targets = BTreeMap::from([(229, Ok(hint))]);
    let current = native();
    let before = serde_json::to_value(&current).unwrap();
    for count in [1, 2, 99] {
        snapshot
            .objects
            .get_mut(&230)
            .unwrap()
            .loaded_interaction_table_count = Some(count);
        assert_eq!(
            scene_candidates(&scene, &snapshot, &targets).unwrap().len(),
            1
        );
        let (clicked, resolved) = classified_target(&snapshot, Some(229), Some(&hint)).unwrap();
        let candidate = Pick {
            clicked,
            resolved,
            ..pick()
        };
        let mut selected = None;
        assert!(prefer_pick(
            &mut selected,
            (candidate, Point::new(48., 48.)),
            Some(&hint)
        ));
        assert!(snapshot.input.is_none());
        assert!(offer_ready(snapshot.input.as_ref()).is_err());
        let mut empty_input = input();
        empty_input.actions.clear();
        empty_input.action_targets.clear();
        let mut empty_menu = menu();
        empty_menu.popup.clear();
        let metadata: PopupOwner =
            serde_json::from_value(serde_json::to_value(owner()).unwrap()).unwrap();
        assert!(visible_actions(&empty_menu, &empty_input, &metadata, &pick()).is_err());
        assert!(
            fresh_enqueue(&offer(), &current, true, Some(2001))
                .unwrap()
                .is_none()
        );
        assert_eq!(serde_json::to_value(&current).unwrap(), before);
    }
}

#[test]
fn tutorial_object_triangle_centroids_prioritize_metadata_without_offer_credit() {
    let mut scene = scene();
    scene.width = 96;
    scene.height = 96;
    scene.vertices = [[-0.75, 0.5, 0.], [0., 0.5, 0.], [-0.75, -0.25, 0.]]
        .map(|position| sims_render::Vertex {
            position,
            ..scene.vertices[0]
        })
        .to_vec();
    scene.draws = [229, 240, 241, 242, 243]
        .map(|object| sims_render::Draw {
            vertices: 0..3,
            texture: 0,
            object: Some(object),
        })
        .to_vec();
    let targets = BTreeMap::from([
        (
            229,
            Ok(JourneyTarget::Object {
                target: 230,
                table_sit: false,
            }),
        ),
        (
            240,
            Ok(JourneyTarget::Object {
                target: 240,
                table_sit: true,
            }),
        ),
        (241, Ok(JourneyTarget::Person(241))),
        (242, Err("unavailable metadata".into())),
    ]);
    let candidates = scene_candidates(&scene, &object_snapshot(), &targets).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].clicked, 229);
    assert_eq!(candidates[0].pixels, [[24., 36.]]);
    assert_eq!(candidates[1].clicked, 240);
    let priorities: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.table_sit)
        .collect();
    assert_eq!(prioritized_indices(&priorities, 8), [1, 0]);
    assert!(offer_ready(None).is_err());
}

#[test]
fn tutorial_object_triangle_candidates_exclude_invalid_projection_and_bound_draw_ranges() {
    let mut scene = scene();
    scene.width = 96;
    scene.height = 96;
    scene.vertices[0].position = [0., 0., 0.];
    scene.vertices = vec![scene.vertices[0]; 3];
    scene.draws[0].vertices = 0..3;
    let targets = BTreeMap::from([(
        229,
        Ok(JourneyTarget::Object {
            target: 229,
            table_sit: true,
        }),
    )]);
    let candidates = scene_candidates(&scene, &object_snapshot(), &targets).unwrap();
    assert_eq!(candidates[0].pixels, [[48., 48.]]);
    scene.draws.push(scene.draws[0].clone());
    assert_eq!(
        scene_candidates(&scene, &object_snapshot(), &targets).unwrap()[0]
            .pixels
            .len(),
        2
    );
    for coordinate in [f32::NAN, f32::INFINITY, 4.] {
        scene.vertices[0].position[0] = coordinate;
        assert!(
            scene_candidates(&scene, &object_snapshot(), &targets)
                .unwrap()
                .is_empty()
        );
    }
    scene.draws[0].vertices = 0..4;
    assert!(scene_candidates(&scene, &object_snapshot(), &targets).is_err());
    scene.draws[0].vertices = 2..1;
    assert!(scene_candidates(&scene, &object_snapshot(), &targets).is_err());
}

#[test]
fn tutorial_object_scene_republication_requires_complete_exact_values_not_alias() {
    let before = Arc::new(scene());
    let mut after = (*before).clone();
    after.textures = Arc::new((*before.textures).clone());
    let after = Arc::new(after);
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(!Arc::ptr_eq(&before.textures, &after.textures));
    require_same_scene(&before, &after).unwrap();
}

#[test]
fn tutorial_object_scene_float_bits_include_signed_zero_and_nan_payloads() {
    let before = scene();
    let mutations: &[fn(&mut sims_render::Scene)] = &[
        |scene| scene.vertices[0].position[0] = -0.,
        |scene| scene.vertices[0].uv[0] = -0.,
        |scene| scene.vertices[0].tint[0] = f32::from_bits(1.0f32.to_bits() + 1),
        |scene| scene.vertices[0].depth_scale = -0.,
        |scene| scene.censors[0].depth = -0.,
        |scene| scene.vertices[0].position[2] = f32::from_bits(0x7fc00001),
    ];
    for mutate in mutations {
        let mut after = before.clone();
        mutate(&mut after);
        assert!(require_same_scene(&before, &after).is_err());
    }
    let mut before = before;
    before.vertices[0].uv[0] = f32::from_bits(0x7fc00001);
    let mut after = before.clone();
    require_same_scene(&before, &after).unwrap();
    after.vertices[0].uv[0] = f32::from_bits(0x7fc00002);
    assert!(require_same_scene(&before, &after).is_err());
}

#[test]
fn tutorial_object_scene_dimensions_draws_censors_and_all_texture_bytes_are_pinned() {
    let before = scene();
    let mutations: &[fn(&mut sims_render::Scene)] = &[
        |scene| scene.width += 1,
        |scene| scene.height += 1,
        |scene| scene.vertices.push(scene.vertices[0]),
        |scene| scene.draws[0].vertices = 0..0,
        |scene| scene.draws[0].texture += 1,
        |scene| scene.draws[0].object = Some(230),
        |scene| scene.draws.clear(),
        |scene| scene.censors[0].rect[0] += 1,
        |scene| scene.censors[0].seed += 1,
        |scene| scene.censors.clear(),
        |scene| Arc::make_mut(&mut scene.textures)[0].width += 1,
        |scene| Arc::make_mut(&mut scene.textures)[0].height += 1,
        |scene| Arc::make_mut(&mut Arc::make_mut(&mut scene.textures)[0].rgba)[3] -= 1,
        |scene| Arc::make_mut(&mut Arc::make_mut(&mut scene.textures)[0].depth)[0] += 1,
        |scene| Arc::make_mut(&mut scene.textures).clear(),
    ];
    for mutate in mutations {
        let mut after = before.clone();
        mutate(&mut after);
        assert!(require_same_scene(&before, &after).is_err());
    }
}

#[test]
fn tutorial_object_preview_ground_or_wrong_leaf_intent_is_not_an_offer() {
    let metadata: PopupOwner =
        serde_json::from_value(serde_json::to_value(owner()).unwrap()).unwrap();
    visible_actions(&menu(), &input(), &metadata, &pick()).unwrap();
    let mut changed = menu();
    changed.preview = true;
    assert!(visible_actions(&changed, &input(), &metadata, &pick()).is_err());
    changed = menu();
    changed.popup[0].intent = Some(Intent::GroundChoice(0));
    assert!(visible_actions(&changed, &input(), &metadata, &pick()).is_err());
    changed.popup[0].intent = Some(Intent::Queue(230, 0));
    assert!(visible_actions(&changed, &input(), &metadata, &pick()).is_err());
    let mut ground = metadata;
    ground.kind = sims_runtime::original_session::live_menu::Kind::Ground;
    assert!(visible_actions(&menu(), &input(), &ground, &pick()).is_err());
}

#[test]
fn tutorial_object_offered_payload_index_and_all_handles_are_rechecked() {
    let offer = offer();
    current_payload(&offer, &input(), cut()).unwrap();
    let mutations: &[fn(&mut JourneyObjectInput)] = &[
        |input| input.actor.1 += 1,
        |input| input.clicked.1 += 1,
        |input| input.resolved.1 += 1,
        |input| input.actions[0].routine += 1,
        |input| input.actions[0].target += 1,
        |input| *input.action_targets.get_mut(&230).unwrap() += 1,
        |input| input.actions.clear(),
    ];
    for mutation in mutations {
        let mut changed = input();
        mutation(&mut changed);
        assert!(current_payload(&offer, &changed, cut()).is_err());
    }
}

#[test]
fn tutorial_object_real_leaf_messages_require_one_matching_captured_choice() {
    use super::super::super::super::super::super::super::headless_input::ElementText;
    let bounds = Rectangle::new(Point::new(30., 40.), iced::Size::new(100., 20.));
    let mut observation = ElementObservation {
        layouts: vec![bounds],
        texts: vec![ElementText {
            text: "Offered".into(),
            bounds,
            container: Some(bounds),
        }],
        interaction: mouse::Interaction::Pointer,
        messages: vec![],
    };
    leaf(&observation, "Offered").unwrap();
    observation.texts.push(ElementText {
        text: "Offered".into(),
        bounds,
        container: Some(bounds),
    });
    assert!(leaf(&observation, "Offered").is_err());
    observation.texts.pop();
    observation.texts[0].container = None;
    assert!(leaf(&observation, "Offered").is_err());
    let captured = [iced::event::Status::Captured; 2];
    choice_messages(&[Message::ObjectChoice(0)], &captured, 0).unwrap();
    assert!(choice_messages(&[], &captured, 0).is_err());
    assert!(choice_messages(&[Message::ObjectChoice(1)], &captured, 0).is_err());
    assert!(
        choice_messages(
            &[Message::ObjectChoice(0), Message::ObjectChoice(0)],
            &captured,
            0
        )
        .is_err()
    );
    assert!(
        choice_messages(
            &[Message::ObjectChoice(0)],
            &[iced::event::Status::Ignored; 2],
            0
        )
        .is_err()
    );
}

#[test]
fn tutorial_object_popup_closure_without_new_enqueue_is_only_submission() {
    let offer = offer();
    assert!(
        fresh_enqueue(&offer, &native(), true, Some(2001))
            .unwrap()
            .is_none()
    );
    let mut walking_previous = native();
    walking_previous.previous = Some(Action {
        id: 39,
        flags: 44,
        ..action()
    });
    assert!(
        fresh_enqueue(&offer, &walking_previous, true, Some(2001))
            .unwrap()
            .is_none()
    );
    let mut current = native();
    current.serial = 41;
    current.pending.push(Action { id: 41, ..action() });
    assert!(
        fresh_enqueue(&offer, &current, false, Some(2001))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fresh_enqueue(&offer, &current, true, Some(2001))
            .unwrap()
            .unwrap()
            .id,
        41
    );
}

#[test]
fn tutorial_object_wrong_serial_payload_or_reused_target_cannot_prove_enqueue() {
    let offer = offer();
    let mut current = native();
    current.serial = 45;
    current.pending.push(Action { id: 40, ..action() });
    assert!(
        fresh_enqueue(&offer, &current, true, Some(2001))
            .unwrap()
            .is_none()
    );
    current.pending[0].id = 41;
    current.pending[0].routine += 1;
    assert!(fresh_enqueue(&offer, &current, true, Some(2001)).is_err());
    current.pending[0] = Action { id: 41, ..action() };
    assert!(fresh_enqueue(&offer, &current, true, Some(2002)).is_err());
    current.pending.push(Action { id: 42, ..action() });
    assert!(fresh_enqueue(&offer, &current, true, Some(2001)).is_err());
    let mut prior_slot = offer;
    prior_slot.prior_ids.insert(41);
    current.pending.pop();
    assert!(
        fresh_enqueue(&prior_slot, &current, true, Some(2001))
            .unwrap()
            .is_none()
    );
}

#[test]
fn tutorial_object_completion_needs_this_new_serial_absent_live_and_success_bit() {
    let accepted = Action { id: 41, ..action() };
    let mut current = native();
    current.previous = Some(Action {
        id: 39,
        flags: accepted.flags | 32,
        ..action()
    });
    assert!(!completed(&accepted, &current));
    current.previous = Some(accepted.clone());
    assert!(!completed(&accepted, &current));
    current.previous.as_mut().unwrap().flags |= 32;
    assert!(completed(&accepted, &current));
    current.pending.push(accepted.clone());
    assert!(!completed(&accepted, &current));
    current.pending.clear();
    current.previous.as_mut().unwrap().target += 1;
    assert!(!completed(&accepted, &current));
}

#[test]
fn tutorial_object_independent_caps_reserve_before_work_and_count_revalidation() {
    phase_capacity(Usage::default(), Usage::default(), 200, 440, 16, 9.).unwrap();
    assert!(phase_capacity(Usage::default(), Usage::default(), 200, 441, 16, 9.).is_err());
    assert!(phase_capacity(Usage::default(), Usage::default(), 200, 199, 0, 9.).is_err());
    assert!(phase_capacity(Usage::default(), Usage::default(), 200, 200, 0, 10.).is_err());
    assert!(
        Usage {
            frames: 64,
            service_polls: 64,
            ..Default::default()
        }
        .reserve(Usage {
            service_polls: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Usage {
            pixel_scans: 64,
            ..Default::default()
        }
        .reserve(Usage {
            pixel_scans: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Usage {
            tasks: 128,
            ..Default::default()
        }
        .reserve(Usage {
            tasks: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Usage {
            effect_polls: 512,
            ..Default::default()
        }
        .reserve(Usage {
            effect_polls: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Usage {
            commands: 256,
            ..Default::default()
        }
        .reserve(Usage {
            commands: 1,
            ..Default::default()
        })
        .is_err()
    );
}

#[test]
fn tutorial_object_simultaneous_new_instruction_and_changed_owner_gets_no_credit() {
    let initial = native();
    let mut current = initial.clone();
    current.owner = Some(421);
    let mut credit = false;
    let result = credit_after_owners(&initial, &current, || {
        credit = true;
        Ok(())
    });
    assert!(result.is_err());
    assert!(!credit);
    current = initial.clone();
    current.owner_lifetime = Some(5002);
    assert!(
        credit_after_owners(&initial, &current, || {
            credit = true;
            Ok(())
        })
        .is_err()
    );
    assert!(!credit);
}

fn held_snapshot() -> JourneyObject {
    let mut snapshot = object_snapshot();
    snapshot.objects.insert(
        105,
        JourneyObjectIdentity {
            lifetime: Some(1001),
            person: true,
            loaded_interaction_table_count: None,
        },
    );
    snapshot.input = Some(input());
    snapshot
}

fn held_native() -> walking::Native {
    let mut current = native();
    current.input_pending = true;
    current
}

fn held_cut() -> Cut {
    Cut {
        popup_epoch: cut().popup_epoch + 1,
        ..cut()
    }
}

fn capture_view(popup: bool) -> iced::Element<'static, Message> {
    let viewport = iced::widget::mouse_area(
        iced::widget::shader::Shader::new(Viewport(Arc::new(scene())))
            .width(iced::Fill)
            .height(iced::Fill),
    )
    .on_enter(Message::ViewportEnter)
    .on_exit(Message::ViewportLeave);
    if popup {
        let button = iced::widget::button("Leaf")
            .width(60)
            .height(24)
            .on_press(Message::ObjectChoice(0));
        let menu = widget_lifetimes::retain_owner(7, button.into());
        iced::widget::stack![
            viewport,
            iced::widget::container(iced::widget::opaque(menu)).center(iced::Fill)
        ]
        .into()
    } else {
        viewport.into()
    }
}

fn retained_capture_view(popup: bool) -> iced::Element<'static, Message> {
    let viewport = capture_view(false);
    if popup {
        let button = iced::widget::button("Leaf")
            .width(60)
            .height(24)
            .on_press(Message::ObjectChoice(0));
        let menu = widget_lifetimes::retain_owner(7, button.into());
        iced::widget::stack![
            viewport,
            iced::widget::container(iced::widget::opaque(menu)).center(iced::Fill)
        ]
        .into()
    } else {
        iced::widget::stack![viewport].into()
    }
}

#[test]
fn tutorial_object_redraw_actual_mouse_area_bounds_emit_enter_leave() {
    let mut ui = UiInput::new().unwrap();
    let area = |width: u32| -> iced::Element<'static, Message> {
        iced::widget::mouse_area(iced::widget::Space::new().width(width).height(100))
            .on_enter(Message::ViewportEnter)
            .on_exit(Message::ViewportLeave)
            .into()
    };
    let motion = ui.element_batch(
        area(100),
        [200, 200],
        &[iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(50., 50.),
        })],
    );
    assert!(matches!(
        motion.messages.as_slice(),
        [Message::ViewportEnter]
    ));
    let leave = ui.element_observation(area(10), [200, 200]).unwrap();
    assert!(matches!(
        leave.messages.as_slice(),
        [Message::ViewportLeave]
    ));
    redraw_notifications(&leave.messages, true).unwrap();
    assert!(
        ui.element_observation(area(10), [200, 200])
            .unwrap()
            .messages
            .is_empty()
    );
    let enter = ui.element_observation(area(100), [200, 200]).unwrap();
    assert!(matches!(
        enter.messages.as_slice(),
        [Message::ViewportEnter]
    ));
    redraw_notifications(&enter.messages, true).unwrap();
}

#[test]
fn tutorial_object_redraw_rejects_non_notifications_and_other_phases() {
    redraw_notifications(&[], false).unwrap();
    redraw_notifications(&[Message::ViewportEnter, Message::ViewportLeave], true).unwrap();
    assert!(redraw_notifications(&[Message::ViewportLeave], false).is_err());
    for message in [
        Message::ObjectChoice(0),
        Message::ObjectCancel,
        Message::GroundChoice(0),
        Message::ViewportMotion {
            pixel: None,
            pan: [0, 0],
        },
        Message::ViewportButton {
            down: false,
            pixel: pick().pixel,
            picked: Some(229),
        },
        Message::DialogKey(13),
        Message::Toggle,
        Message::Frame(Instant::now()),
    ] {
        assert!(redraw_notifications(&[Message::ViewportEnter, message], true).is_err());
    }
}

#[test]
fn tutorial_object_redraw_binding_requires_original_cut_owner_and_payload() {
    let source = json!({"owner":owner(),"input":input(),"menu":menu_identity(Some(&menu()))});
    redraw_binding(held_cut(), held_cut(), &source, &source.clone()).unwrap();
    for field in ["owner", "input", "menu"] {
        let mut changed = source.clone();
        changed[field] = Value::Null;
        assert!(redraw_binding(held_cut(), held_cut(), &source, &changed).is_err());
    }
    for change in [
        Cut {
            generation: held_cut().generation + 1,
            ..held_cut()
        },
        Cut {
            host_widget_epoch: held_cut().host_widget_epoch + 1,
            ..held_cut()
        },
        Cut {
            publication_widget_epoch: held_cut().publication_widget_epoch + 1,
            ..held_cut()
        },
        Cut {
            popup_epoch: held_cut().popup_epoch + 1,
            ..held_cut()
        },
    ] {
        assert!(redraw_binding(held_cut(), change, &source, &source).is_err());
    }
    let replacement = json!({"owner":"new publication"});
    redraw_binding(held_cut(), held_cut(), &replacement, &replacement).unwrap();
    assert!(redraw_binding(held_cut(), held_cut(), &source, &replacement).is_err());
}

#[test]
fn tutorial_object_redraw_registers_each_returned_task_before_stop() {
    let messages = [Message::ViewportEnter, Message::ViewportLeave];
    redraw_notifications(&messages, true).unwrap();
    let mut order = vec![];
    for (index, _) in messages.into_iter().enumerate() {
        let result = walking::keep_then_observe(
            true,
            &mut order,
            Task::done(Event::Completion),
            |order, task| {
                assert!(task.units() > 0);
                order.push((index, "registered"));
                Ok(())
            },
            |order| {
                assert_eq!(order.last(), Some(&(index, "registered")));
                order.push((index, "observed"));
                if index == 1 {
                    anyhow::bail!("new lesson; no more messages")
                }
                Ok(())
            },
        );
        assert_eq!(result.is_err(), index == 1);
    }
    assert_eq!(
        order,
        [
            (0, "registered"),
            (0, "observed"),
            (1, "registered"),
            (1, "observed")
        ]
    );
    assert!(walking::notification_event(Event::Completion).is_ok());
    assert!(walking::notification_event(Event::Frame(Instant::now())).is_err());
    assert!(drained(0, false).is_err());
}

#[test]
fn tutorial_object_redraw_stabilization_uses_existing_poll_and_wall_budgets() {
    let one_redraw = Usage {
        effect_polls: 1,
        ..Default::default()
    };
    let last_slot = Usage {
        effect_polls: 511,
        ..Default::default()
    };
    phase_capacity(last_slot, one_redraw, 200, 200, 0, 9.9).unwrap();
    assert!(
        phase_capacity(
            Usage {
                effect_polls: 512,
                ..Default::default()
            },
            one_redraw,
            200,
            200,
            0,
            0.
        )
        .is_err()
    );
    assert!(phase_capacity(Usage::default(), one_redraw, 200, 200, 0, WALL_SECONDS).is_err());
    assert!(phase_capacity(Usage::default(), one_redraw, 200, 457, 0, 0.).is_err());
}

#[test]
fn tutorial_object_popup_on_press_owns_real_release_without_leaf_credit() {
    let mut ui = UiInput::new().unwrap();
    ui.element_batch(
        capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(40., 32.),
        })],
    );
    let press = ui.element_batch(
        capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))],
    );
    assert!(
        press
            .messages
            .iter()
            .any(|message| matches!(message, Message::ViewportButton { down: true, .. }))
    );
    assert_eq!(press.statuses, [iced::event::Status::Captured]);
    let redraw = ui
        .element_observation(capture_view(true), [80, 64])
        .unwrap();
    redraw_notifications(&redraw.messages, true).unwrap();
    assert_eq!(redraw.interaction, mouse::Interaction::Pointer);
    leaf(&redraw, "Leaf").unwrap();
    let release = ui.element_batch(
        capture_view(true),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))],
    );
    assert!(release.messages.is_empty());
    assert_eq!(release.statuses, [iced::event::Status::Ignored]);
    let proof = Proof {
        viewport_left_press_sent: true,
        ..Default::default()
    };
    diagnostic_press_proof(&proof).unwrap();
    let activation = ui.element_batch(
        capture_view(true),
        [80, 64],
        &[
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
    );
    choice_messages(&activation.messages, &activation.statuses, 0).unwrap();
}

#[test]
fn tutorial_object_popup_outside_and_retained_cursor_use_only_actual_viewport_release() {
    let mut ui = UiInput::new().unwrap();
    ui.element_batch(
        capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(5., 5.),
        })],
    );
    let press = ui.element_batch(
        capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))],
    );
    assert!(
        press
            .messages
            .iter()
            .any(|message| matches!(message, Message::ViewportButton { down: true, .. }))
    );
    let redraw = ui
        .element_observation(capture_view(true), [80, 64])
        .unwrap();
    redraw_notifications(&redraw.messages, true).unwrap();
    let release = ui.element_batch(
        capture_view(true),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))],
    );
    assert!(matches!(
        release.messages.as_slice(),
        [Message::ViewportButton { down: false, .. }]
    ));
    assert_eq!(release.statuses, [iced::event::Status::Captured]);
    assert!(
        !release
            .messages
            .iter()
            .any(|message| matches!(message, Message::ObjectChoice(_)))
    );
    let mut retained = UiInput::new().unwrap();
    retained.element_batch(
        retained_capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(40., 32.),
        })],
    );
    let held = retained.element_batch(
        retained_capture_view(false),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))],
    );
    assert!(
        held.messages
            .iter()
            .any(|message| matches!(message, Message::ViewportButton { down: true, .. }))
    );
    let redraw = retained
        .element_observation(retained_capture_view(true), [80, 64])
        .unwrap();
    redraw_notifications(&redraw.messages, true).unwrap();
    assert_eq!(redraw.interaction, mouse::Interaction::Pointer);
    let release = retained.element_batch(
        retained_capture_view(true),
        [80, 64],
        &[iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))],
    );
    assert!(matches!(
        release.messages.as_slice(),
        [Message::ViewportButton { down: false, .. }]
    ));
    assert_eq!(release.statuses, [iced::event::Status::Captured]);
}

#[test]
fn tutorial_object_popup_press_requires_first_exact_owner_and_all_lifetimes() {
    let check = |current_cut,
                 current: &walking::Native,
                 snapshot: &JourneyObject,
                 owner: &sims_runtime::original_session::live_menu::State| {
        popup_press_identity(
            cut(),
            current_cut,
            &native(),
            current,
            snapshot,
            owner,
            &menu(),
            &pick(),
        )
    };
    check(held_cut(), &held_native(), &held_snapshot(), &owner()).unwrap();
    assert!(check(cut(), &held_native(), &held_snapshot(), &owner()).is_err());
    assert!(
        check(
            Cut {
                popup_epoch: held_cut().popup_epoch + 1,
                ..held_cut()
            },
            &held_native(),
            &held_snapshot(),
            &owner()
        )
        .is_err()
    );
    let mutations: &[fn(&mut walking::Native)] = &[
        |current| current.actor_lifetime = Some(1002),
        |current: &mut walking::Native| current.owner_lifetime = Some(5002),
        |current: &mut walking::Native| current.tick += 1,
        |current: &mut walking::Native| current.serial += 1,
        |current: &mut walking::Native| current.input_pending = false,
    ];
    for mutation in mutations {
        let mut current = held_native();
        mutation(&mut current);
        assert!(check(held_cut(), &current, &held_snapshot(), &owner()).is_err());
    }
    for target in [105, 229, 230] {
        let mut snapshot = held_snapshot();
        snapshot.objects.get_mut(&target).unwrap().lifetime = Some(9999);
        assert!(check(held_cut(), &held_native(), &snapshot, &owner()).is_err());
    }
    let mut changed_owner = serde_json::to_value(owner()).unwrap();
    changed_owner["kind"] = json!("Ground");
    assert!(
        check(
            held_cut(),
            &held_native(),
            &held_snapshot(),
            &serde_json::from_value(changed_owner).unwrap()
        )
        .is_err()
    );
}

#[test]
fn tutorial_object_popup_press_diagnostic_never_credits_offer_release_or_enqueue() {
    let proof = Proof {
        viewport_left_press_sent: true,
        ..Default::default()
    };
    diagnostic_press_proof(&proof).unwrap();
    let mutations: &[fn(&mut Proof)] = &[
        |proof| proof.viewport_left_release_sent = true,
        |proof: &mut Proof| proof.offer_observed = true,
        |proof: &mut Proof| proof.command_sent = true,
        |proof: &mut Proof| proof.enqueue_accepted = true,
        |proof: &mut Proof| proof.active_seen = true,
        |proof: &mut Proof| proof.native_completion_observed = true,
        |proof: &mut Proof| proof.hover_qualified = true,
    ];
    for mutation in mutations {
        let mut changed = Proof {
            viewport_left_press_sent: true,
            ..Default::default()
        };
        mutation(&mut changed);
        assert!(diagnostic_press_proof(&changed).is_err());
    }
    assert!(diagnostic_press_proof(&Proof::default()).is_err());
    assert!(
        fresh_enqueue(&offer(), &native(), true, Some(2001))
            .unwrap()
            .is_none()
    );
}

#[test]
fn tutorial_object_redraw_new_lesson_with_stale_owner_stops_after_registration() {
    struct Observation {
        order: Vec<&'static str>,
        next: Option<Record>,
        boundary_valid: bool,
        proof: Proof,
        streams: usize,
    }
    for completion_delivered in [false, true] {
        for mutation in ["epoch", "owner", "epoch_and_owner", "instruction_only"] {
            let mut state = Observation {
                order: vec![],
                next: None,
                boundary_valid: false,
                proof: Proof {
                    viewport_left_press_sent: true,
                    ..Default::default()
                },
                streams: 0,
            };
            let current = held_native();
            let mut next = record();
            next.id += 1;
            let source = json!({"owner":owner(),"instruction":record()});
            let mut published = source.clone();
            published["instruction"] = serde_json::to_value(&next).unwrap();
            if matches!(mutation, "owner" | "epoch_and_owner") {
                published["owner"]["target"] = json!([231, 2002]);
            }
            let current_cut = if matches!(mutation, "epoch" | "epoch_and_owner") {
                Cut {
                    popup_epoch: held_cut().popup_epoch + 1,
                    ..held_cut()
                }
            } else {
                held_cut()
            };
            if completion_delivered {
                walking::keep_then_observe(
                    true,
                    &mut state,
                    Task::done(Event::Completion),
                    |state, task| {
                        assert!(task.units() > 0);
                        state.streams += 1;
                        state.order.push("notification_registered");
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap();
                assert!(state.streams > 0);
                assert!(matches!(
                    walking::notification_event(Event::Completion).unwrap(),
                    Event::Completion
                ));
                state.order.push("completion_delivered");
            }
            let result: Result<()> = walking::keep_then_observe(
                true,
                &mut state,
                Task::done(Event::Completion),
                |state, task| {
                    assert!(task.units() > 0);
                    state.streams += 1;
                    state.order.push("returned_task_registered");
                    Ok(())
                },
                |state| {
                    bound_redraw_observation(
                        state,
                        |state| {
                            assert!(state.streams > 0);
                            state.next = Some(next.clone());
                            state.order.push("new_lesson_retained");
                        },
                        |state| {
                            state.order.push("bound_redraw_checked");
                            redraw_binding(held_cut(), current_cut, &source, &published)
                        },
                        |state| {
                            credit_after_owners(&held_native(), &current, || {
                                state.boundary_valid = true;
                                state.proof.offer_observed = true;
                                Ok(())
                            })?;
                            anyhow::bail!(BOUNDARY)
                        },
                    )
                },
            );
            let error = result.unwrap_err().to_string();
            assert_ne!(error, BOUNDARY);
            assert_eq!(state.next.as_ref().unwrap().id, next.id);
            assert!(!state.boundary_valid);
            let expected = if completion_delivered {
                vec![
                    "notification_registered",
                    "completion_delivered",
                    "returned_task_registered",
                    "new_lesson_retained",
                    "bound_redraw_checked",
                ]
            } else {
                vec![
                    "returned_task_registered",
                    "new_lesson_retained",
                    "bound_redraw_checked",
                ]
            };
            assert_eq!(state.order, expected);
            notification_unchanged(&held_native(), &current).unwrap();
            diagnostic_press_proof(&state.proof).unwrap();
        }
    }
}

fn popup_release_view(popup: bool) -> iced::Element<'static, Message> {
    let viewport = iced::widget::mouse_area(
        iced::widget::shader::Shader::new(Viewport(Arc::new(candidate_scene())))
            .width(iced::Fill)
            .height(iced::Fill),
    )
    .on_enter(Message::ViewportEnter)
    .on_exit(Message::ViewportLeave);
    if popup {
        let button = iced::widget::button("Leaf")
            .width(60)
            .height(24)
            .on_press(Message::ObjectChoice(0));
        let menu = widget_lifetimes::retain_owner(7, button.into());
        iced::widget::stack![
            viewport,
            iced::widget::container(iced::widget::opaque(menu)).center(iced::Fill)
        ]
        .into()
    } else {
        iced::widget::stack![viewport].into()
    }
}

fn release_pick() -> Pick {
    Pick {
        pixel: [48., 26.],
        ..pick()
    }
}

#[test]
fn tutorial_object_popup_release_absent_zero_preserves_diagnostic_effect_trace() {
    for value in [None, Some("0")] {
        let mut reads = Vec::new();
        let enabled = object_popup_release_enabled_with(|name| {
            reads.push(name.to_owned());
            value
                .map(str::to_owned)
                .ok_or(std::env::VarError::NotPresent)
        })
        .unwrap();
        assert!(!enabled);
        assert_eq!(reads, ["SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_POPUP_RELEASE"]);
        let mut trace = (vec!["prior"], 7usize, 3usize, 2usize);
        let result = popup_press_path(
            enabled,
            &mut trace,
            |_| panic!("disabled release"),
            |trace| {
                trace.0.push("diagnostic_redraw");
                trace.1 += 1;
                trace.0.push("diagnostic_stop");
                anyhow::bail!(
                    "object diagnostic popup-on-press boundary; no release, leaf, or acceptance credit"
                )
            },
        );
        assert_eq!(
            trace,
            (
                vec!["prior", "diagnostic_redraw", "diagnostic_stop"],
                8,
                3,
                2
            )
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            "object diagnostic popup-on-press boundary; no release, leaf, or acceptance credit"
        );
    }
}

#[test]
fn tutorial_object_popup_release_flag_values_and_fresh_dependency_matrix() {
    let dependencies = [
        "SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION",
        "SIMS_HOST_JOURNEY_TUTORIAL_CAMERA",
        "SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE",
        "SIMS_HOST_JOURNEY_TUTORIAL_WALKING",
    ];
    for mask in 0..16 {
        for source in [
            "UserData/Houses/House07.iff",
            "UserData/Houses/House05.iff",
            "UserData2/Houses/House09.iff",
        ] {
            let result = object_popup_release_enabled_with(|name| {
                if name == "SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_POPUP_RELEASE" {
                    Ok("1".into())
                } else if name == "SIMS_HOST_JOURNEY_SOURCE" {
                    Ok(source.into())
                } else {
                    let index = dependencies
                        .iter()
                        .position(|dependency| *dependency == name)
                        .unwrap();
                    Ok(if mask & (1 << index) != 0 { "1" } else { "0" }.into())
                }
            });
            assert_eq!(
                result.is_ok(),
                mask == 15 && source == "UserData/Houses/House07.iff"
            );
        }
    }
    for value in ["", "true", "yes", "2", "01", " 1", "1 "] {
        assert!(object_popup_release_enabled_with(|_| Ok(value.into())).is_err());
    }
    assert!(
        object_popup_release_enabled_with(|_| {
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "synthetic",
            )))
        })
        .is_err()
    );
    for invalid in dependencies {
        assert!(
            object_popup_release_enabled_with(|name| {
                Ok(if name == invalid { "true" } else { "1" }.into())
            })
            .is_err()
        );
    }
    assert!(
        object_popup_release_enabled_with(|name| {
            if name == "SIMS_HOST_JOURNEY_SOURCE" {
                Err(std::env::VarError::NotPresent)
            } else {
                Ok("1".into())
            }
        })
        .is_err()
    );
}

#[test]
fn tutorial_object_popup_release_requires_original_unobscured_available_cursor() {
    let mut ui = UiInput::new().unwrap();
    let position = Point::new(48., 26.);
    ui.element_batch(
        popup_release_view(false),
        [96, 96],
        &[iced::Event::Mouse(mouse::Event::CursorMoved { position })],
    );
    let scene = candidate_scene();
    let viewport = Rectangle::new(Point::ORIGIN, iced::Size::new(96., 96.));
    let menu_bounds = Rectangle::new(Point::new(18., 36.), iced::Size::new(60., 24.));
    original_release_point(
        &ui,
        &release_pick(),
        position,
        &scene,
        [96, 96],
        viewport,
        &[menu_bounds],
        &[viewport, menu_bounds],
    )
    .unwrap();
    let covered = Rectangle::new(Point::new(20., 20.), iced::Size::new(40., 40.));
    assert!(
        original_release_point(
            &ui,
            &release_pick(),
            position,
            &scene,
            [96, 96],
            viewport,
            &[covered],
            &[viewport]
        )
        .is_err()
    );
    assert!(
        original_release_point(
            &ui,
            &release_pick(),
            position,
            &scene,
            [96, 96],
            viewport,
            &[],
            &[viewport, covered]
        )
        .is_err()
    );
    for changed in [
        Rectangle {
            x: 100.,
            ..viewport
        },
        Rectangle {
            width: 95.,
            ..viewport
        },
    ] {
        assert!(
            original_release_point(
                &ui,
                &release_pick(),
                position,
                &scene,
                [96, 96],
                changed,
                &[],
                &[changed]
            )
            .is_err()
        );
    }
    let unavailable = UiInput::new().unwrap();
    assert!(
        original_release_point(
            &unavailable,
            &release_pick(),
            position,
            &scene,
            [96, 96],
            viewport,
            &[],
            &[viewport]
        )
        .is_err()
    );
    assert!(
        original_release_point(
            &ui,
            &release_pick(),
            Point::new(49., 26.),
            &scene,
            [96, 96],
            viewport,
            &[],
            &[viewport]
        )
        .is_err()
    );
    let mut changed_scene = candidate_scene();
    changed_scene.draws[0].object = Some(230);
    assert!(
        original_release_point(
            &ui,
            &release_pick(),
            position,
            &changed_scene,
            [96, 96],
            viewport,
            &[],
            &[viewport]
        )
        .is_err()
    );
    assert!(
        matches!(ui.cursor_event(), iced::Event::Mouse(mouse::Event::CursorMoved {
        position: actual
    }) if actual == position)
    );
}

#[test]
fn tutorial_object_popup_release_changed_epoch_owner_or_handles_stop_before_attempt() {
    for mutation in 0..8 {
        let mut current_cut = held_cut();
        let mut snapshot = held_snapshot();
        let mut native_owner = serde_json::to_value(owner()).unwrap();
        match mutation {
            0 => current_cut.generation += 1,
            1 => current_cut.host_widget_epoch += 1,
            2 => current_cut.publication_widget_epoch += 1,
            3 => current_cut.popup_epoch += 1,
            4 => native_owner["kind"] = json!("Ground"),
            5 => snapshot.objects.get_mut(&105).unwrap().lifetime = Some(9999),
            6 => snapshot.objects.get_mut(&229).unwrap().lifetime = Some(9999),
            7 => snapshot.objects.get_mut(&230).unwrap().lifetime = Some(9999),
            _ => unreachable!(),
        }
        let mut attempted = false;
        let result: Result<()> = (|| {
            popup_press_identity(
                cut(),
                current_cut,
                &native(),
                &held_native(),
                &snapshot,
                &serde_json::from_value(native_owner).unwrap(),
                &menu(),
                &pick(),
            )?;
            release_attempt(
                &Proof {
                    viewport_left_press_sent: true,
                    ..Default::default()
                },
                &mut attempted,
            )
        })();
        assert!(result.is_err());
        assert!(!attempted);
    }

    let (viewer, initial_load) = Viewer::new(Args::parse_from(["sims-room", "--paused"]));
    drop(initial_load);
    let mut publication = Published::capture(&viewer);
    let host = Host::from_viewer_for_test(viewer);
    let scene = Arc::new(candidate_scene());
    let camera = sims_sim::pose::portrait::view::Input {
        viewport: [0, 0, 96, 96],
        center: [0, 0],
        zoom: 1,
        rotation: 0,
    };
    let base = JourneyGround {
        scene: scene.clone(),
        display_size: [96, 96],
        widget_epoch: 5,
        tick: 200,
        actor: 105,
        actor_lifetime: 1001,
        floor: 1,
        camera,
        lot: Arc::new(sims_sim::lot_geometry::Lot {
            size: 2,
            active_size: None,
            heights: vec![[0; 4]; 4],
            pool: vec![0; 4],
            water: vec![255; 4],
            floors: vec![],
        }),
        lot_size: 2,
        input: None,
        object_input: true,
        serial: 40,
        queue: vec![],
        queue_targets: BTreeMap::new(),
    };
    let mut snapshot = held_snapshot();
    snapshot.objects.insert(
        420,
        JourneyObjectIdentity {
            lifetime: Some(5001),
            person: false,
            loaded_interaction_table_count: None,
        },
    );
    publication.scene = Some(scene);
    publication.widget_epoch = 5;
    publication.session_present = true;
    publication.pending = false;
    publication.native_clock = true;
    publication.native_transport = true;
    publication.playing = true;
    publication.native_selection = Some(Ok(Some(105)));
    publication.resident = 105;
    publication.residents = vec![Resident {
        object: 105,
        neighbor: 48,
        name: "Bob".into(),
        enabled: true,
        highlighted: false,
    }];
    publication.tick_label = Some("Tick 200 synthetic".into());
    publication.dialog = Some(record());
    publication.native_popup = Some(owner());
    publication.native_popup_epoch = 7;
    publication.menu = Some(menu());
    publication.journey_walking = Some(Ok(held_native()));
    publication.journey_object = Some(Ok(snapshot));
    publication.journey_ground = Some(Ok(base.clone()));
    publication.journey_camera = Some(Ok(camera));
    publication.journey_targets.insert(
        229,
        Ok(JourneyTarget::Object {
            target: 230,
            table_sit: false,
        }),
    );
    publication.cluster = Some(Arc::new(hud::cluster::Published {
        values: hud::cluster::Values {
            pending: false,
            ready: true,
            mode_raw: Some(2),
            panel_state_raw: None,
            can_show_live: true,
            can_show_buy: false,
            floor: 1,
            native_clock: true,
            pause_raw: None,
            people_visible: true,
            clock: "synthetic".into(),
            funds: "synthetic".into(),
            buttons: Default::default(),
        },
        art: None,
    }));
    let (sender, receiver) = mpsc::channel();
    let mut journey = Journey {
        host,
        executor: ThreadPool::builder().pool_size(1).create().unwrap(),
        sender,
        receiver,
        streams: 0,
        tasks: 0,
        idle: true,
        wall: Instant::now(),
        wall_seconds: 60.,
        epoch: Instant::now(),
        millis: 0,
        commands: 0,
        frames: 0,
        service_polls: 0,
        effect_polls: 0,
        dialogs: 0,
        events: vec![],
        timing: None,
        timing_parent: None,
    };
    let mut ui = UiInput::new().unwrap();
    for mutation in 0..9 {
        let mut current = publication.clone();
        let mut initial_cut = Cut {
            generation: journey.host.current_generation_for_test(),
            host_widget_epoch: journey.host.widget_epoch_for_test(),
            publication_widget_epoch: 5,
            popup_epoch: 6,
        };
        match mutation {
            0 => initial_cut.generation += 1,
            1 => initial_cut.host_widget_epoch += 1,
            2 => current.widget_epoch += 1,
            3 => current.native_popup_epoch += 1,
            4 => {
                let mut native_owner = serde_json::to_value(owner()).unwrap();
                native_owner["target"][1] = json!(9999);
                current.native_popup = Some(serde_json::from_value(native_owner).unwrap());
            }
            5..=7 => {
                let object = [105, 229, 230][mutation - 5];
                current
                    .journey_object
                    .as_mut()
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .objects
                    .get_mut(&object)
                    .unwrap()
                    .lifetime = Some(9999);
            }
            8 => {}
            _ => unreachable!(),
        }
        journey.host.install_synthetic_publication_for_test(current);
        let mut driver = Driver {
            start: Usage::current(&journey),
            entry: Entry {
                instruction: record(),
                cut: initial_cut,
                initial: native(),
                base: base.clone(),
                hud_height: publication.hud_height,
                retained_streams: 0,
                retained_tasks: 0,
            },
            journey: &mut journey,
            wall: Instant::now(),
            probes: 0,
            opened: true,
            pick: Some(release_pick()),
            offer: None,
            accepted: None,
            proof: Proof {
                viewport_left_press_sent: true,
                ..Default::default()
            },
            next: None,
            boundary_valid: false,
            popup_press_diagnostic: None,
            redraw_source: None,
            popup_release_enabled: true,
            popup_release_attempted: false,
            diagnostic: None,
        };
        if mutation == 8 {
            let source = driver.journey.host.publication_for_test().clone();
            driver.require_popup_press(&source).unwrap();
            assert_eq!(driver.probes, 1);
        } else {
            let result = driver.release_popup_press(&mut ui, Point::new(48., 26.));
            assert!(result.is_err(), "mutation {mutation} reached release");
            assert_eq!(
                driver.probes, 0,
                "mutation {mutation} performed a release batch/pick"
            );
        }
        assert!(!driver.popup_release_attempted);
        assert!(!driver.proof.viewport_left_release_sent);
        assert!(!driver.proof.offer_observed);
        assert!(!driver.proof.command_sent);
        assert!(!driver.proof.enqueue_accepted);
        assert_eq!(driver.journey.effect_polls, 0);
        assert_eq!(driver.journey.commands, 0);
        assert_eq!(driver.journey.frames, 0);
        assert_eq!(driver.journey.tasks, 0);
        assert_eq!(driver.journey.streams, 0);
        assert!(driver.journey.events.is_empty());
        assert!(matches!(
            ui.cursor_event(),
            iced::Event::Mouse(mouse::Event::CursorLeft)
        ));
    }
}

#[test]
fn tutorial_object_popup_release_same_cache_actual_shader_capture_then_real_leaf() {
    let mut ui = UiInput::new().unwrap();
    let position = Point::new(48., 26.);
    let pick = release_pick();
    ui.element_batch(
        popup_release_view(false),
        [96, 96],
        &[iced::Event::Mouse(mouse::Event::CursorMoved { position })],
    );
    let press = ui.element_batch(
        popup_release_view(false),
        [96, 96],
        &[iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))],
    );
    viewport_button_messages(&press.messages, &press.statuses, true, &pick).unwrap();
    let first = ui
        .element_observation(popup_release_view(true), [96, 96])
        .unwrap();
    redraw_notifications(&first.messages, true).unwrap();
    let stable = ui
        .element_observation(popup_release_view(true), [96, 96])
        .unwrap();
    assert!(stable.messages.is_empty());
    let (bounds, center) = leaf(&stable, "Leaf").unwrap();
    original_release_point(
        &ui,
        &pick,
        position,
        &candidate_scene(),
        [96, 96],
        Rectangle::new(Point::ORIGIN, iced::Size::new(96., 96.)),
        &[bounds],
        &stable.layouts,
    )
    .unwrap();
    let mut attempted = false;
    let proof = Proof {
        viewport_left_press_sent: true,
        ..Default::default()
    };
    release_attempt(&proof, &mut attempted).unwrap();
    let batch = ui.element_batch(
        popup_release_view(true),
        [96, 96],
        &[iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))],
    );
    let mut order = Vec::new();
    captured_release_with(
        &mut order,
        batch.messages,
        batch.statuses,
        &pick,
        |order, messages, statuses| {
            viewport_button_messages(messages, statuses, false, &pick).unwrap();
            order.push("actual_output_logged");
        },
        |order| {
            order.push("source_rechecked");
            Ok(())
        },
        |order, message| {
            assert!(
                matches!(message, Message::ViewportButton { down: false, pixel, picked: Some(229) }
                if pixel == pick.pixel)
            );
            order.push("genuine_release_forwarded");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        order,
        [
            "actual_output_logged",
            "source_rechecked",
            "genuine_release_forwarded"
        ]
    );
    assert!(!proof.offer_observed && !proof.command_sent && !proof.enqueue_accepted);
    ui.element_batch(
        popup_release_view(true),
        [96, 96],
        &[iced::Event::Mouse(mouse::Event::CursorMoved {
            position: center,
        })],
    );
    let hover = ui
        .element_observation(popup_release_view(true), [96, 96])
        .unwrap();
    assert_eq!(hover.interaction, mouse::Interaction::Pointer);
    assert_eq!(leaf(&hover, "Leaf").unwrap(), (bounds, center));
    let choice = ui.element_batch(
        popup_release_view(true),
        [96, 96],
        &[
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
    );
    choice_messages(&choice.messages, &choice.statuses, 0).unwrap();
}

#[test]
fn tutorial_object_popup_release_logs_ignored_wrong_mixed_output_without_dispatch() {
    let pick = release_pick();
    let release = Message::ViewportButton {
        down: false,
        pixel: pick.pixel,
        picked: Some(pick.clicked.0),
    };
    let cases = vec![
        (vec![], vec![iced::event::Status::Ignored]),
        (vec![release.clone()], vec![iced::event::Status::Ignored]),
        (
            vec![Message::ViewportButton {
                down: true,
                pixel: pick.pixel,
                picked: Some(pick.clicked.0),
            }],
            vec![iced::event::Status::Captured],
        ),
        (
            vec![Message::ViewportButton {
                down: false,
                pixel: [49., 26.],
                picked: Some(pick.clicked.0),
            }],
            vec![iced::event::Status::Captured],
        ),
        (
            vec![Message::ViewportButton {
                down: false,
                pixel: pick.pixel,
                picked: None,
            }],
            vec![iced::event::Status::Captured],
        ),
        (
            vec![Message::ViewportButton {
                down: false,
                pixel: pick.pixel,
                picked: Some(230),
            }],
            vec![iced::event::Status::Captured],
        ),
        (
            vec![Message::ObjectChoice(0)],
            vec![iced::event::Status::Captured],
        ),
        (
            vec![release.clone(), Message::ViewportEnter],
            vec![iced::event::Status::Captured],
        ),
        (vec![release], vec![iced::event::Status::Captured; 2]),
    ];
    for (messages, statuses) in cases {
        let expected = format!("{messages:?}/{statuses:?}");
        let mut order = Vec::new();
        let result = captured_release_with(
            &mut order,
            messages,
            statuses,
            &pick,
            |order, messages, statuses| order.push(format!("{messages:?}/{statuses:?}")),
            |_| panic!("invalid release reached source admission"),
            |_, _| panic!("invalid release dispatched"),
        );
        assert!(result.is_err());
        assert_eq!(order, [expected]);
    }
}

#[test]
fn tutorial_object_popup_release_rechecks_publication_after_capture_before_forward() {
    for changed_cut in [
        held_cut(),
        Cut {
            popup_epoch: held_cut().popup_epoch + 1,
            ..held_cut()
        },
    ] {
        let source = json!({"owner":owner(), "instruction":record(), "geometry":[96,96]});
        let mut changed = source.clone();
        changed["owner"]["actor"][1] = json!(9999);
        let pick = release_pick();
        let mut order = Vec::new();
        let result = captured_release_with(
            &mut order,
            vec![Message::ViewportButton {
                down: false,
                pixel: pick.pixel,
                picked: Some(pick.clicked.0),
            }],
            vec![iced::event::Status::Captured],
            &pick,
            |order, _, _| order.push("actual_output_logged"),
            |order| {
                order.push("source_rechecked");
                redraw_binding(held_cut(), changed_cut, &source, &changed)
            },
            |_, _| panic!("changed publication forwarded"),
        );
        assert!(result.is_err());
        assert_eq!(order, ["actual_output_logged", "source_rechecked"]);
    }
}

#[test]
fn tutorial_object_popup_release_notification_registers_before_bound_instruction_credit() {
    let mut state = (Vec::new(), None, false);
    let mut next = record();
    next.id += 1;
    let result: Result<()> = walking::keep_then_observe(
        true,
        &mut state,
        Task::done(Event::Completion),
        |state, task| {
            assert!(task.units() > 0);
            state.0.push("task_registered");
            Ok(())
        },
        |state| {
            bound_redraw_observation(
                state,
                |state| {
                    state.1 = Some(next.clone());
                    state.0.push("instruction_retained");
                },
                |state| {
                    state.0.push("source_checked");
                    redraw_binding(
                        held_cut(),
                        held_cut(),
                        &json!({"instruction":record()}),
                        &json!({"instruction":next}),
                    )
                },
                |state| {
                    state.2 = true;
                    anyhow::bail!(BOUNDARY)
                },
            )
        },
    );
    assert!(result.is_err());
    assert_ne!(result.unwrap_err().to_string(), BOUNDARY);
    assert_eq!(
        state.0,
        ["task_registered", "instruction_retained", "source_checked"]
    );
    assert_eq!(state.1.unwrap().id, next.id);
    assert!(!state.2);
}

#[test]
fn tutorial_object_popup_release_attempt_is_once_and_cannot_reuse_leaf_credit() {
    let mut attempted = false;
    let proof = Proof {
        viewport_left_press_sent: true,
        ..Default::default()
    };
    release_attempt(&proof, &mut attempted).unwrap();
    assert!(attempted);
    assert!(release_attempt(&proof, &mut attempted).is_err());
    for mutation in 0..8 {
        let mut proof = Proof {
            viewport_left_press_sent: true,
            ..Default::default()
        };
        match mutation {
            0 => proof.viewport_left_press_sent = false,
            1 => proof.viewport_left_release_sent = true,
            2 => proof.offer_observed = true,
            3 => proof.command_sent = true,
            4 => proof.enqueue_accepted = true,
            5 => proof.active_seen = true,
            6 => proof.native_completion_observed = true,
            7 => proof.hover_qualified = true,
            _ => unreachable!(),
        }
        let mut attempted = false;
        assert!(release_attempt(&proof, &mut attempted).is_err());
        assert!(!attempted);
    }
    assert!(
        choice_messages(
            &[Message::ObjectChoice(0), Message::ObjectChoice(0)],
            &[iced::event::Status::Captured; 2],
            0
        )
        .is_err()
    );
}

#[test]
fn tutorial_object_popup_release_closure_is_submission_not_enqueue_or_completion() {
    let retained = offer();
    let mut current = native();
    assert!(
        fresh_enqueue(&retained, &current, true, Some(2001))
            .unwrap()
            .is_none()
    );
    current.serial += 1;
    assert!(
        fresh_enqueue(&retained, &current, true, Some(2001))
            .unwrap()
            .is_none()
    );
    let mut action = retained.action.clone();
    action.id = current.serial;
    current.pending.push(action.clone());
    assert!(
        fresh_enqueue(&retained, &current, false, Some(2001))
            .unwrap()
            .is_none()
    );
    assert!(fresh_enqueue(&retained, &current, true, Some(9999)).is_err());
    let accepted = fresh_enqueue(&retained, &current, true, Some(2001))
        .unwrap()
        .unwrap();
    assert_eq!(accepted, action);
    assert!(!completed(&accepted, &current));
    let mut state = Vec::new();
    assert!(
        popup_press_path(
            true,
            &mut state,
            |state| {
                state.push("release_only_once");
                Ok(())
            },
            |_| panic!("enabled path diagnosed instead of attempting release")
        )
        .unwrap()
    );
    assert_eq!(state, ["release_only_once"]);
}

fn with_progress_driver(enabled: bool, check: impl FnOnce(&mut Driver<'_>)) {
    with_progress_driver_vm(enabled, None, check);
}

fn with_progress_driver_vm(
    enabled: bool, vm: Option<sims_vm::Vm>, check: impl FnOnce(&mut Driver<'_>),
) {
    let (viewer, initial_load) = Viewer::new(Args::parse_from(["sims-room", "--paused"]));
    drop(initial_load);
    let mut publication = Published::capture(&viewer);
    let scene = Arc::new(candidate_scene());
    let camera = sims_sim::pose::portrait::view::Input {
        viewport: [0, 0, 96, 96],
        center: [0, 0],
        zoom: 1,
        rotation: 0,
    };
    let base = JourneyGround {
        scene: scene.clone(),
        display_size: [96, 96],
        widget_epoch: 5,
        tick: 200,
        actor: 105,
        actor_lifetime: 1001,
        floor: 1,
        camera,
        lot: Arc::new(sims_sim::lot_geometry::Lot {
            size: 2,
            active_size: None,
            heights: vec![[0; 4]; 4],
            pool: vec![0; 4],
            water: vec![255; 4],
            floors: vec![],
        }),
        lot_size: 2,
        input: None,
        object_input: true,
        serial: 40,
        queue: vec![],
        queue_targets: BTreeMap::new(),
    };
    let mut snapshot = held_snapshot();
    snapshot.input = None;
    snapshot.objects.insert(
        420,
        JourneyObjectIdentity {
            lifetime: Some(5001),
            person: false,
            loaded_interaction_table_count: None,
        },
    );
    publication.scene = Some(scene);
    publication.widget_epoch = 5;
    publication.session_present = true;
    publication.pending = false;
    publication.native_clock = true;
    publication.native_transport = true;
    publication.playing = true;
    publication.native_selection = Some(Ok(Some(105)));
    publication.resident = 105;
    publication.residents = vec![Resident {
        object: 105,
        neighbor: 48,
        name: "Bob".into(),
        enabled: true,
        highlighted: false,
    }];
    publication.tick_label = Some("Tick 200 synthetic".into());
    publication.dialog = Some(record());
    publication.native_popup = None;
    publication.native_popup_epoch = 6;
    publication.menu = None;
    publication.journey_walking = Some(Ok(native()));
    publication.journey_object = Some(Ok(snapshot));
    publication.journey_ground = Some(Ok(base.clone()));
    publication.journey_camera = Some(Ok(camera));
    publication.journey_targets.insert(
        229,
        Ok(JourneyTarget::Object {
            target: 230,
            table_sit: false,
        }),
    );
    publication.cluster = Some(Arc::new(hud::cluster::Published {
        values: hud::cluster::Values {
            pending: false,
            ready: true,
            mode_raw: Some(2),
            panel_state_raw: None,
            can_show_live: true,
            can_show_buy: false,
            floor: 1,
            native_clock: true,
            pause_raw: None,
            people_visible: true,
            clock: "synthetic".into(),
            funds: "synthetic".into(),
            buttons: Default::default(),
        },
        art: None,
    }));
    let configured = vm.is_some();
    let host = match vm {
        Some(vm) => Host::from_object_progress_fixture_for_test(viewer, publication.clone(), vm, enabled),
        None => Host::from_viewer_for_test(viewer),
    };
    let (sender, receiver) = mpsc::channel();
    let mut journey = Journey {
        host,
        executor: ThreadPool::builder().pool_size(1).create().unwrap(),
        sender,
        receiver,
        streams: 0,
        tasks: 0,
        idle: true,
        wall: Instant::now(),
        wall_seconds: 60.,
        epoch: Instant::now(),
        millis: 0,
        commands: 0,
        frames: 0,
        service_polls: 0,
        effect_polls: 0,
        dialogs: 0,
        events: vec![],
        timing: None,
        timing_parent: None,
    };
    if !configured {
        journey.host.install_synthetic_publication_for_test(publication);
    }
    let publication = journey.host.publication_for_test();
    let entry = Entry {
        instruction: record(),
        cut: Cut::current(&journey),
        initial: native(),
        base,
        hud_height: publication.hud_height,
        retained_streams: 0,
        retained_tasks: 0,
    };
    let mut driver = Driver {
        start: Usage::current(&journey), journey: &mut journey, entry,
        wall: Instant::now(), probes: 0, opened: true, pick: None, offer: None,
        accepted: None, proof: Proof::default(), next: None, boundary_valid: false,
        popup_press_diagnostic: None, redraw_source: None,
        popup_release_enabled: enabled, popup_release_attempted: false,
        diagnostic: enabled.then(|| Box::new(Diagnostic::default())),
    };
    check(&mut driver);
}

#[test]
fn tutorial_object_progress_effect_categories_reconcile_including_timeout_and_finished() {
    let mut diagnostic = Diagnostic::default();
    diagnostic.effects.dispatch = 2;
    diagnostic.effects.redraw = 1;
    for delivery in [
        Ok(Delivery::Action(iced_runtime::Action::Output(Event::Completion))),
        Ok(Delivery::Finished),
        Err(mpsc::RecvTimeoutError::Timeout),
        Ok(Delivery::Action(iced_runtime::Action::Output(Event::Frame(Instant::now())))),
        Err(mpsc::RecvTimeoutError::Disconnected),
    ] {
        diagnostic.effects.receive += 1;
        diagnostic.effects.receive_without_outcome += 1;
        diagnostic.effects.received(&delivery);
    }
    let receipt = diagnostic.receipt(Some(Usage {
        effect_polls: 8, ..Default::default()
    }), 1, 0);
    for outcome in ["completion", "finished", "timeout", "other_output", "disconnected"] {
        assert_eq!(receipt["effects"][outcome], 1);
    }
    assert_eq!(receipt["effect_charge_total"], 8);
    assert_eq!(receipt["effect_charges_reconcile"], true);
    assert_eq!(receipt["receive_outcomes_reconcile"], true);
    assert_eq!(receipt["stream_balance_reconciles"], true);
    assert!(receipt["rejected_reservation"].is_null());
    diagnostic.effects.receive += 1;
    diagnostic.effects.receive_without_outcome += 1;
    assert_eq!(diagnostic.effects.receive, diagnostic.effects.outcome_total() + 1);
    with_progress_driver(true, |driver| {
        let source = driver.journey.host.publication_for_test().clone();
        driver.dispatch(Event::Completion).unwrap();
        let mut input = UiInput::new().unwrap();
        let observation = driver.observation(&mut input).unwrap();
        assert!(observation.messages.is_empty());
        driver.entry.retained_streams = 1;
        driver.journey.streams = 1;
        driver.journey.sender.send(Delivery::Finished).unwrap();
        driver.drain().unwrap();
        let diagnostic = driver.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.effects.dispatch, 1);
        assert_eq!(diagnostic.effects.redraw, 1);
        assert_eq!(diagnostic.effects.receive, 1);
        assert_eq!(diagnostic.effects.finished, 1);
        assert_eq!(driver.journey.effect_polls, 3);
        assert_eq!(driver.journey.commands, 1);
        assert_eq!(driver.journey.tasks, 0);
        assert_eq!(driver.journey.frames, 0);
        assert!(Arc::ptr_eq(&source, driver.journey.host.publication_for_test()));
        let receipt = diagnostic.receipt(driver.usage().ok(), 1, 0);
        assert_eq!(receipt["effect_charges_reconcile"], true);
        assert_eq!(receipt["receive_outcomes_reconcile"], true);
        assert_eq!(receipt["stream_balance_reconciles"], true);
    });
}

#[test]
fn tutorial_object_progress_rejected_dispatch_records_prospective_without_execution() {
    for global_only in [false, true] {
        with_progress_driver(true, |driver| {
            let phase = Usage {
                frames: 6, service_polls: 7, pixel_scans: 9,
                effect_polls: if global_only { 10 } else { 512 },
                commands: if global_only { 20 } else { 256 },
                tasks: if global_only { 30 } else { 128 },
            };
            let baseline = Usage {
                frames: 4, service_polls: 5,
                effect_polls: if global_only { MAX_POLLS - 1 - phase.effect_polls } else { 17 },
                commands: if global_only { MAX_COMMANDS - 1 - phase.commands } else { 19 },
                tasks: if global_only { MAX_COMMANDS - 1 - phase.tasks } else { 23 },
                pixel_scans: 0,
            };
            driver.start = baseline;
            driver.journey.frames = baseline.frames + phase.frames;
            driver.journey.service_polls = baseline.service_polls + phase.service_polls;
            driver.journey.effect_polls = baseline.effect_polls + phase.effect_polls;
            driver.journey.commands = baseline.commands + phase.commands;
            driver.journey.tasks = baseline.tasks + phase.tasks;
            driver.probes = phase.pixel_scans;
            let global = Usage::current(driver.journey);
            let delta = Usage { commands: 1, tasks: 1, effect_polls: 1, ..Default::default() };
            let expected_phase = Usage {
                frames: 6, service_polls: 7, pixel_scans: 9,
                effect_polls: if global_only { 11 } else { 513 },
                commands: if global_only { 21 } else { 257 },
                tasks: if global_only { 31 } else { 129 },
            };
            let expected_global = Usage {
                frames: 10, service_polls: 12, pixel_scans: 0,
                effect_polls: if global_only { MAX_POLLS } else { 530 },
                commands: if global_only { MAX_COMMANDS } else { 276 },
                tasks: if global_only { MAX_COMMANDS } else { 152 },
            };
            assert_eq!(serde_json::to_value(driver.usage().unwrap()).unwrap(), json!(phase));
            if global_only {
                assert!(phase_capacity(phase, delta, 200, 200, 0, 0.).is_ok());
            }
            let source = driver.journey.host.publication_for_test().clone();
            assert!(driver.dispatch(Event::Frame(driver.journey.epoch)).is_err());
            let first = serde_json::to_value(driver.diagnostic.as_ref().unwrap().rejection.as_ref().unwrap()).unwrap();
            assert_eq!(first, json!({
                "site":"dispatch", "consumed":phase, "requested_delta":delta,
                "prospective":expected_phase, "consumed_global":global,
                "prospective_global":expected_global, "tick_reservation":0,
                "native_tick":200, "operation_executed":false,
            }));
            assert_eq!(driver.diagnostic.as_ref().unwrap().rejected_reservations, 1);
            assert!(driver.reserve("second", Usage {
                commands: 2, tasks: 2, effect_polls: 2, ..Default::default()
            }, 0).is_err());
            let diagnostic = driver.diagnostic.as_ref().unwrap();
            assert_eq!(diagnostic.rejected_reservations, 2);
            assert_eq!(serde_json::to_value(diagnostic.rejection.as_ref().unwrap()).unwrap(), first);
            assert_eq!(diagnostic.effects.charge_total(), 0);
            assert_eq!(serde_json::to_value(Usage::current(driver.journey)).unwrap(), json!(global));
            assert_eq!(serde_json::to_value(driver.usage().unwrap()).unwrap(), json!(phase));
            assert_eq!(driver.journey.streams, 0);
            assert!(driver.journey.events.is_empty());
            assert!(!driver.proof.command_sent);
            assert!(!driver.proof.viewport_left_press_sent);
            assert!(!driver.proof.viewport_left_release_sent);
            assert!(Arc::ptr_eq(&source, driver.journey.host.publication_for_test()));
        });
    }
}


#[test]
fn tutorial_object_progress_ledger_preserves_first_prefix_latest_and_overflow_count() {
    with_progress_driver(true, |driver| {
        driver.observe().unwrap();
        let initial = driver.diagnostic.as_ref().unwrap().ledger.entries[0].unwrap();
        let mut ledger = ProgressLedger::default();
        for index in 0..PROGRESS_CAPACITY + 9 {
            ledger.record(ProgressEntry { driver_ms: index as u64, ..initial });
        }
        let latest = ledger.entries[PROGRESS_CAPACITY - 1].unwrap();
        ledger.record(latest);
        assert_eq!(ledger.transitions, PROGRESS_CAPACITY + 9);
        assert_eq!(ledger.observations, PROGRESS_CAPACITY + 10);
        assert_eq!(ledger.omitted_transitions, 9);
        assert_eq!(ledger.entries[0].unwrap().driver_ms, 0);
        assert_eq!(ledger.entries[PROGRESS_CAPACITY - 2].unwrap().driver_ms, 30);
        assert_eq!(latest.driver_ms, 40);
    });
}

#[test]
fn tutorial_object_progress_stale_owner_or_new_instruction_has_no_ledger_credit() {
    for mutation in 0..3 {
        with_progress_driver(true, |driver| {
            let mut publication = (**driver.journey.host.publication_for_test()).clone();
            match mutation {
                0 => publication.journey_walking.as_mut().unwrap().as_mut().unwrap().actor_lifetime = Some(9999),
                1 => publication.dialog.as_mut().unwrap().id += 1,
                2 => {
                    publication.dialog.as_mut().unwrap().id += 1;
                    publication.dialog.as_mut().unwrap().source = "stale-source".into();
                }
                _ => unreachable!(),
            }
            driver.journey.host.install_synthetic_publication_for_test(publication);
            assert!(driver.observe().is_err());
            assert_eq!(driver.diagnostic.as_ref().unwrap().ledger.observations, 0);
            assert_eq!(driver.diagnostic.as_ref().unwrap().ledger.transitions, 0);
            assert!(!driver.proof.enqueue_accepted);
            assert!(!driver.proof.native_completion_observed);
            assert_eq!(driver.boundary_valid, mutation == 1);
            if mutation != 0 {
                assert!(driver.next.is_some());
            }
        });
    }
}

#[test]
fn tutorial_object_progress_disabled_driver_keeps_observation_and_effect_path() {
    with_progress_driver(false, |driver| {
        let source = driver.journey.host.publication_for_test().clone();
        let before = redraw_identity(&source).unwrap();
        driver.observe().unwrap();
        let mut unavailable = (*source).clone();
        unavailable.journey_object = None;
        driver.record_progress(&unavailable, &native());
        assert!(driver.diagnostic.is_none());
        assert_eq!(before, redraw_identity(driver.journey.host.publication_for_test()).unwrap());
        assert_eq!(driver.journey.effect_polls, 0);
        assert_eq!(driver.journey.commands, 0);
        assert_eq!(driver.journey.frames, 0);
        assert_eq!(driver.journey.tasks, 0);
        assert!(driver.journey.events.is_empty());
        assert!(Arc::ptr_eq(&source, driver.journey.host.publication_for_test()));
    });
}

#[test]
fn tutorial_object_progress_production_drain_and_retained_publication_wiring() {
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/original_viewer/worker_host.rs"));
    let constructor = source.split("pub(super) fn new_with_object_progress_for_test(").nth(1).unwrap()
        .split("    #[cfg(test)]\n    fn object_progress_engine(").next().unwrap();
    assert!(constructor.contains("let engine = Self::object_progress_engine(owner);\n        Self::start_initial_worker(fallback, engine, task)"));
    let mut disabled_identity = None;
    for enabled in [false, true] {
        let mut vm = sims_vm::Vm::new(sims_vm::World::default(), 1);
        let thread: sims_vm::Thread = serde_json::from_value(json!({
            "service":null,"dispatch":null,"autonomy":null,"find_best":[],
            "completed_actions":[],"last_result":false,"callback":null,"callback_result":null,
            "stack":[{"continuation":3,"action_target":230,"action_icon":false,
                "code_owner":"synthetic","location":{"source":"synthetic","bhav":4100,"instruction":8},
                "caller":105,"callee":230,"stack_object":230,"args":[],"locals":[],
                "opaque_locals":[],"storage_offset":0}],
            "frame_memory":[],"check":false,"interrupted":false,"status":{"Sleeping":{"wake_tick":205}}
        })).unwrap();
        vm.object_threads.insert(105, 7);
        vm.threads.insert(7, thread);
        let expected = JourneyObjectProgress::capture(&vm, 105, None, None);
        assert_eq!((expected.thread, expected.status, expected.wake_tick, expected.routine, expected.pc),
                   (Some(7), Some("sleeping"), Some(205), Some(4100), Some(8)));
        with_progress_driver_vm(enabled, Some(vm), |driver| {
            let publication = driver.journey.host.publication_for_test();
            let identity = redraw_identity(publication).unwrap();
            assert_eq!(object(publication).unwrap().progress.as_deref().copied(), enabled.then_some(expected));
            if enabled {
                assert_eq!(disabled_identity.as_ref(), Some(&identity));
            } else {
                disabled_identity = Some(identity);
            }
            driver.entry.retained_streams = 1;
            driver.journey.streams = 1;
            driver.journey.sender.send(Delivery::Finished).unwrap();
            driver.drain().unwrap();
            assert_eq!(driver.journey.effect_polls, 1);
            assert_eq!(driver.journey.commands, 0);
            assert_eq!(driver.journey.frames, 0);
            assert_eq!(driver.journey.tasks, 0);
            if enabled {
                let diagnostic = driver.diagnostic.as_ref().unwrap();
                assert_eq!(diagnostic.effects.receive, 1);
                assert_eq!(diagnostic.effects.finished, 1);
                assert_eq!(diagnostic.ledger.entries[0].unwrap().streams, 1);
                let last = diagnostic.ledger.entries[diagnostic.ledger.transitions - 1].unwrap();
                assert_eq!(last.streams, 0);
                assert_eq!(last.native_tick, 200);
                assert_eq!(last.vm, Some(expected));
                let receipt = diagnostic.receipt(driver.usage().ok(), 1, 0);
                assert_eq!(receipt["effect_charges_reconcile"], true);
                assert_eq!(receipt["receive_outcomes_reconcile"], true);
                assert_eq!(receipt["stream_balance_reconciles"], true);
                assert_eq!(receipt["delivery_stream_identity"], "unavailable: shared Delivery is untagged");
            } else {
                assert!(driver.diagnostic.is_none());
                assert!(object(driver.journey.host.publication_for_test()).unwrap().progress.is_none());
            }
        });
    }
}


#[test]
fn tutorial_object_progress_production_queued_completion_keeps_rejected_work_unexecuted() {
    with_progress_driver(true, |driver| {
        driver.journey.effect_polls = 511;
        driver.journey.streams = 1;
        driver.entry.retained_streams = 1;
        driver.diagnostic.as_mut().unwrap().effects.redraw = 511;
        driver.journey.sender.send(Delivery::Action(
            iced_runtime::Action::Output(Event::Completion),
        )).unwrap();
        driver.journey.sender.send(Delivery::Finished).unwrap();
        let source = driver.journey.host.publication_for_test().clone();
        assert!(driver.drain().is_err());
        assert_eq!(driver.journey.effect_polls, 512);
        assert_eq!(driver.journey.commands, 0);
        assert_eq!(driver.journey.tasks, 0);
        assert_eq!(driver.journey.frames, 0);
        assert_eq!(driver.journey.streams, 1);
        assert!(Arc::ptr_eq(&source, driver.journey.host.publication_for_test()));
        let diagnostic = driver.diagnostic.as_ref().unwrap();
        assert_eq!(diagnostic.effects.receive, 1);
        assert_eq!(diagnostic.effects.receive_without_outcome, 0);
        assert_eq!(diagnostic.effects.timeout, 0);
        assert_eq!(diagnostic.effects.completion, 1);
        assert_eq!(diagnostic.effects.finished, 0);
        assert_eq!(diagnostic.effects.dispatch, 0);
        assert_eq!(diagnostic.rejected_reservations, 1);
        assert_eq!(diagnostic.rejection.as_ref().unwrap().site, "dispatch");
        let receipt = diagnostic.receipt(driver.usage().ok(), 1, 1);
        assert_eq!(receipt["effect_charges_reconcile"], true);
        assert_eq!(receipt["receive_outcomes_reconcile"], true);
        assert_eq!(receipt["stream_balance_reconciles"], true);
        assert!(matches!(driver.journey.receiver.try_recv().unwrap(), Delivery::Finished));
        assert!(matches!(driver.journey.receiver.try_recv(), Err(mpsc::TryRecvError::Empty)));
    });
}

