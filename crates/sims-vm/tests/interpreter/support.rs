//! A synthetic lot and program, built in memory. No game data.
//!
//! Resources: 0 is Global.iff, 1 the lamp's file, 2 the lamp's
//! semi-global file, 3 the Sim's file (no semi-global). The lot holds lamp
//! object 1 (type 7) and Sim object 2 (type 8).

use sims_content::Catalog;
use sims_core::GameRng;
use sims_formats::behavior::{Behavior, Instruction};
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::{Frame, Interaction, Person, Slot};
use sims_sim::{Object, World};
use sims_vm::{DEFINITION_WORDS, Fault, Interpreter, Library, Program, Stream, Trap};
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

pub const ERROR: u8 = 253;
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

/// A tree of one parameter that sleeps for that many turns, then returns
/// true. The base game's global tree 280 is one of these.
pub fn sleeper() -> Behavior {
    shaped(
        0x8002,
        0,
        1,
        0,
        vec![ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8])],
    )
}

/// `left = right`, then go to `next`.
pub fn assign(left_scope: u8, left: i16, right_scope: u8, right: i16, next: u8) -> Instruction {
    ins(
        2,
        next,
        next,
        expr(left_scope, left, op::ASSIGN, right_scope, right),
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
        last_result: false,
        removal_skips_resets: false,
        selected: false,
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
        globals: (0..64).map(|n| n + 100).collect(),
        lot_rng: GameRng::from_state(1),
        global_rng: GameRng::from_state(2),
        order: vec![LAMP, SIM],
        types: BTreeMap::from([(7, LAMP_GUID), (8, SIM_GUID)]),
        objects: BTreeMap::from([(LAMP, object(LAMP, LAMP_GUID)), (SIM, sim)]),
        ..World::empty()
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

/// An object definition of `kind` (word 9) for `guid` (words 14 and 15), in
/// no multi-tile group. Its other words count up from 1000.
pub fn definition(guid: u32, kind: i16) -> [i16; DEFINITION_WORDS] {
    let mut words = std::array::from_fn(|i| 1000 + i as i16);
    words[9] = kind;
    words[10] = 0;
    words[11] = 0;
    words[14] = guid as u16 as i16;
    words[15] = (guid >> 16) as u16 as i16;
    words
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
        ..Library::default()
    };
    let mut program = Program {
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
        careers: None,
        zoning: BTreeMap::new(),
        animations: Default::default(),
        people: Default::default(),
    };
    for (resource, guid, kind) in [(LAMP_FILE, LAMP_GUID, 4), (SIM_FILE, SIM_GUID, 2)] {
        let library = program.libraries.get_mut(&resource).unwrap();
        library.definitions.insert(128, definition(guid, kind));
        library.definition_order.push(128);
    }
    program
}

/// The lot's hash without its frame storage. Running any tree leaves
/// words in the running object's frame storage; tests that say a
/// primitive changes nothing else compare this.
pub fn hash_without_frames(world: &World) -> [u8; 32] {
    let mut world = world.clone();
    world.frame_storage.clear();
    world.hash()
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

    /// Adds an object type of `kind` in a file of its own, with the given
    /// Init and Main trees and every other definition word zero. Returns
    /// the file's resource number.
    pub fn add_type(&mut self, guid: u32, kind: i16, init: u16, main: u16) -> usize {
        let resource = self.program.libraries.len();
        let mut words = [0; DEFINITION_WORDS];
        words[9] = kind;
        words[14] = guid as u16 as i16;
        words[15] = (guid >> 16) as u16 as i16;
        let callbacks = [init, main].map(|action| ObjectFunction {
            condition: 0,
            action,
        });
        let library = Library {
            callbacks: BTreeMap::from([(128, callbacks.to_vec())]),
            definitions: BTreeMap::from([(128, words)]),
            definition_order: vec![128],
            ..Library::default()
        };
        self.program.libraries.insert(resource, library);
        self.program.semi_globals.insert(resource, None);
        self.catalog
            .by_guid
            .insert(guid, self.catalog.objects.len());
        self.catalog.objects.push(sims_content::Object {
            guid,
            resource,
            chunk: 128,
            kind: kind as u16,
            master: 0,
        });
        resource
    }

    /// The definition words of the one type in a file.
    pub fn words(&mut self, resource: usize) -> &mut [i16; DEFINITION_WORDS] {
        self.program
            .libraries
            .get_mut(&resource)
            .unwrap()
            .definitions
            .get_mut(&128)
            .unwrap()
    }

    /// Replaces an object's thread with one frame at the start of `tree`.
    /// The object's type must be in the lot's type table.
    pub fn thread(&mut self, object: u16, tree: u16, parameters: &[i16]) {
        let guid = self.world.objects[&object].guid;
        let (&code_owner_type, _) = self.world.types.iter().find(|&(_, &g)| g == guid).unwrap();
        self.world.frame_storage.remove(&object);
        self.world.objects.get_mut(&object).unwrap().stack = vec![Frame {
            stack_object: 0,
            tree,
            node: 0,
            parameters: parameters.to_vec(),
            locals: Vec::new(),
            primitive_state: 0,
            code_owner_type: code_owner_type as i16,
        }];
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

    /// Adds an object of the lamp's type, registered after the others.
    pub fn add_object(&mut self, id: u16) -> &mut Object {
        self.world.order.push(id);
        self.world.objects.insert(id, object(id, LAMP_GUID));
        self.world.objects.get_mut(&id).unwrap()
    }

    /// Adds a Sim of the Sim's type, registered after the others.
    pub fn add_sim(&mut self, id: u16) -> &mut Object {
        let mut sim = object(id, SIM_GUID);
        sim.person = Some(person());
        self.world.order.push(id);
        self.world.objects.insert(id, sim);
        self.world.objects.get_mut(&id).unwrap()
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
