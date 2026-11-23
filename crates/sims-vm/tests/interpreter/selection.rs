//! The end of a load: the house is announced, a Sim is selected and gets
//! the indicator, the game goes to live mode, and the rooms are counted
//! with the objects that stand in them.

use crate::support::*;
use crate::visits::{lot, stand};
use sims_core::F32;
use sims_formats::saved_objects::Slot;
use sims_formats::tuning::{Constant, Constants};
use sims_vm::{Asked, INDICATORS, Interpreter, Trap, selected};
use std::cell::{Cell, RefCell};

/// The person the tests add, and the tile it stands on, inside the
/// walls: room 1.
const PERSON: u16 = 50;
const TILE: [i32; 2] = [2, 3];

/// The lot of the visit tests with a person standing on it: a resident of
/// family 7 with no date and no fame, and six empty slots.
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
    // No fame (Superstar), which would take a star marker.
    person.data[81] = 0;
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
}

/// The lot with two residents, the indicators made and the first
/// resident selected.
fn two_residents() -> Fixture {
    let mut fx = lot_with_person();
    resident(&mut fx, PERSON + 1, [4, 4]);
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    fx
}

#[test]
fn deselecting_refreshes_the_animator_at_the_tick_count() {
    // Deselecting refreshes the Sim's animator (0x58da12 with 0): its
    // clock is read again from the lot's tick count, whatever the house
    // view drew. Nothing else in the fixture's animator moves.
    for target in [Some(PERSON + 1), None] {
        let mut fx = two_residents();
        fx.world.ticks = 1000;
        assert!(!fx.world.animators.contains_key(&PERSON));
        asking(&mut fx, |i, world| i.select(world, target).unwrap());
        let animator = &fx.world.animators[&PERSON];
        assert_eq!(animator.clock, animator.clock_at(1000));
        assert_ne!(animator.clock, 0);
        assert!(!fx.world.animators.contains_key(&(PERSON + 1)));
    }
}

#[test]
fn selecting_another_sim_deselects_the_first() {
    let mut fx = two_residents();
    let [first, second] = fx.world.indicators;
    let ((), calls) = asking(&mut fx, |i, world| {
        i.select(world, Some(PERSON + 1)).unwrap()
    });
    // With nobody selected, both indicators are taken off the lot
    // (0x4468fd, 0x446be8), each twice as an indicator is: the first
    // leaves the Sim's slot, which tells the Sim. Then the new Sim is
    // selected as from nobody.
    assert_eq!(
        calls,
        [
            (first, 10),
            (PERSON, 9),
            (first, 10),
            (second, 10),
            (second, 10),
            (first, 10),
            (first, 10),
            (first, 9),
            (PERSON + 1, 9)
        ]
    );
    assert!(!fx.world.objects[&PERSON].selected);
    assert_eq!(selected(&fx.world), Some(PERSON + 1));
    assert_eq!(fx.world.globals[3], (PERSON + 1) as i16);
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, 0);
    assert_eq!(
        fx.world.objects[&(PERSON + 1)].slots[2].object,
        first as i16
    );
    assert_eq!(fx.world.objects[&first].data[2], (PERSON + 1) as i16);
    assert_eq!(fx.world.objects[&second].position, [-16, -16, 1]);
    // Selecting nobody deselects and announces 0.
    let ((), _) = asking(&mut fx, |i, world| i.select(world, None).unwrap());
    assert_eq!(selected(&fx.world), None);
    assert_eq!(fx.world.globals[3], 0);
    assert_eq!(fx.world.objects[&(PERSON + 1)].slots[2].object, 0);
    assert_eq!(fx.world.objects[&first].data[2], 0);
    assert_eq!(fx.world.objects[&first].position, [-16, -16, 1]);
}

