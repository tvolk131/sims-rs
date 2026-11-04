//! Actual Simulation/VM scheduling, not synthetic Tick acknowledgements.
use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{Room, Simulation, Snapshot, StepStop};
use sims_vm::{Module, Object, Program, Routine, Start, Vm, World};
use std::collections::BTreeMap;

fn fixture(cursor: i32) -> (Program, Simulation, [u64; 2]) {
    let program = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "Sleep".into(),
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
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut world = World {
        scheduler_storage: Some(Default::default()),
        globals: [(10, 7), (17, 0), (19, 2)].into(),
        ..Default::default()
    };
    for id in [1, 2] {
        let mut object = Object {
            native_tree_error: Some(0),
            data: [(1, 0), (25, 9)].into(),
            ..Default::default()
        };
        world.register_scheduler_object(id, &mut object).unwrap();
        object.idle.as_mut().unwrap().flags = if id == 1 { 3 } else { 1 };
        world.objects.insert(id, object);
    }
    let mut vm = Vm::new(world, 1);
    vm.lot_rng_state = Some(1825128472);
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
                    stack_object: id,
                    args: vec![10, 0, 0, 0],
                    check: false,
                },
            )
            .unwrap();
        vm.register_object_thread(id, thread).unwrap();
        threads[id as usize - 1] = thread;
    }
    let mut sim = Simulation::from_placements(
        vm,
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        [1, 2]
            .map(|id| {
                (
                    id,
                    sims_sim::Body {
                        position: sims_sim::Point::new(i32::from(id) * 16, 16),
                        level: 1,
                        facing: 1,
                        radius: 1,
                        elevation: 0,
                    },
                )
            })
            .into(),
        Default::default(),
        None,
    )
    .unwrap();
    sim.enable_registry_scheduler(cursor).unwrap();
    (program, sim, threads)
}
fn restore(p: &Program, sim: &mut Simulation) {
    let saved = serde_json::to_vec(&Snapshot::capture(p, sim)).unwrap();
    *sim = serde_json::from_slice::<Snapshot>(&saved)
        .unwrap()
        .restore(p)
        .unwrap();
}
fn run(p: &Program, sim: &mut Simulation, budget: usize) -> Vec<i16> {
    let mut visits = vec![];
    for _ in 0..100 {
        let step = sim.run_current_tick(p, budget).unwrap();
        visits.extend(step.trace.iter().map(|t| {
            *sim.vm
                .object_threads
                .iter()
                .find(|(_, id)| **id == t.thread)
                .unwrap()
                .0
        }));
        restore(p, sim);
        if step.stop != StepStop::Budget {
            return visits;
        }
    }
    panic!("scheduler failed to complete within its diagnostic bound");
}

