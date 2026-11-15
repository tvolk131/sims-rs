//! Go To Routing Slot to a slot with a sitting weight: the search finds a
//! seat beside a place (0x4838e7 to 0x483a5d), and the route ends by
//! running the seat's Sit tree (0x45c430, 0x45b96b, 0x45b3fa).
//!
//! The lot is Snap's: the six-tile one with room 1 the tiles (2, 2) to
//! (3, 3), and the widget person on tile (2, 2), at (40, 40). The lamp,
//! the slot's object, stands at (40, 56) facing 0 and on no tile of the
//! object grid. Its one slot faces direction 0 and skips the place test
//! (flag 0x800). The seat is an object of its own type on tile (3, 2),
//! one tile from the nearest row of places.

use crate::snap::{TREE, set_slots, slot, snapper};
use crate::support::*;
use sims_formats::execution::{ObjectFunction, Slot};

const GO_TO_SLOT: u16 = 45;
const SEAT: u16 = 40;
const SEAT_GUID: u32 = 0x5ea7;
const SEAT_TYPE: u16 = 31;
/// The seat's Sit tree (function 26): it marks the seat's data word 50
/// and returns true.
const SIT_TREE: u16 = 0x1100;
const SIT: usize = 26;
const MARK: usize = 50;
/// Person word 62 (+0x608), the route's result.
const ROUTE_RESULT: usize = 62;
/// Person word 65 (+0x60e), the kind; above 1 searches without seats.
const KIND: usize = 65;

/// Direction 0, no place test.
const FLAGS: u32 = 0x801;

/// A slot with only a sitting weight, from one to two tiles out.
fn seats_slot() -> Slot {
    Slot {
        sitting: 5,
        flags: FLAGS,
        proximity: [16, 32, 16],
        ..slot(0)
    }
}

/// The lot, with the person able to route (data word 8 bit 3) and the
/// seat at `at`, facing `facing`, whose seat entries (definition word
/// 66) are `entries`.
fn lot(at: [i32; 2], facing: i16, entries: i16) -> (Fixture, u16) {
    let (mut fx, me) = snapper(0);
    fx.world.objects.get_mut(&me).unwrap().data[8] |= 8;
    fx.world
        .objects
        .get_mut(&me)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[KIND] = 0;
    set_slots(&mut fx, vec![seats_slot()]);
    let lamp = fx.lamp_mut();
    lamp.position = [40, 56, 1];
    lamp.data[1] = 0;
    lamp.data[29] = 1;
    let file = fx.add_type(SEAT_GUID, 4, 0, 0);
    fx.world.types.insert(SEAT_TYPE, SEAT_GUID);
    fx.words(file)[66] = entries;
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        SIT + 1,
        ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[SIT].action = SIT_TREE;
    fx.add(
        file,
        SIT_TREE,
        tree(vec![assign(
            scope::STACK_DATA,
            MARK as i16,
            scope::LITERAL,
            99,
            RETURN_TRUE,
        )]),
    );
    let seat = fx.add_object(SEAT);
    seat.guid = SEAT_GUID;
    seat.position = [at[0], at[1], 1];
    seat.data[1] = facing;
    seat.data[8] = 0;
    seat.data[29] = 1;
    // No one sits in it.
    seat.slots[1].object = 0;
    // The object grid of level 1 (array 3) is 64 tiles a side, column
    // first, two bytes a cell.
    let cell = (((at[0] >> 4) * 64 + (at[1] >> 4)) * 2) as usize;
    fx.world.arrays.get_mut(&3).unwrap().data[cell..cell + 2]
        .copy_from_slice(&(SEAT as i16).to_le_bytes());
    (fx, me)
}

/// Runs Go To Routing Slot for the lamp's routing slot 0 as `me`.
fn go_to(fx: &mut Fixture, me: u16) -> Result<bool, sims_vm::Fault> {
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            GO_TO_SLOT,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(me, TREE, LAMP as i16, &[], 100)
}

/// The route's result, person word 62.
fn route_result(fx: &Fixture, me: u16) -> i16 {
    fx.world.objects[&me].person.as_ref().unwrap().data[ROUTE_RESULT]
}

/// Whether the seat's Sit tree ran.
fn sat(fx: &Fixture) -> bool {
    fx.world.objects[&SEAT].data[MARK] == 99
}

/// Gives the seat's Sit tree a false return.
fn refuse_to_sit(fx: &mut Fixture) {
    let file = fx.catalog.object(SEAT_GUID).unwrap().resource;
    fx.add(
        file,
        SIT_TREE,
        tree(vec![assign(
            scope::STACK_DATA,
            MARK as i16,
            scope::LITERAL,
            99,
            RETURN_FALSE,
        )]),
    );
}

fn gives_up(result: Result<bool, sims_vm::Fault>) -> bool {
    matches!(trap(result), sims_vm::Trap::Unsupported(m) if m.contains("0x45af2a"))
}

