//! Remove Object Instance: queueing, yielding and the cases that need the
//! lot.

use crate::factory::{WIDGET, widgets};
use crate::support::*;
use sims_vm::{Interpreter, Status, Trap};

const BUDGET: usize = 100;

/// Remove Object Instance, then set attribute 3 to 1 on either branch but
/// return what the primitive said.
fn remover(mode: i16, flags: u8) -> sims_formats::behavior::Behavior {
    let [low, high] = mode.to_le_bytes();
    tree(vec![
        ins(18, 1, 2, [low, high, flags, 0, 0, 0, 0, 0]),
        assign(scope::MY_ATTRIBUTES, 3, scope::LITERAL, 1, RETURN_TRUE),
        assign(scope::MY_ATTRIBUTES, 3, scope::LITERAL, 1, RETURN_FALSE),
    ])
}

/// A fixture with two widgets (3 and 4) the factory made, off the lot.
fn two() -> (Fixture, usize) {
    let (mut fx, file) = widgets();
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(3));
    assert_eq!(interpreter.create(&mut fx.world, WIDGET), Ok(4));
    (fx, file)
}

#[test]
fn an_object_that_removes_itself_yields_and_stays_registered() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 0));
    fx.thread(3, 4200, &[0; 4]);
    let widget = fx.world.objects.get_mut(&3).unwrap();
    (widget.data[8], widget.data[29]) = (0x0101, 6);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(interpreter.turn(&mut world, 3, BUDGET), Ok(Status::Yielded));
    assert_eq!(world.removals, [3]);
    let widget = &world.objects[&3];
    // Detached: no container, no slot, no room, and flag 0x100 cleared.
    assert_eq!(
        (widget.data[2], widget.data[3], widget.data[14]),
        (0, -1, -1)
    );
    assert_eq!(
        (widget.data[26], widget.data[29], widget.data[8]),
        (0, -5, 1)
    );
    assert!(!widget.removal_skips_resets);
    // Its tree went no further, and the object is still on the lot.
    assert_eq!(widget.attributes[3], 0);
    assert_eq!(widget.stack[0].node, 0);
    assert!(world.order.contains(&3));
    // Running it again queues nothing new.
    assert_eq!(interpreter.turn(&mut world, 3, BUDGET), Ok(Status::Yielded));
    assert_eq!(world.removals, [3]);
}

#[test]
fn flag_bit_0_lets_the_tree_go_on() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 1));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.turn(&mut world, 3, BUDGET),
        Ok(Status::Returned(true))
    );
    assert_eq!(world.removals, [3]);
    assert_eq!(world.objects[&3].attributes[3], 1);
}

#[test]
fn flag_bit_1_is_recorded_on_the_object() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 3));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    interpreter.turn(&mut world, 3, BUDGET).unwrap();
    assert!(world.objects[&3].removal_skips_resets);
    assert!(!world.objects[&4].removal_skips_resets);
}

#[test]
fn a_synchronous_tree_cannot_yield_and_takes_the_true_branch() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(0, 0));
    assert_eq!(fx.run_with(3, 4200, 0, &[], BUDGET), Ok(true));
    assert_eq!(fx.world.removals, [3]);
}

#[test]
fn another_object_is_removed_without_yielding() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(1, 0));
    fx.thread(3, 4200, &[0; 4]);
    fx.world.objects.get_mut(&3).unwrap().stack[0].stack_object = 4;
    // A footprint other than the one the off-lot place gives, as the
    // factory may leave one.
    fx.world.objects.get_mut(&4).unwrap().footprint = [0; 4];
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.turn(&mut world, 3, BUDGET),
        Ok(Status::Returned(true))
    );
    assert_eq!(world.removals, [4]);
    // Leaving the lot works it out again: 16 wide round (-16, -16).
    assert_eq!(world.objects[&4].footprint, [-24, -24, -8, -8]);
    // Requests keep their order.
    fx.world = world;
    fx.add(file, 4200, remover(0, 1));
    fx.thread(3, 4200, &[0; 4]);
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    interpreter.turn(&mut world, 3, BUDGET).unwrap();
    assert_eq!(world.removals, [4, 3]);
}

