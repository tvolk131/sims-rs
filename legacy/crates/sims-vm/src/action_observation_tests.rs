use super::*;
use crate::{
    CallbackBoundary, Module, Object, Routine, Start, Status, Stop,
    action::{self, Actor, Queue, WorldState},
    program::MenuEntry,
};
use sims_formats::behavior::{Behavior, Instruction};

fn predicate(value: bool, branch: u8) -> Instruction {
    Instruction {
        opcode: 2,
        on_true: branch,
        on_false: branch,
        operands: [0, 0, u8::from(!value), 0, 0, 2, 7, 7],
    }
}

fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "authored action observation fixture".into(),
        body: Behavior { format: 0x8002, kind: 0, args: 4, locals: 0, instructions },
    }
}

fn record(serial: u32, priority: i32) -> Action {
    Action {
        id: serial, target: 2, icon_owner: 2, menu_id: 19, priority,
        routine: 4097, ..Default::default()
    }
}

fn fixture() -> (Program, Vm, ThreadId) {
    let module = Module {
        source: "authored".into(),
        routines: [
            (4096, routine(vec![Instruction {
                opcode: 17, on_true: 254, on_false: 255,
                operands: [0, 0, 1, 0, 0, 0, 0, 0],
            }])),
            (4097, routine(vec![predicate(true, 254)])),
            (4098, routine(vec![predicate(true, 254)])),
        ].into(),
        object_interactions: [
            (1, vec![]),
            (2, vec![MenuEntry {
                id: 19, flags: 1, action: 4097, check: 0, ..Default::default()
            }]),
        ].into(),
        ..Default::default()
    };
    let program = Program {
        globals: "authored".into(), modules: [("authored".into(), module)].into(),
        ..Default::default()
    };
    let mut world = World {
        scheduler_storage: Some(crate::scheduler::storage::Storage::default()),
        actions: Some(WorldState { actors: [(1, Actor::default())].into(), ..Default::default() }),
        ..Default::default()
    };
    for id in 1..=2 {
        let mut person: BTreeMap<i16, i16> = (0..256).map(|index| (index, 0)).collect();
        person.insert(58, 30);
        let mut object = Object {
            definition: Some(Definition { source: "authored".into(), object: id as u16, guid: id as u32, slots: 0 }),
            person: (id == 1).then_some(person),
            data: (0..80).map(|index| (index, 0)).collect(),
            motives: (id == 1).then_some([0; 16]),
            ..Default::default()
        };
        world.register_scheduler_object(id, &mut object).unwrap();
        world.objects.insert(id, object);
    }
    let mut vm = Vm::new(world, 123);
    let id = vm.spawn(&program, Start {
        owner: "authored", bhav: 4096, caller: 1, callee: 1,
        stack_object: 2, args: vec![0; 4], check: false,
    }).unwrap();
    vm.register_object_thread(1, id).unwrap();
    (program, vm, id)
}

fn transitions(facts: &[Fact]) -> Vec<(u32, QueueTransition)> {
    facts.iter().filter_map(|fact| match fact {
        Fact::Queue { action, transition } => Some((action.serial, *transition)),
        _ => None,
    }).collect()
}

#[test]
fn one_enqueue_reports_assignment_replacements_and_acceptance_in_order() {
    let (program, mut vm, _) = fixture();
    vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().queue.pending =
        [record(10, 1), record(11, 2)].into();
    let mut facts = Vec::new();
    assert_eq!(vm.enqueue_action_observed(&program, 1, record(0, 100), &mut |fact| facts.push(fact)).unwrap(), (1, true));
    assert_eq!(transitions(&facts), vec![
        (1, QueueTransition::SerialAssigned),
        (10, QueueTransition::ReplacedBeforeStart),
        (11, QueueTransition::ReplacedBeforeStart),
        (1, QueueTransition::InsertionAccepted),
    ]);
    assert_eq!(vm.world.actions.as_ref().unwrap().callbacks.len(), 2);
    for fact in facts {
        if let Fact::Queue { action, .. } = fact {
            assert_eq!(action.actor.handle, Evidence::Known(1));
            assert_eq!(action.target.handle, Evidence::Known(2));
        }
    }
}

