use sims_sim::edit_history::{
    List, Slot, Store,
    command::{Method, Request},
    dependencies::{Call, Operation as O, Return, Side},
};

fn store() -> Store {
    let mut s = Store::default();
    for id in 1..=4 {
        s.construct_command(id).unwrap();
        let c = s.command_mut(id).unwrap();
        c.state = 1;
        c.references = 1;
    }
    s
}
fn next(c: &mut Call, s: &mut Store) -> Option<Request> {
    let r = c.advance(s).unwrap();
    (*c, *s) = serde_json::from_slice(&serde_json::to_vec(&(&*c, &*s)).unwrap()).unwrap();
    c.validate().unwrap();
    s.validate().unwrap();
    let before = s.clone();
    assert_eq!(c.advance(s).unwrap(), r);
    assert_eq!(*s, before);
    r
}
fn request(command: u32, method: Method) -> Request {
    Request { command, method }
}
fn list(side: Side) -> List {
    match side {
        Side::Before => List::Before(1),
        Side::After => List::After(1),
    }
}
fn values(s: &Store, list: List) -> Vec<u32> {
    let v = s.vector(list).unwrap();
    (0..v.len)
        .map(|index| {
            s.read(Slot {
                allocation: v.allocation,
                index,
            })
            .unwrap()
        })
        .collect()
}

#[test]
fn cancel_releases_captured_command_after_undo_and_preserves_prior_success_on_later_failure() {
    for side in [Side::Before, Side::After] {
        for failure in [None, Some(3), Some(2)] {
            let mut s = store();
            for id in [4, 2, 3] {
                s.append(list(side), id).unwrap();
            }
            let mut c = Call::new(1, O::Cancel(side)).unwrap();
            let mut visited = vec![];
            for id in [3, 2, 4] {
                let undo = request(id, Method::Undo);
                assert_eq!(next(&mut c, &mut s), Some(undo));
                visited.push(id);
                // Replacement does not alter the receiver subsequently released.
                let slot = Slot {
                    allocation: s.vector(list(side)).unwrap().allocation,
                    index: 3 - visited.len(),
                };
                s.write(slot, 1).unwrap();
                let failed = failure == Some(id);
                c.acknowledge(undo, if failed { 0x80000000 } else { 0 })
                    .unwrap();
                let destroy = request(id, Method::Destroy);
                assert_eq!(next(&mut c, &mut s), Some(destroy));
                s.deallocate_command(id).unwrap();
                // Destruction's return must not overwrite the preserved undo result.
                c.acknowledge(destroy, u32::MAX).unwrap();
                if failed {
                    break;
                }
            }
            assert_eq!(next(&mut c, &mut s), None);
            assert_eq!(c.result(), Some(Return::Byte(u8::from(failure != Some(3)))));
            assert!(values(&s, list(side)).is_empty());
            assert_eq!(s.command(1).unwrap().references, 1);
            if failure.is_some() {
                assert_eq!(
                    s.command(4).unwrap().references,
                    1,
                    "unvisited commands are not released by native erase"
                );
            }
        }
    }
}

#[test]
fn cancel_failure_erases_fresh_reallocated_list_but_success_keeps_captured_cursor() {
    for answer in [0, 1] {
        let mut s = store();
        s.append(List::After(1), 2).unwrap();
        let mut c = Call::new(1, O::Cancel(Side::After)).unwrap();
        assert_eq!(next(&mut c, &mut s), Some(request(2, Method::Undo)));
        s.append(List::After(1), 3).unwrap(); // replaces allocation
        c.acknowledge(request(2, Method::Undo), answer).unwrap();
        assert_eq!(next(&mut c, &mut s), Some(request(2, Method::Destroy)));
        s.deallocate_command(2).unwrap();
        c.acknowledge(request(2, Method::Destroy), 0).unwrap();
        if answer == 0 {
            assert!(c.advance(&mut s).is_err());
            assert_eq!(values(&s, List::After(1)), [2, 3]);
        } else {
            assert_eq!(next(&mut c, &mut s), None);
            assert_eq!(c.result(), Some(Return::Byte(0)));
            assert!(values(&s, List::After(1)).is_empty());
        }
    }
}

