use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_sim::pose::{
    graphic_appearance::Model,
    graphic_frame as frame, graphic_transform as placement,
    model::{Bound, Matrix},
    model_frame, transform,
};

fn restore<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &mut T) {
    let restored = serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap();
    assert_eq!(*value, restored);
    *value = restored;
}
#[path = "common/native_graphic_bound.rs"]
mod native_graphic_bound;
fn compare(bound: &Bound, expected: &Value) {
    native_graphic_bound::compare(bound, expected, 0.);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_root_frames_execute_actual_model_skin_hierarchy_and_cache_services() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-root-frames.json")).unwrap();
    let program = sims_vm::Program::default();
    let mut frames = 0;
    for sequence in fixture["sequences"].as_array().unwrap() {
        let input = &sequence["input"];
        let mut crt = input["crt_state"].as_u64().unwrap() as u32;
        let mut bound = Bound::root(&mut crt).unwrap();
        bound
            .model
            .set_scale(serde_json::from_value(input["scale"].clone()).unwrap())
            .unwrap();
        let matrix: Matrix = serde_json::from_value(input["matrix"].clone()).unwrap();
        bound.publish(matrix).unwrap();
        let mut state = frame::State {
            clock_mode: input["mode"].as_u64().unwrap() as u8,
            now: -123,
            visible: 1,
            blend: None,
            placement: placement::Placement {
                base: [-999.; 3],
                offset: [4., 5., 6.],
                dirty: true,
                explicit: true,
            },
            matrix,
        };
        compare(&bound, &sequence["initial"]["bound"]);
        assert_eq!(
            state,
            serde_json::from_value(sequence["initial"]["frame"].clone()).unwrap()
        );
        for step in sequence["steps"].as_array().unwrap() {
            let action = &step["action"];
            if let Some(value) = action["visible_byte"].as_u64() {
                state.visible = value as u8;
            }
            let tick = action["tick"].as_i64().unwrap() as i32;
            let mut update = frame::Update::new(
                serde_json::from_value(action["entry"].clone()).unwrap(),
                action["only_visible"].as_bool().unwrap(),
            );
            let mut calls = Vec::new();
            loop {
                restore(&mut update);
                restore(&mut state);
                restore(&mut bound);
                let Some(request) = update
                    .advance(&mut state, tick, Some(Model { model: 1, skin: 1 }))
                    .unwrap()
                else {
                    break;
                };
                let reply = match &request {
                    frame::Request::OutsideLot => {
                        calls.push("outside");
                        frame::Reply::Outside(action["outside"].as_bool().unwrap())
                    }
                    frame::Request::Viewport => {
                        calls.push("viewport");
                        frame::Reply::Viewport([-10, -20, 640, 480])
                    }
                    frame::Request::Visibility { viewport } => {
                        assert_eq!(*viewport, [-10, -20, 640, 480]);
                        calls.push("visibility");
                        let visible = action["visible"].as_bool().unwrap();
                        state.visible = u8::from(visible);
                        frame::Reply::Visible(visible)
                    }
                    frame::Request::Invalidate => {
                        calls.push("invalidate");
                        frame::Reply::Done
                    }
                    frame::Request::Placement => {
                        let mut pass = placement::Update::default();
                        let request = pass
                            .advance(
                                &mut state.placement,
                                state.matrix,
                                placement::Source {
                                    position: [160, 320],
                                    facing: 0,
                                    level: 1,
                                },
                                transform::View {
                                    rotation: 0,
                                    floor_vector: [0.; 3],
                                },
                                transform::Terrain {
                                    size: 2,
                                    corners: &[[0; 4]; 4],
                                },
                            )
                            .unwrap()
                            .unwrap();
                        // The real cXObject +78 is a no-op; it does not move
                        // the physical owner or republish a dependent matrix.
                        assert!(matches!(request, placement::Request::Position(_)));
                        restore(&mut pass);
                        restore(&mut state);
                        restore(&mut bound);
                        pass.acknowledge(&mut state.placement, &request).unwrap();
                        frame::Reply::Done
                    }
                    frame::Request::Model {
                        model,
                        matrix,
                        now,
                        visible,
                    } => {
                        assert_eq!(*model, 1);
                        let mut pass = model_frame::Update::new(*matrix, *now, *visible, true);
                        loop {
                            restore(&mut pass);
                            restore(&mut bound);
                            let Some(request) = pass.next(&mut bound, &program).unwrap() else {
                                break;
                            };
                            assert_eq!(request, model_frame::Request::Geometry);
                            bound.refresh_geometry(false).unwrap();
                            restore(&mut pass);
                            restore(&mut bound);
                            let before = bound.clone();
                            assert_eq!(
                                pass.next(&mut bound, &program).unwrap(),
                                Some(request.clone())
                            );
                            assert_eq!(bound, before);
                            pass.acknowledge(&mut bound, &request).unwrap();
                            assert!(pass.acknowledge(&mut bound, &request).is_err());
                        }
                        assert!(pass.complete());
                        frame::Reply::Done
                    }
                    frame::Request::Skin { skin, now, sample } => {
                        assert_eq!(*skin, 1);
                        let mut pass = model_frame::Skin::new(*now, *sample != 0, true);
                        assert!(pass.next(&mut bound, &program).unwrap().is_none());
                        assert!(pass.complete());
                        restore(&mut pass);
                        restore(&mut bound);
                        assert!(pass.next(&mut bound, &program).unwrap().is_none());
                        frame::Reply::Done
                    }
                    frame::Request::Hierarchy { skin } => {
                        assert_eq!(*skin, 1);
                        bound
                            .skeleton
                            .hierarchy
                            .update(&mut bound.skeleton.skin, false)
                            .unwrap();
                        frame::Reply::Done
                    }
                    frame::Request::Geometry { model } => {
                        assert_eq!(*model, 1);
                        bound.refresh_geometry(false).unwrap();
                        frame::Reply::Done
                    }
                    other => panic!("unexpected service {other:?}"),
                };
                restore(&mut update);
                restore(&mut state);
                restore(&mut bound);
                update.acknowledge(&mut state, &request, reply).unwrap();
            }
            assert!(update.complete());
            assert_eq!(
                state,
                serde_json::from_value(step["frame"].clone()).unwrap(),
                "{action}"
            );
            assert_eq!(json!(calls), step["calls"]);
            compare(&bound, &step["bound"]);
            assert_eq!(json!(crt), step["bound"]["crt_state"]);
            frames += 1;
        }
    }
    assert_eq!(frames, 96);
}
