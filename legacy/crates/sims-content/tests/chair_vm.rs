use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, load_program},
};
use sims_vm::{Snapshot, Start, Status, Stop, Vm};
use std::path::PathBuf;

#[test]
#[ignore = "requires the user's private Complete Collection corpus"]
fn original_sit_checks_call_stack_route_and_resume() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    for (scenario, eligible) in [
        (ChairScenario::Empty, true),
        (ChairScenario::Occupied, false),
        (ChairScenario::AlreadySeated, true),
        (ChairScenario::AutonomousSeated, false),
    ] {
        let fixture = ChairFixture::load(&index, "DiningChairs.iff", 16807, scenario).unwrap();
        let mut state = Vm::new(fixture.world, 1);
        let check = state
            .spawn(
                &program,
                Start {
                    owner: &fixture.owner,
                    bhav: fixture.check,
                    caller: 1,
                    callee: 2,
                    stack_object: 2,
                    args: fixture.args.clone(),
                    check: true,
                },
            )
            .unwrap();
        assert_eq!(
            state.run_slice(&program, check, 100).unwrap().stop,
            Stop::Returned(eligible),
            "{scenario:?}"
        );
        if !eligible {
            continue;
        }
        let action = state
            .spawn(
                &program,
                Start {
                    owner: &fixture.owner,
                    bhav: fixture.action,
                    caller: 1,
                    callee: 2,
                    stack_object: 2,
                    args: fixture.args,
                    check: false,
                },
            )
            .unwrap();
        let result = state.run_slice(&program, action, 100).unwrap();
        if scenario == ChairScenario::AlreadySeated {
            assert_eq!(result.stop, Stop::Returned(true));
            continue;
        }
        assert_eq!(result.stop, Stop::AwaitingRoute);
        assert_eq!(
            state.threads[&action]
                .stack
                .iter()
                .map(|f| f.location.bhav)
                .collect::<Vec<_>>(),
            [0x1001, 0x1004, 0x1003]
        );
        let Status::AwaitingRoute(request) = &state.threads[&action].status else {
            panic!()
        };
        let sims_vm::routing::Destination::Slot {
            index,
            chunk,
            version,
            source,
            ..
        } = &request.destination
        else {
            panic!("expected routing SLOT")
        };
        assert_eq!(*index, 1);
        assert_eq!(*chunk, 100);
        assert_eq!(*version, 10);
        assert!(source.ends_with("Global.far::Global.iff"));
        let id = request.id;
        let bytes = serde_json::to_vec(&Snapshot::capture(&program, &state)).unwrap();
        let mut resumed = Snapshot::from_json(&bytes, &program).unwrap();
        assert_eq!(state, resumed);
        assert_eq!(
            state.run_slice(&program, action, 100).unwrap(),
            resumed.run_slice(&program, action, 100).unwrap()
        );
        // A routing service failure must unwind all original calls through their false branches.
        state.finish_route(&program, action, id, false).unwrap();
        resumed.finish_route(&program, action, id, false).unwrap();
        assert_eq!(state, resumed);
        assert_eq!(resumed.threads[&action].status, Status::Returned(false));
        // A synthetic success response only probes the next compatibility boundary.
        // No route, animation, or completed physical Sit is claimed by this fixture.
        let mut probe = Snapshot::from_json(&bytes, &program).unwrap();
        probe.finish_route(&program, action, id, true).unwrap();
        assert_eq!(
            probe.run_slice(&program, action, 100).unwrap().stop,
            Stop::AwaitingSnap
        );
        let Status::AwaitingSnap(snap) = &probe.threads[&action].status else {
            panic!()
        };
        assert_eq!(snap.route.origin.instruction, 19);
        assert_eq!(snap.route.destination.slot().unwrap().0.snap_target, 0);
    }
}
