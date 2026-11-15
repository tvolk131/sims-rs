//! Grab (opcode 4, 0x451dda): the stack object goes into my first
//! container slot, after its place test (0x433311), and a person's
//! container slots include the five its post-constructor adds (0x45962b).
//! Drop (opcode 5, 0x454430) puts what the slot holds on the floor one
//! tile in front.
//!
//! The lot and the lamp are Create New Object Instance mode 4's: the lamp
//! on tile (2, 2) in room 1, with two empty container slots at its centre,
//! at height 1. The widget waits off the lot. The Sim stands on the
//! lamp's tile with its six slots empty: the tile slot and the five a
//! person adds, its definition having no SLOT.

use crate::create::container;
use crate::find_location::{find, widget};
use crate::support::*;
use sims_formats::execution::Slot;
use sims_formats::saved_objects::Multipart;
use sims_vm::Trap;

const GRAB: u16 = 4;
/// The simulator's mode (global 19): 0 is live mode.
const MODE: usize = 19;
/// The Sim's SLOT chunk, when it has one.
const SIM_SLOTS: u16 = 301;

/// Runs tree 4096 of `me` (the lamp or the Sim), whose one instruction is
/// Grab with the given operands, with the stack object `stack_object`.
fn grab_with(
    fx: &mut Fixture,
    me: u16,
    stack_object: i16,
    operands: [u8; 8],
) -> Result<bool, sims_vm::Fault> {
    let file = if me == SIM { SIM_FILE } else { LAMP_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(GRAB, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(me, 4096, stack_object, &[], 1000)
}

fn grab(fx: &mut Fixture, me: u16, stack_object: i16) -> Result<bool, sims_vm::Fault> {
    grab_with(fx, me, stack_object, [0; 8])
}

/// The lamp's lot, in live mode, with the Sim on the lamp's tile and a
/// widget off the lot that may go at height 1 (data word 4 bit 0) and,
/// with `carried`, at height 6 (bit 5).
fn lot(carried: bool) -> (Fixture, u16) {
    let (mut fx, _) = container();
    fx.world.globals[MODE] = 0;
    let sim = fx.sim_mut();
    sim.position = [40, 40, 1];
    sim.data[29] = 1;
    sim.slots = vec![empty(); 6];
    let id = widget(&mut fx);
    if carried {
        fx.world.objects.get_mut(&id).unwrap().data[4] = 0x21;
    }
    (fx, id)
}

fn empty() -> sims_formats::saved_objects::Slot {
    sims_formats::saved_objects::Slot {
        unknown: 0,
        object: 0,
    }
}

fn held(object: &sims_sim::Object) -> Vec<i16> {
    object.slots[1..].iter().map(|s| s.object).collect()
}

#[test]
fn grab_puts_the_stack_object_in_my_first_container_slot() {
    let (mut fx, id) = lot(false);
    assert_eq!(fx.world.objects[&id].position, [-16, -16, 1]);
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
    let widget = &fx.world.objects[&id];
    // At the lamp's position, in its script slot 0 (saved slot 1), which
    // data words 2 and 3 name; the link words name the lamp and slot 1.
    assert_eq!(widget.position, [40, 40, 1]);
    assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, 0));
    assert_eq!((widget.data[14], widget.data[26]), (1, LAMP as i16));
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
}

#[test]
fn grab_ignores_its_operands() {
    let (mut fx, id) = lot(false);
    let mut other = fx.world.clone();
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
    std::mem::swap(&mut fx.world, &mut other);
    assert_eq!(grab_with(&mut fx, LAMP, id as i16, [0xff; 8]), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), hash_without_frames(&other));
}

#[test]
fn grab_without_a_stack_object_changes_nothing() {
    let (mut fx, _) = lot(false);
    let before = hash_without_frames(&fx.world);
    for stack_object in [0, -1, 77] {
        assert_eq!(grab(&mut fx, LAMP, stack_object), Ok(false));
        assert_eq!(hash_without_frames(&fx.world), before);
    }
}