#[test]
fn dependency_clear_is_forward_does_not_zero_slots_and_reads_after_list_after_callbacks() {
    let mut s = store();
    s.append(List::Before(1), 2).unwrap();
    s.append(List::After(1), 3).unwrap();
    let before_slot = s.begin(List::Before(1)).unwrap();
    let mut c = Call::new(1, O::Clear).unwrap();
    assert_eq!(next(&mut c, &mut s), Some(request(2, Method::Destroy)));
    s.append(List::After(1), 4).unwrap();
    s.deallocate_command(2).unwrap();
    c.acknowledge(request(2, Method::Destroy), 0).unwrap();
    for id in [3, 4] {
        assert_eq!(next(&mut c, &mut s), Some(request(id, Method::Destroy)));
        s.deallocate_command(id).unwrap();
        c.acknowledge(request(id, Method::Destroy), 0).unwrap();
    }
    assert_eq!(next(&mut c, &mut s), None);
    assert_eq!(c.result(), Some(Return::Void));
    assert_eq!(s.read(before_slot).unwrap(), 2);
    assert!(values(&s, List::Before(1)).is_empty() && values(&s, List::After(1)).is_empty());
    s.append(List::Before(1), 0).unwrap();
    assert!(Call::new(1, O::Clear).unwrap().advance(&mut s).is_err());
}

#[test]
fn add_acquires_before_execute_and_registers_even_a_failed_command() {
    for side in [Side::Before, Side::After] {
        for answer in [0, 1, u32::MAX] {
            let mut s = store();
            s.command_mut(2).unwrap().references = u32::MAX;
            let mut c = Call::new(1, O::Add { side, command: 2 }).unwrap();
            assert_eq!(next(&mut c, &mut s), Some(request(2, Method::Execute)));
            assert_eq!(s.command(2).unwrap().references, 0);
            assert!(values(&s, list(side)).is_empty());
            s.append(list(side), 3).unwrap();
            c.acknowledge(request(2, Method::Execute), answer).unwrap();
            assert_eq!(next(&mut c, &mut s), None);
            assert_eq!(c.result(), Some(Return::Void));
            assert_eq!(values(&s, list(side)), [3, 2]);
            assert_eq!(s.command(2).unwrap().state, if answer == 0 { 1 } else { 3 });
        }
    }
}

#[test]
fn undo_is_reverse_redo_is_forward_and_failures_normalize_without_releasing_or_erasing() {
    for side in [Side::Before, Side::After] {
        for undo in [true, false] {
            for fail in [None, Some(2), Some(3)] {
                let mut s = store();
                for id in [2, 3] {
                    s.append(list(side), id).unwrap();
                }
                let mut c = Call::new(1, if undo { O::Undo(side) } else { O::Redo(side) }).unwrap();
                let method = if undo { Method::Undo } else { Method::Redo };
                for id in if undo { [3, 2] } else { [2, 3] } {
                    assert_eq!(next(&mut c, &mut s), Some(request(id, method)));
                    let before = c.clone();
                    assert!(c.acknowledge(request(4, method), 0).is_err());
                    assert_eq!(c, before);
                    c.acknowledge(
                        request(id, method),
                        if fail == Some(id) { u32::MAX } else { 0 },
                    )
                    .unwrap();
                    if fail == Some(id) {
                        break;
                    }
                }
                assert_eq!(next(&mut c, &mut s), None);
                assert_eq!(c.result(), Some(Return::Word(u32::from(fail.is_some()))));
                assert_eq!(values(&s, list(side)), [2, 3]);
                assert_eq!(
                    (
                        s.command(2).unwrap().references,
                        s.command(3).unwrap().references
                    ),
                    (1, 1)
                );
            }
        }
    }
}

