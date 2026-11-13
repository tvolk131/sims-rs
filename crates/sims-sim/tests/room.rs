use sims_formats::{
    behavior::{Behavior, Instruction},
    execution::{Slot, Slots},
};
use sims_sim::{
    Action, Body, Point, Rect, Room, Simulation, Snapshot, StepStop, geometry::destinations,
};
use sims_vm::{Definition, Module, Object, Program, Routine, Status, Vm, World};
use std::collections::BTreeMap;

fn slot() -> Slot {
    Slot {
        kind: 3,
        offset: [0.0; 3],
        standing: 1,
        sitting: 0,
        ground: 0,
        flags: 0x55,
        snap_target: -1,
        proximity: [16; 3],
        max_size: 0,
        unknown: 0,
        gradient: 0.1875,
        height: 0,
        facing: -2,
        resolution: 16,
    }
}
fn ins(opcode: u16, on_true: u8, operands: [u8; 8]) -> Instruction {
    Instruction {
        opcode,
        on_true,
        on_false: 255,
        operands,
    }
}
fn routine(instructions: Vec<Instruction>) -> Routine {
    Routine {
        name: "synthetic room behavior".into(),
        body: Behavior {
            format: 0x8002,
            kind: 0,
            args: 4,
            locals: 1,
            instructions,
        },
    }
}
fn fixture() -> (Program, Simulation) {
    let mut containment = slot();
    containment.kind = 0;
    containment.height = 7;
    containment.flags = 0;
    let mut snap = slot();
    snap.snap_target = 0;
    snap.flags = 0;
    snap.standing = 0;
    let p = Program {
        neighbor_resource_flags: None,
        text_language: None,
        resource_storage: None,
        careers: None,
        accessories: BTreeMap::new(),
        animations: BTreeMap::new(),
        native_animation_absences: BTreeMap::new(),
        globals: "global".into(),
        modules: BTreeMap::from([
            (
                "global".into(),
                Module {
                    source: "global".into(),
                    slots: BTreeMap::from([(
                        100,
                        Slots {
                            version: 10,
                            entries: vec![slot()],
                        },
                    )]),
                    ..Module::default()
                },
            ),
            (
                "chair".into(),
                Module {
                    source: "chair".into(),
                    slots: BTreeMap::from([(
                        128,
                        Slots {
                            version: 10,
                            entries: vec![containment, snap],
                        },
                    )]),
                    routines: BTreeMap::from([
                        (4096, routine(vec![ins(2, 254, [0, 0, 0, 0, 0, 2, 7, 7])])),
                        (
                            4097,
                            routine(vec![
                                ins(0x2d, 1, [0, 0, 2, 0, 0, 0, 0, 0]),
                                ins(0x2e, 2, [0, 0, 3, 0, 4, 0, 0, 0]),
                                ins(0x2c, 254, [1, 0, 0, 0, 0, 64, 0, 0]),
                            ]),
                        ),
                    ]),
                    ..Module::default()
                },
            ),
        ]),
    };
    let actor = Object {
        person: Some(BTreeMap::from([(0, 0), (37, 0), (62, 0), (73, 0)])),
        data: BTreeMap::from([(1, 0), (52, 0), (54, 0)]),
        ..Object::default()
    };
    let chair = Object {
        definition: Some(Definition {
            source: "chair".into(),
            object: 1,
            guid: 42,
            slots: 128,
        }),
        data: BTreeMap::from([(1, 0)]),
        slots: vec![None],
        ..Object::default()
    };
    let vm = Vm::new(
        World {
            objects: BTreeMap::from([(1, actor), (2, chair)]),
            ..World::default()
        },
        1,
    );
    let room = Room {
        width: 96,
        height: 80,
        obstacles: vec![Rect {
            min: Point::new(40, 24),
            max: Point::new(48, 56),
        }],
    };
    let bodies = BTreeMap::from([
        (
            1,
            Body {
                level: 1,
                position: Point::new(16, 40),
                facing: 1,
                radius: 4,
                elevation: 0,
            },
        ),
        (
            2,
            Body {
                level: 1,
                position: Point::new(72, 40),
                facing: 1,
                radius: 6,
                elevation: 0,
            },
        ),
    ]);
    let mut sim = Simulation::new(vm, room, bodies).unwrap();
    queue(&p, &mut sim, 1);
    (p, sim)
}
fn queue(p: &Program, s: &mut Simulation, actor: i16) {
    s.queue_interaction(
        p,
        4096,
        Action {
            owner: "chair".into(),
            bhav: 4097,
            actor,
            target: 2,
            args: vec![],
        },
    )
    .unwrap();
}
fn finish(p: &Program, s: &mut Simulation) -> sims_sim::Step {
    for _ in 0..256 {
        let step = s.run_fixture_tick(p, 100).unwrap();
        if step.stop != StepStop::Waiting {
            return step;
        }
        s.advance_fixture_tick().unwrap();
    }
    panic!("room never stopped")
}
fn add_actor(s: &mut Simulation, id: i16, position: Point) {
    s.vm.world
        .objects
        .insert(id, s.vm.world.objects[&1].clone());
    s.bodies.insert(
        id,
        Body {
            level: 1,
            position,
            facing: 1,
            radius: 4,
            elevation: 0,
        },
    );
}
#[test]
fn slot_directions_rotate_and_old_proximity_units_are_normalized() {
    let (_, sim) = fixture();
    let target = &sim.bodies[&2];
    let caller = &sim.bodies[&1];
    let mut a = slot();
    a.flags = 1;
    let north = destinations(&a, 10, target, caller).unwrap();
    assert_eq!(north[0].position, Point::new(72, 24));
    assert_eq!(north[0].facing, 16);
    assert_eq!(north[0].entry_flags, 1);
    let mut east = target.clone();
    east.facing = 4;
    let g = destinations(&a, 10, &east, caller).unwrap()[0];
    assert_eq!(g.position, Point::new(88, 40));
    assert_eq!(g.facing, 64);
    assert_eq!(g.entry_flags, 1);
    a.proximity = [1; 3];
    assert_eq!(destinations(&a, 9, target, caller).unwrap(), north);
    a.flags |= 512;
    assert_eq!(destinations(&a, 9, &east, caller).unwrap(), north);
    a.flags = 4097;
    a.offset = [0.0, -16.0, 0.0];
    let g = destinations(&a, 10, &east, caller).unwrap()[0];
    assert_eq!(g.position, Point::new(88, 40));
    assert_eq!(g.facing, 4);
}
#[test]
fn unsupported_slot_shapes_are_errors_not_guessed_goals() {
    let (_, s) = fixture();
    for bad in 0..7 {
        let mut a = slot();
        match bad {
            0 => a.flags |= 2,
            1 => a.proximity[1] = 32,
            2 => a.offset[0] = 0.5,
            3 => a.offset[2] = 15.0,
            4 => a.sitting = 1,
            5 => a.flags |= 8192,
            _ => a.facing = 3,
        };
        assert!(
            destinations(&a, 10, &s.bodies[&2], &s.bodies[&1]).is_err(),
            "case {bad}"
        );
    }
}
#[test]
fn actual_route_detours_and_snap_updates_both_sides_of_containment() {
    let (p, mut s) = fixture();
    let start = s.bodies[&1].position;
    assert_eq!(s.run_fixture_tick(&p, 100).unwrap().stop, StepStop::Waiting);
    assert_eq!(s.bodies[&1].position, start);
    let route = s.routes.values().next().unwrap();
    let direct = {
        let dx = (start.x - route.goal.position.x).unsigned_abs();
        let dy = (start.y - route.goal.position.y).unsigned_abs();
        dx.min(dy) * 1414 + dx.abs_diff(dy) * 1000
    };
    assert!(*route.locomotion.distances.last().unwrap() > direct);
    let arrival = route.locomotion.duration();
    assert!(
        route
            .path
            .windows(2)
            .any(|w| w[0].x != w[1].x && w[0].y != w[1].y)
    );
    let path = route.path.clone();
    assert!(path.iter().any(|p| p.y <= 20 || p.y >= 60));
    let step = finish(&p, &mut s);
    assert_eq!(step.stop, StepStop::Animation);
    assert_eq!(s.vm.tick, u64::from(arrival));
    assert_eq!(s.vm.world.objects[&1].container, Some((2, 0)));
    assert_eq!(s.vm.world.objects[&2].slots, [Some(1)]);
    assert_eq!(s.bodies[&1].position, s.bodies[&2].position);
    assert_eq!(s.bodies[&1].elevation, 7);
    assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&62], 0);
    assert!(s.routes.is_empty());
    assert_eq!(s.reservations.len(), 1);
    Snapshot::capture(&p, &s).restore(&p).unwrap();
}
#[test]
fn mid_route_snapshot_replays_identical_movement_bytecode_and_final_state() {
    let (p, mut a) = fixture();
    a.run_fixture_tick(&p, 100).unwrap();
    for _ in 0..17 {
        a.advance_fixture_tick().unwrap();
        a.run_fixture_tick(&p, 100).unwrap();
    }
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &a)).unwrap();
    let mut b = Snapshot::from_json(&bytes, &p).unwrap();
    assert_eq!(a, b);
    loop {
        a.advance_fixture_tick().unwrap();
        b.advance_fixture_tick().unwrap();
        let x = a.run_fixture_tick(&p, 100).unwrap();
        let y = b.run_fixture_tick(&p, 100).unwrap();
        assert_eq!(x, y);
        assert_eq!(a, b);
        if x.stop != StepStop::Waiting {
            assert_eq!(x.stop, StepStop::Animation);
            break;
        }
    }
}

