//! A synthetic lot and program, built in memory. No game data.
//!
//! Resources: 0 is Global.iff, 1 the lamp's file, 2 the lamp's
//! semi-global file, 3 the Sim's file (no semi-global). The lot holds lamp
//! object 1 (type 7) and Sim object 2 (type 8).

use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::{Interaction, Person, Slot};
use sims_sim::{Budget, Object, World};
use sims_vm::{Fault, Interpreter, Library, Program, Stream, Trap};
use std::collections::BTreeMap;

pub const GLOBAL: usize = 0;
pub const LAMP_FILE: usize = 1;
pub const SEMI_FILE: usize = 2;
pub const SIM_FILE: usize = 3;
pub const LAMP: u16 = 1;
pub const SIM: u16 = 2;
pub const LAMP_TYPE: i16 = 7;
pub const LAMP_GUID: u32 = 0x1111;
pub const SIM_GUID: u32 = 0x2222;

pub const RETURN_TRUE: u8 = 254;
pub const RETURN_FALSE: u8 = 255;

/// A tree with four arguments and four locals.
pub fn tree(instructions: Vec<Instruction>) -> Behavior {
    shaped(0x8002, 0, 4, 4, instructions)
}

pub fn shaped(
    format: u16,
    kind: u8,
    args: u8,
    locals: u16,
    instructions: Vec<Instruction>,
) -> Behavior {
    Behavior {
        format,
        kind,
        args,
        locals,
        revision: 0,
        instructions,
    }
}

pub fn ins(opcode: u16, on_true: u8, on_false: u8, operands: [u8; 8]) -> Instruction {
    Instruction {
        opcode,
        on_true,
        on_false,
        operands,
    }
}

/// Expression operands.
pub fn expr(left_scope: u8, left: i16, operator: u8, right_scope: u8, right: i16) -> [u8; 8] {
    let [l0, l1] = left.to_le_bytes();
    let [r0, r1] = right.to_le_bytes();
    [l0, l1, r0, r1, 0, operator, left_scope, right_scope]
}

/// An Expression that returns its result.
pub fn expression(
    left_scope: u8,
    left: i16,
    operator: u8,
    right_scope: u8,
    right: i16,
) -> Instruction {
    ins(
        2,
        RETURN_TRUE,
        RETURN_FALSE,
        expr(left_scope, left, operator, right_scope, right),
    )
}

/// Expression scopes and operators used by the tests.
pub mod scope {
    pub const MY_ATTRIBUTES: u8 = 0;
    pub const STACK_ATTRIBUTES: u8 = 1;
    pub const MY_DATA: u8 = 3;
    pub const STACK_DATA: u8 = 4;
    pub const GLOBAL: u8 = 6;
    pub const LITERAL: u8 = 7;
    pub const TEMPS: u8 = 8;
    pub const PARAMETERS: u8 = 9;
    pub const STACK_OBJECT: u8 = 10;
    pub const LOCALS: u8 = 25;
}
pub mod op {
    pub const GREATER: u8 = 0;
    pub const EQUAL: u8 = 2;
    pub const ADD: u8 = 3;
    pub const ASSIGN: u8 = 5;
}

pub fn object(id: u16, guid: u32) -> Object {
    Object {
        id,
        guid,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: vec![10, 11, 12],
        temps: vec![0; 8],
        data: (0..72).map(|n| n * 2).collect(),
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        // Slot 0 is the tile chain; script slots 0 and 1 follow.
        slots: vec![
            Slot {
                unknown: 0,
                object: 99,
            },
            Slot {
                unknown: 0,
                object: SIM as i16,
            },
            Slot {
                unknown: 0,
                object: 0,
            },
        ],
        dynamic_sprites: vec![5, 6],
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
    }
}

fn interaction() -> Interaction {
    Interaction {
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: 0,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    }
}

pub fn person() -> Person {
    let mut motives = [0.0; 16];
    for (n, m) in motives.iter_mut().enumerate() {
        *m = n as f32 * 10.0 - 50.0;
    }
    Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: [0.0; 16],
        motives,
        data: (0..256).map(|n| n * 3).collect(),
        routing_frames: 0,
        active: interaction(),
        last: interaction(),
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    }
}