#[test]
fn queue_rejection_overflow_discard_and_preinsert_error_are_distinct() {
    let mut queue = Queue { pending: (0..8).map(|serial| record(serial, 100)).collect() };
    let mut facts = Vec::new();
    assert!(!queue.insert_observed(record(9, 1), &mut |fact| facts.push(fact)).unwrap().accepted);
    assert_eq!(transitions(&facts), vec![(9, QueueTransition::InsertionRejected)]);
    facts.clear();
    let mut incoming = record(10, 1);
    incoming.flags = action::PUSH_FRONT;
    queue.insert_observed(incoming, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(transitions(&facts), vec![
        (6, QueueTransition::DiscardedBeforeStart),
        (7, QueueTransition::DiscardedBeforeStart),
        (10, QueueTransition::InsertionAccepted),
    ]);
    for fact in &facts {
        if let Fact::Queue { action, .. } = fact {
            assert_eq!(action.actor.handle, Evidence::Unavailable(Unavailable::ActorNotSupplied));
            assert_eq!(action.target.handle, Evidence::Unavailable(Unavailable::NoRegistryContext));
        }
    }
    queue.pending = (0..9).map(|serial| record(serial, 1)).collect();
    let before = queue.clone();
    facts.clear();
    assert!(queue.insert_observed(record(99, 1), &mut |fact| facts.push(fact)).is_err());
    assert_eq!(queue, before);
    assert_eq!(transitions(&facts), vec![(99, QueueTransition::PreinsertError)]);
}

#[test]
fn assigned_serial_survives_a_later_label_resolution_error() {
    let (program, mut vm, _) = fixture();
    let mut active = record(77, 10);
    active.target = 99;
    vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().active = Some(active);
    let mut plain = vm.clone();
    let mut incoming = record(0, 1);
    incoming.flags = 64 | action::PUSH_FRONT;
    let mut facts = Vec::new();
    let observed = vm.enqueue_action_observed(&program, 1, incoming.clone(), &mut |fact| facts.push(fact));
    assert_eq!(observed, plain.enqueue_action(&program, 1, incoming));
    assert!(observed.is_err());
    assert_eq!(vm, plain);
    assert_eq!(transitions(&facts), vec![(1, QueueTransition::SerialAssigned)]);
    assert!(matches!(facts.last(), Some(Fact::EnqueueFailed { .. })));
    assert_eq!(vm.world.actions.as_ref().unwrap().serial, 1);
}

#[test]
fn queued_cancellation_is_not_an_active_cancel_request_or_return() {
    let (program, mut vm, _) = fixture();
    let state = vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap();
    state.active = Some(record(1, 100));
    state.queue.pending.push_back(record(2, 1));
    let mut facts = Vec::new();
    assert!(vm.cancel_action_observed(&program, 1, 2, &mut |fact| facts.push(fact)).unwrap());
    assert_eq!(transitions(&facts), vec![(2, QueueTransition::CancelledBeforeStart)]);
    assert!(vm.cancel_action_observed(&program, 1, 1, &mut |fact| facts.push(fact)).unwrap());
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ActiveCancelRequested { .. })).count(), 1);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
    assert_ne!(vm.world.actions.as_ref().unwrap().actors[&1].active.as_ref().unwrap().flags & action::CANCELLED, 0);
}

#[test]
fn native_fe_ff_return_value_is_not_the_old_completed_tuple_result() {
    for (target, value, expected) in [(254, true, NativeReturn::TrueFe), (255, false, NativeReturn::FalseFf)] {
        let (program, mut vm, id) = fixture();
        let mut thread = vm.threads.remove(&id).unwrap();
        thread.stack[0].action_target = Some(2);
        thread.last_result = !value;
        let original = thread.stack[0].clone();
        let mut facts = Vec::new();
        crate::branch_observed(&program, &mut thread, target, Some(&vm.world),
            &mut Observation::new(&mut |fact| facts.push(fact), Some(id))).unwrap();
        assert_eq!(thread.status, Status::Returned(value));
        assert_eq!(thread.completed_actions[0].2, !value);
        assert_eq!(facts.len(), 1);
        let Fact::Returned { frame, branch, outermost_action_use, binding } = &facts[0] else { panic!("missing native return") };
        assert_eq!(*branch, expected);
        assert!(*outermost_action_use);
        assert_eq!(frame.frame, original);
        assert_eq!(frame.depth, 1);
        assert_eq!(frame.thread, Some(id));
        assert_eq!(*binding, Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding));
        assert!(matches!(frame.routine_sha256, Evidence::Known(_)));
        assert_eq!(frame.source_content_hash, None);
    }
}

#[test]
fn interpreter_return_disagrees_with_native_history_without_rewriting_history() {
    for (value, branch, expected) in [(true, 254, NativeReturn::TrueFe), (false, 255, NativeReturn::FalseFf)] {
        let (mut program, mut vm, id) = fixture();
        program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![predicate(!value, branch)]));
        vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
        let mut plain = vm.clone();
        let mut facts = Vec::new();
        let run = vm.run_local_slice_observed(&program, id, 2, &mut |fact| facts.push(fact)).unwrap();
        assert_eq!(run, plain.run_slice(&program, id, 2).unwrap());
        assert_eq!(vm, plain);
        assert!(facts.iter().any(|fact| matches!(fact, Fact::Returned { branch, .. } if *branch == expected)));
        assert_eq!(vm.threads[&id].last_result, value);
        assert_eq!(vm.world.actions.as_ref().unwrap().actors[&1].previous.as_ref().unwrap().flags & 32, u32::from(!value) * 32);
    }
}

#[test]
fn reset_unwind_preserves_native_last_result_and_is_not_a_return() {
    let (program, mut vm, id) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    vm.threads.get_mut(&id).unwrap().last_result = true;
    let mut plain = vm.clone();
    let mut reset = crate::person_reset::Reset::new(1, 0);
    let mut plain_reset = reset.clone();
    let mut facts = Vec::new();
    let request = reset.next_observed(&mut vm, &program, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(request, plain_reset.next(&mut plain, &program).unwrap());
    assert_eq!(request, Some(crate::person_reset::Request::NotifyUsed(2)));
    assert_eq!(vm, plain);
    assert_eq!(reset, plain_reset);
    assert!(vm.threads[&id].last_result);
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ResetUnwound { .. })).count(), 1);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
}

