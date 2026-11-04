use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, load_chair_animations, load_program, two_chair_room},
};
use sims_sim::{Simulation, Snapshot, Step, StepStop, interaction::Phase};
use sims_vm::Program;

fn setup(facing: u8) -> (Program, Simulation) {
    setup_with_init(facing, false)
}
fn setup_with_init(facing: u8, init: bool) -> (Program, Simulation) {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut p).unwrap();
    let fixture =
        ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
    let mut s = if init {
        sims_content::vm::initialized_two_chair_room(&p, fixture, facing)
    } else {
        two_chair_room(&p, fixture, facing)
    }
    .unwrap();
    // Main first yields in the original startup Sleep; commands run on its next tick.
    s.advance_fixture_tick().unwrap();
    (p, s)
}
fn run(p: &Program, s: &mut Simulation) -> Step {
    let step = s.run_fixture_tick(p, 1000).unwrap();
    assert!(
        !matches!(
            step.stop,
            StepStop::Budget | StepStop::Faulted | StepStop::Animation
        ),
        "{:?}",
        s.vm.threads
    );
    step
}
fn tick(p: &Program, s: &mut Simulation) -> Step {
    s.advance_fixture_tick().unwrap_or_else(|e| {
        panic!(
            "{e}: tick {} statuses {:?}",
            s.vm.tick,
            s.vm.threads
                .iter()
                .map(|(id, t)| (id, &t.status))
                .collect::<Vec<_>>()
        )
    });
    run(p, s)
}
fn saved(p: &Program, s: &Simulation) -> Simulation {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, s)).unwrap(), p).unwrap()
}
fn seated(s: &Simulation, target: i16) -> bool {
    s.interactions[&1]
        .active
        .as_ref()
        .is_some_and(|a| a.phase == Phase::Seated && a.item.target == target)
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn household_clock_and_person_updates_survive_budget_splits_and_resume() {
    let (p, mut wide) = setup(1);
    wide.vm.world.budget = Some(sims_vm::budget::State {
        balance: 19985,
        expenses: [0, 0, 0, 15, 0, 0, 0, 0, 8],
        notifications: vec![19985],
    });
    wide.vm.world.globals.extend([(13, 9985), (14, 1), (32, 0)]);
    wide.enable_household(
        &p,
        sims_sim::clock::LotClock {
            hour: 23,
            minute: 59,
            second: 58,
            day: 30,
            month: 12,
            year: 1997,
            period: 2,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .unwrap();
    wide.enqueue(&p, 1, 2, 0).unwrap();
    let mut narrow = saved(&p, &wide);
    let initial_hunger = wide.vm.world.objects[&1].motives.unwrap()[7];
    for tick_index in 0..160 {
        let step = run(&p, &mut wide);
        let mut trace = vec![];
        loop {
            let one = narrow.run_fixture_tick(&p, 1).unwrap();
            trace.extend(one.trace);
            assert!(!matches!(one.stop, StepStop::Faulted | StepStop::Animation));
            if tick_index == 59 && one.stop == StepStop::Budget {
                narrow = saved(&p, &narrow);
            }
            if one.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(step.trace, trace, "tick {tick_index}");
        assert_eq!(wide, narrow, "tick {tick_index}");
        // Calling the completed tick again cannot apply another motive update.
        let motives = wide.vm.world.objects[&1].motives;
        let counter = wide.vm.world.objects[&1].person.as_ref().unwrap()[&27];
        run(&p, &mut wide);
        assert_eq!(wide.vm.world.objects[&1].motives, motives);
        assert_eq!(
            wide.vm.world.objects[&1].person.as_ref().unwrap()[&27],
            counter
        );
        if tick_index % 53 == 0 {
            narrow = saved(&p, &narrow);
        }
        wide.advance_fixture_tick().unwrap();
        narrow.advance_fixture_tick().unwrap();
    }
    assert_ne!(
        wide.vm.world.objects[&1].motives.unwrap()[7],
        initial_hunger
    );
    let c = &wide.household.as_ref().unwrap().clock;
    assert_eq!(c.statistics[0][3], 15);
    assert_eq!(
        wide.vm.world.budget.as_ref().unwrap().expenses,
        [0, 0, 0, 0, 0, 0, 0, 0, 8]
    );
    assert_eq!(
        (c.hour, c.minute, c.second, c.day, c.month, c.year),
        (0, 5, 18, 1, 1, 1998)
    );
    assert_eq!(wide.vm.world.globals[&0], c.hour);
    assert_eq!(wide.vm.world.globals[&8], c.year);
    // The original interaction is still running, with the actor physically seated.
    assert_eq!(wide.vm.world.objects[&1].container, Some((2, 0)));
}

fn front_of(chair: &sims_sim::Body) -> sims_sim::Point {
    let (x, y) = match chair.facing {
        1 => (0, -16),
        4 => (16, 0),
        16 => (0, 16),
        64 => (-16, 0),
        _ => unreachable!(),
    };
    sims_sim::Point::new(chair.position.x + x, chair.position.y + y)
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn sitting_prefers_a_reachable_front_for_both_chairs_in_every_orientation() {
    for facing in [1, 4, 16, 64] {
        let (p, initial) = setup(facing);
        for target in [2, 3] {
            let mut s = initial.clone();
            s.enqueue(&p, 1, target, 0).unwrap();
            let first_step = run(&p, &mut s);
            assert!(!s.routes.is_empty(), "native room trace: {first_step:#?}");
            let goal = s.routes.values().next().unwrap().goal;
            assert_eq!(goal.entry_flags, 1, "chair {target}, facing {facing}");
            assert_eq!(goal.position, front_of(&s.bodies[&target]));
            let mut replay = saved(&p, &s);
            let mut saw_front_animation = false;
            while !seated(&s, target) {
                assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
                assert_eq!(s, replay);
                if s.vm.tick == 20 {
                    replay = saved(&p, &s);
                }
                if let Some(animation) = s.animations.get(&1).filter(|a| !a.looping) {
                    assert_eq!(animation.clip, "a2o-sit-front");
                    if !saw_front_animation {
                        replay = saved(&p, &s);
                        saw_front_animation = true;
                    }
                }
                assert!(s.vm.tick < 500);
            }
            assert!(saw_front_animation);
        }
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn sitting_falls_back_if_the_front_is_occupied_or_unreachable() {
    let (p, initial) = setup(1);
    for case in 0..3 {
        let mut s = initial.clone();
        let rect = |x1, y1, x2, y2| sims_sim::Rect {
            min: sims_sim::Point::new(x1, y1),
            max: sims_sim::Point::new(x2, y2),
        };
        if case == 1 {
            // Enclose the front destination without occupying it: placement is
            // valid, but no route can reach it from outside the little enclosure.
            s.room.obstacles.extend([
                rect(78, 30, 98, 32),
                rect(78, 48, 98, 50),
                rect(78, 32, 80, 48),
                rect(96, 32, 98, 48),
            ]);
            let front = s.bodies[&1].bounds_at(front_of(&s.bodies[&2]));
            assert!(s.room.contains(front));
            assert!(s.room.obstacles.iter().all(|r| !r.overlaps(front)));
        } else {
            s.room.obstacles.push(rect(84, 36, 92, 44));
            if case == 2 {
                for (x, y) in [(104, 56), (88, 72), (72, 56)] {
                    s.room.obstacles.push(rect(x - 4, y - 4, x + 4, y + 4));
                }
            }
        }
        s.enqueue(&p, 1, 2, 0).unwrap();
        run(&p, &mut s);
        if case == 2 {
            // Failure feedback is an original yielding behavior, so the action
            // remains active until its animation and cleanup finish.
            let mut saved_feedback = false;
            while s.interactions[&1].active.is_some() {
                tick(&p, &mut s);
                if !saved_feedback {
                    s = saved(&p, &s);
                    saved_feedback = true;
                }
                assert!(s.vm.tick < 1000, "blocked chair feedback did not finish");
            }
            assert!(s.interactions[&1].active.is_none());
            assert!(s.routes.is_empty());
            assert!(s.reservations.is_empty());
            assert!(s.vm.world.objects[&1].container.is_none());
            saved(&p, &s);
            continue;
        }
        let entry = s.routes.values().next().unwrap().goal.entry_flags;
        assert_ne!(entry, 1);
        let clip = match entry {
            4 => "a2o-sit-right",
            16 => "a2o-sit-behind",
            64 => "a2o-sit-left",
            _ => unreachable!(),
        };
        let mut replay = saved(&p, &s);
        let mut saw_animation = false;
        while !seated(&s, 2) {
            assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
            assert_eq!(s, replay);
            if let Some(animation) = s.animations.get(&1).filter(|a| !a.looping) {
                assert_eq!(animation.clip, clip);
                saw_animation = true;
            }
            assert!(s.vm.tick < 500);
        }
        assert!(saw_animation);
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_two_chair_queue_repeats_and_resumes_through_each_phase() {
    let (p, mut s) = setup(1);
    let before = s.clone();
    let menu = s.interaction_menu(&p, 1, 2).unwrap();
    assert_eq!(menu[0].label, "Sit");
    assert!(menu[0].available);
    assert_eq!(s, before, "opening menus must not change simulation or RNG");
    s.enqueue(&p, 1, 2, 0).unwrap();
    s.enqueue(&p, 1, 3, 0).unwrap();
    let final_id = s.enqueue(&p, 1, 2, 0).unwrap();
    let mut replay = saved(&p, &s);
    assert_eq!(run(&p, &mut s), run(&p, &mut replay));
    let mut phases = std::collections::BTreeSet::new();
    let mut stand_calls = 0;
    for _ in 0..1500 {
        let a = tick(&p, &mut s);
        let b = tick(&p, &mut replay);
        assert_eq!(a, b);
        assert_eq!(s, replay);
        stand_calls += a
            .trace
            .iter()
            .filter(|t| t.location.bhav == 4102 && t.location.instruction == 0)
            .count();
        if let Some(active) = &s.interactions[&1].active {
            let phase = format!(
                "{:?}-{}-{}",
                active.phase,
                s.routes.is_empty(),
                s.animations.get(&1).is_some_and(|a| a.looping)
            );
            if phases.insert(phase) {
                replay = saved(&p, &s);
            }
        }
        if seated(&s, 2) && s.interactions[&1].active.as_ref().unwrap().item.id == final_id {
            break;
        }
        assert!(s.vm.tick < 1500);
    }
    assert!(seated(&s, 2));
    assert_eq!(stand_calls, 2);
    assert_eq!(s.vm.world.objects[&3].slots, [None]);
    assert!(s.reservations.is_empty());
    assert!(s.vm.threads.len() <= 4); // two persistent chair Main threads
    s.cancel_interaction(&p, 1, final_id).unwrap();
    replay = saved(&p, &s);
    assert_eq!(run(&p, &mut s), run(&p, &mut replay));
    for _ in 0..200 {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
        if s.interactions[&1].active.is_none() {
            break;
        }
    }
    assert!(s.interactions[&1].active.is_none());
    assert_eq!(s.vm.world.objects[&1].container, None);
    assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&0], 0);
    assert!(
        s.animations
            .values()
            .all(|a| a.clip.starts_with("a2o-idle") || a.looping)
    );
    assert!(s.routes.is_empty());
    assert!(s.reservations.is_empty());
    saved(&p, &s);
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn cancellation_before_start_during_route_and_during_sit_is_recoverable() {
    let (p, initial) = setup(1);
    for cancel_at in [0, 20, 150] {
        let mut s = initial.clone();
        let first = s.enqueue(&p, 1, 2, 0).unwrap();
        let removed = s.enqueue(&p, 1, 2, 0).unwrap();
        s.cancel_interaction(&p, 1, removed).unwrap();
        let last = s.enqueue(&p, 1, 3, 0).unwrap();
        if cancel_at > 0 {
            run(&p, &mut s);
            while s.vm.tick < cancel_at {
                tick(&p, &mut s);
            }
        }
        s.cancel_interaction(&p, 1, first).unwrap();
        let mut replay = saved(&p, &s);
        assert_eq!(run(&p, &mut s), run(&p, &mut replay));
        for _ in 0..900 {
            assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
            assert_eq!(s, replay);
            if seated(&s, 3) {
                break;
            }
        }
        assert!(seated(&s, 3), "cancel at {cancel_at}");
        assert_eq!(s.interactions[&1].active.as_ref().unwrap().item.id, last);
        assert_eq!(s.vm.world.objects[&2].slots, [None]);
        assert!(s.reservations.is_empty());
        saved(&p, &s);
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn standing_prefers_the_clear_front_of_either_chair_regardless_of_saved_entry_flags() {
    for facing in [1, 4, 16, 64] {
        let (p, initial) = setup(facing);
        for target in [2, 3] {
            let mut s = initial.clone();
            let id = s.enqueue(&p, 1, target, 0).unwrap();
            run(&p, &mut s);
            while !seated(&s, target) {
                tick(&p, &mut s);
                assert!(s.vm.tick < 500);
            }
            // Older saves and blocked-front approaches can retain any entry side.
            *s.vm
                .world
                .objects
                .get_mut(&1)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .get_mut(&37)
                .unwrap() = i16::from(facing);
            let chair = s.bodies[&target].clone();
            let front = front_of(&chair);
            s.cancel_interaction(&p, 1, id).unwrap();
            let mut replay = saved(&p, &s);
            assert_eq!(run(&p, &mut s), run(&p, &mut replay));
            assert_eq!(
                s.animations[&1].clip, "a2o-stand-front",
                "chair {target}, facing {facing}"
            );
            assert_eq!(s.bodies[&1].position, front);
            assert_eq!(s.bodies[&1].facing, chair.facing, "face away from the seat");
            let start = s.vm.tick;
            while s.interactions[&1].active.is_some() {
                assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
                assert_eq!(s, replay);
                if s.vm.tick == start + 20 {
                    replay = saved(&p, &s);
                }
                assert!(s.vm.tick < start + 200);
            }
            assert_eq!(s.bodies[&1].position, front);
            assert_eq!(s.bodies[&1].facing, chair.facing);
            assert_eq!(s.vm.world.objects[&1].container, None);
        }
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_exit_tries_other_sides_and_a_fully_blocked_exit_can_retry() {
    let (p, mut s) = setup(1);
    let id = s.enqueue(&p, 1, 2, 0).unwrap();
    run(&p, &mut s);
    while !seated(&s, 2) {
        tick(&p, &mut s);
        assert!(s.vm.tick < 400);
    }
    let seated_state = s.clone();
    // Block every stand SLOT without overlapping the chair's own footprint.
    for (x, y) in [(88, 40), (104, 56), (88, 72), (72, 56)] {
        s.room.obstacles.push(sims_sim::Rect {
            min: sims_sim::Point::new(x - 4, y - 4),
            max: sims_sim::Point::new(x + 4, y + 4),
        });
    }
    // Explicit Stand retains the room command's retry policy. Destination-driven
    // departure now belongs to the route primitive and has native failure semantics.
    s.cancel_interaction(&p, 1, id).unwrap();
    run(&p, &mut s);
    assert!(s.interactions[&1].active.as_ref().unwrap().exit_blocked);
    assert_eq!(s.vm.world.objects[&1].container, Some((2, 0)));
    assert_eq!(s.vm.world.objects[&2].slots, [Some(1)]);
    assert!(s.reservations.is_empty());
    let mut replay = saved(&p, &s);
    for _ in 0..5 {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
    }
    // Leave the front blocked: the original tree must choose an alternate side.
    s.room
        .obstacles
        .truncate(seated_state.room.obstacles.len() + 1);
    s.cancel_interaction(&p, 1, id).unwrap();
    let step = run(&p, &mut s);
    assert!(step.trace.iter().any(|t| t.location.bhav == 4106));
    assert_ne!(s.bodies[&1].position, sims_sim::Point::new(88, 40));
    assert!(s.vm.world.objects[&1].container.is_none());
    s.enqueue(&p, 1, 3, 0).unwrap();
    let mut replay = saved(&p, &s);
    // Cancellation during an exit must not strand a half-standing Sim.
    s.cancel_interaction(&p, 1, id).unwrap();
    replay.cancel_interaction(&p, 1, id).unwrap();
    for _ in 0..700 {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
        if seated(&s, 3) {
            break;
        }
    }
    assert!(seated(&s, 3));
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn destination_route_suspends_for_original_stand_and_preserves_its_parent_request() {
    let (p, mut s) = setup(1);
    s.enqueue(&p, 1, 2, 0).unwrap();
    run(&p, &mut s);
    while !seated(&s, 2) {
        tick(&p, &mut s);
        assert!(s.vm.tick < 400);
    }
    s.enqueue(&p, 1, 3, 0).unwrap();
    let thread = s.interactions[&1].main.unwrap();
    loop {
        let step = s.run_fixture_tick(&p, 1).unwrap();
        if let Some(call) = &s.vm.threads[&thread].service {
            let sims_vm::Status::AwaitingRoute(request) = &call.resume_status else {
                panic!("route must own Stand");
            };
            assert_eq!(request.target, 3);
            assert_eq!(call.callee, 2);
            assert_eq!(call.bhav, 4102);
            let child = &s.vm.threads[&thread].stack[call.base_depth];
            // Chair 3 is south of Chair 2. Relative back is the closest clear exit.
            assert_eq!(child.args, [16]); // Original Stand declares one parameter.
            assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&37], 16);
            break;
        }
        if step.stop != StepStop::Budget {
            s.advance_fixture_tick().unwrap();
        }
        assert!(s.vm.tick < 500);
    }
    let mut replay = saved(&p, &s);
    let mut bad = Snapshot::capture(&p, &s);
    let call = bad
        .state
        .vm
        .threads
        .get_mut(&thread)
        .unwrap()
        .service
        .as_mut()
        .unwrap();
    let sims_vm::Status::AwaitingRoute(request) = &mut call.resume_status else {
        unreachable!()
    };
    request.target = 2;
    assert!(bad.restore(&p).is_err());
    // Budget pauses save the original pending route while the child Stand runs.
    for _ in 0..1000 {
        let a = s.run_fixture_tick(&p, 1).unwrap();
        let b = replay.run_fixture_tick(&p, 1).unwrap();
        assert_eq!(a, b);
        assert_eq!(s, replay);
        replay = saved(&p, &replay);
        if s.vm.threads[&thread].service.is_none() {
            break;
        }
        if a.stop != StepStop::Budget {
            s.advance_fixture_tick().unwrap();
            replay.advance_fixture_tick().unwrap();
        }
    }
    assert!(s.vm.threads[&thread].service.is_none());
    assert_eq!(run(&p, &mut s), run(&p, &mut replay));
    while !seated(&s, 3) {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
        assert!(s.vm.tick < 1000);
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn interaction_snapshots_reject_corrupt_queue_ids_and_thread_bindings() {
    let (p, mut s) = setup(1);
    s.enqueue(&p, 1, 2, 0).unwrap();
    s.enqueue(&p, 1, 3, 0).unwrap();
    run(&p, &mut s);
    let snapshot = Snapshot::capture(&p, &s);
    let mut bad = snapshot.clone();
    bad.state
        .vm
        .world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap()
        .queue
        .pending[0]
        .id = 1;
    assert!(bad.restore(&p).is_err());
    let mut bad = snapshot.clone();
    bad.state
        .vm
        .world
        .actions
        .as_mut()
        .unwrap()
        .actors
        .get_mut(&1)
        .unwrap()
        .queue
        .pending[0]
        .menu_id = 999;
    assert!(bad.restore(&p).is_err());
    let mut bad = snapshot.clone();
    bad.state
        .interactions
        .get_mut(&1)
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .phase = Phase::Exiting;
    assert!(bad.restore(&p).is_err());
    let mut bad = snapshot.clone();
    bad.state
        .interactions
        .get_mut(&1)
        .unwrap()
        .active
        .as_mut()
        .unwrap()
        .thread = 999;
    assert!(bad.restore(&p).is_err());
    let mut bad = snapshot;
    bad.version = 3;
    assert!(bad.restore(&p).is_err());
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn queue_budget_stops_are_resumable_and_reselecting_the_same_chair_runs_original_code() {
    let (p, mut s) = setup(1);
    s.enqueue(&p, 1, 2, 0).unwrap();
    let mut reference = s.clone();
    let expected = run(&p, &mut reference);
    let mut trace = Vec::new();
    let mut events = Vec::new();
    loop {
        let step = s.run_fixture_tick(&p, 1).unwrap();
        trace.extend(step.trace);
        events.extend(step.events);
        s = saved(&p, &s);
        if step.stop != StepStop::Budget {
            break;
        }
        assert!(s.advance_fixture_tick().is_err());
    }
    assert_eq!(s, reference);
    assert_eq!(trace, expected.trace);
    assert_eq!(events, expected.events);
    while !seated(&s, 2) {
        tick(&p, &mut s);
    }
    let position = s.bodies[&1].clone();
    let second = s.enqueue(&p, 1, 2, 0).unwrap();
    let mut result = run(&p, &mut s);
    for _ in 0..400 {
        if s.interactions[&1]
            .active
            .as_ref()
            .is_some_and(|a| a.item.id == second && a.phase == Phase::Seated)
        {
            break;
        }
        let step = tick(&p, &mut s);
        result.trace.extend(step.trace);
        assert_eq!(s.bodies[&1], position);
    }
    assert!(seated(&s, 2));
    assert!(result.trace.iter().any(|t| t.location.bhav == 4097));
    assert!(!result.trace.iter().any(|t| t.location.bhav == 4102));
    assert_eq!(s.bodies[&1], position);
    assert_eq!(s.interactions[&1].active.as_ref().unwrap().item.id, second);
    for _ in 0..sims_sim::interaction::MAX_QUEUE {
        s.enqueue(&p, 1, 3, 0).unwrap();
    }
    let mut before = s.clone();
    assert!(s.enqueue(&p, 1, 3, 0).is_err());
    // Native serial allocation occurs even when the full queue rejects insertion.
    before.vm.world.actions.as_mut().unwrap().serial += 1;
    assert_eq!(s, before);
    saved(&p, &s);
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_chair_main_drives_comfort_and_resumes_while_seated_and_after_exit() {
    let (p, mut s) = setup(1);
    let comfort = |s: &Simulation| f32::from_bits(s.vm.world.objects[&1].motives.unwrap()[6]);
    assert_eq!(comfort(&s), -50.0);
    assert_eq!(s.vm.lifecycles.len(), 3);
    assert!(
        s.vm.lifecycles
            .values()
            .all(|l| l.phase == sims_vm::lifecycle::Phase::Main)
    );
    assert!(s.vm.world.refresh.as_ref().unwrap().rooms[&1].dirty);
    run(&p, &mut s);
    for _ in 0..200 {
        tick(&p, &mut s);
    }
    assert_eq!(comfort(&s), -50.0, "empty chairs do not change motives");
    let interaction = s.enqueue(&p, 1, 2, 0).unwrap();
    run(&p, &mut s);
    let mut saw_original_main = false;
    while !seated(&s, 2) {
        let step = tick(&p, &mut s);
        saw_original_main |= step.trace.iter().any(|t| t.location.bhav == 4113);
        assert!(s.vm.tick < 600);
    }
    assert!(saw_original_main);
    assert_eq!(comfort(&s), -48.0);
    let first = s.vm.tick;
    let mut replay = saved(&p, &s);
    for offset in 1..=180 {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
        assert_eq!(
            comfort(&s),
            -48.0 + 2.0 * (offset / 60) as f32,
            "tick {}",
            s.vm.tick
        );
        if offset == 59 {
            replay = saved(&p, &s);
        }
    }
    assert_eq!(s.vm.tick, first + 180);
    s.cancel_interaction(&p, 1, interaction).unwrap();
    run(&p, &mut s);
    while s.interactions[&1].active.is_some() {
        tick(&p, &mut s);
        assert!(s.vm.tick < 1000);
    }
    let standing = comfort(&s);
    replay = saved(&p, &s);
    for _ in 0..120 {
        assert_eq!(tick(&p, &mut s), tick(&p, &mut replay));
        assert_eq!(s, replay);
    }
    assert_eq!(comfort(&s), standing, "empty chair stops granting comfort");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn native_cancel_fallback_and_cancel_during_eligibility_resume_on_the_same_thread() {
    let (p, initial) = setup(1);
    for during_check in [false, true] {
        let mut s = initial.clone();
        let id = s.enqueue(&p, 1, 2, 0).unwrap();
        if during_check {
            // Original Wait For Notify branches, then Idle enters original Sit check.
            for _ in 0..300 {
                assert_eq!(s.run_fixture_tick(&p, 1).unwrap().stop, StepStop::Budget);
                let main = s.interactions[&1].main.unwrap();
                if s.vm.threads[&main].dispatch.is_some() {
                    break;
                }
            }
            let active = s.interactions[&1].active.as_ref().unwrap();
            assert!(s.vm.threads[&active.thread].dispatch.is_some());
        } else {
            run(&p, &mut s);
            assert!(!s.routes.is_empty());
        }
        let thread = s.interactions[&1].active.as_ref().unwrap().thread;
        s.cancel_interaction(&p, 1, id).unwrap();
        let mut replay = saved(&p, &s);
        let expected = run(&p, &mut s);
        let mut trace = vec![];
        let mut events = vec![];
        loop {
            let step = replay.run_fixture_tick(&p, 1).unwrap();
            trace.extend(step.trace);
            events.extend(step.events);
            assert!(trace.len() < 200, "cancel trace: {trace:#?}");
            replay = saved(&p, &replay);
            if step.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(trace, expected.trace);
        assert_eq!(events, expected.events);
        assert_eq!(s, replay);
        assert!(
            expected.trace.iter().any(|t| t.location.bhav == 8318),
            "original PersonGlobals Cancel Interaction must run"
        );
        assert!(
            expected
                .trace
                .iter()
                .any(|t| t.thread == thread && t.opcode == 17)
        );
        assert!(s.interactions[&1].active.is_none());
        assert!(s.pending_interactions(1).unwrap().is_empty());
        assert!(s.routes.is_empty());
        assert_eq!(s.vm.world.objects[&2].data[&62], 0);
        assert!(s.vm.world.objects[&1].container.is_none());
    }
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_init_and_autonomous_chair_search_resume_then_accept_player_intervention() {
    let (p, mut wide) = setup_with_init(1, true);
    assert_eq!(wide.vm.world.objects[&1].person.as_ref().unwrap()[&36], 50);
    let motives = wide
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .motives
        .as_mut()
        .unwrap();
    for index in [5, 7, 8, 9, 13, 14, 15] {
        motives[index] = 50f32.to_bits();
    }
    motives[6] = (-30f32).to_bits();
    wide.enable_household(
        &p,
        sims_sim::clock::LotClock {
            hour: 8,
            minute: 0,
            second: 0,
            day: 1,
            month: 1,
            year: 1997,
            period: 0,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .unwrap();
    wide.enable_residential_autonomy([2, 3]).unwrap();
    let mut narrow = saved(&p, &wide);
    let mut entered = false;
    let mut restored_search = false;
    let mut selected = false;
    for index in 0..900 {
        if index == 549 {
            assert!(entered, "autonomy did not enter its chair");
            wide.enqueue(&p, 1, 3, 0).unwrap();
            narrow.enqueue(&p, 1, 3, 0).unwrap();
        }
        let step = run(&p, &mut wide);
        selected |= step.trace.iter().any(|t| t.opcode == 30);
        let mut trace = vec![];
        loop {
            let one = narrow.run_fixture_tick(&p, 1).unwrap();
            trace.extend(one.trace);
            assert!(!matches!(one.stop, StepStop::Faulted | StepStop::Animation));
            if !restored_search
                && narrow
                    .vm
                    .threads
                    .values()
                    .any(|t| t.autonomy.is_some() && t.callback.is_some())
            {
                narrow = saved(&p, &narrow);
                restored_search = true;
            }
            if one.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(step.trace, trace, "tick {index}");
        assert_eq!(wide, narrow, "tick {index}");
        if index < 549 {
            entered |= wide.vm.world.objects[&1].container == Some((2, 0));
        }
        if index % 137 == 0 {
            narrow = saved(&p, &narrow);
        }
        wide.advance_fixture_tick().unwrap();
        narrow.advance_fixture_tick().unwrap();
    }
    assert!(selected && entered && restored_search);
    assert_eq!(wide.vm.world.objects[&1].container, Some((3, 0)));
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_critical_comfort_reaction_turn_and_headline_resume_then_sit() {
    let (p, mut wide) = setup_with_init(1, true);
    let motives = wide
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .motives
        .as_mut()
        .unwrap();
    for index in [5, 7, 8, 9, 13, 14, 15] {
        motives[index] = 50f32.to_bits();
    }
    motives[6] = (-90f32).to_bits();
    wide.enable_household(
        &p,
        sims_sim::clock::LotClock {
            hour: 8,
            minute: 0,
            second: 0,
            day: 1,
            month: 1,
            year: 1997,
            period: 0,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .unwrap();
    wide.enable_residential_autonomy([2, 3]).unwrap();
    let mut narrow = saved(&p, &wide);
    let (mut turn, mut headline, mut complaint, mut seated) = (false, false, false, false);
    for tick in 0..1400 {
        let step = run(&p, &mut wide);
        let mut trace = vec![];
        loop {
            let one = narrow.run_fixture_tick(&p, 1).unwrap();
            trace.extend(one.trace);
            assert!(!matches!(one.stop, StepStop::Faulted | StepStop::Animation));
            if one.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(step.trace, trace, "tick {tick}");
        assert_eq!(wide, narrow, "tick {tick}");
        turn |= !wide.turns.is_empty();
        headline |= wide.vm.world.objects[&1]
            .headline
            .as_ref()
            .is_some_and(|h| h.sprite == 205 && h.duration != 0);
        complaint |= wide
            .animations
            .get(&1)
            .is_some_and(|a| a.clip == "a2o-uncomfortable2");
        if tick % 47 == 0
            || !wide.turns.is_empty()
            || (tick % 11 == 0
                && wide.vm.world.objects[&1]
                    .headline
                    .as_ref()
                    .is_some_and(|h| h.duration != 0))
        {
            narrow = saved(&p, &narrow);
        }
        if wide.vm.world.objects[&1].container == Some((3, 0))
            && wide
                .animations
                .get(&1)
                .is_some_and(|p| p.looping && p.clip == "a2o-sit-loop")
        {
            seated = true;
            break;
        }
        wide.advance_fixture_tick().unwrap();
        narrow.advance_fixture_tick().unwrap();
    }
    assert!(turn && headline && complaint && seated);
}
