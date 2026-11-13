//! Nonempty native animation polling at the scoped slice, not a complete household restore proof.
use super::*;
use sims_formats::{
    animation::{Animation, Motion, Samples},
    avatar::{Bone, Skeleton},
};
use sims_sim::{
    character::{Controls, Resident},
    head_look, pose,
};
use sims_vm::{Start, program::AnimationAsset};

fn poll_fixture(flags: u8) -> (Simulation, Program, u64, AnimationAsset) {
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
    simulation.vm.register_object_thread(1, thread).unwrap();
    simulation.vm.world.person = Some(sims_vm::person::WorldState {
        object_order: vec![1],
        person_order: vec![1],
        ..Default::default()
    });
    simulation.vm.world.scheduler_storage = Some(Default::default());
    simulation
        .vm
        .world
        .globals
        .extend([(10, 1), (17, 0), (19, 2)]);
    let mut owner = simulation.vm.world.objects.remove(&1).unwrap();
    owner.data.extend((0..72).map(|field| (field, 0)));
    owner.native_tree_error = Some(0);
    owner.native_flags = Some(0x240);
    simulation
        .vm
        .world
        .register_scheduler_object(1, &mut owner)
        .unwrap();
    owner.idle.as_mut().unwrap().flags = 3;
    simulation.vm.world.objects.insert(1, owner);
    simulation.vm.lot_rng_state = Some(1825128472);
    simulation.vm.crt_rng_state = Some(23);
    simulation.enable_registry_scheduler(0).unwrap();
    (simulation, program, thread, asset)
}
#[test]
fn nonempty_native_host_polls_and_trace_are_unchanged_by_runtime_scoped_observation() -> Result<()>
{
    let (simulation, mut program, thread, asset) = poll_fixture(0);
    program.animations.insert("test".into(), asset);
    let mut session = session()?;
    session.program = program;
    session.simulation = simulation;
    let mut ordinary = session.simulation.clone();
    let mut ordinary_polls = vec![];
    let expected = ordinary
        .run_current_tick_with_host_polls(&session.program, 1000, &mut ordinary_polls)
        .map_err(anyhow::Error::msg)?;
    let mut observed_polls = vec![];
    let mut cuts = vec![];
    let expected_envelope = session.save();
    let actual = with_view(
        &mut session,
        &mut |cut, view| {
            let ServiceCut::AwaitingAnimation {
                thread: owner,
                request,
                ..
            } = cut
            else {
                panic!("unexpected animation poll cut: {cut:?}");
            };
            assert_eq!(owner, thread);
            assert!(matches!(&view.simulation.vm.threads[&thread].status,
                Status::AwaitingAnimation(owner) if owner.id == request));
            view.simulation
                .native_scheduler
                .as_ref()
                .unwrap()
                .validate(&view.simulation)
                .map_err(anyhow::Error::msg)?;
            let mut expected_saved = expected_envelope.clone();
            expected_saved.simulation = Snapshot::capture(&view.program, &view.simulation);
            assert_eq!(bytes(&view.save())?, bytes(&expected_saved)?);
            cuts.push(cut);
            Ok(())
        },
        |program, simulation, observe| {
            simulation
                .run_current_tick_with_host_polls_and_service_observer(
                    program,
                    1000,
                    Some(&mut observed_polls),
                    &mut |cut, simulation| {
                        let before = bytes(simulation).map_err(|error| format!("{error:#}"))?;
                        observe(cut, simulation).map_err(|error| format!("{error:#}"))?;
                        assert_eq!(bytes(simulation).unwrap(), before);
                        Ok(())
                    },
                )
                .map_err(anyhow::Error::msg)
        },
    )?;
    assert_eq!(actual, expected);
    assert_eq!(session.simulation, ordinary);
    assert_eq!(cuts.len(), 1);
    assert_eq!(ordinary_polls.len(), 1);
    let reports = |polls: Vec<sims_sim::HostPollTrace>| {
        polls
            .into_iter()
            .map(|poll| (poll.before_instruction, poll.instruction))
            .collect::<Vec<_>>()
    };
    assert_eq!(reports(observed_polls), reports(ordinary_polls));
    Ok(())
}