#[test]
fn a_missing_object_is_the_false_branch() {
    // Mode 2 names object 0, which never exists; stack object 40 is gone.
    for (mode, stack_object) in [(2, 4), (1, 40), (1, 0), (1, -3)] {
        let (mut fx, file) = two();
        fx.add(file, 4200, remover(mode, 0));
        assert_eq!(fx.run_with(3, 4200, stack_object, &[], BUDGET), Ok(false));
        assert!(fx.world.removals.is_empty());
    }
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(3, 0));
    assert_eq!(code(fx.run_with(3, 4200, 0, &[], BUDGET)), Some(9));
}

#[test]
fn objects_off_the_lot_that_need_more_fail_loudly() {
    let unsupported = |prepare: fn(&mut Fixture, usize)| {
        let (mut fx, file) = two();
        fx.add(file, 4200, remover(1, 0));
        prepare(&mut fx, file);
        let fault = fx.run_with(3, 4200, 4, &[], BUDGET).unwrap_err();
        assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
        assert!(fx.world.removals.is_empty());
    };
    // In a container.
    unsupported(|fx, _| fx.world.objects.get_mut(&4).unwrap().data[2] = 3);
    // Holding something.
    unsupported(|fx, _| {
        let widget = fx.world.objects.get_mut(&4).unwrap();
        widget.slots.push(sims_formats::saved_objects::Slot {
            unknown: 0,
            object: 3,
        });
    });
    // With a callback for leaving the lot.
    unsupported(|fx, file| {
        let callbacks = fx.program.libraries.get_mut(&file).unwrap();
        let table = callbacks.callbacks.get_mut(&128).unwrap();
        table.resize(
            11,
            sims_formats::execution::ObjectFunction {
                condition: 0,
                action: 0,
            },
        );
        table[10].action = 4300;
    });
    // A person: the fixture's Sim, removed by the lamp.
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, remover(1, 0));
    assert!(matches!(trap(fx.run()), Trap::Unsupported(_)));
}

/// Where an object is on the object grid (array 3, level 1).
fn on_grid(fx: &Fixture, tile: (usize, usize)) -> i16 {
    let grid = &fx.world.arrays[&3].data;
    let at = (tile.0 * 64 + tile.1) * 2;
    i16::from_le_bytes([grid[at], grid[at + 1]])
}

