//! Burn (opcode 9, 0x44d96f): fire objects placed on the stack object's
//! tile, or the tile in front of it, and spread to what catches.
//! Synthetic; no game data.
//!
//! The lot is Find Location For's: six tiles a side, walls around the
//! room of tiles (2, 2) to (3, 3), with the widget type whose Init lets it
//! stand on level 1 and on a tile with no floor. The fire type does the
//! same, and is not solid (data word 8 flag 4), so it collides with no
//! widget. Widgets are stood on
//! tiles as the visit tests stand them; the lamp and the Sim are linked
//! from no tile, and catch nothing.

use crate::find_location::placeable;
use crate::support::*;
use crate::visits::stand;
use sims_formats::execution::ObjectFunction;
use sims_formats::saved_objects::Multipart;
use sims_vm::Trap;

const BURN: u16 = 9;
const FIRE: u32 = 0x24c9_5f99;
const FIRE_INIT: u16 = 4096;
const FIRE_MAIN: u16 = 4097;
/// The global tree that sleeps (`factory`).
const SLEEP: u16 = 256;
/// Data word 8: flag 0x800 stops the fire; flag 0x20 too, without byte 1
/// bit 0.
const FLAGS: usize = 8;
/// Data word 40: flag 0x20 catches.
const FLAGS_2: usize = 40;
const CATCHES: i16 = 0x20;
/// The walls of level 1.
const WALLS: u16 = 2;

fn lot() -> Fixture {
    lot_and_widget_file().0
}

/// The lot, and the file of the widget's type.
fn lot_and_widget_file() -> (Fixture, usize) {
    let (mut fx, widgets) = placeable();
    let file = fx.add_type(FIRE, 4, FIRE_INIT, FIRE_MAIN);
    fx.add(
        file,
        FIRE_INIT,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
            assign(scope::MY_DATA, 8, scope::LITERAL, 4, RETURN_TRUE),
        ]),
    );
    fx.add(
        file,
        FIRE_MAIN,
        tree(vec![ins(SLEEP, 0, ERROR, [20, 0, 0, 0, 0, 0, 0, 0])]),
    );
    for id in [LAMP, SIM] {
        let object = fx.world.objects.get_mut(&id).unwrap();
        object.data[FLAGS] = 0;
        object.data[FLAGS_2] = 0;
        object.data[62] = 0;
    }
    (fx, widgets)
}

/// A widget on `tile`, catching or not.
fn widget(fx: &mut Fixture, tile: [i32; 2], catches: bool) -> u16 {
    let id = stand(fx, tile);
    if catches {
        fx.world.objects.get_mut(&id).unwrap().data[FLAGS_2] = CATCHES;
    }
    id
}

/// Runs the lamp's tree 4096, one Burn with operand bytes 0 and 1.
fn burn(fx: &mut Fixture, stack_object: u16, where_: u8, any: u8) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            BURN,
            RETURN_TRUE,
            RETURN_FALSE,
            [where_, any, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(LAMP, 4096, stack_object as i16, &[], 1000)
}

/// The fires on the lot, by ID, with their positions.
fn fires(fx: &Fixture) -> Vec<[i32; 3]> {
    fx.world
        .objects
        .values()
        .filter(|o| o.guid == FIRE)
        .map(|o| o.position)
        .collect()
}

#[test]
fn what_catches_gets_a_fire_at_its_tiles_centre() {
    let mut fx = lot();
    let w = widget(&mut fx, [3, 3], true);
    fx.world.objects.get_mut(&w).unwrap().position = [51, 60, 1];
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(true));
    assert_eq!(fires(&fx), [[56, 56, 1]]);
}

#[test]
fn with_nothing_to_catch_the_fire_goes_at_the_exact_place() {
    let mut fx = lot();
    let w = widget(&mut fx, [3, 3], false);
    fx.world.objects.get_mut(&w).unwrap().position = [53, 54, 1];
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(true));
    assert_eq!(fires(&fx), [[53, 54, 1]]);
    // Now a fire is on the tile, and a second Burn makes none.
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(false));
    assert_eq!(fires(&fx).len(), 1);
}

