use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{scheduler::storage::Storage, *};
use std::collections::BTreeMap;

fn insert(world: &mut World, id: i16) {
    let mut object = Object {
        native_tree_error: Some(0),
        data: BTreeMap::from([(25, 9)]),
        ..Default::default()
    };
    world.register_scheduler_object(id, &mut object).unwrap();
    world.objects.insert(id, object);
}
#[test]
fn registry_holes_and_tail_words_survive_removal_and_reuse() {
    let mut world = World {
        scheduler_storage: Some(Storage::default()),
        ..Default::default()
    };
    insert(&mut world, 4);
    insert(&mut world, 2);
    world.validate_scheduler_storage().unwrap();
    let storage = world.scheduler_storage.as_ref().unwrap();
    assert_eq!(storage.len(), 4);
    assert_eq!(
        (
            storage.word(&world, 1).unwrap(),
            storage.word(&world, 3).unwrap()
        ),
        (0, 0)
    );
    assert_eq!(storage.word(&world, 4).unwrap(), 0xffff);
    world
        .objects
        .get_mut(&4)
        .unwrap()
        .idle
        .as_mut()
        .unwrap()
        .flags = 0xface;
    world
        .objects
        .get_mut(&4)
        .unwrap()
        .idle
        .as_mut()
        .unwrap()
        .countdown = 42;
    world.unregister_scheduler_object(4).unwrap();
    world.objects.remove(&4);
    world.validate_scheduler_storage().unwrap();
    let saved = serde_json::to_string(&world).unwrap();
    world = serde_json::from_str(&saved).unwrap();
    let storage = world.scheduler_storage.as_ref().unwrap();
    assert_eq!(storage.len(), 4);
    assert!(!storage.contains(4));
    assert_eq!(storage.word(&world, 4).unwrap(), 0xface002a);
    insert(&mut world, 4);
    assert_eq!(
        world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .word(&world, 4)
            .unwrap(),
        0xffff
    );
    world.validate_scheduler_storage().unwrap();
}

#[test]
fn registry_rejects_mismatched_occupancy_without_inventing_legacy_history() {
    let mut old = World::default();
    insert(&mut old, 8);
    assert!(old.scheduler_storage.is_none());
    let text = serde_json::to_string(&old).unwrap();
    assert!(!text.contains("scheduler_storage"));
    let restored: World = serde_json::from_str(&text).unwrap();
    assert!(restored.scheduler_storage.is_none());
    let mut world = World {
        scheduler_storage: Some(Storage::default()),
        ..Default::default()
    };
    insert(&mut world, 2);
    let before = world.clone();
    assert!(
        world
            .register_scheduler_object(2, &mut Object::default())
            .is_err()
    );
    assert_eq!(world, before);
    world.objects.remove(&2);
    assert!(world.validate_scheduler_storage().is_err());
}

