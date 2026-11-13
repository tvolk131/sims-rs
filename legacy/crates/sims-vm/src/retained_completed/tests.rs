//! Authored projection protocol tests, never observed native-baseline parity.
use super::*;
use crate::{
    Definition, Frame, Location, Module, Object, Routine,
    lifecycle::{Phase, ServiceCall},
    program::MenuEntry,
    retained_frames::{ContextBinding, RetainedFrame},
    scheduler::storage::Storage,
};
use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::ObjectFunction,
};

fn instruction(opcode: u16) -> Instruction {
    Instruction {
        opcode,
        on_true: 0,
        on_false: 253,
        operands: [0; 8],
    }
}
fn routine(args: u8, opcode: u16) -> Routine {
    Routine {
        name: "AUTHORED completed projection".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals: 0,
            instructions: vec![instruction(opcode)],
        },
    }
}
fn definition(id: u16) -> Definition {
    Definition {
        source: "private".into(),
        object: id,
        guid: u32::from(id) + 6,
        slots: 0,
    }
}
fn program() -> Program {
    let mut module = Module {
        source: "private".into(),
        ..Default::default()
    };
    for id in 1..=3 {
        let mut words = vec![0; 108];
        words[9] = if id == 1 { 2 } else { 4 };
        words[14] = id as i16 + 6;
        if id > 1 {
            words[10] = 1;
            words[11] = if id == 3 { 1 } else { 0 };
        }
        module.object_definition_words.insert(id, words);
        let mut table = vec![
            ObjectFunction {
                action: 0,
                condition: 0
            };
            30
        ];
        table[0].action = if id == 1 { 4099 } else { 4101 };
        table[1].action = if id == 1 { 4096 } else { 4100 };
        module.object_functions.insert(id, table);
        module.object_interactions.insert(
            id,
            vec![MenuEntry {
                id: 12,
                action: 4097,
                label: "authored action".into(),
                ..Default::default()
            }],
        );
    }
    module.object_definition_order = vec![1, 2, 3];
    module.routines = BTreeMap::from([
        (4096, routine(4, 17)),
        (4097, routine(0, 0)),
        (4098, routine(0, 0)),
        (4099, routine(0, 2)),
        (4100, routine(0, 0)),
        (4101, routine(0, 2)),
    ]);
    Program {
        globals: "private".into(),
        modules: BTreeMap::from([("private".into(), module)]),
        ..Default::default()
    }
}
fn frame(
    object: ObjectId,
    callee: ObjectId,
    bhav: u16,
    offset: usize,
    args: Vec<i16>,
) -> RetainedFrame {
    RetainedFrame {
        context_object: callee,
        frame: Frame {
            continuation: 0,
            action_target: None,
            action_icon: false,
            code_owner: "private".into(),
            location: Location {
                source: "private".into(),
                bhav,
                instruction: 0,
            },
            caller: object,
            callee,
            stack_object: callee,
            args,
            locals: vec![],
            opaque_locals: Default::default(),
            storage_offset: offset,
        },
    }
}
fn stack(p: &Program, frames: Vec<RetainedFrame>) -> RetainedStack {
    let mut words = vec![0x5a5ai16; 128];
    let mut end = 0;
    for rf in &frames {
        if rf.frame.storage_offset == end + 8 {
            words[end..end + 8].copy_from_slice(&[-1, 0, 0, 0, 1, 0, 0, 0]);
        }
        let context = 0x9876_0000 + rf.context_object as u32 * 0x111;
        let mut header = crate::frame_header(&rf.frame);
        header[6] = Some(context as i16);
        header[7] = Some((context >> 16) as i16);
        for (i, value) in header.into_iter().enumerate() {
            words[rf.frame.storage_offset + i] = value.unwrap();
        }
        for (i, value) in rf.frame.args.iter().chain(&rf.frame.locals).enumerate() {
            words[rf.frame.storage_offset + 8 + i] = *value;
        }
        end = rf.frame.storage_offset
            + crate::frame_words(rf.frame.args.len(), rf.frame.locals.len());
    }
    RetainedStack::from_bytes(
        p,
        &words
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>(),
        frames,
        (1..=3)
            .map(|id| ContextBinding {
                object: id,
                definition: definition(id as u16),
                native_context: 0x9876_0000 + id as u32 * 0x111,
                registry_lifetime: Some(id as u32 * 101),
            })
            .collect(),
    )
    .unwrap()
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
        interrupted: true,
        status,
    }
}
type Fixture = (
    Program,
    World,
    VmProjection,
    Vec<CompletedThreadInput>,
    Vec<ObjectThreadBinding>,
    CompletedAdapterEpoch,
);
fn fixture() -> Fixture {
    let p = program();
    let mut world = World::default();
    for id in 1..=3 {
        let mut object = Object {
            definition: Some(definition(id as u16)),
            data: (0..128).map(|i| (i, 0)).collect(),
            native_tree_error: Some(0),
            native_flags: Some(0x280),
            native_keep_dynamic: Some(0),
            temps: [11, 12, 13, 14, 15, 16, 17, 18],
            idle: Some(crate::idle::IdleState {
                countdown: 3,
                flags: 1,
                last_run: Some(999),
            }),
            last_update: Some(999),
            ..Default::default()
        };
        if id == 1 {
            object.person = Some(
                (0..256)
                    .map(|i| (i, if i == 13 { 600 } else { 0 }))
                    .collect(),
            );
        } else {
            object.multitile = Some(crate::multitile::Part {
                head: if id == 3 { Some(2) } else { None },
                next: if id == 2 { Some(3) } else { None },
                origin: if id == 3 { [0, 1, 0] } else { [0; 3] },
                offset: if id == 3 { [0, 1, 0] } else { [0; 3] },
            });
        }
        world.objects.insert(id, object);
    }
    world.scheduler_storage = Some(
        Storage::from_retained(
            &world,
            vec![Some(101), Some(202), Some(303)],
            vec![0x10003, 0x10003, 0x10003, 0xdead_beef, 0],
            303,
        )
        .unwrap(),
    );
    let action = crate::action::Action {
        id: 0xfeed_beef,
        target: 2,
        icon_owner: 2,
        menu_id: 12,
        routine: 4097,
        label: Some("native serial stays independent".into()),
        ..Default::default()
    };
    world.actions = Some(crate::action::WorldState {
        serial: 0xfeed_beef,
        actors: BTreeMap::from([(
            1,
            crate::action::Actor {
                active: Some(action.clone()),
                queue: crate::action::Queue {
                    pending: std::collections::VecDeque::from([crate::action::Action {
                        id: 0xfeed_beee,
                        ..action
                    }]),
                },
                ..Default::default()
            },
        )]),
        ..Default::default()
    });
    let mut main = frame(1, 1, 4096, 0, vec![21, 22, 23, 24]);
    main.frame.continuation = 1;
    let mut use_frame = frame(1, 2, 4097, 12, vec![]);
    use_frame.frame.action_target = Some(2);
    let person = CompletedThreadInput {
        source_owner: "authored native person owner".into(),
        adapter_thread: 7,
        object: Some(1),
        stack: stack(&p, vec![main, use_frame]),
        controls: controls(Status::Sleeping { wake_tick: 1200 }),
    };
    let mut object_controls = controls(Status::Sleeping { wake_tick: 1300 });
    object_controls.service = Some(ServiceCall {
        parent: None,
        base_depth: 1,
        resume_status: Status::Sleeping { wake_tick: 1100 },
        owner: "private".into(),
        bhav: 4098,
        caller: 2,
        callee: 2,
        result: None,
    });
    let object = CompletedThreadInput {
        source_owner: "authored native group owner".into(),
        adapter_thread: 11,
        object: Some(2),
        stack: stack(
            &p,
            vec![frame(2, 2, 4100, 0, vec![]), frame(2, 2, 4098, 8, vec![])],
        ),
        controls: object_controls,
    };
    let bindings = (1..=3)
        .map(|object| ObjectThreadBinding {
            object,
            thread: match object {
                1 => Some(7),
                2 => Some(11),
                _ => None,
            },
            lifecycle: match object {
                1 | 2 => Some(Lifecycle {
                    owner: "private".into(),
                    definition: object as u16,
                    init: if object == 1 { 4099 } else { 4101 },
                    main: if object == 1 { 4096 } else { 4100 },
                    phase: Phase::Main,
                    thread: if object == 1 { 7 } else { 11 },
                    run_init: true,
                }),
                _ => None,
            },
        })
        .collect();
    let epoch = CompletedAdapterEpoch {
        fresh: FreshUnpublishedRequestEpoch {
            source_key: "AUTHORED completed boundary; no native authenticity claim".into(),
            accepted_commands: 0,
            published_thread_handles: 0,
            published_request_handles: 0,
        },
        threads: vec![
            IdentityMapping {
                source_owner: person.source_owner.clone(),
                adapter: 7,
            },
            IdentityMapping {
                source_owner: object.source_owner.clone(),
                adapter: 11,
            },
        ],
        requests: vec![],
        next_thread: 30,
        next_request: 50,
    };
    let projection = VmProjection {
        tick: 1000,
        rng_state: 0x1234_5678,
        lot_rng_state: Some(0x4567_89ab),
        crt_rng_state: Some(0xabcd_0123),
        scheduled_object_visit: None,
        player_menu: None,
        lot_rng_scope: None,
        named_calls: vec![],
        native_accessory_host: true,
        sounds: vec![
            crate::SoundEvent {
                tick: 999,
                object: 1,
                stop: false,
                name: "authored.wav".into(),
                flags: 4,
                sample_rate: 22050,
                volume: 77
            };
            2
        ],
        effects: vec![
            crate::SpecialEffect {
                tick: 999,
                object: 2,
                duration_ms: 1000,
                kind: 1,
                size: 2,
                flags: 3,
                text: "authored ordered presentation".into()
            };
            2
        ],
    };
    (p, world, projection, vec![person, object], bindings, epoch)
}
fn assemble(f: Fixture) -> Result<Vm, String> {
    let (p, w, v, t, b, e) = f;
    Vm::from_retained_completed_epoch(&p, w, v, t, b, e)
}
fn rejected(f: Fixture) -> String {
    let (p, w, v, t, b, e) = f;
    let before = w.clone();
    let fp = p.fingerprint();
    let result = Vm::from_retained_completed_epoch(&p, w.clone(), v, t, b, e);
    assert_eq!(
        w, before,
        "owned rejection mutated caller's authored source World"
    );
    assert_eq!(p.fingerprint(), fp);
    result.unwrap_err()
}
fn cold(p: &Program, v: &Vm) {
    let s = Snapshot::capture(p, v);
    let raw = serde_json::to_vec(&s).unwrap();
    let r = serde_json::from_slice::<Snapshot>(&raw)
        .unwrap()
        .restore(p)
        .unwrap();
    assert_eq!(&r, v);
}

