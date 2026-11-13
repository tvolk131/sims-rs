//! Private-asset domestic acceptance; no motive top-ups or background cleanup.
use sims_content::{
    Index,
    domestic::{self, Layout},
};
use sims_sim::{Simulation, Snapshot, StepStop};
use sims_vm::Program;
use std::collections::BTreeSet;
fn setup(seed: u32, layout: Layout) -> (Program, Simulation) {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    domestic::home_with(&index, seed, layout).unwrap()
}
fn save(p: &Program, s: &Simulation) -> Simulation {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap(), p).unwrap_or_else(
        |e| {
            std::fs::write(
                format!("../../cache/domestic-save-failure-{}.json", s.vm.tick),
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!("restore {}: {e}", s.vm.tick)
        },
    )
}
fn step(p: &Program, s: &mut Simulation, budget: usize) -> Vec<sims_vm::Trace> {
    s.advance_fixture_tick().unwrap();
    let mut traces = vec![];
    for round in 0..20000 {
        let result = s.run_fixture_tick(p, budget).unwrap_or_else(|e| {
            std::fs::write(
                "../../cache/domestic-acceptance-failure.json",
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!("tick {}: {e}", s.vm.tick)
        });
        assert!(
            !matches!(result.stop, StepStop::Faulted | StepStop::Animation),
            "tick {}: {:?}",
            s.vm.tick,
            s.vm.threads
                .values()
                .filter(|t| matches!(t.status, sims_vm::Status::Faulted(_)))
                .collect::<Vec<_>>()
        );
        traces.extend(result.trace);
        if budget > 1 && round == 100 {
            std::fs::write(
                "../../cache/domestic-stalled-tick.json",
                serde_json::to_vec_pretty(&Snapshot::capture(p, s)).unwrap(),
            )
            .unwrap();
            panic!(
                "non-yielding tick {}; see cache/domestic-stalled-tick.json",
                s.vm.tick
            );
        }
        if result.stop != StepStop::Budget {
            s.vm.sounds.clear();
            s.vm.effects.clear();
            s.household.as_mut().unwrap().clock_events.clear();
            return traces;
        }
    }
    panic!("stalled tick {}", s.vm.tick)
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn a_week_in_two_kitchens_with_resume_cooking_sleeping_and_carrying() {
    for (seed, layout) in [(7, Layout::StraightKitchen), (31, Layout::CornerKitchen)] {
        if std::env::var("SIMS_TEST_SEED").is_ok_and(|v| v != seed.to_string()) {
            continue;
        }
        let (p, mut full) = setup(seed, layout);
        let mut clips = BTreeSet::new();
        let mut saved = BTreeSet::new();
        let mut deleted = 0;
        let mut previous = full.vm.world.objects.len();
        let mut replica: Option<(Simulation, u64)> = None;
        for tick in 1..=302400 {
            let trace = step(&p, &mut full, 4000);
            if let Some((s, until)) = &mut replica {
                assert_eq!(trace, step(&p, s, 1), "seed {seed} trace {tick}");
                if full != *s {
                    for (name, state) in [("full", &full), ("split", &*s)] {
                        std::fs::write(
                            format!("../../cache/domestic-diff-{name}.json"),
                            serde_json::to_vec_pretty(&Snapshot::capture(&p, state)).unwrap(),
                        )
                        .unwrap();
                    }
                    panic!("seed {seed} state {tick}; see cache/domestic-diff-*.json");
                }
                if tick >= *until {
                    replica = None;
                }
            }
            let count = full.vm.world.objects.len();
            deleted += previous.saturating_sub(count);
            previous = count;
            let mut checkpoint = None;
            for id in [1, 13] {
                if let Some(a) = full.animations.get(&id) {
                    clips.insert((id, a.clip.clone()));
                    for (name, pattern) in [
                        ("cooking", "stove"),
                        ("sleeping", "beds-sleep"),
                        ("washing", "wash"),
                        ("waiting", "wait"),
                    ] {
                        if a.clip.contains(pattern) && !saved.contains(name) {
                            checkpoint = Some(name);
                        }
                    }
                }
                if full.vm.world.objects[&id]
                    .slots
                    .first()
                    .is_some_and(Option::is_some)
                    && !saved.contains("carrying")
                {
                    checkpoint = Some("carrying");
                }
            }
            if replica.is_none() && (checkpoint.is_some() || tick % 43200 == 0) {
                let restored = save(&p, &full);

                assert_eq!(full, restored);
                replica = Some((restored, tick + 300));
                if let Some(name) = checkpoint {
                    saved.insert(name);
                }
            }
            if tick % 43200 == 0 {
                eprintln!(
                    "{layout:?} seed {seed}: day {}, live {count}, removed {deleted}, saves {saved:?}",
                    tick / 43200
                );
            }
        }
        assert_eq!(full.household.as_ref().unwrap().clock.elapsed_days, 7);
        for name in ["cooking", "sleeping", "carrying", "washing"] {
            assert!(
                saved.contains(name),
                "missing {name}, seed {seed}: {clips:?}"
            );
        }
        assert!(deleted > 10, "no recurring cleanup, seed {seed}");
        for id in [1, 13] {
            for clip in ["stove", "beds-sleep", "shower1-scrub", "book"] {
                assert!(
                    clips
                        .iter()
                        .any(|(actor, c)| *actor == id && c.contains(clip)),
                    "#{id} missing {clip}: {clips:?}"
                );
            }
        }
        std::fs::write(
            format!("../../cache/domestic-week-{seed}.json"),
            serde_json::to_vec_pretty(&Snapshot::capture(&p, &full)).unwrap(),
        )
        .unwrap();
    }
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn four_residents_share_doors_and_facilities_for_three_days() {
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    let (p, mut full) = domestic::four_residents(&index, 43).unwrap();
    let mut clips = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut replica: Option<(Simulation, u64)> = None;
    let mut removed = 0;
    let mut previous = full.vm.world.objects.len();
    for tick in 1..=129600 {
        let trace = step(&p, &mut full, 4000);
        if let Some((s, until)) = &mut replica {
            assert_eq!(trace, step(&p, s, 1), "four residents trace {tick}");
            assert_eq!(&full, s, "four residents state {tick}");
            if tick >= *until {
                replica = None;
            }
        }
        for (&id, a) in &full.animations {
            if [1, 13, 32, 33].contains(&id) {
                clips.insert((id, a.clip.clone()));
            }
        }
        removed += previous.saturating_sub(full.vm.world.objects.len());
        previous = full.vm.world.objects.len();
        let phase = full
            .route_control
            .values()
            .find_map(|c| c.callback.map(|c| c.name()));
        let save_now = phase.is_some_and(|p| !seen.contains(p)) || tick % 43200 == 0;
        if replica.is_none() && save_now {
            if let Some(p) = phase {
                seen.insert(p);
            }
            replica = Some((save(&p, &full), tick + 100));
        }
        if tick % 43200 == 0 {
            eprintln!(
                "four residents day {}, live {}, removed {removed}, callbacks {seen:?}",
                tick / 43200,
                previous
            );
        }
    }
    assert!(removed > 10);
    assert!(
        seen.contains("route ask to move animation") && seen.contains("route waiting animation")
    );
    for actor in [1, 13, 32, 33] {
        for activity in ["beds-sleep", "shower1-scrub", "book"] {
            assert!(
                clips
                    .iter()
                    .any(|(id, clip)| *id == actor && clip.contains(activity)),
                "#{actor} missing {activity}"
            );
        }
    }
    std::fs::write(
        "../../cache/domestic-four-three-days.json",
        serde_json::to_vec_pretty(&Snapshot::capture(&p, &full)).unwrap(),
    )
    .unwrap();
}
fn menu(p: &Program, s: &Simulation, target: i16, action: u16) -> usize {
    let d = s.vm.world.objects[&target].definition.as_ref().unwrap();
    p.modules[&d.source].object_interactions[&d.object]
        .iter()
        .position(|e| e.action == action)
        .unwrap()
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn blocked_door_tries_an_original_alternative_and_resumes_during_crossing() {
    let (mut p, mut full) = setup(7, Layout::StraightKitchen);
    let index = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    sims_content::household::add_object(
        &index,
        &mut p,
        &mut full,
        "Doors.iff",
        16815,
        32,
        sims_sim::Body {
            level: 1,
            position: sims_sim::Point::new(136, 120),
            facing: 64,
            radius: 6,
            elevation: 0,
        },
    )
    .unwrap();
    let mut walls = full.architecture.as_ref().unwrap().floors[0]
        .plan
        .walls
        .clone();
    walls
        .iter_mut()
        .find(|w| w.a == sims_sim::Point::new(144, 112))
        .unwrap()
        .door = Some([32, 33]);
    full.set_architecture(&p, walls).unwrap();
    full.vm.world.person.as_mut().unwrap().free_will = false;
    full.vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .motives
        .as_mut()
        .unwrap()[5] = (-30f32).to_bits();
    full.enqueue(&p, 1, 20, menu(&p, &full, 20, 0x100a))
        .unwrap();
    let mut saw_first = false;
    let mut saw_alternative = false;
    let mut sleeping = false;
    for _ in 0..2400 {
        step(&p, &mut full, 4000);
        if !saw_first
            && full
                .route_control
                .values()
                .any(|c| c.caller == 1 && c.last_portal == Some(16))
        {
            saw_first = true;
            for [x, y, z, w] in [
                [126, 136, 128, 168],
                [128, 136, 144, 138],
                [128, 166, 144, 168],
            ] {
                full.room.obstacles.push(sims_sim::Rect {
                    min: sims_sim::Point::new(x, y),
                    max: sims_sim::Point::new(z, w),
                });
            }
        }
        if !saw_alternative
            && full
                .route_control
                .values()
                .any(|c| c.caller == 1 && c.last_portal == Some(32))
        {
            saw_alternative = true;
            let mut a = save(&p, &full);
            let mut b = a.clone();
            for _ in 0..300 {
                assert_eq!(step(&p, &mut a, 4000), step(&p, &mut b, 1));
                assert_eq!(a, b, "alternative portal save/resume");
            }
        }
        sleeping |= full
            .animations
            .get(&1)
            .is_some_and(|a| a.clip.contains("beds-sleep"));
        if sleeping {
            break;
        }
    }
    assert!(
        saw_first && saw_alternative && sleeping,
        "first={saw_first}, alternative={saw_alternative}, sleeping={sleeping}"
    );
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn simultaneous_bed_routes_shoo_wait_save_and_cancel() {
    let (p, mut full) = setup(7, Layout::StraightKitchen);
    full.vm.world.person.as_mut().unwrap().free_will = false;
    for (actor, bed) in [(1, 20), (13, 23)] {
        full.vm
            .world
            .objects
            .get_mut(&actor)
            .unwrap()
            .motives
            .as_mut()
            .unwrap()[5] = (-30f32).to_bits();
        let m = menu(&p, &full, bed, 0x100a);
        full.enqueue(&p, actor, bed, m).unwrap();
    }
    let mut seen = BTreeSet::new();
    let mut sleepers = BTreeSet::new();
    for _ in 0..1800 {
        step(&p, &mut full, 4000);
        for state in full.route_control.values() {
            if let Some(callback) = state.callback {
                let name = callback.name();
                if seen.insert(name) {
                    let mut a = save(&p, &full);
                    let mut b = a.clone();
                    for _ in 0..300 {
                        assert_eq!(step(&p, &mut a, 4000), step(&p, &mut b, 1));
                        assert_eq!(a, b, "contention resumed state");
                    }
                    // Cancel the waiting resident, and compare the original
                    // cleanup with an instruction-at-a-time continuation.
                    let mut a = save(&p, &full);
                    let action = a.vm.world.actions.as_ref().unwrap().actors[&state.caller]
                        .active
                        .as_ref()
                        .unwrap()
                        .id;
                    a.cancel_interaction(&p, state.caller, u64::from(action))
                        .unwrap();
                    let mut b = save(&p, &a);
                    for _ in 0..300 {
                        assert_eq!(step(&p, &mut a, 4000), step(&p, &mut b, 1));
                        assert_eq!(a, b, "cancelled contention state");
                    }
                }
            }
        }
        for (id, animation) in &full.animations {
            if animation.clip.contains("beds-sleep") {
                sleepers.insert(*id);
            }
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from(["route ask to move animation", "route waiting animation"])
    );
    assert_eq!(sleepers, BTreeSet::from([1, 13]));
    assert!(full.route_control.values().all(|s| s.link.is_none()));
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn shoo_factory_and_deletion_restore_inside_the_lot_tick() {
    let (p, mut full) = setup(7, Layout::StraightKitchen);
    full.vm.world.person.as_mut().unwrap().free_will = false;
    for (actor, bed) in [(1, 20), (13, 23)] {
        full.vm
            .world
            .objects
            .get_mut(&actor)
            .unwrap()
            .motives
            .as_mut()
            .unwrap()[5] = (-30f32).to_bits();
        let m = menu(&p, &full, bed, 0x100a);
        full.enqueue(&p, actor, bed, m).unwrap();
    }
    let mut split = full.clone();
    let mut stages = BTreeSet::new();
    for tick in 0..3500 {
        let expected = step(&p, &mut full, 4000);
        split.advance_fixture_tick().unwrap();
        let mut trace = vec![];
        loop {
            let r = split.run_fixture_tick(&p, 1).unwrap();
            assert!(!matches!(r.stop, StepStop::Faulted | StepStop::Animation));
            trace.extend(r.trace);
            let stage = if split.shoo_creation.is_some() {
                Some("factory")
            } else if !split.deletion.is_empty() {
                Some("deletion")
            } else {
                None
            };
            if let Some(stage) = stage {
                stages.insert(stage);
                assert!(split.clone().advance_fixture_tick().is_err());
                split = save(&p, &split);
            }
            if r.stop != StepStop::Budget {
                break;
            }
        }
        split.vm.sounds.clear();
        split.vm.effects.clear();
        split.household.as_mut().unwrap().clock_events.clear();
        assert_eq!(expected, trace, "factory/deletion trace {tick}");
        assert_eq!(full, split, "factory/deletion state {tick}");
        if tick == 800 {
            let target = full
                .vm
                .world
                .objects
                .iter()
                .find_map(|(&id, o)| {
                    o.definition
                        .as_ref()
                        .is_some_and(|d| d.guid == 0x7c4)
                        .then_some(id)
                })
                .unwrap();
            // Two pending Move entries survive while both people sleep. When
            // the original temporary destination expires, deletion runs both
            // function-4 callbacks before freeing it, across budget boundaries.
            for s in [&mut full, &mut split] {
                for actor in [1, 13] {
                    assert!(
                        s.vm.enqueue_action(
                            &p,
                            actor,
                            sims_vm::action::Action {
                                target,
                                icon_owner: target,
                                menu_id: 3,
                                priority: 0,
                                routine: 4107,
                                ..Default::default()
                            }
                        )
                        .unwrap()
                        .1
                    );
                }
            }
        }
    }
    assert_eq!(stages, BTreeSet::from(["factory", "deletion"]));
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn cancellation_during_cooking_sleep_and_carry_keeps_original_cleanup_and_resumes() {
    let (p, mut discovery) = setup(7, Layout::StraightKitchen);
    let mut tested = BTreeSet::new();
    for _ in 0..18000 {
        step(&p, &mut discovery, 4000);
        let clip = discovery
            .animations
            .get(&1)
            .map(|a| a.clip.as_str())
            .unwrap_or("");
        let kind = if clip.contains("stove-cook") {
            Some("cooking")
        } else if clip.contains("beds-sleep") {
            Some("sleeping")
        } else if discovery.vm.world.objects[&1]
            .slots
            .first()
            .is_some_and(Option::is_some)
        {
            Some("carrying")
        } else {
            None
        };
        let Some(kind) = kind.filter(|k| !tested.contains(*k)) else {
            continue;
        };
        let Some(active) = discovery.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
        else {
            continue;
        };
        let cancelled = active.id;
        let mut full = save(&p, &discovery);
        full.vm.world.person.as_mut().unwrap().free_will = false;
        full.cancel_interaction(&p, 1, u64::from(cancelled))
            .unwrap();
        let read = menu(&p, &full, 10, 4110);
        full.enqueue(&p, 1, 10, read).unwrap();
        let mut split = save(&p, &full);
        let mut book = false;
        for tick in 0..7000 {
            assert_eq!(
                step(&p, &mut full, 4000),
                step(&p, &mut split, 1),
                "{kind} trace {tick}"
            );
            assert_eq!(full, split, "{kind} state {tick}");
            if tick % 311 == 0 {
                split = save(&p, &split);
            }
            book |= full
                .animations
                .get(&1)
                .is_some_and(|a| a.clip.contains("book"));
            if book {
                break;
            }
        }
        assert!(book, "{kind}: queued Read never started");
        assert!(
            full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .is_none_or(|a| a.id != cancelled)
        );
        tested.insert(kind);
        eprintln!("Original cancellation and budget-1 resume during {kind}");
        if tested.len() == 3 {
            break;
        }
    }
    assert_eq!(tested.len(), 3, "{tested:?}");
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_serving_feeds_both_residents_at_the_table_then_cleans_up() {
    let (p, mut s) = setup(7, Layout::StraightKitchen);
    s.vm.world.person.as_mut().unwrap().free_will = false;
    let serve = menu(&p, &s, 4, 4107);
    s.enqueue(&p, 1, 4, serve).unwrap();
    let hunger_before = f32::from_bits(s.vm.world.objects[&13].motives.unwrap()[7]);
    let mut peak_hunger = hunger_before;
    let mut clips = BTreeSet::new();
    let mut queued = BTreeSet::new();
    let mut dined = BTreeSet::new();
    let mut prepared = false;
    let mut cooked = false;
    let mut cleaned = false;
    for tick in 1..=16000 {
        step(&p, &mut s, 4000);
        peak_hunger = peak_hunger.max(f32::from_bits(s.vm.world.objects[&13].motives.unwrap()[7]));
        for id in [1, 13] {
            if let Some(a) = s.animations.get(&id) {
                clips.insert((id, a.clip.clone()));
                prepared |= a.clip.contains("counter-foodprep");
                cooked |= a.clip.contains("stove-cook");
                if a.clip.contains("sit-scoot-stabfood") {
                    dined.insert(id);
                }
                cleaned |= a.clip.contains("sink-washdishes");
            }
        }
        let actor = if queued.is_empty() { 1 } else { 13 };
        if cooked
            && ((queued.is_empty()
                && s.vm.world.actions.as_ref().unwrap().actors[&1]
                    .active
                    .is_none())
                || (queued.len() == 1 && dined.contains(&1)))
        {
            let choice =
                s.vm.world
                    .objects
                    .iter()
                    .filter(|(_, o)| {
                        o.definition
                            .as_ref()
                            .is_some_and(|d| d.source.ends_with("Food.iff"))
                    })
                    .find_map(|(&id, o)| {
                        let d = o.definition.as_ref().unwrap();
                        p.modules[&d.source].object_interactions[&d.object]
                            .iter()
                            .enumerate()
                            .filter(|(_, e)| e.label == "Eat")
                            .find_map(|(n, _)| {
                                s.interaction_choice(&p, actor, id, n)
                                    .ok()
                                    .filter(|c| c.available)
                                    .map(|_| (id, n))
                            })
                    });
            if let Some((id, n)) = choice {
                s.enqueue(&p, actor, id, n).unwrap();
                queued.insert(actor);
                eprintln!("Queued resident #{actor} meal at {tick} from #{id}");
            }
        }
        if tick % 503 == 0 {
            s = save(&p, &s);
        }
    }
    std::fs::write(
        "../../cache/domestic-serving-result.json",
        serde_json::to_vec_pretty(&Snapshot::capture(&p, &s)).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Original serving: both residents dined, hunger peak {peak_hunger}, dishwashing {cleaned}"
    );
    assert_eq!(queued.len(), 2, "no available original serving menu");
    assert_eq!(dined, BTreeSet::from([1, 13]), "{clips:?}");
    assert!(prepared && cooked && cleaned);
    assert!(peak_hunger > hunger_before + 10.);
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn sealed_passage_runs_original_feedback_with_save_and_player_cancellation() {
    let (p, mut full) = setup(7, Layout::StraightKitchen);
    full.vm.world.person.as_mut().unwrap().free_will = false;
    // Enclose every approach, so the native retry/shoo sequence must end in
    // feedback even now that a movable person can be asked to clear the way.
    let action = full.enqueue(&p, 1, 2, 0).unwrap();
    for _ in 0..300 {
        step(&p, &mut full, 4000);
        if full.routes.values().any(|r| r.caller == 1) {
            break;
        }
    }
    assert!(full.routes.values().any(|r| r.caller == 1));
    // Close the passage between the sitting area and dining table. Leave the
    // bookcase's occupied strip untouched; it completes the barrier itself.
    for (x0, x1) in [(32, 48), (64, 144)] {
        full.room.obstacles.push(sims_sim::Rect {
            min: sims_sim::Point::new(x0, 130),
            max: sims_sim::Point::new(x1, 131),
        });
    }
    let mut split = full.clone();
    let mut waiting = false;
    let mut cancelled = false;
    let mut clips = BTreeSet::new();
    for tick in 0..2500 {
        assert_eq!(
            step(&p, &mut full, 4000),
            step(&p, &mut split, 1),
            "blocked trace {tick}"
        );
        assert_eq!(full, split, "blocked state {tick}");
        if let Some(a) = full.animations.get(&1) {
            clips.insert(a.clip.clone());
        }
        if !full.route_feedback.is_empty()
            || full
                .animations
                .get(&1)
                .is_some_and(|a| a.clip.contains("routing-failure"))
        {
            waiting = true;
            split = save(&p, &split);
            if !cancelled {
                full.cancel_interaction(&p, 1, action).unwrap();
                split.cancel_interaction(&p, 1, action).unwrap();
                cancelled = true;
            }
        }
    }
    assert!(
        waiting && cancelled,
        "original route failure feedback was not observed: {clips:?}"
    );
    assert!(full.route_feedback.is_empty());
    assert!(
        full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
            .is_none_or(|a| u64::from(a.id) != action)
    );
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn new_person_obstruction_after_progress_replans_to_another_chair_approach() {
    let (p, mut full) = setup(7, Layout::StraightKitchen);
    full.vm.world.person.as_mut().unwrap().free_will = false;
    full.enqueue(&p, 1, 2, 0).unwrap();
    for _ in 0..300 {
        step(&p, &mut full, 4000);
        if full.routes.values().any(|r| r.caller == 1) {
            break;
        }
    }
    let route = full.routes.values().find(|r| r.caller == 1).unwrap();
    let blocked_goal = route.goal.position;
    full.bodies.get_mut(&13).unwrap().position = blocked_goal;
    full.set_architecture(
        &p,
        full.architecture.as_ref().unwrap().floors[0]
            .plan
            .walls
            .clone(),
    )
    .unwrap();
    let mut split = save(&p, &full);
    let mut replanned = false;
    for tick in 0..1600 {
        full.advance_fixture_tick().unwrap();
        let result = full.run_fixture_tick(&p, 4000).unwrap();
        assert!(!matches!(
            result.stop,
            StepStop::Faulted | StepStop::Budget | StepStop::Animation
        ));
        replanned |= result
            .events
            .iter()
            .any(|e| e.detail.contains("native route controller replans"));
        full.vm.sounds.clear();
        full.vm.effects.clear();
        full.household.as_mut().unwrap().clock_events.clear();
        assert_eq!(result.trace, step(&p, &mut split, 1), "replan trace {tick}");
        assert_eq!(full, split, "replan state {tick}");
        if tick % 79 == 0 {
            split = save(&p, &split);
        }
    }
    assert!(replanned);
    assert_eq!(full.vm.world.objects[&1].container, Some((2, 0)));
    assert_eq!(full.bodies[&13].position, blocked_goal);
    assert!(full.route_feedback.is_empty());
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn both_original_beds_restore_energy_and_complete_the_wake_routine() {
    let (p, mut s) = setup(7, Layout::StraightKitchen);
    s.vm.world.person.as_mut().unwrap().free_will = false;
    // Initial needs for this controlled sleep test; never replenished afterward.
    for actor in [1, 13] {
        s.vm.world
            .objects
            .get_mut(&actor)
            .unwrap()
            .motives
            .as_mut()
            .unwrap()[5] = (-30f32).to_bits();
    }
    // Isolate the bed's full sleep/wake cycle from doorway contention, which
    // has separate failure-feedback and route-replanning tests above.
    let sleep = menu(&p, &s, 23, 4106);
    s.enqueue(&p, 13, 23, sleep).unwrap();
    let mut second_queued = false;
    let mut slept = BTreeSet::new();
    let mut woke = BTreeSet::new();
    for tick in 1..=20000 {
        step(&p, &mut s, 4000);
        for actor in [1, 13] {
            if let Some(a) = s.animations.get(&actor) {
                if a.clip.contains("beds-sleep") {
                    slept.insert(actor);
                }
                if a.clip.contains("beds-getout") {
                    woke.insert(actor);
                }
            }
        }
        if slept.contains(&13) && !second_queued {
            let sleep = menu(&p, &s, 20, 4106);
            s.enqueue(&p, 1, 20, sleep).unwrap();
            second_queued = true;
        }
        if tick % 503 == 0 {
            s = save(&p, &s);
        }
        if woke.len() == 2
            && [1, 13].iter().all(|id| {
                s.vm.world.objects[id].container.is_none()
                    && s.vm.world.actions.as_ref().unwrap().actors[id]
                        .active
                        .is_none()
            })
        {
            for id in [1, 13] {
                assert!(f32::from_bits(s.vm.world.objects[&id].motives.unwrap()[5]) > 0.);
            }
            assert_eq!(slept.len(), 2);
            return;
        }
    }
    panic!("original bed sleep/wake did not finish: slept {slept:?}, woke {woke:?}");
}
