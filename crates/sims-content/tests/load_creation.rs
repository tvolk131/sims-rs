//! The shared frozen factory must be independent of host instruction budgets.
use serde_json::json;
use sims_formats::behavior::{Behavior, Instruction};
use sims_sim::{
    lot_creation::{OffLot, Request},
    lot_links::Links,
};
use sims_vm::*;
use std::collections::BTreeMap;

#[test]
#[ignore = "requires the original HDSI controller assets"]
fn original_offlot_factory_budget_and_resume() {
    offlot_factory(false);
}

#[test]
#[ignore = "requires the original HDSI controller assets"]
fn original_offlot_factory_nested_update_preserves_scheduled_parent() {
    offlot_factory(true);
}

fn offlot_factory(scheduled_parent: bool) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted")).unwrap();
    let mut p = sims_content::vm::load_program(&index, "HDSI.iff", "Global.iff").unwrap();
    let owner = "synthetic-parent".to_owned();
    let mut operands = [0; 8];
    operands[..4].copy_from_slice(&0xfd4fb8e4_u32.to_le_bytes());
    operands[4] = 6;
    p.modules.insert(
        owner.clone(),
        Module {
            source: owner.clone(),
            routines: BTreeMap::from([(
                4096,
                Routine {
                    name: "create original controller".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 0,
                        instructions: vec![
                            Instruction {
                                opcode: 0x2a,
                                on_true: 1,
                                on_false: 255,
                                operands,
                            },
                            Instruction {
                                opcode: 2,
                                on_true: 254,
                                on_false: 255,
                                operands: [0; 8],
                            },
                        ],
                    },
                },
            )]),
            object_functions: BTreeMap::from([(
                1,
                vec![
                    sims_formats::execution::ObjectFunction {
                        action: 0,
                        condition: 0,
                    },
                    sims_formats::execution::ObjectFunction {
                        action: 4096,
                        condition: 0,
                    },
                ],
            )]),
            ..Default::default()
        },
    );
    let mut world = World::default();
    world.person = Some(person::WorldState {
        object_order: vec![2],
        ..Default::default()
    });
    world.refresh = Some(refresh::RefreshState::default());
    world.objects.insert(
        2,
        Object {
            definition: Some(Definition {
                source: owner.clone(),
                object: 1,
                guid: 7,
                slots: 0,
            }),
            ..Default::default()
        },
    );
    let mut initial = Vm::new(world, 0x51a5cafe);
    initial.tick = 1234;
    let parent = initial
        .restore_object_stack(
            &p,
            2,
            &[loading::SavedFrame {
                owner,
                tree: 4096,
                node: 0,
                stack_object: 0,
                args: vec![],
                locals: vec![],
                primitive_state: 0,
            }],
            128,
            false,
        )
        .unwrap();
    if scheduled_parent {
        initial.world.objects.get_mut(&2).unwrap().idle = Some(idle::IdleState {
            flags: 3,
            ..Default::default()
        });
        initial.begin_scheduled_object_visit(2).unwrap();
    }
    initial.run_slice(&p, parent, 1).unwrap();
    let mut results = vec![];
    for budget in [100_000, 1] {
        let mut vm = initial.clone();
        let mut links = Links {
            size: 2,
            heads: vec![vec![0; 4]; 2],
            next: BTreeMap::from([(1, 999), (2, 0)]),
        };
        let mut factory = OffLot::begin(
            &mut vm,
            &p,
            &mut links,
            parent,
            construction::ObjectDate {
                year: 1997,
                month: 1,
                day: 1,
            },
            [12, 30],
        )
        .unwrap();
        assert_eq!(
            links.next[&1], 0,
            "reused IDs cannot retain a physical successor"
        );
        assert_eq!(vm.world.person.as_ref().unwrap().object_order, [1, 2]);
        let mut boundaries = 0;
        let mut created = vec![];
        while let Some(request) = factory.advance(&mut vm, &p, budget).unwrap() {
            boundaries += 1;
            assert_eq!(vm.tick, initial.tick);
            assert!(matches!(
                vm.threads[&parent].status,
                Status::AwaitingCreation(_)
            ));
            match request {
                Request::Budget => (),
                Request::Created(id) => {
                    created.push(id);
                    assert_eq!(id, 1);
                    assert!(matches!(
                        vm.threads[&vm.object_threads[&id]].status,
                        Status::Sleeping { .. }
                    ));
                    // Save at the observer boundary, before returning the ID to the parent.
                    vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
                }
                request => panic!("unexpected original Init/Main service: {request:?}"),
            }
            if boundaries == 5 {
                vm = Snapshot::capture(&p, &vm).restore(&p).unwrap();
            }
            factory = serde_json::from_value(json!(factory)).unwrap();
        }
        assert_eq!(
            created,
            [1],
            "do not repeat the factory notification on restore"
        );
        assert_eq!(vm.threads[&parent].status, Status::Ready);
        assert_eq!(vm.threads[&parent].stack[0].location.instruction, 1);
        assert_eq!(vm.threads[&parent].stack[0].stack_object, 1);
        if scheduled_parent {
            let visit = vm.scheduled_object_visit.as_ref().unwrap();
            assert_eq!(visit.object(), 2);
            assert_eq!(visit.chain().count(), 1);
            assert!(!visit.finished());
            assert_eq!(vm.world.objects[&1].last_update, Some(initial.tick));
        }
        if budget == 1 {
            assert!(boundaries > 5);
        }
        results.push(vm);
    }
    assert_eq!(results[0], results[1]);
}