#[test]
fn grab_takes_the_false_branch_when_the_slot_refuses() {
    let refusals: [fn(&mut Fixture, u16); 4] = [
        // The slot holds another object.
        |fx, _| fx.lamp_mut().slots[1].object = SIM as i16,
        // The lamp takes less than the widget weighs (data word 27, 0).
        |fx, _| fx.lamp_mut().data[28] = -1,
        // The lamp is not solid (data word 8 flag 4).
        |fx, _| fx.lamp_mut().data[8] = 4,
        // The widget may not go at height 1 (data word 4 bit 0).
        |fx, id| fx.world.objects.get_mut(&id).unwrap().data[4] = 2,
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let (mut fx, id) = lot(false);
        refuse(&mut fx, id);
        let before = hash_without_frames(&fx.world);
        assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(false), "case {case}");
        assert_eq!(hash_without_frames(&fx.world), before, "case {case}");
    }
    // An object cannot hold itself.
    let (mut fx, _) = lot(false);
    let before = hash_without_frames(&fx.world);
    assert_eq!(grab(&mut fx, LAMP, LAMP as i16), Ok(false));
    assert_eq!(hash_without_frames(&fx.world), before);
}

#[test]
fn a_person_carries_in_its_first_own_slot_at_height_6() {
    // The widget may go at height 1 only: the Sim's slot 0 is at height 6
    // and refuses it.
    let (mut fx, id) = lot(false);
    let before = hash_without_frames(&fx.world);
    assert_eq!(grab(&mut fx, SIM, id as i16), Ok(false));
    assert_eq!(hash_without_frames(&fx.world), before);
    // With bit 5 of data word 4 it goes in.
    let (mut fx, id) = lot(true);
    assert_eq!(grab(&mut fx, SIM, id as i16), Ok(true));
    let widget = &fx.world.objects[&id];
    assert_eq!(widget.position, [40, 40, 1]);
    assert_eq!((widget.data[2], widget.data[3]), (SIM as i16, 0));
    assert_eq!((widget.data[14], widget.data[26]), (1, SIM as i16));
    assert_eq!(held(fx.sim()), [id as i16, 0, 0, 0, 0]);
}

#[test]
fn a_persons_definition_slots_come_before_its_own() {
    // The Sim's definition has one container slot at height 1, which is
    // its slot 0; the five a person adds follow.
    let (mut fx, id) = lot(false);
    fx.words(SIM_FILE)[20] = SIM_SLOTS as i16;
    let slot = Slot {
        kind: 0,
        offset: [0.0; 3],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [16; 3],
        max_size: 0,
        unknown: 0,
        gradient: 0.1875,
        height: 0,
        facing: -2,
        resolution: 16,
    };
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .slots
        .insert(SIM_SLOTS, vec![slot]);
    fx.sim_mut().slots.push(empty());
    assert_eq!(grab(&mut fx, SIM, id as i16), Ok(true));
    assert_eq!(held(fx.sim()), [id as i16, 0, 0, 0, 0, 0]);
    // A person whose slots number neither the definition's and its own
    // fails.
    let (mut fx, id) = lot(true);
    fx.sim_mut().slots.truncate(3);
    let result = grab(&mut fx, SIM, id as i16);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("keeps 3 slots")));
}

#[test]
fn a_person_takes_nothing_outside_live_mode() {
    for mode in [1, 2, 119] {
        let (mut fx, id) = lot(true);
        fx.world.globals[MODE] = mode;
        let before = hash_without_frames(&fx.world);
        assert_eq!(grab(&mut fx, SIM, id as i16), Ok(false), "mode {mode}");
        assert_eq!(hash_without_frames(&fx.world), before, "mode {mode}");
    }
    // The lamp is no person: mode 1 is no matter to it.
    let (mut fx, id) = lot(false);
    fx.world.globals[MODE] = 1;
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
}

#[test]
fn grab_takes_an_object_out_of_the_container_that_holds_it() {
    let (mut fx, id) = lot(true);
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
    assert_eq!(grab(&mut fx, SIM, id as i16), Ok(true));
    assert_eq!(held(fx.lamp()), [0, 0]);
    assert_eq!(held(fx.sim()), [id as i16, 0, 0, 0, 0]);
    let widget = &fx.world.objects[&id];
    assert_eq!((widget.data[2], widget.data[26]), (SIM as i16, SIM as i16));
}