#[test]
fn supplied_people_group_actions_services_and_all_state_survive_without_execution() {
    let (p, w, c, t, b, e) = fixture();
    let before = w.clone();
    let raw = t.iter().map(|t| t.stack.raw_bytes()).collect::<Vec<_>>();
    let v = Vm::from_retained_completed_epoch(&p, w, c.clone(), t.clone(), b.clone(), e.clone())
        .unwrap();
    assert_eq!(v.world, before);
    assert_eq!(
        (v.tick, v.rng_state, v.lot_rng_state, v.crt_rng_state),
        (c.tick, c.rng_state, c.lot_rng_state, c.crt_rng_state)
    );
    assert_eq!(v.next_thread, 30);
    assert_eq!(v.next_request, 50);
    assert_eq!(v.sounds, c.sounds);
    assert_eq!(v.effects, c.effects);
    for binding in &b {
        assert_eq!(
            v.object_threads.get(&binding.object),
            binding.thread.as_ref()
        );
        assert_eq!(
            v.lifecycles.get(&binding.object),
            binding.lifecycle.as_ref()
        );
    }
    for (input, raw) in t.iter().zip(raw) {
        let restored = &v.threads[&input.adapter_thread];
        assert_eq!(
            restored,
            &input
                .stack
                .clone()
                .into_thread(&p, input.controls.clone())
                .unwrap()
        );
        assert_eq!(
            restored
                .frame_memory
                .iter()
                .flat_map(|w| w.unwrap().to_le_bytes())
                .collect::<Vec<_>>(),
            raw
        );
    }
    assert_eq!(v.world.actions.as_ref().unwrap().serial, 0xfeed_beef);
    assert_eq!(
        serde_json::to_value(&v.world.scheduler_storage).unwrap()["last_handle"],
        303
    );
    cold(&p, &v);
}

