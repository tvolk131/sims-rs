//! The end of a load: the house is announced, a Sim is selected and gets
//! the indicator, the game goes to live mode, and the rooms are counted
//! with the objects that stand in them.

use crate::support::*;
use crate::visits::{lot, stand};
use sims_formats::saved_objects::Slot;
use sims_formats::tuning::{Constant, Constants};
use sims_sim::rooms::RoomError;
use sims_vm::{Asked, INDICATORS, Interpreter, Trap, selected};
use std::cell::RefCell;

/// The person the tests add, and the tile it stands on, inside the
/// walls: room 1.
const PERSON: u16 = 50;
const TILE: [i32; 2] = [2, 3];

/// The lot of the visit tests with a person standing on it: a resident of
/// family 7 with no date, and six empty slots.
fn lot_with_person() -> Fixture {
    let (mut fx, _) = lot();
    resident(&mut fx, PERSON, TILE);
    fx.world.globals[9] = 7;
    fx
}

/// Adds a person of the Sim's type standing on a tile of level 1, first
/// on that tile.
fn resident(fx: &mut Fixture, id: u16, tile: [i32; 2]) {
    let position = [tile[0] * 16 + 8, tile[1] * 16 + 8, 1];
    let room = fx.world.room_at(position).unwrap();
    let sim = fx.add_sim(id);
    sim.position = position;
    sim.data[29] = room;
    (sim.data[2], sim.data[3], sim.data[14], sim.data[26]) = (0, -1, -1, 0);
    // Nothing the room update reads is set: not hidden, no light, no
    // impact, no category.
    for word in [7, 34, 39, 40, 51, 59] {
        sim.data[word] = 0;
    }
    sim.slots = vec![
        Slot {
            unknown: 0,
            object: 0,
        };
        6
    ];
    let person = sim.person.as_mut().unwrap();
    (person.data[32], person.data[61], person.data[77]) = (0, 7, 0);
    let cell = ((tile[0] * 64 + tile[1]) * 2) as usize;
    let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
    assert_eq!(grid[cell..cell + 2], [0, 0]);
    grid[cell..cell + 2].copy_from_slice(&id.to_le_bytes());
}

/// Runs `step` and returns what it gave with the callbacks it asked.
fn asking<T>(
    fx: &mut Fixture,
    step: impl FnOnce(&Interpreter, &mut sims_sim::World) -> T,
) -> (T, Vec<(u16, usize)>) {
    let asked = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.asked = Some(&asked);
    let result = step(&interpreter, &mut fx.world);
    let calls = asked
        .into_inner()
        .into_iter()
        .map(|Asked { object, function }| (object, function))
        .collect();
    (result, calls)
}

fn unsupported<T: std::fmt::Debug>(result: Result<T, sims_vm::Fault>) -> String {
    match result.expect_err("expected a fault").trap {
        Trap::Unsupported(what) => what,
        other => panic!("expected an unsupported fault, got {other:?}"),
    }
}

#[test]
fn announcing_the_house_makes_the_indicators_and_clears_the_selection() {
    let mut fx = lot_with_person();
    assert_eq!(fx.world.indicators, [0, 0]);
    fx.world.globals[3] = PERSON as i16;
    let (made, _) = asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let [first, second] = fx.world.indicators;
    assert_eq!(made, Some([first, second]));
    assert_eq!(fx.world.objects[&first].guid, INDICATORS[0]);
    assert_eq!(fx.world.objects[&second].guid, INDICATORS[1]);
    // The house saved who was selected, but nobody has the mark.
    assert_eq!(fx.world.globals[3], 0);
    // With a Sim selected the registry does nothing.
    fx.world.objects.get_mut(&PERSON).unwrap().selected = true;
    fx.world.globals[3] = PERSON as i16;
    let before = fx.world.hash();
    let (made, calls) = asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!((made, calls), (None, vec![]));
    assert_eq!(fx.world.hash(), before);
}

