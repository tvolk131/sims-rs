use serde_json::{Value, json};
use sims_formats::{
    animation::{Animation, Motion},
    avatar::{Bone, Skeleton},
};
use sims_sim::pose::{
    animator::{Animator, Event, Settings, Slot},
    events::Entry,
    properties::{Context, Handler, Request},
    update::Dispatch,
};
use std::collections::BTreeMap;
fn resources() -> (Skeleton, Animation) {
    (
        Skeleton {
            name: "synthetic".into(),
            bones: vec![Bone {
                name: "bone".into(),
                parent: None,
                translation: [0.; 3],
                rotation: [0., 0., 0., 1.],
                properties: vec![],
                flags: [0; 3],
                wiggle: Default::default(),
            }],
        },
        Animation {
            name: "synthetic".into(),
            skill: "synthetic".into(),
            duration_ms: 1000.,
            distance: 0.,
            moving: false,
            translation_count: 0,
            rotation_count: 0,
            frames: 30,
            events: vec![],
            motions: vec![Motion {
                bone: "bone".into(),
                frames: 30,
                duration_ms: 1000.,
                translation: None,
                rotation: None,
                properties: vec![],
                event_lists: vec![],
            }],
        },
    )
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_property_effects_services_and_fifo_preserve_order_across_restores() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-property-dispatch.json")).unwrap();
    let (rig, clip) = resources();
    let settings = Settings {
        fade_enabled: true,
        fade_duration: 200,
        sampling_config: 1,
    };
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1200);
    for (i, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let context: Context = serde_json::from_value(c["context"].clone()).unwrap();
        let mut person: BTreeMap<i16, i16> = serde_json::from_value(c["person"].clone()).unwrap();
        let mut animator = Animator::new(123, 0, 0, true, 7);
        for slot in [Slot::Foreground, Slot::Carry, Slot::Base] {
            animator
                .start(slot, &rig, Some(&clip), false, 123, settings)
                .unwrap();
        }
        if c["blend"].as_bool().unwrap() {
            animator
                .start(Slot::Foreground, &rig, Some(&clip), false, 123, settings)
                .unwrap();
        }
        let id = animator
            .slot(serde_json::from_value(c["slot"].clone()).unwrap())
            .unwrap();
        let mut state = serde_json::to_value(&animator).unwrap();
        state["anchor_valid"] = c["anchor_valid"].clone();
        state["anchor_bone"] = c["anchor_bone"].clone();
        let layer = state["layers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["id"] == id.0)
            .unwrap();
        layer["anchor_bone"] = c["layer_anchor"].clone();
        layer["anchor_state"] = json!(17);
        layer["layer"]["clock"]["speed"] = c["speed"].clone();
        animator = serde_json::from_value(state).unwrap();
        let mut handler = Handler::new(Event {
            layer: id,
            dispatch: Dispatch {
                binding: 0,
                bone: c["bone"].as_u64().unwrap() as usize,
                motion: 0,
                event: 0,
                phase: 0.,
                entry: Entry {
                    time: 0,
                    properties: serde_json::from_value(c["properties"].clone()).unwrap(),
                },
            },
        });
        let mut actions = vec![];
        while let Some(request) = handler
            .next(&mut animator, &mut person, context)
            .unwrap_or_else(|e| panic!("case {i}: {e}"))
        {
            (animator, person, handler) = serde_json::from_slice(
                &serde_json::to_vec(&(&animator, &person, &handler)).unwrap(),
            )
            .unwrap();
            assert_eq!(
                handler.next(&mut animator, &mut person, context).unwrap(),
                Some(request.clone()),
                "pending {i}"
            );
            let result = matches!(request, Request::FootstepSurface { .. })
                .then(|| c["surface"].as_i64().unwrap() as i32);
            actions.push(request);
            handler.acknowledge(result).unwrap();
            (animator, person, handler) = serde_json::from_slice(
                &serde_json::to_vec(&(&animator, &person, &handler)).unwrap(),
            )
            .unwrap();
        }
        assert!(handler.complete());
        let state = serde_json::to_value(&animator).unwrap();
        let layer = state["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id.0)
            .unwrap();
        let released = if c["blend"].as_bool().unwrap() && state["blend"].is_null() {
            vec![1]
        } else {
            vec![]
        };
        let actual = json!({"person":person,"actions":actions,"interruptible":layer["layer"]["interruptible"],"layer_anchor":layer["anchor_bone"],"anchor_state":layer["anchor_state"],"anchor_bone":state["anchor_bone"],"anchor_valid":state["anchor_valid"],"numbered_events":state["numbered_events"],"blend":state["blend"],"promote":state["promote_next_foreground"],"released":released});
        assert_eq!(actual, row["original"], "native case {i}: {c}");
        let mut events = vec![];
        while let Some(value) = animator.pop_numbered_event() {
            events.push(value);
        }
        assert_eq!(
            json!(events),
            row["original"]["numbered_events"],
            "FIFO {i}"
        );
    }
}
