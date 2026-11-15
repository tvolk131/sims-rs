//! Grab (opcode 4, 0x451dda): the stack object goes into my first
//! container slot, after its place test (0x433311), and a person's
//! container slots include the five its post-constructor adds (0x45962b).
//!
//! The lot and the lamp are Create New Object Instance mode 4's: the lamp
//! on tile (2, 2) in room 1, with two empty container slots at its centre,
//! at height 1. The widget waits off the lot. The Sim stands on the
//! lamp's tile with its six slots empty: the tile slot and the five a
//! person adds, its definition having no SLOT.

use crate::create::container;
use crate::find_location::widget;
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
