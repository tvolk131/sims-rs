use sims_sim::object_catalog::move_tool::{cleanup::*, hover};
use std::collections::BTreeMap;

#[derive(Default)]
struct Host {
    objects: BTreeMap<u32, (bool, i16)>,
    cleared: Vec<u32>,
}
impl View for Host {
    fn disposable(&self, id: u32) -> Result<bool, String> {
        Ok(self.objects.get(&id).ok_or("object absent")?.0)
    }
    fn script_id(&self, id: u32) -> Result<i16, String> {
        Ok(self.objects.get(&id).ok_or("object absent")?.1)
    }
    fn clear_payload_cost(&mut self, id: u32) -> Result<(), String> {
        self.cleared.push(id);
        Ok(())
    }
}
impl hover::View for Host {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Ok(self
            .objects
            .iter()
            .find(|(_, v)| v.1 == id)
            .map(|(&id, _)| id))
    }
    fn flags(&self, _: u32) -> Result<u32, String> {
        Ok(0x1234_ffff)
    }
}
fn inputs() -> Inputs {
    Inputs {
        registry: 30,
        window: 40,
        key_function: 50,
        control_state: 60,
    }
}
fn next(c: &mut Call, s: &mut State, v: &mut Host, i: Inputs) -> Option<Request> {
    let r = c.advance(s, v, i).unwrap();
    *c = serde_json::from_slice(&serde_json::to_vec(c).unwrap()).unwrap();
    let before = s.clone();
    let cleared = v.cleared.clone();
    assert_eq!(c.advance(s, v, i).unwrap(), r);
    assert_eq!(*s, before);
    assert_eq!(v.cleared, cleared);
    if let Some(r) = r {
        let before = c.clone();
        assert!(
            c.acknowledge(
                r,
                if matches!(r, Request::Key { .. }) {
                    Reply::Done
                } else {
                    Reply::Key(0)
                }
            )
            .is_err()
        );
        assert_eq!(*c, before);
    }
    r
}
fn done(c: &mut Call, r: Request) {
    c.acknowledge(r, Reply::Done).unwrap();
    *c = serde_json::from_slice(&serde_json::to_vec(c).unwrap()).unwrap();
}

#[test]
fn deactivation_captures_discard_after_stop_and_rereads_history_between_callbacks() {
    let mut s = State::constructor_fields(1);
    s.payload = Some(2);
    s.held = Some(3);
    s.tool.flags[0] = 1;
    s.tool.cursor = 77;
    s.tool.special = 88;
    s.tool.position = [5, 6];
    s.tool.subtype = 99;
    s.bytes = [4; 12];
    let mut v = Host::default();
    v.objects.insert(4, (true, 5));
    let mut c = Call::new(Kind::Deactivate);
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::StopPayload(2))
    );
    s.tool.flags[0] = 0;
    s.held = Some(4);
    s.payload = Some(7);
    done(&mut c, Request::StopPayload(2));
    assert_eq!(next(&mut c, &mut s, &mut v, inputs()), Some(Request::Reset));
    assert_eq!(v.cleared, [7]);
    // Reset can erase all three predicate inputs. The captured result survives.
    s.tool.flags[0] = 1;
    s.held = None;
    s.payload = None;
    s.clipboard = 8;
    done(&mut c, Request::Reset);
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::Undo(8))
    );
    s.clipboard = 9;
    done(&mut c, Request::Undo(8));
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::ClearRedo(9))
    );
    done(&mut c, Request::ClearRedo(9));
    assert_eq!(next(&mut c, &mut s, &mut v, inputs()), None);
    assert!(c.complete());
    assert_eq!(
        (s.tool.argument, s.result, s.tool.mode, s.tool.definition),
        (-1, -1, 1, None)
    );
    assert_eq!(s.tool.flags, [0; 5]);
    assert_eq!(
        (
            s.tool.cursor,
            s.tool.special,
            s.tool.position,
            s.tool.subtype
        ),
        (77, 88, [5, 6], 99)
    );
    for offset in [0x79, 0x7a, 0x80, 0x98] {
        assert_eq!(s.byte(offset), 4);
    }
    for offset in [0x74, 0x7b, 0x81, 0x88, 0x89, 0x95, 0x96] {
        assert_eq!(s.byte(offset), 0);
    }
    assert_eq!(s.byte(0x97), 1);
}