/// A tile of a group refuses any container at once (0x422ac8, code
/// 0x10): the false branch, and nothing changes.
#[test]
fn a_tile_of_a_group_is_never_grabbed() {
    let (mut fx, id) = lot(false);
    fx.world.objects.get_mut(&id).unwrap().multipart = Some(Multipart {
        offset: [0; 3],
        group: [0; 3],
        parent: 0,
    });
    let before = hash_without_frames(&fx.world);
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(false));
    assert_eq!(hash_without_frames(&fx.world), before);
}

/// A put that fails partway, on what the port does not model (here an
/// object facing 8, whose footprint is not worked out), leaves the lot as
/// it was: the widget stays in the lamp.
#[test]
fn a_grab_whose_put_fails_leaves_the_lot_as_it_was() {
    let (mut fx, id) = lot(true);
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
    fx.world.objects.get_mut(&id).unwrap().data[1] = 8;
    // The failed run leaves its own frame on my stack.
    let lot_hash = |world: &sims_sim::World| {
        let mut world = world.clone();
        for object in world.objects.values_mut() {
            object.stack.clear();
        }
        hash_without_frames(&world)
    };
    let before = lot_hash(&fx.world);
    let result = grab(&mut fx, SIM, id as i16);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("facing 8")));
    assert_eq!(lot_hash(&fx.world), before);
}

const DROP: u16 = 5;

/// Runs tree 4096 of the lamp, whose one instruction is Drop with the
/// given operands.
fn drop_with(fx: &mut Fixture, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(DROP, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4096, 0, &[], 1000)
}

#[test]
fn drop_puts_what_i_hold_one_tile_in_front_of_me() {
    // 0x454430: one tile from mine in my direction (0x42228a), on my
    // level, out of my first container slot. The operands are not read.
    let places = [
        [40, 24],
        [56, 24],
        [56, 40],
        [56, 56],
        [40, 56],
        [24, 56],
        [24, 40],
        [24, 24],
    ];
    for (facing, place) in places.into_iter().enumerate() {
        let (mut fx, id) = lot(false);
        assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
        fx.lamp_mut().data[1] = facing as i16;
        assert_eq!(drop_with(&mut fx, [0xff; 8]), Ok(true), "facing {facing}");
        let widget = &fx.world.objects[&id];
        assert_eq!(widget.position, [place[0], place[1], 1], "facing {facing}");
        assert_eq!((widget.data[2], widget.data[14]), (0, -1));
        assert_eq!(held(fx.lamp()), [0, 0]);
    }
}

#[test]
fn drop_takes_the_false_branch_when_nothing_is_held_or_the_place_refuses() {
    // Nothing in my first container slot.
    let (mut fx, _) = lot(false);
    let before = hash_without_frames(&fx.world);
    assert_eq!(drop_with(&mut fx, [0; 8]), Ok(false));
    assert_eq!(hash_without_frames(&fx.world), before);
    // A widget already stands on the tile in front (facing 7, tile
    // (1, 1)): the place test refuses and nothing changes.
    let (mut fx, id) = lot(false);
    assert_eq!(grab(&mut fx, LAMP, id as i16), Ok(true));
    let blocker = widget(&mut fx);
    assert_eq!(find(&mut fx, 0, blocker, 0), Ok(true));
    assert_eq!(fx.world.objects[&blocker].position, [24, 24, 1]);
    fx.lamp_mut().data[1] = 7;
    let before = fx.world.objects[&id].position;
    assert_eq!(drop_with(&mut fx, [0; 8]), Ok(false));
    assert_eq!(fx.world.objects[&id].position, before);
    assert_eq!(fx.world.objects[&blocker].position, [24, 24, 1]);
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
}

const DROP_ONTO: u16 = 43;