#[test]
fn relative_routes_share_movement_budgeting_and_save_resume() {
    for facing in [254, 255, 2] {
        let (mut p, mut full) = fixture();
        // No synthetic routing SLOT lookup: this original primitive constructs
        // its descriptor directly, including the default failure-tree flag.
        p.modules
            .get_mut("chair")
            .unwrap()
            .routines
            .get_mut(&4097)
            .unwrap()
            .body
            .instructions = vec![ins(0x1b, 254, [5, 0, 0, facing, 0, 0, 0, 0])];
        let mut split = full.clone();
        for _ in 0..256 {
            let result = full.run_fixture_tick(&p, 100).unwrap();
            while split.run_fixture_tick(&p, 1).unwrap().stop == StepStop::Budget {
                split = Snapshot::capture(&p, &split).restore(&p).unwrap();
            }
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
            assert_eq!(full, split);
            if result.stop == StepStop::Complete {
                break;
            }
            assert_eq!(result.stop, StepStop::Waiting);
            full.advance_fixture_tick().unwrap();
            split.advance_fixture_tick().unwrap();
        }
        assert!(full.routes.is_empty());
        assert_eq!(full.bodies[&1].position, Point::new(72, 24));
        assert_eq!(
            full.bodies[&1].facing,
            match facing {
                254 => 16,
                255 => 1,
                _ => 4,
            }
        );
        assert_eq!(full.vm.world.objects[&1].person.as_ref().unwrap()[&62], 0);
    }
}
#[test]
fn budgeted_check_and_route_resume_without_advancing_or_repeating_movement() {
    let (p, mut full) = fixture();
    let mut split = full.clone();
    let initial = split.bodies[&1].position;
    assert_eq!(
        split.run_fixture_tick(&p, 0).unwrap().stop,
        StepStop::Budget
    );
    split = Snapshot::capture(&p, &split).restore(&p).unwrap();
    assert!(split.advance_fixture_tick().is_err());
    full.run_fixture_tick(&p, 100).unwrap();
    while split.run_fixture_tick(&p, 1).unwrap().stop == StepStop::Budget {
        split = Snapshot::capture(&p, &split).restore(&p).unwrap();
    }
    assert_eq!(full, split);
    assert_eq!(split.bodies[&1].position, initial);
    for _ in 0..256 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let result = full.run_fixture_tick(&p, 100).unwrap();
        while split.run_fixture_tick(&p, 1).unwrap().stop == StepStop::Budget {}
        let saved = split.clone();
        split.run_fixture_tick(&p, 1).unwrap();
        assert_eq!(saved, split, "repeated service call moved twice");
        assert_eq!(full, split);
        if result.stop == StepStop::Animation {
            return;
        }
    }
    panic!()
}
#[test]
fn blocked_room_returns_false_and_releases_reservation() {
    let (p, mut s) = fixture();
    s.room.obstacles[0].min.y = 0;
    s.room.obstacles[0].max.y = 80;
    let start = s.bodies[&1].position;
    let step = finish(&p, &mut s);
    assert_eq!(step.stop, StepStop::Complete);
    assert_eq!(s.vm.threads[&2].status, Status::Returned(false));
    assert_eq!(s.bodies[&1].position, start);
    assert!(s.reservations.is_empty());
    assert!(s.routes.is_empty());
    assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&62], 3);
    assert!(
        step.events
            .iter()
            .any(|e| e.detail.contains("RouteResult=3; branch false"))
    );
}
#[test]
fn occupied_chair_and_competing_routes_cannot_double_claim() {
    let (p, mut s) = fixture();
    add_actor(&mut s, 3, Point::new(16, 64));
    queue(&p, &mut s, 3);
    s.run_fixture_tick(&p, 100).unwrap();
    assert_eq!(s.reservations.len(), 1);
    assert_eq!(s.routes.len(), 1);
    assert_eq!(s.vm.threads[&4].status, Status::Returned(false));
    assert_eq!(s.vm.world.objects[&3].person.as_ref().unwrap()[&62], 9);
    assert_eq!(finish(&p, &mut s).stop, StepStop::Animation);
    let (p, mut s) = fixture();
    add_actor(&mut s, 3, Point::new(72, 40));
    s.vm.world.objects.get_mut(&3).unwrap().container = Some((2, 0));
    s.vm.world.objects.get_mut(&2).unwrap().slots[0] = Some(3);
    s.bodies.get_mut(&3).unwrap().elevation = 7;
    assert_eq!(finish(&p, &mut s).stop, StepStop::Complete);
    assert_eq!(s.vm.world.objects[&2].slots, [Some(3)]);
}
#[test]
fn dynamic_blockage_target_movement_and_interruption_fail_without_teleporting() {
    for case in 0..3 {
        let (p, mut s) = fixture();
        s.run_fixture_tick(&p, 100).unwrap();
        s.advance_fixture_tick().unwrap();
        s.run_fixture_tick(&p, 100).unwrap();
        let current = s.bodies[&1].position;
        match case {
            0 => {
                let next = s.routes[&2].path[s.routes[&2].next];
                s.room.obstacles.push(Rect {
                    min: Point::new(next.x - 4, next.y - 4),
                    max: Point::new(next.x + 4, next.y + 4),
                });
            }
            1 => s.bodies.get_mut(&2).unwrap().position.x += 1,
            _ => s.vm.interrupt(2).unwrap(),
        }
        s.advance_fixture_tick().unwrap();
        let result = s.run_fixture_tick(&p, 100).unwrap();
        assert_eq!(result.stop, StepStop::Complete);
        assert_eq!(s.bodies[&1].position, current);
        assert!(s.reservations.is_empty());
        assert!(s.routes.is_empty());
    }
}
#[test]
fn snapshot_rejects_path_teleports_bad_requests_and_invalid_containment() {
    let (p, mut s) = fixture();
    s.run_fixture_tick(&p, 100).unwrap();
    for case in 0..7 {
        let mut a = Snapshot::capture(&p, &s);
        match case {
            0 => a.state.routes.get_mut(&2).unwrap().path[2] = Point::new(90, 70),
            1 => a.state.routes.get_mut(&2).unwrap().path[2].x = i32::MAX,
            2 => a.state.routes.get_mut(&2).unwrap().next = usize::MAX,
            3 => a.state.routes.get_mut(&2).unwrap().request = 999,
            4 => {
                a.state.reservations.insert(2, 999);
            }
            5 => a.state.routes.get_mut(&2).unwrap().last_tick = 999,
            _ => a.state.routes.get_mut(&2).unwrap().goal.entry_flags = 0,
        }
        assert!(a.restore(&p).is_err(), "case {case}");
    }
    finish(&p, &mut s);
    let mut a = Snapshot::capture(&p, &s);
    a.state.bodies.get_mut(&1).unwrap().position.x += 1;
    assert!(a.restore(&p).is_err());
    let mut a = Snapshot::capture(&p, &s);
    if let Status::AwaitingAnimation(r) = &mut a.state.vm.threads.get_mut(&2).unwrap().status {
        r.operands[0] = 55;
    }
    assert!(a.restore(&p).is_err());
}