#[test]
fn reset_retains_initial_gate_but_reads_current_payload_held_and_registry() {
    for purchased in [false, true] {
        let mut s = State::constructor_fields(1);
        s.held = Some(2);
        s.payload = Some(3);
        s.tool.flags[0] = u8::from(purchased);
        let mut v = Host::default();
        v.objects.insert(12, (false, -15));
        let mut i = inputs();
        let mut c = Call::new(Kind::Reset);
        let r = if purchased {
            Request::ObjectMode(2)
        } else {
            Request::PayloadKind {
                payload: 3,
                value: 3,
            }
        };
        assert_eq!(next(&mut c, &mut s, &mut v, i), Some(r));
        s.payload = Some(4);
        s.clipboard = 5;
        done(&mut c, r);
        if !purchased {
            let r = Request::Append {
                clipboard: 5,
                payload: 4,
            };
            assert_eq!(next(&mut c, &mut s, &mut v, i), Some(r));
            s.payload = Some(6);
            done(&mut c, r);
        }
        let r = Request::ReleasePayload(if purchased { 4 } else { 6 });
        assert_eq!(next(&mut c, &mut s, &mut v, i), Some(r));
        s.payload = Some(7);
        s.held = Some(11);
        done(&mut c, r);
        assert_eq!(
            next(&mut c, &mut s, &mut v, i),
            Some(Request::DetachObject(11))
        );
        assert_eq!(s.payload, None, "unconditional post-release store");
        s.held = Some(12);
        i.registry = 70;
        done(&mut c, Request::DetachObject(11));
        let r = Request::DeleteObject {
            registry: 70,
            id: -15,
        };
        assert_eq!(next(&mut c, &mut s, &mut v, i), Some(r));
        s.held = Some(13);
        done(&mut c, r);
        assert_eq!(next(&mut c, &mut s, &mut v, i), Some(Request::Hover));
        assert_eq!(s.held, None);
        done(&mut c, Request::Hover);
        assert_eq!(next(&mut c, &mut s, &mut v, i), Some(Request::Cursor));
        s.payload = Some(14);
        done(&mut c, Request::Cursor);
        assert_eq!(
            next(&mut c, &mut s, &mut v, i),
            Some(Request::ReleasePayload(14))
        );
        s.payload = Some(15);
        done(&mut c, Request::ReleasePayload(14));
        assert_eq!(
            next(&mut c, &mut s, &mut v, i),
            Some(Request::ClearCatalogSelection)
        );
        assert_eq!(s.payload, None);
        done(&mut c, Request::ClearCatalogSelection);
        assert_eq!(next(&mut c, &mut s, &mut v, i), None);
    }
}

#[test]
fn cleared_held_pointer_does_not_skip_mandatory_detach_and_delete() {
    let mut s = State::constructor_fields(1);
    s.held = Some(2);
    s.payload = Some(3);
    s.tool.flags[0] = 1;
    let mut v = Host::default();
    let mut c = Call::new(Kind::Reset);
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::ObjectMode(2))
    );
    s.held = None;
    done(&mut c, Request::ObjectMode(2));
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::ReleasePayload(3))
    );
    done(&mut c, Request::ReleasePayload(3));
    assert_eq!(
        next(&mut c, &mut s, &mut v, inputs()),
        Some(Request::DetachObject(0))
    );
    done(&mut c, Request::DetachObject(0));
    assert!(c.advance(&mut s, &mut v, inputs()).is_err());
    assert!(!c.complete());
}

#[test]
fn empty_reset_skips_history_but_releases_tail_payload_and_clears_selection_after_child() {
    let mut s = State::constructor_fields(1);
    s.payload = Some(2);
    let mut v = Host::default();
    let mut c = Call::new(Kind::Reset);
    for r in [
        Request::Hover,
        Request::Cursor,
        Request::ReleasePayload(2),
        Request::ClearCatalogSelection,
    ] {
        assert_eq!(next(&mut c, &mut s, &mut v, inputs()), Some(r));
        done(&mut c, r);
    }
    assert_eq!(next(&mut c, &mut s, &mut v, inputs()), None);
    for flag in [0, 1, 255] {
        s.set_byte(0x98, flag);
        let mut c = Call::new(Kind::ClearCatalogSelection);
        if flag != 0 {
            assert_eq!(
                next(&mut c, &mut s, &mut v, inputs()),
                Some(Request::ClearSelection(60))
            );
            assert_eq!(s.byte(0x98), flag);
            s.set_byte(0x98, 42);
            done(&mut c, Request::ClearSelection(60));
        }
        assert_eq!(next(&mut c, &mut s, &mut v, inputs()), None);
        assert_eq!(s.byte(0x98), 0);
    }
}

