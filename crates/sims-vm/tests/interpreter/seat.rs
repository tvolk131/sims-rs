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
    go_to_with(fx, me, 0)
}

/// Runs Go To Routing Slot for the lamp's routing slot 0 as `me`, with
/// operand byte 4 `byte_4` (bit 0: the route may not end near the slot).
fn go_to_with(fx: &mut Fixture, me: u16, byte_4: u8) -> Result<bool, sims_vm::Fault> {
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            GO_TO_SLOT,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 1, 0, byte_4, 0, 0, 0],
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

/// Whether the route failed: Go To Routing Slot's false return.
fn gives_up(result: Result<bool, sims_vm::Fault>) -> bool {
    result == Ok(false)
}

/// The lamp's "Route failure feedback" tree: it writes its two arguments
/// and the lamp's use count (data word 62) to the stack object's data
/// words 51 to 53, and returns false.
const FEEDBACK_TREE: u16 = 0x1200;

fn give_feedback_tree(fx: &mut Fixture) {
    fx.add(
        LAMP_FILE,
        FEEDBACK_TREE,
        tree(vec![
            assign(scope::STACK_DATA, 51, scope::PARAMETERS, 0, 1),
            assign(scope::STACK_DATA, 52, scope::PARAMETERS, 1, 2),
            assign(scope::STACK_DATA, 53, scope::STACK_DATA, 62, RETURN_FALSE),
        ]),
    );
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .names
        .insert(FEEDBACK_TREE, "Route failure feedback".to_owned());
}

/// The lamp's data words 51 to 53, which the feedback tree writes.
fn feedback(fx: &Fixture) -> [i16; 3] {
    let data = &fx.lamp().data;
    [data[51], data[52], data[53]]
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
    // 0x45b3fa: person word 62 is 0, so the result is 5. The lamp has no
    // failure feedback tree, so giving up fails the route at once.
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
    eprintln!(
        "{result:?} {} {:?}",
        route_result(&fx, me),
        fx.world.routing.routes[&me].contexts.len()
    );
    assert!(matches!(trap(result), sims_vm::Trap::Unsupported(m) if m.contains("0x45c4a8")),);
    let sim = &fx.world.objects[&me];
    assert!(sim.stack.iter().all(|frame| frame.tree != SIT_TREE + 1));
    assert!(sim.stack.iter().all(|frame| frame.primitive_state != 4));
    assert!(!sat(&fx));
}

/// Puts an action at the front of `me`'s queue (flag 2), so that it wants
/// to be interrupted (0x45d7c0).
fn interrupting(fx: &mut Fixture, me: u16) {
    let person = fx
        .world
        .objects
        .get_mut(&me)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data[71] = 0;
    person.queue.insert(
        0,
        sims_formats::saved_objects::Interaction {
            flags: 2,
            ..person.active.clone()
        },
    );
}

#[test]
fn a_person_who_wants_to_be_interrupted_fails_with_result_4_if_its_animator_may_stop() {
    // 0x45b18b: an action pushed to the front of the queue (flag 2)
    // interrupts (0x45d7c0). With no animator, the loaded one's state (3)
    // may stop (0x589ead): the route fails with result 4 before any
    // search, and no failure feedback runs.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    interrupting(&mut fx, me);
    assert_eq!(go_to(&mut fx, me), Ok(false));
    assert!(!sat(&fx));
    assert_eq!(route_result(&fx, me), 4);
    // An animator in a state that may not stop (4) goes on to the seat.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    interrupting(&mut fx, me);
    fx.world.animators.insert(
        me,
        sims_sim::animator::Animator {
            state: 4,
            ..sims_sim::animator::LOADED
        },
    );
    assert_eq!(go_to(&mut fx, me), Ok(true));
    assert!(sat(&fx));
}

#[test]
fn on_the_way_to_a_door_an_interrupted_person_fails_with_result_4() {
    // The lamp is in room 2 (data word 29), so the route goes through a
    // door (step 2, 0x45afd0), which first tests for an interruption as
    // step 1 does: the route fails with result 4 before any door is
    // chosen.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.lamp_mut().data[29] = 2;
    interrupting(&mut fx, me);
    assert_eq!(go_to(&mut fx, me), Ok(false));
    assert_eq!(route_result(&fx, me), 4);
}