#[test]
fn snapshot_checks_all_bodies_before_geometry_and_rejects_extreme_values() {
    let (p, s) = fixture();
    for case in 0..4 {
        let mut a = Snapshot::capture(&p, &s);
        match case {
            0 => a.state.bodies.get_mut(&2).unwrap().radius = i32::MAX,
            1 => a.state.bodies.get_mut(&2).unwrap().position.x = i32::MIN,
            2 => {
                let b = a.state.bodies.remove(&2).unwrap();
                a.state.bodies.insert(9, b);
            }
            _ => a.state.room.width = i32::MAX,
        }
        assert!(a.restore(&p).is_err());
    }
}

fn animated_fixture() -> (Program, Simulation) {
    use sims_formats::animation::{Animation, Samples, TimedProperty};
    use sims_vm::program::AnimationAsset;
    let (mut p, mut s) = fixture();
    let obj = s.vm.world.objects.get_mut(&1).unwrap();
    obj.data.insert(17, 0);
    obj.person
        .as_mut()
        .unwrap()
        .extend([(42, 0), (58, 30), (64, 0), (65, 0)]);
    let module = p.modules.get_mut("chair").unwrap();
    module.object_animation_tables.insert(1, 129);
    module.strings.insert(129, vec!["".into(), "test".into()]);
    module.sounds.insert(4, "chair_event".into());
    let mut animate = ins(0x2c, 4, [1, 0, 0, 0, 0, 64, 0, 0]);
    animate.on_false = 3;
    module.routines.insert(
        4097,
        routine(vec![
            ins(0x2d, 1, [0, 0, 2, 0, 0, 0, 0, 0]),
            ins(0x2e, 2, [0, 0, 3, 0, 4, 0, 0, 0]),
            animate,
            ins(0x17, 2, [4, 0, 0, 0, 2, 0, 0, 0]),
            ins(2, 5, [0, 0, 1, 0, 0, 5, 18, 7]),
            ins(0x2c, 6, [0; 8]),
            ins(0x2e, 7, [0, 0, 3, 0, 0, 0, 0, 0]),
            ins(0x31, 254, [0; 8]),
        ]),
    );
    p.modules.get_mut("global").unwrap().strings.insert(
        150,
        vec!["".into(), "idle".into(), "idle".into(), "idle".into()],
    );
    for (name, frames, events) in [
        (
            "test",
            6,
            vec![
                TimedProperty {
                    time_ms: 40,
                    bone: "ROOT".into(),
                    pairs: vec![("xevt".into(), "100".into()), ("xevt".into(), "101".into())],
                },
                TimedProperty {
                    time_ms: 80,
                    bone: "ROOT".into(),
                    pairs: vec![("righthand".into(), "2".into())],
                },
            ],
        ),
        ("idle", 7, vec![]),
    ] {
        p.animations.insert(
            name.into(),
            AnimationAsset {
                source: "synthetic".into(),
                content_hash: "test".into(),
                samples_source: "synthetic samples".into(),
                samples_hash: "test samples".into(),
                clip: Animation {
                    name: name.into(),
                    skill: name.into(),
                    duration_ms: 200.0,
                    distance: 0.0,
                    moving: false,
                    translation_count: 0,
                    rotation_count: 0,
                    frames,
                    motions: vec![],
                    events,
                },
                samples: Samples {
                    translations: vec![],
                    rotations: vec![],
                },
            },
        );
    }
    (p, s)
}
fn to_animation(p: &Program, s: &mut Simulation) {
    loop {
        s.run_fixture_tick(p, 100).unwrap();
        if !s.animations.is_empty() {
            break;
        }
        s.advance_fixture_tick().unwrap();
    }
}

