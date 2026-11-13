//! Private-content acceptance: two original people, room portals and social scripts.
use sims_content::{Index, household};
use sims_sim::{Simulation, Snapshot, StepStop};
use sims_vm::Program;
use std::collections::BTreeSet;
fn setup() -> (Program, Simulation) {
    let i = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    household::two_sim_home(&i).unwrap()
}
fn menu(p: &Program, s: &Simulation, id: i16, bhav: u16) -> usize {
    let d = s.vm.world.objects[&id].definition.as_ref().unwrap();
    p.modules[&d.source].object_interactions[&d.object]
        .iter()
        .position(|e| e.action == bhav)
        .unwrap()
}
fn saved(p: &Program, s: &Simulation) -> Simulation {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap(), p).unwrap_or_else(
        |e| {
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache");
            std::fs::write(
                dir.join(format!("home-bad-save-{}.json", s.vm.tick)),
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!("restore tick {}: {e}", s.vm.tick)
        },
    )
}
fn step(
    p: &Program,
    s: &mut Simulation,
    budget: usize,
    check_callbacks: bool,
) -> Vec<sims_vm::Trace> {
    s.advance_fixture_tick().unwrap();
    let mut trace = vec![];
    let mut saved_callback = false;
    let mut slices = 0;
    loop {
        slices += 1;
        if slices > 10000 {
            std::fs::write(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../cache/home-stalled.json"),
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!("stalled tick {} budget {budget}", s.vm.tick);
        }
        let r = s.run_fixture_tick(p, budget).unwrap_or_else(|e| {
            std::fs::write(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../cache/home-service-failure.json"),
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!("tick {}: {e}", s.vm.tick)
        });
        trace.extend(r.trace);
        assert!(
            !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
            "tick {}: {:?}",
            s.vm.tick,
            s.vm.threads
                .values()
                .filter(|t| matches!(t.status, sims_vm::Status::Faulted(_)))
                .map(|t| &t.status)
                .collect::<Vec<_>>()
        );
        if check_callbacks
            && !saved_callback
            && (s.immediate.is_some() || s.cancellation.is_some() || s.reach_callback.is_some())
        {
            *s = saved(p, s);
            saved_callback = true;
        }
        if r.stop != StepStop::Budget {
            break;
        }
    }
    s.vm.sounds.clear();
    s.household.as_mut().unwrap().clock_events.clear();
    trace
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_talk_saves_during_coordination_and_changes_relationship_against_idle_control() {
    let (p, mut full) = setup();
    full.vm.world.person.as_mut().unwrap().free_will = false;
    let mut control = full.clone();
    let talk = menu(&p, &full, 13, 0x2051);
    full.enqueue(&p, 1, 13, talk).unwrap();
    let mut split = full.clone();
    let mut coordinated = false;
    let mut saved_social = false;
    for tick in 1..=7000 {
        step(&p, &mut control, 4000, false);
        assert_eq!(
            step(&p, &mut full, 4000, false),
            step(&p, &mut split, 1, true),
            "trace {tick}"
        );
        assert_eq!(full, split, "state {tick}");
        coordinated |= full.vm.world.objects.len() > 21;
        if tick % 137 == 0 {
            split = saved(&p, &split);
            saved_social |= full.vm.threads.values().any(|t| {
                t.stack
                    .iter()
                    .any(|f| f.code_owner.ends_with("HDSI_Talk.iff"))
            });
        }
    }
    assert!(coordinated && saved_social);
    let relation = |s: &Simulation, a, b| {
        s.vm.world.person.as_ref().unwrap().neighbors[&a].relationships[&b][0]
    };
    // Main initializes household acquaintances to 20. This seeded Talk chooses
    // a disliked topic: the listener's directional score falls by three.
    assert_eq!(
        (relation(&control, 1, 2), relation(&control, 2, 1)),
        (20, 20)
    );
    assert_eq!((relation(&full, 1, 2), relation(&full, 2, 1)), (17, 20));
    eprintln!(
        "Original coordinated Talk: relationship 17/20 versus idle control 20/20, identical budget-1 trace and save/resume"
    );
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn three_days_two_residents_shared_facilities_portals_and_resume() {
    let (p, mut full) = setup();
    let mut split = full.clone();
    let mut rooms = BTreeSet::new();
    let mut clips = BTreeSet::new();
    let mut portal_saved = false;
    let mut reach_saved = false;
    let mut carried_turn_saved = false;
    let mut highest = 0;
    for tick in 1..=129600 {
        assert_eq!(
            step(&p, &mut full, 4000, false),
            step(&p, &mut split, 1, true),
            "trace {tick}"
        );
        assert_eq!(full, split, "state {tick}");
        highest = highest.max(full.vm.world.objects.len());
        for id in [1, 13] {
            rooms.insert((id, full.vm.world.objects[&id].data[&29]));
            if let Some(a) = full.animations.get(&id) {
                clips.insert((id, a.clip.clone()));
            }
        }
        let portal = full.vm.threads.values().any(|t| {
            t.service.as_ref().is_some_and(|s| {
                s.chain()
                    .any(|s| [14, 15, 16, 17, 18, 19].contains(&s.callee))
            })
        });
        let reach = full
            .vm
            .threads
            .values()
            .any(|t| matches!(t.status, sims_vm::Status::AwaitingReach(_)));
        let carried_turn = full.turns.keys().any(|id| {
            full.vm.world.objects[id]
                .slots
                .first()
                .is_some_and(Option::is_some)
        });
        if tick % 997 == 0
            || (portal && !portal_saved)
            || (reach && !reach_saved)
            || (carried_turn && !carried_turn_saved)
        {
            split = saved(&p, &split);
            portal_saved |= portal;
            reach_saved |= reach;
            carried_turn_saved |= carried_turn;
            if reach || carried_turn {
                std::fs::write(
                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
                        "../../cache/home-{}-{tick}.json",
                        if reach { "reach" } else { "carried-turn" }
                    )),
                    serde_json::to_vec_pretty(&Snapshot::capture(&p, &full)).unwrap(),
                )
                .unwrap();
            }
        }
        if tick % 14400 == 0 {
            eprintln!(
                "two residents: tick {tick}, live objects {}",
                full.vm.world.objects.len()
            );
        }
    }
    assert_eq!(full.household.as_ref().unwrap().clock.elapsed_days, 3);
    assert!(portal_saved && reach_saved && carried_turn_saved);
    std::fs::write(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../cache/two-sim-home-three-days.json"),
        serde_json::to_vec_pretty(&Snapshot::capture(&p, &full)).unwrap(),
    )
    .unwrap();
    for id in [1, 13] {
        assert!(
            rooms.contains(&(id, 1)) && rooms.contains(&(id, 2)) && rooms.contains(&(id, 3)),
            "missing room visits {rooms:?}"
        );
        for part in ["shower1-scrub", "book", "nap"] {
            assert!(
                clips
                    .iter()
                    .any(|(actor, c)| *actor == id && c.contains(part)),
                "missing {id} {part}: {clips:?}"
            );
        }
    }
    eprintln!(
        "Three days: {} actor/clip pairs, peak {highest} objects, room visits {rooms:?}",
        clips.len()
    );
    // Queue a bathroom action for each resident, remove each pending action, and
    // verify commands remain local to that resident and survive serialization.
    for s in [&mut full, &mut split] {
        s.vm.world.person.as_mut().unwrap().free_will = false;
        for id in [1, 13] {
            let n = menu(&p, s, 9, 4098);
            let action = s.enqueue(&p, id, 9, n).unwrap();
            s.cancel_interaction(&p, id, action).unwrap();
        }
    }
    split = saved(&p, &split);
    for _ in 0..200 {
        assert_eq!(
            step(&p, &mut full, 4000, false),
            step(&p, &mut split, 1, true)
        );
        assert_eq!(full, split);
    }
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn cancellation_and_resume_during_a_portal_route_and_coordinated_talk() {
    for social in [false, true] {
        let (p, mut full) = setup();
        full.vm.world.person.as_mut().unwrap().free_will = false;
        let (target, bhav) = if social { (13, 0x2051) } else { (9, 4098) };
        let entry = menu(&p, &full, target, bhav);
        full.enqueue(&p, 1, target, entry).unwrap();
        let mut reached = false;
        for _ in 0..1500 {
            step(&p, &mut full, 4000, false);
            let waiting = if social {
                full.vm
                    .threads
                    .values()
                    .filter(|t| t.stack.first().is_some_and(|f| [1, 13].contains(&f.caller)))
                    .filter(|t| {
                        t.stack
                            .iter()
                            .any(|f| f.code_owner.ends_with("HDSI_Talk.iff"))
                    })
                    .count()
                    == 2
                    && full.vm.tick >= 150
            } else {
                full.routes.values().any(|r| r.caller == 1)
                    && full.vm.threads.values().any(|t| {
                        t.service
                            .as_ref()
                            .is_some_and(|s| s.chain().any(|s| [14, 15].contains(&s.callee)))
                    })
            };
            if waiting {
                reached = true;
                break;
            }
        }
        assert!(
            reached,
            "did not reach cancellation boundary social={social}"
        );
        let mut split = saved(&p, &full);
        let id = full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
            .unwrap()
            .id;
        for s in [&mut full, &mut split] {
            s.cancel_interaction(&p, 1, id.into()).unwrap();
        }
        split = saved(&p, &split);
        for tick in 0..4000 {
            assert_eq!(
                step(&p, &mut full, 4000, false),
                step(&p, &mut split, 1, true)
            );
            assert_eq!(full, split);
            if tick % 251 == 0 {
                split = saved(&p, &split);
            }
        }
        assert!(
            full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .is_none_or(|a| a.id != id),
            "cancelled action still active"
        );
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn home_snapshot_rejects_room_cache_and_family_corruption() {
    let (p, s) = setup();
    assert_eq!(saved(&p, &s), s);
    for change in 0..4 {
        let mut bad = Snapshot::capture(&p, &s);
        match change {
            0 => bad.state.vm.world.positions.get_mut(&1).unwrap().x += 16,
            1 => {
                bad.state
                    .vm
                    .world
                    .objects
                    .get_mut(&1)
                    .unwrap()
                    .data
                    .insert(29, 99);
            }
            2 => bad
                .state
                .vm
                .world
                .person
                .as_mut()
                .unwrap()
                .families
                .get_mut(&1)
                .unwrap()
                .push(1),
            _ => {
                bad.state.architecture.as_mut().unwrap().floors[0].rooms[0] = 99;
            }
        }
        assert!(bad.restore(&p).is_err(), "accepted mutation {change}");
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn reach_real_clip_preserves_all_phases_through_save_resume() {
    use sims_formats::behavior::{Behavior, Instruction};
    use sims_vm::{Routine, Start, Vm, World};
    use std::collections::BTreeMap;
    let (mut p, home) = setup();
    assert!(!p.modules[&p.globals].routines.contains_key(&900));
    p.modules
        .get_mut(&p.globals.clone())
        .unwrap()
        .routines
        .insert(
            900,
            Routine {
                name: "synthetic Reach service integration".into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 4,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode: 47,
                        on_true: 254,
                        on_false: 255,
                        operands: [2, 0, 0, 0, 0, 0, 0, 0],
                    }],
                },
            },
        );
    let mut vm = Vm::new(
        World {
            objects: BTreeMap::from([(1, home.vm.world.objects[&1].clone())]),
            ..Default::default()
        },
        1,
    );
    let id = vm
        .spawn(
            &p,
            Start {
                owner: &p.globals,
                bhav: 900,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![0; 4],
                check: false,
            },
        )
        .unwrap();
    let mut full = Simulation::new(
        vm,
        sims_sim::Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        BTreeMap::from([(
            1,
            sims_sim::Body {
                level: 1,
                position: sims_sim::Point::new(32, 32),
                facing: 1,
                radius: 0,
                elevation: 0,
            },
        )]),
    )
    .unwrap();
    let mut split = full.clone();
    let mut phases = BTreeSet::new();
    for tick in 0..300 {
        for (s, budget) in [(&mut full, 4000), (&mut split, 1)] {
            if tick > 0 {
                s.advance_fixture_tick().unwrap();
            }
            loop {
                let r = s.run_fixture_tick(&p, budget).unwrap();
                assert!(!matches!(r.stop, StepStop::Faulted | StepStop::Animation));
                if budget == 1 {
                    *s = saved(&p, s);
                }
                if r.stop != StepStop::Budget {
                    break;
                }
            }
        }
        assert_eq!(full, split);
        if matches!(
            full.vm.threads[&id].status,
            sims_vm::Status::AwaitingReach(_)
        ) {
            phases.insert(full.vm.threads[&id].stack.last().unwrap().continuation);
            let mut bad = Snapshot::capture(&p, &full);
            bad.state.animations.get_mut(&1).unwrap().pickup = false;
            assert!(bad.restore(&p).is_err());
        }
        if full.vm.threads[&id].status == sims_vm::Status::Returned(true) {
            assert!(phases.contains(&1) && phases.contains(&2));
            return;
        }
    }
    panic!("Reach did not complete");
}