#[test]
fn a_famous_sim_deselected_takes_and_releases_a_star_marker() {
    // Without Superstar (expansions & 0x40 == 0), deselecting a Sim with word
    // 81 >= 10 leaves no star marker in the head slot (0x446bf4).
    let mut fx = two_residents();
    register_star(&mut fx);
    fx.world
        .objects
        .get_mut(&PERSON)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[81] = 10;
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON + 1)));
    assert!(result.is_ok());
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, 0);

    // With Superstar (expansions = 0x40), deselecting PERSON places a pooled
    // star marker in its head slot (0x446c15), and re-selecting PERSON releases
    // that star marker back off the lot (0x4467d7 -> 0x4481eb) and reuses the
    // pool on the next deselect without allocating more stars.
    let mut fx = two_residents();
    fx.program.expansions = 0x40;
    register_star(&mut fx);
    fx.world
        .objects
        .get_mut(&PERSON)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[81] = 10;
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON + 1)));
    assert!(result.is_ok());
    let star_id = fx.world.objects[&PERSON].slots[2].object as u16;
    assert_ne!(star_id, 0);
    assert_eq!(fx.world.objects[&star_id].guid, STAR_MARKER);
    assert_eq!(
        fx.world
            .objects
            .values()
            .filter(|o| o.guid == STAR_MARKER)
            .count(),
        4
    );

    // Re-selecting PERSON releases the star marker off-lot and puts indicator 0
    // in PERSON's head slot.
    let first = fx.world.indicators[0];
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)));
    assert!(result.is_ok());
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, first as i16);
    assert_eq!(fx.world.objects[&star_id].data[2], 0);
    assert_eq!(fx.world.objects[&star_id].position, [-16, -16, 1]);

    // Deselecting PERSON again reuses the existing pool (still 4 total stars).
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON + 1)));
    assert!(result.is_ok());
    let star_id_2 = fx.world.objects[&PERSON].slots[2].object as u16;
    assert_ne!(star_id_2, 0);
    assert_eq!(fx.world.objects[&star_id_2].guid, STAR_MARKER);
    assert_eq!(
        fx.world
            .objects
            .values()
            .filter(|o| o.guid == STAR_MARKER)
            .count(),
        4
    );

    // Selecting a Sim on a lot smaller than 64 tiles whose position is on the
    // outer border tile (0x446727 -> 0x422632) leaves its head slot untouched
    // and does not attach indicator 0.
    let mut fx = two_residents();
    fx.program.expansions = 0x40;
    register_star(&mut fx);
    fx.world
        .objects
        .get_mut(&PERSON)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[81] = 10;
    asking(&mut fx, |i, world| {
        i.select(world, Some(PERSON + 1)).unwrap()
    });
    let border_star = fx.world.objects[&PERSON].slots[2].object as u16;
    assert_ne!(border_star, 0);
    fx.world.size = 32;
    fx.world.objects.get_mut(&PERSON).unwrap().position = [31 * 16 + 8, 3 * 16 + 8, 1];
    let first = fx.world.indicators[0];
    asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    assert_eq!(
        fx.world.objects[&PERSON].slots[2].object, border_star as i16,
        "border-tile Sim on smaller lot must keep its star marker untouched"
    );
    assert_eq!(fx.world.objects[&first].data[2], 0);

    // A Sim that is not a person still cannot be selected, after the
    // deselection.
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(LAMP)));
    assert!(matches!(result.unwrap_err().trap, Trap::Invalid(_)));
}

const STAR_MARKER: u32 = 0xE933_7020;
const OTHER: u16 = PERSON + 1;

/// Gives the lot's catalog the Superstar star marker, whose Init does
/// nothing.
fn register_star(fx: &mut Fixture) {
    let file = fx.add_type(STAR_MARKER, 4, 0, 4096);
    fx.add(
        file,
        4096,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            0,
            RETURN_TRUE,
        )]),
    );
}

/// Sets a person's fame (person word 81).
fn famous(fx: &mut Fixture, id: u16, fame: i16) {
    let sim = fx.world.objects.get_mut(&id).unwrap();
    sim.person.as_mut().unwrap().data[81] = fame;
}

fn stars(fx: &Fixture) -> usize {
    fx.world
        .objects
        .values()
        .filter(|o| o.guid == STAR_MARKER)
        .count()
}

