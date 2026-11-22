//! Set To Next: each mode over state the lot holds, exhaustion, the
//! destination, and the modes that fail loudly. The lamp runs.

use crate::support::*;
use sims_sim::neighbors::{Neighbor, Neighbors};
use sims_vm::{Fault, Trap};

/// A neighborhood whose record vector holds `guids` in order; None is an
/// empty slot. Each record's ID is its place plus one.
fn neighborhood(guids: &[Option<u32>]) -> sims_sim::CowBox<Neighbors> {
    Neighbors {
        slots: guids
            .iter()
            .enumerate()
            .map(|(place, guid)| {
                guid.map(|guid| Neighbor {
                    id: place as i16 + 1,
                    guid,
                    revision: 5,
                    data: vec![0; 256],
                    relationships: Vec::new(),
                    friends: 0,
                    stale: true,
                })
            })
            .collect(),
        ..Neighbors::default()
    }
    .into()
}

/// Mode 5 (0x425b15): the next occupied slot after the cursor, by place
/// plus one, skipping empty slots. A cursor that is negative or past the
/// last slot gives 0, which is the false branch with 0 stored.
#[test]
fn mode_5_walks_the_neighborhood_records() {
    let mut fx = Fixture::new();
    fx.world.neighbors = neighborhood(&[Some(LAMP_GUID), None, Some(SIM_GUID), Some(0x5555)]);
    assert_eq!(scan(&mut fx, 5), (vec![1, 3, 4], 0));
    for cursor in [-1, 4, 40] {
        assert_eq!(step(&mut fx, 5, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], 0);
    }
}

/// Mode 7: the next record, as mode 5 walks them, whose type has the
/// operands' GUID. Each record passed is stored on the way; running out
/// stores 0 and takes the false branch.
#[test]
fn mode_7_walks_the_records_of_one_person_type() {
    let guid = SIM_GUID.to_le_bytes();
    let operands = [
        guid[0],
        guid[1],
        guid[2],
        guid[3],
        0x80 | 7,
        scope::TEMPS,
        0,
        0,
    ];
    let mut fx = Fixture::new();
    fx.world.neighbors = neighborhood(&[Some(SIM_GUID), Some(LAMP_GUID), None, Some(SIM_GUID)]);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(31, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    let mut visited = Vec::new();
    fx.lamp_mut().temps[0] = 0;
    while fx.run().unwrap() {
        visited.push(fx.lamp().temps[0]);
    }
    assert_eq!((visited, fx.lamp().temps[0]), (vec![1, 4], 0));
    // A record whose GUID no definition has is not compared: the original
    // compares the GUID of the definition the record is linked to.
    fx.world.neighbors = neighborhood(&[Some(0x5555), Some(SIM_GUID)]);
    fx.lamp_mut().temps[0] = 0;
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}

/// Operands with the cursor in temp `temp`.
fn in_temp(mode: u8, temp: u8) -> [u8; 8] {
    [0, 0, 0, 0, 0x80 | mode, scope::TEMPS, 0, temp]
}

/// Runs Set To Next with the cursor in temp 0, starting at `cursor`.
fn step(fx: &mut Fixture, mode: u8, cursor: i16) -> Result<bool, Fault> {
    fx.lamp_mut().temps[0] = cursor;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(31, RETURN_TRUE, RETURN_FALSE, in_temp(mode, 0))]),
    );
    fx.run()
}

/// The cursors a scan visits from zero, ending with the cursor left
/// behind once it is exhausted.
fn scan(fx: &mut Fixture, mode: u8) -> (Vec<i16>, i16) {
    let mut visited = Vec::new();
    let mut cursor = 0;
    while step(fx, mode, cursor).unwrap_or_else(|f| panic!("{f}")) {
        cursor = fx.lamp().temps[0];
        visited.push(cursor);
        assert!(visited.len() < 20, "the scan does not end");
    }
    (visited, fx.lamp().temps[0])
}

#[test]
fn mode_0_walks_registration_order() {
    let mut fx = Fixture::new();
    fx.add_object(5);
    fx.world.order = vec![5, 2, 1];
    // Exhaustion keeps the last cursor.
    assert_eq!(scan(&mut fx, 0), (vec![5, 2, 1], 1));
    // A cursor that is not registered is exhausted at once and kept.
    assert_eq!(step(&mut fx, 0, 40), Ok(false));
    assert_eq!(fx.lamp().temps[0], 40);
}

#[test]
fn the_stack_object_is_the_default_cursor() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            ins(31, 1, RETURN_FALSE, [0; 8]),
            expression(
                scope::STACK_OBJECT,
                0,
                op::EQUAL,
                scope::LITERAL,
                SIM as i16,
            ),
        ]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, LAMP as i16, &[], 10), Ok(true));
}

