//! Authored VM ownership controls for native45ad63's state2 portal boundary.
//! Native source/state witnesses live in the separately retained Goth fixture.
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    Definition, Module, Object, Program, Routine, Snapshot, Start, Status, Stop, Vm, World, action,
};
use std::collections::BTreeMap;

fn routine(opcode: u16, operands: [u8; 8]) -> Routine {
    Routine {
        name: "authored ownership control".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions: vec![Instruction {
                opcode,
                on_true: 254,
                on_false: 255,
                operands,
            }],
        },
    }
}
fn boundary() -> (Program, Vm, u64) {
    let mut p = Program {
        globals: "actor".into(),
        ..Program::default()
    };
    p.modules.insert(
        "actor".into(),
        Module {
            source: "actor".into(),
            routines: BTreeMap::from([(4096, routine(27, [0, 0, 254, 255, 0, 0, 2, 0]))]),
            ..Module::default()
        },
    );
    let mut words = vec![0; 108];
    words[9] = 8;
    words[14] = 3;
    p.modules.insert(
        "portal".into(),
        Module {
            source: "portal".into(),
            routines: BTreeMap::from([(4100, routine(0, [0; 8]))]),
            object_definition_words: BTreeMap::from([(1, words)]),
            ..Module::default()
        },
    );
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            person: Some(BTreeMap::from([(65, 0)])),
            ..Object::default()
        },
    );
    world.objects.insert(2, Object::default());
    world.objects.insert(
        3,
        Object {
            definition: Some(Definition {
                source: "portal".into(),
                object: 1,
                guid: 3,
                slots: 0,
            }),
            data: BTreeMap::from([(62, 0)]),
            ..Object::default()
        },
    );
    world.actions = Some(action::WorldState {
        actors: BTreeMap::from([(1, action::Actor::default())]),
        ..action::WorldState::default()
    });
    let mut vm = Vm::new(world, 77);
    let id = vm
        .spawn(
            &p,
            Start {
                owner: "actor",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 2,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    assert_eq!(vm.run_slice(&p, id, 1).unwrap().stop, Stop::AwaitingRoute);
    (p, vm, id)
}

#[test]
fn native_portal_state_two_survives_save_and_requires_its_route_service() {
    let (p, mut vm, id) = boundary();
    vm.push_native_portal_service(&p, id, "portal".into(), 4100, 3)
        .unwrap();
    assert_eq!(vm.threads[&id].stack[0].continuation, 2);
    assert_eq!(vm.threads[&id].stack[1].action_target, Some(3));
    let restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
    assert_eq!(restored, vm);
    let mut unowned = vm.clone();
    unowned.threads.get_mut(&id).unwrap().service = None;
    assert!(
        Snapshot::capture(&p, &unowned)
            .restore(&p)
            .unwrap_err()
            .contains("continuation")
    );
    let mut wrong = vm.clone();
    wrong
        .threads
        .get_mut(&id)
        .unwrap()
        .service
        .as_mut()
        .unwrap()
        .resume_status = Status::Ready;
    assert!(
        Snapshot::capture(&p, &wrong)
            .restore(&p)
            .unwrap_err()
            .contains("continuation")
    );
}

#[test]
fn generic_route_service_keeps_its_existing_state_and_failed_native_entry_is_atomic() {
    let (p, mut vm, id) = boundary();
    let before = vm.clone();
    assert!(
        vm.push_native_portal_service(&p, id, "portal".into(), 4999, 3)
            .is_err()
    );
    assert_eq!(vm, before);
    vm.push_portal_service(&p, id, "portal".into(), 4100, 3)
        .unwrap();
    assert_eq!(vm.threads[&id].stack[0].continuation, 0);
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
}

#[test]
fn returned_portal_keeps_state_two_owned_by_the_existing_request() {
    let (mut p, mut vm, id) = boundary();
    p.modules
        .get_mut("portal")
        .unwrap()
        .routines
        .insert(4100, routine(2, [0, 0, 0, 0, 0, 2, 7, 7]));
    vm.push_native_portal_service(&p, id, "portal".into(), 4100, 3)
        .unwrap();
    let mut resumed = Snapshot::capture(&p, &vm).restore(&p).unwrap();
    assert_eq!(
        vm.run_slice(&p, id, 1).unwrap(),
        resumed.run_slice(&p, id, 1).unwrap()
    );
    assert_eq!(vm, resumed);
    assert_eq!(vm.threads[&id].service.as_ref().unwrap().result, Some(true));
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    vm.pop_service(id).unwrap();
    assert_eq!(vm.threads[&id].stack[0].continuation, 2);
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    let mut bare = vm.clone();
    bare.threads.get_mut(&id).unwrap().status = Status::Ready;
    assert!(
        Snapshot::capture(&p, &bare)
            .restore(&p)
            .unwrap_err()
            .contains("continuation")
    );
    let Status::AwaitingRoute(r) = &mut vm.threads.get_mut(&id).unwrap().status else {
        panic!("route return expected")
    };
    r.target = 99;
    assert!(Snapshot::capture(&p, &vm).restore(&p).is_err());
}
