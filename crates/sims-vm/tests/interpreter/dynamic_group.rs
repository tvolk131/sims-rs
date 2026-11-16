//! Dynamic groups (a group whose type has callback function 8, such as
//! flood.iff's puddles): a new one placed by Create New Object Instance
//! joining the tiles around it (0x4235ee), each of its tiles learning
//! which of its neighbours belong to it (0x4234ab, 0x4234d8), and a tile
//! leaving its group (0x42346a, 0x4228d6) before Find Location For mode 1
//! takes it off or the end of the lot update deletes it. Synthetic; no
//! game data.
//!
//! The lot is the six-tile one of the room tests (`crate::rooms::build`),
//! with the three-tile group of `crate::group_factory`: the lead, a tile
//! one along (offset (0, 1)) and a tile one across (offset (1, 0)).

use crate::group_factory::{COUNT, MASTER, counting, group};
use crate::support::*;
use sims_formats::behavior::Behavior;
use sims_formats::execution::ObjectFunction;
use sims_sim::World;
use sims_vm::{DEFINITION_WORDS, Library};
use std::collections::BTreeMap;

const CREATE: u16 = 42;
const FIND_LOCATION: u16 = 16;
/// The dynamic group function's tree in each tile's file.
const NEIGHBOURS: u16 = 4300;
/// The group file's Init and Main (`crate::group_factory`).
const INIT: u16 = 4096;
const MAIN: u16 = 4097;
/// The chunks of the group's three tiles: one along, the lead and one
/// across.
const TILES: [u16; 3] = [129, 130, 131];

/// Function 8's tree: temps 0 to 3 take its four arguments and temp 4 its
/// stack object, and temp 5 counts its calls.
fn neighbours() -> Behavior {
    tree(vec![
        assign(scope::TEMPS, 0, scope::PARAMETERS, 0, 1),
        assign(scope::TEMPS, 1, scope::PARAMETERS, 1, 2),
        assign(scope::TEMPS, 2, scope::PARAMETERS, 2, 3),
        assign(scope::TEMPS, 3, scope::PARAMETERS, 3, 4),
        assign(scope::TEMPS, 4, scope::STACK_OBJECT, 0, 5),
        ins(
            2,
            RETURN_TRUE,
            RETURN_FALSE,
            expr(scope::TEMPS, 5, op::ADD, scope::LITERAL, 1),
        ),
    ])
}

/// Gives each of `chunks` in `file` function 8 ([`neighbours`]), and the
/// file's Init lets each tile stand on level 1 (data word 4 bit 0) on a
/// tile with no floor (data word 42 flag 2).
fn make_dynamic(fx: &mut Fixture, file: usize, chunks: &[u16]) {
    let library = fx.program.libraries.get_mut(&file).unwrap();
    for chunk in chunks {
        let table = library.callbacks.get_mut(chunk).unwrap();
        table.resize_with(11, || ObjectFunction {
            condition: 0,
            action: 0,
        });
        table[8].action = NEIGHBOURS;
    }
    library.behaviors.insert(NEIGHBOURS, neighbours());
    library.behaviors.insert(
        INIT,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
        ]),
    );
}

/// The six-tile lot with its rooms worked out and the lot's grids, its
/// indicators made, and the three-tile group's type dynamic on every
/// tile. Function 10 of every tile counts in global [`COUNT`].
fn lot() -> Fixture {
    let (mut fx, _) = crate::find_location::placeable();
    let file = group(&mut fx);
    counting(&mut fx, file, false);
    make_dynamic(&mut fx, file, &TILES);
    fx
}

/// Runs the lamp's tree 4200: Create New Object Instance mode 8 of
/// `guid` where the lamp stands (local 0 names the lamp).
fn create_where_the_lamp_stands(fx: &mut Fixture, guid: u32) -> Result<bool, sims_vm::Fault> {
    let [g0, g1, g2, g3] = guid.to_le_bytes();
    fx.add(
        LAMP_FILE,
        4200,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 1),
            ins(
                CREATE,
                RETURN_TRUE,
                RETURN_FALSE,
                [g0, g1, g2, g3, 8, 0, 0, 0],
            ),
        ]),
    );
    fx.run_with(LAMP, 4200, SIM as i16, &[], 1000)
}

