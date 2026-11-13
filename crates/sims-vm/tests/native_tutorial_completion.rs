use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Object, Program, Routine, Snapshot, Start, Status, Vm, World};

fn program(operands: [u8; 8]) -> Program {
    Program {
        modules: [(
            "tutorial".into(),
            Module {
                source: "tutorial".into(),
                routines: [(
                    4096,
                    Routine {
                        name: "finish tutorial".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 1,
                                on_true: 254,
                                on_false: 255,
                                operands,
                            }],
                        },
                    },
                )]
                .into(),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    }
}

fn world() -> World {
    World {
        globals: [(19, 3), (26, 1), (31, 47)].into(),
        neighborhood: [(0, 71), (1, 3), (2, 7), (3, -9), (15, 54)].into(),
        objects: [
            (322, Object::default()),
            (
                134,
                Object {
                    person: Some([(29, 1), (36, 0), (61, 1), (67, 0)].into()),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        ..Default::default()
    }
}

fn start(vm: &mut Vm, program: &Program) -> u64 {
    vm.spawn(
        program,
        Start {
            owner: "tutorial",
            bhav: 4096,
            caller: 322,
            callee: 322,
            stack_object: 0,
            args: vec![],
            check: false,
        },
    )
    .unwrap()
}

fn roundtrip(program: &Program, vm: &Vm) -> Vm {
    let saved = Snapshot::capture(program, vm);
    let bytes = serde_json::to_vec(&saved).unwrap();
    let restored = serde_json::from_slice::<Snapshot>(&bytes)
        .unwrap()
        .restore(program)
        .unwrap();
    assert_eq!(&restored, vm);
    restored
}

#[test]
fn tutorial_completion_updates_only_the_native_lot_and_neighborhood_words() {
    for operands in [[0; 8], [0, 0, 255, 255, 128, 1, 22, 7]] {
        let program = program(operands);
        let mut vm = Vm::new(world(), 2571507393);
        vm.tick = 201111;
        let thread = start(&mut vm, &program);
        let before = vm.clone();
        let mut expected = before.world.clone();
        expected.globals.insert(26, 0);
        expected.neighborhood.insert(1, 1);
        expected.neighborhood.insert(2, 0);
        let mut restored = roundtrip(&program, &vm);
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap(),
            restored.run_slice(&program, thread, 1).unwrap()
        );
        assert_eq!(vm, restored);
        assert_eq!(vm.world, expected);
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        assert_eq!(vm.tick, before.tick);
        assert_eq!(vm.rng_state, before.rng_state);
        assert_eq!(vm.lot_rng_state, before.lot_rng_state);
        assert_eq!(vm.crt_rng_state, before.crt_rng_state);
        roundtrip(&program, &vm);
    }
}

#[test]
fn tutorial_completion_is_idempotent_without_fabricating_resident_cleanup() {
    let program = program([0; 8]);
    let mut vm = Vm::new(world(), 17);
    for _ in 0..2 {
        let before = vm.world.objects.clone();
        let thread = start(&mut vm, &program);
        vm.run_slice(&program, thread, 1).unwrap();
        assert_eq!(vm.threads[&thread].status, Status::Returned(true));
        assert_eq!(vm.world.objects, before);
        assert_eq!(vm.world.globals[&19], 3);
        assert_eq!(vm.world.globals[&26], 0);
        assert_eq!(vm.world.neighborhood[&1], 1);
        assert_eq!(vm.world.neighborhood[&2], 0);
        assert_eq!(vm.world.objects[&134].person.as_ref().unwrap()[&29], 1);
        assert_eq!(vm.world.objects[&134].person.as_ref().unwrap()[&36], 0);
        assert_eq!(vm.world.ui_owner, Default::default());
        roundtrip(&program, &vm);
    }
}

#[test]
fn tutorial_completion_rejects_incomplete_storage_before_any_world_write() {
    let program = program([0; 8]);
    for missing in [26, 1, 2] {
        let mut world = world();
        if missing == 26 {
            world.globals.remove(&missing);
        } else {
            world.neighborhood.remove(&missing);
        }
        let mut vm = Vm::new(world, 29);
        let thread = start(&mut vm, &program);
        let before = vm.clone();
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap().stop,
            sims_vm::Stop::Faulted
        );
        assert!(matches!(&vm.threads[&thread].status, Status::Faulted(fault)
            if fault.message.contains("tutorial completion storage is incomplete")));
        assert_eq!(vm.world, before.world);
        assert_eq!(vm.tick, before.tick);
        assert_eq!(vm.rng_state, before.rng_state);
        assert_eq!(vm.lot_rng_state, before.lot_rng_state);
        assert_eq!(vm.crt_rng_state, before.crt_rng_state);
        assert!(matches!(vm.threads[&thread].status, Status::Faulted { .. }));
    }
}

#[test]
fn tutorial_completion_does_not_alias_other_generic_selectors() {
    for operands in [[0, 1, 0, 0, 0, 0, 0, 0], [8, 0, 0, 0, 0, 0, 0, 0]] {
        let program = program(operands);
        let mut vm = Vm::new(world(), 31);
        let thread = start(&mut vm, &program);
        let before = vm.world.clone();
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap().stop,
            sims_vm::Stop::Faulted
        );
        assert!(matches!(&vm.threads[&thread].status, Status::Faulted(fault)
            if fault.message.contains("unsupported Generic Sims Call")));
        assert_eq!(vm.world, before);
    }
}
