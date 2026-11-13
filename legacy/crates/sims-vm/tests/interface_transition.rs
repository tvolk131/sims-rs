use sims_vm::interface::{
    buy,
    mode_change::{self as mode, Change, Fields, Inputs, Notice, Request as R},
    tool,
};

fn fields(mode: i32) -> Fields {
    Fields {
        mode,
        dirty: 0x80040,
        build_tool: 3,
        buy_tool: 0,
        catalog: 7,
        subcatalog: 99,
        camera_enabled: 1,
        notify_pending: 1,
    }
}
fn inputs() -> Inputs {
    Inputs {
        application_clock: 42,
        object_tool_address: 0x1000,
        build_view_present: true,
        lot_mode: 0,
        lot_number: 7,
        zoning: 1,
        paused: 0,
        menu_present: false,
        menu_active: 0,
    }
}
fn trace(
    target: i32,
    force: u8,
    f: &mut Fields,
    input: &mut Inputs,
    mut callback: impl FnMut(&R, &mut Fields, &mut Inputs),
) -> Vec<R> {
    let mut call = Change::new(target, force);
    let mut seen = vec![];
    while let Some(request) = call.advance(f, *input).unwrap() {
        let bytes = serde_json::to_vec(&call).unwrap();
        let mut resumed: Change = serde_json::from_slice(&bytes).unwrap();
        resumed.validate().unwrap();
        let frozen = f.clone();
        assert_eq!(resumed.advance(f, *input).unwrap(), Some(request.clone()));
        assert_eq!(*f, frozen, "pending retry repeated writes");
        resumed.acknowledge(&request).unwrap();
        call.acknowledge(&request).unwrap();
        assert_eq!(call, resumed);
        callback(&request, f, input);
        seen.push(request);
        assert!(seen.len() < 30);
    }
    assert!(call.complete());
    let frozen = f.clone();
    assert_eq!(call.advance(f, *input).unwrap(), None);
    assert_eq!(*f, frozen);
    seen
}

#[test]
fn live_to_buy_publishes_after_old_tool_teardown_and_before_catalog_callbacks() {
    let mut f = fields(2);
    let events = trace(1, 0, &mut f, &mut inputs(), |r, f, _| match r {
        R::ClearTool => assert_eq!(f.mode, 2),
        R::ResetBuySelection => {
            assert_eq!(f.mode, 1);
            assert_eq!((f.catalog, f.subcatalog), (1, 0));
        }
        R::Notify => {
            assert_eq!(f.dirty, 0x80040);
            f.dirty = 0x40;
            f.notify_pending = 255;
        }
        _ => (),
    });
    assert_eq!(
        events,
        [
            R::ClearTool,
            R::ResetBuySelection,
            R::SelectBuyTool(0),
            R::WindowPause {
                paused: 1,
                show_overlay: 1
            },
            R::Notice(Notice::Buy),
            R::Notify
        ]
    );
    assert_eq!(f.dirty, 0xfff7ffff);
    assert_eq!(
        f.notify_pending, 0,
        "notification callback does not retain a new pending byte"
    );
}

#[test]
fn return_to_live_keeps_lifecycle_services_and_reads_clock_after_refresh() {
    let mut f = fields(0);
    let events = trace(2, 0, &mut f, &mut inputs(), |r, f, i| {
        if matches!(
            r,
            R::RefreshHouse
                | R::RefreshObjects
                | R::TimedResourceUpdates(_)
                | R::ReconcileLotControllers
                | R::CloseObjectTool(_)
                | R::ClearBuildCache
        ) {
            assert_eq!(f.mode, 0);
        }
        if *r == R::RefreshObjects {
            i.application_clock = 900;
        }
        if *r == R::SelectLiveTool {
            i.paused = 128;
            assert_eq!(f.mode, 2);
        }
    });
    assert_eq!(
        events,
        [
            R::RefreshHouse,
            R::RefreshObjects,
            R::TimedResourceUpdates(900),
            R::ReconcileLotControllers,
            R::CloseObjectTool(0x1008),
            R::ClearBuildCache,
            R::SelectLiveTool,
            R::WindowPause {
                paused: 128,
                show_overlay: 0
            },
            R::Notice(Notice::Live),
            R::Notify
        ]
    );
    assert_eq!(f.dirty, u32::MAX);
}