#[test]
fn giving_up_near_the_slot_runs_the_failure_feedback_tree() {
    // 0x45aef7: the route may end near the slot, and the slot's object,
    // the frame's stack object, has a "Route failure feedback" tree. It
    // runs on the lamp with the result (5) and the context's word +0x8c
    // (0) as its arguments, while the person uses the lamp. Person word
    // 72 is cleared, and back from the tree (step 17, 0x45afb7) the route
    // fails whatever the tree returned.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    refuse_to_sit(&mut fx);
    give_feedback_tree(&mut fx);
    fx.lamp_mut().data[62] = 3;
    fx.world
        .objects
        .get_mut(&me)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[72] = 7;
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(feedback(&fx), [5, 0, 4]);
    assert_eq!(route_result(&fx, me), 5);
    assert_eq!(fx.world.objects[&me].person.as_ref().unwrap().data[72], 0);
    // The use ended with the tree.
    assert_eq!(fx.lamp().data[62], 3);
    assert!(fx.world.routing.routes[&me].contexts.is_empty());
}

#[test]
fn a_route_that_may_not_end_near_gives_no_feedback() {
    // Operand byte 4 bit 0 clears the slot's flag 0x4000, so giving up
    // goes to step 7 at once (0x45af1d).
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    refuse_to_sit(&mut fx);
    give_feedback_tree(&mut fx);
    let before = feedback(&fx);
    assert!(gives_up(go_to_with(&mut fx, me, 1)));
    assert_eq!(feedback(&fx), before);
    assert_eq!(route_result(&fx, me), 5);
}

#[test]
fn a_feedback_tree_that_cannot_be_pushed_fails_loudly() {
    // A feedback tree with more than four arguments: 0x47484a would read
    // past the four it is given.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    refuse_to_sit(&mut fx);
    give_feedback_tree(&mut fx);
    fx.add(
        LAMP_FILE,
        FEEDBACK_TREE,
        shaped(
            0x8002,
            0,
            8,
            0,
            vec![ins(2, RETURN_TRUE, RETURN_TRUE, [0; 8])],
        ),
    );
    let fault = trap(go_to(&mut fx, me));
    assert!(
        matches!(&fault, sims_vm::Trap::Unsupported(m) if m.contains("0x45afb2")),
        "{fault:?}"
    );
    // Nothing was pushed and step 17 was not written: the original
    // writes it only after the push (0x45afac, then 0x45b6ba and
    // 0x45b9a0). The routing frame is on top, at the step it had saved.
    let person = &fx.world.objects[&me];
    let routing = person.stack.last().unwrap();
    assert_eq!(routing.tree, TREE);
    assert_eq!(routing.primitive_state, 4);
}

#[test]
fn a_pet_looks_for_no_seat() {
    // 0x4823d0: a person of kind above 1 searches with seats off
    // (0x483414's argument; the sitting weight counts as 0 at 0x4835f8).
    // Before that only a person of kind 0 or 1 searched. A slot with only
    // a standing weight is searched as for anyone else.
    let standing = |kind: i16| {
        let (mut fx, me) = lot([56, 40], 4, 1 << 2);
        set_slots(
            &mut fx,
            vec![Slot {
                standing: 5,
                sitting: 0,
                ..seats_slot()
            }],
        );
        let person = fx
            .world
            .objects
            .get_mut(&me)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        person.data[KIND] = kind;
        // The search plans a route and the person sets off, which a
        // synchronous run cannot wait for.
        let result = go_to(&mut fx, me);
        assert_eq!(code(result), Some(0x44f), "kind {kind}");
        format!("{:?}", fx.world.routing.routes[&me])
    };
    assert_eq!(standing(2), standing(0));
    // With a sitting weight, the route then drops the slot's seats for
    // the pet (0x46de84), which is not ported.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    let person = fx
        .world
        .objects
        .get_mut(&me)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.data[KIND] = 2;
    let result = go_to(&mut fx, me);
    eprintln!(
        "{result:?} {} {:?}",
        route_result(&fx, me),
        fx.world.routing.routes[&me].contexts.len()
    );
    assert!(matches!(trap(result), sims_vm::Trap::Unsupported(ref m) if m.contains("0x46de84")),);
    assert!(!sat(&fx));
}

