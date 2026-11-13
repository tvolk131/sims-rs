use sims_content::{Index, vm::*};
use sims_sim::{Snapshot, StepStop, interaction::Phase};
use sims_vm::Status;
fn setup() -> (sims_vm::Program, sims_sim::Simulation) {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let i = Index::scan(&root).unwrap();
    let mut p = load_program(&i, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&i, &mut p).unwrap();
    let s = two_chair_room(
        &p,
        ChairFixture::load(&i, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap(),
        1,
    )
    .unwrap();
    (p, s)
}
fn run(p: &sims_vm::Program, s: &mut sims_sim::Simulation) -> sims_sim::Step {
    let r = s.run_fixture_tick(p, 1000).unwrap();
    assert_eq!(r.stop, StepStop::Waiting, "{:?}", s.vm.threads);
    r
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn persistent_original_main_idles_dispatches_and_returns_across_rng_states() {
    let (p, initial) = setup();
    let main = initial.interactions[&1].main.unwrap();
    assert!(matches!(
        initial.vm.threads[&main].status,
        Status::Sleeping { .. }
    ));
    let mut saw_reverse = false;
    let mut saw_seated_idle = false;
    for seed in [0, 1, 2, u32::MAX] {
        let mut s = initial.clone();
        s.vm.rng_state = seed;
        for _ in 0..180 {
            s.advance_fixture_tick().unwrap();
            run(&p, &mut s);
        }
        let id = s.enqueue(&p, 1, 2, 0).unwrap();
        run(&p, &mut s);
        for _ in 0..650 {
            saw_reverse |= s.animations.get(&1).is_some_and(|p| p.reverse);
            if s.interactions[&1]
                .active
                .as_ref()
                .is_some_and(|a| a.phase == Phase::Seated)
            {
                break;
            }
            s.advance_fixture_tick().unwrap();
            run(&p, &mut s);
            if s.vm.tick % 47 == 0 {
                s = Snapshot::capture(&p, &s).restore(&p).unwrap();
            }
        }
        assert_eq!(s.vm.world.objects[&1].container, Some((2, 0)));
        assert_eq!(
            s.interactions[&1].active.as_ref().unwrap().phase,
            Phase::Seated
        );
        for _ in 0..80 {
            s.advance_fixture_tick().unwrap();
            let r = run(&p, &mut s);
            saw_seated_idle |= r.trace.iter().any(|t| t.location.bhav == 8229);
        }
        s.cancel_interaction(&p, 1, id).unwrap();
        run(&p, &mut s);
        for _ in 0..400 {
            if s.interactions[&1].active.is_none() {
                break;
            }
            s.advance_fixture_tick().unwrap();
            run(&p, &mut s);
        }
        assert!(s.interactions[&1].active.is_none());
        assert!(s.vm.world.objects[&1].container.is_none());
        assert_eq!(s.interactions[&1].main, Some(main));
        assert_eq!(s.vm.threads.len(), 3);
        assert_eq!(s.vm.threads[&main].stack[0].location.bhav, 4096);
        s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        for _ in 0..20 {
            s.advance_fixture_tick().unwrap();
            run(&p, &mut s);
        }
    }
    assert!(
        saw_reverse,
        "original reset-idle should reverse a stance transition"
    );
    assert!(saw_seated_idle);
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn missing_autonomy_context_is_explicit_and_corrupt_person_directories_are_rejected() {
    let (p, mut s) = setup();
    s.vm.world.person.as_mut().unwrap().free_will = true;
    // Exercise the original autonomy branch without the residential advertisement
    // context. Full households supply that context; this tiny fixture does not.
    let owner = s.vm.world.objects[&1]
        .definition
        .as_ref()
        .unwrap()
        .source
        .clone();
    let main =
        s.vm.spawn(
            &p,
            sims_vm::Start {
                owner: &owner,
                bhav: 8233,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    let run = s.vm.run_slice(&p, main, 1000).unwrap();
    assert_eq!(run.stop, sims_vm::Stop::Faulted);
    assert!(
        matches!(&s.vm.threads[&main].status,Status::Faulted(e) if e.message.contains("autonomy")),
        "{:?}",
        s.vm.threads[&main].status
    );
    let (_, s) = setup();
    let mut saved = Snapshot::capture(&p, &s);
    saved
        .state
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .person_order
        .push(2);
    assert!(saved.restore(&p).is_err());
}