pub fn world() -> World {
    let mut sim = object(SIM, SIM_GUID);
    sim.attributes = vec![20, 21];
    sim.person = Some(person());
    World {
        ticks: 0,
        globals: (0..64).map(|n| n + 100).collect(),
        scheduler_cursor: 0,
        lot_rng: GameRng::from_state(1),
        global_rng: GameRng::from_state(2),
        budget: Budget {
            funds: 0,
            pending_funds: 0,
            lot_value: 0,
            objects_value: 0,
            architecture_value: 0,
            days: Vec::new(),
        },
        order: vec![LAMP, SIM],
        types: BTreeMap::from([(7, LAMP_GUID), (8, SIM_GUID)]),
        objects: BTreeMap::from([(LAMP, object(LAMP, LAMP_GUID)), (SIM, sim)]),
        arrays: BTreeMap::new(),
    }
}

fn catalog() -> Catalog {
    let definition = |guid, resource, kind| sims_content::Object {
        guid,
        resource,
        chunk: 128,
        kind,
        master: 0,
    };
    Catalog {
        global: GLOBAL,
        objects: vec![
            definition(LAMP_GUID, LAMP_FILE, 4),
            definition(SIM_GUID, SIM_FILE, 2),
        ],
        by_guid: BTreeMap::from([(LAMP_GUID, 0), (SIM_GUID, 1)]),
        semi_globals: BTreeMap::from([(LAMP_FILE, Some(SEMI_FILE))]),
        diagnostics: Vec::new(),
    }
}

fn program() -> Program {
    let library = |constants: Vec<(u16, Vec<i16>)>, callbacks: Vec<u16>| Library {
        behaviors: BTreeMap::new(),
        constants: constants.into_iter().collect(),
        callbacks: BTreeMap::from([(
            128,
            callbacks
                .into_iter()
                .map(|action| ObjectFunction {
                    condition: 0,
                    action,
                })
                .collect(),
        )]),
    };
    Program {
        global: GLOBAL,
        libraries: BTreeMap::from([
            (GLOBAL, library(vec![(256, vec![300, 301])], vec![])),
            (
                LAMP_FILE,
                library(vec![(4096, vec![99, -5])], vec![4096, 0, 4097]),
            ),
            (SEMI_FILE, library(vec![(8192, vec![7])], vec![])),
            (SIM_FILE, library(vec![], vec![])),
        ]),
        semi_globals: BTreeMap::from([(LAMP_FILE, Some(SEMI_FILE)), (SIM_FILE, None)]),
    }
}

pub struct Fixture {
    pub program: Program,
    pub catalog: Catalog,
    pub world: World,
    pub stream: Stream,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            program: program(),
            catalog: catalog(),
            world: world(),
            stream: Stream::Lot,
        }
    }

    pub fn add(&mut self, resource: usize, id: u16, behavior: Behavior) -> &mut Self {
        self.program
            .libraries
            .get_mut(&resource)
            .unwrap()
            .behaviors
            .insert(id, behavior);
        self
    }

    pub fn interpreter(&self) -> Interpreter<'_> {
        Interpreter::new(&self.program, &self.catalog, self.stream)
    }

    pub fn run_with(
        &mut self,
        object: u16,
        tree: u16,
        stack_object: i16,
        args: &[i16],
        budget: usize,
    ) -> Result<bool, Fault> {
        let interpreter = Interpreter::new(&self.program, &self.catalog, self.stream);
        interpreter.run_tree(&mut self.world, object, tree, stack_object, args, budget)
    }

    /// Runs the lamp's tree 4096 with the Sim as stack object.
    pub fn run(&mut self) -> Result<bool, Fault> {
        self.run_with(LAMP, 4096, SIM as i16, &[], 1000)
    }

    /// Runs one Expression as the lamp's tree 4096.
    pub fn expression(
        &mut self,
        left_scope: u8,
        left: i16,
        operator: u8,
        right_scope: u8,
        right: i16,
    ) -> Result<bool, Fault> {
        self.add(
            LAMP_FILE,
            4096,
            tree(vec![expression(
                left_scope,
                left,
                operator,
                right_scope,
                right,
            )]),
        );
        self.run()
    }

    pub fn lamp(&self) -> &Object {
        &self.world.objects[&LAMP]
    }

    pub fn sim(&self) -> &Object {
        &self.world.objects[&SIM]
    }

    pub fn lamp_mut(&mut self) -> &mut Object {
        self.world.objects.get_mut(&LAMP).unwrap()
    }

    pub fn sim_mut(&mut self) -> &mut Object {
        self.world.objects.get_mut(&SIM).unwrap()
    }
}

/// The trap of a failed run.
pub fn trap(result: Result<bool, Fault>) -> Trap {
    result.expect_err("expected a fault").trap
}

/// The native error code of a failed run.
pub fn code(result: Result<bool, Fault>) -> Option<u16> {
    trap(result).code()
}
