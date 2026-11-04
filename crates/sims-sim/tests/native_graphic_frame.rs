use serde_json::{Value, json};
use sims_sim::pose::{graphic_appearance::Model, graphic_frame::*};

fn round_trip(update: &mut Update, state: &mut State) {
    let original = (update.clone(), state.clone());
    (*update, *state) = serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!((update.clone(), state.clone()), original);
    update.validate().unwrap();
}
fn model(id: u64) -> Option<Model> {
    (id != 0).then_some(Model {
        model: id,
        skin: id,
    })
}
fn merge(target: &mut Value, patch: &Value) {
    if let Some(fields) = patch.as_object() {
        for (key, value) in fields {
            if value.is_object() {
                merge(&mut target[key], value);
            } else {
                target[key] = value.clone();
            }
        }
    } else {
        *target = patch.clone();
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_base_graphic_frames_retain_captures_rereads_and_service_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-frame.json")).unwrap();
    for (case_index, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut update = Update::new(
            serde_json::from_value(c["entry"].clone()).unwrap(),
            c["only_visible"].as_bool().unwrap(),
        );
        let tick = c["tick"].as_i64().unwrap() as i32;
        let mut model_id = c["model"].as_u64().unwrap();
        let mut viewport: [i32; 4] = serde_json::from_value(c["viewport"].clone()).unwrap();
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        let mut events = Vec::new();
        let mut applied = Vec::new();
        loop {
            round_trip(&mut update, &mut state);
            let Some(request) = update.advance(&mut state, tick, model(model_id)).unwrap() else {
                break;
            };
            round_trip(&mut update, &mut state);
            let before = (update.clone(), state.clone());
            assert_eq!(
                update
                    .advance(&mut state, tick.wrapping_add(100), None)
                    .unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone()), before);
            let wrong = if request == Request::Viewport {
                Reply::Done
            } else {
                Reply::Viewport([9; 4])
            };
            assert!(update.acknowledge(&mut state, &request, wrong).is_err());
            assert_eq!((update.clone(), state.clone()), before);
            let (name, event, reply) = match &request {
                Request::ReleaseBlend { model, layer } => {
                    ("release", json!({"model":model,"layer":layer}), Reply::Done)
                }
                Request::OutsideLot => (
                    "outside",
                    json!({}),
                    Reply::Outside(c["outside"].as_bool().unwrap()),
                ),
                Request::Placement => ("placement", json!({}), Reply::Done),
                Request::Viewport => ("viewport", json!({}), Reply::Viewport(viewport)),
                Request::Visibility { viewport } => {
                    let n = counts.get("visibility").copied().unwrap_or(0).min(1);
                    let visible = c["visibility"][n].as_bool().unwrap();
                    state.visible = u8::from(visible);
                    (
                        "visibility",
                        json!({"viewport":viewport}),
                        Reply::Visible(visible),
                    )
                }
                Request::Model {
                    model,
                    matrix,
                    now,
                    visible,
                } => (
                    "model",
                    json!({"model":model,"matrix":matrix,"now":now,"visible":visible}),
                    Reply::Done,
                ),
                Request::Invalidate => ("invalidate", json!({}), Reply::Done),
                Request::Skin { skin, now, sample } => (
                    "skin",
                    json!({"skin":skin,"now":now,"sample":sample}),
                    Reply::Done,
                ),
                Request::Hierarchy { skin } => ("hierarchy", json!({"skin":skin}), Reply::Done),
                Request::Geometry { model } => ("geometry", json!({"model":model}), Reply::Done),
            };
            let mut event = event;
            event["service"] = json!(name);
            events.push(event);
            let count = counts.entry(name).or_default();
            for (i, mutation) in c["mutations"].as_array().unwrap().iter().enumerate() {
                if mutation["after"] != name
                    || mutation["occurrence"].as_u64().unwrap_or(0) != *count as u64
                {
                    continue;
                }
                applied.push(i);
                if let Some(changes) = mutation.get("state") {
                    let mut value = serde_json::to_value(&state).unwrap();
                    merge(&mut value, changes);
                    state = serde_json::from_value(value).unwrap();
                }
                if let Some(id) = mutation["model"].as_u64() {
                    model_id = id;
                }
                if let Some(value) = mutation.get("viewport") {
                    viewport = serde_json::from_value(value.clone()).unwrap();
                }
            }
            *count += 1;
            // Restore after effects but before ack: polling must not repeat
            // any boundary, including model/layer destruction or callbacks.
            round_trip(&mut update, &mut state);
            let before_ack = (update.clone(), state.clone());
            assert_eq!(
                update.advance(&mut state, tick, model(model_id)).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone()), before_ack);
            let reply = if request == Request::Viewport {
                Reply::Viewport(viewport)
            } else {
                reply
            };
            update
                .acknowledge(&mut state, &request, reply.clone())
                .unwrap();
            let after_ack = (update.clone(), state.clone());
            assert!(update.acknowledge(&mut state, &request, reply).is_err());
            assert_eq!((update.clone(), state.clone()), after_ack);
            assert!(events.len() < 20);
        }
        assert!(update.complete());
        assert_eq!(json!(events), expected["events"], "case {case_index}: {c}");
        assert_eq!(json!(applied), expected["applied"], "case {case_index}");
        assert_eq!(
            state,
            serde_json::from_value::<State>(expected["state"].clone()).unwrap(),
            "case {case_index}: {c}"
        );
        assert_eq!(json!(model_id), expected["model"]);
        let done = (update.clone(), state.clone());
        assert!(
            update
                .advance(&mut state, tick, model(model_id))
                .unwrap()
                .is_none()
        );
        assert_eq!((update, state), done);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn base_graphic_frame_rejects_corrupt_captured_skin_and_stale_replies() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-frame.json")).unwrap();
    let mut state: State =
        serde_json::from_value(fixture["cases"][0]["input"]["state"].clone()).unwrap();
    let mut update = Update::new(Entry::Refresh, false);
    assert!(
        update
            .acknowledge(&mut state, &Request::Invalidate, Reply::Done)
            .is_err()
    );
    let mut value = serde_json::to_value(update).unwrap();
    value["stage"] = json!("Hierarchy");
    value["skin"] = json!(0);
    let restored: Update = serde_json::from_value(value).unwrap();
    assert!(restored.validate().is_err());
}
