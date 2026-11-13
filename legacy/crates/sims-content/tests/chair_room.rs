use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, chair_room, load_program},
};
use sims_sim::{Snapshot, StepStop};
use sims_vm::Status;
use std::path::PathBuf;

#[test]
#[ignore = "requires the user's private Complete Collection corpus"]
fn original_chair_routes_snaps_and_selects_animation_in_all_cardinal_orientations() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    for facing in [1, 4, 16, 64] {
        let fixture =
            ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
        let mut a = chair_room(&program, fixture, false, facing).unwrap();
        assert_eq!(
            a.run_fixture_tick(&program, 100).unwrap().stop,
            StepStop::Waiting
        );
        let route = &a.routes[&2];
        assert!(route.path.len() > 50);
        // The recovered candidate scoring may choose either reachable side.
        // Verify that the original Sit tree selects the animation belonging
        // to that entry, rather than freezing an older host pathfinder's choice.
        let animation = match route.goal.entry_flags {
            1 => 1,
            16 => 2,
            64 => 3,
            4 => 4,
            entry => panic!("invalid chair entry {entry}"),
        };
        assert_eq!(a.bodies[&1].position, route.path[0]);
        for _ in 0..31 {
            a.advance_fixture_tick().unwrap();
            a.run_fixture_tick(&program, 100).unwrap();
        }
        let bytes = serde_json::to_vec(&Snapshot::capture(&program, &a)).unwrap();
        let mut b = Snapshot::from_json(&bytes, &program).unwrap();
        loop {
            a.advance_fixture_tick().unwrap();
            b.advance_fixture_tick().unwrap();
            let x = a.run_fixture_tick(&program, 100).unwrap();
            let y = b.run_fixture_tick(&program, 100).unwrap();
            assert_eq!(x, y);
            assert_eq!(a, b);
            if x.stop != StepStop::Waiting {
                assert_eq!(x.stop, StepStop::Animation);
                break;
            }
            assert!(a.vm.tick < 256);
        }
        let Status::AwaitingAnimation(request) = &a.vm.threads[&2].status else {
            panic!()
        };
        assert_eq!(request.origin.bhav, 0x1003);
        assert_eq!(request.operands, [animation, 0, 0, 0, 0, 64, 0, 0]);
        assert_eq!(a.vm.world.objects[&1].container, Some((2, 0)));
        assert_eq!(a.vm.world.objects[&2].slots[0], Some(1));
        assert_eq!(a.vm.world.objects[&1].person.as_ref().unwrap()[&0], 1);
        assert_eq!(a.bodies[&1].elevation, 7);
        assert_eq!(a.vm.world.objects[&1].person.as_ref().unwrap()[&73], 0);
        Snapshot::capture(&program, &a).restore(&program).unwrap();
    }
    for (scenario, blocked, action_result) in [
        (ChairScenario::Empty, true, Some(false)),
        (ChairScenario::Occupied, false, None),
        (ChairScenario::AlreadySeated, false, Some(true)),
        (ChairScenario::AutonomousSeated, false, None),
    ] {
        let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, scenario).unwrap();
        let mut sim = chair_room(&program, fixture, blocked, 1).unwrap();
        let before = sim.bodies.clone();
        if blocked {
            // This minimal fixture deliberately omits PersonGlobals and the
            // named failure-feedback tree. The complete household acceptance
            // exercises that yielding tree; missing resources must fail here.
            assert_eq!(
                sim.run_fixture_tick(&program, 100).unwrap_err(),
                "named behavior module missing"
            );
            assert_eq!(sim.bodies, before);
            continue;
        }
        let step = sim.run_fixture_tick(&program, 100).unwrap();
        assert_eq!(step.stop, StepStop::Complete, "{scenario:?}");
        assert_eq!(sim.bodies, before);
        if let Some(value) = action_result {
            assert_eq!(sim.vm.threads[&2].status, Status::Returned(value));
        } else {
            assert_eq!(sim.vm.threads.len(), 1);
            assert_eq!(sim.vm.threads[&1].status, Status::Returned(false));
        }
        assert!(sim.routes.is_empty());
        assert!(sim.reservations.is_empty());
        Snapshot::capture(&program, &sim).restore(&program).unwrap();
    }
}
