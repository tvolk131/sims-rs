use serde_json::{Value, json};
use sims_sim::{
    lot_environment::{Tuning, manager},
    lot_geometry::{FloorSource, Lot},
    lot_presenter::{
        camera, hover::History, lifecycle, open, transition, visibility, walls::cutaway::Mask,
    },
};

fn snapshot(s: &camera::State, v: &open::State) -> Value {
    let a = &s.settings;
    json!({
        "transition": transition::State {
            origin: v.origin, pending: v.pending, zoom: v.zoom as u8, widths: v.widths,
            level: v.floor.level, world_rotation: a.world_rotation,
            display_rotation: v.display_rotation, world_size: 64, world_bounds: a.bounds,
            camera_mode: a.camera_mode, moving: a.moving, skip_edges: a.skip_edges,
            cutaway: v.floor.visibility.enabled, include_visible: v.floor.visibility.include_visible,
            graphics_enabled: a.graphics_enabled, ui_present: a.ui_present, depth_scales: a.depth_scales,
        },
        "scroll": {"origin":v.origin,"pending":v.pending,"deferred":a.deferred,
            "locked":a.locked,"attempts":a.attempts,"bottom_margin":a.bottom_margin},
        "active":a.motion.active,"application":a.application_value_bits,"saved":a.saved_value_bits
    })
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_composed_camera_services_and_shared_state_restore() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/camera-composition.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 96);
    let floor = FloorSource {
        coverings: vec![0; 4096],
        walls: vec![Default::default(); 4096],
        object_heads: vec![0; 4096],
        flags: vec![0; 4096],
    };
    let lot = Lot::rebuild(
        64,
        vec![[0; 4]; 4096],
        vec![0; 4096],
        vec![255; 4096],
        vec![floor.clone(), floor],
    )
    .unwrap();
    let mut manager = manager::Manager::new(
        &lot,
        Tuning::from_lookup(|_| None).unwrap(),
        manager::Settings {
            daylight_multiplier: 1.0,
            light_range: 1.0,
            rooms_enabled: true,
            models: [None; 6],
        },
    )
    .unwrap();
    // Native probe explicitly stops at an empty room directory/wall graph.
    // Room masks and nonempty wall records have their own native fixtures.
    manager.rooms.clear();
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut t = c["transition"].clone();
        t["skip_edges"] = json!(u8::from(t["skip_edges"].as_bool().unwrap()));
        let t: transition::State = serde_json::from_value(t).unwrap();
        let sc: sims_sim::lot_presenter::scroll::State =
            serde_json::from_value(c["scroll"].clone()).unwrap();
        let initial_view = open::State {
            origin: t.origin,
            pending: t.pending,
            zoom: u32::from(t.zoom),
            display_rotation: t.display_rotation,
            widths: t.widths,
            floor: lifecycle::State {
                level: t.level,
                all_walls: 0,
                history: History {
                    capacity: 3,
                    rooms: vec![],
                },
                visibility: visibility::State {
                    enabled: t.cutaway,
                    dirty: 0,
                    include_visible: t.include_visible,
                    mask: Mask {
                        first_row: 0,
                        rows: vec![0; 64],
                    },
                },
            },
        };
        let initial = camera::State::from_lot(
            &lot,
            &manager,
            &initial_view,
            camera::Settings {
                roof: None,
                clock: 123,
                application_mode: 2,
                viewport: serde_json::from_value(c["viewport"].clone()).unwrap(),
                bounds: t.world_bounds,
                world_rotation: t.world_rotation,
                camera_mode: t.camera_mode,
                moving: t.moving,
                projection_ready: c["projection_ready"].as_u64().unwrap() as u32,
                renderer_ready: c["renderer_ready"].as_bool().unwrap(),
                graphics_enabled: t.graphics_enabled,
                ui_present: t.ui_present,
                skip_edges: t.skip_edges,
                depth_scales: t.depth_scales,
                floor_heights: vec![23, 58, 116, 232],
                deferred: sc.deferred,
                locked: sc.locked,
                attempts: sc.attempts,
                bottom_margin: sc.bottom_margin,
                motion: serde_json::from_value(c["motion"].clone()).unwrap(),
                duration_input: 17,
                application_value_bits: c["application"].as_u64().unwrap() as u32,
                saved_value_bits: c["saved"].as_u64().unwrap() as u32,
            },
        )
        .unwrap();
        let target = c["target"].as_u64().unwrap() as u32;
        let force = c["force"].as_u64().unwrap() as u8;
        let refresh_ui = c["refresh_ui"].as_u64().unwrap() as u8;
        let request = if c["operation"] == "zoom" {
            open::Request::Zoom {
                zoom: target,
                force,
                refresh_ui,
            }
        } else {
            open::Request::Rotate {
                rotation: target,
                force,
                refresh_ui,
            }
        };
        let inputs = camera::Inputs {
            roof_controls_present: false,
            lot: &lot,
            rooms: &manager,
            application_mode: 2,
            selected: None,
            clock: 123,
        };
        let mut completed = vec![];
        for budget in [1, 10000] {
            let (mut state, mut view) = (initial.clone(), initial_view.clone());
            let mut cursor = camera::Cursor::begin(request.clone(), &mut state, &mut view).unwrap();
            let mut events = vec![];
            for slice in 0..2000 {
                if budget == 1 {
                    let bytes = serde_json::to_vec(&(&state, &view, &cursor)).unwrap();
                    (state, view, cursor) = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(
                        serde_json::to_vec(&(&state, &view, &cursor)).unwrap(),
                        bytes
                    );
                }
                match cursor
                    .advance(&mut state, &mut view, &inputs, budget)
                    .unwrap()
                {
                    camera::Progress::Pending => (),
                    camera::Progress::Complete => break,
                    camera::Progress::Service(service) => {
                        let before = (state.clone(), view.clone(), cursor.clone());
                        assert_eq!(
                            cursor
                                .advance(&mut state, &mut view, &inputs, budget)
                                .unwrap(),
                            camera::Progress::Service(service.clone())
                        );
                        assert_eq!((&state, &view, &cursor), (&before.0, &before.1, &before.2));
                        let token = matches!(
                            service,
                            camera::Service::Transition(
                                transition::Request::ResolveUi
                                    | transition::Request::ResolveSurface
                            ) | camera::Service::Cancel(
                                sims_sim::lot_presenter::motion::cancel::Request::ResolveSurface
                            )
                        );
                        let bad = if token {
                            camera::Reply::Done
                        } else {
                            camera::Reply::Token(Some(9))
                        };
                        assert!(
                            cursor
                                .acknowledge(&mut state, &mut view, &service, bad)
                                .is_err()
                        );
                        assert_eq!((&state, &view, &cursor), (&before.0, &before.1, &before.2));
                        let mut event = snapshot(&state, &view);
                        event["service"] = serde_json::to_value(&service).unwrap();
                        let index = events.len();
                        assert_eq!(
                            event, row["original"]["events"][index],
                            "case {n}, event {index}, budget {budget}"
                        );
                        events.push(event);
                        let reply = if token {
                            camera::Reply::Token(Some(
                                if matches!(
                                    service,
                                    camera::Service::Transition(transition::Request::ResolveUi)
                                ) {
                                    33
                                } else {
                                    7
                                },
                            ))
                        } else {
                            camera::Reply::Done
                        };
                        cursor
                            .acknowledge(&mut state, &mut view, &service, reply)
                            .unwrap();
                    }
                }
                assert!(slice < 1999, "case {n} stalled");
            }
            assert!(cursor.complete());
            let mut final_state = snapshot(&state, &view);
            final_state["events"] = json!(events);
            assert_eq!(
                final_state, row["original"],
                "case {n} final, budget {budget}"
            );
            completed.push((state, view, cursor));
        }
        assert_eq!(
            completed[0], completed[1],
            "case {n} work budget altered state"
        );
    }
}
