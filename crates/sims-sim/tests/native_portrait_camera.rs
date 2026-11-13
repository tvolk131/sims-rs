use serde::{Serialize, de::DeserializeOwned};
use sims_sim::{
    lot_environment::{Tuning, manager},
    lot_geometry::{FloorSource, Lot},
    lot_presenter::{
        camera::{self, motion_frame, portrait_centering as portrait},
        camera_command::{self, Zoom},
        centering,
        hover::History,
        lifecycle, motion, open,
        scroll::Bounds,
        visibility,
        walls::cutaway::Mask,
    },
};

fn restore<T: Serialize + DeserializeOwned>(value: T) -> T {
    serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap()
}

fn center_state() -> centering::State {
    centering::State {
        level: 1,
        origin: [137, -239],
        pending: [91, -73],
        viewport: [11, 17, 812, 618],
        bounds: [0, 63, 0, 63],
        zoom: 2,
        graphics_ready: 1,
        motion_active: 0,
        motion_policy: 0,
        motion_duration: 17,
        duration_input: 1000,
        moving: 0,
        motion_disabled: 0,
        motion_allowed: 1,
        camera_mode: 0,
        application_value_bits: 0x3f80_0000,
        saved_value_bits: 99,
    }
}

fn unbounded() -> Bounds {
    Bounds {
        x: [-100_000, 100_000],
        y: [-100_000, 100_000],
        bottom_margin: 23,
    }
}

#[test]
fn portrait_projection_rotation_height_and_raw_origin_are_not_world_centering() {
    for (rotation, point) in [
        (0, [101, 203]),
        (1, [203, 922]),
        (2, [922, 820]),
        (3, [820, 101]),
    ] {
        for (zoom, adjustment) in [(0, 9), (1, 19), (2, 39), (3, 78)] {
            let mut state = center_state();
            state.zoom = zoom;
            state.graphics_ready = 0;
            state.bounds = [0, 0, 0, 0];
            let mut cursor = portrait::Cursor::begin(&state, [101, 203], 1, 2, 1, rotation);
            let project = portrait::Request::Project { point, level: 1 };
            assert_eq!(cursor.request(), Some(project.clone()));
            (state, cursor) = restore((state, cursor));
            cursor
                .acknowledge(&mut state, &project, portrait::Reply::Screen([121, -57]))
                .unwrap();
            let target = [594, -60 + adjustment];
            let bounds = portrait::Request::Bounds { target };
            assert_eq!(cursor.request(), Some(bounds.clone()));
            (state, cursor) = restore((state, cursor));
            cursor
                .acknowledge(&mut state, &bounds, portrait::Reply::Bounds(unbounded()))
                .unwrap();
            let configure = portrait::Request::ConfigureMotion {
                start: [137, -239],
                target,
                style: 1,
            };
            assert_eq!(cursor.request(), Some(configure.clone()));
            assert_eq!(state.pending, [91, -73]);
            assert_eq!(state.motion_policy, 1);
            assert_eq!(state.motion_duration, 1000);
            cursor
                .acknowledge(&mut state, &configure, portrait::Reply::Done)
                .unwrap();
            assert!(cursor.complete());
            assert_eq!(state.moving, 1);
            assert_eq!(state.camera_mode, 1);
            assert_eq!(state.saved_value_bits, 0x3f80_0000);
            assert_eq!(state.application_value_bits, 0);
        }
    }
}

#[test]
fn active_portrait_motion_still_projects_and_bounds_but_does_not_retarget() {
    let mut state = center_state();
    state.motion_active = 1;
    let original = state.clone();
    let mut cursor = portrait::Cursor::begin(&state, [280, 360], 1, 2, 1, 0);
    let request = cursor.request().unwrap();
    cursor
        .acknowledge(&mut state, &request, portrait::Reply::Screen([121, -57]))
        .unwrap();
    let bounds = cursor.request().unwrap();
    assert!(matches!(bounds, portrait::Request::Bounds { .. }));
    let before = (state.clone(), cursor.clone());
    assert!(
        cursor
            .acknowledge(&mut state, &bounds, portrait::Reply::Done)
            .is_err()
    );
    assert_eq!((state.clone(), cursor.clone()), before);
    cursor
        .acknowledge(&mut state, &bounds, portrait::Reply::Bounds(unbounded()))
        .unwrap();
    assert!(cursor.complete());
    assert_eq!(state, original);
}

