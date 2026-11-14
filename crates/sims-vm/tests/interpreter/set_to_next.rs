//! Set To Next: each mode over state the lot holds, exhaustion, the
//! destination, and the modes that fail loudly. The lamp runs.

use crate::support::*;
use sims_sim::neighbors::{Neighbor, Neighbors};
use sims_vm::{Fault, Trap};

/// A neighborhood whose record vector holds `guids` in order; None is an
/// empty slot. Each record's ID is its place plus one.
fn neighborhood(guids: &[Option<u32>]) -> Neighbors {
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

#[test]
fn mode_3_walks_a_multi_tile_group() {
    let mut fx = Fixture::new();
    // Tiles come before their head in registration order; the chain is
    // still the head first, then its tiles in that order.
    for (id, parent) in [(12, 10), (11, 10), (10, 0), (20, 0)] {
        fx.add_object(id).multipart = Some(sims_formats::saved_objects::Multipart {
            offset: [0; 3],
            group: [0; 3],
            parent,
        });
    }
    for (cursor, next) in [(10, 12), (12, 11)] {
        assert_eq!(step(&mut fx, 3, cursor), Ok(true));
        assert_eq!(fx.lamp().temps[0], next);
    }
    // The last tile leads back to the head.
    assert_eq!(step(&mut fx, 3, 11), Ok(true));
    assert_eq!(fx.lamp().temps[0], 10);
    // A head with no tiles, an object in no group and a missing object all
    // store zero.
    for cursor in [20, LAMP as i16, 77, 0] {
        assert_eq!(step(&mut fx, 3, cursor), Ok(false));
        assert_eq!(fx.lamp().temps[0], 0);
    }
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
    for mode in [10, 11, 13, 127] {
        let mut fx = Fixture::new();
        let text = trap(step(&mut fx, mode, 0)).to_string();
        assert!(
            text.contains(&format!("Set To Next mode {mode} ")),
            "{text}"
        );
        assert_eq!(fx.lamp().temps[0], 0);
    }
}