#[test]
fn nested_dispatch_displacement_and_two_returns_do_not_inherit_the_active_serial() {
    let (mut program, mut vm, id) = fixture();
    let module = program.modules.get_mut("authored").unwrap();
    module.routines.insert(4097, routine(vec![Instruction {
        opcode: 30, on_true: 254, on_false: 255, operands: [0; 8],
    }]));
    module.object_interactions.get_mut(&2).unwrap()[0].action = 4098;
    vm.world.objects.get_mut(&1).unwrap().data.insert(20, 19);
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let mut facts = Vec::new();
    for _ in 0..3 {
        vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
    }
    assert!(facts.iter().any(|fact| matches!(fact,
        Fact::ActiveDisplaced { previous, installed } if previous.serial == 1 && installed.serial == 2)));
    assert!(facts.iter().any(|fact| matches!(fact,
        Fact::FrameEntered { serial_assignment_pending: true, candidate, .. } if candidate.serial == 0)));
    let exits: Vec<_> = facts.iter().filter_map(|fact| match fact {
        Fact::Returned { frame, outermost_action_use, binding, .. } => Some((frame.depth, *outermost_action_use, binding.clone())),
        _ => None,
    }).collect();
    assert_eq!(exits, vec![
        (3, false, Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)),
        (2, true, Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)),
    ]);
}

#[test]
fn recycled_script_id_and_wrapped_serial_never_rewrite_earlier_identity() {
    let (program, mut vm, _) = fixture();
    vm.world.actions.as_mut().unwrap().serial = u32::MAX;
    let mut facts = Vec::new();
    vm.enqueue_action_observed(&program, 1, record(0, 1), &mut |fact| facts.push(fact)).unwrap();
    vm.world.unregister_scheduler_object(2).unwrap();
    let mut replacement = vm.world.objects.remove(&2).unwrap();
    vm.world.register_scheduler_object(2, &mut replacement).unwrap();
    vm.world.objects.insert(2, replacement);
    vm.world.actions.as_mut().unwrap().serial = u32::MAX;
    vm.enqueue_action_observed(&program, 1, record(0, 1), &mut |fact| facts.push(fact)).unwrap();
    let accepted: Vec<_> = facts.iter().filter_map(|fact| match fact {
        Fact::Queue { action, transition: QueueTransition::InsertionAccepted } => Some((action.serial, action.target.handle.clone())),
        _ => None,
    }).collect();
    assert_eq!(accepted, vec![(0, Evidence::Known(2)), (0, Evidence::Known(3))]);
    assert_eq!(vm.world.actions.as_ref().unwrap().actors[&1].queue.pending.len(), 2);
    for fact in &facts {
        if let Fact::Queue { action, .. } = fact {
            assert_eq!(action.retained_lifetime_binding, Unavailable::NativeRecordHasNoLifetimeBinding);
        }
    }
}

#[test]
fn unknown_registry_storage_stays_unavailable_not_a_script_id_handle() {
    let (program, mut vm, _) = fixture();
    vm.world.scheduler_storage = None;
    let mut facts = Vec::new();
    vm.enqueue_action_observed(&program, 1, record(0, 1), &mut |fact| facts.push(fact)).unwrap();
    let Fact::Queue { action, .. } = &facts[0] else { panic!("missing serial assignment") };
    assert_eq!(action.actor.handle, Evidence::Unavailable(Unavailable::RegistryStorageMissing));
    assert_eq!(action.target.handle, Evidence::Unavailable(Unavailable::RegistryStorageMissing));
}

#[test]
fn observer_on_off_preserves_full_vm_bytes_trace_rng_ticks_stops_and_budgets() {
    let (mut program, mut observed, id) = fixture();
    program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![
        Instruction { opcode: 8, on_true: 1, on_false: 255, operands: [0, 0, 8, 0, 10, 0, 7, 0] },
        predicate(true, 254),
    ]));
    observed.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let mut plain = observed.clone();
    let seed = plain.rng_state;
    let mut facts = Vec::new();
    for budget in [0, 1, 1, 2, 8] {
        let expected = plain.run_slice(&program, id, budget).unwrap();
        let actual = observed.run_local_slice_observed(&program, id, budget, &mut |fact| facts.push(fact)).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(observed, plain);
        assert_eq!(serde_json::to_vec(&observed).unwrap(), serde_json::to_vec(&plain).unwrap());
        assert_eq!(observed.advance_tick(), plain.advance_tick());
        assert_eq!(observed, plain);
    }
    assert_ne!(observed.rng_state, seed);
    assert_eq!(observed.tick, plain.tick);
    assert!(facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
}