#[test]
fn immediate_portrait_center_wraps_short_then_clamps_and_clears_pending_before_scroll() {
    let mut state = center_state();
    state.motion_active = 1;
    let mut cursor = portrait::Cursor::begin(&state, [280, 360], 1, 2, 0, 0);
    let project = cursor.request().unwrap();
    cursor
        .acknowledge(&mut state, &project, portrait::Reply::Screen([i16::MIN, 0]))
        .unwrap();
    assert_eq!(
        cursor.request(),
        Some(portrait::Request::Bounds {
            target: [537, -32_668],
        })
    );
    let bounds = cursor.request().unwrap();
    cursor
        .acknowledge(
            &mut state,
            &bounds,
            portrait::Reply::Bounds(Bounds {
                x: [1000, -1000],
                y: [-500, 500],
                bottom_margin: 23,
            }),
        )
        .unwrap();
    let scroll = portrait::Request::Scroll {
        position: [1000, -500],
        immediate: 1,
    };
    assert_eq!(cursor.request(), Some(scroll.clone()));
    assert_eq!(state.pending, [0, 0]);
    assert_eq!(state.origin, [137, -239]);
    cursor
        .acknowledge(&mut state, &scroll, portrait::Reply::Done)
        .unwrap();
    assert_eq!(cursor.request(), Some(portrait::Request::InvalidateView));
    cursor
        .acknowledge(
            &mut state,
            &portrait::Request::InvalidateView,
            portrait::Reply::Done,
        )
        .unwrap();
    assert!(cursor.complete());
}

#[test]
fn portrait_nonperson_or_other_floor_uses_existing_world_owner_contract() {
    for (object_type, level) in [(1, 1), (2, 2)] {
        for animated in [0, 1, 2] {
            let cursor = portrait::Cursor::begin(
                &center_state(),
                [101, 203],
                level,
                object_type,
                animated,
                1,
            );
            assert_eq!(
                cursor.request(),
                Some(portrait::Request::World {
                    point: [203, 922],
                    level,
                    style: 1,
                    immediate: u8::from(animated == 0),
                })
            );
        }
    }
}

#[test]
fn portrait_motion_notification_tests_disabled_against_exactly_one() {
    for disabled in [0, 1, 2] {
        let mut state = center_state();
        state.motion_disabled = disabled;
        let mut cursor = portrait::Cursor::begin(&state, [280, 360], 1, 2, 1, 0);
        let project = cursor.request().unwrap();
        cursor
            .acknowledge(&mut state, &project, portrait::Reply::Screen([121, -57]))
            .unwrap();
        let bounds = cursor.request().unwrap();
        cursor
            .acknowledge(&mut state, &bounds, portrait::Reply::Bounds(unbounded()))
            .unwrap();
        let configure = cursor.request().unwrap();
        cursor
            .acknowledge(&mut state, &configure, portrait::Reply::Done)
            .unwrap();
        assert_eq!(state.moving, 1);
        assert_eq!(state.camera_mode, u8::from(disabled != 1));
        assert_eq!(state.application_value_bits == 0, disabled != 1);
    }
}

#[test]
fn live_motion_frame_consumes_displacement_once_and_unlocks_after_drawing() {
    let mut state = center_state();
    state.motion_active = 1;
    state.pending = [0, 0];
    let mut cursor = motion_frame::Cursor::begin(&mut state).unwrap();
    for coordinate in [-200, 160] {
        let request = cursor.request().unwrap();
        (state, cursor) = restore((state, cursor));
        cursor
            .acknowledge(
                &mut state,
                &request,
                motion_frame::Reply::Coordinate(coordinate),
            )
            .unwrap();
    }
    assert_eq!(
        cursor.request(),
        Some(motion_frame::Request::Prepare(
            motion::frame::Request::Scroll {
                position: [160, -200],
                immediate: 0,
            }
        ))
    );
    state.pending = [23, 39];
    let scroll = cursor.request().unwrap();
    cursor
        .acknowledge(&mut state, &scroll, motion_frame::Reply::Done)
        .unwrap();
    assert_eq!(state.origin, [160, -200]);
    assert_eq!(state.pending, [23, 39]);
    let synchronize = cursor.request().unwrap();
    cursor
        .acknowledge(&mut state, &synchronize, motion_frame::Reply::Done)
        .unwrap();
    assert_eq!(state.motion_disabled, 1);
    while let Some(request) = cursor.request() {
        (state, cursor) = restore((state, cursor));
        let reply = if matches!(
            request,
            motion_frame::Request::Displacement(
                motion::frame::displacement::Request::DrawingEnabled
            )
        ) {
            motion_frame::Reply::Enabled(1)
        } else {
            motion_frame::Reply::Done
        };
        cursor.acknowledge(&mut state, &request, reply).unwrap();
        assert_eq!(state.origin, [160, -200]);
    }
    assert!(cursor.complete());
    assert_eq!(state.pending, [0, 0]);
    assert_eq!(state.motion_disabled, 0);
    state.motion_active = 0;
    state.moving = 0;
    assert!(motion_frame::Cursor::begin(&mut state).unwrap().complete());
    assert_eq!(state.origin, [160, -200]);
}