/// The seat's Stand tree (function 27).
const STAND_TREE: u16 = 0x1102;
const STAND: usize = 27;
/// Scope 18: my person data.
const MY_PERSON_DATA: u8 = 18;

/// Sits the person in the seat (posture 1, person word 0), held in its
/// container slot, with `returns` the seat's Stand tree's branch, or no
/// Stand function. The Stand tree writes its first argument into the
/// seat's data word 61, and stands the person (posture 0).
fn seated(fx: &mut Fixture, me: u16, stand: Option<u8>) {
    let person = fx.world.objects.get_mut(&me).unwrap();
    (person.data[14], person.data[26]) = (1, SEAT as i16);
    person.person.as_mut().unwrap().data[0] = 1;
    fx.world.objects.get_mut(&SEAT).unwrap().slots[1].object = me as i16;
    let Some(returns) = stand else {
        return;
    };
    let file = fx.catalog.object(SEAT_GUID).unwrap().resource;
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        STAND + 1,
        ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[STAND].action = STAND_TREE;
    fx.add(
        file,
        STAND_TREE,
        tree(vec![
            assign(scope::STACK_DATA, 61, scope::PARAMETERS, 0, 1),
            assign(MY_PERSON_DATA, 0, scope::LITERAL, 0, returns),
        ]),
    );
}

/// The person's posture, person word 0.
fn posture(fx: &Fixture, me: u16) -> i16 {
    fx.world.objects[&me].person.as_ref().unwrap().data[0]
}

#[test]
fn a_person_already_in_the_seat_beside_a_place_is_there() {
    // 0x45b1d7, 0x48177b: the person sits in the seat a candidate names,
    // so the route succeeds without the Sit tree running again.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    seated(&mut fx, me, None);
    assert_eq!(go_to(&mut fx, me), Ok(true));
    assert!(!sat(&fx));
    assert_eq!(posture(&fx, me), 1);
}

#[test]
fn a_seated_person_stands_up_through_the_seats_stand_tree_first() {
    // 0x45b1ef: the seat faces away, so no candidate names it; the
    // person runs its Stand tree (0x45b6d4) with the seat as stack
    // object, and when it returns true with the person standing
    // (0x45b439) the candidates are found again. There are none, so the
    // route gives up with result 10.
    let (mut fx, me) = lot([56, 40], 0, 1 << 2);
    seated(&mut fx, me, Some(RETURN_TRUE));
    fx.world.objects.get_mut(&SEAT).unwrap().data[61] = -99;
    assert!(gives_up(go_to(&mut fx, me)));
    assert_ne!(fx.world.objects[&SEAT].data[61], -99);
    assert_eq!(posture(&fx, me), 0);
    assert_eq!(route_result(&fx, me), 10);
    // A Stand tree that returns false gives up with result 6.
    let (mut fx, me) = lot([56, 40], 0, 1 << 2);
    seated(&mut fx, me, Some(RETURN_FALSE));
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(route_result(&fx, me), 6);
    // A seat with no Stand function fails the route at once.
    let (mut fx, me) = lot([56, 40], 0, 1 << 2);
    seated(&mut fx, me, None);
    assert_eq!(go_to(&mut fx, me), Ok(false));
    assert_eq!(posture(&fx, me), 1);
}

#[test]
fn with_no_door_to_go_through_the_route_gives_up_with_result_2() {
    // The lamp is in room 2, so the route goes through a door (step 2);
    // the lot has none to choose (0x45c7c2 is false), so it gives up
    // with result 2 (0x45b036).
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.lamp_mut().data[29] = 2;
    assert!(gives_up(go_to(&mut fx, me)));
    assert!(!sat(&fx));
    assert_eq!(route_result(&fx, me), 2);
}

/// Go To Relative Position (opcode 27).
const GO_TO_RELATIVE: u16 = 27;

/// Runs Go To Relative Position as `me` with stack object `target`, to
/// any place (location and facing 0xfe).
fn go_relative(fx: &mut Fixture, me: u16, target: i16) -> Result<bool, sims_vm::Fault> {
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            GO_TO_RELATIVE,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 0xfe, 0xfe, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(me, TREE, target, &[], 100)
}