#[test]
fn animation_event_queue_budget_checkpoint_and_idle_loop_resume_exactly() {
    let (p, mut a) = animated_fixture();
    to_animation(&p, &mut a);
    let began = a.vm.tick;
    a.advance_fixture_tick().unwrap();
    assert_eq!(a.run_fixture_tick(&p, 0).unwrap().stop, StepStop::Budget);
    assert_eq!(
        a.animations[&1].pending.iter().copied().collect::<Vec<_>>(),
        [101]
    );
    assert_eq!(a.animations[&1].delivered, 1);
    let mut b =
        Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &a)).unwrap(), &p).unwrap();
    assert_eq!(
        a.run_fixture_tick(&p, 100).unwrap(),
        b.run_fixture_tick(&p, 100).unwrap()
    );
    assert_eq!(a.vm.sounds.len(), 2);
    while a.vm.tick < began + 5 {
        a.advance_fixture_tick().unwrap();
        b.advance_fixture_tick().unwrap();
        assert_eq!(
            a.run_fixture_tick(&p, 100).unwrap(),
            b.run_fixture_tick(&p, 100).unwrap()
        );
        assert_eq!(a, b);
    }
    assert_eq!(a.vm.threads[&2].status, Status::Returned(true));
    assert_eq!(a.animations[&1].clip, "idle");
    assert!(a.animations[&1].looping);
    assert_eq!(a.vm.sounds.len(), 2);
    assert_eq!(a.vm.world.objects[&1].container, Some((2, 0)));
    assert!(a.reservations.is_empty());
    for _ in 0..15 {
        a.advance_fixture_tick().unwrap();
        a.run_fixture_tick(&p, 100).unwrap();
        Snapshot::capture(&p, &a).restore(&p).unwrap();
    }
    assert_eq!(a.animations[&1].frame_fifths, 90 % 35);
}