#[test]
fn successful_pop_fact_survives_a_later_error_in_the_same_branch_call() {
    let (mut program, mut vm, id) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    program.modules.get_mut("authored").unwrap().routines.get_mut(&4096).unwrap().body.instructions[0].on_true = 253;
    let mut thread = vm.threads.remove(&id).unwrap();
    thread.stack[0].continuation = 0;
    let mut plain = thread.clone();
    let mut facts = Vec::new();
    let result = crate::branch_observed(&program, &mut thread, 254, Some(&vm.world),
        &mut Observation::new(&mut |fact| facts.push(fact), Some(id)));
    assert_eq!(result, crate::branch(&program, &mut plain, 254));
    assert!(result.is_err());
    assert_eq!(thread, plain);
    assert_eq!(facts.len(), 1);
    assert!(matches!(facts[0], Fact::Returned { .. }));
}

#[test]
fn exit_is_not_reemitted_by_failed_completion_bookkeeping() {
    let (program, mut vm, id) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    vm.world.objects.get_mut(&2).unwrap().data.remove(&62);
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let run = vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(run, plain.run_slice(&program, id, 1).unwrap());
    assert_eq!(run.stop, Stop::Faulted);
    assert_eq!(vm, plain);
    vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::Returned { .. })).count(), 1);
}

#[test]
fn dequeue_error_and_prestart_rejection_are_not_frame_entry() {
    for missing_entry in [true, false] {
        let (program, mut vm, id) = fixture();
        let mut action = record(0, 100);
        if missing_entry { action.menu_id = 99; } else { action.target = 0; }
        vm.enqueue_action(&program, 1, action).unwrap();
        let mut plain = vm.clone();
        let mut facts = Vec::new();
        let run = vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
        assert_eq!(run, plain.run_slice(&program, id, 1).unwrap());
        assert_eq!(vm, plain);
        assert!(transitions(&facts).contains(&(1, QueueTransition::Dequeued)));
        assert!(!facts.iter().any(|fact| matches!(fact, Fact::FrameEntered { .. } | Fact::Returned { .. })));
        if missing_entry {
            assert!(facts.iter().any(|fact| matches!(fact, Fact::DispatchFailed { frame_entered: Evidence::Known(false), .. })));
        } else {
            assert!(transitions(&facts).contains(&(1, QueueTransition::PrestartRejected)));
        }
    }
}

#[test]
fn reference_removal_is_a_queue_cause_not_a_return_or_actor_erasure() {
    let (program, mut vm, _) = fixture();
    vm.enqueue_action(&program, 1, record(0, 1)).unwrap();
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let callbacks = vm.remove_action_object_references_observed(1, 2, &mut |fact| facts.push(fact)).unwrap();
    let expected = plain.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().remove_object_references(2).unwrap();
    assert_eq!(callbacks, expected);
    assert_eq!(vm, plain);
    assert_eq!(transitions(&facts), vec![(1, QueueTransition::ReferenceRemoved { target: 2 })]);
    assert!(vm.world.objects.contains_key(&2));
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
}

#[test]
fn ordinary_and_callback_boundary_frames_are_not_action_terminals() {
    for callback in [false, true] {
        let (program, mut vm, id) = fixture();
        let mut thread = vm.threads.remove(&id).unwrap();
        if callback {
            thread.callback = Some(CallbackBoundary {
                external: true, parent: None, base_depth: 0,
                resume_status: Status::Ready, resume_check: false,
            });
        }
        let mut facts = Vec::new();
        crate::branch_observed(&program, &mut thread, 254, Some(&vm.world),
            &mut Observation::new(&mut |fact| facts.push(fact), Some(id))).unwrap();
        assert!(facts.is_empty());
        if callback { assert_eq!(thread.callback_result, Some(true)); }
    }
}

#[test]
fn active_cancel_request_fact_survives_a_later_missing_person_error() {
    let (program, mut vm, _) = fixture();
    vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().active = Some(record(7, 1));
    vm.world.objects.get_mut(&1).unwrap().person = None;
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let result = vm.cancel_action_observed(&program, 1, 7, &mut |fact| facts.push(fact));
    assert_eq!(result, plain.cancel_action(&program, 1, 7));
    assert!(result.is_err());
    assert_eq!(vm, plain);
    assert_eq!(facts.len(), 1);
    assert!(matches!(facts[0], Fact::ActiveCancelRequested { .. }));
}

#[test]
fn frame_entry_is_emitted_before_later_action_storage_binding_failure() {
    let (mut program, mut vm, id) = fixture();
    program.modules.get_mut("authored").unwrap().routines.insert(4096, routine(vec![Instruction {
        opcode: 30, on_true: 254, on_false: 255, operands: [0; 8],
    }]));
    vm.world.objects.get_mut(&1).unwrap().data.insert(20, 19);
    vm.world.actions = None;
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let run = vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(run, plain.run_slice(&program, id, 1).unwrap());
    assert_eq!(run.stop, Stop::Faulted);
    assert_eq!(vm, plain);
    assert!(facts.iter().any(|fact| matches!(fact, Fact::FrameEntered { .. })));
    assert!(facts.iter().any(|fact| matches!(fact, Fact::DispatchFailed { frame_entered: Evidence::Known(true), .. })));
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::FrameBound { .. })));
    assert_eq!(vm.threads[&id].stack.len(), 2);
}