/// Runs Find Location For mode 1 (take the object off) with `id` as the
/// stack object.
fn take_off(fx: &mut Fixture, id: u16) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4201,
        tree(vec![ins(
            FIND_LOCATION,
            RETURN_TRUE,
            RETURN_FALSE,
            [1, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(LAMP, 4201, id as i16, &[], 1000)
}

/// The group made where the lamp stands, at tile (2, 2) of level 1: tiles
/// 3 (the lead, at (2, 2)), 4 (one along, at (2, 3)) and 5 (one across,
/// at (3, 2)).
fn placed() -> Fixture {
    let mut fx = lot();
    fx.lamp_mut().position = [40, 40, 1];
    assert_eq!(create_where_the_lamp_stands(&mut fx, MASTER), Ok(true));
    fx
}

/// What a tile's function 8 last had: its four arguments, its stack
/// object, and how many times it ran.
fn told(world: &World, id: u16) -> ([i16; 4], i16, i16) {
    let temps = &world.objects[&id].temps;
    ([temps[0], temps[1], temps[2], temps[3]], temps[4], temps[5])
}

/// Each tile's lead (0 for a lead) and the chain of tiles.
fn links(world: &World) -> (Vec<(u16, i16)>, BTreeMap<u16, u16>) {
    let leads = [3, 4, 5]
        .into_iter()
        .filter_map(|id| world.objects.get(&id))
        .map(|o| (o.id, o.multipart.as_ref().unwrap().parent))
        .collect();
    (leads, world.next_parts.clone())
}

#[test]
fn a_placed_dynamic_group_tells_each_tile_which_of_the_eight_tiles_around_it_hold_its_group() {
    let fx = placed();
    let at = |id: u16| fx.world.objects[&id].position;
    assert_eq!(at(3), [40, 40, 1]);
    assert_eq!(at(4), [40, 56, 1]);
    assert_eq!(at(5), [56, 40, 1]);
    // 0x4234d8: bit d of the mask for direction (facing + d) % 8, facing
    // 0 here; direction d steps the position by (step(d)[1], step(d)[0])
    // (0x42228a). From the lead (2, 2), tile 4 at (2, 3) is direction 4
    // and tile 5 at (3, 2) direction 2. From tile 4, the lead is
    // direction 0 and tile 5 direction 1. From tile 5, the lead is
    // direction 6 and tile 4 direction 5. Function 8 runs once for each
    // tile (0x4234ab from the lead along the chain), with stack object 0
    // and the arguments mask, 0, 0, 0 (0x422d46).
    for id in [3, 4, 5] {
        assert_eq!(fx.world.objects[&id].data[1], 0, "tile {id} facing");
    }
    assert_eq!(told(&fx.world, 3), ([1 << 2 | 1 << 4, 0, 0, 0], 0, 1));
    assert_eq!(told(&fx.world, 4), ([1 << 0 | 1 << 1, 0, 0, 0], 0, 1));
    assert_eq!(told(&fx.world, 5), ([1 << 5 | 1 << 6, 0, 0, 0], 0, 1));
}

#[test]
fn a_tile_leaving_its_group_tells_the_rest_from_the_lead_by_each_tiles_own_facing() {
    let mut fx = placed();
    // Each tile reads its own facing (data word 1, +0x4c) for the mask.
    fx.world.objects.get_mut(&3).unwrap().data[1] = 2;
    fx.world.objects.get_mut(&5).unwrap().data[1] = 7;
    let before = fx.world.globals[COUNT];
    // Tile 4 is not the lead: it leaves the chain (0x4228d6), the lead's
    // group (3, then 5) works out its neighbours, then tile 4 on its own,
    // and the take-off takes tile 4 alone (one function 10).
    assert_eq!(take_off(&mut fx, 4), Ok(true));
    assert_eq!(fx.world.globals[COUNT] - before, 1);
    assert_eq!(
        links(&fx.world),
        (vec![(3, 0), (4, 0), (5, 3)], [(3, 5)].into())
    );
    // The lead, facing 2: tile 5 at direction 2 is d = 0.
    assert_eq!(told(&fx.world, 3), ([1, 0, 0, 0], 0, 2));
    // Tile 5, facing 7: the lead at direction 6 is d = 7.
    assert_eq!(told(&fx.world, 5), ([1 << 7, 0, 0, 0], 0, 2));
    // Tile 4 was still on the lot, alone in its group.
    assert_eq!(told(&fx.world, 4), ([0, 0, 0, 0], 0, 2));
    assert_eq!(fx.world.objects[&4].position, [-16, -16, 1]);
    assert_eq!(fx.world.objects[&3].position, [40, 40, 1]);
}

#[test]
fn a_lead_leaving_its_group_hands_it_to_the_next_tile() {
    let mut fx = placed();
    let before = fx.world.globals[COUNT];
    // 0x4228d6: tiles 4 and 5 take tile 4 as their lead, tile 4 then leads
    // (lead 0), and tile 3 is a group of its own. The group from tile 4
    // works out its neighbours, then tile 3.
    assert_eq!(take_off(&mut fx, 3), Ok(true));
    assert_eq!(fx.world.globals[COUNT] - before, 1);
    assert_eq!(
        links(&fx.world),
        (vec![(3, 0), (4, 0), (5, 4)], [(4, 5)].into())
    );
    // Tile 4 at (2, 3) sees tile 5 at direction 1; tile 5 sees tile 4 at
    // direction 5; the lead, on its own, sees none.
    assert_eq!(told(&fx.world, 4), ([1 << 1, 0, 0, 0], 0, 2));
    assert_eq!(told(&fx.world, 5), ([1 << 5, 0, 0, 0], 0, 2));
    assert_eq!(told(&fx.world, 3), ([0, 0, 0, 0], 0, 2));
    assert_eq!(fx.world.objects[&3].position, [-16, -16, 1]);
}

#[test]
fn a_tile_off_the_lot_leaves_its_group_without_telling_anyone() {
    // The group the factory makes stays off the lot (0x4234ab and 0x4234d8
    // do nothing there).
    let mut fx = lot();
    assert_eq!(crate::group_factory::create(&mut fx, MASTER), Ok(3));
    assert_eq!(take_off(&mut fx, 4), Ok(true));
    assert_eq!(
        links(&fx.world),
        (vec![(3, 0), (4, 0), (5, 3)], [(3, 5)].into())
    );
    for id in [3, 4, 5] {
        assert_eq!(told(&fx.world, id).2, 0, "tile {id}");
    }
}

/// Adds a file of a one-tile dynamic group, like flood.iff's puddles: its
/// master [`PUDDLE`] and its one tile (the lead), group 9, whose type has
/// function 8, and Main sleeps. Returns the file.
fn puddles(fx: &mut Fixture) -> usize {
    let resource = fx.program.libraries.len();
    let mut library = Library::default();
    for (chunk, guid, sub) in [(128, PUDDLE, -1), (129, PUDDLE_TILE, 0)] {
        let mut words = [0; DEFINITION_WORDS];
        words[9] = 4;
        words[10] = 9;
        words[11] = sub;
        words[14] = guid as u16 as i16;
        words[15] = (guid >> 16) as u16 as i16;
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
    // Main sleeps, as the group's does.
    library.behaviors.insert(
        MAIN,
        tree(vec![ins(256, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
    );
    fx.program.libraries.insert(resource, library);
    fx.program.semi_globals.insert(resource, None);
    make_dynamic(fx, resource, &[129]);
    resource
}
const PUDDLE: u32 = 0x5000;
const PUDDLE_TILE: u32 = 0x5001;

/// The object Create New Object Instance made last.
fn newest(fx: &Fixture) -> u16 {
    fx.world.order[0]
}

#[test]
fn a_puddle_beside_others_of_its_own_type_joins_each_of_their_groups() {
    let mut fx = lot();
    puddles(&mut fx);
    fx.lamp_mut().position = [40, 40, 1];
    assert_eq!(create_where_the_lamp_stands(&mut fx, PUDDLE), Ok(true));
    let first = newest(&fx);
    // On its own, the tile is told an empty mask.
    assert_eq!(told(&fx.world, first), ([0; 4], 0, 1));
    // A diagonal is not one of the four sides 0x4235ee looks at.
    fx.lamp_mut().position = [56, 56, 1];
    assert_eq!(create_where_the_lamp_stands(&mut fx, PUDDLE), Ok(true));
    let second = newest(&fx);
    assert_ne!(second, first);
    assert_eq!(told(&fx.world, second), ([0; 4], 0, 1));
    assert!(fx.world.next_parts.is_empty());
    // A third on tile (2, 3), in the middle of it (0x44e770): the first
    // is beside it in direction 0 and the second in direction 2. It joins
    // the first's group (0x423368), then the second's group takes that
    // whole group: the second's chain goes on to the first's lead, and
    // the first and the third take the second as their lead, with their
    // offsets from it, in tiles.
    fx.lamp_mut().position = [33, 63, 1];
    assert_eq!(create_where_the_lamp_stands(&mut fx, PUDDLE), Ok(true));
    let third = newest(&fx);
    assert_eq!(fx.world.objects[&third].position, [40, 56, 1]);
    assert_eq!(
        fx.world.next_parts,
        BTreeMap::from([(second, first), (first, third)])
    );
    let part = |id: u16| fx.world.objects[&id].multipart.clone().unwrap();
    assert_eq!(part(second).parent, 0);
    for (id, offset) in [(first, [-1, -1]), (third, [-1, 0])] {
        let part = part(id);
        assert_eq!(part.parent, second as i16, "{id}");
        assert_eq!(part.offset[..2], offset, "{id}");
        // Facing 0: the place in the group is the offset.
        assert_eq!(part.group[..2], offset, "{id}");
    }
    // Each tile of the group, from the lead, is told which of the eight
    // tiles around it hold the group: the second (3, 3) the first in
    // direction 7 and the third in 6; the first (2, 2) the second in 3
    // and the third in 4; the third the first in 0 and the second in 2.
    assert_eq!(told(&fx.world, second), ([0xc0, 0, 0, 0], 0, 2));
    assert_eq!(told(&fx.world, first), ([0x18, 0, 0, 0], 0, 2));
    assert_eq!(told(&fx.world, third), ([0x05, 0, 0, 0], 0, 1));
}

#[test]
fn a_group_joined_takes_its_new_leads_facing_and_turns_its_places_back() {
    // As above, with the second puddle facing 2: the first and the third
    // take that facing, and their places in the group are their offsets
    // turned by -2 (0x42369d with 6): (x, y) becomes (y, -x).
    let mut fx = lot();
    puddles(&mut fx);
    fx.lamp_mut().position = [40, 40, 1];
    create_where_the_lamp_stands(&mut fx, PUDDLE).unwrap();
    let first = newest(&fx);
    fx.lamp_mut().position = [56, 56, 1];
    create_where_the_lamp_stands(&mut fx, PUDDLE).unwrap();
    let second = newest(&fx);
    fx.world.objects.get_mut(&second).unwrap().data[1] = 2;
    fx.lamp_mut().position = [33, 63, 1];
    create_where_the_lamp_stands(&mut fx, PUDDLE).unwrap();
    let third = newest(&fx);
    for (id, offset, place) in [(first, [-1, -1], [-1, 1]), (third, [-1, 0], [0, 1])] {
        let object = &fx.world.objects[&id];
        assert_eq!(object.data[1], 2, "{id}");
        let part = object.multipart.as_ref().unwrap();
        assert_eq!(part.offset[..2], offset, "{id}");
        assert_eq!(part.group[..2], place, "{id}");
    }
    // Bit d of a mask is direction (2 + d) % 8 now.
    assert_eq!(told(&fx.world, second).0[0], 0x30);
    assert_eq!(told(&fx.world, first).0[0], 0x06);
    assert_eq!(told(&fx.world, third).0[0], 0x41);
}

#[test]
fn a_dynamic_group_is_placed_in_the_middle_of_its_tile() {
    let mut fx = lot();
    puddles(&mut fx);
    fx.lamp_mut().position = [33, 63, 1];
    assert_eq!(create_where_the_lamp_stands(&mut fx, PUDDLE), Ok(true));
    assert_eq!(fx.world.objects[&newest(&fx)].position, [40, 56, 1]);
}
