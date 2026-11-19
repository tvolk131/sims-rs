//! The factory for multi-tile objects (0x440b2d, from 0x440b63), through
//! `Interpreter::create`, and how a group leaves: Find Location For mode 1,
//! Remove Object Instance and the deletion (0x440f37). Synthetic; no game
//! data.

use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_vm::{DEFINITION_WORDS, Library, Trap};
use std::collections::BTreeMap;

/// The group's catalog master, which scripts ask for.
pub const MASTER: u32 = 0x4000;
/// Its tiles: the lead at the group's origin, one a row along and one a
/// column across.
pub const ALONG: u32 = 0x4001;
const LEAD: u32 = 0x4002;
const ACROSS: u32 = 0x4003;
/// Each tile's Init and Main trees.
const INIT: u16 = 4096;
const MAIN: u16 = 4097;
const SLEEP: u16 = 256;

/// Definition words (`sims_vm::factory::word`).
const GROUP: usize = 10;
const SUB_INDEX: usize = 11;
const PRICE: usize = 18;

/// Adds a file of four definitions, in this resource order: the master
/// (sub-index -1, price 900), the tile at (0, 1), the tile at (0, 0) and
/// the tile at (1, 0), all of group 7. Init sets attribute 0 to 7 and
/// Main sleeps.
pub fn group(fx: &mut Fixture) -> usize {
    let resource = fx.program.libraries.len();
    let definitions = [
        (128, MASTER, -1),
        (129, ALONG, 0x0001),
        (130, LEAD, 0),
        (131, ACROSS, 0x0100),
    ];
    let mut library = Library::default();
    for (chunk, guid, sub) in definitions {
        let mut words = [0; DEFINITION_WORDS];
        words[9] = 4;
        words[GROUP] = 7;
        words[SUB_INDEX] = sub;
        words[14] = guid as u16 as i16;
        words[15] = (guid >> 16) as u16 as i16;
        if sub == -1 {
            words[PRICE] = 900;
        }
        library.definitions.insert(chunk, words);
        library.definition_order.push(chunk);
        library.callbacks.insert(
            chunk,
            [INIT, MAIN]
                .map(|action| ObjectFunction {
                    condition: 0,
                    action,
                })
                .to_vec(),
        );
        fx.catalog.by_guid.insert(guid, fx.catalog.objects.len());
        fx.catalog.objects.push(sims_content::Object {
            guid,
            resource,
            chunk,
            kind: 4,
            master: u16::from(sub != -1),
        });
    }
    library.behaviors = BTreeMap::from([
        (
            INIT,
            tree(vec![assign(
                scope::MY_ATTRIBUTES,
                0,
                scope::LITERAL,
                7,
                RETURN_TRUE,
            )]),
        ),
        (
            MAIN,
            tree(vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
        ),
    ]);
    fx.program.libraries.insert(resource, library);
    fx.program.semi_globals.insert(resource, None);
    fx.add(GLOBAL, SLEEP, sleeper());
    resource
}

pub fn create(fx: &mut Fixture, guid: u32) -> Result<u16, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.create(&mut fx.world, guid)
}

#[test]
fn a_group_is_made_lead_first_from_its_catalog_master() {
    let mut fx = Fixture::new();
    group(&mut fx);
    // The lead (the tile at the group's origin) comes first, then the
    // others in resource order; each takes the lowest unused ID.
    assert_eq!(create(&mut fx, MASTER), Ok(3));
    let made: Vec<_> = [3, 4, 5].map(|id| fx.world.objects[&id].guid).to_vec();
    assert_eq!(made, [LEAD, ALONG, ACROSS]);
    // Each went to the head of the registration list.
    assert_eq!(fx.world.order, [5, 4, 3, LAMP, SIM]);
    // The tiles chain from the lead, and each knows its place in the group
    // and its offset from the lead (facing 0).
    assert_eq!(fx.world.next_parts, BTreeMap::from([(3, 4), (4, 5)]));
    let multipart = |id: u16| {
        let m = fx.world.objects[&id].multipart.clone().unwrap();
        (m.offset, m.group, m.parent)
    };
    assert_eq!(multipart(3), ([0; 3], [0; 3], 0));
    assert_eq!(multipart(4), ([0, 1, 0], [0, 1, 0], 3));
    assert_eq!(multipart(5), ([1, 0, 0], [1, 0, 0], 3));
    // The lead takes the master's price; every tile ran Init after the
    // group's reset, had its first turn, and is ready, off the lot.
    assert_eq!(fx.world.objects[&3].data[41], 900);
    for id in [3, 4, 5] {
        let tile = &fx.world.objects[&id];
        assert_eq!(tile.attributes[0], 7, "tile {id}");
        assert_eq!(tile.countdown(), Some(19), "tile {id}");
        assert_eq!(tile.flags() & 1, 1, "tile {id}");
        assert_eq!(tile.position, [-16, -16, 1], "tile {id}");
    }
    // The master itself is never made.
    assert!(fx.world.objects.values().all(|o| o.guid != MASTER));
}