#[test]
fn reset_pop_fact_survives_failed_use_count_bookkeeping_without_becoming_return() {
    let (program, mut vm, id) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    vm.world.objects.get_mut(&2).unwrap().data.remove(&62);
    let mut thread = vm.threads.remove(&id).unwrap();
    let mut plain_thread = thread.clone();
    let mut plain_vm = vm.clone();
    let old_result = thread.last_result;
    let mut facts = Vec::new();
    let result = vm.pop_person_reset_frame_with_observation(&mut thread, Some(&program), Some(2),
        &mut Observation::new(&mut |fact| facts.push(fact), Some(id)));
    assert_eq!(result, plain_vm.pop_person_reset_frame(&mut plain_thread));
    assert!(result.is_err());
    assert_eq!(thread, plain_thread);
    assert_eq!(vm, plain_vm);
    assert_eq!(thread.last_result, old_result);
    assert_eq!(facts.len(), 1);
    assert!(matches!(facts[0], Fact::ResetUnwound { notification_target: Some(2), .. }));
}

#[test]
fn real_function_four_callback_tree_does_not_complete_an_active_action() {
    let (mut program, mut vm, id) = fixture();
    program.modules.get_mut("authored").unwrap().object_functions.insert(2, (0..5).map(|index| {
        sims_formats::execution::ObjectFunction { action: if index == 4 { 4098 } else { 0 }, condition: 0 }
    }).collect());
    vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().active = Some(record(9, 1));
    vm.world.actions.as_mut().unwrap().callbacks.push_back(action::Callback { object: 2, function: 4, stack_object: 1 });
    let (callback, owner, bhav) = vm.next_action_callback(&program).unwrap().unwrap();
    vm.world.actions.as_mut().unwrap().callbacks.pop_front();
    let mut thread = vm.threads.remove(&id).unwrap();
    vm.try_push_callback_start(&program, &mut thread, Start {
        owner: &owner, bhav, caller: callback.object, callee: callback.object,
        stack_object: callback.stack_object, args: vec![0; 4], check: true,
    }).unwrap();
    vm.threads.insert(id, thread);
    let mut facts = Vec::new();
    vm.run_local_slice_observed(&program, id, 1, &mut |fact| facts.push(fact)).unwrap();
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. } | Fact::ResetUnwound { .. })));
    assert_eq!(vm.world.actions.as_ref().unwrap().actors[&1].active.as_ref().unwrap().id, 9);
}

fn run_with_disabled_parity(program: &Program, vm: &mut Vm, id: ThreadId, budget: usize) -> (Result<Run, String>, Vec<Fact>) {
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let actual = vm.run_local_slice_observed(program, id, budget, &mut |fact| facts.push(fact));
    assert_eq!(actual, plain.run_local_slice(program, id, budget));
    assert_eq!(*vm, plain);
    assert_eq!(serde_json::to_vec(vm).unwrap(), serde_json::to_vec(&plain).unwrap());
    assert_eq!(vm.rng_state, plain.rng_state);
    assert_eq!(vm.tick, plain.tick);
    (actual, facts)
}

fn functional_binding(program: &mut Program) {
    let mut functions = vec![sims_formats::execution::ObjectFunction { action: 0, condition: 0 }; 30];
    functions[18].action = 4098;
    program.modules.get_mut("authored").unwrap().object_functions.insert(2, functions);
}

#[test]
fn delegated_ui_unknown_mode_reports_real_fe_ff_terminals_with_disabled_parity() {
    for (branch, expected) in [(254, NativeReturn::TrueFe), (255, NativeReturn::FalseFf)] {
        let (mut program, mut vm, id) = fixture();
        program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![Instruction {
            opcode: 0x22, on_true: branch, on_false: 253, operands: [255; 8],
        }]));
        vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
        let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 2);
        assert_ne!(run.unwrap().stop, Stop::Faulted);
        let exits: Vec<_> = facts.iter().filter_map(|fact| match fact {
            Fact::Returned { frame, branch, outermost_action_use, binding } => Some((frame, branch, outermost_action_use, binding)),
            _ => None,
        }).collect();
        assert_eq!(exits.len(), 1);
        assert_eq!(*exits[0].1, expected);
        assert!(*exits[0].2);
        assert_eq!(*exits[0].3, Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding));
        assert_eq!(exits[0].0.frame.location.bhav, 4097);
        assert_eq!(exits[0].0.frame.location.instruction, 0);
        assert_eq!(exits[0].0.depth, 2);
    }
}

#[test]
fn delegated_functional_unavailable_reports_the_actual_return_with_disabled_parity() {
    for (branch, expected) in [(254, NativeReturn::TrueFe), (255, NativeReturn::FalseFf)] {
        let (mut program, mut vm, id) = fixture();
        program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![Instruction {
            opcode: 0x14, on_true: 253, on_false: branch, operands: [255; 8],
        }]));
        vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
        let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 2);
        assert_ne!(run.unwrap().stop, Stop::Faulted);
        assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::Returned { branch, .. } if *branch == expected)).count(), 1);
        assert!(!facts.iter().any(|fact| matches!(fact, Fact::ActionUseEntered { .. })));
    }
}