/// Runs the Sim's tree 4096, whose one instruction is Drop Onto from the
/// `source` pair of words into the `destination` pair, with the stack
/// object `stack_object` and the tree's four arguments `args`.
fn drop_onto(
    fx: &mut Fixture,
    source: [i16; 2],
    destination: [i16; 2],
    stack_object: i16,
    args: &[i16],
) -> Result<bool, sims_vm::Fault> {
    let mut operands = [0; 8];
    for (n, word) in source.into_iter().chain(destination).enumerate() {
        operands[2 * n..2 * n + 2].copy_from_slice(&word.to_le_bytes());
    }
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(DROP_ONTO, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(SIM, 4096, stack_object, args, 1000)
}

/// The lot with the Sim carrying the widget in its slot 0.
fn carrying() -> (Fixture, u16) {
    let (mut fx, id) = lot(true);
    assert_eq!(grab(&mut fx, SIM, id as i16), Ok(true));
    (fx, id)
}

#[test]
fn drop_onto_puts_what_my_slot_holds_into_a_slot_of_the_stack_object() {
    // Slots given as themselves (first word 0), or by argument: source
    // argument 2 (0), destination argument 0 (1).
    for (source, destination, args) in [([0, 0], [0, 1], [0; 4]), ([1, 2], [1, 0], [1, 0, 0, 0])] {
        let (mut fx, id) = carrying();
        let result = drop_onto(&mut fx, source, destination, LAMP as i16, &args);
        assert_eq!(result, Ok(true), "{source:?} {destination:?}");
        let widget = &fx.world.objects[&id];
        // At the lamp's position, level 1, in its script slot 1.
        assert_eq!(widget.position, [40, 40, 1]);
        assert_eq!((widget.data[2], widget.data[3]), (LAMP as i16, 1));
        assert_eq!(widget.data[26], LAMP as i16);
        assert_eq!(held(fx.lamp()), [0, id as i16]);
        assert_eq!(held(fx.sim()), [0; 5]);
    }
}

#[test]
fn drop_onto_from_an_empty_slot_takes_the_false_branch() {
    let (mut fx, _) = lot(true);
    let before = hash_without_frames(&fx.world);
    assert_eq!(
        drop_onto(&mut fx, [0, 0], [0, 0], LAMP as i16, &[]),
        Ok(false)
    );
    assert_eq!(hash_without_frames(&fx.world), before);
}

#[test]
fn drop_onto_takes_the_false_branch_when_the_place_test_refuses() {
    let refusals: [fn(&mut Fixture, u16); 2] = [
        // The lamp's slot 1 holds another object.
        |fx, _| fx.lamp_mut().slots[2].object = LAMP as i16,
        // The widget may not go at height 1 (data word 4 bit 0).
        |fx, id| fx.world.objects.get_mut(&id).unwrap().data[4] = 0x20,
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let (mut fx, id) = carrying();
        refuse(&mut fx, id);
        // The Grab's true is the Sim's last result; the false branch
        // leaves false.
        fx.sim_mut().last_result = false;
        let before = hash_without_frames(&fx.world);
        let result = drop_onto(&mut fx, [0, 0], [0, 1], LAMP as i16, &[]);
        assert_eq!(result, Ok(false), "case {case}");
        assert_eq!(hash_without_frames(&fx.world), before, "case {case}");
    }
}

#[test]
fn drop_onto_names_a_slot_i_have_and_a_stack_object() {
    // 0x44e9e6: the source slot must be from 0 to my records less 2 (the
    // Sim has six: the tile chain and slots 0 to 4), before the stack
    // object is looked at.
    let (mut fx, _) = carrying();
    for (slot, stack_object, expected) in [
        (5, LAMP as i16, 0x1b),
        (-1, LAMP as i16, 0x1b),
        (5, 0, 0x1b),
        (0, 0, 0x15),
        (0, 77, 0x15),
    ] {
        let result = drop_onto(&mut fx, [0, slot], [0, 1], stack_object, &[]);
        assert_eq!(code(result), Some(expected), "{slot} {stack_object}");
    }
}

#[test]
fn drop_onto_naming_an_argument_the_frame_does_not_have_is_native_error_8() {
    // 0x44e983 and 0x44e9bb: an argument below 0 or past the frame's four,
    // for either slot; the source is read first.
    let (mut fx, _) = carrying();
    for (source, destination) in [
        ([1, 4], [0, 0]),
        ([1, -1], [0, 0]),
        ([0, 0], [1, 4]),
        ([0, 0], [1, -1]),
        ([1, 4], [0, 99]),
    ] {
        let result = drop_onto(&mut fx, source, destination, LAMP as i16, &[]);
        assert_eq!(code(result), Some(8), "{source:?} {destination:?}");
    }
}