#[test]
fn selecting_a_sim_marks_it_and_puts_the_indicator_in_its_slot() {
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let [first, second] = fx.world.indicators;
    assert_eq!(selected(&fx.world), None);
    let ((), calls) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    // The indicator is taken off the lot, which takes it off once more;
    // then it is put in the slot, and it and the Sim are told.
    assert_eq!(calls, [(first, 10), (first, 10), (first, 9), (PERSON, 9)]);
    assert_eq!(selected(&fx.world), Some(PERSON));
    assert_eq!(fx.world.globals[3], PERSON as i16);
    let person = &fx.world.objects[&PERSON];
    let slots: Vec<i16> = person.slots.iter().map(|slot| slot.object).collect();
    assert_eq!(slots, [0, 0, first as i16, 0, 0, 0]);
    let indicator = &fx.world.objects[&first];
    // Held by the Sim in slot 1 as scripts count it, the Sim's third.
    assert_eq!((indicator.data[2], indicator.data[3]), (PERSON as i16, 1));
    assert_eq!((indicator.data[14], indicator.data[26]), (2, PERSON as i16));
    assert_eq!(indicator.position, person.position);
    assert_eq!(indicator.data[29], 1);
    // The Sim is still the first object on its tile.
    assert_eq!((person.data[14], person.data[26]), (-1, 0));
    // The second indicator stays off the lot.
    assert_eq!(fx.world.objects[&second].position, [-16, -16, 1]);
    // Selecting the selected Sim again does nothing.
    let before = fx.world.hash();
    let ((), calls) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    assert_eq!(calls, []);
    assert_eq!(fx.world.hash(), before);
    // Deselecting is not ported.
    let (result, _) = asking(&mut fx, |i, world| i.select(world, None));
    assert!(unsupported(result).contains("deselecting"));
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(LAMP)));
    assert!(unsupported(result).contains("deselecting"));
}

#[test]
fn selecting_nobody_or_a_sim_that_is_off_the_lot() {
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let first = fx.world.indicators[0];
    // Nobody selected, nobody to select: nothing happens, not even to
    // global 3.
    fx.world.globals[3] = 9;
    asking(&mut fx, |i, world| i.select(world, None).unwrap());
    assert_eq!(fx.world.globals[3], 9);
    // A Sim that is off the lot gets the mark; the indicator leaves the
    // lot and goes nowhere.
    let sim = fx.world.objects.get_mut(&PERSON).unwrap();
    sim.position = [-16, -16, 1];
    let ((), calls) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    assert_eq!(calls, [(first, 10), (first, 10)]);
    assert_eq!(fx.world.globals[3], PERSON as i16);
    assert!(fx.world.objects[&PERSON].selected);
    assert_eq!(fx.world.objects[&first].data[2], 0);
    // Only a person can be selected.
    let mut fx = lot_with_person();
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(LAMP)));
    assert!(matches!(result.unwrap_err().trap, Trap::Invalid(_)));
}

#[test]
fn a_lot_without_indicators_still_selects() {
    let mut fx = lot_with_person();
    let ((), calls) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    assert_eq!(calls, []);
    assert_eq!(selected(&fx.world), Some(PERSON));
    assert_eq!(fx.world.globals[3], PERSON as i16);
}

#[test]
fn what_the_selection_does_not_port_fails() {
    // A Sim with a date, which the second indicator marks.
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let sim = fx.world.objects.get_mut(&PERSON).unwrap();
    sim.person.as_mut().unwrap().data[77] = -3;
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)));
    assert!(unsupported(result).contains("date"));
    // A Sim whose slot 1 already holds something.
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let widget = stand(&mut fx, [4, 4]);
    fx.world.objects.get_mut(&PERSON).unwrap().slots[2].object = widget as i16;
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)));
    assert!(unsupported(result).contains("slot 1"));
}

#[test]
fn an_indicator_that_leaves_the_lot_is_taken_off_twice() {
    // The widget of the visit tests leaves once; an indicator that
    // stands on the lot leaves twice, and ends off the lot all the same.
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let first = fx.world.indicators[0];
    asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    // Deselect by hand, so the indicator can be moved again.
    fx.world.objects.get_mut(&PERSON).unwrap().selected = false;
    resident(&mut fx, PERSON + 1, [4, 4]);
    let ((), calls) = asking(&mut fx, |i, world| {
        i.select(world, Some(PERSON + 1)).unwrap()
    });
    // Leaving the first Sim's slot tells that Sim too, each time.
    assert_eq!(
        calls,
        [
            (first, 10),
            (PERSON, 9),
            (first, 10),
            (first, 9),
            (PERSON + 1, 9)
        ]
    );
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, 0);
    assert_eq!(
        fx.world.objects[&(PERSON + 1)].slots[2].object,
        first as i16
    );
    assert_eq!(fx.world.objects[&first].data[2], (PERSON + 1) as i16);
}

