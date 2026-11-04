use super::*;
use crate::{
    Room,
    character::{Controls, Resident},
    head_look, pose,
};
use sims_vm::{Object, autonomy::Placement, person_loading::RoomMembership};
use std::collections::BTreeMap;

struct Person;
impl selection::View for Person {
    fn person_order(&self) -> &[ObjectId] {
        &[]
    }
    fn flags(&self, _: ObjectId) -> Result<u32, String> {
        Ok(0)
    }
    fn script_id(&self, object: ObjectId) -> Result<i16, String> {
        Ok(object)
    }
}

fn suspended() -> (Simulation, Notification, Program) {
    let mut simulation = Simulation::new(
        Vm::new(World::default(), 1),
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        BTreeMap::new(),
    )
    .unwrap();
    simulation.vm.world.person = Some(sims_vm::person::WorldState {
        person_order: vec![105],
        object_order: vec![105],
        ..Default::default()
    });
    simulation.vm.world.globals.insert(20, 0);
    simulation.vm.world.objects.insert(
        105,
        Object {
            person: Some((0..256).map(|field| (field, 0)).collect()),
            data: [(11, 105)].into(),
            native_flags: Some(2),
            ..Default::default()
        },
    );
    simulation.vm.world.positions.insert(
        105,
        Placement {
            x: 16,
            y: 16,
            level: 1,
        },
    );
    simulation.native_lot = Some(crate::lot_runtime::State {
        placement: lot_placement::State {
            links: Links {
                size: 4,
                heads: vec![vec![0; 16]],
                next: BTreeMap::new(),
            },
            bounds: BTreeMap::new(),
            flags: vec![vec![0; 16]],
            spatial_dirty: Some(false),
        },
        membership: RoomMembership::constructed(&simulation.vm.world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: crate::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        movement: None,
        completion: None,
        route_paths: BTreeMap::new(),
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: BTreeMap::new(),
        selected_person: Some(105),
        room_effects: vec![],
        routing_policy: Default::default(),
    });
    let local = pose::LocalBone {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    };
    let matrix = pose::model::Matrix::IDENTITY;
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
        pose::transform::Placement {
            base: [0.; 3],
            previous: [0.; 3],
            offset: [0.; 3],
            dirty: false,
        },
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
    let mut constructor = pose::construction::State::new(0);
    constructor.carry_selection = Some(-1);
    simulation.characters.insert(
        105,
        Resident::new(
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
                actor: 105,
                sound_visible: false,
                selected: true,
                debug_sound: false,
                position_x: 16.,
                appearance_mode: 0,
                snow: false,
                lot_kind: 1,
            },
        ),
    );
    let mut execution = Execution::clear();
    execution.selection = selection::Selection::new(Some(105));
    loop {
        let request = execution.selection.advance(&Person).unwrap().unwrap();
        if request == selection::Request::Notify(105) {
            break;
        }
        execution.selection.acknowledge(request).unwrap();
    }
    execution.external = Some(Request::Notify(105));
    simulation.native_selection = Some(execution);
    simulation.native_selection_broadcasts = true;
    let call = simulation.pending_selection_notification().unwrap().0;
    (simulation, call, Program::default())
}

fn restore(simulation: &Simulation) -> Simulation {
    serde_json::from_slice(&serde_json::to_vec(simulation).unwrap()).unwrap()
}