#[test]
fn functional_nested_entry_and_two_exits_are_produced_in_one_slice_without_serial_adoption() {
    let (mut program, mut vm, id) = fixture();
    functional_binding(&mut program);
    program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![Instruction {
        opcode: 0x14, on_true: 254, on_false: 255, operands: [0; 8],
    }]));
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 3);
    assert_ne!(run.unwrap().stop, Stop::Faulted);
    let entry = facts.iter().position(|fact| matches!(fact, Fact::ActionUseEntered { .. })).unwrap();
    let Fact::ActionUseEntered { frame, binding } = &facts[entry] else { unreachable!() };
    assert_eq!(frame.frame.location.bhav, 4098);
    assert_eq!(frame.frame.location.instruction, 0);
    assert_eq!(frame.frame.action_target, Some(2));
    assert_eq!(frame.frame.storage_offset, 24);
    assert_eq!(frame.depth, 3);
    assert_eq!(frame.thread, Some(id));
    assert_eq!(*binding, Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding));
    let exits: Vec<_> = facts.iter().enumerate().filter_map(|(index, fact)| match fact {
        Fact::Returned { outermost_action_use, binding, .. } => Some((index, *outermost_action_use, binding)),
        _ => None,
    }).collect();
    assert_eq!(exits.len(), 2);
    assert!(exits[0].0 > entry);
    assert!(!exits[0].1);
    assert!(exits[1].1);
    assert!(exits.iter().all(|exit| *exit.2 == Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding)));
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::FrameBound { frame, .. } if frame.frame.location.bhav == 4098)));
}

#[test]
fn functional_entry_fact_precedes_a_later_icon_selection_error() {
    let (mut program, mut vm, id) = fixture();
    functional_binding(&mut program);
    program.modules.get_mut("authored").unwrap().routines.insert(4096, routine(vec![Instruction {
        opcode: 0x14, on_true: 254, on_false: 255, operands: [0, 0, 1, 0, 0, 0, 0, 0],
    }]));
    let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 1);
    assert_eq!(run.unwrap().stop, Stop::Faulted);
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ActionUseEntered { .. })).count(), 1);
    assert_eq!(vm.threads[&id].stack.len(), 2);
    assert_eq!(vm.world.objects[&2].data[&62], 1);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. } | Fact::FrameBound { .. })));
}

#[test]
fn failed_functional_push_does_not_report_entry() {
    let (mut program, mut vm, id) = fixture();
    functional_binding(&mut program);
    program.modules.get_mut("authored").unwrap().routines.remove(&4098);
    program.modules.get_mut("authored").unwrap().routines.insert(4096, routine(vec![Instruction {
        opcode: 0x14, on_true: 254, on_false: 255, operands: [0; 8],
    }]));
    let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 1);
    assert_eq!(run.unwrap().stop, Stop::Faulted);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::ActionUseEntered { .. })));
    assert_eq!(vm.threads[&id].stack.len(), 1);
}

#[test]
fn local_named_target_entry_and_terminal_use_the_same_borrowed_sink() {
    let (mut program, mut vm, id) = fixture();
    let module = program.modules.get_mut("authored").unwrap();
    module.routines.get_mut(&4098).unwrap().name = "authored named target".into();
    module.strings.insert(300, vec!["authored named target".into()]);
    module.routines.insert(4097, routine(vec![Instruction {
        opcode: 0x1c, on_true: 254, on_false: 255, operands: [44, 1, 0, 0, 1, 2, 0, 0],
    }]));
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 3);
    assert_ne!(run.unwrap().stop, Stop::Faulted);
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ActionUseEntered { frame, .. } if frame.frame.location.bhav == 4098)).count(), 1);
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::Returned { frame, .. } if frame.frame.location.bhav == 4098)).count(), 1);
}

#[test]
fn other_delegated_local_terminal_branches_preserve_disabled_execution() {
    for opcode in [0x0e, 0x1c, 0x16, 0x23, 3] {
        let (mut program, mut vm, id) = fixture();
        let operands = if opcode == 0x0e { [255; 8] } else if opcode == 0x16 { [1, 0, 0, 0, 0, 0, 0, 0] } else { [0; 8] };
        program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![Instruction {
            opcode, on_true: 254, on_false: 255, operands,
        }]));
        if opcode == 0x16 {
            vm.world.objects.get_mut(&1).unwrap().person.as_mut().unwrap().insert(0, 1);
        }
        if opcode == 3 {
            vm.world.person = Some(crate::person::WorldState { free_will: false, ..Default::default() });
        }
        vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
        vm.run_local_slice(&program, id, 1).unwrap();
        if opcode == 0x23 {
            vm.threads.get_mut(&id).unwrap().stack.last_mut().unwrap().continuation = 2;
        }
        let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 1);
        assert_ne!(run.unwrap().stop, Stop::Faulted, "opcode {opcode:#x}");
        assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::Returned { .. })).count(), 1, "opcode {opcode:#x}");
    }
}