#[test]
fn an_object_with_flag_0x800_on_the_tile_stops_the_fire() {
    for catches in [false, true] {
        let mut fx = lot();
        let w = widget(&mut fx, [3, 3], catches);
        fx.world.objects.get_mut(&w).unwrap().data[FLAGS] = 0x800;
        assert_eq!(burn(&mut fx, w, 0, 1), Ok(false));
        assert!(fires(&fx).is_empty());
    }
}

#[test]
fn flag_0x20_or_word_62_stops_the_fire_without_byte_1() {
    for (word, value) in [(FLAGS, 0x20), (62, 1)] {
        let mut fx = lot();
        let w = widget(&mut fx, [3, 3], true);
        fx.world.objects.get_mut(&w).unwrap().data[word] = value;
        assert_eq!(burn(&mut fx, w, 0, 0), Ok(false), "word {word}");
        assert!(fires(&fx).is_empty());
        assert_eq!(burn(&mut fx, w, 0, 1), Ok(true), "word {word}");
        assert_eq!(fires(&fx), [[56, 56, 1]]);
    }
}

#[test]
fn what_catches_on_the_tile_is_gathered_and_shares_the_fire() {
    // The second widget catches too, so its word 62 is tested.
    let mut fx = lot();
    let w = widget(&mut fx, [3, 3], true);
    let other = widget(&mut fx, [3, 3], true);
    fx.world.objects.get_mut(&other).unwrap().data[62] = 1;
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(false));
    // Without the test, one fire serves the tile.
    assert_eq!(burn(&mut fx, w, 0, 1), Ok(true));
    assert_eq!(fires(&fx), [[56, 56, 1]]);
    // One that does not catch is not gathered.
    let mut fx = lot();
    let w = widget(&mut fx, [3, 3], true);
    let other = widget(&mut fx, [3, 3], false);
    fx.world.objects.get_mut(&other).unwrap().data[62] = 1;
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(true));
}

#[test]
fn an_object_around_whose_footprint_overlaps_the_tile_gets_its_own_fire() {
    // Tile (3, 3) spans 48 to 64 in both coordinates. The footprint is
    // the least first and second coordinates, then the greatest.
    for (footprint, gathered) in [([40, 50, 52, 60], true), ([24, 50, 48, 60], false)] {
        let mut fx = lot();
        let w = widget(&mut fx, [3, 3], true);
        let other = widget(&mut fx, [2, 3], false);
        fx.world.objects.get_mut(&other).unwrap().footprint = footprint;
        assert_eq!(burn(&mut fx, w, 0, 0), Ok(true));
        let mut expected = vec![[56, 56, 1]];
        if gathered {
            expected.push([40, 56, 1]);
        }
        assert_eq!(fires(&fx), expected, "{footprint:?}");
    }
}

#[test]
fn the_tiles_of_a_group_that_catches_are_gathered() {
    // Two widgets made a group by hand: the lead catching on (3, 3), the
    // other tile on (1, 1), not catching, with word 62 set. The lead's
    // group gathers the other tile, whose word 62 then stops the fire
    // without byte 1. With byte 1 each tile gets a fire. A type with a
    // tree for function 8 gathers no group.
    for (dynamic, any, result, fires_at) in [
        (false, 0, false, vec![]),
        (false, 1, true, vec![[56, 56, 1], [24, 24, 1]]),
        (true, 0, true, vec![[56, 56, 1]]),
    ] {
        let (mut fx, file) = lot_and_widget_file();
        let lead = widget(&mut fx, [3, 3], true);
        let tile = widget(&mut fx, [1, 1], false);
        fx.world.objects.get_mut(&tile).unwrap().data[62] = 1;
        for (id, parent) in [(lead, 0), (tile, lead as i16)] {
            fx.world.objects.get_mut(&id).unwrap().multipart = Some(Multipart {
                offset: [0; 3],
                group: [0; 3],
                parent,
            });
        }
        fx.world.next_parts.insert(lead, tile);
        fx.words(file)[10] = 1;
        if dynamic {
            let table = fx
                .program
                .libraries
                .get_mut(&file)
                .unwrap()
                .callbacks
                .get_mut(&128)
                .unwrap();
            table.resize_with(9, || ObjectFunction {
                condition: 0,
                action: 0,
            });
            table[8].action = 4097;
        }
        assert_eq!(burn(&mut fx, lead, 0, any), Ok(result), "{dynamic} {any}");
        assert_eq!(fires(&fx), fires_at, "{dynamic} {any}");
    }
}

