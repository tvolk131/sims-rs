//! Authored exact-byte arena cases. These are replacement-VM ownership tests,
//! not original engine vectors or a synthesized native Thread observation.
use super::*;
use crate::{Location, Module, Object, Routine, Snapshot, Start, Stop, World};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};

const CONTEXT: u32 = 0x9876_abcd;
fn expression(temp: i16, source_scope: u8, source: i16, next: u8) -> Instruction {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&temp.to_le_bytes());
    operands[2..4].copy_from_slice(&source.to_le_bytes());
    operands[5..].copy_from_slice(&[5, 8, source_scope]);
    Instruction {
        opcode: 2,
        on_true: next,
        on_false: 253,
        operands,
    }
}
fn routine(args: u8, locals: u16, instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "authored arena".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals,
            instructions,
        },
    }
}
fn definition() -> Definition {
    Definition {
        source: "private".into(),
        object: 1,
        guid: 7,
        slots: 0,
    }
}
fn program() -> Program {
    let mut words = vec![0; 108];
    words[14] = 7;
    Program {
        globals: "global".into(),
        modules: BTreeMap::from([
            (
                "private".into(),
                Module {
                    source: "private".into(),
                    routines: BTreeMap::from([
                        (
                            4096,
                            routine(
                                1,
                                0,
                                vec![
                                    Instruction {
                                        opcode: 4097,
                                        on_true: 1,
                                        on_false: 253,
                                        operands: [0; 8],
                                    },
                                    expression(2, 7, 77, 254),
                                ],
                            ),
                        ),
                        (
                            4097,
                            routine(
                                0,
                                2,
                                vec![expression(0, 25, 0, 1), expression(1, 25, 1, 254)],
                            ),
                        ),
                        (4098, routine(0, 0, vec![expression(3, 7, 42, 254)])),
                    ]),
                    object_definition_words: BTreeMap::from([(1, words)]),
                    object_functions: BTreeMap::from([(
                        1,
                        vec![
                            ObjectFunction {
                                action: 4098,
                                condition: 0,
                            },
                            ObjectFunction {
                                action: 4096,
                                condition: 0,
                            },
                        ],
                    )]),
                    ..Default::default()
                },
            ),
            (
                "global".into(),
                Module {
                    source: "global".into(),
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    }
}
fn controls() -> ThreadControls {
    // Explicitly AUTHORED known controls, not inferred from native stack bytes.
    ThreadControls {
        service: None,
        dispatch: None,
        autonomy: None,
        find_best: vec![],
        completed_actions: vec![],
        last_result: true,
        callback: None,
        callback_result: None,
        check: false,
        interrupted: false,
        status: Status::Ready,
    }
}
fn frame() -> Frame {
    Frame {
        continuation: 0,
        action_target: None,
        action_icon: false,
        code_owner: "private".into(),
        location: Location {
            source: "private".into(),
            bhav: 4096,
            instruction: 0,
        },
        caller: 1,
        callee: 1,
        stack_object: 0,
        args: vec![101],
        locals: vec![],
        opaque_locals: Default::default(),
        storage_offset: 0,
    }
}
fn binding() -> ContextBinding {
    ContextBinding {
        object: 1,
        definition: definition(),
        native_context: CONTEXT,
        registry_lifetime: Some(1),
    }
}
fn raw_bytes(words: usize) -> Vec<u8> {
    let mut memory = vec![0x5a5ai16; words];
    let f = frame();
    let mut header = crate::frame_header(&f);
    header[6] = Some(CONTEXT as i16);
    header[7] = Some((CONTEXT >> 16) as i16);
    for i in 0..8 {
        memory[i] = header[i].unwrap();
    }
    memory[8] = 101;
    // The next child's locals alias words equal to a context pointer. They are
    // raw integers, never made opaque by value-based pointer detection.
    if words >= 22 {
        memory[20] = CONTEXT as i16;
        memory[21] = (CONTEXT >> 16) as i16;
    }
    memory.into_iter().flat_map(i16::to_le_bytes).collect()
}
fn retained(p: &Program, words: usize) -> Thread {
    Thread::from_retained_frames(
        p,
        &raw_bytes(words),
        vec![RetainedFrame {
            frame: frame(),
            context_object: 1,
        }],
        controls(),
        vec![binding()],
    )
    .unwrap()
}
fn vm(p: &Program, words: usize) -> Vm {
    let mut w = World {
        scheduler_storage: Some(Default::default()),
        ..Default::default()
    };
    for object in [1, 2] {
        let mut value = Object {
            definition: Some(definition()),
            ..Default::default()
        };
        w.register_scheduler_object(object, &mut value).unwrap();
        w.objects.insert(object, value);
    }
    let mut v = Vm::new(w, 0x1234_5678);
    v.tick = 37;
    v.lot_rng_state = Some(0x4567_89ab);
    v.crt_rng_state = Some(0xaabb_ccdd);
    v.threads.insert(1, retained(p, words));
    v.next_thread = 2;
    v
}
fn cold(p: &Program, v: &Vm) -> Vm {
    let bytes = serde_json::to_vec(&Snapshot::capture(p, v)).unwrap();
    let snap: Snapshot = serde_json::from_slice(&bytes).unwrap();
    let restored = snap.restore(p).unwrap();
    assert_eq!(&restored, v);
    restored
}
fn start(target: i16) -> Start<'static> {
    Start {
        owner: "private",
        bhav: 4098,
        caller: 1,
        callee: target,
        stack_object: target,
        args: vec![],
        check: true,
    }
}
fn streams(v: &Vm) -> (u64, u32, Option<u32>, Option<u32>) {
    (v.tick, v.rng_state, v.lot_rng_state, v.crt_rng_state)
}

#[test]
fn direct_bytes_preserve_every_word_and_require_all_controls() {
    let p = program();
    let bytes = raw_bytes(256);
    let stack = RetainedStack::from_bytes(
        &p,
        &bytes,
        vec![RetainedFrame {
            frame: frame(),
            context_object: 1,
        }],
        vec![binding()],
    )
    .unwrap();
    assert_eq!(stack.raw_bytes(), bytes);
    let t = stack.into_thread(&p, controls()).unwrap();
    assert_eq!(t.frame_memory.len(), 256);
    assert!(t.frame_memory.iter().all(Option::is_some));
    let after: Vec<u8> = t
        .frame_memory
        .iter()
        .flat_map(|w| w.unwrap().to_le_bytes())
        .collect();
    assert_eq!(after, bytes);
    let value = serde_json::to_value(controls()).unwrap();
    for key in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<ThreadControls>(missing).is_err(),
            "absent control accepted: {key}"
        );
    }
    assert_eq!(
        serde_json::from_value::<ThreadControls>(value).unwrap(),
        controls()
    );
}

#[test]
fn call_return_inherits_literal_stale_words_and_keeps_context_metadata_through_cold() {
    let p = program();
    let mut warm = vm(&p, 256);
    let before = streams(&warm);
    let original = warm.threads[&1].frame_memory.clone();
    assert!(
        crate::raw_frame_word(&warm.threads[&1], 6)
            .unwrap_err()
            .contains("opaque native")
    );
    assert_eq!(
        crate::raw_frame_word(&warm.threads[&1], 20).unwrap(),
        CONTEXT as i16
    );
    assert_eq!(warm.run_slice(&p, 1, 1).unwrap().stop, Stop::Budget);
    let child = &warm.threads[&1].stack[1];
    assert_eq!(child.locals, [CONTEXT as i16, (CONTEXT >> 16) as i16]);
    assert!(child.opaque_locals.is_empty());
    assert_eq!(warm.threads[&1].frame_memory[18], Some(CONTEXT as i16));
    assert!(crate::raw_frame_word(&warm.threads[&1], 18).is_err());
    let mut resumed = cold(&p, &warm);
    for v in [&mut warm, &mut resumed] {
        assert_eq!(v.run_slice(&p, 1, 2).unwrap().stop, Stop::Budget);
        assert_eq!(v.threads[&1].stack.len(), 1);
        assert_eq!(
            v.world.objects[&1].temps[..2],
            [CONTEXT as i16, (CONTEXT >> 16) as i16]
        );
        assert_eq!(
            crate::raw_frame_word(&v.threads[&1], 18).unwrap(),
            CONTEXT as i16
        );
        assert_eq!(v.threads[&1].frame_memory[6], Some(CONTEXT as i16));
        assert_eq!(&v.threads[&1].frame_memory[24..], &original[24..]);
        assert_eq!(streams(v), before);
        cold(&p, v);
    }
    assert_eq!(warm, resumed);
    let mut resumed = cold(&p, &warm);
    assert_eq!(
        warm.run_slice(&p, 1, 2).unwrap(),
        resumed.run_slice(&p, 1, 2).unwrap()
    );
    assert_eq!(warm, resumed);
    assert!(warm.threads[&1].stack.is_empty());
    assert_eq!(
        crate::raw_frame_word(&warm.threads[&1], 6).unwrap(),
        CONTEXT as i16
    );
    cold(&p, &warm);
    assert_eq!(streams(&warm), before);
}

#[test]
fn callback_and_service_cut_preserve_buffers_and_fresh_input() {
    let p = program();
    for service in [false, true] {
        let mut warm = vm(&p, 256);
        let before = streams(&warm);
        let original = warm.threads[&1].frame_memory.clone();
        if service {
            warm.push_service_call(&p, 1, start(1)).unwrap();
        } else {
            warm.begin_immediate_callback(&p, 1, start(1)).unwrap();
        }
        let offset = warm.threads[&1].stack[1].storage_offset;
        assert_eq!(offset, if service { 12 } else { 20 });
        assert_eq!(
            warm.threads[&1].frame_memory[offset + 6],
            Some(CONTEXT as i16)
        );
        let mut resumed = cold(&p, &warm);
        for v in [&mut warm, &mut resumed] {
            v.run_slice(&p, 1, 1).unwrap();
            assert_eq!(v.threads[&1].stack.len(), 1);
            assert_eq!(v.world.objects[&1].temps[3], 42);
            assert_eq!(
                crate::raw_frame_word(&v.threads[&1], (offset + 6) as i64).unwrap(),
                CONTEXT as i16
            );
            if service {
                assert_eq!(v.pop_service(1).unwrap().result, Some(true));
            } else {
                assert_eq!(v.threads[&1].callback_result, Some(true));
            }
            assert_eq!(
                &v.threads[&1].frame_memory[offset + 8..],
                &original[offset + 8..]
            );
            assert_eq!(streams(v), before);
            cold(&p, v);
        }
        assert_eq!(warm, resumed);
        warm.begin_immediate_callback(&p, 1, start(1)).unwrap();
        warm.run_slice(&p, 1, 1).unwrap();
        cold(&p, &warm);
        assert_eq!(streams(&warm), before);
    }
}

#[test]
fn unknown_context_and_growth_fail_before_callback_or_service_mutation() {
    let p = program();
    for (words, target, expected) in [(256, 2, "unobserved"), (12, 1, "growth")] {
        for service in [false, true] {
            let mut v = vm(&p, words);
            let before = v.clone();
            let error = if service {
                v.push_service_call(&p, 1, start(target))
            } else {
                v.begin_immediate_callback(&p, 1, start(target))
            }
            .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert_eq!(v, before);
        }
    }
    let mut v = vm(&p, 12);
    let before = v.clone();
    assert_eq!(v.run_slice(&p, 1, 1).unwrap().stop, Stop::Faulted);
    assert_eq!(v.threads[&1].stack, before.threads[&1].stack);
    assert_eq!(v.threads[&1].frame_memory, before.threads[&1].frame_memory);
    assert_eq!(v.world, before.world);
    assert_eq!(streams(&v), streams(&before));
    assert!(matches!(&v.threads[&1].status,Status::Faulted(f)if f.message.contains("growth")));
}

#[test]
fn forged_context_topology_and_raw_frames_are_rejected_without_defaults() {
    let p = program();
    let raw = raw_bytes(256);
    for change in 0..6 {
        let mut bytes = raw.clone();
        let mut f = frame();
        let mut b = binding();
        match change {
            0 => bytes[12] ^= 1,
            1 => bytes[16] ^= 1,
            2 => f.location.source = "global".into(),
            3 => f.storage_offset = 4,
            4 => b.definition.guid = 8,
            5 => b.native_context = 0,
            _ => unreachable!(),
        }
        assert!(
            RetainedStack::from_bytes(
                &p,
                &bytes,
                vec![RetainedFrame {
                    frame: f,
                    context_object: 1
                }],
                vec![b]
            )
            .is_err()
        );
    }
    let v = vm(&p, 256);
    for change in 0..6 {
        let mut snapshot = Snapshot::capture(&p, &v);
        let t = snapshot.state.threads.get_mut(&1).unwrap();
        let a = t.retained_frame_arena.as_mut().unwrap();
        match change {
            0 => {
                a.active_contexts.clear();
            }
            1 => {
                a.bindings.get_mut(&1).unwrap().native_context ^= 1;
            }
            2 => {
                t.frame_memory[255] = None;
            }
            3 => {
                t.frame_memory.truncate(8);
            }
            4 => {
                a.active_contexts.insert(12, 1);
            }
            5 => {
                a.bindings.get_mut(&1).unwrap().registry_lifetime = Some(2);
            }
            _ => unreachable!(),
        }
        assert!(
            snapshot.restore(&p).is_err(),
            "forged retained owner accepted: {change}"
        );
    }
}

#[test]
fn normal_and_saved_stack_paths_keep_absent_native_metadata() {
    let p = program();
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            definition: Some(definition()),
            ..Default::default()
        },
    );
    let mut v = Vm::new(w, 1);
    let id = v
        .spawn(
            &p,
            Start {
                owner: "private",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 0,
                args: vec![101],
                check: false,
            },
        )
        .unwrap();
    assert!(v.threads[&id].retained_frame_arena.is_none());
    v.run_slice(&p, id, 1).unwrap();
    assert!(v.threads[&id].frame_memory[6].is_none());
    cold(&p, &v);
    let mut historical = serde_json::to_value(Snapshot::capture(&p, &v)).unwrap();
    for t in historical["state"]["threads"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        t.as_object_mut().unwrap().remove("retained_frame_arena");
    }
    let restored = serde_json::from_value::<Snapshot>(historical)
        .unwrap()
        .restore(&p)
        .unwrap();
    assert_eq!(restored, v);
    let mut w = World::default();
    w.objects.insert(
        1,
        Object {
            definition: Some(definition()),
            ..Default::default()
        },
    );
    let mut saved = Vm::new(w, 1);
    let id = saved
        .restore_object_stack(
            &p,
            1,
            &[crate::loading::SavedFrame {
                owner: "private".into(),
                tree: 4096,
                node: 0,
                stack_object: 0,
                args: vec![101],
                locals: vec![],
                primitive_state: 0,
            }],
            512,
            false,
        )
        .unwrap();
    assert!(saved.threads[&id].retained_frame_arena.is_none());
    assert_eq!(saved.threads[&id].frame_memory[6], None);
    cold(&p, &saved);
}

