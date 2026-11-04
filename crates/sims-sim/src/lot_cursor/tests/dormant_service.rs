//! Authored ownership checks; the private128-tick case covers the real portal.
use super::*;
use sims_vm::{Start, Status, ThreadId};

fn park_service(p: &Program, s: &mut Simulation) -> ThreadId {
    let thread = s.vm.object_threads[&1];
    assert_eq!(s.vm.lifecycles[&1].phase, sims_vm::lifecycle::Phase::Main);
    s.vm.push_service_call(
        p,
        thread,
        Start {
            owner: "objects",
            bhav: 4102,
            caller: 1,
            callee: 2,
            stack_object: 2,
            args: vec![0; 4],
            check: false,
        },
    )
    .unwrap();
    s.vm.run_slice(p, thread, 2).unwrap();
    assert!(s.vm.threads[&thread].service.is_some());
    assert!(!matches!(
        s.vm.threads[&thread].status,
        Status::Faulted(_) | Status::Returned(_)
    ));
    thread
}
fn service_fixture() -> (Program, Simulation, Definition, ThreadId) {
    let (p, mut s, d) = fixture(false);
    let thread = park_service(&p, &mut s);
    (p, s, d, thread)
}
fn unchanged_rejection(p: &Program, mut s: Simulation) {
    let before = serde_json::to_vec(&s).unwrap();
    assert_eq!(
        s.begin_cursor_factory(p, None, 0, 1, false, Default::default()),
        Err("cursor factory overlaps synchronous work".into())
    );
    assert_eq!(serde_json::to_vec(&s).unwrap(), before);
    assert_eq!(
        s.begin_object_pickup(p, 1, false),
        Err("pickup overlaps synchronous work".into())
    );
    assert_eq!(serde_json::to_vec(&s).unwrap(), before);
    // An empty, real editor execution is sufficient here: active owners must
    // reject before inspecting a placement caller, without changing state.
    if s.cursor_has_active_vm_callback() {
        let caller = crate::edit_history::object::driver::Execution::new(
            crate::edit_history::command::Request {
                command: 7,
                method: crate::edit_history::command::Method::Undo,
            },
        )
        .unwrap();
        assert_eq!(
            s.begin_object_command_placement(p, &caller, false),
            Err("snapshot placement overlaps synchronous work".into())
        );
        assert_eq!(serde_json::to_vec(&s).unwrap(), before);
    }
}

#[test]
fn parked_registered_main_service_is_not_a_global_cursor_owner() {
    let (p, mut s, d, thread) = service_fixture();
    let before = serde_json::to_vec(&s).unwrap();
    assert_eq!(
        s.begin_cursor_factory(&p, None, 0, 1, false, Default::default()),
        Ok(false)
    );
    assert_eq!(
        serde_json::to_vec(&s).unwrap(),
        before,
        "mere guard/resource check changed state or RNG"
    );
    let parked = serde_json::to_vec(&s.vm.threads[&thread]).unwrap();
    let rng = (s.vm.rng_state, s.vm.lot_rng_state, s.vm.crt_rng_state);
    assert_eq!(
        s.begin_cursor_factory(&p, Some(d), 0, 1, false, Default::default()),
        Ok(true)
    );
    assert_eq!(serde_json::to_vec(&s.vm.threads[&thread]).unwrap(), parked);
    assert_eq!(
        (s.vm.rng_state, s.vm.lot_rng_state, s.vm.crt_rng_state),
        rng
    );
}

#[test]
fn parked_service_also_allows_each_concrete_cursor_lifecycle_entry() {
    let (p, mut base, cursor) = returned_cursor(false, 0);
    let thread = park_service(&p, &mut base);
    let parked = serde_json::to_vec(&base.vm.threads[&thread]).unwrap();
    for operation in 0..4 {
        let mut s = base.clone();
        match operation {
            0 => s.begin_cursor_detach(&p, cursor).unwrap(),
            1 => s.begin_cursor_rotation(&p, cursor, 1, false).unwrap(),
            2 => s
                .begin_cursor_cleanup(&p, cursor, cleanup::Kind::Discard)
                .unwrap(),
            _ => s.begin_cursor_release(&p, cursor).unwrap(),
        }
        assert_eq!(serde_json::to_vec(&s.vm.threads[&thread]).unwrap(), parked);
        assert_eq!(
            (s.vm.rng_state, s.vm.lot_rng_state, s.vm.crt_rng_state),
            (
                base.vm.rng_state,
                base.vm.lot_rng_state,
                base.vm.crt_rng_state
            )
        );
    }
}