/// Selects for a household of the given member types on a lot, and gives
/// who was selected and what global 3 holds.
fn select_resident(fx: &mut Fixture, members: &[u32]) -> (Option<u16>, i16) {
    let (sim, _) = asking(fx, |i, world| {
        i.select_resident(world, 12_345, members).unwrap()
    });
    (sim, fx.world.globals[3])
}

#[test]
fn the_familys_first_member_is_selected_if_the_lot_allows_it() {
    let household = |lot: i16, zone: Option<i32>, visitor: i16, family: i16| {
        let mut fx = lot_with_person();
        fx.world.globals[10] = lot;
        fx.world.globals[3] = 99;
        fx.world.globals[32] = 0;
        if let Some(zone) = zone {
            fx.program.zoning.insert(lot, zone);
        }
        let person = fx.world.objects.get_mut(&PERSON).unwrap();
        let data = &mut person.person.as_mut().unwrap().data;
        (data[32], data[61]) = (visitor, family);
        fx
    };
    // A residential lot: a Sim who lives there.
    let mut fx = household(5, Some(1), 0, 7);
    assert_eq!(
        select_resident(&mut fx, &[SIM_GUID, LAMP_GUID]),
        (Some(PERSON), PERSON as i16)
    );
    // The funds are set again, split at 10,000 across globals 14 and 13.
    assert_eq!(fx.world.budget.funds, 12_345);
    assert_eq!((fx.world.globals[14], fx.world.globals[13]), (1, 2345));
    // Looking the member up made its type's instance list.
    assert_eq!(fx.world.instances[&SIM_GUID], [PERSON]);
    // A visitor there is not selected. Nobody was selected, so nothing
    // is announced and global 3 stays.
    let mut fx = household(5, Some(1), 1, 7);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]), (None, 99));
    assert!(!fx.world.objects[&PERSON].selected);
    // Lots 21 to 30: anyone whose visitor word is not 2.
    let mut fx = household(25, None, 1, 0);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]).0, Some(PERSON));
    let mut fx = household(25, None, 2, 7);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]), (None, 99));
    // A community lot, or lots 40 to 49: the selected family's Sim, or
    // one whose visitor word is not 2.
    let mut fx = household(60, Some(2), 2, 7);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]).0, Some(PERSON));
    let mut fx = household(45, None, 2, 8);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]), (None, 99));
    let mut fx = household(45, None, 1, 8);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]).0, Some(PERSON));
    // Any other lot: nothing is selected.
    let mut fx = household(60, None, 0, 7);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]), (None, 99));
    // A first member who is not on the lot, or no member: nobody.
    let mut fx = household(5, Some(1), 0, 7);
    assert_eq!(select_resident(&mut fx, &[0x7777, SIM_GUID]), (None, 99));
    assert_eq!(fx.world.instances[&0x7777], []);
    let mut fx = household(5, Some(1), 0, 7);
    assert_eq!(select_resident(&mut fx, &[]), (None, 99));
    // A selected Sim that belongs to a family stays selected, whoever
    // the first member is.
    let mut fx = household(5, Some(1), 1, 7);
    fx.world.objects.get_mut(&PERSON).unwrap().selected = true;
    assert_eq!(select_resident(&mut fx, &[0x7777]), (Some(PERSON), 99));
    // The first of two Sims of the member's type is the one selected.
    let mut fx = household(5, Some(1), 0, 7);
    resident(&mut fx, PERSON - 10, [4, 4]);
    assert_eq!(select_resident(&mut fx, &[SIM_GUID]).0, Some(PERSON - 10));
}