#[test]
fn animation_clock_budget_splits_do_not_change_completion_or_emit_events_twice() {
    let (p, mut a) = animated_fixture();
    let mut b = a.clone();
    loop {
        let result = a.run_fixture_tick(&p, 100).unwrap();
        while b.run_fixture_tick(&p, 1).unwrap().stop == StepStop::Budget {
            b = Snapshot::capture(&p, &b).restore(&p).unwrap();
        }
        let saved = b.clone();
        b.run_fixture_tick(&p, 1).unwrap();
        assert_eq!(saved, b);
        assert_eq!(a, b);
        if result.stop == StepStop::Complete {
            break;
        }
        a.advance_fixture_tick().unwrap();
        b.advance_fixture_tick().unwrap();
    }
}

#[test]
fn native_numbered_events_yield_on_bad_order_and_supply_missing_tail() {
    let (mut p, mut full) = animated_fixture();
    p.modules
        .get_mut("chair")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions[2]
        .operands[6] = 2;
    p.animations.get_mut("test").unwrap().clip.events[0].pairs = vec![
        ("xevt".into(), "1".into()),
        ("xevt".into(), "0".into()),
        ("xevt".into(), "100".into()),
    ];
    to_animation(&p, &mut full);
    let began = full.vm.tick;
    let mut split = Snapshot::capture(&p, &full).restore(&p).unwrap();
    for offset in 1..=5 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let step = full.run_fixture_tick(&p, 100).unwrap();
        while split.run_fixture_tick(&p, 1).unwrap().stop == StepStop::Budget {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        }
        split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        assert_eq!(full, split);
        if offset == 1 {
            assert_eq!(full.animations[&1].numbered_events, 0);
            assert_eq!(full.animations[&1].pending, [0, 100]);
            let before = full.clone();
            full.run_fixture_tick(&p, 100).unwrap();
            assert_eq!(
                full, before,
                "yield must defer the next queued event to another tick"
            );
        }
        if offset == 5 {
            assert_eq!(step.stop, StepStop::Complete);
        }
    }
    assert_eq!(full.vm.tick, began + 5);
    assert_eq!(
        full.vm.sounds.len(),
        3,
        "two real accepted events plus missing numbered event 1"
    );
}

