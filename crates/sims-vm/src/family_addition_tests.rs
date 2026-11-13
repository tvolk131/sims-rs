use super::*;
use crate::{
    Definition, Object, Program, Start, Status, Vm,
    person::{Neighbor, WorldState},
};
use sims_formats::behavior::{Behavior, Instruction};

const BETTY: u32 = 844195932;
const BOB: u32 = 850010198;

fn world() -> World {
    let mut world = World::default();
    let mut people = WorldState::default();
    people.controlled_family = 1;
    people.neighbor_slots = Some(vec![None; 48]);
    for (neighbor, guid, family) in [(47, BETTY, 0), (48, BOB, 1)] {
        people.neighbor_slots.as_mut().unwrap()[neighbor as usize - 1] = Some(neighbor);
        people.neighbors.insert(
            neighbor,
            Neighbor {
                guid,
                person: [(61, family), (67, 9)].into(),
                ..Default::default()
            },
        );
    }
    for (family, members, guids) in [
        (0, vec![], vec![]),
        (1, vec![48], vec![BOB]),
        (2, vec![], vec![]),
    ] {
        people.families.insert(family, members);
        people.family_metadata.insert(
            family,
            Metadata {
                flags: 15,
                dirty: false,
                friend_count: Some(3),
                member_guids: Some(guids),
            },
        );
    }
    people.lot_zoning.insert(7, 1);
    people.person_order = vec![7, 8, 9];
    for (object, guid, family, visitor) in [(7, BOB, 1, 0), (8, BETTY, 0, 5), (9, BETTY, 0, 7)] {
        world.objects.insert(
            object,
            Object {
                definition: Some(Definition {
                    source: "family".into(),
                    object: object as u16,
                    guid,
                    slots: 0,
                }),
                person: Some([(32, visitor), (61, family), (67, 9)].into()),
                ..Default::default()
            },
        );
    }
    world.objects.insert(47, Object::default());
    world.globals.insert(10, 7);
    world.person = Some(people);
    world
}

#[derive(Default)]
struct Observer {
    closed: usize,
    visits: Vec<(Vec<i16>, i16, i16)>,
}

impl Host for Observer {
    fn close_panel(&mut self, _world: &mut World) -> Result<(), String> {
        self.closed += 1;
        Ok(())
    }
    fn residential(&mut self, _world: &World) -> Result<bool, String> {
        Ok(true)
    }
    fn observe(&mut self, stage: Stage, world: &World) {
        if matches!(stage, Stage::Visitors) {
            let people = world.person.as_ref().unwrap();
            self.visits.push((
                people.families[&people.controlled_family].clone(),
                people.neighbors[&47].person[&61],
                world.objects[&8].person.as_ref().unwrap()[&61],
            ));
        }
    }
    fn broadcast(&mut self, world: &mut World, change: Change) -> Result<(), String> {
        world.person.as_mut().unwrap().family_changes.push(change);
        Ok(())
    }
}

#[test]
fn native_family_addition_uses_neighbor_guid_and_refreshes_before_person_words() {
    let mut world = world();
    let objects = world.objects.len();
    let unrelated = world.objects[&47].clone();
    let mut host = Observer::default();
    assert!(add(&mut world, 47, &mut host).unwrap());
    let people = world.person.as_ref().unwrap();
    assert_eq!(host.closed, 0);
    assert_eq!(host.visits, vec![(vec![48, 47], 0, 0)]);
    assert_eq!(people.families[&1], vec![48, 47]);
    assert_eq!(
        people.family_metadata[&1].member_guids,
        Some(vec![BOB, BETTY])
    );
    assert_eq!(
        (
            people.family_metadata[&1].flags,
            people.family_metadata[&1].dirty
        ),
        (7, true)
    );
    assert_eq!(people.family_metadata[&1].friend_count, Some(3));
    assert_eq!(
        (
            people.neighbors[&47].person[&61],
            people.neighbors[&47].person[&67]
        ),
        (1, 0)
    );
    assert_eq!(
        (
            world.objects[&8].person.as_ref().unwrap()[&61],
            world.objects[&8].person.as_ref().unwrap()[&67]
        ),
        (1, 0)
    );
    assert_eq!(world.objects[&9].person.as_ref().unwrap()[&61], 0);
    assert_eq!(world.objects[&9].person.as_ref().unwrap()[&67], 9);
    assert_eq!(world.objects[&8].person.as_ref().unwrap()[&32], 0);
    assert_eq!(world.objects[&9].person.as_ref().unwrap()[&32], 0);
    assert_eq!(world.objects[&47], unrelated);
    assert_eq!(world.objects.len(), objects);
    assert_eq!(
        people.family_changes,
        vec![Change {
            family: 1,
            neighbor: 47,
            guid: BETTY
        }]
    );
}