#[test]
fn mode_1_walks_people_and_ends_at_zero() {
    let mut fx = Fixture::new();
    fx.add_object(3);
    fx.add_sim(4);
    assert_eq!(scan(&mut fx, 1), (vec![2, 4], 0));
    // A cursor that is not a person also ends at zero.
    assert_eq!(step(&mut fx, 1, LAMP as i16), Ok(false));
    assert_eq!(fx.lamp().temps[0], 0);
}

#[test]
fn modes_1_and_12_walk_the_person_vector_not_registration_order() {
    let mut fx = Fixture::new();
    fx.add_sim(4);
    fx.add_sim(6);
    // A person made at run time heads the registration list but ends the
    // person vector.
    fx.world.order = vec![6, 1, 2, 4];
    fx.world.people = vec![2, 4, 6];
    assert_eq!(scan(&mut fx, 1), (vec![2, 4, 6], 0));
    // Mode 12 wraps round the same vector, within the family.
    for id in [2, 4, 6] {
        fx.world
            .objects
            .get_mut(&id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .data[61] = 1;
    }
    assert_eq!(step(&mut fx, 12, 6), Ok(true));
    assert_eq!(fx.lamp().temps[0], 2);
    // A vector naming an object that is not a person, or one that is
    // gone, is invalid, not skipped.
    fx.world.people.push(1);
    let fault = step(&mut fx, 1, 0).unwrap_err();
    assert!(
        matches!(fault.trap, Trap::Invalid(ref m) if m.contains("not a person")),
        "{fault}"
    );
    fx.world.people.pop();
    fx.world.people.push(9);
    let fault = step(&mut fx, 1, 0).unwrap_err();
    assert!(
        matches!(fault.trap, Trap::Invalid(ref m) if m.contains("gone")),
        "{fault}"
    );
}

#[test]
fn registration_order_is_validated_before_cursor_or_mode_6_parameter() {
    let mut fx = Fixture::new();
    fx.world.order = vec![LAMP, SIM, 99];

    // Mode 0 from cursor 0 and from a cursor not in the list must both trap on
    // the missing registered object 99.
    for cursor in [0i16, 40] {
        let fault = step(&mut fx, 0, cursor).unwrap_err();
        assert!(
            matches!(fault.trap, Trap::Invalid(ref m) if m.contains("registered object 99 does not exist")),
            "cursor {cursor}: {fault}"
        );
    }

    // Mode 6 with no parameters must still trap on the missing registered
    // object before checking parameter 0.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(31, RETURN_TRUE, RETURN_FALSE, in_temp(6, 0))]),
    );
    let fault = fx.run_with(LAMP, 4096, 0, &[], 10).unwrap_err();
    assert!(
        matches!(fault.trap, Trap::Invalid(ref m) if m.contains("registered object 99 does not exist")),
        "{fault}"
    );
}

#[test]
fn mode_2_skips_people() {
    let mut fx = Fixture::new();
    fx.add_object(3);
    fx.add_sim(4);
    fx.add_object(5);
    assert_eq!(scan(&mut fx, 2), (vec![1, 3, 5], 5));
}

#[test]
fn mode_6_matches_data_59_against_parameter_0() {
    let mut fx = Fixture::new();
    for id in [3, 4, 5] {
        fx.add_object(id).data[59] = if id == 4 { 9 } else { 7 };
    }
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(31, RETURN_TRUE, RETURN_FALSE, in_temp(6, 0))]),
    );
    let mut visited = Vec::new();
    while fx.run_with(LAMP, 4096, 0, &[7], 10).unwrap() {
        visited.push(fx.lamp().temps[0]);
    }
    assert_eq!(visited, [3, 5]);
    assert_eq!(fx.lamp().temps[0], 5);
}

#[test]
fn mode_6_needs_a_parameter_only_with_objects_left() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            0,
            0,
            vec![ins(31, RETURN_TRUE, RETURN_FALSE, in_temp(6, 0))],
        ),
    );
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
    // From the last object nothing is compared.
    fx.lamp_mut().temps[0] = SIM as i16;
    assert_eq!(fx.run(), Ok(false));
}

/// A tile type: definition word 10 (the group) set.
const TILE_GUID: u32 = 0x3333;

/// Registers object `id` of the tile type, with `parent` as its lead
/// (0 for the lead itself), first in registration order as a new object
/// goes.
fn add_tile(fx: &mut Fixture, id: u16, parent: i16) {
    let mut tile = object(id, TILE_GUID);
    tile.multipart = Some(sims_formats::saved_objects::Multipart {
        offset: [0; 3],
        group: [0; 3],
        parent,
    });
    fx.world.order.insert(0, id);
    fx.world.objects.insert(id, tile);
}