#[test]
fn animation_missing_assets_unsupported_properties_and_forged_cursors_are_rejected() {
    let (mut p, mut s) = animated_fixture();
    to_animation(&p, &mut s);
    for case in 0..5 {
        let mut a = Snapshot::capture(&p, &s);
        let player = a.state.animations.get_mut(&1).unwrap();
        match case {
            0 => player.frame_fifths = 1000,
            1 => player.next_event = 1,
            2 => player.pending.push_back(10),
            3 => player.started_tick += 1,
            _ => player.speed = u64::MAX,
        };
        assert!(a.restore(&p).is_err());
    }
    let snapshot = Snapshot::capture(&p, &s);
    p.animations.get_mut("test").unwrap().samples_hash = "changed".into();
    assert!(snapshot.restore(&p).is_err());
    let (mut p, mut s) = animated_fixture();
    p.animations.remove("test");
    for _ in 0..150 {
        match s.run_fixture_tick(&p, 100) {
            Err(e) => {
                assert!(e.contains("missing loaded animation"));
                break;
            }
            Ok(_) => s.advance_fixture_tick().unwrap(),
        }
    }
    assert!(!matches!(s.vm.threads[&2].status, Status::Returned(true)));
    let (mut p, mut s) = animated_fixture();
    p.animations.get_mut("test").unwrap().clip.events[0].pairs[0].0 = "anchor".into();
    for _ in 0..150 {
        match s.run_fixture_tick(&p, 100) {
            Err(e) => {
                assert!(e.contains("unsupported animation time property"));
                break;
            }
            Ok(_) => s.advance_fixture_tick().unwrap(),
        }
    }
    assert!(s.animations.is_empty());
}

#[test]
fn hurryable_animation_uses_double_speed_and_registered_idle_thread_wakes_same_tick() {
    let (mut p, mut s) = animated_fixture();
    s.vm.world.objects.get_mut(&1).unwrap().data.insert(17, 1);
    p.modules
        .get_mut("chair")
        .unwrap()
        .routines
        .insert(4100, routine(vec![ins(0, 254, [0; 8])]));
    let idle =
        s.vm.spawn(
            &p,
            sims_vm::Start {
                owner: "chair",
                bhav: 4100,
                caller: 2,
                callee: 2,
                stack_object: 2,
                args: vec![1000],
                check: false,
            },
        )
        .unwrap();
    s.vm.register_object_thread(2, idle).unwrap();
    to_animation(&p, &mut s);
    let began = s.vm.tick;
    assert_eq!(s.animations[&1].speed, 12);
    assert_eq!(finish(&p, &mut s).stop, StepStop::Complete);
    assert_eq!(s.vm.tick, began + 3);
    assert_eq!(s.vm.threads[&idle].status, Status::Returned(true));
    assert!(!s.vm.threads[&idle].interrupted);
    assert_eq!(s.vm.sounds.len(), 2);
}