#[test]
fn going_live_marks_the_rooms_and_the_frame_counts_them() {
    let mut fx = lot_with_person();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let world = &mut fx.world;
    let all = [0, 1, 1024];
    // The person stands in room 1, which has no light: whether the room
    // is lit for them is not ported.
    let fault = interpreter.update_rooms(world, 0).unwrap_err();
    assert_eq!(
        fault.trap,
        Trap::Unsupported(RoomError::Occupancy(1).to_string())
    );
    // Outside the walls a person is fine: room 0 has daylight.
    let (mut fx, _) = lot();
    resident(&mut fx, PERSON, [1, 1]);
    // A lamp inside: a light source worth 30, with 5 of impact, and of a
    // category the lot's statistics know.
    let lamp = stand(&mut fx, [2, 2]);
    let object = fx.world.objects.get_mut(&lamp).unwrap();
    (object.data[40], object.data[51], object.data[7]) = (0x400, 30, 5);
    (object.data[34], object.data[39], object.data[59]) = (0, 2, 73);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let world = &mut fx.world;
    (world.globals[4], world.globals[12], world.globals[19]) = (0, 2009, 3);
    assert_eq!(interpreter.update_rooms(world, 0).unwrap(), all);
    world.mark_all_rooms();
    assert_eq!(interpreter.update_pending_rooms(world).unwrap(), all);
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    let room = &world.rooms.rooms[&1];
    // Tile 2, 2 is listed twice, so the lamp counts twice.
    assert_eq!((room.objects, room.lights, room.lamp_light), (2, 2, 60));
    assert_eq!((room.impact, room.good, room.word_39), (10, 2, 4));
    // The person counts once for each time the outdoor room lists its
    // tile.
    let outside = &world.rooms.rooms[&0];
    let listed = outside.tiles.iter().filter(|t| **t == [1, 1, 1]).count();
    assert!(listed > 0);
    assert_eq!(outside.people, listed as i32);
    // Live mode: every room waits, the statistics see the lamp's
    // category, the size goes to global 23 and the mode is 0.
    interpreter.go_live(world).unwrap();
    assert_eq!(world.rooms.pending, 3);
    assert!(world.rooms.rooms.values().all(|room| room.dirty));
    assert_eq!((world.globals[19], world.globals[37]), (0, 4));
    assert_eq!(world.globals[23], 6);
    // Before the frame the waiting rooms are counted and the interface's
    // wait check finds nothing.
    assert_eq!(interpreter.before_frame(world).unwrap(), all);
    assert_eq!((world.rooms.pending, world.globals[12]), (0, 0));
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    // Night falls: the outdoor rooms lose their light and stay marked,
    // and the lamp's room does not change.
    world.globals[4] = 2;
    world.mark_all_rooms();
    assert_eq!(interpreter.before_frame(world).unwrap(), all);
    let marked: Vec<i16> = world
        .rooms
        .rooms
        .iter()
        .filter(|(_, room)| room.dirty)
        .map(|(&id, _)| id)
        .collect();
    assert_eq!(marked, [0, 1024]);
    assert_eq!(world.rooms.pending, 0);
    // A light source that is off the lot adds no light.
    let object = world.objects.get_mut(&lamp).unwrap();
    object.position = [-16, -16, 1];
    world.mark_all_rooms();
    interpreter.update_pending_rooms(world).unwrap();
    let room = &world.rooms.rooms[&1];
    assert_eq!((room.objects, room.lights, room.lamp_light), (2, 0, 0));
}

#[test]
fn the_room_constants_come_from_the_global_files_table_3() {
    let mut fx = Fixture::new();
    let defaults = fx.program.room_tuning();
    assert_eq!(defaults, sims_sim::rooms::Tuning::default());
    let entry = |name: &str, value: f32| Constant {
        name: name.to_owned(),
        value,
        comment: String::new(),
    };
    let table = |entries| Constants {
        version: 1,
        entries,
    };
    let library = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    // Table 2 is not the one.
    library
        .tuning
        .insert(2, table(vec![entry("area clip to score", 10.0)]));
    assert_eq!(fx.program.room_tuning(), defaults);
    let library = fx.program.libraries.get_mut(&GLOBAL).unwrap();
    library.tuning.insert(
        3,
        table(vec![
            entry("area clip to score", 80.0),
            entry("outdoor nighttime bonus", -5.0),
        ]),
    );
    let tuning = fx.program.room_tuning();
    assert_eq!(tuning.area_clip, sims_core::F32::from_f32(80.0));
    assert_eq!(tuning.nighttime_bonus, sims_core::F32::from_f32(-5.0));
    assert_eq!(tuning.light_slope, defaults.light_slope);
}