#[test]
fn simulation_paused_passes_execute_controllers_and_resume_live_without_clock_fabrication() {
    let (p, initial, [first, second]) = fixture(0);
    let mut results = vec![];
    for budget in [1000, 1] {
        let mut sim = initial.clone();
        let frozen_thread = sim.vm.threads[&second].clone();
        let frozen_object = sim.vm.world.objects[&2].clone();
        for expected in [9, 8, 7] {
            assert_eq!(run(&p, &mut sim, budget), [1]);
            assert_eq!(sim.native_scheduler.as_ref().unwrap().result(), Some(false));
            let before = sim.clone();
            assert!(sim.advance_fixture_tick().is_err());
            assert_eq!(sim, before);
            assert!(!sim.finish_registry_pass().unwrap());
            assert_eq!(sim.vm.tick, 500);
            assert_eq!(sim.vm.threads[&first].stack[0].args[0], expected);
            assert_eq!(sim.vm.threads[&second], frozen_thread);
            assert_eq!(sim.vm.world.objects[&2], frozen_object);
            restore(&p, &mut sim);
        }
        sim.vm.world.globals.insert(19, 0);
        assert_eq!(run(&p, &mut sim, budget), [1, 2]);
        assert!(sim.finish_registry_pass().unwrap());
        assert_eq!(sim.vm.tick, 501);
        assert_eq!(sim.vm.threads[&second].stack[0].args[0], 9);
        restore(&p, &mut sim);
        results.push(sim);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn saved_registry_cursor_and_captured_mode_survive_zero_budget_and_restore() {
    let (p, mut sim, [first, second]) = fixture(2);
    sim.vm.world.globals.insert(19, 0);
    let frozen = sim.vm.threads[&first].clone();
    assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
    assert_eq!(sim.visit, Some((second, Some(2))));
    restore(&p, &mut sim);
    sim.vm.world.globals.insert(19, 2);
    assert_eq!(run(&p, &mut sim, 1), [2]);
    assert_eq!(sim.vm.threads[&first], frozen);
    assert!(
        sim.finish_registry_pass().unwrap(),
        "entry gate must survive a callback's mode change"
    );
    assert_eq!(sim.native_scheduler.as_ref().unwrap().cursor, 0);
    assert_eq!(run(&p, &mut sim, 1), [1]);
    assert!(!sim.finish_registry_pass().unwrap());
    assert_eq!(sim.vm.tick, 501);
}

#[test]
fn native_error_service_is_a_stable_boundary_and_does_not_advance_time() {
    let (p, mut sim, _) = fixture(0);
    let object = sim.vm.world.objects.get_mut(&1).unwrap();
    object.native_tree_error = Some(8);
    object.idle.as_mut().unwrap().flags = 4;
    assert!(
        sim.run_current_tick(&p, 100)
            .unwrap_err()
            .contains("439a6f")
    );
    restore(&p, &mut sim);
    let before = sim.clone();
    assert!(
        sim.run_current_tick(&p, 100)
            .unwrap_err()
            .contains("439a6f")
    );
    assert!(sim.finish_registry_pass().is_err());
    assert_eq!(sim, before);
    assert_eq!(sim.vm.tick, 500);
}

#[test]
fn native_pass_restore_rejects_lost_owners_and_clock_changes() {
    let (p, mut sim, _) = fixture(0);
    assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
    let before = Snapshot::capture(&p, &sim);
    let mut missing = before.clone();
    missing.state.native_scheduler = None;
    assert!(missing.restore(&p).is_err());
    let mut missing = before.clone();
    missing.state.vm.scheduled_object_visit = None;
    assert!(missing.restore(&p).is_err());
    let mut changed = before;
    changed.state.vm.tick += 1;
    assert!(changed.restore(&p).is_err());
    sim.native_scheduler.as_mut().unwrap().cursor = -1;
    let before = sim.clone();
    assert!(sim.run_current_tick(&p, 100).is_err());
    assert_eq!(sim, before);
}

#[test]
fn suspended_visit_resumes_before_checking_new_person_hosts() {
    let (p, mut sim, [first, second]) = fixture(0);
    assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
    // Model the scheduler-visible interval between a factory registering a
    // person and its asset host installing the character. This is a selection
    // test, not a fabricated completed factory or valid standalone snapshot.
    sim.vm.world.objects.get_mut(&2).unwrap().person = Some(Default::default());
    let other = sim.vm.threads[&second].clone();
    assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
    // The current Update must be allowed to finish. Selection of another
    // object still rejects the missing character host at the next boundary.
    assert!(
        sim.run_current_tick(&p, 1000)
            .unwrap_err()
            .contains("native scheduler requires native person/animation hosts")
    );
    assert_eq!(sim.vm.threads[&first].stack[0].args[0], 9);
    assert_eq!(sim.vm.threads[&second], other);
    assert!(sim.visit.is_none());
    assert_eq!(sim.vm.tick, 500);
}

#[test]
fn random_instructions_and_yielded_services_keep_the_same_lot_stream_across_restores() {
    for paused in [false, true] {
        let (mut p, mut initial, _) = fixture(0);
        initial.vm.rng_state = 2449368041;
        initial.vm.crt_rng_state = Some(1234);
        initial
            .vm
            .world
            .globals
            .insert(19, if paused { 2 } else { 0 });
        let random = |temp| Instruction {
            opcode: 8,
            on_true: temp + 1,
            on_false: 255,
            operands: [temp, 0, 8, 0, 100, 0, 7, 0],
        };
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions = vec![
            random(0),
            random(1),
            Instruction {
                opcode: 0,
                on_true: 254,
                on_false: 255,
                operands: [0; 8],
            },
        ];
        let mut results = vec![];
        for budget in [1, 1000] {
            let mut sim = initial.clone();
            assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
            assert_eq!(sim.vm.rng_state, 1825128472);
            let mut lot = 1825128472;
            // A synchronous service nested inside the suspended object visit
            // sees the same active process stream as Random Number instructions.
            assert_eq!(
                sims_vm::rng::next(&mut sim.vm.rng_state),
                sims_vm::rng::next(&mut lot)
            );
            restore(&p, &mut sim);
            for _ in 0..2 {
                let before = sim.clone();
                assert_eq!(sim.run_current_tick(&p, 0).unwrap().stop, StepStop::Budget);
                assert_eq!(
                    sim, before,
                    "resuming a budget must not re-enter the RNG swap"
                );
            }
            run(&p, &mut sim, budget);
            for id in if paused { vec![1] } else { vec![1, 2] } {
                for i in 0..2 {
                    let expected = (sims_vm::rng::next(&mut lot) % 100) as i16;
                    assert_eq!(sim.vm.world.objects[&id].temps[i], expected);
                }
            }
            assert_eq!(sim.vm.lot_rng_state, Some(lot));
            assert!(sim.vm.lot_rng_scope.is_none());
            assert_eq!(sim.vm.rng_state, 2449368041);
            assert_eq!(sim.vm.crt_rng_state, Some(1234));
            restore(&p, &mut sim);
            // World/tail callbacks use global RNG even before the caller has
            // published the clock boundary. Ending that boundary cannot swap again.
            let mut global = 2449368041;
            assert_eq!(
                sims_vm::rng::next(&mut sim.vm.rng_state),
                sims_vm::rng::next(&mut global)
            );
            assert_eq!(sim.finish_registry_pass().unwrap(), !paused);
            assert_eq!(sim.vm.rng_state, global);
            assert_eq!(sim.vm.lot_rng_state, Some(lot));
            restore(&p, &mut sim);
            results.push(sim);
        }
        assert_eq!(results[0], results[1]);
    }
}

#[test]
fn suspended_error_service_requires_its_rng_owner_and_rejects_legacy_midpass_snapshots() {
    let (p, mut sim, _) = fixture(0);
    let object = sim.vm.world.objects.get_mut(&1).unwrap();
    object.native_tree_error = Some(8);
    object.idle.as_mut().unwrap().flags = 4;
    assert!(sim.run_current_tick(&p, 10).unwrap_err().contains("439a6f"));
    assert!(sim.vm.scheduled_object_visit.is_none());
    assert!(sim.vm.lot_rng_scope.is_some());
    restore(&p, &mut sim);
    let before = Snapshot::capture(&p, &sim);
    let mut missing = before.clone();
    missing.state.native_scheduler = None;
    assert!(missing.restore(&p).unwrap_err().contains("RNG scope"));
    let mut legacy = serde_json::to_value(&before).unwrap();
    legacy["state"]["vm"]
        .as_object_mut()
        .unwrap()
        .remove("lot_rng_scope");
    assert!(
        serde_json::from_value::<Snapshot>(legacy)
            .unwrap()
            .restore(&p)
            .unwrap_err()
            .contains("RNG scope")
    );
    let mut changed = before;
    changed.state.vm.lot_rng_state = Some(99);
    assert!(changed.restore(&p).unwrap_err().contains("stored seed"));
}

#[test]
fn gameplay_never_falls_back_to_bare_thread_scheduling() {
    let (p, mut sim, _) = fixture(0);
    let before = sim.clone();
    assert!(sim.run_fixture_tick(&p, 100).is_err());
    assert_eq!(sim, before, "fixture entry changed a real scheduler owner");
    sim.native_scheduler = None;
    let before = sim.clone();
    let error = sim.run_current_tick(&p, 100).unwrap_err();
    assert!(error.contains("legacy snapshots cannot be silently upgraded"));
    assert_eq!(sim, before, "missing registry owner executed bare threads");
}
