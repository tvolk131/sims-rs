use sims_formats::{
    animation::{Animation, Samples},
    behavior::{Behavior, Instruction},
};
use sims_sim::{Body, Point, Room, Simulation, Snapshot, StepStop};
use sims_vm::{
    Module, Object, Program, Routine, Start, Vm, World, action, program::AnimationAsset,
};
use std::collections::BTreeMap;
fn fixture(count: u8) -> (Program, Simulation) {
    let p = Program {
        globals: "test".into(),
        modules: BTreeMap::from([(
            "test".into(),
            Module {
                source: "test".into(),
                strings: BTreeMap::from([(156, vec!["unused".into(), "clip".into()])]),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "interruptible".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 1,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 44,
                                on_true: 254,
                                on_false: 0,
                                operands: [1, 0, 0, 0, 3, 8, count, 0],
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        animations: BTreeMap::from([(
            "clip".into(),
            AnimationAsset {
                source: "synthetic".into(),
                content_hash: String::new(),
                samples_source: String::new(),
                samples_hash: String::new(),
                clip: Animation {
                    name: "clip".into(),
                    skill: "clip".into(),
                    duration_ms: 2000.,
                    distance: 0.,
                    moving: false,
                    translation_count: 0,
                    rotation_count: 0,
                    frames: 60,
                    motions: vec![],
                    events: vec![],
                },
                samples: Samples {
                    translations: vec![],
                    rotations: vec![],
                },
            },
        )]),
        ..Default::default()
    };
    let mut w = World::default();
    let mut person: BTreeMap<_, _> = (0..256).map(|i| (i, 0)).collect();
    person.insert(58, 30);
    w.objects.insert(
        1,
        Object {
            person: Some(person),
            motives: Some([0f32.to_bits(); 16]),
            data: BTreeMap::from([(1, 0)]),
            ..Default::default()
        },
    );
    w.actions = Some(action::WorldState {
        live_family: true,
        actors: BTreeMap::from([(1, action::Actor::default())]),
        ..Default::default()
    });
    let mut vm = Vm::new(w, 1);
    vm.spawn(
        &p,
        Start {
            owner: "test",
            bhav: 4096,
            caller: 1,
            callee: 1,
            stack_object: 1,
            args: vec![0],
            check: false,
        },
    )
    .unwrap();
    let s = Simulation::new(
        vm,
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        BTreeMap::from([(
            1,
            Body {
                level: 1,
                position: Point::new(24, 24),
                facing: 1,
                radius: 1,
                elevation: 0,
            },
        )]),
    )
    .unwrap();
    (p, s)
}
#[test]
fn object_animation_requests_use_actor_and_active_icon_ages_without_adult_fallback() {
    for age in [17, 18] {
        for partner_age in [17, 18] {
            for flags in [0, 1, 16, 17] {
                for active_matches in [false, true] {
                    let (mut p, mut s) = fixture(0);
                    let m = p.modules.get_mut("test").unwrap();
                    m.routines.get_mut(&4096).unwrap().body.instructions[0].operands =
                        [1, 0, 0, 0, 0, flags, 0, 0];
                    m.strings = (129..=132)
                        .map(|table| (table, vec!["unused".into(), format!("table-{table}")]))
                        .collect();
                    // OBJD's link does not replace any of the fixed native tables.
                    m.object_animation_tables.insert(1, 200);
                    m.strings
                        .insert(200, vec!["unused".into(), "wrong-table".into()]);
                    let mut words = vec![0; 108];
                    words[9] = 2;
                    m.object_definition_words.insert(1, words);
                    let o = s.vm.world.objects.get_mut(&1).unwrap();
                    o.data.insert(17, 0);
                    o.person.as_mut().unwrap().insert(58, age);
                    o.definition = Some(sims_vm::Definition {
                        source: "test".into(),
                        object: 1,
                        guid: 1,
                        slots: 0,
                    });
                    let mut partner = o.clone();
                    partner.person.as_mut().unwrap().insert(58, partner_age);
                    s.vm.world.objects.insert(2, partner);
                    s.vm.world
                        .actions
                        .as_mut()
                        .unwrap()
                        .actors
                        .get_mut(&1)
                        .unwrap()
                        .active = Some(action::Action {
                        target: if active_matches { 1 } else { 2 },
                        icon_owner: 2,
                        ..Default::default()
                    });
                    s.vm.run_slice(&p, 1, 1).unwrap();
                    let expected = match (
                        age < 18,
                        if flags == 0 && active_matches {
                            partner_age < 18
                        } else {
                            age < 18
                        },
                    ) {
                        (false, false) => 129,
                        (true, true) => 130,
                        (false, true) => 131,
                        (true, false) => 132,
                    };
                    let before = s.clone();
                    assert_eq!(
                        s.pending_animation_assets(&p).unwrap(),
                        std::collections::BTreeSet::from([format!("table-{expected}")])
                    );
                    assert_eq!(s, before);
                    p.modules.get_mut("test").unwrap().strings.remove(&expected);
                    assert_eq!(
                        s.pending_animation_assets(&p).unwrap_err(),
                        "missing object animation table"
                    );
                }
            }
        }
    }
}

#[test]
fn clearing_background_and_carry_layers_requests_no_clip() {
    for flags in [1, 16] {
        let (mut p, mut s) = fixture(0);
        p.modules
            .get_mut("test")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body
            .instructions[0]
            .operands = [0, 0, 0, 0, 3, flags, 0, 0];
        p.animations.clear();
        s.vm.run_slice(&p, 1, 1).unwrap();
        let before = s.clone();
        assert!(s.pending_animation_assets(&p).unwrap().is_empty());
        assert_eq!(s, before);
    }
}

#[test]
fn automatic_carry_asset_suspension_preserves_selector_and_can_resume() {
    let (mut p, mut s) = fixture(0);
    let mut strings = vec![String::new(); 39];
    strings[38] = "CARRY-CLIP".into();
    p.modules
        .get_mut("test")
        .unwrap()
        .strings
        .insert(150, strings);
    s.vm.world.objects.get_mut(&1).unwrap().slots = vec![Some(2)];
    s.vm.world.objects.insert(
        2,
        Object {
            container: Some((1, 0)),
            data: BTreeMap::from([(1, 0)]),
            ..Default::default()
        },
    );
    s.bodies.insert(2, s.bodies[&1].clone());
    for id in [1, 2] {
        s.vm.world.positions.insert(
            id,
            sims_vm::autonomy::Placement {
                x: 24,
                y: 24,
                level: 1,
            },
        );
    }
    let expected = std::collections::BTreeSet::from(["carry-clip".into()]);
    let initial = s.clone();
    assert_eq!(s.pending_animation_assets(&p).unwrap(), expected);
    assert_eq!(s, initial);
    for _ in 0..2 {
        assert_eq!(
            s.run_fixture_tick(&p, 100).unwrap_err(),
            "automatic carry animation not loaded: carry-clip"
        );
        assert_eq!(s, initial);
        s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        assert_eq!(s.pending_animation_assets(&p).unwrap(), expected);
    }
    let mut clip = p.animations["clip"].clone();
    clip.clip.name = "carry-clip".into();
    p.animations.insert("carry-clip".into(), clip);
    let mut eager = initial;
    assert_eq!(s.run_fixture_tick(&p, 100), eager.run_fixture_tick(&p, 100));
    assert_eq!(s, eager);
    assert_eq!(s.carry[&1].selector.selection, 0);
    assert_eq!(s.carry[&1].layer.as_ref().unwrap().clip, "carry-clip");
    assert!(s.pending_animation_assets(&p).unwrap().is_empty());
    assert_eq!(Snapshot::capture(&p, &s).restore(&p).unwrap(), eager);
}

#[test]
fn interruptible_foreground_returns_early_without_teleporting_cursor_and_resumes() {
    for count in [0, 2] {
        let (p, mut full) = fixture(count);
        let mut split = full.clone();
        for tick in 0..10 {
            if tick == 3 {
                for s in [&mut full, &mut split] {
                    s.vm.world
                        .objects
                        .get_mut(&1)
                        .unwrap()
                        .motives
                        .as_mut()
                        .unwrap()[5] = (-100f32).to_bits();
                }
            }
            if tick > 0 {
                full.advance_fixture_tick().unwrap();
                split.advance_fixture_tick().unwrap();
            }
            let whole = full.run_fixture_tick(&p, 100).unwrap();
            let mut trace = vec![];
            loop {
                let part = split.run_fixture_tick(&p, 1).unwrap();
                trace.extend(part.trace);
                split = Snapshot::capture(&p, &split).restore(&p).unwrap();
                if part.stop != StepStop::Budget {
                    break;
                }
            }
            assert_eq!(whole.trace, trace);
            assert!(full == split, "tick {tick}, count {count}");
            if tick >= 3 {
                let playback = &full.animations[&1];
                assert!(playback.completed && !playback.ended);
                assert_eq!(playback.frame_fifths, tick * 6);
                assert_eq!(playback.substitute_events, count);
            }
        }
    }
}

#[test]
fn expression_direction_write_is_visible_to_geometry_at_a_budget_checkpoint() {
    let (mut p, mut sim) = fixture(0);
    p.modules
        .get_mut("test")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![Instruction {
        opcode: 2,
        on_true: 254,
        on_false: 255,
        operands: [1, 0, 6, 0, 0, 5, 3, 7],
    }];
    sim.run_fixture_tick(&p, 1).unwrap();
    assert_eq!(sim.vm.world.objects[&1].data[&1], 6);
    assert_eq!(sim.bodies[&1].facing, 64);
    assert!(Snapshot::capture(&p, &sim).restore(&p).unwrap() == sim);
}

#[test]
fn animation_owner_survives_cross_object_callback_checkpoint() {
    let (mut p, mut s) = fixture(0);
    let m = p.modules.get_mut("test").unwrap();
    m.strings.insert(303, vec!["callback".into()]);
    for (id, name, opcode, operands) in [
        (4097, "sender", 28, [47, 1, 0, 0, 1, 1, 0, 0]),
        (4098, "callback", 2, [0, 0, 0, 0, 0, 2, 7, 7]),
    ] {
        m.routines.insert(
            id,
            Routine {
                name: name.into(),
                body: Behavior {
                    format: 0x8002,
                    kind: 0,
                    args: 0,
                    locals: 0,
                    instructions: vec![Instruction {
                        opcode,
                        operands,
                        on_true: 254,
                        on_false: 255,
                    }],
                },
            },
        );
    }
    let definition = sims_vm::Definition {
        source: "test".into(),
        object: 1,
        guid: 0,
        slots: 0,
    };
    s.vm.world.objects.get_mut(&1).unwrap().definition = Some(definition.clone());
    s.vm.world.objects.insert(
        2,
        Object {
            definition: Some(definition),
            data: BTreeMap::from([(1, 0)]),
            ..Default::default()
        },
    );
    s.bodies.insert(
        2,
        Body {
            position: Point::new(48, 48),
            ..s.bodies[&1].clone()
        },
    );
    s.vm.object_threads.insert(1, 1);
    let source =
        s.vm.spawn(
            &p,
            Start {
                owner: "test",
                bhav: 4097,
                caller: 2,
                callee: 2,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    let mut saved = false;
    for _ in 0..20 {
        let part = s.run_fixture_tick(&p, 1).unwrap();
        if s.vm.named_calls.last().is_some_and(|c| c.result.is_none()) {
            assert!(s.vm.threads[&1].check);
            assert!(!s.animations[&1].completed);
            assert!(s.advance_fixture_tick().is_err());
            saved = true;
        }
        s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        if part.stop != StepStop::Budget {
            break;
        }
    }
    assert!(saved);
    assert_eq!(
        s.vm.threads[&source].status,
        sims_vm::Status::Returned(true)
    );
    assert!(!s.vm.threads[&1].check);
    assert!(matches!(
        s.vm.threads[&1].status,
        sims_vm::Status::AwaitingAnimation(_)
    ));
    assert_eq!(s.animations[&1].frame_fifths, 0);
}