#[test]
fn supplied_finished_phase_and_run_init_are_not_replaced_by_main_defaults() {
    let (mut p, w, c, mut t, mut b, e) = fixture();
    // Deliberately no thread frame can execute this constructor/Init body.
    p.modules
        .get_mut("private")
        .unwrap()
        .routines
        .get_mut(&4101)
        .unwrap()
        .body
        .instructions[0]
        .opcode = 0xffff;
    t[1].stack = stack(&p, vec![]);
    t[1].controls = controls(Status::Returned(false));
    b[1].lifecycle.as_mut().unwrap().phase = Phase::Finished;
    b[1].lifecycle.as_mut().unwrap().run_init = false;
    let v = Vm::from_retained_completed_epoch(&p, w, c, t, b, e).unwrap();
    assert_eq!(v.lifecycles[&2].phase, Phase::Finished);
    assert!(!v.lifecycles[&2].run_init);
    assert!(v.threads[&11].stack.is_empty());
    cold(&p, &v);
}

#[test]
fn supported_suspended_request_uses_only_its_explicit_private_mapping() {
    let (mut p, w, c, mut t, b, mut e) = fixture();
    p.modules
        .get_mut("private")
        .unwrap()
        .routines
        .get_mut(&4100)
        .unwrap()
        .body
        .instructions[0]
        .opcode = 0x2c;
    t[1].stack = stack(&p, vec![frame(2, 2, 4100, 0, vec![])]);
    t[1].controls = controls(Status::AwaitingAnimation(crate::AnimationRequest {
        id: 17,
        origin: Location {
            source: "private".into(),
            bhav: 4100,
            instruction: 0,
        },
        caller: 2,
        operands: [0; 8],
    }));
    e.requests.push(IdentityMapping {
        source_owner: "authored suspended animator owner".into(),
        adapter: 17,
    });
    let f = (p, w, c, t, b, e);
    let mut missing = f.clone();
    missing.5.requests.clear();
    assert!(rejected(missing).contains("request mapping differs"));
    let mut duplicate = f.clone();
    duplicate.5.requests.push(duplicate.5.requests[0].clone());
    assert!(rejected(duplicate).contains("identity mapping"));
    let (p, w, c, t, b, e) = f;
    let v = Vm::from_retained_completed_epoch(&p, w, c, t, b, e).unwrap();
    assert!(matches!(&v.threads[&11].status,Status::AwaitingAnimation(r) if r.id==17));
    assert_eq!(v.next_request, 50);
    cold(&p, &v);
}