#[test]
fn a_relative_route_whose_stack_object_is_gone_takes_the_false_branch_on_its_first_run() {
    // 0x45adb5: before any context is made (0x45ae65).
    for target in [0, 99] {
        let (mut fx, me) = lot([56, 40], 4, 1 << 2);
        assert_eq!(go_relative(&mut fx, me, target), Ok(false), "{target}");
        assert!(
            fx.world
                .routing
                .routes
                .get(&me)
                .is_none_or(|route| route.contexts.is_empty())
        );
    }
}

/// The lot with the lamp a door (definition word 9 8) in room 0, the
/// person being in room 1, and the person already on a route whose
/// context goes through door `door`.
fn to_a_door(door: u16) -> (Fixture, u16) {
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.words(LAMP_FILE)[9] = 8;
    fx.lamp_mut().data[29] = 0;
    let mut outer = crate::visits::context(me);
    outer.word_68 = i32::from(door);
    outer.slot.flags |= 0x4000;
    fx.world.routing.routes.insert(
        me,
        sims_sim::route::Route {
            contexts: vec![outer],
            path: Vec::new(),
        },
    );
    (fx, me)
}

#[test]
fn a_route_to_a_door_one_of_my_routes_goes_through_fails_with_result_1() {
    // 0x45b140, 0x45b176: the slot is in another room, and its object is
    // a door the outer route goes through (context +0x68). The route
    // fails at once: result 1, the false branch, and its context goes.
    let (mut fx, me) = to_a_door(LAMP);
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(route_result(&fx, me), 1);
    let route = &fx.world.routing.routes[&me];
    assert_eq!(route.contexts.len(), 1);
    assert_eq!(route.contexts[0].word_68, i32::from(LAMP));
}

#[test]
fn a_route_to_a_door_no_route_of_mine_goes_through_looks_for_a_way_in() {
    // Through another door, the route goes on to choose a door into the
    // slot's room (0x45b140); the lot has none, so it gives up with
    // result 2 (0x45b036) rather than failing as above.
    let (mut fx, me) = to_a_door(SEAT);
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(route_result(&fx, me), 2);
}

#[test]
fn a_person_going_through_everything_plans_to_a_slot_outside_its_walled_room() {
    // Bit 0 of person word 68: Go To Routing Slot looks for no door
    // (0x45b140 is skipped), and the planner neither fails for a slot in
    // another room (0x4824a0) nor keeps the room's own rectangles, walls
    // included (0x4827e3). The lamp stands outside room 1, on tile (4, 2);
    // the path leaves the walled room across its wall, and the route is
    // under way, so the synchronous run cannot wait (error 0x44f).
    // **Unverified** path: no capture covers such a person.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    set_slots(
        &mut fx,
        vec![Slot {
            standing: 5,
            sitting: 0,
            ..seats_slot()
        }],
    );
    fx.lamp_mut().position = [72, 40, 1];
    fx.lamp_mut().data[29] = 0;
    let person = fx.world.objects.get_mut(&me).unwrap();
    person.person.as_mut().unwrap().data[68] = 1;
    assert_eq!(code(go_to(&mut fx, me)), Some(0x44f));
    assert_eq!(
        fx.world.routing.routes[&me].path,
        vec![[40, 40], [24, 72], [40, 72]]
    );
    // Without the bit, the route looks for a door into room 0; the lot
    // has none, so it gives up with result 2 (0x45b036).
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.lamp_mut().position = [72, 40, 1];
    fx.lamp_mut().data[29] = 0;
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(route_result(&fx, me), 2);
}

#[test]
fn a_search_that_reaches_no_goal_gives_up_with_result_3() {
    // 0x482b35: the person stands at (72, 72), outside the walls of room
    // 1, which its room word names, so the room's own rectangles leave no
    // way to any of the four candidates (all with positive scores). The
    // search ends with no goal: the chosen candidate is forgotten
    // (0x481745; none was chosen, so nothing changes), the context's
    // result is 3 and the planner answers false (0x482e6d). The route
    // gives up with that result and its context goes.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    set_slots(
        &mut fx,
        vec![Slot {
            standing: 5,
            sitting: 0,
            ..seats_slot()
        }],
    );
    let person = fx.world.objects.get_mut(&me).unwrap();
    person.position = [72, 72, 1];
    person.footprint = [69, 69, 75, 75];
    assert!(gives_up(go_to(&mut fx, me)));
    assert_eq!(route_result(&fx, me), 3);
    let route = &fx.world.routing.routes[&me];
    assert!(route.contexts.is_empty() && route.path.is_empty());
}