fn independent_creation() -> BTreeMap<u64, crate::lot_creation::Execution> {
    use sims_formats::behavior::Instruction;
    let (mut program, mut simulation) = crate::lot_runtime::tests::fixture();
    let module = program.modules.get_mut("objects").unwrap();
    let template = module.routines[&4096].clone();
    module.routines.get_mut(&4096).unwrap().body.instructions = vec![Instruction {
        opcode: 42,
        on_true: 254,
        on_false: 255,
        operands: [2, 0, 0, 0, 3, 0, 0, 0],
    }];
    for (number, next) in [(4097, 254), (4098, 0)] {
        let mut routine = template.clone();
        routine.body.instructions = vec![Instruction {
            opcode: 2,
            on_true: next,
            on_false: 255,
            operands: [0, 0, 0, 0, 0, 5, 8, 7],
        }];
        module.routines.insert(number, routine);
    }
    let mut words = module.object_definition_words[&1].clone();
    words[14] = 2;
    module.object_definition_words.insert(2, words);
    let mut functions = module.object_functions[&1].clone();
    functions[0].action = 4097;
    functions[1].action = 4098;
    module.object_functions.insert(2, functions);
    for number in [1, 5, 6] {
        module.named_constants.insert(
            number,
            sims_formats::tuning::Constants {
                version: 1,
                entries: vec![],
            },
        );
    }
    for number in [501, 502, 503, 504] {
        module.strings.insert(number, vec![String::new(); 9]);
    }
    simulation
        .architecture
        .as_mut()
        .unwrap()
        .bind_footprints(&program)
        .unwrap();
    simulation
        .vm
        .world
        .globals
        .extend([(15, 0), (32, 0), (33, 0), (34, 0), (36, 0)]);
    simulation.vm.world.refresh = Some(Default::default());
    simulation
        .enable_household(
            &program,
            crate::clock::LotClock {
                hour: 12,
                day: 1,
                period: 0,
                minute: 0,
                second: 0,
                month: 1,
                year: 1997,
                elapsed_days: 0,
                month_days: 30,
                ticks: 0,
                statistics: [[0; 8]; 6],
            },
        )
        .unwrap();
    let creator = simulation.vm.object_threads[&4];
    simulation
        .vm
        .threads
        .get_mut(&creator)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 1;
    simulation
        .vm
        .world
        .objects
        .get_mut(&4)
        .unwrap()
        .data
        .insert(1, 6);
    for _ in 0..128 {
        let step = simulation.run_fixture_tick(&program, 1).unwrap();
        assert_ne!(step.stop, crate::StepStop::Faulted);
        if !simulation.native_creations.is_empty()
            && simulation.vm.lifecycles.values().any(|lifecycle| {
                lifecycle.definition == 2 && lifecycle.phase == sims_vm::lifecycle::Phase::Main
            })
        {
            assert!(
                simulation
                    .vm
                    .threads
                    .values()
                    .all(|thread| thread.callback.is_none())
            );
            assert!(simulation.person_factories.is_empty());
            assert!(simulation.characters.is_empty());
            let restored = crate::Snapshot::capture(&program, &simulation)
                .restore(&program)
                .unwrap();
            return restored.native_creations;
        }
    }
    panic!("ordinary object creation did not suspend during immediate Main")
}

#[test]
fn selection_notification_view_refresh_rejects_independent_creation_before_binding_and_each_slice()
{
    let creations = independent_creation();
    let (mut simulation, call, program) = suspended();
    simulation.start_selection_notification(call).unwrap();
    let mut before_binding = simulation.clone();
    before_binding.native_creations = creations.clone();
    let before = before_binding.clone();
    assert!(
        before_binding
            .selection_notification_view_refresh(call, 1)
            .is_err()
    );
    assert_eq!(before_binding, before);
    let token = simulation
        .selection_notification_view_refresh(call, 1)
        .unwrap();
    assert!(
        !simulation
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .unwrap()
    );
    simulation.native_creations = creations;
    let before = simulation.clone();
    assert!(
        simulation
            .validate_selection_notification_view_refresh(token)
            .is_err()
    );
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    assert_eq!(simulation, before);
}