#[test]
fn only_a_catalog_master_with_a_lead_makes_a_group() {
    // A tile asked for on its own (0x440b90).
    let mut fx = Fixture::new();
    group(&mut fx);
    let fault = create(&mut fx, ALONG).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x440b90")),
        "{fault}"
    );
    // A group with no tile at its origin and none marked lead (0x440c9b).
    let mut fx = Fixture::new();
    let file = group(&mut fx);
    let words = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .definitions
        .get_mut(&130)
        .unwrap();
    words[SUB_INDEX] = 0x0101;
    let before = fx.world.objects.len();
    let fault = create(&mut fx, MASTER).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x440c9b")),
        "{fault}"
    );
    assert_eq!(fx.world.objects.len(), before);
}

/// Function 10 (leaving the lot) of every tile adds 1 to global
/// [`COUNT`]; with `reset`, the lead's type also has a Reset tree
/// (function 3) that adds 100.
pub fn counting(fx: &mut Fixture, file: usize, reset: bool) {
    let library = fx.program.libraries.get_mut(&file).unwrap();
    for (&chunk, table) in library.callbacks.iter_mut() {
        table.resize_with(11, || ObjectFunction {
            condition: 0,
            action: 0,
        });
        table[10].action = LEAVING;
        if reset && chunk == 130 {
            table[3].action = RESET;
        }
    }
    for (id, add) in [(LEAVING, 1), (RESET, 100)] {
        library.behaviors.insert(
            id,
            tree(vec![expression(
                scope::GLOBAL,
                COUNT as i16,
                op::ADD,
                scope::LITERAL,
                add,
            )]),
        );
    }
}

/// The trees [`counting`] adds, and the global they count in (36, which
/// a script may write).
const LEAVING: u16 = 4098;
const RESET: u16 = 4099;
pub const COUNT: usize = 36;

/// Gives the type of the tile one along from the lead (object 4) the
/// dynamic group function, 8 (0x42344e).
pub fn dynamic(fx: &mut Fixture, file: usize) {
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&129)
        .unwrap();
    table.resize_with(11, || ObjectFunction {
        condition: 0,
        action: 0,
    });
    table[8].action = MAIN;
}

fn remove_now(fx: &mut Fixture, id: u16) -> Result<(), sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.remove_now(&mut fx.world, id)
}

#[test]
fn a_group_is_deleted_from_its_lead_and_each_tiles_reset_takes_what_is_left_off() {
    let mut fx = placed_group(false, false);
    let before = fx.world.globals[COUNT];
    // Removing a tile that is not the lead (0x44123f): its take-off takes
    // the whole group off, asking each of the three tiles function 10
    // (0x422d86). The deletion goes to the lead (0x440f6f), which deletes
    // every tile from itself on (0x440fe3). Each tile's reset, with no
    // Reset tree, takes what is left of the group off again (0x432055):
    // three tiles, then two, then one.
    remove_now(&mut fx, 4).unwrap();
    assert_eq!(fx.world.globals[COUNT] - before, 3 + 3 + 2 + 1);
    for id in [3, 4, 5] {
        assert!(!fx.world.objects.contains_key(&id), "tile {id}");
    }
    assert!(fx.world.next_parts.is_empty());
    assert_eq!(fx.world.order, [LAMP, SIM]);
}

#[test]
fn the_next_tile_leads_what_is_left_when_the_lead_goes() {
    let mut fx = placed_group(false, true);
    let before = fx.world.globals[COUNT];
    // The lead's Reset tree runs instead of a take-off. Once the lead is
    // gone, tile 4 leads tile 5 (0x4228d6), so its reset takes those two
    // off, and tile 5's then itself.
    remove_now(&mut fx, 3).unwrap();
    assert_eq!(fx.world.globals[COUNT] - before, 3 + 100 + 2 + 1);
    assert!(fx.world.next_parts.is_empty());
    assert_eq!(fx.world.order, [LAMP, SIM]);
}