/// The one star marker in neither pool: lost.
fn in_no_pool(fx: &Fixture) -> u16 {
    let pools = &fx.world.indicator_queues;
    let lost: Vec<u16> = fx
        .world
        .objects
        .values()
        .filter(|o| o.guid == STAR_MARKER)
        .map(|o| o.id)
        .filter(|id| !pools.available_stars.contains(id) && !pools.stars_in_use.contains(id))
        .collect();
    let [lost] = lost[..] else {
        panic!("one star is lost: {lost:?}");
    };
    lost
}

/// The original's pool order for a famous Sim on a border tile. A star
/// refresh (0x446ee9) pops the pool's last star (0x447012) before the tile
/// check (0x44702e), and puts it in the in-use pool only on success
/// (0x4471b0): the star is lost, and the next famous Sim gets another. A
/// deselection (0x446bf0) checks the tile first (0x446d17) and pops only
/// after placing (0x446ed9), so it loses none.
#[test]
fn a_star_refresh_on_a_border_tile_loses_the_popped_star() {
    // The refresh: OTHER, not selected, is famous on a lot's border tile.
    let mut fx = two_residents();
    fx.program.expansions = 0x40;
    register_star(&mut fx);
    fx.world.size = 32;
    famous(&mut fx, OTHER, 10);
    let inside = fx.world.objects[&OTHER].position;
    fx.world.objects.get_mut(&OTHER).unwrap().position = [31 * 16 + 8, 3 * 16 + 8, 1];
    fx.world.indicator_queues.queue_star(OTHER);
    // With a Sim selected, announcing the house only refreshes (0x4a8679).
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!(
        fx.world.objects[&OTHER].slots[2].object, 0,
        "no star on the border"
    );
    assert_eq!(stars(&fx), 4, "the pool was made (0x446356)");
    let pools = &fx.world.indicator_queues;
    assert!(pools.stars_in_use.is_empty());
    assert_eq!(pools.available_stars.len(), 3);
    let lost = in_no_pool(&fx);
    assert_eq!(fx.world.objects[&lost].data[2], 0, "held by nobody");

    // Back inside, the next refresh gets another star from the pool, and
    // the pool is not refilled.
    fx.world.objects.get_mut(&OTHER).unwrap().position = inside;
    fx.world.indicator_queues.queue_star(OTHER);
    // With a Sim selected, announcing the house only refreshes (0x4a8679).
    let next = *fx.world.indicator_queues.available_stars.last().unwrap();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let star = fx.world.objects[&OTHER].slots[2].object as u16;
    assert_eq!(star, next, "the available pool's last star");
    assert_ne!(star, lost, "the lost star is in neither pool");
    assert_eq!(stars(&fx), 4);
    let pools = &fx.world.indicator_queues;
    assert_eq!(pools.stars_in_use, [star]);
    assert_eq!(pools.available_stars.len(), 2);
    assert_eq!(in_no_pool(&fx), lost);

    // Deleting a star forgets it (0x448097).
    fx.world.indicator_queues.forget(star);
    assert!(fx.world.indicator_queues.stars_in_use.is_empty());

    // The deselection: PERSON is famous on the border tile when OTHER is
    // selected. No star goes on it, and none is lost.
    let mut fx = two_residents();
    fx.program.expansions = 0x40;
    register_star(&mut fx);
    fx.world.size = 32;
    famous(&mut fx, PERSON, 10);
    fx.world.objects.get_mut(&PERSON).unwrap().position = [31 * 16 + 8, 3 * 16 + 8, 1];
    asking(&mut fx, |i, world| i.select(world, Some(OTHER)).unwrap());
    assert_eq!(stars(&fx), 4);
    assert!(
        fx.world.objects[&PERSON].slots.iter().all(|slot| fx
            .world
            .objects
            .get(&(slot.object as u16))
            .is_none_or(|o| o.guid != STAR_MARKER)),
        "no star on the border"
    );
    let pools = &fx.world.indicator_queues;
    assert_eq!(pools.available_stars.len(), 4);
    assert!(pools.stars_in_use.is_empty());
}