#[test]
fn missing_duplicate_unknown_mapping_and_association_inputs_reject_atomically() {
    let mut f = fixture();
    f.3.pop();
    assert!(rejected(f).contains("missing input"));
    let mut f = fixture();
    f.3.push(f.3[0].clone());
    assert!(rejected(f).contains("duplicate/unknown"));
    let mut f = fixture();
    f.5.threads.push(f.5.threads[0].clone());
    assert!(rejected(f).contains("identity mapping"));
    let mut f = fixture();
    f.5.threads[1].source_owner = f.5.threads[0].source_owner.clone();
    assert!(rejected(f).contains("identity mapping"));
    let mut f = fixture();
    f.3[0].source_owner = "unknown actual owner".into();
    assert!(rejected(f).contains("unknown"));
    let mut f = fixture();
    f.4.pop();
    assert!(rejected(f).contains("coverage"));
    let mut f = fixture();
    f.4.push(f.4[0].clone());
    assert!(rejected(f).contains("object binding"));
    let mut f = fixture();
    f.4[0].thread = Some(29);
    assert!(rejected(f).contains("unknown/different"));
    let mut f = fixture();
    f.4[0].lifecycle.as_mut().unwrap().thread = 11;
    assert!(rejected(f).contains("lifecycle differs"));
    let mut f = fixture();
    f.3[0].object = Some(3);
    assert!(rejected(f).contains("unknown/different"));
    let mut f = fixture();
    f.5.requests.push(IdentityMapping {
        source_owner: "no retained request owner".into(),
        adapter: 9,
    });
    assert!(rejected(f).contains("request mapping differs"));
    let mut f = fixture();
    f.5.next_thread = 11;
    assert!(rejected(f).contains("identity mapping"));
    let mut f = fixture();
    f.5.next_request = 0;
    assert!(rejected(f).contains("counter"));
    let mut f = fixture();
    f.5.fresh.accepted_commands = 1;
    assert!(rejected(f).contains("unpublished"));
    let mut f = fixture();
    f.5.fresh.published_thread_handles = 1;
    assert!(rejected(f).contains("unpublished"));
    let mut f = fixture();
    f.5.fresh.published_request_handles = 1;
    assert!(rejected(f).contains("unpublished"));
}

