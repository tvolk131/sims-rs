//! Synthetic wrapper integration; native factory/visitor corpora cover the
//! component semantics. No original household state is manufactured here.
use super::*;
use crate::{Snapshot, StepStop, clock::LotClock};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{Module, Routine, person};

fn instruction(opcode: u16, operands: [u8; 8], next: u8) -> Instruction {
    Instruction {
        opcode,
        operands,
        on_true: next,
        on_false: 255,
    }
}
fn routine(name: &str, args: u8, instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: name.into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args,
            locals: 0,
            instructions,
        },
    }
}
// This authored wrapper fixture deliberately starts beyond the producer gate.
// Only the loaded byte is known: no FCNS/provider/control/tuning value is
// fabricated, and later consumers must still reject the unknown words.
fn authored_loaded_process() -> sims_vm::autonomy_constants::Owner {
    let mut owner = sims_vm::autonomy_constants::Owner::default();
    owner.process.loaded = Some(1);
    owner
}

fn fixture(
    neighbor: bool,
    autonomous: bool,
    visitor: bool,
    flagged: bool,
) -> (Program, Simulation, ThreadId) {
    fixture_mode(neighbor, autonomous, visitor, flagged, 6)
}
fn fixture_mode(
    neighbor: bool,
    autonomous: bool,
    visitor: bool,
    flagged: bool,
    mode: u8,
) -> (Program, Simulation, ThreadId) {
    let (mut p, mut s) = crate::lot_runtime::tests::fixture();
    s.vm.world.autonomy_constants = Some(authored_loaded_process());
    let global = p.modules.get_mut("objects").unwrap();
    for id in [1, 5, 6] {
        global.named_constants.insert(
            id,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for id in [501, 502, 503, 504] {
        global.strings.insert(id, vec!["(-100;1)(100;1)".into(); 9]);
    }
    global.strings.insert(8192, vec!["".into(), "lgt".into()]);
    global.routines.insert(
        4096,
        routine(
            "creator",
            0,
            vec![
                instruction(42, [0, 0, 0, 0, mode, 4, 0, 0], 1),
                instruction(2, [0, 0, 0, 0, 0, 5, 8, 8], 254),
            ],
        ),
    );
    let mut m = Module {
        source: "person".into(),
        semiglobal: Some("objects".into()),
        ..Default::default()
    };
    let mut words = vec![0; 128];
    words[9] = 2;
    words[14] = 99;
    words[19] = 200;
    m.object_definition_words.insert(1, words);
    let mut callbacks = vec![
        sims_formats::execution::ObjectFunction {
            action: 0,
            condition: 0
        };
        27
    ];
    callbacks[0].action = 4097;
    callbacks[1].action = 4096;
    m.object_functions.insert(1, callbacks);
    let mut profile = vec![String::new(); 35];
    profile[12] = "male".into();
    profile[13] = "27".into();
    profile[14] = "lgt".into();
    m.strings.insert(200, profile);
    m.routines.insert(
        4096,
        routine(
            "Main",
            1,
            vec![
                instruction(2, [0, 0, 10, 0, 0, 5, 9, 7], 1),
                instruction(0, [0; 8], 0),
            ],
        ),
    );
    m.routines.insert(
        4097,
        routine(
            "Init",
            0,
            vec![instruction(
                2,
                [32, 0, u8::from(autonomous), 0, 0, 5, 18, 7],
                254,
            )],
        ),
    );
    if visitor {
        m.routines.insert(
            4098,
            routine(
                "init visitor",
                0,
                vec![
                    instruction(2, [7, 0, 1, 0, 0, 3, 8, 7], 1),
                    instruction(2, [6, 0, 321u16 as u8, 1, 0, 5, 8, 7], 254),
                ],
            ),
        );
    }
    p.modules.insert(m.source.clone(), m);
    s.architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&p)
        .unwrap();
    s.vm.world.refresh = Some(Default::default());
    s.vm.world.actions = Some(Default::default());
    let people = s.vm.world.person.as_mut().unwrap();
    people.lot_zoning.insert(1, 1);
    people.neighbor_slots = Some(vec![None; 25]);
    if neighbor {
        people.neighbor_slots.as_mut().unwrap()[24] = Some(25);
        people.neighbors.insert(
            25,
            person::Neighbor {
                guid: 99,
                person_revision: 5,
                person: (0..256)
                    .map(|i| (i, if i == 68 && flagged { 1 } else { 0 }))
                    .collect(),
                ..Default::default()
            },
        );
    }
    let mut created_image = sims_vm::image_cache::Entry::new(99);
    created_image.images[0] = Some(1);
    let mut caller_image = sims_vm::image_cache::Entry::new(1);
    caller_image.images[0] = Some(2);
    s.vm.world.image_cache.entries = vec![created_image, caller_image];
    s.vm.world.image_cache.refs = vec![1, 1];
    s.vm.crt_rng_state = Some(0x51a5cafe);
    s.enable_household(
        &p,
        LotClock {
            hour: 12,
            day: 1,
            period: 0,
            minute: 0,
            second: 0,
            month: 1,
            year: 1997,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .unwrap();
    s.vm.world.globals.insert(10, 1);
    let parent = s.vm.object_threads[&4];
    s.vm.threads.get_mut(&parent).unwrap().stack[0].stack_object = 25;
    s.vm.run_slice(&p, parent, 1).unwrap();
    assert!(matches!(
        s.vm.threads[&parent].status,
        Status::AwaitingCreation(_)
    ));
    (p, s, parent)
}

#[test]
fn offlot_neighbor_wrapper_preserves_visitor_order_and_every_budget_boundary() {
    for flagged in [false, true] {
        for neighbor in [false, true] {
            for autonomous in [false, true] {
                for visitor in [false, true] {
                    let mut results = vec![];
                    for budget in [1, 1024] {
                        let (p, mut s, parent) = fixture(neighbor, autonomous, visitor, flagged);
                        let tick = s.vm.tick;
                        let mut visitor_saves = 0;
                        let mut complete = false;
                        for _ in 0..1000 {
                            let Status::AwaitingCreation(r) = s.vm.threads[&parent].status.clone()
                            else {
                                panic!("creator not suspended")
                            };
                            let mut step = Step {
                                stop: StepStop::Waiting,
                                trace: vec![],
                                events: vec![],
                            };
                            complete = s
                                .service_creation(&p, parent, r, &mut budget.clone(), &mut step)
                                .unwrap();
                            s.sync_spatial_world();
                            assert_eq!(s.vm.tick, tick);
                            assert_eq!(
                                s.vm.world.autonomy_constants,
                                Some(authored_loaded_process())
                            );
                            if let Some((&id, state)) = s.person_factories.iter().next() {
                                assert!(
                                    state.off_lot
                                        && state.placement.is_none()
                                        && state.acceptance.is_none()
                                );
                                let position = s.vm.world.positions[&id];
                                assert_eq!([position.x, position.y, position.level], [-16, -16, 1]);
                                if state.visitor.as_ref().is_some_and(|v| !v.complete) {
                                    visitor_saves += 1;
                                    assert!(!state.finalized && state.wrapper_finished);
                                    assert!(s.advance_fixture_tick().is_err());
                                    let mut invalid = Snapshot::capture(&p, &s);
                                    invalid.state.person_factories.get_mut(&id).unwrap().off_lot =
                                        false;
                                    assert!(invalid.restore(&p).is_err());
                                }
                            }
                            let before = s.clone();
                            s = serde_json::from_slice::<Snapshot>(
                                &serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
                            )
                            .unwrap()
                            .restore(&p)
                            .unwrap();
                            assert_eq!(s, before);
                            if complete {
                                break;
                            }
                        }
                        assert!(
                            complete,
                            "neighbor={neighbor} autonomous={autonomous} visitor={visitor} budget={budget}; factories={:?}; threads={:?}",
                            s.person_factories, s.vm.threads
                        );
                        if neighbor {
                            let (&id, state) = s.person_factories.iter().next().unwrap();
                            assert!(state.finalized && state.factory.is_complete());
                            assert_eq!(state.visitor.is_some(), autonomous && visitor);
                            let object = &s.vm.world.objects[&id];
                            assert_eq!(object.temps[7], i16::from(autonomous && visitor));
                            assert_eq!(
                                object.temps[6],
                                if autonomous && visitor { 321 } else { 0 }
                            );
                            assert_eq!(s.vm.threads[&parent].stack[0].stack_object, id);
                            if budget == 1 && autonomous && visitor {
                                assert!(visitor_saves > 0);
                            }
                        } else {
                            assert!(s.person_factories.is_empty());
                            assert_eq!(s.vm.threads[&parent].status, Status::Returned(false));
                        }
                        assert_eq!(
                            s.vm.world.image_cache.entries[0].dirty,
                            [neighbor && flagged; 7]
                        );
                        assert_eq!(
                            s.vm.world.image_cache.refs,
                            [u32::from(!(neighbor && flagged)), 1]
                        );
                        assert_eq!(s.vm.world.image_cache.entries[1].dirty, [false; 7]);
                        assert_eq!(
                            s.vm.world.image_cache.ui,
                            Some(if neighbor && flagged {
                                sims_vm::image_cache::CHANGED
                            } else {
                                0
                            })
                        );
                        results.push(s);
                    }
                    assert_eq!(results[0], results[1]);
                }
            }
        }
    }
}

#[test]
fn rejected_person_cleanup_retains_committed_effects_and_false_wrapper_tail() {
    let mut results = vec![];
    for budget in [1, 1024] {
        let (mut p, mut s, parent) = fixture_mode(true, true, true, true, 5);
        let creator = p
            .modules
            .get_mut("objects")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap();
        creator.body.instructions[0].on_false = 2;
        creator
            .body
            .instructions
            .push(instruction(2, [0, 0, 0, 0, 0, 5, 8, 8], 254));
        let tick = s.vm.tick;
        let mut place =
            crate::lot_placement::live::Execution::new(crate::lot_placement::Operation::set(
                4,
                crate::lot_links::Descriptor {
                    on_lot: true,
                    position: [40, 40],
                    level: 1,
                    container: None,
                    slot: -1,
                },
                true,
            ));
        let mut step = Step {
            stop: StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        assert!(
            s.drive_live_placement(&p, &mut place, &mut 1024, &mut step)
                .unwrap()
        );
        let mut saved = 0;
        let mut rejected = None;
        let mut committed_rng = None;
        let mut committed_neighbor = None;
        for _ in 0..2000 {
            let Status::AwaitingCreation(r) = s.vm.threads[&parent].status.clone() else {
                panic!("creation owner not suspended")
            };
            let done = s
                .service_creation(&p, parent, r, &mut budget.clone(), &mut step)
                .unwrap();
            s.sync_spatial_world();
            assert_eq!(s.vm.tick, tick);
            assert_eq!(
                s.vm.world.autonomy_constants,
                Some(authored_loaded_process())
            );
            if let Some((&id, state)) = s
                .person_factories
                .iter()
                .find(|(_, s)| s.rejection_cleanup.is_some())
            {
                rejected = Some(id);
                saved += 1;
                assert!(state.wrapper_finished && state.factory.is_complete() && !state.finalized);
                assert!(
                    state
                        .acceptance
                        .as_ref()
                        .unwrap()
                        .outcome()
                        .is_some_and(|o| !o.accepted)
                );
                assert!(state.placement.is_none() && state.visitor.is_none());
                committed_rng.get_or_insert((
                    s.vm.rng_state,
                    s.vm.lot_rng_state,
                    s.vm.crt_rng_state,
                ));
                committed_neighbor
                    .get_or_insert(s.vm.world.person.as_ref().unwrap().neighbors[&25].clone());
                for variant in 0..5 {
                    let mut forged = Snapshot::capture(&p, &s);
                    let f = forged.state.person_factories.get_mut(&id).unwrap();
                    match variant {
                        0 => f.parent = Some(u64::MAX),
                        1 => f.request += 1,
                        2 => f.finalized = true,
                        3 => f.rejection_cleanup.as_mut().unwrap().target = 4,
                        _ => f.off_lot = true,
                    }
                    assert!(
                        forged.restore(&p).is_err(),
                        "forged cleanup variant{variant} accepted"
                    );
                }
            }
            let before = s.clone();
            s = serde_json::from_slice::<Snapshot>(
                &serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
            )
            .unwrap()
            .restore(&p)
            .unwrap();
            assert_eq!(s, before);
            if done {
                break;
            }
        }
        assert!(s.person_factories.is_empty());
        assert_eq!(s.vm.threads[&parent].status, Status::Ready);
        assert_eq!(s.vm.threads[&parent].stack[0].location.instruction, 2);
        assert_eq!(s.vm.threads[&parent].stack[0].stack_object, 25);
        assert!(s.vm.world.image_cache.entries[0].dirty.iter().all(|x| *x));
        assert!(s.deletion.is_empty() && s.retired_characters.is_empty());
        if budget == 1 {
            let id = rejected.expect("no actual rejected cleanup checkpoint");
            assert!(saved > 0 && !s.vm.world.objects.contains_key(&id));
            assert_eq!(
                Some((s.vm.rng_state, s.vm.lot_rng_state, s.vm.crt_rng_state)),
                committed_rng
            );
            assert_eq!(
                Some(s.vm.world.person.as_ref().unwrap().neighbors[&25].clone()),
                committed_neighbor
            );
        }
        results.push(s);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn pending_rejected_cleanup_prefix_leaves_unrelated_factories_unchanged() {
    let (p, mut s, parent) = fixture(true, true, true, false);
    for _ in 0..1000 {
        let Status::AwaitingCreation(r) = s.vm.threads[&parent].status.clone() else {
            panic!("factory owner absent")
        };
        let mut step = Step {
            stop: StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        let done = s
            .service_creation(&p, parent, r, &mut 1, &mut step)
            .unwrap();
        assert_eq!(
            s.vm.world.autonomy_constants,
            Some(authored_loaded_process())
        );
        let before = s.clone();
        let events = step.clone();
        assert!(
            s.drive_pending_rejected_person_cleanup(&p, &mut 1000, &mut step)
                .unwrap()
        );
        assert_eq!(s, before);
        assert_eq!(step, events);
        if done {
            assert!(s.person_factories.values().all(|f| f.finalized));
            return;
        }
    }
    panic!("unrelated person factory did not finish");
}