#[test]
fn selection_notification_view_refresh_slices_restore_and_keep_parent() {
    let (initial, call, program) = suspended();
    let mut results = vec![];
    for budget in [1, 1000] {
        let mut simulation = initial.clone();
        simulation.start_selection_notification(call).unwrap();
        let token = simulation
            .selection_notification_view_refresh(call, 1)
            .unwrap();
        let mut complete = false;
        for _ in 0..32 {
            complete = simulation
                .refresh_selection_notification_view_person(&program, 105, token, budget)
                .unwrap();
            assert_eq!(
                simulation.pending_selection_notification(),
                Some((call, true))
            );
            simulation = restore(&simulation);
            simulation
                .native_selection
                .as_ref()
                .unwrap()
                .validate(&simulation.vm, &program)
                .unwrap();
            simulation.validate_characters(&program).unwrap();
            simulation
                .validate_selection_notification_view_refresh(token)
                .unwrap();
            if complete {
                break;
            }
            assert!(simulation.complete_selection_notification(call).is_err());
            let request = simulation.characters[&105]
                .character
                .pending_request()
                .cloned();
            let response = match request {
                None => continue,
                Some(pose::refresh::Request::Placement) => {
                    pose::refresh::Response::Placement(pose::model::Matrix::IDENTITY)
                }
                Some(pose::refresh::Request::Geometry { .. }) => pose::refresh::Response::Done,
                other => panic!("unexpected synthetic character service: {other:?}"),
            };
            simulation
                .characters
                .get_mut(&105)
                .unwrap()
                .character
                .acknowledge(response)
                .unwrap();
        }
        assert!(complete);
        simulation.complete_selection_notification(call).unwrap();
        assert!(
            simulation
                .validate_selection_notification_view_refresh(token)
                .is_err()
        );
        results.push(simulation);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn selection_notification_view_refresh_rejects_stale_mismatched_and_absent_parents() {
    let (mut simulation, call, program) = suspended();
    assert!(
        simulation
            .selection_notification_view_refresh(call, 1)
            .is_err()
    );
    assert_eq!(
        simulation
            .refresh_view_person(&program, 105, 1)
            .unwrap_err(),
        "view refresh overlaps selection"
    );
    simulation.start_selection_notification(call).unwrap();
    let unbound = simulation
        .native_selection
        .as_ref()
        .unwrap()
        .notification_view_refresh
        .unwrap();
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, unbound, 1)
            .is_err()
    );
    assert!(
        simulation
            .selection_notification_view_refresh(call, 0)
            .is_err()
    );
    let token = simulation
        .selection_notification_view_refresh(call, 1)
        .unwrap();
    assert!(
        simulation
            .selection_notification_view_refresh(call, 1)
            .is_err()
    );
    for wrong in [
        NotificationViewRefresh {
            caller_serial: 0,
            ..token
        },
        NotificationViewRefresh {
            caller_serial: 2,
            ..token
        },
        NotificationViewRefresh {
            tick: token.tick + 1,
            ..token
        },
        NotificationViewRefresh {
            notification: Notification {
                payload: 106,
                ..call
            },
            ..token
        },
        NotificationViewRefresh {
            notification: Notification {
                owner: NotificationOwner::Selection { target: Some(106) },
                ..call
            },
            ..token
        },
        NotificationViewRefresh {
            notification: Notification {
                owner: NotificationOwner::Deletion {
                    object: 105,
                    depth: 1,
                },
                ..call
            },
            ..token
        },
    ] {
        let before = simulation.clone();
        assert!(
            simulation
                .refresh_selection_notification_view_person(&program, 105, wrong, 1)
                .is_err()
        );
        assert_eq!(simulation, before);
    }
    let mut absent = simulation.clone();
    absent.native_selection = None;
    assert!(
        absent
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    let mut not_started = simulation.clone();
    not_started.native_selection.as_mut().unwrap().started = false;
    assert!(
        not_started
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    let mut orphan = simulation.clone();
    orphan
        .native_selection
        .as_mut()
        .unwrap()
        .notification_view_refresh = None;
    assert!(
        orphan
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    let mut stale = simulation.clone();
    stale.vm.tick += 1;
    assert!(
        stale
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    let mut deleted = simulation.clone();
    deleted.vm.world.objects.remove(&105);
    assert!(
        deleted
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
    simulation.deletion.push(crate::removal::Deletion::new(105));
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .is_err()
    );
}

#[test]
fn selection_notification_view_refresh_rejects_previous_same_tick_caller_lifetime() {
    let (mut simulation, call, program) = suspended();
    let next = simulation.native_selection.clone();
    simulation.start_selection_notification(call).unwrap();
    let previous = simulation
        .selection_notification_view_refresh(call, 1)
        .unwrap();
    simulation.complete_selection_notification(call).unwrap();
    simulation.native_selection = next;
    simulation.start_selection_notification(call).unwrap();
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, previous, 1)
            .is_err()
    );
    let current = simulation
        .selection_notification_view_refresh(call, 2)
        .unwrap();
    assert_eq!(previous.notification(), current.notification());
    assert_ne!(previous, current);
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, previous, 1)
            .is_err()
    );
    assert!(
        !simulation
            .refresh_selection_notification_view_person(&program, 105, current, 0)
            .unwrap()
    );
    simulation = restore(&simulation);
    simulation
        .validate_selection_notification_view_refresh(current)
        .unwrap();
    assert!(
        simulation
            .validate_selection_notification_view_refresh(previous)
            .is_err()
    );
}