#[test]
fn reset_final_use_reports_serial_specific_clear_before_the_first_host_request() {
    let (program, mut vm, id) = fixture();
    let (serial, _) = vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    let mut plain = vm.clone();
    let mut reset = crate::person_reset::Reset::new(1, 0);
    let mut plain_reset = reset.clone();
    let mut facts = Vec::new();
    let request = reset.next_observed(&mut vm, &program, &mut |fact| facts.push(fact)).unwrap();
    assert_eq!(request, plain_reset.next(&mut plain, &program).unwrap());
    assert_eq!(request, Some(crate::person_reset::Request::NotifyUsed(2)));
    assert_eq!(vm, plain);
    assert_eq!(reset, plain_reset);
    let unwind = facts.iter().position(|fact| matches!(fact, Fact::ResetUnwound { .. })).unwrap();
    let clears: Vec<_> = facts.iter().enumerate().filter_map(|(index, fact)| match fact {
        Fact::ActiveCleared { action, cause } => Some((index, action.serial, *cause)),
        _ => None,
    }).collect();
    assert_eq!(clears, vec![(unwind + 1, serial, ClearCause::Reset)]);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
    assert!(vm.world.actions.as_ref().unwrap().actors[&1].active.is_none());
    vm.clear_person_action_with_observation(1, true, ClearCause::Reset, &mut Observation::new(&mut |fact| facts.push(fact), Some(id))).unwrap();
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ActiveCleared { .. })).count(), 1);
}

#[test]
fn nested_reset_unwind_clears_the_active_record_only_when_the_last_use_disappears() {
    let (program, mut vm, id) = fixture();
    let (serial, _) = vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    vm.run_local_slice(&program, id, 1).unwrap();
    let mut thread = vm.threads.remove(&id).unwrap();
    vm.push_target_tree(&program, &mut thread, "authored".into(), 4098, 2, 0).unwrap();
    let mut plain = vm.clone();
    let mut plain_thread = thread.clone();
    let mut facts = Vec::new();
    for final_use in [false, true] {
        let result = vm.pop_person_reset_frame_with_observation(&mut thread, Some(&program), Some(2), &mut Observation::new(&mut |fact| facts.push(fact), Some(id)));
        assert_eq!(result, plain.pop_person_reset_frame(&mut plain_thread));
        result.unwrap();
        assert_eq!(thread, plain_thread);
        assert_eq!(vm, plain);
        assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ActiveCleared { .. })).count(), usize::from(final_use));
    }
    assert!(facts.iter().any(|fact| matches!(fact, Fact::ActiveCleared { action, cause: ClearCause::Reset } if action.serial == serial)));
    assert_eq!(facts.iter().filter(|fact| matches!(fact, Fact::ResetUnwound { binding: Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding), .. })).count(), 2);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. })));
}

#[test]
fn dispatch_failure_preserves_the_serial_allocated_before_missing_actor_storage() {
    for previous_serial in [41, u32::MAX] {
        let (mut program, mut vm, id) = fixture();
        program.modules.get_mut("authored").unwrap().routines.insert(4096, routine(vec![Instruction {
            opcode: 30, on_true: 254, on_false: 255, operands: [0; 8],
        }]));
        vm.world.objects.get_mut(&1).unwrap().data.insert(20, 19);
        let actions = vm.world.actions.as_mut().unwrap();
        actions.serial = previous_serial;
        actions.actors.clear();
        let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 1);
        assert_eq!(run.unwrap().stop, Stop::Faulted);
        let allocated = previous_serial.wrapping_add(1);
        assert_eq!(transitions(&facts), vec![(allocated, QueueTransition::SerialAssigned)]);
        let assignment = facts.iter().position(|fact| matches!(fact, Fact::Queue { transition: QueueTransition::SerialAssigned, .. })).unwrap();
        let failure = facts.iter().position(|fact| matches!(fact, Fact::DispatchFailed { action, frame_entered: Evidence::Known(true) } if action.serial == allocated)).unwrap();
        assert!(failure > assignment);
        assert!(facts.iter().any(|fact| matches!(fact, Fact::FrameEntered { candidate, serial_assignment_pending: true, .. } if candidate.serial == 0)));
        assert!(!facts.iter().any(|fact| matches!(fact, Fact::FrameBound { .. })));
        assert_eq!(vm.world.actions.as_ref().unwrap().serial, allocated);
        assert_eq!(vm.threads[&id].stack.len(), 2);
    }
}

