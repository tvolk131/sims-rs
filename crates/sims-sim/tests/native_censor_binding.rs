//! Native synchronization/selection corpus. Mesh callback outcomes are supplied
//! in both implementations; this does not compare drawable or pixel rendering.
use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::pose::{
    appearance::binding_selected,
    refresh::{Censor, Request},
};

#[derive(Deserialize)]
struct Binding {
    flags: u32,
    bone: i32,
    success: bool,
}
#[derive(Deserialize)]
struct Input {
    bindings: Vec<Binding>,
    bones: Vec<bool>,
    callback: bool,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    steps: Vec<Value>,
}
#[derive(Deserialize)]
struct Report {
    capture_sha256: String,
    cases: Vec<Case>,
    direct: Vec<Value>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_mask_selection_empty_instances_and_replacement_order() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/native-censor-binding.json")).unwrap();
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report.cases.len(), 65);
    let mut steps = 0;
    for case in report.cases {
        let mut state = Censor {
            binding: None,
            mask: 0,
        };
        let mut next = 1;
        let mut selected = vec![];
        for expected in case.steps {
            let raw = expected["raw"].as_i64().unwrap() as i16;
            let mask = Censor::normalized_mask(raw);
            let mut events = vec![];
            while let Some(request) = state.next_request(mask) {
                // Restore each pending host boundary without consuming it.
                let restored: Censor =
                    serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
                assert_eq!(restored.next_request(mask), Some(request.clone()));
                match restored.next_request(mask).unwrap() {
                    Request::UnbindCensor { binding } => {
                        events.push(json!({"op":"destroy","instance":binding}));
                        state = Censor {
                            binding: None,
                            mask: 0,
                        };
                        selected.clear();
                    }
                    Request::BindCensor { mask } => {
                        events.push(json!({"op":"allocate","instance":next}));
                        for (index, b) in case.input.bindings.iter().enumerate() {
                            if binding_selected(b.flags, mask as u32)
                                && case.input.bones.get(b.bone as usize) == Some(&true)
                                && case.input.callback
                            {
                                events.push(json!({"op":"mesh","index":index,"success":b.success}));
                                if b.success {
                                    selected.push(index);
                                }
                            }
                        }
                        state = Censor {
                            binding: Some(next),
                            mask,
                        };
                        next += 1;
                    }
                    other => panic!("unexpected censor request: {other:?}"),
                }
            }
            assert_eq!(
                json!({"raw":raw,"mask":state.mask,"instance":state.binding,
                "selected":selected,"events":events}),
                expected
            );
            steps += 1;
        }
    }
    assert_eq!(steps, 452);
    assert_eq!(report.direct.len(), 35);
    for row in report.direct {
        let input: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let mask = row["input"]["mask"].as_u64().unwrap() as u32;
        let mut events = vec![json!({"op":"allocate","instance":1})];
        for (index, binding) in input.bindings.iter().enumerate() {
            if binding_selected(binding.flags, mask)
                && input.bones.get(binding.bone as usize) == Some(&true)
                && input.callback
            {
                events.push(json!({"op":"mesh","index":index,"success":binding.success}));
            }
        }
        assert_eq!(json!(events), row["events"]);
    }
}