#[test]
fn a_person_in_no_room_has_no_way() {
    // 0x45c3ca to 0x45c3e4: the person's room (data word 29) is looked up
    // in the lot's rooms (0x466fcc gives null for one not there). A person
    // in no room, such as one off the lot, gets the answer 0 (0x45c4c7) as
    // when the planner finds no way, and the candidates are dropped.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    let room = 77;
    assert!(!fx.world.rooms.rooms.contains_key(&room));
    fx.world.objects.get_mut(&me).unwrap().data[29] = room;
    // The lamp too, so the route goes to the slot and not through a door.
    fx.lamp_mut().data[29] = room;
    assert!(gives_up(go_to(&mut fx, me)));
    assert!(!sat(&fx));
    // The planner never ran, so its result (3) was not written.
    assert_ne!(route_result(&fx, me), 3);
    let route = &fx.world.routing.routes[&me];
    assert!(route.contexts.is_empty() && route.path.is_empty());
}

/// A route context as the load makes it for a person saved part way
/// through a route (0x460443): it names no object.
fn loaded_context() -> sims_sim::route::Context {
    sims_sim::route::Context {
        candidates: Vec::new(),
        slot: sims_sim::route::RouteSlot {
            offset: [0; 3],
            number: -1,
            standing: 0,
            sitting: 0,
            ground: 0,
            flags: 0,
            snap_target: -1,
            proximity: [16; 3],
            gradient: 0x3e40_0000,
            facing: -2,
            resolution: 16,
        },
        target: 0,
        person: 0,
        chosen: -1,
        best: -1,
        word_58: 0,
        time: 0,
        saved: [0; 2],
        word_68: 0,
        start: [0; 3],
        byte_78: true,
        objects: [0; 2],
        byte_84: false,
        result: 0,
        word_8c: 0,
        byte_8e: false,
        word_94: 0,
        limit: 0x30,
        action: 0,
    }
}

/// `me`'s thread at Go To Routing Slot for the lamp's slot 0, at route
/// step `state`. Its true branch marks `me`'s data word 50 with 1 and its
/// false branch with 2; either then sleeps.
fn walking(fx: &mut Fixture, me: u16, state: i32) {
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        shaped(
            0x8002,
            0,
            1,
            0,
            vec![
                ins(GO_TO_SLOT, 1, 2, [0, 0, 1, 0, 0, 0, 0, 0]),
                assign(scope::MY_DATA, MARK as i16, scope::LITERAL, 1, 3),
                assign(scope::MY_DATA, MARK as i16, scope::LITERAL, 2, 3),
                ins(0, RETURN_TRUE, RETURN_FALSE, [0; 8]),
            ],
        ),
    );
    fx.thread(me, TREE, &[100]);
    let frame = &mut fx.world.objects.get_mut(&me).unwrap().stack[0];
    frame.stack_object = LAMP as i16;
    frame.primitive_state = state;
}

fn tick(fx: &mut Fixture, me: u16) {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.tick_object(&mut fx.world, me).unwrap();
}

#[test]
fn a_route_context_the_load_made_is_popped_and_the_route_starts_again() {
    // 0x45ae71: the person was saved at route step 3 with two contexts.
    // The run pops the last, which names no object, puts the step back to
    // 0 and yields; the next run starts the route again on top of the
    // context left.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    walking(&mut fx, me, 3);
    fx.world.routing.routes.entry(me).or_default().contexts = vec![loaded_context(); 2];
    tick(&mut fx, me);
    let frame = &fx.world.objects[&me].stack[0];
    assert_eq!((frame.node, frame.primitive_state), (0, 0));
    assert_eq!(fx.world.routing.routes[&me].contexts, [loaded_context()]);
    assert_eq!(fx.world.objects[&me].data[MARK], 0);
    // The route runs as a fresh one: the person sits in the seat, the
    // route's context is popped and the true branch is taken.
    tick(&mut fx, me);
    assert!(sat(&fx));
    assert_eq!(fx.world.routing.routes[&me].contexts, [loaded_context()]);
    assert_eq!(fx.world.objects[&me].data[MARK], 1);
}