#[test]
fn native_family_addition_transfers_and_reorders_without_closing_the_panel() {
    for previous in [1, 2] {
        let mut world = world();
        let people = world.person.as_mut().unwrap();
        people
            .neighbors
            .get_mut(&47)
            .unwrap()
            .person
            .insert(61, previous);
        people.families.get_mut(&previous).unwrap().insert(0, 47);
        people
            .family_metadata
            .get_mut(&previous)
            .unwrap()
            .member_guids
            .as_mut()
            .unwrap()
            .insert(0, BETTY);
        world
            .objects
            .get_mut(&8)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(61, previous);
        let mut host = Observer::default();
        assert!(add(&mut world, 47, &mut host).unwrap());
        let people = world.person.as_ref().unwrap();
        assert_eq!(host.closed, 0);
        assert_eq!(people.families[&1], vec![48, 47]);
        assert!(people.families[&2].is_empty());
        assert_eq!(
            people.family_changes,
            vec![
                Change {
                    family: previous,
                    neighbor: 47,
                    guid: BETTY
                },
                Change {
                    family: 1,
                    neighbor: 47,
                    guid: BETTY
                },
            ]
        );
    }
}

#[test]
fn native_family_addition_rejects_missing_slots_and_failed_membership_without_writes() {
    for neighbor in [-1, 0, 46, 49] {
        let mut world = world();
        let before = world.clone();
        let mut host = Observer::default();
        assert!(
            add(&mut world, neighbor, &mut host)
                .unwrap_err()
                .contains("native error 0x37")
        );
        assert_eq!(world, before);
        assert_eq!(host.closed, 0);
    }
    for variant in 0..3 {
        let mut world = world();
        let people = world.person.as_mut().unwrap();
        match variant {
            0 => {
                people.controlled_family = 99;
            }
            1 => {
                people.families.get_mut(&1).unwrap().push(47);
                people
                    .family_metadata
                    .get_mut(&1)
                    .unwrap()
                    .member_guids
                    .as_mut()
                    .unwrap()
                    .push(BETTY);
            }
            _ => {
                people.neighbors.get_mut(&47).unwrap().person.insert(61, 2);
            }
        }
        let before = world.clone();
        let mut host = Observer::default();
        assert!(!add(&mut world, 47, &mut host).unwrap());
        assert_eq!(world, before);
        assert_eq!(host.closed, 0);
    }
}

#[test]
fn native_family_addition_accepts_real_family_zero_and_no_live_person() {
    let mut world = world();
    let people = world.person.as_mut().unwrap();
    people.controlled_family = 0;
    people.person_order.retain(|object| *object == 7);
    world.objects.remove(&8);
    world.objects.remove(&9);
    let mut expected_objects = world.objects.clone();
    expected_objects
        .get_mut(&7)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(32, 1);
    assert!(add(&mut world, 47, &mut Live).unwrap());
    assert_eq!(world.person.as_ref().unwrap().families[&0], vec![47]);
    assert_eq!(world.person.as_ref().unwrap().neighbors[&47].person[&61], 0);
    assert_eq!(world.objects, expected_objects);
}

#[test]
fn native_family_addition_primitive_branches_and_cold_restores_without_creation_or_rng() {
    let mut module = crate::Module {
        source: "family".into(),
        ..Default::default()
    };
    module.routines.insert(
        4096,
        crate::Routine {
            name: "add family".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 1,
                    on_true: 254,
                    on_false: 255,
                    operands: [4, 0, 0, 0, 0, 0, 0, 0],
                }],
            },
        },
    );
    let program = Program {
        modules: [("family".into(), module)].into(),
        ..Default::default()
    };
    let mut vm = Vm::new(world(), 12345);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "family",
                bhav: 4096,
                caller: 7,
                callee: 7,
                stack_object: 47,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    let mut restored = crate::Snapshot::capture(&program, &vm)
        .restore(&program)
        .unwrap();
    let objects = vm.world.objects.len();
    assert_eq!(
        vm.run_slice(&program, thread, 1).unwrap(),
        restored.run_slice(&program, thread, 1).unwrap()
    );
    assert_eq!(vm, restored);
    assert_eq!(vm.threads[&thread].status, Status::Returned(true));
    assert_eq!(vm.world.objects.len(), objects);
    assert_eq!((vm.tick, vm.rng_state), (0, 12345));
    let saved = serde_json::to_vec(&crate::Snapshot::capture(&program, &vm)).unwrap();
    assert_eq!(
        vm,
        serde_json::from_slice::<crate::Snapshot>(&saved)
            .unwrap()
            .restore(&program)
            .unwrap()
    );
}
