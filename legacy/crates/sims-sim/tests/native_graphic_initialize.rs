use serde_json::{Value, json};
use sims_sim::pose::graphic_initialize::*;
use std::collections::BTreeMap;

fn restore(update: &mut Update, state: &mut State, callbacks: &mut BTreeMap<u64, Option<u64>>) {
    let before = (update.clone(), state.clone(), callbacks.clone());
    (*update, *state, *callbacks) =
        serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
    assert_eq!((update.clone(), state.clone(), callbacks.clone()), before);
    update.validate().unwrap();
}
fn patch(state: &mut State, value: &Value) {
    if let Some(fields) = value.as_object() {
        let mut current = json!(state);
        for (key, value) in fields {
            current[key] = value.clone();
        }
        *state = serde_json::from_value(current).unwrap();
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_base_initializer_preserves_failure_and_callback_order() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-initialize.json")).unwrap();
    for (n, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut callbacks: BTreeMap<u64, Option<u64>> =
            serde_json::from_value(c["callbacks"].clone()).unwrap();
        let entry = match c["entry"].as_str().unwrap() {
            "Owner" => Entry::Owner(c["target_owner"].as_u64().unwrap()),
            "Profile" => Entry::Profile,
            "Model" => Entry::Model,
            _ => unreachable!(),
        };
        let mut update = Update::new(99, entry);
        let mut viewport = serde_json::from_value::<[i32; 4]>(c["viewport"].clone()).unwrap();
        let mut events = vec![];
        let mut applied = vec![];
        let mut bound = None;
        loop {
            restore(&mut update, &mut state, &mut callbacks);
            let Some(request) = update.advance(&mut state, &mut callbacks).unwrap() else {
                break;
            };
            let before = (update.clone(), state.clone(), callbacks.clone());
            assert_eq!(
                update.advance(&mut state, &mut callbacks).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone(), callbacks.clone()), before);
            let wrong = if matches!(request, Request::CameraViewport) {
                Reply::Done
            } else {
                Reply::Viewport([0; 4])
            };
            assert!(update.acknowledge(&mut state, &request, wrong).is_err());
            assert_eq!((update.clone(), state.clone(), callbacks.clone()), before);
            let (name, mut event, reply) = match &request {
                Request::ClearAppearances => ("clear", json!({}), Reply::Done),
                Request::Profile { owner } => (
                    "profile",
                    json!({"owner":owner}),
                    Reply::Profile(c["profile_result"].as_u64()),
                ),
                Request::ReloadAppearance { force } => {
                    assert!(!force);
                    ("appearance", json!({}), Reply::Done)
                }
                Request::DestroyAccessories => ("destroy", json!({}), Reply::Done),
                Request::Skeleton { profile } => (
                    "skeleton",
                    json!({"profile":profile}),
                    Reply::Skeleton(
                        c["skeletons"][*profile as usize - 1]
                            .as_str()
                            .unwrap()
                            .into(),
                    ),
                ),
                Request::ConstructModel => {
                    callbacks.insert(3, None);
                    ("construct", json!({}), Reply::Model(3))
                }
                Request::BindModel { model, skeleton } => {
                    let status = c["bind_status"].as_i64().unwrap() as i32;
                    bound = Some(status);
                    (
                        "bind",
                        json!({"model":model,"skeleton":skeleton}),
                        Reply::Bound(status),
                    )
                }
                Request::Reset => ("reset", json!({}), Reply::Done),
                Request::BindAccessories => ("accessories", json!({}), Reply::Done),
                Request::Placement => ("placement", json!({}), Reply::Done),
                Request::CameraViewport => ("viewport", json!({}), Reply::Viewport(viewport)),
                Request::Visibility { viewport } => {
                    ("visibility", json!({"viewport":viewport}), Reply::Done)
                }
            };
            event["service"] = json!(name);
            event["state"] = json!(state);
            events.push(event);
            for (i, m) in c["mutations"].as_array().unwrap().iter().enumerate() {
                if m["after"] != name {
                    continue;
                }
                applied.push(i);
                patch(&mut state, &m["state"]);
                if let Some(v) = m.get("viewport") {
                    viewport = serde_json::from_value(v.clone()).unwrap();
                }
                if let Some(values) = m["callbacks"].as_object() {
                    for (id, v) in values {
                        callbacks.insert(id.parse().unwrap(), v.as_u64());
                    }
                }
            }
            let reply = if request == Request::CameraViewport {
                Reply::Viewport(viewport)
            } else {
                reply
            };
            restore(&mut update, &mut state, &mut callbacks);
            let before = (update.clone(), state.clone(), callbacks.clone());
            assert_eq!(
                update.advance(&mut state, &mut callbacks).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), state.clone(), callbacks.clone()), before);
            update
                .acknowledge(&mut state, &request, reply.clone())
                .unwrap();
            let after = (update.clone(), state.clone());
            assert!(update.acknowledge(&mut state, &request, reply).is_err());
            assert_eq!((update.clone(), state.clone()), after);
        }
        assert!(update.complete());
        assert_eq!(update.bind_status(), bound);
        assert_eq!(json!(events), row["original"]["events"], "case {n}: {c}");
        assert_eq!(json!(applied), row["original"]["applied"], "case {n}");
        assert_eq!(json!(state), row["original"]["state"], "case {n}");
        assert_eq!(json!(callbacks), row["original"]["callbacks"], "case {n}");
    }
}

#[test]
fn base_initializer_rejects_missing_callback_storage_without_acknowledging_reset() {
    let mut state = State {
        owner: 1,
        model: None,
        profile: Some(1),
        flags: [1; 4],
        appearance_index: -1,
        selection: -1,
        foreground: None,
    };
    let mut callbacks = BTreeMap::new();
    let mut update = Update::new(99, Entry::Model);
    for reply in [Reply::Skeleton("".into()), Reply::Model(3), Reply::Bound(0)] {
        let request = update.advance(&mut state, &mut callbacks).unwrap().unwrap();
        update.acknowledge(&mut state, &request, reply).unwrap();
    }
    let before = (update.clone(), state.clone());
    assert!(update.advance(&mut state, &mut callbacks).is_err());
    assert_eq!((update.clone(), state.clone()), before);
    callbacks.insert(3, None);
    assert_eq!(
        update.advance(&mut state, &mut callbacks).unwrap(),
        Some(Request::Reset)
    );
    assert_eq!(callbacks[&3], Some(99));
}