#[test]
fn a_route_with_no_context_or_no_object_gives_false() {
    // 0x45ae65: past step 0 with no context left (a reset erased them),
    // the run gives false and writes nothing.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    walking(&mut fx, me, 3);
    tick(&mut fx, me);
    assert_eq!(fx.world.objects[&me].data[MARK], 2);
    assert!(!fx.world.routing.routes.contains_key(&me));
    // At step 0 with the stack object off the lot, the same.
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
    fx.lamp_mut().position = [-16, -16, 1];
    assert_eq!(go_to(&mut fx, me), Ok(false));
    assert!(!fx.world.routing.routes.contains_key(&me));
}

/// The lot with `me` part way through a route to the lamp's slot 0: its
/// thread is at Go To Routing Slot in step `step`, with the lamp as stack
/// object and the route's context made, and its animator arriving as
/// `arrival` (1 or -1: under way, so it may not stop).
fn mid_route(step: i32, arrival: i32) -> (Fixture, u16) {
    let (mut fx, me) = lot([56, 40], 4, 1 << 2);
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
    fx.thread(me, TREE, &[]);
    let frame = &mut fx.world.objects.get_mut(&me).unwrap().stack[0];
    frame.stack_object = LAMP as i16;
    frame.primitive_state = step;
    let mut ctx = crate::visits::context(me);
    ctx.byte_78 = true;
    fx.world.routing.routes.insert(
        me,
        sims_sim::route::Route {
            contexts: vec![ctx],
            path: Vec::new(),
        },
    );
    fx.world.animators.insert(
        me,
        sims_sim::animator::Animator {
            arrival,
            ..sims_sim::animator::LOADED
        },
    );
    (fx, me)
}

fn turn(fx: &mut Fixture, me: u16) -> Result<sims_vm::Status, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, me, 100)
}

#[test]
fn a_route_whose_stack_object_is_lost_fails() {
    // 0x45ae8d: on a later run, a stack object that is gone, or off the
    // lot, sends a route not under way to step 19, which fails it
    // (0x45afc4) at once: unlike step 20, it does not wait for an
    // animator that is under way. The route ends with result 1, the false
    // branch, and its context goes; the end of the route asks the
    // animator to stop (0x589e2d), and the request is kept.
    for off in [None, Some([-16, -16])] {
        let (mut fx, me) = mid_route(1, 1);
        if let Some(at) = off {
            fx.lamp_mut().position = [at[0], at[1], 1];
        } else {
            fx.world.objects.remove(&LAMP);
            fx.world.order.retain(|&id| id != LAMP);
        }
        assert_eq!(
            turn(&mut fx, me),
            Ok(sims_vm::Status::Returned(false)),
            "{off:?}"
        );
        assert!(fx.world.routing.routes[&me].contexts.is_empty(), "{off:?}");
        assert_eq!(route_result(&fx, me), 1, "{off:?}");
        assert!(fx.world.stops_asked.contains(&me), "{off:?}");
    }
}

#[test]
fn a_route_under_way_whose_stack_object_is_lost_asks_its_animator_to_stop() {
    // Step 3 goes to step 20 (0x45b352): the animator is asked to stop
    // (0x589e2d). One under way (arrival 1) does not, so the run yields
    // in step 20 and the request is kept.
    let (mut fx, me) = mid_route(3, 1);
    fx.world.objects.remove(&LAMP);
    fx.world.order.retain(|&id| id != LAMP);
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Yielded));
    assert_eq!(fx.world.objects[&me].stack[0].primitive_state, 20);
    assert!(fx.world.stops_asked.contains(&me));
    assert_eq!(fx.world.routing.routes[&me].contexts.len(), 1);
    // The next run tests the stack object first (0x45ae8d): still lost,
    // the saved step 20 becomes 19, and the route fails without waiting
    // for the animator, still under way.
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Returned(false)));
    assert_eq!(fx.world.animators[&me].arrival, 1);
    assert!(fx.world.routing.routes[&me].contexts.is_empty());
}

#[test]
fn a_stack_object_past_the_lots_size_is_lost() {
    // 0x432595 tests the object's place with 0x422632, against the lot's
    // size less 1: the lot's last tile is off it, though inside the
    // arrays' 64 tiles. Under way, the route goes to step 20 and yields.
    let (mut fx, me) = mid_route(3, 1);
    let last = i32::from(fx.world.size) - 1;
    assert!(last < 63);
    fx.lamp_mut().position = [last * 16 + 8, 40, 1];
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Yielded));
    assert_eq!(fx.world.objects[&me].stack[0].primitive_state, 20);
}