fn fixture() -> (Program, Vm, [u64; 2]) {
    let routine = Routine {
        name: "active controller Sleep".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 0,
            instructions: vec![Instruction {
                opcode: 0,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            }],
        },
    };
    let program = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(4096, routine)]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut world = World {
        scheduler_storage: Some(Storage::default()),
        ..Default::default()
    };
    for id in [1, 2] {
        insert(&mut world, id);
        world
            .objects
            .get_mut(&id)
            .unwrap()
            .idle
            .as_mut()
            .unwrap()
            .flags = 3;
    }
    let mut vm = Vm::new(world, 1);
    vm.tick = 500;
    let mut threads = [0; 2];
    for id in [1, 2] {
        let thread = vm
            .spawn(
                &program,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: id,
                    callee: id,
                    stack_object: 2,
                    args: vec![3, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        vm.register_object_thread(id, thread).unwrap();
        threads[(id - 1) as usize] = thread;
    }
    (program, vm, threads)
}
fn restore(vm: &mut Vm, program: &Program) {
    let saved = serde_json::to_string(&Snapshot::capture(program, vm)).unwrap();
    let next = serde_json::from_str::<Snapshot>(&saved)
        .unwrap()
        .restore(program)
        .unwrap();
    assert_eq!(*vm, next);
    *vm = next;
}

#[test]
fn run_while_paused_controller_can_use_passive_sleep_and_resume() {
    let (p, mut vm, [thread, _]) = fixture();
    vm.world.objects.get_mut(&1).unwrap().data.insert(25, 0);
    vm.begin_scheduled_object_visit(1).unwrap();
    assert_eq!(vm.run_slice(&p, thread, 1).unwrap().stop, Stop::Sleeping);
    assert_eq!(vm.threads[&thread].stack[0].args[0], 3);
    assert_eq!(vm.world.objects[&1].idle.as_ref().unwrap().countdown, 2);
    assert_eq!(vm.world.objects[&1].idle.as_ref().unwrap().flags, 3);
    assert_eq!(
        vm.threads[&thread].status,
        Status::Sleeping { wake_tick: 503 }
    );
    restore(&mut vm, &p);
    vm.finish_object_update(1, true).unwrap();
    vm.end_scheduled_object_visit().unwrap();
    vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .idle
        .as_mut()
        .unwrap()
        .countdown = 0;
    vm.begin_scheduled_object_visit(1).unwrap();
    vm.run_slice(&p, thread, 1).unwrap();
    assert_eq!(vm.world.objects[&1].idle.as_ref().unwrap().countdown, -1);
    assert_eq!(vm.threads[&thread].status, Status::Returned(true));
    restore(&mut vm, &p);
}

#[test]
fn actual_sleep_reenters_each_visit_without_advancing_game_time_or_other_threads() {
    let (p, mut vm, [first, second]) = fixture();
    let other = vm.threads[&second].clone();
    let other_object = vm.world.objects[&2].clone();
    for visit in 0..3 {
        vm.begin_scheduled_object_visit(1).unwrap();
        let before = vm.clone();
        assert!(vm.begin_scheduled_object_visit(1).is_err());
        assert_eq!(vm, before);
        assert!(vm.advance_tick().is_err());
        assert!(vm.run_slice(&p, second, 1).is_err());
        assert!(vm.end_scheduled_object_visit().is_err());
        assert!(vm.finish_object_update(1, true).is_err());
        assert_eq!(vm.run_slice(&p, first, 0).unwrap().stop, Stop::Budget);
        restore(&mut vm, &p);
        assert_eq!(vm.run_slice(&p, first, 1).unwrap().stop, Stop::Sleeping);
        assert_eq!(vm.threads[&first].stack[0].args[0], 2 - visit);
        assert_eq!(
            vm.threads[&first].status,
            Status::Sleeping { wake_tick: 501 }
        );
        restore(&mut vm, &p);
        assert!(vm.run_slice(&p, first, 100).unwrap().trace.is_empty());
        vm.finish_object_update(1, true).unwrap();
        let after = vm.clone();
        vm.finish_object_update(1, true).unwrap();
        assert_eq!(vm, after);
        assert_eq!(vm.world.objects[&1].data[&25], 8 - visit);
        assert!(vm.run_slice(&p, first, 1).is_err());
        restore(&mut vm, &p);
        vm.end_scheduled_object_visit().unwrap();
        assert_eq!(vm.tick, 500);
        assert_eq!(vm.threads[&second], other);
        assert_eq!(vm.world.objects[&2], other_object);
    }
}

#[test]
fn headline_countdown_runs_once_per_visit_at_a_frozen_clock() {
    let (p, mut vm, [first, _]) = fixture();
    vm.world.objects.get_mut(&1).unwrap().headline = Some(headline::State {
        duration: 10,
        frames: 1,
        step: 1,
        ..Default::default()
    });
    for remaining in [9, 8, 7] {
        vm.begin_scheduled_object_visit(1).unwrap();
        assert_eq!(vm.run_slice(&p, first, 1).unwrap().stop, Stop::Sleeping);
        vm.finish_object_update(1, true).unwrap();
        vm.finish_object_update(1, true).unwrap();
        assert_eq!(
            vm.world.objects[&1].headline.as_ref().unwrap().duration,
            remaining
        );
        vm.end_scheduled_object_visit().unwrap();
    }
    assert_eq!(vm.tick, 500);
}

#[test]
fn restored_visit_cannot_claim_another_thread_or_a_later_clock() {
    let (p, mut vm, _) = fixture();
    vm.begin_scheduled_object_visit(1).unwrap();
    let saved = Snapshot::capture(&p, &vm);
    let mut value = serde_json::to_value(&saved).unwrap();
    value["state"]["scheduled_object_visit"]["object"] = 2.into();
    assert!(
        serde_json::from_value::<Snapshot>(value)
            .unwrap()
            .restore(&p)
            .is_err()
    );
    let mut later = saved;
    later.state.tick += 1;
    assert!(later.restore(&p).is_err());
}

#[test]
fn synchronous_callback_can_suspend_inside_the_same_object_visit() {
    let (p, mut vm, [first, _]) = fixture();
    vm.begin_scheduled_object_visit(1).unwrap();
    vm.begin_immediate_callback(
        &p,
        first,
        Start {
            owner: "test",
            bhav: 4096,
            caller: 1,
            callee: 1,
            stack_object: 1,
            args: vec![0, 0, 0, 0],
            check: true,
        },
    )
    .unwrap();
    assert!(vm.threads[&first].check);
    restore(&mut vm, &p);
    vm.run_slice(&p, first, 1).unwrap();
    assert!(vm.threads[&first].callback.is_none());
    assert_eq!(vm.threads[&first].stack[0].args[0], 3);
    restore(&mut vm, &p);
    assert_eq!(vm.run_slice(&p, first, 1).unwrap().stop, Stop::Sleeping);
    vm.finish_object_update(1, true).unwrap();
    vm.end_scheduled_object_visit().unwrap();
    assert_eq!(vm.world.objects[&1].data[&25], 8);
}

#[test]
fn completed_visit_allows_callbacks_but_never_ordinary_reentry() {
    let (program, mut vm, [thread, other]) = fixture();
    vm.begin_scheduled_object_visit(1).unwrap();
    assert_eq!(
        vm.run_slice(&program, thread, 1).unwrap().stop,
        Stop::Sleeping
    );
    vm.finish_object_update(1, true).unwrap();
    let suspended = vm.threads[&thread].clone();
    let visit = vm.scheduled_object_visit.clone();
    for _ in 0..2 {
        vm.begin_immediate_callback(
            &program,
            thread,
            Start {
                owner: "test",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![0; 4],
                check: true,
            },
        )
        .unwrap();
        restore(&mut vm, &program);
        vm.run_slice(&program, thread, 1).unwrap();
        assert!(vm.threads[&thread].callback.is_none());
        restore(&mut vm, &program);
    }
    assert_eq!(vm.threads[&thread].stack, suspended.stack);
    assert_eq!(vm.threads[&thread].status, suspended.status);
    assert!(!vm.threads[&thread].check);
    assert_eq!(vm.scheduled_object_visit, visit);
    assert_eq!(vm.world.objects[&1].data[&25], 8);
    assert_eq!(vm.tick, 500);
    let before = vm.clone();
    assert_eq!(
        vm.run_slice(&program, thread, 1).unwrap_err(),
        "native object update has already returned"
    );
    assert_eq!(vm, before);
    assert!(vm.run_slice(&program, other, 1).is_err());
    assert_eq!(vm, before);
    vm.end_scheduled_object_visit().unwrap();
    assert!(vm.scheduled_object_visit.is_none());
}

#[test]
fn nested_engine_update_preserves_the_parent_visit_across_save_resume() {
    let (p, mut vm, [first, second]) = fixture();
    vm.begin_scheduled_object_visit(1).unwrap();
    assert_eq!(vm.run_slice(&p, first, 1).unwrap().stop, Stop::Sleeping);
    let parent = vm.scheduled_object_visit.clone();
    vm.begin_nested_object_visit(2).unwrap();
    assert_eq!(
        vm.scheduled_object_visit.as_ref().unwrap().chain().count(),
        2
    );
    assert!(vm.begin_nested_object_visit(1).is_err());
    assert!(vm.run_slice(&p, first, 1).is_err());
    assert!(vm.finish_object_update(1, true).is_err());
    restore(&mut vm, &p);
    assert_eq!(vm.run_slice(&p, second, 1).unwrap().stop, Stop::Sleeping);
    vm.finish_object_update(2, true).unwrap();
    restore(&mut vm, &p);
    vm.end_scheduled_object_visit().unwrap();
    assert_eq!(vm.scheduled_object_visit, parent);
    vm.finish_object_update(1, true).unwrap();
    vm.end_scheduled_object_visit().unwrap();
    assert_eq!(vm.tick, 500);
    assert_eq!(
        (
            vm.world.objects[&1].data[&25],
            vm.world.objects[&2].data[&25]
        ),
        (8, 8)
    );
}

#[test]
fn native_pass_drives_real_vm_sleep_while_an_ordinary_object_stays_frozen() {
    use sims_vm::scheduler::{Inputs, Pass, Reply, Request, registry::Borrowed};
    let (p, mut vm, [_, second]) = fixture();
    vm.world
        .objects
        .get_mut(&2)
        .unwrap()
        .idle
        .as_mut()
        .unwrap()
        .flags = 1;
    let other = vm.threads[&second].clone();
    let other_object = vm.world.objects[&2].clone();
    let input = Inputs {
        paused: 0,
        mode: 2,
        lot_number: 7,
        tick: vm.tick as u32,
    };
    let mut cursor = 0;
    for expected in [2, 1, 0] {
        let mut pass = Pass::begin(input, &Borrowed::new(&mut vm).unwrap(), &mut cursor).unwrap();
        let mut visits = 0;
        while let Some(request) = pass
            .advance(&mut Borrowed::new(&mut vm).unwrap(), &mut cursor, input)
            .unwrap()
        {
            let Request::Tick { object, tick } = request else {
                panic!("fixture error")
            };
            assert_eq!((object, tick), (1, 500));
            let object_id = Borrowed::new(&mut vm).unwrap().object_id(object).unwrap();
            vm.begin_scheduled_object_visit(object_id).unwrap();
            let thread = vm.scheduled_object_visit.as_ref().unwrap().thread();
            assert_eq!(vm.run_slice(&p, thread, 1).unwrap().stop, Stop::Sleeping);
            restore(&mut vm, &p);
            vm.finish_object_update(object_id, true).unwrap();
            vm.end_scheduled_object_visit().unwrap();
            pass.acknowledge(&request, Reply::Tick(1)).unwrap();
            visits += 1;
        }
        assert_eq!(visits, 1);
        assert_eq!(pass.result(), Some(false));
        assert_eq!(cursor, 0);
        assert_eq!(vm.tick, 500);
        assert_eq!(
            vm.threads[&vm.object_threads[&1]].stack[0].args[0],
            expected
        );
        assert_eq!(vm.threads[&second], other);
        assert_eq!(vm.world.objects[&2], other_object);
    }
}

#[test]
fn live_registry_errors_are_owned_and_reused_ids_cannot_replace_retained_handles() {
    use sims_vm::scheduler::{Registry, registry::Borrowed};
    let (_, mut vm, [first, _]) = fixture();
    let original_handle = Borrowed::new(&mut vm).unwrap().object(1).unwrap().unwrap();
    assert_eq!(
        Borrowed::new(&mut vm)
            .unwrap()
            .error(original_handle)
            .unwrap(),
        0
    );
    vm.world.objects.get_mut(&1).unwrap().native_tree_error = Some(0x44f);
    assert_eq!(
        Borrowed::new(&mut vm)
            .unwrap()
            .error(original_handle)
            .unwrap(),
        0x44f
    );
    vm.world.objects.get_mut(&1).unwrap().native_tree_error = None;
    assert!(
        Borrowed::new(&mut vm)
            .unwrap()
            .error(original_handle)
            .unwrap_err()
            .contains("unknown")
    );
    vm.world.objects.get_mut(&1).unwrap().native_tree_error = Some(0);
    vm.threads.get_mut(&first).unwrap().status = Status::Faulted(Fault {
        message: "unsupported host service".into(),
        stack: vec![],
    });
    assert!(
        Borrowed::new(&mut vm)
            .unwrap()
            .error(original_handle)
            .unwrap_err()
            .contains("no native TreeSim error mapping")
    );
    vm.object_threads.remove(&1);
    vm.world.unregister_scheduler_object(1).unwrap();
    vm.world.objects.remove(&1);
    insert(&mut vm.world, 1);
    let borrow = Borrowed::new(&mut vm).unwrap();
    let replacement = borrow.object(1).unwrap().unwrap();
    assert_ne!(replacement, original_handle);
    assert_eq!(borrow.object_id(replacement).unwrap(), 1);
    assert!(
        borrow
            .error(original_handle)
            .unwrap_err()
            .contains("lifetime has ended")
    );
    assert_eq!(borrow.error(replacement).unwrap(), 0);
}

#[test]
fn live_registry_preserves_visit_marker_and_unknown_error_blocks_flag_clear() {
    use sims_vm::scheduler::{Inputs, Pass, Registry, registry::Borrowed};
    let (_, mut vm, _) = fixture();
    vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .idle
        .as_mut()
        .unwrap()
        .last_run = Some(123);
    {
        let mut borrow = Borrowed::new(&mut vm).unwrap();
        for invalid in [0, -1, 3, 65537, i32::MAX] {
            assert!(borrow.object(invalid).is_err());
            assert!(borrow.word(invalid).is_err());
            assert!(borrow.set_word(invalid, 0).is_err());
        }
        borrow.set_word(1, 0x40007).unwrap();
    }
    assert_eq!(
        vm.world.objects[&1].idle.as_ref().unwrap().last_run,
        Some(123)
    );
    vm.world.objects.get_mut(&1).unwrap().native_tree_error = None;
    let input = Inputs {
        paused: 1,
        mode: 0,
        lot_number: 7,
        tick: 500,
    };
    let mut cursor = 0;
    let mut pass = Pass::begin(input, &Borrowed::new(&mut vm).unwrap(), &mut cursor).unwrap();
    assert!(
        pass.advance(&mut Borrowed::new(&mut vm).unwrap(), &mut cursor, input)
            .unwrap_err()
            .contains("unknown")
    );
    assert_eq!(Borrowed::new(&mut vm).unwrap().word(1).unwrap(), 0x40007);
    vm.world.objects.get_mut(&1).unwrap().native_tree_error = Some(0);
    pass.advance(&mut Borrowed::new(&mut vm).unwrap(), &mut cursor, input)
        .unwrap();
    assert_eq!(Borrowed::new(&mut vm).unwrap().word(1).unwrap(), 7);
}

#[test]
fn replacement_fault_cannot_complete_a_native_object_update() {
    let (_, mut vm, [thread, _]) = fixture();
    vm.begin_scheduled_object_visit(1).unwrap();
    vm.threads.get_mut(&thread).unwrap().status = Status::Faulted(Fault {
        message: "unsupported host service".into(),
        stack: vec![],
    });
    let before = vm.clone();
    assert!(vm.finish_object_update(1, true).is_err());
    assert!(vm.end_scheduled_object_visit().is_err());
    assert_eq!(vm, before);
}

#[test]
fn nested_yielding_services_survive_native_updates_and_cold_restore() {
    for budget in [1, 1000] {
        let (p, mut vm, [thread, other]) = fixture();
        let untouched = vm.threads[&other].clone();
        vm.begin_scheduled_object_visit(1).unwrap();
        for _ in 0..2 {
            vm.push_service_call(
                &p,
                thread,
                Start {
                    owner: "test",
                    bhav: 4096,
                    caller: 1,
                    callee: 2,
                    stack_object: 2,
                    args: vec![2, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        }
        let outer = vm.threads[&thread].stack[..2].to_vec();
        assert!(vm.finish_object_update(1, true).is_err());
        for remaining in [1, 0] {
            assert_eq!(
                vm.run_slice(&p, thread, budget).unwrap().stop,
                Stop::Sleeping
            );
            assert_eq!(vm.threads[&thread].stack[2].args[0], remaining);
            assert_eq!(vm.threads[&thread].stack[..2], outer);
            assert_eq!(
                vm.threads[&thread]
                    .service
                    .as_ref()
                    .unwrap()
                    .chain()
                    .count(),
                2
            );
            restore(&mut vm, &p);
            vm.finish_object_update(1, true).unwrap();
            vm.end_scheduled_object_visit().unwrap();
            restore(&mut vm, &p);
            vm.begin_scheduled_object_visit(1).unwrap();
        }
        // A returned child must still be consumed by its own host before the
        // object tail may finish. Merely retaining a service is not a yield.
        vm.run_slice(&p, thread, budget).unwrap();
        assert_eq!(
            vm.threads[&thread].service.as_ref().unwrap().result,
            Some(true)
        );
        let before = vm.clone();
        assert!(vm.finish_object_update(1, true).is_err());
        assert!(vm.end_scheduled_object_visit().is_err());
        assert_eq!(vm, before);
        restore(&mut vm, &p);
        vm.pop_service(thread).unwrap();
        assert_eq!(
            vm.run_slice(&p, thread, budget).unwrap().stop,
            Stop::Sleeping
        );
        vm.finish_object_update(1, true).unwrap();
        vm.end_scheduled_object_visit().unwrap();
        restore(&mut vm, &p);
        assert_eq!(vm.tick, 500);
        assert_eq!(vm.threads[&other], untouched);
        assert_eq!(vm.world.objects[&1].data[&25], 6);
    }
}