#[test]
fn callback_writes_are_reread_at_original_gates() {
    let mut f = fields(2);
    let events = trace(1, 0, &mut f, &mut inputs(), |r, f, i| {
        if *r == R::ClearTool {
            f.mode = 0;
        }
        if *r == R::ClearBuildCache {
            i.lot_mode = 1;
            i.lot_number = 40;
        }
        if *r == R::ResetBuySelection {
            f.buy_tool = 3;
        }
    });
    assert_eq!(
        &events[0..5],
        &[
            R::ClearTool,
            R::CloseObjectTool(0x1008),
            R::ClearBuildCache,
            R::ResetBuySelection,
            R::SelectBuyTool(3)
        ]
    );
    assert_eq!(f.catalog, 3);

    let mut f = fields(1);
    let events = trace(2, 0, &mut f, &mut inputs(), |r, f, i| {
        if *r == R::RefreshHouse {
            f.mode = 2;
        }
        if matches!(r, R::WindowPause { .. }) {
            i.menu_present = true;
        }
    });
    assert!(
        events.contains(&R::RefreshObjects),
        "entry decision survives callback mode change"
    );
    assert!(
        !events
            .iter()
            .any(|r| matches!(r, R::CloseObjectTool(_) | R::Notice(_)))
    );
    assert_eq!(events.last(), Some(&R::Notify));
}

#[test]
fn equal_modes_force_camera_and_unknown_modes_keep_native_branches() {
    for value in [-3, 0, 1, 2, 3, 4, i32::MAX] {
        let mut f = fields(value);
        let before = f.clone();
        assert!(trace(value, 0, &mut f, &mut inputs(), |_, _, _| {}).is_empty());
        assert_eq!(f, before);
    }
    let mut f = fields(4);
    let events = trace(4, 255, &mut f, &mut inputs(), |_, f, _| {
        assert_eq!(f.camera_enabled, 0)
    });
    assert_eq!(
        events,
        [
            R::CloseObjectTool(0x1008),
            R::ClearTool,
            R::ExpandPanel,
            R::WindowPause {
                paused: 1,
                show_overlay: 1
            },
            R::Notice(Notice::Camera),
            R::Notify
        ]
    );
    let mut f = fields(2);
    let events = trace(-7, 0, &mut f, &mut inputs(), |_, _, _| {});
    assert_eq!(
        events,
        [
            R::ClearTool,
            R::WindowPause {
                paused: 1,
                show_overlay: 0
            },
            R::Notice(Notice::Options),
            R::Notify
        ]
    );
    assert_eq!(f.mode, -7);
}

#[test]
fn null_minus_eight_and_callback_arguments_survive_all_mode_pairs() {
    for old in [0, 1, 2, 3, 4, -1] {
        for target in [0, 1, 2, 3, 4, -1] {
            for address in [0, u32::MAX - 7, 0x1000] {
                let mut f = fields(old);
                let mut i = Inputs {
                    object_tool_address: address,
                    ..inputs()
                };
                let events = trace(target, 1, &mut f, &mut i, |_, _, _| {});
                assert_eq!(
                    events.iter().any(|r| matches!(r, R::CloseObjectTool(_))),
                    old != 2 && address == 0x1000
                );
                assert_eq!(f.mode, target);
                assert_eq!(f.dirty, u32::MAX);
                assert_eq!(f.camera_enabled, u8::from(old != 4));
            }
        }
    }
}

