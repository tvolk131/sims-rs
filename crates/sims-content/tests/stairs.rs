//! Private original staircase scripts with deterministic continuation checks.
use sims_content::{Index, stairs};
use sims_sim::{Simulation, Snapshot, StepStop};
use sims_vm::Program;
fn setup() -> (Program, Simulation) {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    stairs::laboratory(&index).unwrap()
}
fn step(p: &Program, s: &mut Simulation, budget: usize) -> Vec<sims_vm::Trace> {
    s.advance_fixture_tick().unwrap();
    let mut trace = vec![];
    for _ in 0..100000 {
        let r = s.run_fixture_tick(p, budget).unwrap();
        assert!(
            !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
            "tick {}: {:?}",
            s.vm.tick,
            s.vm.threads
                .iter()
                .filter(|(_, t)| matches!(t.status, sims_vm::Status::Faulted(_)))
                .collect::<Vec<_>>()
        );
        trace.extend(r.trace);
        if r.stop != StepStop::Budget {
            return trace;
        }
    }
    panic!("tick did not yield")
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_stair_round_trip_restores_during_approach_climb_descent_and_departure() {
    let (p, mut full) = setup();
    full.enqueue(&p, 1, 2, 0).unwrap();
    let mut split = Snapshot::capture(&p, &full).restore(&p).unwrap();
    let mut clips = std::collections::BTreeSet::new();
    let mut downstairs = false;
    for _ in 0..2500 {
        let trace = step(&p, &mut full, 4000);
        assert_eq!(trace, step(&p, &mut split, 1));
        assert_eq!(full, split, "tick {}", full.vm.tick);
        if full.vm.tick % 17 == 0 {
            split = Snapshot::from_json(
                &serde_json::to_vec(&Snapshot::capture(&p, &full)).unwrap(),
                &p,
            )
            .unwrap();
        }
        if let Some(a) = full.animations.get(&1) {
            clips.insert(a.clip.clone());
        }
        if !downstairs
            && full.interactions[&1]
                .active
                .as_ref()
                .is_some_and(|a| a.phase == sims_sim::interaction::Phase::Seated)
        {
            for s in [&mut full, &mut split] {
                s.enqueue(&p, 1, 3, 0).unwrap();
            }
            downstairs = true;
        }
        if downstairs && full.vm.world.objects[&1].container == Some((3, 0)) {
            assert_eq!(full.bodies[&1].level, 1);
            assert!(clips.contains("a2o-stair-climb"));
            assert!(clips.contains("a2o-stair-descend"));
            return;
        }
    }
    panic!("round trip unfinished; clips {clips:?}")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn opposite_stair_entries_wait_release_and_restore_without_overlap() {
    let (p, mut s) = setup();
    sims_content::household::add_resident(
        &p,
        &mut s,
        13,
        2,
        sims_sim::Body {
            level: 2,
            position: sims_sim::Point::new(72, 40),
            facing: 16,
            radius: 3,
            elevation: 0,
        },
    )
    .unwrap();
    s.enqueue(&p, 1, 2, 0).unwrap();
    s.enqueue(&p, 13, 3, 0).unwrap();
    let mut waiting = false;
    let mut restored = None;
    for _ in 0..2000 {
        let trace = step(&p, &mut s, 4000);
        if let Some(copy) = &mut restored {
            assert_eq!(trace, step(&p, copy, 1));
            assert_eq!(s, *copy);
        }
        let wait = s.vm.threads.values().any(|t| {
            t.stack
                .iter()
                .any(|f| f.location.bhav == 4110 && f.location.source.ends_with("stair.iff"))
        });
        if wait && !waiting {
            waiting = true;
            restored = Some(
                Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p)
                    .unwrap(),
            );
        }
        let climbing = s
            .animations
            .values()
            .filter(|a| a.clip.contains("stair-") && !a.completed)
            .count();
        assert!(
            climbing <= 1,
            "opposite traversals overlap on the same stairs"
        );
        if s.vm.world.objects[&1].container == Some((2, 0))
            && s.vm.world.objects[&13].container == Some((3, 0))
        {
            assert!(waiting, "fixture did not exercise original portal wait");
            assert_eq!((s.bodies[&1].level, s.bodies[&13].level), (2, 1));
            assert_eq!(s.vm.world.objects[&53].attributes[0], 0);
            assert_eq!(s.vm.world.objects[&57].attributes[0], 0);
            return;
        }
    }
    panic!("opposite stair routes unfinished, waiting={waiting}")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn cancel_during_original_climb_completes_crossing_and_releases_stairs() {
    let (p, mut s) = setup();
    let action = s.enqueue(&p, 1, 2, 0).unwrap();
    let mut cancelled = false;
    let mut copy = None;
    for _ in 0..1000 {
        let trace = step(&p, &mut s, 4000);
        if let Some(c) = &mut copy {
            assert_eq!(trace, step(&p, c, 1));
            assert_eq!(s, *c);
        }
        if !cancelled
            && s.animations
                .get(&1)
                .is_some_and(|a| a.clip == "a2o-stair-climb" && a.frame_fifths > 150)
        {
            s.cancel_interaction(&p, 1, action).unwrap();
            copy = Some(
                Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p)
                    .unwrap(),
            );
            cancelled = true;
        }
        if cancelled && s.interactions[&1].active.is_none() {
            assert_eq!(s.bodies[&1].level, 2);
            assert!(s.vm.world.objects[&1].container.is_none());
            assert_eq!(s.vm.world.objects[&53].data[&8] & 32, 0);
            assert_eq!(s.vm.world.objects[&57].data[&8] & 32, 0);
            assert!(s.reservations.is_empty());
            return;
        }
    }
    panic!("cancelled climb did not clean up")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn blocked_upstairs_landing_tries_the_other_original_staircase() {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    let (p, mut s) = stairs::home(&index, 7).unwrap();
    s.vm.world.person.as_mut().unwrap().free_will = false;
    s.bodies.get_mut(&10).unwrap().position = sims_sim::Point::new(168, 104);
    s.bodies.get_mut(&1).unwrap().position = sims_sim::Point::new(168, 200);
    s.vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .motives
        .as_mut()
        .unwrap()[5] = (-30f32).to_bits();
    let def = s.vm.world.objects[&20].definition.as_ref().unwrap();
    let menu = p.modules[&def.source].object_interactions[&def.object]
        .iter()
        .position(|m| m.action == 4106)
        .unwrap();
    s.enqueue(&p, 1, 20, menu).unwrap();
    let mut first = false;
    let mut alternate = false;
    let mut restored = None;
    for _ in 0..4000 {
        let trace = step(&p, &mut s, 4000);
        if let Some(c) = &mut restored {
            assert_eq!(trace, step(&p, c, 1));
            assert_eq!(s, *c);
        }
        first |= s
            .route_control
            .values()
            .any(|r| r.caller == 1 && r.last_portal == Some(53));
        if !alternate
            && s.route_control
                .values()
                .any(|r| r.caller == 1 && r.last_portal == Some(71))
        {
            alternate = true;
            restored = Some(
                Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p)
                    .unwrap(),
            );
        }
        if s.animations
            .get(&1)
            .is_some_and(|a| a.clip.contains("beds-sleep"))
        {
            assert!(first && alternate, "first={first}, alternate={alternate}");
            assert_eq!(s.bodies[&1].level, 2);
            return;
        }
    }
    panic!(
        "blocked landing did not find the alternate staircase: first={first}, alternate={alternate}"
    )
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_cleanup_trash_accessory_crosses_floors_with_resume() {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    let (mut p, mut s) = stairs::home(&index, 7).unwrap();
    s.vm.world.person.as_mut().unwrap().free_will = false;
    s.bodies.get_mut(&1).unwrap().level = 2;
    s.bodies.get_mut(&1).unwrap().position = sims_sim::Point::new(88, 136);
    sims_content::household::add_object(
        &index,
        &mut p,
        &mut s,
        "trashpile.iff",
        16807,
        100,
        sims_sim::Body {
            level: 2,
            position: sims_sim::Point::new(104, 136),
            facing: 1,
            radius: 4,
            elevation: 0,
        },
    )
    .unwrap();
    while s.run_fixture_tick(&p, 4000).unwrap().stop == StepStop::Budget {}
    s.enqueue(&p, 1, 100, 0).unwrap();
    let mut saved = None;
    let mut carried = None;
    for _ in 0..3000 {
        let trace = step(&p, &mut s, 4000);
        if let Some(copy) = &mut saved {
            assert_eq!(trace, step(&p, copy, 1));
            assert_eq!(s, *copy);
        }
        if saved.is_none()
            && s.animations
                .get(&1)
                .is_some_and(|a| a.clip == "a2o-stair-descend" && !a.completed)
        {
            assert!(
                !s.vm.world.objects[&1].accessories.is_empty(),
                "trash carry accessory missing on stairs"
            );
            carried = Some(1);
            saved = Some(
                Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p)
                    .unwrap(),
            );
        }
        if carried.is_some() && !s.vm.world.objects.contains_key(&100) && s.bodies[&1].level == 1 {
            assert!(s.vm.world.objects[&1].slots[0].is_none());
            return;
        }
    }
    panic!("cleanup did not carry trash downstairs: {carried:?}")
}

#[test]
#[ignore = "requires private Complete Collection assets; three autonomous simulated days"]
fn three_autonomous_days_four_people_both_floors_and_original_domestic_routines() {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    let (p, mut s) = stairs::home(&index, 7).unwrap();
    let mut levels = std::collections::BTreeSet::new();
    let mut slept = std::collections::BTreeSet::new();
    let mut clips = std::collections::BTreeSet::new();
    let mut saves = 0;
    for tick in 1..=129600 {
        step(&p, &mut s, 4000);
        for id in [1, 13, 32, 33] {
            levels.insert((id, s.bodies[&id].level));
            if let Some(a) = s.animations.get(&id) {
                clips.insert(a.clip.clone());
                if a.clip.contains("beds-sleep") {
                    slept.insert(id);
                }
            }
            for child in s.vm.world.objects[&id].slots.iter().flatten() {
                assert_eq!(
                    s.bodies[child].level, s.bodies[&id].level,
                    "carried object floor at {tick}"
                );
            }
        }
        s.vm.sounds.clear();
        s.vm.effects.clear();
        s.household.as_mut().unwrap().clock_events.clear();
        if tick % 1000 == 0 {
            let snapshot = Snapshot::capture(&p, &s);
            if let Err(e) = snapshot.clone().restore(&p) {
                std::fs::write(
                    "../../cache/two-floor-save-failure.json",
                    serde_json::to_vec_pretty(&snapshot).unwrap(),
                )
                .unwrap();
                panic!("tick {tick}: {e}");
            }
            saves += 1;
        }
    }
    for id in [1, 13, 32, 33] {
        assert!(
            levels.contains(&(id, 1)) && levels.contains(&(id, 2)),
            "resident {id} did not use both floors"
        );
        assert!(slept.contains(&id), "resident {id} did not reach a bed");
    }
    for expected in [
        "stair-climb",
        "stair-descend",
        "stove-cook",
        "sink-washdishes",
        "shower",
        "toilet",
        "read",
    ] {
        assert!(
            clips.iter().any(|c| c.contains(expected)),
            "missing routine {expected}; clips {clips:?}"
        );
    }
    std::fs::write(
        "../../cache/two-floor-three-days.json",
        serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Four Sims completed three days on both floors, all four slept; {saves} snapshot restores; {} original clips",
        clips.len()
    );
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_landing_snap_shoos_person_and_resumes_inside_factory_and_retry() {
    let (p, mut s) = setup();
    sims_content::household::add_resident(
        &p,
        &mut s,
        13,
        2,
        sims_sim::Body {
            level: 2,
            position: sims_sim::Point::new(88, 40),
            facing: 16,
            radius: 3,
            elevation: 0,
        },
    )
    .unwrap();
    s.enqueue(&p, 1, 2, 0).unwrap();
    let mut copy: Option<Simulation> = None;
    let mut factory = false;
    let mut shoo = false;
    let mut waited = false;
    for _ in 0..1200 {
        s.advance_fixture_tick().unwrap();
        if let Some(c) = &mut copy {
            c.advance_fixture_tick().unwrap();
        }
        loop {
            let result = s.run_fixture_tick(&p, 1).unwrap();
            if let Some(c) = &mut copy {
                assert_eq!(result, c.run_fixture_tick(&p, 1).unwrap());
                assert_eq!(s, *c);
            }
            if !factory
                && s.shoo_creation.as_ref().is_some_and(|c| {
                    matches!(c.origin, sims_sim::contention::CreationOrigin::Snap { .. })
                })
            {
                factory = true;
                copy = Some(
                    Snapshot::from_json(
                        &serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
                        &p,
                    )
                    .unwrap(),
                );
            }
            assert!(
                !matches!(result.stop, StepStop::Faulted | StepStop::Animation),
                "tick {} {:?}",
                s.vm.tick,
                result
            );
            if result.stop != StepStop::Budget {
                break;
            }
        }
        shoo |= s.vm.world.actions.as_ref().unwrap().actors[&13]
            .active
            .as_ref()
            .is_some_and(|a| a.menu_id == 3);
        waited |= s.vm.threads.values().any(|t| {
            t.stack
                .iter()
                .any(|f| f.location.bhav == 4113 && f.location.source.ends_with("stair.iff"))
                && matches!(t.status, sims_vm::Status::Sleeping { .. })
        });
        if s.vm.world.objects[&1].container == Some((2, 0)) {
            assert!(
                factory && shoo && waited,
                "factory={factory}, shoo={shoo}, waited={waited}"
            );
            assert_ne!(s.bodies[&13].position, sims_sim::Point::new(88, 40));
            assert_eq!(s.bodies[&13].level, 2);
            return;
        }
    }
    panic!("blocked landing did not clear: factory={factory}, shoo={shoo}, waited={waited}")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_food_carry_descends_to_kitchen_and_resumes_with_child_on_same_floor() {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    let (p, mut s) = stairs::home(&index, 7).unwrap();
    s.vm.world.person.as_mut().unwrap().free_will = false;
    s.bodies.get_mut(&4).unwrap().level = 2;
    s.bodies.get_mut(&4).unwrap().position = sims_sim::Point::new(184, 88);
    s.bodies.get_mut(&1).unwrap().level = 2;
    s.bodies.get_mut(&1).unwrap().position = sims_sim::Point::new(200, 88);
    let d = s.vm.world.objects[&4].definition.as_ref().unwrap();
    let serve = p.modules[&d.source].object_interactions[&d.object]
        .iter()
        .position(|m| m.action == 4107)
        .unwrap();
    s.enqueue(&p, 1, 4, serve).unwrap();
    let mut restored = None;
    let mut carried = None;
    for _ in 0..6000 {
        let trace = step(&p, &mut s, 4000);
        if let Some(copy) = &mut restored {
            assert_eq!(trace, step(&p, copy, 1));
            assert_eq!(s, *copy);
        }
        if let Some(child) = s.vm.world.objects[&1].slots.first().copied().flatten() {
            assert_eq!(s.bodies[&child].level, s.bodies[&1].level);
            if restored.is_none()
                && s.animations
                    .get(&1)
                    .is_some_and(|a| a.clip == "a2o-stair-descend" && !a.completed)
            {
                carried = Some(child);
                restored = Some(
                    Snapshot::from_json(
                        &serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(),
                        &p,
                    )
                    .unwrap(),
                );
            }
        }
        if carried.is_some()
            && s.animations
                .get(&1)
                .is_some_and(|a| a.clip.contains("stove-cook"))
        {
            return;
        }
    }
    panic!("food did not reach the downstairs kitchen, carried={carried:?}")
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn cancelling_original_stair_wait_releases_waiter_without_crossing() {
    let (p, mut s) = setup();
    sims_content::household::add_resident(
        &p,
        &mut s,
        13,
        2,
        sims_sim::Body {
            level: 2,
            position: sims_sim::Point::new(72, 40),
            facing: 16,
            radius: 3,
            elevation: 0,
        },
    )
    .unwrap();
    s.enqueue(&p, 1, 2, 0).unwrap();
    let action = s.enqueue(&p, 13, 3, 0).unwrap();
    let mut cancelled = false;
    let mut saved = None;
    for _ in 0..1500 {
        let trace = step(&p, &mut s, 4000);
        if let Some(copy) = &mut saved {
            assert_eq!(trace, step(&p, copy, 1));
            assert_eq!(s, *copy);
        }
        let waiting = s.vm.threads.values().any(|t| {
            t.stack.first().is_some_and(|f| f.caller == 13)
                && t.stack
                    .iter()
                    .any(|f| f.location.bhav == 4110 && f.location.source.ends_with("stair.iff"))
        });
        if !cancelled && waiting {
            s.cancel_interaction(&p, 13, action).unwrap();
            cancelled = true;
            saved = Some(
                Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p)
                    .unwrap(),
            );
        }
        if cancelled
            && s.vm.world.objects[&1].container == Some((2, 0))
            && s.interactions[&13].active.is_none()
        {
            assert_eq!(s.bodies[&13].level, 2);
            assert!(s.vm.world.objects[&13].container.is_none());
            for id in [53, 57] {
                assert_eq!(s.vm.world.objects[&id].attributes[0], 0);
                assert_eq!(s.vm.world.objects[&id].data[&8] & 32, 0);
                assert!(!s.reservations.contains_key(&id));
            }
            return;
        }
    }
    panic!("wait cancellation did not settle: cancelled={cancelled}")
}
