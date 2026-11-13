//! Authored protocol cases, not captured native-baseline or gameplay parity.
use super::*;
use crate::{
    Definition, Location, Module, Object, Routine, Snapshot, Start,
    lifecycle::{Lifecycle, ServiceCall},
    retained_completed::{
        CompletedAdapterEpoch, CompletedThreadInput, IdentityMapping, ObjectThreadBinding,
    },
    retained_epoch::VmProjection,
    retained_frames::{RetainedFrame, RetainedStack, ThreadControls},
    scheduler::{Inputs, Pass, Request, registry::Borrowed},
};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};
use std::collections::BTreeMap;
const CONTEXT: u32 = 0x1020_3000;
fn instruction(opcode: u16) -> Instruction {
    Instruction {
        opcode,
        on_true: 0,
        on_false: 253,
        operands: [0; 8],
    }
}
fn routine(args: u8, locals: u16, opcode: u16) -> Routine {
    Routine {
        name: "AUTHORED completed registry".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals,
            instructions: vec![instruction(opcode)],
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
    words[9] = 4;
    words[14] = 7;
    let mut functions = vec![
        ObjectFunction {
            action: 0,
            condition: 0
        };
        30
    ];
    functions[1].action = 4096;
    functions[3].action = 4098;
    let mut callback = instruction(2);
    callback.on_true = 254;
    callback.operands[2..4].copy_from_slice(&42i16.to_le_bytes());
    callback.operands[5..].copy_from_slice(&[5, 8, 7]);
    let mut reset = routine(0, 0, 2);
    reset.body.instructions = vec![callback];
    Program {
        globals: "global".into(),
        modules: BTreeMap::from([
            (
                "private".into(),
                Module {
                    source: "private".into(),
                    object_definition_words: BTreeMap::from([(1, words)]),
                    object_functions: BTreeMap::from([(1, functions)]),
                    routines: BTreeMap::from([
                        (4096, routine(4, 1, 4097)),
                        (4097, routine(2, 2, 280)),
                        (4098, reset),
                    ]),
                    ..Default::default()
                },
            ),
            (
                "global".into(),
                Module {
                    source: "global".into(),
                    routines: BTreeMap::from([(280, routine(1, 0, 0))]),
                    ..Default::default()
                },
            ),
        ]),
        ..Default::default()
    }
}
fn frames() -> Vec<Frame> {
    let mut offset = 0;
    [(4096, 4, 1), (4097, 2, 2), (280, 1, 0)]
        .into_iter()
        .map(|(bhav, args, locals)| {
            let frame = Frame {
                continuation: 0,
                action_target: None,
                action_icon: false,
                code_owner: "private".into(),
                location: Location {
                    source: if bhav < 4096 { "global" } else { "private" }.into(),
                    bhav,
                    instruction: 0,
                },
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: if bhav == 280 { vec![10] } else { vec![0; args] },
                locals: vec![0; locals],
                opaque_locals: Default::default(),
                storage_offset: offset,
            };
            offset += crate::frame_words(args, locals);
            frame
        })
        .collect()
}
fn stack(p: &Program) -> RetainedStack {
    let frames = frames();
    let mut words = vec![0x5a5ai16; 128];
    for f in &frames {
        let mut header = crate::frame_header(f);
        header[6] = Some(CONTEXT as i16);
        header[7] = Some((CONTEXT >> 16) as i16);
        for (i, v) in header.into_iter().enumerate() {
            words[f.storage_offset + i] = v.unwrap();
        }
        for (i, v) in f.args.iter().chain(&f.locals).enumerate() {
            words[f.storage_offset + 8 + i] = *v;
        }
    }
    RetainedStack::from_bytes(
        p,
        &words
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>(),
        frames
            .into_iter()
            .map(|frame| RetainedFrame {
                frame,
                context_object: 1,
            })
            .collect(),
        vec![ContextBinding {
            object: 1,
            definition: definition(),
            native_context: CONTEXT,
            registry_lifetime: Some(1),
        }],
    )
    .unwrap()
}
fn world() -> World {
    // Entirely authored; defaults here are never inferred for native import.
    let mut world = World {
        scheduler_storage: Some(Default::default()),
        ..Default::default()
    };
    let mut object = Object {
        definition: Some(definition()),
        native_tree_error: Some(0),
        native_flags: Some(0x80),
        data: (0..128).map(|i| (i, 0)).collect(),
        ..Default::default()
    };
    world.register_scheduler_object(1, &mut object).unwrap();
    object.idle.as_mut().unwrap().flags = 1;
    object.idle.as_mut().unwrap().countdown = 2;
    world.objects.insert(1, object);
    world
}
fn fresh() -> FreshUnpublishedRequestEpoch {
    FreshUnpublishedRequestEpoch {
        source_key: "AUTHORED explicit adapter".into(),
        accepted_commands: 0,
        published_thread_handles: 0,
        published_request_handles: 0,
    }
}
fn adapter() -> AdapterOwnership {
    AdapterOwnership {
        fresh: fresh(),
        retained_construction_owners: vec![],
        retained_native_consumers: vec![],
        non_group_objects: vec![1],
        construction_ownership_source: "AUTHORED no construction owner".into(),
        native_consumer_ownership_source: "AUTHORED no native consumer".into(),
        group_ownership_source: "AUTHORED ordinary non-group".into(),
    }
}
fn token() -> Token {
    let a = adapter();
    Token {
        schema: 1,
        profile: PROFILE.into(),
        boundary: Boundary {
            kind: "baseline".into(),
            tick: 37,
            eip: 0x46d51d,
            simulator: 0x100000,
            registry: 0x200000,
            advanced: 1,
            advanced_observation: Observation {
                address: 0x300001,
                bytes: 1,
                raw: "01".into(),
                sha256: ADVANCED.into(),
            },
            scheduler_cursor: 0,
            main_thread_tib_sha256: "1".repeat(64),
            main_thread_stack_sha256: "2".repeat(64),
        },
        source_binding: SourceBinding {
            resource: 0x400000,
            context: CONTEXT,
            definition: 0x500000,
            guid: 7,
            objd: 1,
        },
        object: 1,
        native_pointer: 0x600000,
        cached_main: 4096,
        cached_init: 0,
        frame_shapes: frames()
            .iter()
            .map(|f| FrameShape {
                offset_words: f.storage_offset,
                tree: f.location.bhav,
                node: f.location.instruction,
                context: CONTEXT,
                argc: f.args.len(),
                locals: f.locals.len(),
            })
            .collect(),
        native_reentry: Reentry {
            kind: "ordinary-main-at-sleep-next-eligible-registry-tick".into(),
            last_frame: 2,
            argument_index: 0,
            argument_value: 10,
            registry_word: 0x10002,
            countdown: 2,
            flags: 1,
            known_flags: Flags {
                enabled: true,
                run_while_paused: false,
                error_service_bit: false,
            },
            last_yield_tick: "not inferred".into(),
            wake_tick: "not synthesized".into(),
        },
        reference_image_sha256: IMAGE.into(),
        source_witness_sha256: WITNESS.into(),
        raw_capture_sha256: "3".repeat(64),
        adapter_epoch_source: a.fresh.source_key,
        construction_ownership_source: a.construction_ownership_source,
        native_consumer_ownership_source: a.native_consumer_ownership_source,
        group_ownership_source: a.group_ownership_source,
    }
}
fn controls(status: Status) -> ThreadControls {
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
        status,
    }
}
fn assembled(p: &Program, owner: Owner, source: &str) -> Result<Vm, String> {
    Vm::from_retained_completed_epoch(
        p,
        world(),
        VmProjection {
            tick: 37,
            rng_state: 0x12345678,
            lot_rng_state: Some(0x456789ab),
            crt_rng_state: Some(0xaabbccdd),
            scheduled_object_visit: None,
            player_menu: None,
            lot_rng_scope: None,
            named_calls: vec![],
            native_accessory_host: false,
            sounds: vec![],
            effects: vec![],
        },
        vec![CompletedThreadInput {
            source_owner: "AUTHORED native thread".into(),
            adapter_thread: 1,
            object: Some(1),
            stack: stack(p),
            controls: controls(Status::RegistrySuspended(owner)),
        }],
        vec![ObjectThreadBinding {
            object: 1,
            thread: Some(1),
            lifecycle: Some(Lifecycle {
                owner: "private".into(),
                definition: 1,
                init: 0,
                main: 4096,
                phase: Phase::Main,
                thread: 1,
                run_init: false,
            }),
        }],
        CompletedAdapterEpoch {
            fresh: FreshUnpublishedRequestEpoch {
                source_key: source.into(),
                ..fresh()
            },
            threads: vec![IdentityMapping {
                source_owner: "AUTHORED native thread".into(),
                adapter: 1,
            }],
            requests: vec![],
            next_thread: 2,
            next_request: 1,
        },
    )
}
fn vm(p: &Program) -> Vm {
    let owner = stack(p)
        .registry_suspension(p, &world(), 37, token(), adapter())
        .unwrap();
    assembled(p, owner, &fresh().source_key).unwrap()
}
fn cold(p: &Program, v: &Vm) -> Vm {
    let snapshot = Snapshot::capture(p, v);
    let json: Snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let restored = json.restore(p).unwrap();
    assert_eq!(&restored, v);
    restored
}
fn rng(v: &Vm) -> (u32, Option<u32>, Option<u32>) {
    (v.rng_state, v.lot_rng_state, v.crt_rng_state)
}
#[test]
fn owned_three_frame_assembly_preserves_every_supplied_owner_and_all_rng() {
    let p = program();
    let w = world();
    let s = stack(&p);
    let before = (w.clone(), s.clone());
    let owner = s
        .registry_suspension(&p, &w, 37, token(), adapter())
        .unwrap();
    assert_eq!((w, s), before);
    let v = assembled(&p, owner.clone(), &fresh().source_key).unwrap();
    assert_eq!(v.world, before.0);
    assert_eq!(v.threads[&1].stack, frames());
    assert_eq!(rng(&v), (0x12345678, Some(0x456789ab), Some(0xaabbccdd)));
    assert_eq!(v.next_thread, 2);
    assert_eq!(v.next_request, 1);
    assert_eq!(v.tick, 37);
    assert_eq!(v.world.objects[&1].temps, [0; 8]);
    cold(&p, &v);
    assert!(
        assembled(&p, owner, "different unpublished domain")
            .unwrap_err()
            .contains("epoch")
    );
    // Old JSON shapes remain identical. The new variant is also appended,
    // preserving the existing serde enum discriminant order for binary users.
    assert_eq!(serde_json::to_string(&Status::Ready).unwrap(), "\"Ready\"");
    assert_eq!(
        serde_json::to_string(&Status::Sleeping { wake_tick: 37 }).unwrap(),
        "{\"Sleeping\":{\"wake_tick\":37}}"
    );
}
#[test]
fn exact_python_mock_projection_token_schema_roundtrips_without_defaults() {
    // Produced by the finalized helper over its AUTHORED mock-memory fixture.
    // Real Program/resident/provider qualification is deliberately not claimed.
    let raw: serde_json::Value = serde_json::from_str(include_str!("token.authored.json")).unwrap();
    let token: Token = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(&token).unwrap(), raw);
    let adapter = AdapterOwnership {
        fresh: FreshUnpublishedRequestEpoch {
            source_key: token.adapter_epoch_source.clone(),
            ..fresh()
        },
        retained_construction_owners: vec![],
        retained_native_consumers: vec![],
        non_group_objects: vec![token.object],
        construction_ownership_source: token.construction_ownership_source.clone(),
        native_consumer_ownership_source: token.native_consumer_ownership_source.clone(),
        group_ownership_source: token.group_ownership_source.clone(),
    };
    token.validate(&adapter).unwrap();
    for key in [
        "boundary",
        "source_witness_sha256",
        "adapter_epoch_source",
        "frame_shapes",
    ] {
        let mut missing = raw.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<Token>(missing).is_err(), "{key}");
    }
    let mut unknown = raw;
    unknown["wake_tick"] = 38.into();
    assert!(serde_json::from_value::<Token>(unknown).is_err());
}
#[test]
fn source_boundary_adapter_and_initial_scalar_mismatches_are_read_only_errors() {
    let p = program();
    let s = stack(&p);
    let w = world();
    for field in 0..16 {
        let mut t = token();
        let mut a = adapter();
        match field {
            0 => t.profile.push('x'),
            1 => t.boundary.eip = 0x415e20,
            2 => t.boundary.advanced = 0,
            3 => t.source_witness_sha256 = "0".repeat(64),
            4 => t.source_binding.context += 4,
            5 => t.source_binding.guid += 1,
            6 => t.cached_init = 4100,
            7 => t.frame_shapes[1].tree = 280,
            8 => t.native_reentry.argument_value += 1,
            9 => {
                t.native_reentry.countdown += 1;
                t.native_reentry.registry_word += 1;
            }
            10 => a.fresh.accepted_commands = 1,
            11 => a.retained_construction_owners.push(1),
            12 => a.retained_native_consumers.push(1),
            13 => a.non_group_objects.clear(),
            14 => t.boundary.tick += 1,
            _ => t.native_reentry.wake_tick = "38".into(),
        }
        assert!(
            s.registry_suspension(&p, &w, 37, t, a).is_err(),
            "field {field}"
        );
    }
    assert_eq!(s, stack(&p));
    assert_eq!(w, world());
    for field in 0..6 {
        let mut w = world();
        match field {
            0 => w.objects.get_mut(&1).unwrap().person = Some(BTreeMap::new()),
            1 => w.objects.get_mut(&1).unwrap().native_flags = None,
            2 => w.objects.get_mut(&1).unwrap().native_tree_error = None,
            3 => w.objects.get_mut(&1).unwrap().native_tree_error = Some(2),
            4 => {
                w.objects
                    .get_mut(&1)
                    .unwrap()
                    .definition
                    .as_mut()
                    .unwrap()
                    .guid += 1
            }
            _ => w.objects.get_mut(&1).unwrap().idle = None,
        }
        assert!(
            s.registry_suspension(&p, &w, 37, token(), adapter())
                .is_err(),
            "World {field}"
        );
    }
    let mut p2 = p.clone();
    p2.modules
        .get_mut("private")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions[0]
        .opcode = 2;
    assert!(
        s.registry_suspension(&p2, &w, 37, token(), adapter())
            .unwrap_err()
            .contains("pure direct")
    );
    assert!(
        s.native_suspension(
            &p,
            crate::native_suspension::Evidence {
                image_sha256: IMAGE.into(),
                entry_pc: 0x415e20,
                caller_return: 0x415f92,
                entry_code: [0; 32]
            }
        )
        .is_err()
    ); // No three-frame C0 conversion.
}
#[test]
fn generic_execution_and_ineligible_tick_leave_every_owner_unchanged() {
    let p = program();
    let mut v = vm(&p);
    let before = v.clone();
    for budget in [0, 1, 100] {
        assert!(v.run_slice(&p, 1, budget).is_err());
        assert_eq!(v, before);
    }
    assert!(v.advance_tick().is_err());
    assert_eq!(v, before);
    assert!(v.begin_nested_object_visit(1).is_err());
    assert_eq!(v, before);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.run_round(&p, 1))).is_err());
    assert_eq!(v, before);
    assert!(v.begin_scheduled_object_visit(1).is_err());
    assert_eq!(v, before);
    for field in 0..3 {
        let mut v = before.clone();
        let o = v.world.objects.get_mut(&1).unwrap();
        o.idle.as_mut().unwrap().countdown = -1;
        match field {
            0 => o.idle.as_mut().unwrap().flags = 0,
            1 => o.native_tree_error = None,
            _ => o.native_tree_error = Some(2),
        }
        let same = v.clone();
        assert!(v.begin_scheduled_object_visit(1).is_err());
        assert_eq!(v, same);
    }
}
#[test]
fn actual_registry_decrements_then_eligible_tick_consumes_the_distinct_owner() {
    let p = program();
    let mut v = vm(&p);
    let original_rng = rng(&v);
    let mut cursor = 0;
    for count in [1, 0] {
        let input = Inputs {
            paused: 0,
            mode: 0,
            lot_number: 0,
            tick: v.tick as u32,
        };
        let mut pass = Pass::begin(input, &Borrowed::new(&mut v).unwrap(), &mut cursor).unwrap();
        assert_eq!(
            pass.advance(&mut Borrowed::new(&mut v).unwrap(), &mut cursor, input)
                .unwrap(),
            None
        );
        assert_eq!(v.world.objects[&1].idle.as_ref().unwrap().countdown, count);
        assert!(matches!(v.threads[&1].status, Status::RegistrySuspended(_)));
        assert!(v.finish_registry_pass(&pass).unwrap());
        v = cold(&p, &v);
    }
    let input = Inputs {
        paused: 0,
        mode: 0,
        lot_number: 0,
        tick: v.tick as u32,
    };
    let mut pass = Pass::begin(input, &Borrowed::new(&mut v).unwrap(), &mut cursor).unwrap();
    let request = pass
        .advance(&mut Borrowed::new(&mut v).unwrap(), &mut cursor, input)
        .unwrap()
        .unwrap();
    assert!(matches!(request, Request::Tick { .. }));
    v.begin_scheduled_object_visit(1).unwrap();
    assert_eq!(v.threads[&1].status, Status::Ready);
    v = cold(&p, &v);
    let run = v.run_slice(&p, 1, 1).unwrap();
    assert_eq!(run.trace.len(), 1);
    assert_eq!(rng(&v), original_rng);
    assert_eq!(pass.request(), Some(&request));
    // This VM-only component has not supplied the real native Tick's host tail.
}
fn start() -> Start<'static> {
    Start {
        owner: "private".into(),
        bhav: 4098,
        caller: 1,
        callee: 1,
        stack_object: 1,
        args: vec![],
        check: false,
    }
}
#[test]
fn immediate_callback_retains_prefix_and_reset_and_wake_do_not_create_deadlines() {
    let p = program();
    let mut woke = vm(&p);
    let mut current = woke.threads[&1].clone();
    woke.wake_object(&p, &mut current, 1).unwrap();
    assert_eq!(woke.threads[&1].stack.last().unwrap().args[0], 2);
    let Status::RegistrySuspended(owner) = &woke.threads[&1].status else {
        panic!("lost registry owner");
    };
    assert_eq!(owner.token().native_reentry.argument_value, 10);
    cold(&p, &woke);
    let mut warm = vm(&p);
    let initial_rng = rng(&warm);
    let mut other = cold(&p, &warm);
    warm.begin_immediate_callback(&p, 1, start()).unwrap();
    other.begin_immediate_callback(&p, 1, start()).unwrap();
    assert_eq!(warm, other);
    other = cold(&p, &other);
    let before = warm.clone();
    assert!(warm.finish_loaded_reset(1).is_err());
    assert_eq!(warm, before);
    assert_eq!(
        warm.run_slice(&p, 1, 1).unwrap(),
        other.run_slice(&p, 1, 1).unwrap()
    );
    assert_eq!(warm, other);
    other = cold(&p, &other);
    assert_eq!(warm, other);
    assert!(matches!(
        warm.threads[&1].status,
        Status::RegistrySuspended(_)
    ));
    warm.finish_loaded_reset(1).unwrap();
    warm = cold(&p, &warm);
    assert_eq!(warm.world.objects[&1].idle.as_ref().unwrap().countdown, -1);
    let mut current = warm.threads[&1].clone();
    warm.wake_object(&p, &mut current, 1).unwrap();
    warm = cold(&p, &warm);
    assert_eq!(rng(&warm), initial_rng);
    assert!(matches!(
        warm.threads[&1].status,
        Status::RegistrySuspended(_)
    ));
    assert!(warm.run_slice(&p, 1, 1).is_err());
}
#[test]
fn cold_restore_rejects_forged_source_lifetime_lifecycle_callback_and_service_owners() {
    let p = program();
    let v = vm(&p);
    for field in 0..10 {
        let mut raw = serde_json::to_value(Snapshot::capture(&p, &v)).unwrap();
        let t = &mut raw["state"]["threads"]["1"];
        let owner = &mut t["status"]["RegistrySuspended"];
        match field {
            0 => owner["token"]["boundary"]["eip"] = 0.into(),
            1 => owner["binding"]["registry_lifetime"] = 2.into(),
            2 => owner["token"]["frame_shapes"][1]["node"] = 1.into(),
            3 => owner["adapter"]["fresh"]["accepted_commands"] = 1.into(),
            4 => t["check"] = true.into(),
            5 => raw["state"]["lifecycles"]
                .as_object_mut()
                .unwrap()
                .remove("1")
                .map(|_| ())
                .unwrap(),
            6 => raw["state"]["lifecycles"]["1"]["run_init"] = true.into(),
            7 => raw["state"]["object_threads"]["1"] = 2.into(),
            8 => raw["state"]["tick"] = 36.into(),
            _ => {
                raw["state"]["world"]["objects"]["1"]["native_tree_error"] = serde_json::Value::Null
            }
        }
        let rejected = match serde_json::from_value::<Snapshot>(raw) {
            Err(_) => true,
            Ok(saved) => saved.restore(&p).is_err(),
        };
        assert!(rejected, "field {field}");
    }
    let mut callback = v.clone();
    let before = callback.clone();
    assert!(callback.push_service_call(&p, 1, start()).is_err());
    assert_eq!(callback, before);
    callback.begin_immediate_callback(&p, 1, start()).unwrap();
    callback
        .threads
        .get_mut(&1)
        .unwrap()
        .callback
        .as_mut()
        .unwrap()
        .resume_check = true;
    assert!(Snapshot::capture(&p, &callback).restore(&p).is_err());
    let mut service = v.clone();
    let owner = service.threads[&1].status.clone();
    service.threads.get_mut(&1).unwrap().status = Status::Ready;
    service.threads.get_mut(&1).unwrap().service = Some(ServiceCall {
        parent: None,
        base_depth: 3,
        resume_status: owner,
        owner: "private".into(),
        bhav: 4098,
        caller: 1,
        callee: 1,
        result: None,
    });
    assert!(
        Snapshot::capture(&p, &service)
            .restore(&p)
            .unwrap_err()
            .contains("target-service parent is unsupported")
    );
    assert_eq!(cold(&p, &v), v);
}