#[test]
fn a_person_around_counts_two_units_wider_and_is_not_moved() {
    // The Sim on tile (2, 3) reaches 47 in the first coordinate, short of
    // tile (3, 3), but a person's footprint is widened by 2, so it is
    // gathered and moved, which resets it first: that fails here, as the
    // fixture's Sim has no profile. At 45 it stays short even widened.
    for (reach, gathered) in [(47, true), (45, false)] {
        let mut fx = lot();
        let w = widget(&mut fx, [3, 3], true);
        // The Sim alone on tile (2, 3).
        let cell = ((2 * 64 + 3) * 2) as usize;
        fx.world.arrays.get_mut(&3).unwrap().data[cell..cell + 2]
            .copy_from_slice(&SIM.to_le_bytes());
        let sim = fx.sim_mut();
        sim.position = [40, 56, 1];
        sim.footprint = [40, 50, reach, 60];
        for slot in &mut sim.slots {
            slot.object = 0;
        }
        let result = burn(&mut fx, w, 0, 1);
        if gathered {
            let trap = trap(result);
            assert!(
                matches!(&trap, Trap::Unsupported(m) if m.contains("moving person 2")),
                "{trap:?}"
            );
        } else {
            assert_eq!(result, Ok(true));
            assert_eq!(fires(&fx), [[56, 56, 1]]);
        }
    }
}

#[test]
fn a_gathered_person_resets_and_moves_to_its_tiles_centre() {
    // 0x44df25: the person on the burning tile, off its centre, resets
    // (its thread restarts at Main and its Reset tree runs), is put at the
    // centre of the tile it stood on at its level, and the fire spreads
    // to the tile as to anything that catches.
    use crate::visits::{PERSON, RESIDENT_MAIN, resident};
    let mut fx = lot();
    let w = widget(&mut fx, [3, 3], true);
    let people = resident(&mut fx);
    fx.program.people = people;
    // The person's Reset tree notes 6 in the lamp's attribute 5.
    fx.world.objects.get_mut(&LAMP).unwrap().attributes.resize(8, 0);
    let sim = fx.world.objects.get_mut(&PERSON).unwrap();
    sim.position = [50, 61, 1];
    sim.data[FLAGS] = 0;
    sim.data[FLAGS_2] = CATCHES;
    sim.data[62] = 0;
    fx.thread(PERSON, 4096, &[0; 4]);
    fx.world
        .animators
        .insert(PERSON, sims_sim::animator::LOADED);
    assert_eq!(burn(&mut fx, w, 0, 1), Ok(true));
    let sim = &fx.world.objects[&PERSON];
    assert_eq!(sim.position, [56, 56, 1]);
    assert_eq!((sim.stack.len(), sim.stack[0].tree), (1, RESIDENT_MAIN));
    assert_eq!(fx.world.objects[&LAMP].attributes[5], 6);
    // Its virtual function 0x74 unset its animator's position.
    assert_eq!(
        fx.world.animators[&PERSON].position,
        [sims_sim::animator::UNSET; 3]
    );
    assert_eq!(fires(&fx), [[56, 56, 1]]);
}

#[test]
fn in_front_burns_the_next_tile_in_the_stack_objects_direction() {
    // Direction 4 steps the second coordinate up a tile: from (2, 2) to
    // (2, 3), whose north side has no wall.
    let mut fx = lot();
    let w = widget(&mut fx, [2, 2], false);
    fx.world.objects.get_mut(&w).unwrap().data[1] = 4;
    assert_eq!(burn(&mut fx, w, 1, 0), Ok(true));
    assert_eq!(fires(&fx), [[40, 56, 1]]);
}