/// The available star pool is a stack in the order stars come back: the
/// original appends a star let go from a head (0x4481eb: 0x4483bd) and
/// takes the last (0x447012). Two famous Sims take the two newest stars;
/// letting the newer go and then the older puts the older last, so a
/// famous Sim refreshed on a border tile loses the older.
#[test]
fn the_star_pool_hands_out_the_last_star_let_go() {
    const THIRD: u16 = PERSON + 2;
    let mut fx = two_residents();
    resident(&mut fx, THIRD, [2, 2]);
    fx.program.expansions = 0x40;
    register_star(&mut fx);
    for sim in [OTHER, THIRD] {
        famous(&mut fx, sim, 10);
        fx.world.indicator_queues.queue_star(sim);
    }
    // PERSON is selected: the refresh puts stars on OTHER, then THIRD.
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let held = |fx: &Fixture, sim: u16| fx.world.objects[&sim].slots[2].object as u16;
    let (newer, older) = (held(&fx, OTHER), held(&fx, THIRD));
    assert_eq!(fx.world.indicator_queues.stars_in_use, [newer, older]);
    assert!(
        newer > older,
        "made in order, the last made handed out first"
    );
    // Selecting OTHER lets its star go (0x4467d7 -> 0x4481eb); it is then
    // no longer famous, so selecting THIRD next gives it no star again.
    asking(&mut fx, |i, world| i.select(world, Some(OTHER)).unwrap());
    famous(&mut fx, OTHER, 0);
    asking(&mut fx, |i, world| i.select(world, Some(THIRD)).unwrap());
    let pools = &fx.world.indicator_queues;
    assert!(pools.stars_in_use.is_empty());
    assert_eq!(pools.available_stars[2..], [newer, older]);
    // PERSON, famous and not selected, is refreshed on a border tile: the
    // star popped and lost is the older one, let go last.
    fx.world.size = 32;
    famous(&mut fx, PERSON, 10);
    fx.world.objects.get_mut(&PERSON).unwrap().position = [31 * 16 + 8, 3 * 16 + 8, 1];
    fx.world.indicator_queues.queue_star(PERSON);
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!(in_no_pool(&fx), older);
    assert_eq!(
        fx.world.indicator_queues.available_stars.last(),
        Some(&newer)
    );
}

/// A put runs the arrival (0x44752e) when the object was off the active
/// lot: 0x433366 tests the old position with 0x422632 on the lot, so
/// against the lot's size, not the 64 tiles of its arrays. A famous Sim
/// moved from tile 31 to tile 30 of a 32-tile lot arrives and takes a
/// star; on a 64-tile lot tile 31 is inside and nothing arrives. Only
/// the x word is compared for a move (0x4335a2..0x4335a5), so a move
/// from (3, 31) to (3, 30) on the 32-tile lot runs no arrival.
#[test]
fn a_famous_sim_moved_in_from_a_small_lots_border_takes_a_star() {
    const FIND_LOCATION: u16 = 16;
    let star_on = |size: i16, from: [i32; 2], to: [i32; 2]| {
        let mut fx = two_residents();
        fx.program.expansions = 0x40;
        register_star(&mut fx);
        fx.world.size = size;
        // No pool and no water anywhere, for the place test.
        fx.world.arrays.insert(9, crate::rooms::array(1, 0xff));
        fx.world.arrays.insert(10, crate::rooms::array(1, 0xff));
        // OTHER, famous and not selected, stands on tile `from`.
        let grid = &mut fx.world.arrays.get_mut(&3).unwrap().data;
        let cell = |t: [i32; 2]| ((t[0] * 64 + t[1]) * 2) as usize;
        grid[cell([4, 4])..cell([4, 4]) + 2].copy_from_slice(&[0, 0]);
        grid[cell(from)..cell(from) + 2].copy_from_slice(&OTHER.to_le_bytes());
        let other = fx.world.objects.get_mut(&OTHER).unwrap();
        other.position = [from[0] * 16 + 8, from[1] * 16 + 8, 1];
        // It may stand on level 1 (data word 4 bit 0) on a tile with no
        // floor (data word 42 flag 2), as the lot's tiles have none.
        other.data[4] |= 1;
        other.data[42] |= 2;
        famous(&mut fx, OTHER, 10);
        // Find Location mode 2 puts OTHER where the lamp is, on tile `to`,
        // with the lot told (0x433340).
        fx.lamp_mut().position = [to[0] * 16 + 8, to[1] * 16 + 8, 1];
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                FIND_LOCATION,
                RETURN_TRUE,
                RETURN_FALSE,
                [2, 0, 0, 0, 0, 0, 0, 0],
            )]),
        );
        let ran = fx.run_with(LAMP, 4096, OTHER as i16, &[], 1000);
        assert_eq!(ran, Ok(true), "lot size {size}");
        assert_eq!(
            fx.world.objects[&OTHER].position,
            [to[0] * 16 + 8, to[1] * 16 + 8, 1]
        );
        let star = fx.world.objects[&OTHER].slots[2].object as u16;
        fx.world
            .objects
            .get(&star)
            .is_some_and(|o| o.guid == STAR_MARKER)
    };
    assert!(
        star_on(32, [31, 3], [30, 3]),
        "tile 31 is off a 32-tile lot: OTHER arrives"
    );
    assert!(
        !star_on(64, [31, 3], [30, 3]),
        "tile 31 is on a 64-tile lot: no arrival"
    );
    assert!(!star_on(32, [3, 31], [3, 30]), "x did not move: no arrival");
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

