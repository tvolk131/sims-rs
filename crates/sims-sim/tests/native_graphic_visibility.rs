use serde_json::{Value, json};
use sims_sim::pose::{
    graphic_frame::State, graphic_transform::Placement, graphic_visibility::*, model::Matrix,
};

fn snapshot(state: &State, dirty: u8, model: Option<u64>) -> Value {
    json!({"model":model,"dirty":dirty,"visible":state.visible,"matrix":state.matrix,"base":state.placement.base,"offset":state.placement.offset})
}
fn restore(update: &mut Update, state: &mut State) {
    let before = (update.clone(), state.clone());
    (*update, *state) = serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
    assert_eq!((update.clone(), state.clone()), before);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_visibility_matches_original_gates_and_live_reads_after_camera() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-visibility.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut dirty = c["state"]["dirty"].as_u64().unwrap() as u8;
        let mut model = c["state"]["model"].as_u64();
        let mut state = State {
            clock_mode: 0,
            now: 0,
            visible: c["state"]["visible"].as_u64().unwrap() as u8,
            blend: None,
            placement: Placement {
                base: serde_json::from_value(c["state"]["base"].clone()).unwrap(),
                offset: serde_json::from_value(c["state"]["offset"].clone()).unwrap(),
                dirty: dirty != 0,
                explicit: false,
            },
            matrix: serde_json::from_value(c["state"]["matrix"].clone()).unwrap(),
        };
        let mut update = Update::new(serde_json::from_value(c["viewport"].clone()).unwrap());
        let mut calls = vec![];
        loop {
            restore(&mut update, &mut state);
            let Some(request) = update
                .advance(&mut state, dirty, c["enabled"].as_bool().unwrap(), model)
                .unwrap()
            else {
                break;
            };
            let before = (update.clone(), state.clone());
            assert_eq!(
                update.advance(&mut state, 255, false, None).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone()), before);
            let reply = match &request {
                Request::Camera => {
                    calls.push(json!({"service":"camera","state":snapshot(&state,dirty,model)}));
                    let m = &c["mutation"];
                    if let Some(v) = m.get("model") {
                        model = v.as_u64();
                    }
                    if let Some(v) = m.get("dirty") {
                        dirty = v.as_u64().unwrap() as u8;
                    }
                    if let Some(v) = m.get("visible") {
                        state.visible = v.as_u64().unwrap() as u8;
                    }
                    if let Some(v) = m.get("base") {
                        state.placement.base = serde_json::from_value(v.clone()).unwrap();
                    }
                    if let Some(v) = m.get("offset") {
                        state.placement.offset = serde_json::from_value(v.clone()).unwrap();
                    }
                    if let Some(v) = m.get("matrix") {
                        state.matrix = serde_json::from_value(v.clone()).unwrap();
                    }
                    Reply::Camera(serde_json::from_value(c["camera"].clone()).unwrap())
                }
                Request::Project {
                    model,
                    matrix,
                    camera,
                    viewport,
                } => {
                    calls.push(json!({"service":"project","model":model,"matrix":matrix,"camera":camera,"viewport":viewport}));
                    Reply::Visible(c["result"].as_u64().unwrap() as u8)
                }
            };
            restore(&mut update, &mut state);
            update.acknowledge(&mut state, &request, reply).unwrap();
            assert!(
                update
                    .acknowledge(&mut state, &request, Reply::Visible(0))
                    .is_err()
            );
        }
        assert!(update.complete());
        assert_eq!(snapshot(&state, dirty, model), row["original"]["state"]);
        assert_eq!(json!(calls), row["original"]["calls"]);
        assert_eq!(json!(state.visible), row["original"]["result"]);
    }
}

#[test]
fn missing_model_after_camera_remains_an_unacknowledged_error() {
    let mut state = State {
        clock_mode: 0,
        now: 0,
        visible: 9,
        blend: None,
        placement: Placement {
            base: [0.; 3],
            offset: [0.; 3],
            dirty: false,
            explicit: false,
        },
        matrix: Matrix::IDENTITY,
    };
    let mut update = Update::new([0, 0, 800, 600]);
    assert_eq!(
        update.advance(&mut state, 0, true, Some(1)).unwrap(),
        Some(Request::Camera)
    );
    update
        .acknowledge(
            &mut state,
            &Request::Camera,
            Reply::Camera(Matrix::IDENTITY),
        )
        .unwrap();
    let before = (update.clone(), state.clone());
    assert!(update.advance(&mut state, 0, true, None).is_err());
    assert_eq!((update, state), before);
}
