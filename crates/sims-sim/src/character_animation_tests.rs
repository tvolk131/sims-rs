use crate::{
    Room, Simulation,
    character::{Controls, Resident},
    head_look, pose,
};
use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::{Bone, Skeleton},
    behavior::{Behavior, Instruction},
};
use sims_vm::{
    AnimationRequest, Definition, Module, Object, Program, Routine, Start, Status, Vm, World,
    program::AnimationAsset,
};
use std::collections::BTreeMap;

#[test]
fn native_command_defers_dirty_profile_without_consuming_animation_state() {
    let (mut s, p, id, _) = fixture(0);
    let Status::AwaitingAnimation(r) = s.vm.threads[&id].status.clone() else {
        panic!()
    };
    s.characters.get_mut(&1).unwrap().profile_dirty = true;
    let before = s.clone();
    assert!(s.native_command_profile(&p, id, &r).unwrap());
    assert!(s.pending_animation_assets(&p).unwrap().is_empty());
    assert_eq!(
        s, before,
        "resource inspection must not consume profile invalidation"
    );
    s.defer_skill_profile(
        1,
        crate::character_profile::Origin::Command {
            thread: id,
            request: r.clone(),
        },
    )
    .unwrap();
    assert!(!s.characters[&1].profile_dirty);
    assert_eq!(s.pending_skill_profiles(), [(1, false)]);
    s.validate_skill_profiles().unwrap();
    assert!(s.advance_fixture_tick().is_err());
    let restored: Simulation = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(s, restored);
    s.begin_skill_profile(1).unwrap();
    assert!(s.begin_skill_profile(1).is_err());
    s.complete_skill_profile(1).unwrap();
    assert!(!s.native_command_profile(&p, id, &r).unwrap());
    assert_eq!(
        s.pending_animation_assets(&p).unwrap(),
        ["test".to_owned()].into()
    );
    assert_eq!(s.characters[&1].character, before.characters[&1].character);
}

#[test]
fn native_command_reset_without_skill_lookup_leaves_profile_dirty() {
    let (mut s, mut p, id, _) = fixture(0);
    let mut r = match s.vm.threads[&id].status.clone() {
        Status::AwaitingAnimation(r) => r,
        _ => panic!(),
    };
    r.operands[..2].fill(0);
    p.modules
        .get_mut("person")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions[0]
        .operands = r.operands;
    s.vm.threads.get_mut(&id).unwrap().status = Status::AwaitingAnimation(r.clone());
    s.characters.get_mut(&1).unwrap().profile_dirty = true;
    assert!(!s.native_command_profile(&p, id, &r).unwrap());
    assert!(s.service_animation(&p, id, &r, &mut vec![]).unwrap());
    assert!(s.characters[&1].profile_dirty);
}

