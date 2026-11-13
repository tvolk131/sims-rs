use sims_vm::interface::{
    buy::definition::{Reply, Request, Selection},
    tool::Fields,
};

#[test]
fn definition_selection_captures_ordinal_before_reset_and_publishes_final_classification() {
    for answer in [0, (-20000_i32) as u32, u32::MAX] {
        let mut f = Fields {
            active: Some(10),
            argument: 20,
        };
        let mut c = Selection::new(0xdeadbeef);
        let requests = [
            (Request::Find(0xdeadbeef), Reply::Number(30)),
            (Request::LookupTool, Reply::Number(40)),
            (Request::Ordinal(30), Reply::Number(50)),
            (Request::Reset(40), Reply::Done),
            (
                Request::Replace {
                    tool: 40,
                    argument: 50,
                },
                Reply::Done,
            ),
            (
                Request::Classify {
                    tool: 40,
                    definition: 30,
                },
                Reply::Number(answer),
            ),
        ];
        for (n, (r, reply)) in requests.into_iter().enumerate() {
            assert_eq!(c.advance(&mut f).unwrap(), Some(r));
            assert_eq!(f.argument, if n == 0 { 20 } else { 99 });
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
            let before = f.clone();
            assert_eq!(c.advance(&mut f).unwrap(), Some(r));
            assert_eq!(f, before);
            let before = c.clone();
            assert!(
                c.acknowledge(
                    r,
                    if matches!(reply, Reply::Done) {
                        Reply::Number(0)
                    } else {
                        Reply::Done
                    }
                )
                .is_err()
            );
            assert_eq!(c, before);
            // Callback writes to shared fields do not replace captured owners/ordinal.
            f.active = Some(77);
            f.argument = 99;
            c.acknowledge(r, reply).unwrap();
            c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        }
        assert_eq!(c.advance(&mut f).unwrap(), None);
        assert!(c.complete());
        assert_eq!(
            f,
            Fields {
                active: Some(77),
                argument: if answer == 0 { 50 } else { answer }
            }
        );
    }
}

#[test]
fn missing_definition_uses_default_tool_but_missing_tool_is_an_explicit_failure() {
    for definition in [0, 30] {
        let mut f = Fields {
            active: None,
            argument: 7,
        };
        let mut c = Selection::new(42);
        let r = Request::Find(42);
        assert_eq!(c.advance(&mut f).unwrap(), Some(r));
        c.acknowledge(r, Reply::Number(definition)).unwrap();
        let r = if definition == 0 {
            Request::SelectDefault
        } else {
            Request::LookupTool
        };
        assert_eq!(c.advance(&mut f).unwrap(), Some(r));
        c.acknowledge(
            r,
            if definition == 0 {
                Reply::Done
            } else {
                Reply::Number(0)
            },
        )
        .unwrap();
        if definition == 0 {
            assert_eq!(c.advance(&mut f).unwrap(), None);
            assert!(c.complete());
        } else {
            assert!(c.advance(&mut f).is_err());
            assert!(!c.complete());
        }
        assert_eq!(
            f,
            Fields {
                active: None,
                argument: 7
            }
        );
    }
}

#[test]
fn corrupted_selected_definition_request_does_not_mutate_shared_fields() {
    let mut f = Fields {
        active: Some(4),
        argument: 5,
    };
    let before = f.clone();
    let mut c = Selection::new(42);
    c.advance(&mut f).unwrap();
    let mut saved = serde_json::to_value(c).unwrap();
    saved["pending"]["Find"] = serde_json::json!(43);
    let mut c: Selection = serde_json::from_value(saved).unwrap();
    assert!(c.advance(&mut f).is_err());
    assert_eq!(f, before);
}
