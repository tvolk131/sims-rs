use sims_sim::edit_history::{
    List, Store,
    command::{self, Method, Operation as C},
    manager::{Call, Operation as H, Service},
};

fn command(s: &mut Store, id: u32, state: u32, refs: u32) {
    s.construct_command(id).unwrap();
    let c = s.commands.get_mut(&id).unwrap();
    c.state = state;
    c.references = refs;
}
fn next(c: &mut Call, s: &mut Store) -> Option<Service> {
    let r = c.advance(s).unwrap();
    (*c, *s) = serde_json::from_slice(&serde_json::to_vec(&(&*c, &*s)).unwrap()).unwrap();
    let before = s.clone();
    assert_eq!(c.advance(s).unwrap(), r);
    assert_eq!(*s, before);
    r
}
fn virtual_(id: u32, method: Method) -> Service {
    Service::Virtual(command::Request {
        command: id,
        method,
    })
}
fn values(s: &Store, list: List) -> Vec<u32> {
    let mut slot = s.begin(list).unwrap();
    let mut result = vec![];
    while slot != s.end(list).unwrap() {
        result.push(s.read(slot).unwrap());
        slot = slot.next().unwrap();
    }
    result
}

#[test]
fn object_tool_close_releases_undo_then_redo_without_undoing_commands() {
    use sims_sim::{edit_history::tool, object_catalog::move_tool::cleanup::Request};
    let mut store = Store::default();
    store.construct_manager(1).unwrap();
    command(&mut store, 2, 1, 2);
    command(&mut store, 4, 1, 1);
    store.append(List::Undo(1), 2).unwrap();
    store.append(List::Redo(1), 4).unwrap();
    let mut undo = tool::Call::new(Request::ClearUndo(1)).unwrap().unwrap();
    assert_eq!(undo.advance(&mut store).unwrap(), None);
    assert!(undo.complete());
    assert!(values(&store, List::Undo(1)).is_empty());
    assert_eq!(values(&store, List::Redo(1)), [4]);
    assert_eq!(store.commands[&2].references, 1);
    let mut redo = tool::Call::new(Request::ClearRedo(1)).unwrap().unwrap();
    let request = virtual_(4, Method::Destroy);
    assert_eq!(redo.advance(&mut store).unwrap(), Some(request));
    (redo, store) = serde_json::from_slice(&serde_json::to_vec(&(redo, store)).unwrap()).unwrap();
    store.deallocate_command(4).unwrap();
    redo.acknowledge(request, 0).unwrap();
    assert_eq!(redo.advance(&mut store).unwrap(), None);
    assert!(redo.complete());
    assert!(values(&store, List::Redo(1)).is_empty());
    assert_eq!(store.commands[&2].state, 1);
}

#[test]
fn command_state_keeps_full_return_values_and_overwrites_callback_state_after_return() {
    for initial in [0, 1, 2, 3, u32::MAX] {
        for op in [C::Execute, C::Undo, C::Redo] {
            for first in [0, 1, 0x80000000, u32::MAX] {
                let mut s = Store::default();
                command(&mut s, 7, initial, 99);
                let mut c = command::Call::new(7, op).unwrap();
                let first_method = match op {
                    C::Execute => Method::Execute,
                    C::Redo => Method::Redo,
                    C::Undo if initial == 0 => Method::Execute,
                    _ => Method::Undo,
                };
                let mut methods = vec![first_method];
                if op == C::Undo && initial == 0 && first == 0 {
                    methods.push(Method::Undo);
                }
                for (n, method) in methods.iter().copied().enumerate() {
                    let r = command::Request { command: 7, method };
                    assert_eq!(c.advance(&mut s).unwrap(), Some(r));
                    (c, s) =
                        serde_json::from_slice(&serde_json::to_vec(&(&c, &s)).unwrap()).unwrap();
                    let before = s.clone();
                    assert_eq!(c.advance(&mut s).unwrap(), Some(r));
                    assert_eq!(s, before);
                    let before = c.clone();
                    assert!(
                        c.acknowledge(command::Request { command: 8, method }, 0)
                            .is_err()
                    );
                    assert_eq!(c, before);
                    s.commands.get_mut(&7).unwrap().state = 77;
                    c.acknowledge(r, if n == 0 { first } else { 0xdeadbeef })
                        .unwrap();
                }
                assert_eq!(c.advance(&mut s).unwrap(), None);
                let answer = if methods.len() == 2 {
                    0xdeadbeef
                } else {
                    first
                };
                assert_eq!(c.result(), Some(answer));
                let state = if *methods.last().unwrap() == Method::Undo {
                    2 + u32::from(answer != 0)
                } else if answer == 0 {
                    1
                } else {
                    3
                };
                assert_eq!(s.commands[&7].state, state);
                assert_eq!(s.commands[&7].references, 99);
            }
        }
    }
}