pub(crate) fn fixture(flags: u8) -> (Simulation, Program, u64, AnimationAsset) {
    let source = Skeleton {
        name: "test".into(),
        bones: vec![Bone {
            name: "ROOT".into(),
            parent: None,
            translation: [0.; 3],
            rotation: [0., 0., 0., 1.],
            properties: vec![],
            flags: [0; 3],
            wiggle: Default::default(),
        }],
    };
    let clip = Animation {
        name: "test".into(),
        skill: "test".into(),
        duration_ms: 1000.,
        distance: 0.,
        moving: false,
        translation_count: 0,
        rotation_count: 0,
        frames: 30,
        events: vec![],
        motions: vec![Motion {
            bone: "ROOT".into(),
            frames: 30,
            duration_ms: 1000.,
            translation: None,
            rotation: None,
            properties: vec![],
            event_lists: vec![],
        }],
    };
    let asset = AnimationAsset {
        source: "test".into(),
        content_hash: "test".into(),
        samples_source: "test".into(),
        samples_hash: "test".into(),
        clip,
        samples: Samples {
            translations: vec![],
            rotations: vec![],
        },
    };
    let mut program = Program {
        globals: "person".into(),
        ..Program::default()
    };
    program.modules.insert(
        "person".into(),
        Module {
            source: "person".into(),
            object_definition_words: [(1, vec![0; 103])].into(),
            strings: [
                (150, vec!["test".into(); 40]),
                (156, vec!["test".into(); 3]),
            ]
            .into(),
            routines: [(
                4096,
                Routine {
                    name: "animation".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 1,
                        locals: 2,
                        instructions: vec![Instruction {
                            opcode: 0x2c,
                            on_true: 254,
                            on_false: 0,
                            operands: [2, 0, 1, 0, 3, flags, 2, 0],
                        }],
                    },
                },
            )]
            .into(),
            ..Module::default()
        },
    );
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            definition: Some(Definition {
                source: "person".into(),
                object: 1,
                guid: 1,
                slots: 0,
            }),
            person: Some(
                (0..256)
                    .map(|key| (key, if key == 58 { 30 } else { 0 }))
                    .collect(),
            ),
            ..Object::default()
        },
    );
    let mut vm = Vm::new(world, 1);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "person",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![2],
                check: false,
            },
        )
        .unwrap();
    vm.run_round(&program, 1);
    let mut simulation = Simulation::from_placements(
        vm,
        Room {
            width: 16,
            height: 16,
            obstacles: vec![],
        },
        BTreeMap::new(),
        [1].into(),
        None,
    )
    .unwrap();
    let local = pose::LocalBone {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    };
    let matrix = pose::model::Matrix::IDENTITY;
    let mut constructor = pose::construction::State::new(0);
    constructor.carry_selection = Some(-1);
    let character = pose::refresh::Character::new(
        pose::live::Live::new(
            pose::animator::Animator::new(0, 0, 0, false, -1),
            vec![pose::sampling::BoneInput {
                local,
                priority: 0,
                blend_marker: 0,
            }],
        ),
        pose::refresh::Hierarchy {
            parents: vec![None],
            absolute: vec![head_look::rig::AbsoluteBone {
                translation: local.translation,
                rotation: local.rotation,
            }],
            updates: vec![0],
            evaluated_generation: 0,
        },
        head_look::State {
            last_clock: 0,
            basis_heading: 0.,
            target_heading: 0.,
            turn: 0.,
            target_point: Some([0.; 3]),
        },
        head_look::rig::Rig {
            head: None,
            hand: None,
            pelvis: None,
            forward: None,
            turn_bone: None,
        },
        matrix.values,
        constructor.placement.clone(),
        pose::model::Model {
            placement: matrix,
            scale: matrix,
            bounds_transform: matrix,
            bounds_flags: 0,
        },
        pose::refresh::Censor {
            binding: None,
            mask: 0,
        },
        0,
    )
    .unwrap();
    let mut resident = Resident::new(
        character,
        0,
        Controls {
            ordinary: constructor.prefix().unwrap(),
            movement: pose::movement::Movement::new(constructor.movement),
            settings: pose::animator::Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            movement_anchors: [None; 2],
            geometry_generation: 0,
            animation_events: 0,
            engaged: 0,
        },
        pose::properties::Context {
            actor: 1,
            sound_visible: false,
            selected: false,
            debug_sound: false,
            position_x: 0.,
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        },
    );
    resident.source = Some(source);
    simulation.characters.insert(1, resident);
    (simulation, program, thread, asset)
}
fn request(s: &Simulation, id: u64) -> AnimationRequest {
    let Status::AwaitingAnimation(r) = &s.vm.threads[&id].status else {
        panic!("animation not waiting")
    };
    r.clone()
}
fn restore(s: &Simulation) -> Simulation {
    serde_json::from_slice(&serde_json::to_vec(s).unwrap()).unwrap()
}

fn confirmed_absence_fixture() -> (Simulation, Program, u64, AnimationAsset) {
    let (simulation, mut program, thread, mut fallback) = fixture(0);
    program
        .modules
        .get_mut("person")
        .unwrap()
        .strings
        .get_mut(&150)
        .unwrap()[37] = "fallback".into();
    program
        .native_animation_absences
        .insert("test".into(), "a".repeat(64));
    fallback.clip.name = "fallback".into();
    fallback.clip.skill = "fallback".into();
    (simulation, program, thread, fallback)
}

#[test]
fn native_command_unknown_clip_still_requires_an_asset_without_mutation() {
    let (mut simulation, mut program, thread, _) = fixture(0);
    program
        .native_animation_absences
        .insert("unrelated".into(), "a".repeat(64));
    let request = request(&simulation, thread);
    let before = simulation.clone();
    assert_eq!(
        simulation
            .pending_native_command_animation_assets(&program)
            .unwrap(),
        ["test".into()].into()
    );
    assert!(
        simulation
            .pending_required_animation_assets(&program)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        simulation.pending_animation_assets(&program).unwrap(),
        ["test".into()].into()
    );
    let mut events = Vec::new();
    assert!(
        simulation
            .service_animation(&program, thread, &request, &mut events)
            .unwrap_err()
            .contains("asset needs loading")
    );
    assert!(events.is_empty());
    assert_eq!(simulation, before);
}