#[test]
fn expansion_catalog_precedence_uses_signed_lot_numbers_and_exact_zoning() {
    let cases = [
        (-32768, 1),
        (-1, 1),
        (0, 1),
        (20, 1),
        (21, 2),
        (30, 2),
        (31, 1),
        (39, 1),
        (40, 3),
        (49, 3),
        (50, 1),
        (80, 1),
        (81, 5),
        (89, 5),
        (90, 1),
        (92, 1),
        (93, 6),
        (98, 6),
        (99, 1),
        (32767, 1),
    ];
    for (lot, expected) in cases {
        for zoning in [0, 1, 3, -1] {
            assert_eq!(mode::catalog(1, lot, zoning), expected);
        }
        for lot_mode in [-1, 0, 2] {
            assert_eq!(mode::catalog(lot_mode, lot, 2), 1);
        }
    }
    for lot in [-1, 7, 31, 50, 81, 89, 90, 92, 99] {
        assert_eq!(mode::catalog(1, lot, 2), 4);
    }
    assert_eq!(mode::catalog(1, 21, 2), 2);
    assert_eq!(mode::catalog(1, 40, 2), 3);
    assert_eq!(mode::catalog(1, 93, 2), 6);
}

#[test]
fn incorrect_service_or_saved_stage_does_not_acknowledge_transition() {
    let mut f = fields(2);
    let mut call = Change::new(1, 0);
    assert_eq!(call.advance(&mut f, inputs()).unwrap(), Some(R::ClearTool));
    let before = call.clone();
    assert!(call.acknowledge(&R::ResetBuySelection).is_err());
    assert_eq!(call, before);
    let mut raw = serde_json::to_value(&call).unwrap();
    raw["stage"] = serde_json::json!("Complete");
    let invalid: Change = serde_json::from_value(raw).unwrap();
    assert!(invalid.validate().is_err());
}

#[test]
fn indexed_tool_activates_twice_and_deactivates_post_callback_old_owner() {
    let mut f = tool::Fields {
        active: Some(7),
        argument: 99,
    };
    let mut call = tool::Change::select(6, u32::MAX);
    let mut trace = vec![];
    while let Some(r) = call.advance(&mut f).unwrap() {
        let saved = serde_json::to_vec(&call).unwrap();
        call = serde_json::from_slice(&saved).unwrap();
        call.validate().unwrap();
        assert_eq!(call.advance(&mut f).unwrap(), Some(r.clone()));
        let reply = match r {
            tool::Request::Lookup(6) => tool::Reply::Tool(11),
            tool::Request::Activate {
                tool: 11,
                argument: u32::MAX,
            } => {
                if trace.len() == 1 {
                    assert_eq!(f.active, Some(7));
                    f.active = Some(19);
                } else {
                    assert_eq!(f.active, Some(11));
                    assert_eq!(f.argument, u32::MAX);
                }
                tool::Reply::Done
            }
            tool::Request::Deactivate(19) => {
                assert_eq!(f.active, Some(19));
                f.argument = 55;
                tool::Reply::Done
            }
            _ => panic!("unexpected tool callback {r:?}"),
        };
        call.acknowledge(&r, reply).unwrap();
        trace.push(r);
    }
    assert!(call.complete());
    assert_eq!(trace.len(), 4);
    assert_eq!(trace[1], trace[3]);
    assert_eq!(
        f,
        tool::Fields {
            active: Some(11),
            argument: u32::MAX
        }
    );
}

#[test]
fn tool_same_owner_is_not_an_early_return_and_bad_replies_preserve_wait() {
    let mut f = tool::Fields {
        active: Some(7),
        argument: 0,
    };
    let mut call = tool::Change::replace(Some(7), 91).unwrap();
    assert_eq!(
        call.advance(&mut f).unwrap(),
        Some(tool::Request::Deactivate(7))
    );
    call.acknowledge(&tool::Request::Deactivate(7), tool::Reply::Done)
        .unwrap();
    let r = tool::Request::Activate {
        tool: 7,
        argument: 91,
    };
    assert_eq!(call.advance(&mut f).unwrap(), Some(r.clone()));
    let before = call.clone();
    assert!(call.acknowledge(&r, tool::Reply::Tool(0)).is_err());
    assert_eq!(before, call);
    call.acknowledge(&r, tool::Reply::Done).unwrap();
    assert!(call.complete());
    let mut call = tool::Change::select(7, 0);
    let r = call.advance(&mut f).unwrap().unwrap();
    let before = call.clone();
    assert!(call.acknowledge(&r, tool::Reply::Tool(0)).is_err());
    assert_eq!(call, before);
}