#[test]
fn lifecycle_reinstall_retains_native_arena_and_callback_context() {
    let p = program();
    let mut v = vm(&p, 256);
    let before = streams(&v);
    let raw = v.threads[&1].frame_memory.clone();
    assert_eq!(v.install_lifecycle(&p, 1, true, Some(1)).unwrap(), 1);
    assert_eq!(v.threads[&1].stack.len(), 2);
    assert_eq!(v.threads[&1].stack[1].location.bhav, 4098);
    let mut resumed = cold(&p, &v);
    for w in [&mut v, &mut resumed] {
        w.run_slice(&p, 1, 1).unwrap();
        assert_eq!(w.threads[&1].stack.len(), 1);
        assert_eq!(w.threads[&1].frame_memory[6], Some(CONTEXT as i16));
        assert_eq!(&w.threads[&1].frame_memory[28..], &raw[28..]);
        assert_eq!(streams(w), before);
        cold(&p, w);
    }
    assert_eq!(v, resumed);
}

#[test]
fn reused_id_same_definition_and_unknown_lifetime_never_borrow_old_context() {
    let p = program();
    let mut binding = binding();
    binding.registry_lifetime = None;
    let partial = RetainedStack::from_bytes(
        &p,
        &raw_bytes(256),
        vec![RetainedFrame {
            frame: frame(),
            context_object: 1,
        }],
        vec![binding],
    )
    .unwrap();
    assert_eq!(partial.raw_bytes(), raw_bytes(256));
    assert!(
        partial
            .into_thread(&p, controls())
            .unwrap_err()
            .contains("lifetime is unknown")
    );
    let mut v = vm(&p, 256);
    v.world.unregister_scheduler_object(1).unwrap();
    v.world.objects.remove(&1).unwrap();
    let mut replacement = Object {
        definition: Some(definition()),
        ..Default::default()
    };
    v.world
        .register_scheduler_object(1, &mut replacement)
        .unwrap();
    v.world.objects.insert(1, replacement);
    assert_eq!(
        v.world.scheduler_storage.as_ref().unwrap().handle(1),
        Some(3)
    );
    assert_eq!(v.world.objects[&1].definition, Some(definition()));
    for service in [false, true] {
        let before = v.clone();
        let error = if service {
            v.push_service_call(&p, 1, start(1))
        } else {
            v.begin_immediate_callback(&p, 1, start(1))
        }
        .unwrap_err();
        assert!(error.contains("lifetime mismatch"), "{error}");
        assert_eq!(v, before);
    }
    assert!(
        Snapshot::capture(&p, &v)
            .restore(&p)
            .unwrap_err()
            .contains("lifetime mismatch")
    );
}

#[test]
fn metadata_flush_preflights_all_frames_before_any_arena_write() {
    let p = program();
    let mut v = vm(&p, 256);
    v.run_slice(&p, 1, 1).unwrap();
    let t = v.threads.get_mut(&1).unwrap();
    t.stack[0].args[0] = -7654;
    t.retained_frame_arena
        .as_mut()
        .unwrap()
        .active_contexts
        .remove(&12);
    let before = t.frame_memory.clone();
    assert!(
        crate::save_frame_storage(t)
            .unwrap_err()
            .contains("binding absent")
    );
    assert_eq!(t.frame_memory, before);
}