/// Runs the lamp's tree 4200: `instruction`, with tile 4 as the stack
/// object.
fn on_tile(fx: &mut Fixture, opcode: u16, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4200,
        tree(vec![ins(opcode, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4200, 4, &[], 1000)
}

/// A group (tiles 3, 4 and 5, off the lot) on a lot whose rooms are
/// worked out, so objects leave through the lot's placement.
/// With `reset`, the lead's type has a Reset tree ([`counting`]).
fn placed_group(dynamic_group: bool, reset: bool) -> Fixture {
    let (mut fx, _) = crate::find_location::placeable();
    let file = group(&mut fx);
    counting(&mut fx, file, reset);
    if dynamic_group {
        dynamic(&mut fx, file);
    }
    assert_eq!(create(&mut fx, MASTER), Ok(3));
    fx
}

#[test]
fn find_location_mode_1_on_a_tile_takes_its_group_off() {
    const FIND_LOCATION: u16 = 16;
    let mut fx = placed_group(false, false);
    let before = fx.world.globals[COUNT];
    assert_eq!(
        on_tile(&mut fx, FIND_LOCATION, [1, 0, 0, 0, 0, 0, 0, 0]),
        Ok(true)
    );
    assert_eq!(fx.world.globals[COUNT] - before, 3);
    // A tile whose type has the dynamic group function leaves its group
    // first (0x42346a), so the take-off takes it alone: the lead keeps
    // the third tile. (Its neighbours: `crate::dynamic_group`.)
    let mut fx = placed_group(true, false);
    let before = fx.world.globals[COUNT];
    assert_eq!(
        on_tile(&mut fx, FIND_LOCATION, [1, 0, 0, 0, 0, 0, 0, 0]),
        Ok(true)
    );
    assert_eq!(fx.world.globals[COUNT] - before, 1);
    assert_eq!(fx.world.next_parts, BTreeMap::from([(3, 5)]));
    assert_eq!(fx.world.objects[&4].multipart.as_ref().unwrap().parent, 0);
}

#[test]
fn removing_a_tile_queues_its_lead() {
    const REMOVE: u16 = 18;
    let mut fx = placed_group(false, false);
    let before = fx.world.globals[COUNT];
    assert_eq!(on_tile(&mut fx, REMOVE, [1, 0, 0, 0, 0, 0, 0, 0]), Ok(true));
    assert_eq!(fx.world.removals, [3]);
    assert_eq!(fx.world.globals[COUNT] - before, 3);
    // A tile whose type has the dynamic group function goes alone
    // (0x442def); its take-off is still its group's (0x422d86), and it
    // stays in its group until it is deleted (0x4424a6).
    let mut fx = placed_group(true, false);
    let before = fx.world.globals[COUNT];
    assert_eq!(on_tile(&mut fx, REMOVE, [1, 0, 0, 0, 0, 0, 0, 0]), Ok(true));
    assert_eq!(fx.world.removals, [4]);
    assert_eq!(fx.world.globals[COUNT] - before, 3);
    assert_eq!(fx.world.next_parts, BTreeMap::from([(3, 4), (4, 5)]));
}

#[test]
fn a_group_removed_before_the_rooms_are_worked_out_fails_loudly() {
    // The load's clear removes objects before it works the rooms out; a
    // group's take-off needs them.
    let mut fx = Fixture::new();
    let file = group(&mut fx);
    counting(&mut fx, file, false);
    assert_eq!(create(&mut fx, MASTER), Ok(3));
    let before = fx.world.clone();
    let fault = remove_now(&mut fx, 4).unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x4412a4")),
        "{fault}"
    );
    assert_eq!(fx.world.hash(), before.hash());
}

#[test]
fn a_group_whose_lead_is_a_person_type_works_out_again_who_is_a_visitor() {
    // The factory's signal 0xe2 names the lead (0x440ed7), and the lot
    // runs the visitor pass when the lead's definition is a person's
    // (type 2, 0x4163f7). On a residential lot the Sim, a member of the
    // family global 9 names, gets person word 32 0.
    let made = |lead_kind: i16| {
        let mut fx = Fixture::new();
        let file = group(&mut fx);
        fx.program
            .libraries
            .get_mut(&file)
            .unwrap()
            .definitions
            .get_mut(&130)
            .unwrap()[9] = lead_kind;
        let lot = fx.world.globals[10];
        fx.program.zoning.insert(lot, 1);
        let sim = fx.world.objects.get_mut(&SIM).unwrap();
        sim.person.as_mut().unwrap().data[32] = 2;
        assert_eq!(create(&mut fx, MASTER), Ok(3));
        fx.world.objects[&SIM].person.as_ref().unwrap().data[32]
    };
    assert_eq!(made(2), 0);
    // A lead of any other type leaves it.
    assert_eq!(made(4), 2);
}