#[test]
fn release_wraps_and_only_exact_zero_destroys_without_reading_after_destruction() {
    for refs in [0, 1, 2, 0x80000000, u32::MAX] {
        let mut s = Store::default();
        command(&mut s, 7, 3, refs);
        let mut c = command::Call::new(7, C::Release).unwrap();
        let r = c.advance(&mut s).unwrap();
        assert_eq!(s.commands[&7].references, refs.wrapping_sub(1));
        if refs == 1 {
            let r = r.unwrap();
            assert_eq!(r.method, Method::Destroy);
            (c, s) = serde_json::from_slice(&serde_json::to_vec(&(&c, &s)).unwrap()).unwrap();
            assert_eq!(c.advance(&mut s).unwrap(), Some(r));
            s.deallocate_command(7).unwrap();
            c.acknowledge(r, 0xdeadbeef).unwrap();
            assert_eq!(c.advance(&mut s).unwrap(), None);
            assert_eq!(c.result(), Some(0));
        } else {
            assert_eq!(r, None);
            assert_eq!(c.result(), Some(refs.wrapping_sub(1)));
        }
    }
}

#[test]
fn append_executes_once_retains_command_and_clears_redo_before_notify() {
    for state in [0, 1, 3] {
        let mut s = Store::default();
        s.construct_manager(1).unwrap();
        command(&mut s, 2, state, 4);
        command(&mut s, 3, 2, 1);
        s.append(List::Redo(1), 3).unwrap();
        s.append(List::Redo(1), 0).unwrap();
        let mut c = Call::new(H::Append {
            manager: 1,
            command: 2,
        });
        if state != 1 {
            assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Execute)));
            s.commands.get_mut(&2).unwrap().state = 88;
            c.acknowledge(virtual_(2, Method::Execute), 0).unwrap();
        }
        assert_eq!(next(&mut c, &mut s), Some(virtual_(3, Method::Destroy)));
        assert_eq!(values(&s, List::Undo(1)), [2]);
        assert_eq!(s.commands[&2].references, 5);
        assert_eq!(s.commands[&2].state, 1);
        let slot = s.begin(List::Redo(1)).unwrap();
        // The original clears the same slot after destruction, even when its
        // current value was replaced by a callback.
        s.write(slot, 99).unwrap();
        s.deallocate_command(3).unwrap();
        c.acknowledge(virtual_(3, Method::Destroy), 71).unwrap();
        assert_eq!(next(&mut c, &mut s), Some(Service::Notify));
        assert!(values(&s, List::Redo(1)).is_empty());
        assert_eq!(s.read(slot).unwrap(), 0);
        c.acknowledge(Service::Notify, 0).unwrap();
        assert_eq!(next(&mut c, &mut s), None);
        assert!(c.complete());
    }
}

#[test]
fn failed_append_clears_both_histories_without_acquiring_or_broadcasting() {
    let mut s = Store::default();
    s.construct_manager(1).unwrap();
    command(&mut s, 2, 0, 1);
    command(&mut s, 3, 1, 1);
    command(&mut s, 4, 2, 2);
    s.append(List::Undo(1), 3).unwrap();
    s.append(List::Redo(1), 4).unwrap();
    let mut c = Call::new(H::Append {
        manager: 1,
        command: 2,
    });
    assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Execute)));
    c.acknowledge(virtual_(2, Method::Execute), 0x80000000)
        .unwrap();
    assert_eq!(next(&mut c, &mut s), Some(virtual_(3, Method::Destroy)));
    s.deallocate_command(3).unwrap();
    c.acknowledge(virtual_(3, Method::Destroy), 0).unwrap();
    assert_eq!(next(&mut c, &mut s), None);
    assert_eq!(s.commands[&2].references, 1);
    assert_eq!(s.commands[&2].state, 3);
    assert_eq!(s.commands[&4].references, 1);
    assert!(values(&s, List::Undo(1)).is_empty() && values(&s, List::Redo(1)).is_empty());
}

#[test]
fn undo_and_redo_transfer_the_fresh_top_without_reference_count_changes() {
    for undo in [true, false] {
        let mut s = Store::default();
        s.construct_manager(1).unwrap();
        command(&mut s, 2, 1, 3);
        command(&mut s, 3, 1, 4);
        let (source, target, op, method) = if undo {
            (List::Undo(1), List::Redo(1), H::Undo(1), Method::Undo)
        } else {
            (List::Redo(1), List::Undo(1), H::Redo(1), Method::Redo)
        };
        s.append(source, 2).unwrap();
        let mut c = Call::new(op);
        assert_eq!(next(&mut c, &mut s), Some(virtual_(2, method)));
        // Callback appends another command, reallocating the source. Caller
        // captures the command receiver but later rereads source end.
        s.append(source, 3).unwrap();
        c.acknowledge(virtual_(2, method), 0).unwrap();
        assert_eq!(next(&mut c, &mut s), Some(Service::Notify));
        assert_eq!(values(&s, source), [2]);
        assert_eq!(values(&s, target), [3]);
        assert_eq!(
            (s.commands[&2].references, s.commands[&3].references),
            (3, 4)
        );
        assert_eq!(s.commands[&2].state, if undo { 2 } else { 1 });
        c.acknowledge(Service::Notify, 0).unwrap();
        assert_eq!(next(&mut c, &mut s), None);
    }
}