struct Scene {
    lot: Lot,
    manager: manager::Manager,
    camera: camera::State,
    view: open::State,
}

impl Scene {
    fn new() -> Self {
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
        manager.rooms.clear();
        let view = open::State {
            origin: [0, 0],
            pending: [0, 0],
            zoom: 2,
            display_rotation: 0,
            widths: [64, 32, 32, 16],
            floor: lifecycle::State {
                level: 1,
                all_walls: 0,
                history: History {
                    capacity: 3,
                    rooms: vec![],
                },
                visibility: visibility::State {
                    enabled: 0,
                    dirty: 0,
                    include_visible: 0,
                    mask: Mask {
                        first_row: 0,
                        rows: vec![0; 64],
                    },
                },
            },
        };
        let camera = camera::State::from_lot(
            &lot,
            &manager,
            &view,
            camera::Settings {
                roof: None,
                clock: 7,
                application_mode: 2,
                viewport: [0, 0, 800, 600],
                bounds: [0, 63, 0, 63],
                world_rotation: 0,
                camera_mode: 0,
                moving: 0,
                projection_ready: 1,
                renderer_ready: true,
                graphics_enabled: true,
                ui_present: true,
                skip_edges: 0,
                depth_scales: [1.0f32.to_bits(); 4],
                floor_heights: vec![23, 58, 116, 232],
                deferred: 1,
                locked: 0,
                attempts: 0,
                bottom_margin: 0,
                motion: motion::State {
                    x: [0; 4],
                    y: [0; 4],
                    active: 0,
                    policy: 0,
                    duration: 17,
                    speed_bits: 1.0f32.to_bits(),
                    rate_bits: 0,
                },
                duration_input: 1000,
                application_value_bits: 0x3f80_0000,
                saved_value_bits: 99,
            },
        )
        .unwrap();
        Self {
            lot,
            manager,
            camera,
            view,
        }
    }

    fn drive(&mut self, request: open::Request, clock: u32, budget: usize) -> Vec<camera::Service> {
        let mut cursor = camera::Cursor::begin(request, &mut self.camera, &mut self.view).unwrap();
        let mut services = Vec::new();
        for _ in 0..2000 {
            (self.camera, self.view, cursor) =
                restore((self.camera.clone(), self.view.clone(), cursor));
            let inputs = camera::Inputs {
                roof_controls_present: false,
                lot: &self.lot,
                rooms: &self.manager,
                application_mode: 2,
                selected: None,
                clock,
            };
            match cursor
                .advance(&mut self.camera, &mut self.view, &inputs, budget)
                .unwrap()
            {
                camera::Progress::Complete => return services,
                camera::Progress::Pending => (),
                camera::Progress::Service(service) => {
                    let before = (self.camera.clone(), self.view.clone(), cursor.clone());
                    assert_eq!(
                        cursor
                            .advance(&mut self.camera, &mut self.view, &inputs, budget)
                            .unwrap(),
                        camera::Progress::Service(service.clone())
                    );
                    assert_eq!(
                        (self.camera.clone(), self.view.clone(), cursor.clone()),
                        before
                    );
                    let reply = match &service {
                        camera::Service::Displacement(
                            motion::frame::displacement::Request::DrawingEnabled,
                        ) => camera::Reply::Enabled(1),
                        camera::Service::Floor(lifecycle::Request::ResolveSurface)
                        | camera::Service::Cancel(motion::cancel::Request::ResolveSurface) => {
                            camera::Reply::Token(Some(7))
                        }
                        _ => camera::Reply::Done,
                    };
                    cursor
                        .acknowledge(&mut self.camera, &mut self.view, &service, reply)
                        .unwrap();
                    services.push(service);
                }
            }
        }
        panic!("live camera root stalled");
    }
}