/// The date of the date tests: a second resident.
const DATE: u16 = 51;

fn date_word(fx: &mut Fixture, id: u16, value: i16) {
    let sim = fx.world.objects.get_mut(&id).unwrap();
    sim.person.as_mut().unwrap().data[77] = value;
}

#[test]
fn selecting_a_sim_with_a_date_puts_the_second_indicator_on_the_date() {
    // 0x446924..0x446bae: a word 77 below 0 names the date by its negated
    // ID. The second indicator leaves the lot and goes into the date's
    // head slot (record 2, slot 1), letting go what was there.
    let mut fx = lot_with_person();
    resident(&mut fx, DATE, [3, 2]);
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let [first, second] = fx.world.indicators;
    date_word(&mut fx, PERSON, -(DATE as i16));
    let ((), _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, first as i16);
    assert_eq!(fx.world.objects[&DATE].slots[2].object, second as i16);
    let marker = &fx.world.objects[&second];
    assert_eq!((marker.data[2], marker.data[3]), (DATE as i16, 1));
    assert_eq!(marker.position, fx.world.objects[&DATE].position);

    // A word 77 above 0 does nothing here, and neither does a date that
    // is not on the lot (only the indicator leaves).
    for date in [DATE as i16, -999] {
        let mut fx = lot_with_person();
        resident(&mut fx, DATE, [3, 2]);
        asking(&mut fx, |i, world| i.announce_house(world).unwrap());
        let second = fx.world.indicators[1];
        date_word(&mut fx, PERSON, date);
        let ((), _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
        assert_eq!(fx.world.objects[&DATE].slots[2].object, 0);
        assert_eq!(fx.world.objects[&second].data[2], 0);
    }
}

#[test]
fn writing_a_date_or_fame_queues_an_indicator_refresh() {
    // 0x4538e3: resolving person word 77 or 81 with write intent queues
    // the person, before the write. A date is queued once and only while
    // word 77 is not 0 (0x4448a6); fame every time (0x444962).
    let mut fx = lot_with_person();
    let write = |fx: &mut Fixture, word: i16, value: i16| {
        fx.add(
            SIM_FILE,
            4096,
            tree(vec![assign(
                scope::MY_PERSON,
                word,
                scope::LITERAL,
                value,
                RETURN_TRUE,
            )]),
        );
        assert_eq!(fx.run_with(PERSON, 4096, 0, &[], 100), Ok(true));
    };
    write(&mut fx, 77, -(DATE as i16));
    assert_eq!(fx.world.indicator_queues, Default::default());
    write(&mut fx, 77, 0);
    write(&mut fx, 77, 5);
    assert_eq!(fx.world.indicator_queues.dates, [PERSON]);
    write(&mut fx, 77, 6);
    assert_eq!(fx.world.indicator_queues.dates, [PERSON]);
    write(&mut fx, 81, 3);
    write(&mut fx, 81, 4);
    assert_eq!(fx.world.indicator_queues.stars, [PERSON, PERSON]);
    assert!(fx.world.indicator_queues.dirty);
}

#[test]
fn the_indicator_refresh_moves_the_date_marker_and_empties_the_queues() {
    // 0x4464db -> 0x4472a2: the second indicator leaves the lot, and goes
    // onto the date of a selected person whose word 77 is below 0. The
    // queues are emptied and the flag cleared (0x448847, 0x446643).
    let mut fx = lot_with_person();
    resident(&mut fx, DATE, [3, 2]);
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let ((), _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)).unwrap());
    let second = fx.world.indicators[1];
    date_word(&mut fx, PERSON, -(DATE as i16));
    fx.world.indicator_queues = sims_sim::IndicatorQueues {
        dates: vec![PERSON],
        stars: vec![DATE],
        dirty: true,
        ..Default::default()
    };
    // Announcing the house with a Sim selected only refreshes (0x4a8679).
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!(fx.world.objects[&DATE].slots[2].object, second as i16);
    assert_eq!(fx.world.indicator_queues, Default::default());

    // A person that is not selected loses the marker and gives no new one.
    fx.world.objects.get_mut(&PERSON).unwrap().selected = false;
    fx.world.indicator_queues = sims_sim::IndicatorQueues {
        dates: vec![PERSON],
        stars: Vec::new(),
        dirty: true,
        ..Default::default()
    };
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!(fx.world.objects[&DATE].slots[2].object, 0);
    assert_eq!(fx.world.objects[&second].data[2], 0);

    // Nothing is refreshed while the flag is clear.
    fx.world.objects.get_mut(&PERSON).unwrap().selected = true;
    fx.world.indicator_queues.dates = vec![PERSON];
    let before = fx.world.hash();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    assert_eq!(fx.world.hash(), before);
}