#[test]
fn native_command_confirmed_absence_retries_real_fallback_transactionally() {
    let (mut simulation, mut program, thread, fallback) = confirmed_absence_fixture();
    let request = request(&simulation, thread);
    let resident = simulation.characters.get_mut(&1).unwrap();
    resident
        .character
        .skin
        .animator
        .start(
            pose::animator::Slot::Foreground,
            resident.source.as_ref().unwrap(),
            Some(&fallback.clip),
            false,
            0,
            resident.controls.settings,
        )
        .unwrap();
    let before = simulation.clone();
    for _ in 0..2 {
        assert_eq!(
            simulation
                .pending_native_command_animation_assets(&program)
                .unwrap(),
            ["fallback".into()].into()
        );
        assert!(
            simulation
                .pending_required_animation_assets(&program)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            simulation.pending_animation_assets(&program).unwrap(),
            ["fallback".into()].into()
        );
        let mut events = Vec::new();
        assert!(
            simulation
                .service_animation(&program, thread, &request, &mut events)
                .unwrap_err()
                .contains("asset needs loading")
        );
        assert!(events.is_empty());
        assert_eq!(simulation, before);
        simulation = restore(&simulation);
    }
    program.animations.insert("fallback".into(), fallback);
    let mut resumed = restore(&simulation);
    for current in [&mut simulation, &mut resumed] {
        let mut events = Vec::new();
        assert!(
            current
                .pending_animation_assets(&program)
                .unwrap()
                .is_empty()
        );
        assert!(
            !current
                .service_animation(&program, thread, &request, &mut events)
                .unwrap()
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].thread, thread);
        assert_eq!(events[0].detail, "missing native Foreground animation tree");
        assert_eq!(
            current.vm.threads[&thread].status,
            Status::AwaitingAnimation(request.clone())
        );
        let resident = &current.characters[&1];
        let animator = &resident.character.skin.animator;
        let foreground = animator.slot(pose::animator::Slot::Foreground).unwrap();
        assert_eq!(animator.layer(foreground).unwrap().name, "fallback");
        assert!(!animator.finished(pose::animator::Slot::Foreground).unwrap());
        assert_eq!(resident.controls.engaged, 1);
        assert!(current.animations.is_empty());
        let waiting = current.clone();
        assert!(
            !current
                .service_animation(&program, thread, &request, &mut events)
                .unwrap()
        );
        assert_eq!(events.len(), 1);
        assert_eq!(*current, waiting);
    }
    assert_eq!(simulation, resumed);
}

#[test]
fn native_command_loaded_clip_wins_over_absence_proof() {
    let (mut simulation, mut program, thread, asset) = fixture(0);
    let request = request(&simulation, thread);
    program
        .native_animation_absences
        .insert("test".into(), "a".repeat(64));
    program.animations.insert("test".into(), asset);
    assert!(
        simulation
            .pending_animation_assets(&program)
            .unwrap()
            .is_empty()
    );
    let mut events = Vec::new();
    assert!(
        !simulation
            .service_animation(&program, thread, &request, &mut events)
            .unwrap()
    );
    assert!(events.is_empty());
    let animator = &simulation.characters[&1].character.skin.animator;
    let foreground = animator.slot(pose::animator::Slot::Foreground).unwrap();
    assert_eq!(animator.layer(foreground).unwrap().name, "test");
    assert_eq!(
        simulation.vm.threads[&thread].status,
        Status::AwaitingAnimation(request)
    );
}

#[test]
fn native_command_absence_does_not_exempt_legacy_or_automatic_carry_assets() {
    let (mut simulation, mut program, thread, _) = confirmed_absence_fixture();
    simulation.characters.clear();
    simulation.vm.world.objects.get_mut(&1).unwrap().slots = vec![Some(2)];
    simulation.vm.world.objects.insert(2, Object::default());
    program
        .modules
        .get_mut("person")
        .unwrap()
        .strings
        .get_mut(&150)
        .unwrap()[38] = "carry".into();
    program
        .native_animation_absences
        .insert("carry".into(), "a".repeat(64));
    let request = request(&simulation, thread);
    let before = simulation.clone();
    let expected: std::collections::BTreeSet<String> = ["carry".into(), "test".into()].into();
    assert!(
        simulation
            .pending_native_command_animation_assets(&program)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        simulation
            .pending_required_animation_assets(&program)
            .unwrap(),
        expected
    );
    assert_eq!(
        simulation.pending_animation_assets(&program).unwrap(),
        expected
    );
    let mut events = Vec::new();
    assert_eq!(
        simulation
            .service_animation(&program, thread, &request, &mut events)
            .unwrap_err(),
        "missing loaded animation test"
    );
    assert_eq!(
        simulation.sync_carry(&program).unwrap_err(),
        "automatic carry animation not loaded: carry"
    );
    assert!(events.is_empty());
    assert_eq!(simulation, before);
}