#[test]
fn cursor_preserves_key_import_and_rereads_window_and_tool_after_keys() {
    let mut v = Host::default();
    let mut s = State::constructor_fields(1);
    s.set_byte(0x79, 77);
    let before = s.clone();
    let mut c = Call::new(Kind::Cursor);
    assert_eq!(
        next(
            &mut c,
            &mut s,
            &mut v,
            Inputs {
                window: 0,
                ..inputs()
            }
        ),
        None
    );
    assert_eq!(s, before);
    for mode in [1, 3, 4, 5] {
        for ctrl in [0, 1, -1, i16::MIN] {
            for choice in 0..9 {
                let mut s = State::constructor_fields(1);
                let mut c = Call::new(Kind::Cursor);
                let shift = Request::Key {
                    function: 50,
                    key: 0x10,
                };
                assert_eq!(next(&mut c, &mut s, &mut v, inputs()), Some(shift));
                c.acknowledge(shift, Reply::Key(-1)).unwrap();
                let i = Inputs {
                    window: 99,
                    key_function: 999,
                    ..inputs()
                };
                let control = Request::Key {
                    function: 50,
                    key: 0x11,
                };
                assert_eq!(next(&mut c, &mut s, &mut v, i), Some(control));
                s.tool.mode = mode;
                s.tool.flags[2] = 1;
                s.held = (choice != 1).then_some(2);
                s.set_byte(0x80, u8::from(choice == 0));
                s.set_byte(0x7a, u8::from(choice == 2));
                s.set_byte(0x7b, u8::from(choice != 3));
                s.result = match choice {
                    4 => 0,
                    5 => 2,
                    6 => 4,
                    7 => 6,
                    _ => -1,
                };
                c.acknowledge(control, Reply::Key(ctrl)).unwrap();
                let active = ctrl < 0 && matches!(mode, 3 | 4);
                let value = if active {
                    6
                } else {
                    [12, 10, 35, 11, 14, 16, 15, 13, 12][choice]
                };
                let r = Request::SetCursor { window: 99, value };
                assert_eq!(next(&mut c, &mut s, &mut v, i), Some(r));
                assert_eq!(s.byte(0x79), u8::from(active));
                done(&mut c, r);
                assert_eq!(next(&mut c, &mut s, &mut v, i), None);
            }
        }
    }
}

#[derive(Default)]
struct HoverView(BTreeMap<i16, u32>, BTreeMap<u32, u32>);
impl hover::View for HoverView {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Ok(self.0.get(&id).copied())
    }
    fn flags(&self, id: u32) -> Result<u32, String> {
        self.1.get(&id).copied().ok_or("flags absent".into())
    }
}
#[test]
fn hover_publishes_after_old_flags_then_looks_up_new_identity_and_flags_afresh() {
    let mut v = HoverView::default();
    v.0.extend([(1, 10), (2, 20)]);
    v.1.extend([(10, 0xffff_ffff), (20, 0)]);
    let mut hovered = 1;
    let mut c = hover::Transfer::new(0x10002, 0x100); // low byte zero
    let old = hover::Request {
        object: 10,
        low: 14,
    };
    assert_eq!(c.advance(&mut hovered, &v).unwrap(), Some(old));
    assert_eq!(hovered, 1);
    c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
    assert_eq!(c.advance(&mut hovered, &v).unwrap(), Some(old));
    let before = c.clone();
    assert!(c.acknowledge(hover::Request { low: 0, ..old }).is_err());
    assert_eq!(c, before);
    v.0.insert(2, 30);
    v.1.insert(30, 0x1000_000a);
    hovered = -4;
    c.acknowledge(old).unwrap();
    let new = hover::Request {
        object: 30,
        low: 11,
    };
    assert_eq!(c.advance(&mut hovered, &v).unwrap(), Some(new));
    assert_eq!(hovered, 2);
    c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
    assert_eq!(c.advance(&mut hovered, &v).unwrap(), Some(new));
    c.acknowledge(new).unwrap();
    assert_eq!(c.advance(&mut hovered, &v).unwrap(), None);
    assert!(c.complete());
    let mut same = hover::Transfer::new(2, 255);
    assert_eq!(
        same.advance(&mut hovered, &HoverView::default()).unwrap(),
        None
    );
    let mut absent = hover::Transfer::new(65535, 1);
    assert_eq!(
        absent.advance(&mut hovered, &HoverView::default()).unwrap(),
        None
    );
    assert_eq!(hovered, -1);
}