#[test]
fn selecting_a_sim_whose_head_slot_holds_a_non_indicator_puts_the_indicator_there() {
    // 0x4467d7 -> 0x4481eb: a non-indicator object in slot 1 is left untouched
    // by release_from_head and the first indicator takes slot 1.
    let mut fx = lot_with_person();
    asking(&mut fx, |i, world| i.announce_house(world).unwrap());
    let first = fx.world.indicators[0];
    let widget = stand(&mut fx, [4, 4]);
    fx.world.objects.get_mut(&PERSON).unwrap().slots[2].object = widget as i16;
    let (result, _) = asking(&mut fx, |i, world| i.select(world, Some(PERSON)));
    assert!(result.is_ok());
    assert_eq!(fx.world.objects[&PERSON].slots[2].object, first as i16);
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
    assert_eq!(fx.world.instances[&0x7777], [0u16; 0]);
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
    // The person stands in room 1, which has no light. The room has not
    // been told anyone is in it, so it shows its own light.
    assert_eq!(interpreter.update_rooms(world, 0).unwrap(), all);
    let dark = world.rooms.rooms[&1].clone();
    assert!(dark.people > 0 && !dark.occupied && dark.lights == 0);
    assert_eq!(dark.shown_light(), dark.light);
    // (A first count can change a room's light and so mark it again.)
    for room in world.rooms.rooms.values_mut() {
        room.dirty = false;
    }
    world.rooms.pending = 0;
    // Told, it shows 0.8 and is counted again once; the count does not
    // change what it shows, so it is not marked again.
    world.note_occupant(1, true);
    assert_eq!(world.rooms.pending, 1);
    assert_eq!(interpreter.update_pending_rooms(world).unwrap(), [1]);
    assert_eq!(world.rooms.rooms[&1].shown_light(), F32::from_f32(0.8));
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    // A frame of the house view counts the rooms that wait before it
    // draws anyone (0x4a8895 at 0x4a8b03). Drawing this person fails, as
    // the fixture supplies no appearance for it; its room has been
    // counted by then.
    world.note_occupant(1, false);
    assert_eq!(world.rooms.pending, 1);
    let fault = interpreter.draw_frame(world, 1).unwrap_err();
    assert!(fault.to_string().contains("profile"), "{fault}");
    assert_eq!(world.rooms.pending, 0);
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    // Outside the walls: room 0 has daylight.
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

/// A room update builds the cutaway masks of its level's rooms at its
/// end, whether or not it counted any (0x466c3c, at 0x466ef6), and the
/// interpreter notes the levels for the view.
#[test]
fn a_room_update_notes_the_levels_whose_masks_it_built() {
    let mut fx = lot_with_person();
    let rebuilt = Cell::new(0);
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.masks_rebuilt = Some(&rebuilt);
    let world = &mut fx.world;
    assert!(!interpreter.update_rooms(world, 1).unwrap().is_empty());
    assert_eq!(rebuilt.take(), 1);
    // With no room marked none is counted, and the masks are built all
    // the same.
    for room in world.rooms.rooms.values_mut() {
        room.dirty = false;
    }
    assert_eq!(interpreter.update_rooms(world, 2).unwrap(), []);
    assert_eq!(rebuilt.take(), 2);
    // With nothing waiting there is no update.
    world.rooms.pending = 0;
    assert_eq!(interpreter.update_pending_rooms(world).unwrap(), []);
    assert_eq!(rebuilt.get(), 0);
    world.note_occupant(1, true);
    assert_eq!(world.rooms.pending, 1);
    interpreter.update_pending_rooms(world).unwrap();
    assert_eq!(rebuilt.take(), 1);
    world.mark_all_rooms();
    interpreter.update_pending_rooms(world).unwrap();
    assert_eq!(rebuilt.take(), 3);
    // A frame counts the rooms that wait first (0x4a8895), so it notes
    // them too.
    world.mark_all_rooms();
    let _ = interpreter.draw_frame(world, 1);
    assert_eq!(rebuilt.take(), 3);
    // Updates of each level in one step add up.
    interpreter.update_rooms(world, 1).unwrap();
    interpreter.update_rooms(world, 2).unwrap();
    assert_eq!(rebuilt.take(), 3);
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

#[test]
fn before_the_frame_the_selected_familys_friend_count_is_worked_out() {
    use sims_sim::neighbors::{FAMILY, Family, Neighbor, Row};
    let (mut fx, _) = lot();
    let lot_number = fx.world.globals[10];
    fx.program.zoning.insert(lot_number, 1);
    // Records 1 and 2 like each other 25, the default threshold: record
    // 2, of family 7, is a friend of family 5's one member.
    let record = |id: i16, guid: u32, family: i16, target: i32| {
        let mut data = vec![0; 256];
        data[FAMILY] = family;
        Neighbor {
            id,
            guid,
            revision: 5,
            data,
            relationships: vec![Row {
                target,
                values: vec![25],
            }],
            friends: 0,
            stale: true,
        }
    };
    fx.world.neighbors.slots = vec![Some(record(1, 0xa, 5, 2)), Some(record(2, 0xb, 7, 1))];
    let family = |id: i32, members: &[u32]| Family {
        id,
        house: 0,
        members: members.to_vec(),
        friends: -1,
        stale: true,
    };
    fx.world.families = vec![family(7, &[0xb]), family(5, &[0xa]), family(5, &[])];
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    // An empty lot counts nothing.
    fx.world.globals[9] = 0;
    interpreter.before_frame(&mut fx.world).unwrap();
    assert!(fx.world.families.iter().all(|f| f.stale && f.friends == -1));
    // The first family whose ID is in global 9, even with no house.
    fx.world.globals[9] = 5;
    interpreter.before_frame(&mut fx.world).unwrap();
    let counts: Vec<(i32, bool)> = fx
        .world
        .families
        .iter()
        .map(|f| (f.friends, f.stale))
        .collect();
    assert_eq!(counts, [(-1, true), (1, false), (-1, true)]);
}