#[test]
fn step_20_goes_on_to_fail_once_its_animator_stops() {
    // A route saved in step 20 whose stack object is back: step 20 asks
    // the animator, which may stop (arrival 0), stops it (its segment
    // goes to -1) and goes on to step 19, which fails the route.
    let (mut fx, me) = mid_route(20, 0);
    fx.world.animators.get_mut(&me).unwrap().motion.segment = 5;
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Returned(false)));
    assert_eq!(fx.world.animators[&me].motion.segment, -1);
    assert_eq!(route_result(&fx, me), 1);
    assert!(fx.world.routing.routes[&me].contexts.is_empty());
}

#[test]
fn a_lost_route_sets_its_contexts_byte_0x78_again() {
    // 0x45aebb: with byte +0x78 clear, any step but 2 and 3 sets it again
    // and goes on; written back as step 20 yields.
    let (mut fx, me) = mid_route(3, 1);
    fx.world.objects.remove(&LAMP);
    let route = fx.world.routing.routes.get_mut(&me).unwrap();
    route.contexts[0].byte_78 = false;
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Yielded));
    assert!(fx.world.routing.routes[&me].contexts[0].byte_78);
    // Step 2 with the byte clear is not ported.
    let (mut fx, me) = mid_route(2, 0);
    let route = fx.world.routing.routes.get_mut(&me).unwrap();
    route.contexts[0].byte_78 = false;
    let result = turn(&mut fx, me);
    assert!(
        matches!(result, Err(ref fault) if format!("{fault}").contains("0x45aea5")),
        "{result:?}"
    );
}

/// Another person, whom the route of `mid_route` asked (context +0x80) to
/// do its action numbered 77 (+0x90).
const ASKED: u16 = 60;

fn asked_to_move(fx: &mut Fixture, me: u16) {
    let asked = fx.add_sim(ASKED);
    let person = asked.person.as_mut().unwrap();
    person.data[33] = 40;
    let ctx = fx.world.routing.routes.get_mut(&me).unwrap();
    let ctx = ctx.contexts.last_mut().unwrap();
    ctx.objects[1] = ASKED;
    ctx.action = 77;
}

#[test]
fn a_route_ending_while_the_asked_person_runs_the_action_cancels_it() {
    // 0x45b914 calls the asked person's 0x45d5f8 with the action's
    // number; the person runs it (+0xa84), so 0x45d617 lowers its
    // priority (person word 33) to 0 and sets the running action's flag
    // 0x100, as the player's cancel of a running action does.
    let (mut fx, me) = mid_route(1, 0);
    fx.world.objects.remove(&LAMP);
    asked_to_move(&mut fx, me);
    let asked = fx.world.objects.get_mut(&ASKED).unwrap();
    let person = asked.person.as_mut().unwrap();
    person.active.uid = 77;
    let flags = person.active.flags;
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Returned(false)));
    let person = fx.world.objects[&ASKED].person.as_ref().unwrap();
    assert_eq!(person.data[33], 0);
    assert_eq!(person.active.flags, flags | 0x100);
    assert_eq!(person.active.uid, 77);
    // The Sim's interaction table has no entry with menu number 33
    // (0x41f0ff), so nothing is queued.
    assert!(person.queue.is_empty());
}

#[test]
fn a_route_ending_takes_the_asked_action_off_the_queue() {
    // 0x45d66e: queued, the action is dropped; the running one is left.
    let (mut fx, me) = mid_route(1, 0);
    fx.world.objects.remove(&LAMP);
    asked_to_move(&mut fx, me);
    let person = fx
        .world
        .objects
        .get_mut(&ASKED)
        .unwrap()
        .person
        .as_mut()
        .unwrap();
    person.active.uid = 76;
    let mut queued = person.active.clone();
    queued.uid = 77;
    queued.target = 0;
    person.queue.push(queued);
    let flags = person.active.flags;
    assert_eq!(turn(&mut fx, me), Ok(sims_vm::Status::Returned(false)));
    let person = fx.world.objects[&ASKED].person.as_ref().unwrap();
    assert!(person.queue.is_empty());
    assert_eq!((person.active.uid, person.active.flags), (76, flags));
    assert_eq!(person.data[33], 40);
}
