//! The factory for multi-tile objects (0x440b2d, from 0x440b63), through
//! `Interpreter::create`. Synthetic; no game data.

use crate::support::*;
use sims_formats::execution::ObjectFunction;
use sims_vm::{DEFINITION_WORDS, Library, Trap};
use std::collections::BTreeMap;

/// The group's catalog master, which scripts ask for.
const MASTER: u32 = 0x4000;
/// Its tiles: the lead at the group's origin, one a row along and one a
/// column across.
const ALONG: u32 = 0x4001;
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
fn group(fx: &mut Fixture) -> usize {
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

fn create(fx: &mut Fixture, guid: u32) -> Result<u16, sims_vm::Fault> {
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