#[test]
fn in_front_a_wall_facing_back_must_be_one_to_see_through() {
    // From (1, 2) direction 2 steps to (2, 2), whose west wall faces back.
    // The lot's walls have style 1.
    let mut fx = lot();
    let w = widget(&mut fx, [1, 2], false);
    fx.world.objects.get_mut(&w).unwrap().data[1] = 2;
    assert_eq!(burn(&mut fx, w, 1, 0), Ok(false));
    assert!(fires(&fx).is_empty());
    // A wall of style 3 lets it through.
    let cell = (2 * 64 + 2) * 14;
    fx.world.arrays.get_mut(&WALLS).unwrap().data[cell + 2] = 3;
    assert_eq!(burn(&mut fx, w, 1, 0), Ok(true));
    assert_eq!(fires(&fx), [[40, 40, 1]]);
    // A diagonal wall stops it whatever the style.
    let mut fx = lot();
    let w = widget(&mut fx, [1, 2], false);
    fx.world.objects.get_mut(&w).unwrap().data[1] = 2;
    let walls = &mut fx.world.arrays.get_mut(&WALLS).unwrap().data;
    walls[cell + 2] = 3;
    walls[cell] |= 0x10;
    assert_eq!(burn(&mut fx, w, 1, 0), Ok(false));
}

#[test]
fn a_tile_off_the_lots_inside_takes_the_false_branch() {
    // From (2, 1) direction 0 steps the second coordinate down, to (2, 0),
    // outside the tiles 1 to 4.
    let mut fx = lot();
    let w = widget(&mut fx, [2, 1], false);
    fx.world.objects.get_mut(&w).unwrap().data[1] = 0;
    assert_eq!(burn(&mut fx, w, 1, 0), Ok(false));
    assert!(fires(&fx).is_empty());
}

#[test]
fn burn_needs_a_stack_object_standing_on_the_lot() {
    let mut fx = lot();
    assert_eq!(code(burn(&mut fx, 77, 0, 0)), Some(0x15));
    let off = crate::find_location::widget(&mut fx);
    assert_eq!(burn(&mut fx, off, 0, 0), Ok(false));
    assert!(fires(&fx).is_empty());
}

#[test]
fn a_content_without_the_fire_type_fails_loudly() {
    let (mut fx, _) = placeable();
    let w = widget(&mut fx, [3, 3], true);
    let trap = trap(burn(&mut fx, w, 0, 0));
    assert!(
        matches!(&trap, Trap::Unsupported(m) if m.contains("fire type")),
        "{trap:?}"
    );
}

#[test]
fn the_fire_goes_to_the_level_the_place_test_left_its_object_on() {
    // Sol's review reproduction: the fire's function 5 (may overlap) runs
    // during its place test and moves the catching widget upstairs with
    // Find Location For. The put reads the widget's level again
    // (0x44e065, 0x44e06f), so the fire goes upstairs too.
    let (mut fx, widgets) = lot_and_widget_file();
    let w = widget(&mut fx, [3, 3], true);
    fx.world.objects.get_mut(&w).unwrap().data[4] = 3;
    fx.world.objects.get_mut(&w).unwrap().data[42] = 3;
    fx.words(widgets)[16] = 0;
    fx.lamp_mut().position = [56, 56, 2];
    let at = (3 * 64 + 3) * 2;
    fx.world.arrays.get_mut(&101).unwrap().data[at] = 1;
    let file = fx.catalog.object(FIRE).unwrap().resource;
    fx.add(
        file,
        FIRE_INIT,
        tree(vec![
            assign(scope::MY_DATA, 4, scope::LITERAL, 3, 1),
            assign(scope::MY_DATA, 42, scope::LITERAL, 3, RETURN_TRUE),
        ]),
    );
    const OVERLAP: u16 = 4200;
    crate::visits::function(&mut fx, file, 5, OVERLAP);
    fx.add(
        file,
        OVERLAP,
        tree(vec![
            // Not solid from here on, so moving the widget does not call
            // this again.
            assign(scope::MY_DATA, 8, scope::LITERAL, 4, 1),
            assign(scope::LOCALS, 0, scope::LITERAL, LAMP as i16, 2),
            // Find Location For: the widget goes upstairs, by the lamp.
            ins(16, RETURN_TRUE, ERROR, [0, 0, 3, 0, 0, 0, 0, 0]),
        ]),
    );
    assert_eq!(burn(&mut fx, w, 0, 0), Ok(true));
    assert_eq!(fx.world.objects[&w].position, [56, 56, 2]);
    assert_eq!(fires(&fx), [[56, 56, 2]]);
}
