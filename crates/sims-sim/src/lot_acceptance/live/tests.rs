use super::*;
use crate::{StepStop, lot_runtime::tests::fixture};
use sims_vm::refresh::RefreshState;

fn input(level: i32, covering: u16) -> Input {
    Input {
        size: 6,
        position: [40, 40],
        level,
        person: false,
        placement_flags: 0,
        height_mask: 1,
        outside: 0,
        water: 255,
        pool: 16,
        heights: [0; 4],
        covering,
        tile_flags: 0,
        occupants: vec![],
    }
}

fn pending(level: i32, covering: u16) -> Base {
    let mut floor = Floor::new(input(level, covering), 77).unwrap();
    assert_eq!(floor.next(), Some(Request::WallFit));
    floor
        .resume(
            Request::WallFit,
            Reply::Geometry(Outcome {
                accepted: true,
                error: 77,
            }),
        )
        .unwrap();
    assert_eq!(floor.next(), Some(Request::ActiveLevel));
    Base {
        object: 1,
        position: [40, 40],
        level,
        container: None,
        floor,
        collision: None,
        callback: None,
    }
}

fn step() -> Step {
    Step {
        stop: StepStop::Budget,
        trace: vec![],
        events: vec![],
    }
}

#[test]
fn active_level_rejections_use_current_retained_view_without_vm_work() {
    for (level, covering, visible, error) in [
        (1, 1, 1, 0x13),
        (1, 1, 2, 0x2a),
        (2, 0, 2, 0x12),
        (2, 0, 1, 0x29),
        (2, 0, -7, 0x29),
    ] {
        let (program, mut simulation) = fixture();
        simulation.native_lot.as_mut().unwrap().diagnostics.error = 77;
        simulation.vm.world.refresh = Some(RefreshState {
            visible_level: 1,
            ..Default::default()
        });
        let bytes = serde_json::to_vec(&pending(level, covering)).unwrap();
        let mut execution: Base = serde_json::from_slice(&bytes).unwrap();
        simulation.vm.world.refresh.as_mut().unwrap().visible_level = visible;
        let before = simulation.vm.clone();
        let mut remaining = 0;
        let mut report = step();
        assert!(
            simulation
                .drive_base_acceptance(&program, &mut execution, &mut remaining, &mut report)
                .unwrap()
        );
        assert_eq!(
            execution.outcome(),
            Some(Outcome {
                accepted: false,
                error
            })
        );
        assert_eq!(
            simulation.native_lot.as_ref().unwrap().diagnostics.error,
            error
        );
        assert_eq!(simulation.vm, before);
        assert_eq!(remaining, 0);
        assert_eq!(report, step());
    }
}

#[test]
fn missing_active_level_owner_preserves_the_pending_rejection() {
    let (program, mut simulation) = fixture();
    simulation.native_lot.as_mut().unwrap().diagnostics.error = 77;
    simulation.vm.world.refresh = None;
    let mut execution = pending(1, 1);
    let before = execution.clone();
    let vm = simulation.vm.clone();
    let diagnostics = simulation.native_lot.as_ref().unwrap().diagnostics;
    let mut remaining = 0;
    let mut report = step();
    assert_eq!(
        simulation
            .drive_base_acceptance(&program, &mut execution, &mut remaining, &mut report)
            .unwrap_err(),
        "native acceptance active view level unavailable"
    );
    assert_eq!(execution, before);
    assert_eq!(simulation.vm, vm);
    assert_eq!(
        simulation.native_lot.as_ref().unwrap().diagnostics,
        diagnostics
    );
    assert_eq!(remaining, 0);
    assert_eq!(report, step());
}

#[test]
fn early_floor_rejection_does_not_require_an_active_view_owner() {
    let (program, mut simulation) = fixture();
    simulation.native_lot.as_mut().unwrap().diagnostics.error = 77;
    simulation.vm.world.refresh = None;
    let mut execution = pending(1, 1);
    let mut outside = input(1, 1);
    outside.position = [0, 0];
    execution.position = outside.position;
    execution.floor = Floor::new(outside, 77).unwrap();
    let vm = simulation.vm.clone();
    let mut remaining = 0;
    let mut report = step();
    assert!(
        simulation
            .drive_base_acceptance(&program, &mut execution, &mut remaining, &mut report)
            .unwrap()
    );
    assert_eq!(
        execution.outcome(),
        Some(Outcome {
            accepted: false,
            error: 1
        })
    );
    assert_eq!(simulation.vm, vm);
    assert_eq!(remaining, 0);
    assert_eq!(report, step());
}