#[test]
fn locomotion_speed_turns_and_checkpoints_follow_distance_in_eight_directions() {
    use sims_sim::locomotion::Locomotion;
    let p = Program::default();
    // Equal physical distances: 28 subtiles axially vs 20*sqrt(2) diagonally.
    let axial: Vec<_> = (0..=28).map(|i| Point::new(i, 0)).collect();
    let diagonal: Vec<_> = (0..=20).map(|i| Point::new(i, i)).collect();
    let a = Locomotion::plan(&p, &axial, 4, 4).unwrap();
    let b = Locomotion::plan(&p, &diagonal, 8, 8).unwrap();
    assert!(
        a.duration().abs_diff(b.duration()) <= 1,
        "diagonals must not move sqrt(2) times faster"
    );
    let path: Vec<_> = (0..=20)
        .map(|i| Point::new(20 + i, 40 - i))
        .chain((1..=20).map(|i| Point::new(40 + i, 20)))
        .collect();
    let mut movement = Locomotion::plan(&p, &path, 64, 16).unwrap();
    let mut last = movement.sample(&path, 0.);
    for tick in 1..=movement.duration() {
        let old_distance = movement.distance();
        movement.elapsed = tick;
        assert!(movement.distance() - old_distance <= 1000);
        let sample = movement.sample(&path, 0.);
        assert!(
            (sample.heading - last.heading).abs() < 0.25,
            "turn must take multiple ticks"
        );
        let restored: Locomotion =
            serde_json::from_slice(&serde_json::to_vec(&movement).unwrap()).unwrap();
        assert_eq!(restored, movement);
        assert_eq!(
            restored.sample(&path, 0.5).position,
            movement.sample(&path, 0.5).position
        );
        last = sample;
    }
    assert_eq!(last.position, [60., 20.]);
    assert_eq!(movement.facing(), 16);
}