fn tile_type(fx: &mut Fixture) {
    let resource = fx.add_type(TILE_GUID, 4, 0, 0);
    fx.program
        .libraries
        .get_mut(&resource)
        .unwrap()
        .definitions
        .get_mut(&128)
        .unwrap()[10] = 7;
}

/// Mode 3 (0x44ec79) follows the tiles' next links (+0x18c), the order
/// they joined their lead's list, and from the last tile goes back to the
/// lead (+0x190). Registration order, where each new object goes first,
/// plays no part: here it is 12, 11, 10, and the list is 10, 11, 12.
#[test]
fn mode_3_follows_the_group_s_links() {
    let mut fx = Fixture::new();
    tile_type(&mut fx);
    add_tile(&mut fx, 10, 0);
    add_tile(&mut fx, 11, 10);
    add_tile(&mut fx, 12, 10);
    add_tile(&mut fx, 20, 0);
    fx.world.next_parts.insert(10, 11);
    fx.world.next_parts.insert(11, 12);
    for (cursor, next) in [(10, 11), (11, 12), (12, 10)] {
        assert_eq!(step(&mut fx, 3, cursor), Ok(true));
        assert_eq!(fx.lamp().temps[0], next);
    }
    // A lead with no tiles, an object whose definition word 10 is 0 (the
    // lamp), a missing object and zero all store zero (0x44ecc9).
    for cursor in [20, LAMP as i16, 77, 0, -3] {
        assert_eq!(step(&mut fx, 3, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], 0);
    }
    // Word 10 is read first: a tile of a type without it stores zero.
    let part = fx.world.objects[&12].multipart.clone();
    fx.add_object(30).multipart = part;
    assert_eq!(step(&mut fx, 3, 30), Ok(false));
    assert_eq!(fx.lamp().temps[0], 0);
    // An object of a type with word 10 that is no tile: the original reads
    // through the null cast (0x44ecaa).
    fx.world.objects.get_mut(&12).unwrap().multipart = None;
    assert!(matches!(trap(step(&mut fx, 3, 12)), Trap::Unsupported(m) if m.contains("0x44ecaa")));
}

#[test]
fn mode_12_cycles_through_a_family() {
    let mut fx = Fixture::new();
    let family = |fx: &mut Fixture, id: u16, family: i16| {
        let sim = if id == SIM {
            fx.sim_mut()
        } else {
            fx.add_sim(id)
        };
        sim.person.as_mut().unwrap().data[61] = family;
    };
    family(&mut fx, SIM, 5);
    family(&mut fx, 3, 6);
    family(&mut fx, 4, 5);
    assert_eq!(step(&mut fx, 12, 2), Ok(true));
    assert_eq!(fx.lamp().temps[0], 4);
    // It wraps around the person vector.
    assert_eq!(step(&mut fx, 12, 4), Ok(true));
    assert_eq!(fx.lamp().temps[0], 2);
    // A family of one, and zero, are exhausted and keep the cursor.
    for cursor in [3, 0] {
        assert_eq!(step(&mut fx, 12, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], cursor);
    }
    // A cursor that is not a person.
    assert_eq!(code(step(&mut fx, 12, LAMP as i16)), Some(0x1c));
    assert_eq!(code(step(&mut fx, 12, 40)), Some(0x1c));
}

#[test]
fn the_destination_must_take_a_write() {
    let mut fx = Fixture::new();
    for mode in [0, 4] {
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                31,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 0, 0, 0, 0x80 | mode, scope::LITERAL, 0, 0],
            )]),
        );
        assert_eq!(code(fx.run()), Some(0x13));
    }
    // Out of bounds.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(31, RETURN_TRUE, RETURN_FALSE, in_temp(0, 8))]),
    );
    assert_eq!(code(fx.run()), Some(2));
}

#[test]
fn modes_over_state_the_lot_lacks_fail_loudly() {
    // Mode 10 is here because the fixture's program has no career list.
    for mode in [10, 13, 127] {
        let mut fx = Fixture::new();
        let text = trap(step(&mut fx, mode, 0)).to_string();
        assert!(
            text.contains(&format!("Set To Next mode {mode} ")),
            "{text}"
        );
        assert_eq!(fx.lamp().temps[0], 0);
    }
}

#[test]
fn mode_11_takes_the_false_branch_with_no_other_lot() {
    // The neighborhood's other lots (+0x11c) are empty in every capture
    // of the original, so the walk ends at once from any cursor and
    // stores nothing.
    for cursor in [0, 3, -1] {
        let mut fx = Fixture::new();
        assert_eq!(step(&mut fx, 11, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], cursor);
    }
}