#[test]
fn parked_service_reaches_pickup_and_snapshot_caller_validation_unchanged() {
    let (p, base, _, thread) = service_fixture();
    let mut s = base.clone();
    let parked = serde_json::to_vec(&s.vm.threads[&thread]).unwrap();
    s.begin_object_pickup(&p, 2, false).unwrap();
    assert_eq!(serde_json::to_vec(&s.vm.threads[&thread]).unwrap(), parked);
    assert_eq!(
        (s.vm.rng_state, s.vm.lot_rng_state, s.vm.crt_rng_state),
        (
            base.vm.rng_state,
            base.vm.lot_rng_state,
            base.vm.crt_rng_state
        )
    );

    let mut s = base;
    let before = serde_json::to_vec(&s).unwrap();
    let caller = crate::edit_history::object::driver::Execution::new(
        crate::edit_history::command::Request {
            command: 7,
            method: crate::edit_history::command::Method::Undo,
        },
    )
    .unwrap();
    assert_eq!(
        s.begin_object_command_placement(&p, &caller, false),
        Err("snapshot placement caller absent".into()),
        "a dormant Main must not replace the precise editor caller check"
    );
    assert_eq!(serde_json::to_vec(&s).unwrap(), before);
}

#[test]
fn unrelated_or_forged_main_service_ownership_keeps_the_guard_and_exact_input() {
    let (p, base, _, thread) = service_fixture();
    for case in 0..10 {
        let mut s = base.clone();
        match case {
            0 => {
                s.vm.object_threads.remove(&1);
            }
            1 => s.vm.lifecycles.get_mut(&1).unwrap().phase = sims_vm::lifecycle::Phase::Init,
            2 => s.vm.lifecycles.get_mut(&1).unwrap().thread = thread + 100,
            3 => s.vm.lifecycles.get_mut(&1).unwrap().owner = "unrelated".into(),
            4 => s.vm.lifecycles.get_mut(&1).unwrap().definition = 99,
            5 => s.vm.world.scheduler_storage = Some(Default::default()),
            6 => s.vm.threads.get_mut(&thread).unwrap().stack[0].caller = 2,
            7 => {
                s.vm.threads
                    .get_mut(&thread)
                    .unwrap()
                    .service
                    .as_mut()
                    .unwrap()
                    .caller = 2
            }
            8 => {
                s.vm.threads
                    .get_mut(&thread)
                    .unwrap()
                    .service
                    .as_mut()
                    .unwrap()
                    .base_depth = 0
            }
            _ => {
                s.vm.threads
                    .get_mut(&thread)
                    .unwrap()
                    .service
                    .as_mut()
                    .unwrap()
                    .result = Some(true)
            }
        }
        unchanged_rejection(&p, s);
    }
}

#[test]
fn actual_synchronous_owners_are_not_mistaken_for_parked_main_services() {
    let (p, base, _, thread) = service_fixture();
    for case in 0..5 {
        let mut s = base.clone();
        let frame = s.vm.threads[&thread].stack.last().unwrap().clone();
        match case {
            0 => {
                s.immediate = Some(crate::immediate::Execution {
                    actor: 1,
                    thread,
                    action: sims_vm::action::Action {
                        target: 2,
                        ..Default::default()
                    },
                    owner: "objects".into(),
                    requester: None,
                    phase: crate::immediate::Phase::Check,
                    saved_temps: None,
                    priority: 0,
                })
            }
            1 => {
                s.reach_callback = Some(crate::reach::Callback {
                    request: sims_vm::reach::Request {
                        id: 1,
                        origin: frame.location,
                        caller: 1,
                        target: 2,
                        operands: [0; 8],
                    },
                    thread,
                    object: 2,
                    owner: "objects".into(),
                    bhav: 4102,
                    depth: s.vm.threads[&thread].stack.len(),
                })
            }
            2 => {
                s.cancellation = Some(crate::cancellation::Execution {
                    callback: sims_vm::action::Callback {
                        object: 2,
                        function: 4,
                        stack_object: 1,
                    },
                    thread,
                    depth: s.vm.threads[&thread].stack.len(),
                    owner: "objects".into(),
                    bhav: 4102,
                    backlog: Default::default(),
                })
            }
            3 => {
                let t = s.vm.threads.get_mut(&thread).unwrap();
                t.callback = Some(sims_vm::CallbackBoundary {
                    external: true,
                    parent: None,
                    base_depth: t.stack.len(),
                    resume_status: t.status.clone(),
                    resume_check: t.check,
                });
            }
            _ => s.visit = Some((thread, Some(1))),
        }
        unchanged_rejection(&p, s);
    }
}