#[test]
fn route_resume_covers_every_movement_phase_and_rejects_forged_timing() {
    use sims_sim::locomotion::Kind;
    let (p, mut sim) = fixture();
    sim.run_fixture_tick(&p, 100).unwrap();
    let route = &sim.routes[&2];
    let checkpoints: Vec<_> = route
        .locomotion
        .phases
        .iter()
        .map(|phase| phase.start_tick + phase.ticks / 2)
        .collect();
    let kinds: Vec<_> = route
        .locomotion
        .phases
        .iter()
        .map(|phase| phase.kind)
        .collect();
    for kind in [Kind::Start, Kind::Walk, Kind::Turn, Kind::Stop] {
        assert!(kinds.contains(&kind));
    }
    for tick in checkpoints {
        while sim.vm.tick < u64::from(tick) {
            sim.advance_fixture_tick().unwrap();
            sim.run_fixture_tick(&p, 100).unwrap();
        }
        let saved = Snapshot::capture(&p, &sim);
        let mut resumed = Snapshot::from_json(&serde_json::to_vec(&saved).unwrap(), &p).unwrap();
        let mut reference = sim.clone();
        while reference.routes.contains_key(&2) {
            reference.advance_fixture_tick().unwrap();
            resumed.advance_fixture_tick().unwrap();
            assert_eq!(
                reference.run_fixture_tick(&p, 100).unwrap(),
                resumed.run_fixture_tick(&p, 100).unwrap()
            );
            assert_eq!(reference, resumed);
        }
        for case in 0..5 {
            let mut bad = saved.clone();
            let r = bad.state.routes.get_mut(&2).unwrap();
            match case {
                0 => r.locomotion.elapsed = u32::MAX,
                1 => r.locomotion.phases[0].ticks += 1,
                2 => r.locomotion.distances[1] += 1,
                3 => r.locomotion.phases[0].clip = "different-clip".into(),
                _ => r.started_tick = u64::MAX,
            }
            assert!(bad.restore(&p).is_err(), "case {case}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_background_layer_start_clear_loop_and_foreground_preservation() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../sims-vm/tests/fixtures/home-extra.json")).unwrap();
    for row in report["backgrounds"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let number = c["number"].as_u64().unwrap() as u16;
        let (mut p, mut s) = animated_fixture();
        let m = p.modules.get_mut("chair").unwrap();
        m.strings.insert(129, vec!["idle".into(); 56]);
        let mut b = [0; 8];
        b[..2].copy_from_slice(&number.to_le_bytes());
        b[5] = 1;
        m.routines
            .insert(4097, routine(vec![ins(44, 1, [0; 8]), ins(44, 254, b)]));
        if c["clip"] == true {
            s.backgrounds.insert(
                1,
                sims_sim::carry::Layer {
                    clip: "idle".into(),
                    started_tick: 0,
                },
            );
        }
        while !s.animations.contains_key(&1) {
            assert_ne!(s.run_fixture_tick(&p, 1).unwrap().stop, StepStop::Faulted);
        }
        let foreground = s.animations[&1].clone();
        loop {
            let r = s.run_fixture_tick(&p, 1).unwrap();
            assert_ne!(r.stop, StepStop::Faulted);
            if r.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(s.backgrounds.contains_key(&1), e["clip"].as_bool().unwrap());
        assert_eq!(s.animations[&1], foreground);
        assert_eq!(e["foreground_preserved"], true);
        assert_eq!(e["returned"], 1);
        if number != 0 {
            assert_eq!(e["loop"], 1);
            assert_eq!(
                e["calls"].as_array().unwrap().last().unwrap(),
                &serde_json::json!(["priority", 4294967286u32])
            );
        }
        assert_eq!(Snapshot::capture(&p, &s).restore(&p).unwrap(), s);
    }
}
#[test]
fn original_failure_feedback_arguments_yield_and_resume_restore_route_result() {
    let (mut p, mut s) = fixture();
    // Block every path; the native route result is 3. Named feedback may sleep
    // and clobber the person's result, but state 17 must restore it afterwards.
    s.room.obstacles[0].min.y = 0;
    s.room.obstacles[0].max.y = 80;
    s.vm.world.objects.get_mut(&1).unwrap().definition = s.vm.world.objects[&2].definition.clone();
    s.vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(72, 17);
    s.vm.world.objects.get_mut(&1).unwrap().temps = [17, 18, 19, 20, 21, 22, 23, 24];
    s.vm.world.objects.get_mut(&2).unwrap().data.insert(62, 0);
    let mut feedback = routine(vec![
        ins(0, 1, [0; 8]),
        ins(2, 255, [62, 0, 99, 0, 0, 5, 18, 7]),
    ]);
    feedback.name = "Route failure feedback".into();
    p.modules
        .get_mut("global")
        .unwrap()
        .routines
        .insert(398, feedback);
    s.vm.world.actions = Some(sims_vm::action::WorldState {
        actors: BTreeMap::from([(1, Default::default())]),
        ..Default::default()
    });
    let mut saw = false;
    let mut slept = false;
    for _ in 0..40 {
        loop {
            let r = s.run_fixture_tick(&p, 1).unwrap();
            assert!(
                !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
                "{:?}",
                s.vm.threads
            );
            if !s.route_feedback.is_empty() {
                if !saw {
                    let call = s.vm.threads[&2].stack.last().unwrap();
                    assert_eq!(call.args, [3, 0, 0, 0]);
                    assert_eq!(call.stack_object, 2);
                    assert_eq!(
                        s.vm.world.objects[&1].temps,
                        [17, 18, 19, 20, 21, 22, 23, 24]
                    );
                    assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&72], 0);
                    assert_eq!(s.vm.world.objects[&2].data[&62], 1);
                }
                saw = true;
                slept |= matches!(s.vm.threads[&2].status, Status::Sleeping { .. });
                s = Snapshot::capture(&p, &s).restore(&p).unwrap();
            }
            if r.stop != StepStop::Budget {
                break;
            }
        }
        if s.vm.threads[&2].status == Status::Returned(false) {
            break;
        }
        s.advance_fixture_tick().unwrap();
    }
    assert!(saw && slept);
    assert_eq!(s.vm.threads[&2].status, Status::Returned(false));
    assert!(s.route_feedback.is_empty());
    assert_eq!(s.vm.world.objects[&1].person.as_ref().unwrap()[&62], 3);
    assert_eq!(s.vm.world.objects[&2].data[&62], 0);
}

#[test]
fn transient_script_direction_is_preserved_and_spatial_cache_wraps_at_budget_boundaries() {
    let (mut p, mut s) = fixture();
    p.modules
        .get_mut("chair")
        .unwrap()
        .routines
        .get_mut(&4097)
        .unwrap()
        .body
        .instructions = vec![ins(2, 1, [1, 0, 10, 0, 0, 5, 3, 7]), ins(0, 254, [0; 8])];
    for _ in 0..8 {
        let r = s.run_fixture_tick(&p, 1).unwrap();
        s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        if s.vm.world.objects[&1].data[&1] == 10 {
            assert_eq!(s.bodies[&1].facing, 4);
            return;
        }
        assert_eq!(r.stop, StepStop::Budget);
    }
    panic!("did not execute raw direction write");
}

#[test]
fn old_bare_thread_snapshots_are_rejected_instead_of_guessing_registry_history() {
    let (p, mut s) = fixture();
    s.run_fixture_tick(&p, 1).unwrap();
    let snapshot = Snapshot::capture(&p, &s);
    assert_eq!(snapshot.version, 56);
    assert_eq!(snapshot.clone().restore(&p).unwrap(), s);
    let mut old = snapshot;
    old.version = 55;
    old.state.fixture_driver = None;
    let error = old.restore(&p).unwrap_err();
    assert!(error.contains("legacy simulation snapshot"));
}