#[test]
fn failed_undo_clears_current_history_then_notifies_and_empty_undo_fails() {
    let mut s = Store::default();
    s.construct_manager(1).unwrap();
    command(&mut s, 2, 1, 1);
    s.append(List::Undo(1), 2).unwrap();
    let mut c = Call::new(H::Undo(1));
    assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Undo)));
    c.acknowledge(virtual_(2, Method::Undo), u32::MAX).unwrap();
    assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Destroy)));
    s.deallocate_command(2).unwrap();
    c.acknowledge(virtual_(2, Method::Destroy), 7).unwrap();
    assert_eq!(next(&mut c, &mut s), Some(Service::Notify));
    assert!(values(&s, List::Undo(1)).is_empty());
    c.acknowledge(Service::Notify, 0).unwrap();
    assert_eq!(next(&mut c, &mut s), None);
    let mut empty = Call::new(H::Undo(1));
    assert!(empty.advance(&mut s).is_err());
    assert!(!empty.complete());
}

#[test]
fn clear_rereads_live_end_but_never_rebases_a_captured_allocation_slot() {
    for reallocate in [false, true] {
        let mut s = Store::default();
        s.construct_manager(1).unwrap();
        for id in [2, 3, 4] {
            command(&mut s, id, 1, 1);
        }
        s.append(List::Undo(1), 2).unwrap();
        s.append(List::Undo(1), 3).unwrap();
        if !reallocate {
            s.append(List::Undo(1), 0).unwrap(); // capacity4
            s.pop_end(List::Undo(1)).unwrap();
        }
        let slot = s.begin(List::Undo(1)).unwrap();
        let mut c = Call::new(H::ClearUndo(1));
        assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Destroy)));
        s.append(List::Undo(1), 4).unwrap();
        s.deallocate_command(2).unwrap();
        c.acknowledge(virtual_(2, Method::Destroy), 0).unwrap();
        if reallocate {
            assert!(s.read(slot).is_err());
            assert!(
                c.advance(&mut s).is_err(),
                "freed native cursor must not silently rebase"
            );
            assert!(!c.complete());
        } else {
            for id in [3, 4] {
                assert_eq!(next(&mut c, &mut s), Some(virtual_(id, Method::Destroy)));
                s.deallocate_command(id).unwrap();
                c.acknowledge(virtual_(id, Method::Destroy), 0).unwrap();
            }
            assert_eq!(next(&mut c, &mut s), None);
            assert!(values(&s, List::Undo(1)).is_empty());
        }
    }
}

#[test]
fn malformed_snapshot_or_wrong_child_reply_does_not_mutate_history() {
    let mut s = Store::default();
    s.construct_manager(1).unwrap();
    command(&mut s, 2, 0, 1);
    let mut c = Call::new(H::Append {
        manager: 1,
        command: 2,
    });
    assert_eq!(next(&mut c, &mut s), Some(virtual_(2, Method::Execute)));
    let before = c.clone();
    assert!(c.acknowledge(Service::Notify, 0).is_err());
    assert_eq!(c, before);
    let mut v = serde_json::to_value(&c).unwrap();
    v["stack"].as_array_mut().unwrap().pop();
    let mut corrupt: Call = serde_json::from_value(v).unwrap();
    let before = s.clone();
    assert!(corrupt.advance(&mut s).is_err());
    assert_eq!(s, before);
    let mut v = serde_json::to_value(&s).unwrap();
    let duplicate = v["lists"][0].clone();
    v["lists"].as_array_mut().unwrap().push(duplicate);
    assert!(serde_json::from_value::<Store>(v).is_err());
}