fn buy_fields() -> buy::Fields {
    buy::Fields {
        dirty: 3,
        catalog: 1,
        subcatalog: 1,
        indices: [[2, 3]; 7],
        cells: (100..164).collect(),
        selected_table: 77,
        selected_object: 78,
        selection_aux: 79,
    }
}
#[test]
fn buy_selection_keeps_unknown_catalog_and_fault_prefix_semantics() {
    for catalog in [-1, 0, 1, 2, 3, 4, 5, 6, 7] {
        let mut f = buy::Fields {
            catalog,
            ..buy_fields()
        };
        f.reset_selection().unwrap();
        assert_eq!(
            f.selected_table,
            if (0..=6).contains(&catalog) { 119 } else { 77 }
        );
        assert_eq!((f.selected_object, f.selection_aux, f.dirty), (0, 0, 0x403));
    }
    let mut f = buy_fields();
    f.indices[1] = [-1, 0];
    assert!(f.reset_selection().is_err());
    assert_eq!(
        (
            f.selected_table,
            f.selected_object,
            f.selection_aux,
            f.dirty
        ),
        (77, 0, 0, 3)
    );
    f.indices[1] = [0x20000000, 0];
    f.reset_selection().unwrap();
    assert_eq!(
        f.selected_table, 100,
        "native LEA/multiply arithmetic wraps"
    );
}

#[test]
fn repeat_buy_button_only_cycles_catalogs_zero_one_two_but_always_clears_cache() {
    for catalog in -1..=7 {
        let mut f = buy::Fields {
            catalog,
            ..buy_fields()
        };
        f.repeat_buy_button().unwrap();
        assert!(f.cells.iter().all(|n| *n == 0));
        let expected = match catalog {
            0 => 1,
            1 => 0,
            other => other,
        };
        assert_eq!(f.catalog, expected);
        if (0..=2).contains(&catalog) {
            assert_eq!(
                (
                    f.subcatalog,
                    f.selected_table,
                    f.selected_object,
                    f.selection_aux,
                    f.dirty
                ),
                (0, 0, 0, 0, 0x403)
            );
        } else {
            assert_eq!(
                (
                    f.subcatalog,
                    f.selected_table,
                    f.selected_object,
                    f.selection_aux,
                    f.dirty
                ),
                (1, 77, 78, 79, 3)
            );
        }
    }
}

#[test]
#[ignore = "private original4cea5d corpus; generate only with no Cargo/rustc processes"]
fn transition_matches_original_mode_change_instructions() {
    use serde_json::{Value, json};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/mode-transition.json");
    let corpus: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        corpus["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (index, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
        let mut input: Inputs = serde_json::from_value(c["input"].clone()).unwrap();
        let mut events = vec![];
        trace(
            c["target"].as_i64().unwrap() as i32,
            c["force"].as_u64().unwrap() as u8,
            &mut f,
            &mut input,
            |r, fields, input| {
                events.push(json!({"request":r,"fields":fields}));
                let request = serde_json::to_value(r).unwrap();
                let name = request
                    .as_str()
                    .unwrap_or_else(|| request.as_object().unwrap().keys().next().unwrap());
                if let Some(mutation) = c["mutations"].get(name) {
                    if let Some(changes) = mutation.get("fields") {
                        let mut raw = serde_json::to_value(&*fields).unwrap();
                        raw.as_object_mut()
                            .unwrap()
                            .extend(changes.as_object().unwrap().clone());
                        *fields = serde_json::from_value(raw).unwrap();
                    }
                    if let Some(changes) = mutation.get("input") {
                        let mut raw = serde_json::to_value(&*input).unwrap();
                        raw.as_object_mut()
                            .unwrap()
                            .extend(changes.as_object().unwrap().clone());
                        *input = serde_json::from_value(raw).unwrap();
                    }
                }
            },
        );
        assert_eq!(
            json!({"events":events,"fields":f}),
            row["original"],
            "native case{index}: {c}"
        );
    }
}