#[test]
fn activation_composes_real_cleanup_and_hover_before_classifying_and_restores_each_child() {
    use sims_sim::object_catalog::{
        definition::Catalog,
        move_tool::{
            Groups,
            dispatch::{Dispatch, Service},
        },
    };
    let mut s = State::constructor_fields(1);
    s.payload = Some(2);
    s.hover = 3;
    s.set_byte(0x98, 1);
    s.tool.cursor = 42;
    s.tool.position = [9, 8];
    let mut v = Host::default();
    v.objects.insert(10, (false, 3));
    let mut c = Dispatch::activate(u32::MAX);
    let expected = [
        Service::External(Request::StopPayload(2)),
        Service::Flags(hover::Request {
            object: 10,
            low: 14,
        }),
        Service::External(Request::Key {
            function: 50,
            key: 0x10,
        }),
        Service::External(Request::Key {
            function: 50,
            key: 0x11,
        }),
        Service::External(Request::SetCursor {
            window: 40,
            value: 10,
        }),
        Service::External(Request::ReleasePayload(2)),
        Service::External(Request::ClearSelection(60)),
    ];
    for r in expected {
        assert_eq!(
            c.advance(
                &mut s,
                &mut v,
                inputs(),
                &Groups::default(),
                &Catalog::default(),
                0
            )
            .unwrap(),
            Some(r)
        );
        (c, s) = serde_json::from_slice(&serde_json::to_vec(&(&c, &s)).unwrap()).unwrap();
        c.validate().unwrap();
        let before = s.clone();
        assert_eq!(
            c.advance(
                &mut s,
                &mut v,
                inputs(),
                &Groups::default(),
                &Catalog::default(),
                0
            )
            .unwrap(),
            Some(r)
        );
        assert_eq!(s, before);
        if matches!(r, Service::Flags(_)) {
            let mut corrupt = serde_json::to_value(&c).unwrap();
            corrupt["stack"].as_array_mut().unwrap().pop();
            let mut orphan: Dispatch = serde_json::from_value(corrupt).unwrap();
            assert!(
                orphan
                    .advance(
                        &mut s,
                        &mut v,
                        inputs(),
                        &Groups::default(),
                        &Catalog::default(),
                        0
                    )
                    .is_err()
            );
            assert_eq!(s, before);
            let mut corrupt = serde_json::to_value(&c).unwrap();
            corrupt["stack"].as_array_mut().unwrap().last_mut().unwrap()["Hover"]["target"] =
                serde_json::json!(99);
            let mut wrong: Dispatch = serde_json::from_value(corrupt).unwrap();
            assert!(
                wrong
                    .advance(
                        &mut s,
                        &mut v,
                        inputs(),
                        &Groups::default(),
                        &Catalog::default(),
                        0
                    )
                    .is_err()
            );
            assert_eq!(s, before);
        }
        let wrong = Service::External(Request::Reset);
        let before = c.clone();
        assert!(c.acknowledge(wrong, Reply::Done).is_err());
        assert_eq!(c, before);
        let reply = if matches!(r, Service::External(Request::Key { .. })) {
            Reply::Key(0)
        } else {
            Reply::Done
        };
        c.acknowledge(r, reply).unwrap();
        c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
    }
    assert_eq!(
        c.advance(
            &mut s,
            &mut v,
            inputs(),
            &Groups::default(),
            &Catalog::default(),
            0
        )
        .unwrap(),
        None
    );
    assert!(c.complete());
    assert_eq!(s.hover, 0);
    assert_eq!(s.payload, None);
    assert_eq!(s.byte(0x98), 0);
    assert_eq!(s.tool.cursor, 42);
    assert_eq!(s.tool.position, [0; 2]);
    assert_eq!(s.tool.flags, [0, 0, 0, 1, 0]);
    assert_eq!(s.tool.argument, -1);
}

#[test]
fn viewport_leave_disables_motion_before_detach_and_retains_purchase_ownership() {
    for held in [None, Some(3)] {
        let mut state = State::constructor_fields(1);
        state.held = held;
        state.payload = Some(7);
        state.tool.flags[0] = 1;
        let mut before = state.clone();
        before.set_byte(0x97, 0);
        let mut call = Call::new(Kind::ViewportLeave);
        let mut host = Host::default();
        let request = next(&mut call, &mut state, &mut host, inputs());
        assert_eq!(request, held.map(Request::ObjectMode));
        assert_eq!(state, before);
        if let Some(r) = request {
            assert!(!call.complete());
            done(&mut call, r);
        }
        assert_eq!(next(&mut call, &mut state, &mut host, inputs()), None);
        assert!(call.complete());
        assert_eq!(state, before);
        assert!(host.cleared.is_empty());
    }
}