#[test]
fn existing_program_world_frame_and_host_owner_guards_remain_active() {
    let mut f = fixture();
    f.1.objects
        .get_mut(&3)
        .unwrap()
        .multitile
        .as_mut()
        .unwrap()
        .origin = [9, 9, 9];
    assert!(rejected(f).contains("multi-tile"));
    let mut f = fixture();
    f.4[0].lifecycle.as_mut().unwrap().main = 4100;
    assert!(rejected(f).contains("lifecycle"));
    let mut f = fixture();
    f.4[0].lifecycle.as_mut().unwrap().phase = Phase::Init;
    assert!(rejected(f).contains("Init callback"));
    let mut f = fixture();
    f.3[0]
        .controls
        .completed_actions
        .push((1, 2, true, false, 0));
    assert!(rejected(f).contains("action frame returns"));
    let mut f = fixture();
    f.1.scheduler_storage = None;
    assert!(rejected(f).contains("storage is absent"));
    let mut f = fixture();
    f.1.objects.remove(&2);
    assert!(!rejected(f).is_empty());
    let mut f = fixture();
    f.2.lot_rng_scope=Some(serde_json::from_value(serde_json::json!({"tick":1000,"saved_global":305419896u32,"lot_at_entry":1164413355u32})).unwrap());
    assert!(rejected(f).contains("active scheduler/RNG"));
    let mut f = fixture();
    f.2.scheduled_object_visit = Some(
        serde_json::from_value(
            serde_json::json!({"parent":null,"object":1,"thread":7,"tick":1000,"finished":false}),
        )
        .unwrap(),
    );
    assert!(rejected(f).contains("active scheduler/RNG"));
    let mut f = fixture();
    let w = &mut f.1;
    w.scheduler_storage = Some(
        Storage::from_retained(
            w,
            vec![Some(100), Some(202), Some(303)],
            vec![0x10003, 0x10003, 0x10003, 0xdead_beef, 0],
            303,
        )
        .unwrap(),
    );
    assert!(rejected(f).contains("lifetime"));
    let mut f = fixture();
    f.2.lot_rng_state = None; // Absence is explicit; no seed is installed.
    let p = f.0.clone();
    let v = assemble(f).unwrap();
    assert_eq!(v.lot_rng_state, None);
    cold(&p, &v);
}

