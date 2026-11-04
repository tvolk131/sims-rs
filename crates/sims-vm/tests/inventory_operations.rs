//! Original x86 component vectors on authored records, not full-lot tick parity.
//! No executable bytes/assets; the query's existing person_services corpus stays independent.
use serde::Deserialize;
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    person::{InventoryEntry, Neighbor, WorldState},
    *,
};
use std::collections::BTreeMap;

const OWNER_GUID: u32 = 0x7fd96b54;
#[derive(Debug, Deserialize)]
struct Input {
    entries: Vec<InventoryEntry>,
    flags: u16,
    token_kind: u8,
    guid: u32,
    temps: Option<[i16; 8]>,
}
#[derive(Deserialize)]
struct Original {
    error: Option<u32>,
    returned: bool,
    caller_inventory: Vec<InventoryEntry>,
    selected_inventory: Vec<InventoryEntry>,
    temps: [i16; 8],
    dirty_word: u32,
}
#[derive(Deserialize)]
struct Row {
    input: Input,
    original: Original,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Row>,
    original_code_bytes_matched: bool,
    instruction_windows: usize,
}

fn setup(input: &Input, operation: u8) -> (Program, Vm, ThreadId) {
    let mut operands = [0; 8];
    operands[0] = operation;
    operands[1] = input.token_kind;
    operands[2..4].copy_from_slice(&input.flags.to_le_bytes());
    operands[4..].copy_from_slice(&input.guid.to_le_bytes());
    let program = Program {
        globals: "inventory-probe".into(),
        modules: BTreeMap::from([(
            "inventory-probe".into(),
            Module {
                source: "inventory-probe".into(),
                routines: BTreeMap::from([(
                    4096,
                    Routine {
                        name: "inventory probe".into(),
                        body: Behavior {
                            format: 0x8002,
                            kind: 0,
                            args: 0,
                            locals: 0,
                            instructions: vec![Instruction {
                                opcode: 0x33,
                                on_true: 0,
                                on_false: 0,
                                operands,
                            }],
                        },
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut world = World::default();
    let mut people = WorldState::default();
    for (id, guid) in [(1, OWNER_GUID), (2, OWNER_GUID + 2)] {
        world.objects.insert(
            id,
            Object {
                definition: Some(Definition {
                    source: "inventory-probe".into(),
                    object: id as u16,
                    guid,
                    slots: 0,
                }),
                person: Some(BTreeMap::new()),
                temps: if id == 1 {
                    input.temps.unwrap_or([11, 22, 33, 44, 2, 66, 77, 88])
                } else {
                    [0; 8]
                },
                ..Default::default()
            },
        );
        people.neighbors.insert(
            id,
            Neighbor {
                guid,
                inventory: input.entries.clone(),
                ..Default::default()
            },
        );
    }
    world.person = Some(people);
    let mut vm = Vm::new(world, 0x12345678);
    vm.lot_rng_state = Some(0x87654321);
    vm.crt_rng_state = Some(0xabcdef01);
    let thread = vm
        .spawn(
            &program,
            Start {
                owner: "inventory-probe",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: if input.guid == 0 { 1 } else { 2 },
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (program, vm, thread)
}

fn cold(program: &Program, vm: &Vm) -> Vm {
    Snapshot::from_json(
        &serde_json::to_vec(&Snapshot::capture(program, vm)).unwrap(),
        program,
    )
    .unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_inventory_remove_vectors_and_cold_resume() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/inventory_remove.json")).unwrap();
    assert!(report.original_code_bytes_matched);
    assert_eq!(report.instruction_windows, 397);
    assert_eq!(report.cases.len(), 912);
    for (case, row) in report.cases.into_iter().enumerate() {
        let (program, mut vm, thread) = setup(&row.input, 1);
        let mut resumed = cold(&program, &vm);
        assert_eq!(vm, resumed);
        let mut expected = vm.clone();
        expected.threads.get_mut(&thread).unwrap().last_result = row.original.returned;
        let people = expected.world.person.as_mut().unwrap();
        people.neighbors.get_mut(&1).unwrap().inventory = row.original.caller_inventory;
        people.neighbors.get_mut(&2).unwrap().inventory = row.original.selected_inventory;
        people.inventory_dirty = row.original.dirty_word == 0x04000000;
        expected.world.objects.get_mut(&1).unwrap().temps = row.original.temps;
        assert!(row.original.error.is_none());
        let run = vm.run_slice(&program, thread, 1).unwrap();
        assert_eq!(
            run.stop,
            Stop::Budget,
            "case{case}, {:?}: {:?}",
            row.input,
            run.trace
        );
        assert_eq!(
            vm, expected,
            "case{case}: world, temps, frames, RNG, tick or unrelated VM processing differs"
        );
        assert_eq!(
            resumed.run_slice(&program, thread, 1).unwrap(),
            run,
            "case{case}"
        );
        assert_eq!(vm, resumed, "case{case}");
        // Also retain the result and pending next instruction through a cold boundary.
        resumed = cold(&program, &resumed);
        assert_eq!(
            resumed, vm,
            "case{case}: changed inventory was lost on resume"
        );
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap(),
            resumed.run_slice(&program, thread, 1).unwrap()
        );
        assert_eq!(
            vm, resumed,
            "case{case}: next instruction differs after resume"
        );
    }
}

#[test]
fn unsupported_inventory_operations_and_invalid_owners_remain_faults() {
    let input = Input {
        entries: vec![InventoryEntry {
            kind: 6,
            guid: 0x52e387db,
            count: 5,
        }],
        flags: 0x2002,
        token_kind: 6,
        guid: 0x52e387db,
        temps: Some([0, 22, 33, 44, 2, 66, 77, 88]),
    };
    for operation in [0, 2, 4, 5, 6, 7, 255] {
        let (program, mut vm, thread) = setup(&input, operation);
        let world = vm.world.clone();
        let run = vm.run_slice(&program, thread, 1).unwrap();
        assert_eq!(run.stop, Stop::Faulted);
        assert!(
            run.trace[0]
                .effect
                .contains("unsupported inventory operation")
        );
        assert_eq!(vm.world, world);
        assert_eq!(vm.rng_state, 0x12345678);
    }
    let (program, mut vm, thread) = setup(&input, 1);
    vm.world.objects.get_mut(&2).unwrap().person = None;
    let world = vm.world.clone();
    let run = vm.run_slice(&program, thread, 1).unwrap();
    assert_eq!(run.stop, Stop::Faulted);
    assert!(run.trace[0].effect.contains("native error 0x1c"));
    assert_eq!(vm.world, world); // Even quantity0 still validates the owner first.
}

#[test]
fn query_after_removal_uses_compacted_index_and_preserves_output_alias_order() {
    let guid = 0x52e387db;
    let input = Input {
        entries: vec![
            InventoryEntry {
                kind: 5,
                guid,
                count: 9,
            },
            InventoryEntry {
                kind: 6,
                guid,
                count: 1,
            },
            InventoryEntry {
                kind: 6,
                guid,
                count: 7,
            },
        ],
        flags: 2,
        token_kind: 6,
        guid,
        temps: Some([1, 22, 33, 44, 2, 66, 77, 88]),
    };
    for index_temp in [2, 3] {
        let (mut program, mut vm, thread) = setup(&input, 1);
        let body = &mut program
            .modules
            .get_mut("inventory-probe")
            .unwrap()
            .routines
            .get_mut(&4096)
            .unwrap()
            .body;
        body.instructions[0].on_true = 1;
        body.instructions[0].on_false = 1;
        let mut query = body.instructions[0].clone();
        query.operands[0] = 3;
        query.operands[2..4].copy_from_slice(&((2u16 << 5) | (index_temp << 11)).to_le_bytes());
        body.instructions.push(query);
        assert_eq!(
            vm.run_slice(&program, thread, 1).unwrap().stop,
            Stop::Budget
        );
        assert_eq!(
            vm.world.person.as_ref().unwrap().neighbors[&1].inventory,
            vec![input.entries[0].clone(), input.entries[2].clone()]
        );
        let mut resumed = cold(&program, &vm);
        let run = vm.run_slice(&program, thread, 1).unwrap();
        assert_eq!(run.stop, Stop::Budget);
        assert_eq!(resumed.run_slice(&program, thread, 1).unwrap(), run);
        assert_eq!(resumed, vm);
        assert_eq!(
            vm.world.objects[&1].temps[2],
            if index_temp == 2 { 1 } else { 7 }
        );
        assert_eq!(vm.world.objects[&1].temps[index_temp as usize], 1);
        assert_eq!(
            vm.world.person.as_ref().unwrap().neighbors[&2].inventory,
            input.entries
        );
    }
}