#[test]
fn direct_notification_scan_reports_removals_but_preserves_unrelated_records_and_execution() {
    let (program, mut vm, _) = fixture();
    let mut unrelated = record(10, 1);
    unrelated.target = 1;
    unrelated.icon_owner = 1;
    let mut icon_match = unrelated.clone();
    icon_match.id = 12;
    icon_match.icon_owner = 2;
    vm.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().queue.pending =
        [unrelated.clone(), record(11, 1), icon_match].into();
    let mut plain = vm.clone();
    let mut reset = crate::person_reset::Reset::notified(1, 0, 2, 0);
    let mut plain_reset = reset.clone();
    let mut facts = Vec::new();
    let mut requests = Vec::new();
    for _ in 0..4 {
        let result = reset.next_observed(&mut vm, &program, &mut |fact| facts.push(fact));
        assert_eq!(result, plain_reset.next(&mut plain, &program));
        assert_eq!(vm, plain);
        assert_eq!(reset, plain_reset);
        let request = result.unwrap();
        if request.is_none() { break; }
        requests.push(request.unwrap());
    }
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| matches!(request, crate::person_reset::Request::Cancel(_))));
    assert_eq!(transitions(&facts), vec![
        (10, QueueTransition::ReferenceScanDequeued { target: 2 }),
        (11, QueueTransition::ReferenceScanDequeued { target: 2 }),
        (11, QueueTransition::ReferenceRemoved { target: 2 }),
        (12, QueueTransition::ReferenceScanDequeued { target: 2 }),
        (12, QueueTransition::ReferenceRemoved { target: 2 }),
    ]);
    assert_eq!(vm.world.actions.as_ref().unwrap().actors[&1].queue.pending, std::collections::VecDeque::from([unrelated]));
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. } | Fact::ResetUnwound { .. } | Fact::ActiveCleared { .. })));
}

#[test]
fn unrelated_direct_reference_control_does_not_claim_a_return_or_a_removal() {
    let (program, mut vm, _) = fixture();
    vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
    let mut plain = vm.clone();
    let mut facts = Vec::new();
    let actual = vm.remove_action_object_references_observed(1, 1, &mut |fact| facts.push(fact));
    let expected = plain.world.actions.as_mut().unwrap().actors.get_mut(&1).unwrap().remove_object_references(1);
    assert_eq!(actual, expected);
    assert_eq!(vm, plain);
    assert!(facts.is_empty());
}

#[test]
fn delegated_local_branch_errors_preserve_partial_state_without_fabricating_returns() {
    for opcode in [0x22, 0x14] {
        let (mut program, mut vm, id) = fixture();
        program.modules.get_mut("authored").unwrap().routines.insert(4097, routine(vec![Instruction {
            opcode, on_true: 253, on_false: 253, operands: [255; 8],
        }]));
        vm.enqueue_action(&program, 1, record(0, 100)).unwrap();
        let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 2);
        assert_eq!(run.unwrap().stop, Stop::Faulted);
        assert_eq!(vm.threads[&id].stack.len(), 2);
        assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. } | Fact::ActionUseEntered { .. })));
        assert_eq!(vm.world.objects[&2].data[&62], 1);
    }
}

#[test]
fn functional_ordinary_object_entry_is_not_an_action_use_entry_or_terminal() {
    let (mut program, mut vm, id) = fixture();
    functional_binding(&mut program);
    vm.world.objects.get_mut(&1).unwrap().person = None;
    program.modules.get_mut("authored").unwrap().routines.insert(4096, routine(vec![Instruction {
        opcode: 0x14, on_true: 254, on_false: 255, operands: [0; 8],
    }]));
    let (run, facts) = run_with_disabled_parity(&program, &mut vm, id, 2);
    assert_ne!(run.unwrap().stop, Stop::Faulted);
    assert!(!facts.iter().any(|fact| matches!(fact, Fact::Returned { .. } | Fact::ActionUseEntered { .. })));
    assert_eq!(vm.world.objects[&2].data[&62], 0);
}

#[test]
fn shared_entry_hook_runs_at_push_before_a_later_retained_storage_error() {
    let (program, mut vm, id) = fixture();
    let mut thread = vm.threads.remove(&id).unwrap();
    thread.retained_frame_arena = Some(serde_json::from_value(serde_json::json!({
        "bindings": {"2": {
            "object": 2,
            "definition": vm.world.objects[&2].definition.clone(),
            "native_context": 123,
            "registry_lifetime": 2
        }},
        "active_contexts": {}
    })).unwrap());
    thread.frame_memory.resize(24, Some(0));
    let mut frame = thread.stack[0].clone();
    frame.storage_offset = 12;
    frame.action_target = Some(2);
    let mut plain_thread = thread.clone();
    let mut facts = Vec::new();
    let result = crate::retained_frames::finish_entry_with(&mut thread, frame.clone(), Some(2), |entered| {
        facts.push(Fact::ActionUseEntered {
            frame: FrameFingerprint::capture(&vm.world, Some(&program), Some(id), entered.stack.len(), entered.stack.last().unwrap()),
            binding: Evidence::Unavailable(Unavailable::FrameHasNoSerialBinding),
        });
    });
    assert_eq!(result, crate::retained_frames::finish_entry(&mut plain_thread, frame, Some(2)));
    assert!(result.is_err());
    assert_eq!(thread, plain_thread);
    assert_eq!(thread.stack.len(), 2);
    assert_eq!(facts.len(), 1);
    assert!(matches!(&facts[0], Fact::ActionUseEntered { frame, .. } if frame.depth == 2 && frame.frame.storage_offset == 12));
}
