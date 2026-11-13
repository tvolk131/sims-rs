use sims_vm::interface::panel::{self, Fields, Inputs, Request, Setter};

#[test]
fn corrected_panel_name_preserves_historical_save_encoding() {
    use sims_vm::ui_owner::wait::Condition;
    assert_eq!(
        serde_json::to_string(&Condition::PanelExpanded).unwrap(),
        "\"ToolIdle\""
    );
    assert_eq!(
        serde_json::from_str::<Condition>("\"ToolIdle\"").unwrap(),
        Condition::PanelExpanded
    );
}

#[test]
fn setter_preserves_original_callback_order_and_resumes() {
    for value in [-1, 0, 1, 2] {
        for force in [0, 1, 255] {
            for old in [-1, 0, 1, 2] {
                let mut fields = Fields {
                    value: old,
                    flags: 0x81234567,
                };
                let mut call = Setter::new(value, force);
                let input = Inputs {
                    app_present: true,
                    window_present: true,
                };
                let mut events = vec![];
                while let Some(request) = call.advance(&mut fields, input).unwrap() {
                    let bytes = serde_json::to_vec(&call).unwrap();
                    let mut resumed: Setter = serde_json::from_slice(&bytes).unwrap();
                    let before = fields.clone();
                    assert_eq!(
                        resumed.advance(&mut fields, input).unwrap(),
                        Some(request.clone())
                    );
                    assert_eq!(fields, before);
                    if request == Request::Whoosh {
                        assert_eq!(fields.value, old);
                        assert_eq!(fields.flags, 0x81234567);
                        // The native write occurs after the callback and ORs fresh flags.
                        fields.flags = 0x40000020;
                        fields.value = 77;
                    } else {
                        assert_eq!(fields.value, value);
                        assert_ne!(fields.flags & panel::DIRTY, 0);
                    }
                    let frozen = call.clone();
                    assert!(
                        call.acknowledge(&Request::WindowLayout(value != 1))
                            .is_err()
                    );
                    assert_eq!(call, frozen);
                    call.acknowledge(&request).unwrap();
                    resumed.acknowledge(&request).unwrap();
                    assert_eq!(call, resumed);
                    events.push(request);
                }
                assert!(call.complete());
                if old == value && force == 0 {
                    assert!(events.is_empty());
                } else {
                    assert_eq!(events.contains(&Request::Whoosh), value == 0 && force == 0);
                    assert_eq!(events.last(), Some(&Request::WindowLayout(value == 1)));
                    let original = if value == 0 && force == 0 {
                        0x40000020
                    } else {
                        0x81234567
                    };
                    assert_eq!(fields.flags, original | panel::DIRTY);
                }
            }
        }
    }
}

#[test]
fn window_presence_is_read_after_whoosh_and_no_change_does_not_notify() {
    let mut fields = Fields::constructed(0);
    let mut call = Setter::new(0, 0);
    let absent = Inputs {
        app_present: false,
        window_present: true,
    };
    assert_eq!(
        call.advance(&mut fields, absent).unwrap(),
        Some(Request::Whoosh)
    );
    call.acknowledge(&Request::Whoosh).unwrap();
    assert_eq!(
        call.advance(
            &mut fields,
            Inputs {
                app_present: true,
                window_present: true
            }
        )
        .unwrap(),
        Some(Request::WindowLayout(false))
    );
    for input in [
        absent,
        Inputs {
            app_present: true,
            window_present: false,
        },
    ] {
        let mut fields = Fields::constructed(0);
        let mut call = Setter::new(0, 1);
        assert_eq!(call.advance(&mut fields, input).unwrap(), None);
        assert_eq!(fields.value, 0);
    }
    assert_eq!(panel::publication(&fields), Some(0));
    assert_eq!(panel::live_button_target(2, 1), Ok(0));
    assert_eq!(panel::live_button_target(2, 0), Ok(1));
    assert!(panel::live_button_target(3, 0).is_err());
}

#[test]
#[ignore = "private original4cfc75 corpus; generate only with no Cargo/rustc jobs"]
fn panel_setter_matches_original_instructions() {
    use serde_json::{Value, json};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/panel-state.json");
    let data: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        data["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in data["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut fields = Fields {
            value: c["old"].as_i64().unwrap() as i32,
            flags: c["flags"].as_u64().unwrap() as u32,
        };
        let mut call = Setter::new(
            c["value"].as_i64().unwrap() as i32,
            c["force"].as_u64().unwrap() as u8,
        );
        let mut input = Inputs {
            app_present: c["app"].as_bool().unwrap(),
            window_present: c["window"].as_bool().unwrap(),
        };
        let mut events = vec![];
        while let Some(request) = call.advance(&mut fields, input).unwrap() {
            events.push(json!({"request":request,"fields":fields}));
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            if request == Request::Whoosh && c["mutate"].as_bool().unwrap() {
                fields.flags = 0x40000020;
                fields.value = 77;
                input.app_present = !input.app_present;
                input.window_present = !input.window_present;
            }
            call.acknowledge(&request).unwrap();
        }
        assert_eq!(
            json!({"events":events,"fields":fields}),
            row["original"],
            "{c}"
        );
    }
}