#[test]
fn association_absence_is_required_and_unknown_wire_fields_reject() {
    let value = serde_json::json!({"object":3,"thread":null,"lifecycle":null});
    assert!(serde_json::from_value::<ObjectThreadBinding>(value.clone()).is_ok());
    for name in ["thread", "lifecycle"] {
        let mut bad = value.clone();
        bad.as_object_mut().unwrap().remove(name);
        assert!(serde_json::from_value::<ObjectThreadBinding>(bad).is_err());
    }
    let mut bad = value;
    bad["guessed_main"] = serde_json::json!(false);
    assert!(serde_json::from_value::<ObjectThreadBinding>(bad).is_err());
}

fn callback_fixture(animation: bool) -> Fixture {
    let (mut p, w, c, mut t, b, mut e) = fixture();
    let ins = &mut p
        .modules
        .get_mut("private")
        .unwrap()
        .routines
        .get_mut(&4100)
        .unwrap()
        .body
        .instructions[0];
    ins.opcode = if animation { 0x2c } else { 0x16 };
    if !animation {
        ins.operands[0] = 1;
    }
    let mut parent = frame(2, 2, 4100, 0, vec![]);
    if !animation {
        parent.frame.continuation = 1;
    }
    t[1].stack = stack(&p, vec![parent, frame(2, 2, 4101, 16, vec![])]);
    t[1].controls = controls(Status::Ready);
    t[1].controls.check = true;
    let origin = Location {
        source: "private".into(),
        bhav: 4100,
        instruction: 0,
    };
    let status = if animation {
        Status::AwaitingAnimation(crate::AnimationRequest {
            id: 17,
            origin,
            caller: 2,
            operands: [0; 8],
        })
    } else {
        Status::AwaitingTurn(crate::TurnRequest {
            id: 17,
            origin,
            caller: 2,
            target: 2,
            mode: 1,
        })
    };
    t[1].controls.callback = Some(crate::CallbackBoundary {
        external: true,
        parent: None,
        base_depth: 1,
        resume_status: status,
        resume_check: false,
    });
    e.requests.push(IdentityMapping {
        source_owner: "authored callback parent request".into(),
        adapter: 17,
    });
    (p, w, c, t, b, e)
}

#[test]
fn validated_callback_parent_request_is_preserved_but_unvalidated_kind_stops_explicitly() {
    let f = callback_fixture(true);
    assert!(rejected(f).contains("unsupported completed callback-parent AwaitingAnimation"));
    let (p, w, c, t, b, e) = callback_fixture(false);
    let v = Vm::from_retained_completed_epoch(&p, w, c, t, b, e).unwrap();
    assert!(
        matches!(&v.threads[&11].callback.as_ref().unwrap().resume_status,Status::AwaitingTurn(r)if r.id==17)
    );
    cold(&p, &v);
}

#[test]
fn unknown_named_and_query_thread_owners_are_not_discarded() {
    let mut f = fixture();
    f.2.named_calls.push(crate::named::Call {
        requester: 29,
        recipient: 11,
        object: 2,
        depth: 1,
        owner: "private".into(),
        bhav: 4101,
        base_depth: 1,
        result: None,
    });
    assert!(rejected(f).contains("named callback requester absent"));
    let mut f = fixture();
    f.2.player_menu=Some(serde_json::from_value(serde_json::json!({"actor":1,"target":2,"thread":29,"tick":1000,"count":0,"next":0,"saved_temps":null,"waiting":null,"actions":[]})).unwrap());
    assert!(rejected(f).contains("player-menu thread absent"));
    let mut f = fixture();
    f.3[0].controls.find_best.push(crate::find_best::Query {
        depth: 0,
        caller: 1,
        reference: 2,
        mode: 15,
        flags: 0,
        room: None,
        order: vec![],
        cursor: 0,
        pending: None,
        best: None,
        best_score: 0.,
    });
    assert!(rejected(f).contains("Find Best"));
}