#[test]
fn selection_notification_view_refresh_rejects_competitors_and_retains_failure_owner() {
    let (mut simulation, call, program) = suspended();
    simulation.start_selection_notification(call).unwrap();
    let token = simulation
        .selection_notification_view_refresh(call, 1)
        .unwrap();
    for competing in [
        "flags",
        "indicator",
        "placement",
        "refresh",
        "factory",
        "character",
    ] {
        let mut other = simulation.clone();
        let execution = other.native_selection.as_mut().unwrap();
        match competing {
            "flags" => execution.flags = Some(selection::FlagUpdate::new(105, 2)),
            "indicator" => {
                execution.indicators = Some(indicator::Update::new(
                    Some(105),
                    indicator::Entry::Selection,
                ))
            }
            "placement" => {
                execution.placement = Some(lot_placement::live::Execution::new(
                    lot_placement::Operation::remove(105, true),
                ))
            }
            "refresh" => execution.refresh = Some(Default::default()),
            "factory" => {
                execution.factory_root = Some(sims_vm::Definition {
                    source: "unrelated".into(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                })
            }
            "character" => {
                other.characters.get_mut(&105).unwrap().refresh_tick = Some(other.vm.tick)
            }
            _ => unreachable!(),
        }
        let before = other.clone();
        assert!(
            other
                .refresh_selection_notification_view_person(&program, 105, token, 1)
                .is_err()
        );
        assert_eq!(other, before);
    }
    let mut update = simulation.clone();
    update.characters.get_mut(&105).unwrap().evaluated_tick = 1;
    assert_eq!(
        update
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .unwrap_err(),
        "native ordinary person update required before placement refresh"
    );
    assert_eq!(
        update
            .native_selection
            .as_ref()
            .unwrap()
            .notification_view_person,
        Some(105)
    );
    assert_eq!(update.pending_selection_notification(), Some((call, true)));
    assert!(
        !simulation
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .unwrap()
    );
    let before = simulation.clone();
    assert!(
        simulation
            .refresh_selection_notification_view_person(&program, 106, token, 1)
            .is_err()
    );
    assert_eq!(simulation, before);
    simulation
        .vm
        .world
        .objects
        .get_mut(&105)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .remove(&30);
    assert_eq!(
        simulation
            .refresh_selection_notification_view_person(&program, 105, token, 1)
            .unwrap_err(),
        "native censor field absent"
    );
    assert_eq!(
        simulation
            .native_selection
            .as_ref()
            .unwrap()
            .notification_view_person,
        Some(105)
    );
    assert_eq!(
        simulation.pending_selection_notification(),
        Some((call, true))
    );
    assert!(simulation.characters[&105].character.busy());
    assert!(simulation.complete_selection_notification(call).is_err());
    assert_eq!(restore(&simulation), simulation);
}