#[test]
fn an_object_on_the_lot_leaves_it_through_the_placement() {
    let (mut fx, _) = crate::find_location::placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let id = crate::find_location::widget(&mut fx);
    assert_eq!(
        crate::find_location::find(&mut fx, LAMP as i16, id, 1),
        Ok(true)
    );
    let placed = fx.world.objects[&id].position;
    let tile = ((placed[0] >> 4) as usize, (placed[1] >> 4) as usize);
    assert_eq!(on_grid(&fx, tile), id as i16);
    fx.world.objects.get_mut(&id).unwrap().data[8] |= 0x100;
    // Remove the stack object, with flag bit 1.
    fx.add(
        LAMP_FILE,
        4200,
        tree(vec![ins(
            18,
            RETURN_TRUE,
            RETURN_FALSE,
            [1, 0, 2, 0, 0, 0, 0, 0],
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4200, id as i16, &[], BUDGET), Ok(true));
    assert_eq!(fx.world.removals, [id]);
    let widget = &fx.world.objects[&id];
    // Off the lot, in no container and no room, flag 0x100 cleared, and
    // gone from its tile.
    assert_eq!(widget.position, [-16, -16, 1]);
    assert_eq!((widget.data[2], widget.data[3]), (0, -1));
    assert_eq!(widget.data[29], -5);
    assert_eq!(widget.data[8] & 0x100, 0);
    assert!(widget.removal_skips_resets);
    assert_eq!(on_grid(&fx, tile), 0);
    // Still registered until the removals are carried out, and asking
    // again queues nothing new.
    assert!(fx.world.order.contains(&id));
    assert_eq!(fx.run_with(LAMP, 4200, id as i16, &[], BUDGET), Ok(true));
    assert_eq!(fx.world.removals, [id]);
}

#[test]
fn an_object_on_the_lot_removed_at_once_leaves_through_the_placement() {
    // 0x44123f: once the lot's rooms are worked out, an object of one
    // tile that is not a person leaves by its virtual function 0x50 with
    // 1, the lot's placement (0x433125), and is then deleted. The narrow
    // way the load was compared through would refuse an object on the
    // lot.
    let (mut fx, _) = crate::find_location::placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let id = crate::find_location::widget(&mut fx);
    assert_eq!(
        crate::find_location::find(&mut fx, LAMP as i16, id, 1),
        Ok(true)
    );
    let placed = fx.world.objects[&id].position;
    let tile = ((placed[0] >> 4) as usize, (placed[1] >> 4) as usize);
    assert_eq!(on_grid(&fx, tile), id as i16);
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    assert_eq!(interpreter.remove_now(&mut fx.world, id), Ok(()));
    assert!(!fx.world.objects.contains_key(&id));
    assert!(!fx.world.order.contains(&id));
    assert_eq!(on_grid(&fx, tile), 0);
}

#[test]
fn a_fault_in_the_removed_objects_callback_keeps_where_it_happened() {
    let (mut fx, file) = crate::find_location::placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let id = crate::find_location::widget(&mut fx);
    assert_eq!(
        crate::find_location::find(&mut fx, LAMP as i16, id, 1),
        Ok(true)
    );
    // The widget's tree for leaving the lot (function 10, tree 4300)
    // calls a tree that is not there.
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        11,
        sims_formats::execution::ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[10].action = 4300;
    fx.add(
        file,
        4300,
        tree(vec![ins(4999, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    fx.add(LAMP_FILE, 4200, remover(1, 0));
    let fault = fx.run_with(LAMP, 4200, id as i16, &[], BUDGET).unwrap_err();
    // The lamp's instruction reports it, and says it happened in the
    // widget's tree 4300.
    assert_eq!(fault.object, LAMP);
    let Trap::Invalid(message) = &fault.trap else {
        panic!("{fault}");
    };
    assert!(
        message.starts_with(&format!("object {id}, tree 0x10cc node 0: ")),
        "{message}"
    );
    assert!(fx.world.removals.is_empty());
}

/// Deleting a holder removes what it holds (0x440eeb, 0x441196 ->
/// 0x44123f). For a star marker that removal's prelude takes the star out
/// of both pools (0x44126f -> 0x447d8b, 0x447e69 -> 0x448097) before the
/// star is taken off the lot (0x4412a4). So when the holder's PLACED
/// (function 9, 0x433125) faults after the star is detached from it, the
/// star is in no pool, not left in use where Play Mode's fallback, which
/// reads only the holder's slots, would not find it.
#[test]
fn a_fault_after_a_held_star_is_detached_leaves_it_in_no_pool() {
    const STAR_MARKER: u32 = 0xE933_7020;
    let (mut fx, file) = crate::find_location::placeable();
    fx.lamp_mut().position = [40, 40, 1];
    let holder = crate::find_location::widget(&mut fx);
    assert_eq!(
        crate::find_location::find(&mut fx, LAMP as i16, holder, 1),
        Ok(true)
    );
    let star_file = fx.add_type(STAR_MARKER, 4, 0, 4096);
    fx.add(
        star_file,
        4096,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            0,
            RETURN_TRUE,
        )]),
    );
    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let star = interpreter.create(&mut fx.world, STAR_MARKER).unwrap();

    // The holder holds the star in its slot 1, and the star is in use.
    let at = fx.world.objects[&holder].position;
    let held = fx.world.objects.get_mut(&star).unwrap();
    held.position = at;
    // Container and slot (words 2, 3), and what it stands on and in
    // which slot (words 26, 14).
    held.data[2] = holder as i16;
    held.data[3] = 1;
    held.data[26] = holder as i16;
    held.data[14] = 1;
    let holding = fx.world.objects.get_mut(&holder).unwrap();
    holding.slots.resize(
        2,
        sims_formats::saved_objects::Slot {
            unknown: 0,
            object: 0,
        },
    );
    holding.slots[1].object = star as i16;
    fx.world.indicator_queues.stars_in_use = vec![star];
    fx.world.indicator_queues.available_stars = vec![];

    // The holder's PLACED (function 9, tree 4300) calls a tree that is
    // not there.
    let table = fx
        .program
        .libraries
        .get_mut(&file)
        .unwrap()
        .callbacks
        .get_mut(&128)
        .unwrap();
    table.resize(
        10,
        sims_formats::execution::ObjectFunction {
            condition: 0,
            action: 0,
        },
    );
    table[9].action = 4300;
    fx.add(
        file,
        4300,
        tree(vec![ins(4999, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );

    let interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    let fault = interpreter.remove_now(&mut fx.world, holder).unwrap_err();
    assert_eq!(fault.object, holder, "{fault}");
    // The star was detached and parked off the lot before the fault.
    let parked = &fx.world.objects[&star];
    assert_eq!(parked.data[2], 0);
    assert_eq!(parked.position, [-16, -16, 1]);
    assert_eq!(fx.world.objects[&holder].slots[1].object, 0);
    assert!(fx.world.indicator_queues.stars_in_use.is_empty());
    assert!(fx.world.indicator_queues.available_stars.is_empty());
}

#[test]
fn an_object_whose_holder_is_queued_is_not_queued() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(1, 0));
    // Widget 4 sits in slot 0 of widget 3 (data word 26 names the holder,
    // word 14 the slot plus one), and widget 3 is queued already.
    fx.world.removals.push(3);
    let widget = fx.world.objects.get_mut(&4).unwrap();
    (widget.data[26], widget.data[14]) = (3, 1);
    let before = widget.clone();
    assert_eq!(fx.run_with(3, 4200, 4, &[], BUDGET), Ok(true));
    assert_eq!(fx.world.removals, [3]);
    let widget = &fx.world.objects[&4];
    assert_eq!(widget.data, before.data);
    assert_eq!(widget.position, before.position);
}

#[test]
fn a_tile_of_a_group_fails_loudly() {
    let (mut fx, file) = two();
    fx.add(file, 4200, remover(1, 0));
    fx.world.objects.get_mut(&4).unwrap().multipart =
        Some(sims_formats::saved_objects::Multipart {
            offset: [0; 3],
            group: [0; 3],
            parent: 0,
        });
    let fault = fx.run_with(3, 4200, 4, &[], BUDGET).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
    assert!(fx.world.removals.is_empty());
}

#[test]
fn a_person_removed_other_than_by_itself_yielding_needs_the_lot() {
    // The take-off's animator refresh (0x58da12) is worked out on a copy
    // of the lot through the placement, which this lot without rooms
    // cannot give: a person removed by another object, or going on (flag
    // bit 0), or in a synchronous tree, fails loudly and is not queued.
    // On a lot with rooms: `create_person`.
    let (mut fx, file) = two();
    let person = fx.sim().person.clone();
    assert!(person.is_some());
    fx.world.objects.get_mut(&4).unwrap().person = person;
    fx.add(file, 4200, remover(1, 0));
    fx.thread(3, 4200, &[0; 4]);
    fx.world.objects.get_mut(&3).unwrap().stack[0].stack_object = 4;
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    let fault = interpreter.turn(&mut world, 3, BUDGET).unwrap_err();
    assert!(fault.to_string().contains("placement"), "{fault}");
    assert!(world.removals.is_empty());
    for (flags, synchronous) in [(1, false), (0, true)] {
        let (mut fx, file) = two();
        let person = fx.sim().person.clone();
        fx.world.objects.get_mut(&3).unwrap().person = person;
        fx.add(file, 4200, remover(0, flags));
        let (fault, removals) = if synchronous {
            let fault = fx.run_with(3, 4200, 0, &[], BUDGET).unwrap_err();
            (fault, fx.world.removals.clone())
        } else {
            fx.thread(3, 4200, &[0; 4]);
            let interpreter = fx.interpreter();
            let mut world = fx.world.clone();
            let fault = interpreter.turn(&mut world, 3, BUDGET).unwrap_err();
            (fault, world.removals)
        };
        assert!(fault.to_string().contains("placement"), "{fault}");
        assert!(removals.is_empty());
    }
}