#[test]
fn saved_portrait_and_real_scroll_frames_are_budget_independent() {
    let mut results = Vec::new();
    for budget in [1, 17] {
        let mut scene = Scene::new();
        let center = open::Request::CenterPerson {
            point: [280, 360],
            level: 1,
        };
        assert!(scene.drive(center.clone(), 1000, budget).is_empty());
        assert_eq!(scene.camera.settings.attempts, 0);
        assert_eq!(scene.camera.settings.motion.x[3], 2000);
        assert_eq!(scene.camera.settings.motion.y[3], 2000);
        let motion = scene.camera.settings.motion.clone();
        assert!(scene.drive(center, 1100, budget).is_empty());
        assert_eq!(scene.camera.settings.motion, motion);
        let mut events = Vec::new();
        for clock in [1250, 1500, 2000] {
            events.extend(scene.drive(open::Request::MotionFrame, clock, budget));
            assert_eq!(scene.view.pending, [0, 0]);
            assert_eq!(scene.camera.settings.locked, 0);
        }
        assert_eq!(scene.camera.settings.attempts, 3);
        assert_eq!(scene.camera.settings.motion.active, 0);
        assert_eq!(scene.camera.settings.moving, 1);
        let final_origin = scene.view.origin;
        let final_frame = scene.drive(open::Request::MotionFrame, 2001, budget);
        assert!(matches!(
            final_frame.first(),
            Some(camera::Service::Cancel(motion::cancel::Request::Invalidate))
        ));
        assert_eq!(scene.camera.settings.attempts, 3);
        assert_eq!(scene.camera.settings.moving, 0);
        assert_eq!(scene.camera.settings.application_value_bits, 0x3f80_0000);
        assert_eq!(scene.view.origin, final_origin);
        assert!(
            events
                .iter()
                .any(|service| matches!(service, camera::Service::Displacement(_)))
        );
        results.push((scene.camera, scene.view, events, final_frame));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn pending_only_frames_consume_once_for_surface_scroll_and_full_invalidation() {
    for pending in [[23, 39], [800, 600]] {
        let mut scene = Scene::new();
        scene.view.pending = pending;
        let events = scene.drive(open::Request::MotionFrame, 1000, 1);
        assert_eq!(scene.view.origin, pending);
        assert_eq!(scene.view.pending, [0, 0]);
        assert_eq!(scene.camera.settings.attempts, 0);
        assert_eq!(scene.camera.settings.locked, 0);
        assert!(events.contains(&camera::Service::Displacement(
            motion::frame::displacement::Request::NotifyScroll
        )));
        assert_eq!(
            events.contains(&camera::Service::Displacement(
                motion::frame::displacement::Request::Invalidate
            )),
            pending == [800, 600]
        );
        assert!(scene.drive(open::Request::MotionFrame, 1001, 1).is_empty());
        assert_eq!(scene.view.origin, pending);
    }
}

#[test]
fn other_floor_person_runs_floor_and_world_center_services_without_roof_ownership() {
    let mut scene = Scene::new();
    let services = scene.drive(
        open::Request::CenterPerson {
            point: [280, 360],
            level: 2,
        },
        1000,
        1,
    );
    assert_eq!(scene.view.floor.level, 2);
    assert_eq!(scene.camera.settings.motion.active, 1);
    assert!(services.contains(&camera::Service::Center(
        centering::Request::RefreshFloorUi { level: 2, flag: 1 }
    )));
    assert!(!services.iter().any(|service| matches!(
        service,
        camera::Service::Roof(_) | camera::Service::Portrait(_)
    )));
}

#[test]
fn live_camera_commands_are_direct_restorable_and_not_publication_calls() {
    for mut command in [Zoom::center_person([280, 360], 1), Zoom::motion_frame()] {
        command.validate().unwrap();
        assert!(command.direct());
        let request = command.request().unwrap().clone();
        command = restore(command);
        let camera_command::Request::Camera(camera_request) = &request else {
            panic!("expected camera call")
        };
        assert!(Zoom::publication(camera_request.clone()).is_err());
        command.acknowledge(&request).unwrap();
        assert!(command.complete());
    }
}