#[test]
fn a_free_seat_beside_a_place_is_sat_in_through_its_sit_tree() {
    // The nearest row of places is one tile toward the person; from its
    // middle place, (40, 40), the step of seat entry 2 (one tile on in
    // the second coordinate) is the seat, which faces back (direction 0
    // plus 4). The route ends by running the seat's Sit tree with the
    // seat as stack object; its true return is the route's success.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    assert_eq!(go_to(&mut fx, me), Ok(true));
    assert!(sat(&fx));
    let route = &fx.world.routing.routes[&me];
    assert!(route.contexts.is_empty());
}

#[test]
fn a_sit_tree_that_returns_false_gives_up_with_result_5() {
    // 0x45b3fa: person word 62 is 0, so the result is 5. Giving up then
    // runs the failure feedback tree, which is not ported.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    refuse_to_sit(&mut fx);
    assert!(gives_up(go_to(&mut fx, me)));
    assert!(sat(&fx));
    assert_eq!(route_result(&fx, me), 5);
}

#[test]
fn a_seat_another_holds_is_taken_and_gives_result_9() {
    // The seat's container slot (its slot 1) holds the Sim.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.world.objects.get_mut(&SEAT).unwrap().slots[1].object = SIM as i16;
    assert!(gives_up(go_to(&mut fx, me)));
    assert!(!sat(&fx));
    assert_eq!(route_result(&fx, me), 9);
}

#[test]
fn seats_only_and_no_seat_gives_result_10() {
    // With no standing weight, byte 0 of the refusals starts set. A seat
    // facing another way, one with only entry 7 (whose step from no place
    // of the search reaches it), one passed over on its tile (data word 8
    // flag 0x10 without 4) and one with no Sit tree are no seat.
    type Change = fn(&mut Fixture);
    let changes: [Change; 4] = [
        |fx| fx.world.objects.get_mut(&SEAT).unwrap().data[1] = 0,
        |fx| {
            *fx.words(fx.catalog.object(SEAT_GUID).unwrap().resource)
                .get_mut(66)
                .unwrap() = 1 << 7
        },
        |fx| fx.world.objects.get_mut(&SEAT).unwrap().data[8] = 0x10,
        |fx| {
            let file = fx.catalog.object(SEAT_GUID).unwrap().resource;
            fx.program
                .libraries
                .get_mut(&file)
                .unwrap()
                .callbacks
                .get_mut(&128)
                .unwrap()[SIT]
                .action = 0;
        },
    ];
    for (i, change) in changes.into_iter().enumerate() {
        let (mut fx, me) = lot([56, 40], 4, 1 << 2);
        change(&mut fx);
        assert!(gives_up(go_to(&mut fx, me)), "change {i}");
        assert!(!sat(&fx), "change {i}");
        assert_eq!(route_result(&fx, me), 10, "change {i}");
    }
}

#[test]
fn a_seat_is_found_across_a_diagonal() {
    // The lamp faces 1, so the places and their seat steps turn an eighth
    // through the matrix (0x483d7c): from the nearest row's middle place,
    // entry 2's step lands on tile (3, 3), and the seat there faces 5.
    let (mut fx, me) = lot([56, 56], 5, 1 << 2);
    fx.lamp_mut().data[1] = 1;
    assert_eq!(go_to(&mut fx, me), Ok(true));
    assert!(sat(&fx));
}

#[test]
fn the_place_itself_comes_first_when_standing_weighs_at_least_as_much() {
    // Standing weight 5 against sitting 5: each place is tried as index
    // -1 before its seats, so the middle place is a place to stand and the
    // route walks there (a walk yields, which a synchronous run cannot).
    // Standing 4: the place is tried as index 8, after its seats.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    set_slots(
        &mut fx,
        vec![Slot {
            standing: 5,
            ..seats_slot()
        }],
    );
    let walked = go_to(&mut fx, me).unwrap_err();
    assert_eq!(walked.trap.code(), Some(1103), "{walked}");
    assert!(!sat(&fx));
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    set_slots(
        &mut fx,
        vec![Slot {
            standing: 4,
            ..seats_slot()
        }],
    );
    assert_eq!(go_to(&mut fx, me), Ok(true));
    assert!(sat(&fx));
}

#[test]
fn a_sit_tree_that_cannot_be_pushed_fails_loudly_and_leaves_the_route_as_it_was() {
    // The seat's Sit callback names a tree its file lacks. The route's
    // step 4 and its saved context are put back before the failure.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    let file = fx.catalog.object(SEAT_GUID).unwrap().resource;
    fx.program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap()[SIT]
        .action = SIT_TREE + 1;
    let result = go_to(&mut fx, me);
    assert!(matches!(trap(result), sims_vm::Trap::Unsupported(m) if m.contains("0x45c4a8")),);
    let sim = &fx.world.objects[&me];
    assert!(sim.stack.iter().all(|frame| frame.tree != SIT_TREE + 1));
    assert!(sim.stack.iter().all(|frame| frame.primitive_state != 4));
    assert!(!sat(&fx));
}