#[derive(serde::Serialize, serde::Deserialize)]
enum Runner {
    Manager(Call),
    Command(command::Call),
}
impl Runner {
    fn advance(&mut self, s: &mut Store) -> Option<Service> {
        match self {
            Self::Manager(c) => c.advance(s).unwrap(),
            Self::Command(c) => c.advance(s).unwrap().map(Service::Virtual),
        }
    }
    fn ack(&mut self, r: Service, value: u32) {
        match self {
            Self::Manager(c) => c.acknowledge(r, value).unwrap(),
            Self::Command(c) => {
                let Service::Virtual(r) = r else {
                    panic!("command cannot notify")
                };
                c.acknowledge(r, value).unwrap();
            }
        }
    }
}
fn corpus_case(c: &serde_json::Value, restore: bool) -> serde_json::Value {
    use serde_json::json;
    let mut s = Store::default();
    s.construct_manager(99).unwrap();
    for n in 0..3 {
        command(
            &mut s,
            n + 1,
            c["states"][n as usize].as_u64().unwrap() as u32,
            c["refs"][n as usize].as_u64().unwrap() as u32,
        );
    }
    for (name, list) in [("undo", List::Undo(99)), ("redo", List::Redo(99))] {
        let values = c[name].as_array().unwrap();
        for v in values {
            s.append(list, v.as_u64().unwrap() as u32).unwrap();
        }
        // Native fixture has capacity16; preserve the same allocation lifetime
        // across callback appends. No original allocator behavior is claimed.
        while s.vector(list).unwrap().len < 16 {
            s.append(list, 0).unwrap();
        }
        while s.vector(list).unwrap().len > values.len() {
            s.pop_end(list).unwrap();
        }
    }
    let op = c["op"].as_str().unwrap();
    let mut runner = match op {
        "Execute" => Runner::Command(command::Call::new(1, C::Execute).unwrap()),
        "UndoCommand" => Runner::Command(command::Call::new(1, C::Undo).unwrap()),
        "RedoCommand" => Runner::Command(command::Call::new(1, C::Redo).unwrap()),
        "Release" => Runner::Command(command::Call::new(1, C::Release).unwrap()),
        "Append" => Runner::Manager(Call::new(H::Append {
            manager: 99,
            command: 1,
        })),
        "Undo" => Runner::Manager(Call::new(H::Undo(99))),
        "Redo" => Runner::Manager(Call::new(H::Redo(99))),
        "ClearUndo" => Runner::Manager(Call::new(H::ClearUndo(99))),
        "ClearRedo" => Runner::Manager(Call::new(H::ClearRedo(99))),
        _ => panic!("unknown corpus op"),
    };
    let mut patched = false;
    let mut events = vec![];
    while let Some(r) = runner.advance(&mut s) {
        if restore {
            (runner, s) =
                serde_json::from_slice(&serde_json::to_vec(&(&runner, &s)).unwrap()).unwrap();
            let before = s.clone();
            assert_eq!(runner.advance(&mut s), Some(r));
            assert_eq!(s, before);
        }
        let answer = match r {
            Service::Notify => {
                events.push(json!(["Notify"]));
                0
            }
            Service::Virtual(r) => {
                let state = &s.commands[&r.command];
                events.push(json!([
                    format!("{:?}", r.method),
                    r.command,
                    state.state,
                    state.references
                ]));
                if r.method != Method::Destroy {
                    s.commands.get_mut(&r.command).unwrap().state = 99;
                }
                if c["append_during_callback"] == true && !patched {
                    let list = if matches!(op, "Undo" | "ClearUndo") {
                        List::Undo(99)
                    } else {
                        List::Redo(99)
                    };
                    s.append(list, 3).unwrap();
                    patched = true;
                }
                c["answer"].as_u64().unwrap() as u32
            }
        };
        runner.ack(r, answer);
        if restore {
            runner = serde_json::from_slice(&serde_json::to_vec(&runner).unwrap()).unwrap();
        }
        assert!(events.len() < 30);
    }
    let mut out = json!({"events":events,"commands":(1..=3).map(|id|[s.commands[&id].state,s.commands[&id].references]).collect::<Vec<_>>(),"undo":values(&s,List::Undo(99)),"redo":values(&s,List::Redo(99))});
    if let Runner::Command(c) = runner {
        out["return_value"] = json!(c.result().unwrap());
    }
    out
}

#[test]
#[ignore = "private prepared inputs; not an original-instruction comparison"]
fn prepared_edit_history_cases_restore_every_command_and_notification() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-buy-history-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v["native_execution"], false);
    assert_eq!(v["cases"].as_array().unwrap().len(), 114);
    for c in v["cases"].as_array().unwrap() {
        assert_eq!(corpus_case(c, false), corpus_case(c, true), "{c}");
    }
}

#[test]
#[ignore = "original x86 corpus; generate only with no Cargo/rustc processes"]
fn edit_history_matches_original_caller_instructions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/edit-history.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v["native_execution"], true);
    assert_eq!(
        v["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        v["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(v["cases"].as_array().unwrap().len(), 114);
    for row in v["cases"].as_array().unwrap() {
        assert_eq!(
            corpus_case(&row["input"], true),
            row["original"],
            "{}",
            row["input"]
        );
    }
}