#[test]
fn native_command_asset_retry_and_event_continuations_survive_restore() {
    let (mut s, mut p, id, asset) = fixture(32);
    let r = request(&s, id);
    let before = s.clone();
    assert_eq!(
        s.pending_animation_assets(&p).unwrap(),
        ["test".into()].into()
    );
    assert_eq!(s, before, "asset query mutated native state");
    assert!(s.service_animation(&p, id, &r, &mut vec![]).is_err());
    assert_eq!(s, before);
    p.animations.insert("test".into(), asset);
    assert!(!s.service_animation(&p, id, &r, &mut vec![]).unwrap());
    assert_eq!(s.characters[&1].controls.engaged, 1);
    let before = s.clone();
    assert!(!s.service_animation(&p, id, &r, &mut vec![]).unwrap());
    assert_eq!(s, before);
    assert!(s.animations.is_empty() && s.carry.is_empty() && s.backgrounds.is_empty());
    // The bridge is exercised with an explicit completed skin pass; the lot's
    // ordinary-update scheduler is independently required and is not bypassed.
    s.vm.tick = 40;
    let resident = s.characters.get_mut(&1).unwrap();
    resident
        .character
        .skin
        .begin(2000, true, false, false)
        .unwrap();
    let person =
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
    assert!(
        resident
            .character
            .skin
            .next(&p, person, resident.context)
            .unwrap()
            .is_none()
    );
    let mut resumed = restore(&s);
    for expected in [0, 1] {
        for current in [&mut s, &mut resumed] {
            let r = request(current, id);
            assert!(current.service_animation(&p, id, &r, &mut vec![]).unwrap());
            assert_eq!(
                current.vm.threads[&id].stack.last().unwrap().locals[1],
                expected
            );
            current.vm.run_round(&p, 1);
        }
        assert_eq!(s, resumed);
        resumed = restore(&resumed);
    }
    for current in [&mut s, &mut resumed] {
        let r = request(current, id);
        assert!(current.service_animation(&p, id, &r, &mut vec![]).unwrap());
        assert_eq!(current.vm.threads[&id].status, Status::Returned(true));
        assert_eq!(current.characters[&1].controls.engaged, 0);
        assert_eq!(current.characters[&1].controls.ordinary.mode, 3);
        assert!(
            current.characters[&1]
                .character
                .skin
                .animator
                .slot(pose::animator::Slot::Base)
                .is_some()
        );
    }
    assert_eq!(s, resumed);
}

#[test]
fn native_resource_owner_uses_action_record_or_indirect_stack_object() {
    let (mut s, mut p, id, _) = fixture(0);
    let owner = s.vm.world.objects[&1].clone();
    s.vm.world.objects.insert(
        2,
        Object {
            definition: Some(Definition {
                source: "chair".into(),
                object: 2,
                guid: 2,
                slots: 0,
            }),
            ..owner
        },
    );
    p.modules.insert(
        "chair".into(),
        Module {
            source: "chair".into(),
            strings: [(129, vec!["chair".into(); 3])].into(),
            ..Module::default()
        },
    );
    p.modules.get_mut("person").unwrap().strings.extend([
        (129, vec!["person".into(); 3]),
        (128, vec!["global-one".into(); 3]),
        (130, vec!["global-two".into(); 3]),
    ]);
    let mut r = request(&s, id);
    r.operands[4] = 0;
    assert_eq!(
        s.command_name(&p, id, &r, 2, pose::command::Lookup::Operand)
            .unwrap()
            .as_deref(),
        Some("person")
    );
    s.vm.threads
        .get_mut(&id)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .action_target = Some(2);
    assert_eq!(
        s.command_name(&p, id, &r, 2, pose::command::Lookup::Operand)
            .unwrap()
            .as_deref(),
        Some("chair")
    );
    r.operands[5] = 4;
    assert_eq!(
        s.command_name(&p, id, &r, 2, pose::command::Lookup::Operand)
            .unwrap()
            .as_deref(),
        Some("person")
    );
    for (source, name) in [(1, "global-one"), (2, "global-two"), (3, "test")] {
        r.operands[4] = source;
        assert_eq!(
            s.command_name(&p, id, &r, 2, pose::command::Lookup::Operand)
                .unwrap()
                .as_deref(),
            Some(name)
        );
    }
}

#[test]
fn native_command_gate_precedes_skeleton_and_indirect_operand_reads() {
    let (mut s, p, id, _) = fixture(4);
    s.characters.get_mut(&1).unwrap().controls.ordinary.mode = 4;
    s.characters.get_mut(&1).unwrap().source = None;
    // Operand 2 is outside this one-argument frame, but a blocked animator
    // yields before even reading that operand in the original routine.
    let r = request(&s, id);
    assert!(s.pending_animation_assets(&p).unwrap().is_empty());
    assert!(!s.service_animation(&p, id, &r, &mut vec![]).unwrap());
    assert_eq!(s.characters[&1].controls.engaged, 0);
    assert_eq!(s.characters[&1].animation_yield, Some((id, r.id, 0)));
}