fn corpus_case(c: &serde_json::Value, restore: bool) -> serde_json::Value {
    use serde_json::json;
    let mut s = Store::default();
    s.construct_command(99).unwrap();
    for n in 0..3 {
        s.construct_command(n + 1).unwrap();
        let command = s.command_mut(n + 1).unwrap();
        command.state = c["states"][n as usize].as_u64().unwrap() as u32;
        command.references = c["refs"][n as usize].as_u64().unwrap() as u32;
    }
    for (name, list) in [("before", List::Before(99)), ("after", List::After(99))] {
        let values = c[name].as_array().unwrap();
        for value in values {
            s.append(list, value.as_u64().unwrap() as u32).unwrap();
        }
        // Match the original harness's preallocated capacity; allocator calls
        // are outside this caller-comparison corpus.
        while s.vector(list).unwrap().len < 16 {
            s.append(list, 0).unwrap();
        }
        while s.vector(list).unwrap().len > values.len() {
            s.pop_end(list).unwrap();
        }
    }
    let op = c["op"].as_str().unwrap();
    let side = if op.ends_with("After") {
        Side::After
    } else {
        Side::Before
    };
    let operation = match op {
        "ClearDeps" => O::Clear,
        "CancelBefore" | "CancelAfter" => O::Cancel(side),
        "AddBefore" | "AddAfter" => O::Add { side, command: 1 },
        "UndoBefore" | "UndoAfter" => O::Undo(side),
        "RedoBefore" | "RedoAfter" => O::Redo(side),
        _ => panic!("unknown dependency operation"),
    };
    let mut call = Call::new(99, operation).unwrap();
    let mut events = vec![];
    let mut patched = false;
    while let Some(r) = call.advance(&mut s).unwrap() {
        if restore {
            (call, s) = serde_json::from_slice(&serde_json::to_vec(&(&call, &s)).unwrap()).unwrap();
            let before = s.clone();
            assert_eq!(call.advance(&mut s).unwrap(), Some(r));
            assert_eq!(s, before);
        }
        let command = s.command(r.command).unwrap();
        events.push(json!([
            format!("{:?}", r.method),
            r.command,
            command.state,
            command.references
        ]));
        if r.method != Method::Destroy {
            s.command_mut(r.command).unwrap().state = 99;
        }
        if c["append_during_callback"] == true && !patched {
            s.append(
                if side == Side::Before {
                    List::Before(99)
                } else {
                    List::After(99)
                },
                3,
            )
            .unwrap();
            patched = true;
        }
        let answer = c["answer_by_command"][r.command.to_string()]
            .as_u64()
            .unwrap_or_else(|| c["answer"].as_u64().unwrap()) as u32;
        // The native corpus supplies this same virtual body, including a
        // non-deallocating destructor. It compares callers, not object undo.
        call.acknowledge(r, answer).unwrap();
        if restore {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
        assert!(events.len() < 30);
    }
    s.validate().unwrap();
    let mut out = json!({"events": events,
        "commands": (1..=3).map(|id| {let c = s.command(id).unwrap(); [c.state,c.references]}).collect::<Vec<_>>(),
        "before":values(&s,List::Before(99)), "after":values(&s,List::After(99))});
    match call.result().unwrap() {
        Return::Void => (),
        Return::Byte(v) => out["return_byte"] = json!(v),
        Return::Word(v) => out["return_value"] = json!(v),
    }
    out
}

#[test]
#[ignore = "private prepared dependency inputs; not original-instruction validation"]
fn prepared_dependency_cases_restore_every_virtual_boundary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-buy-history-dependency-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], false);
    let rows = corpus["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 182);
    for (n, c) in rows.iter().enumerate() {
        assert_eq!(corpus_case(c, false), corpus_case(c, true), "case {n}: {c}");
    }
}

#[test]
#[ignore = "original x86 dependency corpus; requires no Cargo/rustc processes during generation"]
fn dependencies_match_original_caller_instructions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/edit-dependencies.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], true);
    assert_eq!(
        corpus["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = corpus["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 182);
    for (n, c) in rows.iter().enumerate() {
        for restore in [false, true] {
            assert_eq!(
                corpus_case(&c["input"], restore),
                c["original"],
                "case {n}, restore={restore}: {}",
                c["input"]
            );
        }
    }
}
