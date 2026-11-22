//! Snap (opcode 46, 0x454976): a person put on the place a routing slot
//! of the stack object names, at once.
//!
//! The lot is Find Location For's: the six-tile one of the room tests,
//! with no floors, where the widget may stand without one. The person is
//! a widget person made on the lamp's tile; the lamp, which stands on no
//! tile of the object grid, is the object whose slot it snaps to.

use crate::find_location::{create, person_widget};
use crate::support::*;
use sims_core::F32;
use sims_formats::execution::Slot;
use sims_vm::Trap;

const SNAP: u16 = 46;
/// The person's tree that runs Snap.
pub(crate) const TREE: u16 = 4200;
/// The lamp's SLOT chunk.
const SLOTS: u16 = 300;
/// Slot flags: the placement skips its checks, the direction follows the
/// object's facing.
const SKIP_CHECKS: u32 = 0x800;
const TURNED: u32 = 0x1000;

/// A routing slot at the object, with the given flags.
pub(crate) fn slot(flags: u32) -> Slot {
    Slot {
        kind: 3,
        offset: [0.0; 3],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags,
        snap_target: -1,
        proximity: [16; 3],
        max_size: 0,
        unknown: 0,
        gradient: 0.1875,
        height: 0,
        facing: -2,
        resolution: 16,
    }
}

/// The person, made on the lamp's tile (2, 2) in room 1, facing 3, and
/// the widget's file. The lamp has one routing slot with `flags`.
pub(crate) fn snapper(flags: u32) -> (Fixture, u16) {
    let mut fx = person_widget();
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    let me = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    fx.words(LAMP_FILE)[20] = SLOTS as i16;
    set_slots(&mut fx, vec![slot(flags)]);
    let person = fx.world.objects.get_mut(&me).unwrap();
    person.data[1] = 3;
    person.data[52] = 7;
    assert_eq!(person.position, [40, 40, 1]);
    assert_eq!(person.data[29], 1);
    (fx, me)
}

pub(crate) fn set_slots(fx: &mut Fixture, slots: Vec<Slot>) {
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .insert(SLOTS, slots);
}

/// Runs Snap as `me` in mode `mode` with slot `index`, flags `flags` and
/// the stack object `target`.
fn snap(
    fx: &mut Fixture,
    me: u16,
    target: i16,
    index: i16,
    mode: i16,
    flags: u8,
) -> Result<bool, sims_vm::Fault> {
    let [i0, i1] = index.to_le_bytes();
    let [m0, m1] = mode.to_le_bytes();
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            SNAP,
            RETURN_TRUE,
            RETURN_FALSE,
            [i0, i1, m0, m1, flags, 0, 0, 0],
        )]),
    );
    fx.run_with(me, TREE, target, &[], 100)
}

/// What Snap changes: my position and data words, the object grid and
/// my animator's position and destination.
fn unmoved(fx: &Fixture, me: u16) -> impl PartialEq + std::fmt::Debug + use<> {
    let person = &fx.world.objects[&me];
    let animator = &fx.world.animators[&me];
    (
        person.position,
        person.data.clone(),
        fx.world.arrays[&3].data.clone(),
        animator.position,
        animator.destination,
    )
}

/// Sets what not fitting changes, so [`unmoved`] can compare the rest:
/// only word 52, to 0 (the fixture's 7 is written over; no object is
/// collided with, so word 54 stays 0).
fn unfit(fx: &mut Fixture, me: u16) {
    fx.world.objects.get_mut(&me).unwrap().data[52] = 0;
}

fn floats(values: [i32; 3]) -> [F32; 3] {
    values.map(|n| F32::from_bits((n as f32).to_bits()))
}

#[test]
fn snap_puts_me_on_the_object_facing_my_own_way() {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    // The lamp stands outside the walls, on tile (4, 1) in room 0. With
    // flag 0x800 the walls between are not checked.
    fx.lamp_mut().position = [72, 24, 1];
    fx.lamp_mut().data[1] = 5;
    fx.world.animators.get_mut(&me).unwrap().position = floats([40, 40, 3]);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 2), Ok(true));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [72, 24, 1]);
    assert_eq!(person.data[1], 3);
    assert_eq!(person.data[52], 0);
    assert_eq!(person.data[29], 0);
    // The object grid holds it on its new tile and no longer on its old.
    let grid = &fx.world.arrays[&3].data;
    let cell = |a: usize, b: usize| {
        i16::from_le_bytes([grid[(a * 64 + b) * 2], grid[(a * 64 + b) * 2 + 1]])
    };
    assert_eq!(cell(4, 1), me as i16);
    assert_eq!(cell(2, 2), 0);
    // The animator was unset, then placed itself on the person's tile.
    let animator = &fx.world.animators[&me];
    assert_eq!(animator.position, floats([72, 24, 3]));
    assert_eq!(animator.destination, floats([72, 24, 3]));
}

#[test]
fn slot_flag_0x1000_turns_me_with_the_object() {
    // The object's facing, turned by the lowest of bits 0 to 6 set, kept
    // in 0 to 7.
    for (bits, facing, expected) in [
        (0, 5, 5),
        (4, 5, 7),
        (2 | 8, 7, 0),
        (0x40, 6, 4),
        (0x80, 6, 6),
    ] {
        let (mut fx, me) = snapper(SKIP_CHECKS | TURNED | bits);
        fx.lamp_mut().position = [56, 24, 1];
        fx.lamp_mut().data[1] = facing;
        assert_eq!(
            snap(&mut fx, me, LAMP as i16, 0, 3, 0),
            Ok(true),
            "bits {bits:#x}"
        );
        assert_eq!(fx.world.objects[&me].data[1], expected, "bits {bits:#x}");
        let animator = &fx.world.animators[&me];
        assert_eq!(animator.position[2], floats([0, 0, i32::from(expected)])[2]);
    }
    // Without the flag, my own facing, whatever the bits.
    let (mut fx, me) = snapper(SKIP_CHECKS | 4);
    fx.lamp_mut().position = [56, 24, 1];
    fx.lamp_mut().data[1] = 1;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[1], 3);
}

#[test]
fn upstairs_a_person_needs_a_floor() {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    // Tile (3, 2) on level 2: the floor cell is column 3, row 2.
    fx.lamp_mut().position = [56, 40, 2];
    let start = fx.world.clone();
    // Without placement flag 0x200 a tile above the ground needs a floor
    // (0x4330b7). The person does not fit. A floor on tile (2, 3) is not
    // this tile's.
    fx.world.arrays.get_mut(&101).unwrap().data[(2 * 64 + 3) * 2] = 1;
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert_eq!(result, Ok(false));
    // With a floor (here in the word's high byte), and placement flag 1
    // to stand on one, it does.
    fx.world = start.clone();
    fx.world.arrays.get_mut(&101).unwrap().data[(3 * 64 + 2) * 2 + 1] = 1;
    fx.world.objects.get_mut(&me).unwrap().data[42] |= 1;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [56, 40, 2]);
    assert_eq!(person.data[29], 1024);
    // Placement flag 0x200 skips the test: no floor is needed.
    fx.world = start;
    fx.world.objects.get_mut(&me).unwrap().data[42] |= 0x200;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].position, [56, 40, 2]);
}

#[test]
fn a_person_with_no_animator_entry_has_the_loaded_one() {
    // A person whose animator has not run has no entry; Snap writes the
    // loaded animator as its entry, unset and placed again.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [72, 24, 1];
    fx.world.animators.remove(&me);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    let mut expected = sims_sim::animator::LOADED;
    expected.position = floats([72, 24, 3]);
    expected.destination = expected.position;
    let animator = &fx.world.animators[&me];
    assert_eq!(animator.position, expected.position);
    assert_eq!(animator.destination, expected.destination);
    assert_eq!(animator.clock, expected.clock);
}

#[test]
fn the_slot_tables_own_record_and_flags_come_first() {
    // Flags the table holds in place of the type's: here they add
    // 0x1000, so the direction follows the lamp.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    fx.lamp_mut().data[1] = 6;
    fx.world
        .routing
        .slot_flags
        .insert((LAMP, 0), (SKIP_CHECKS | TURNED) as i32);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[1], 6);
    // A record written over the type's is used whole: here it has no
    // flag 0x800.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    let record = sims_sim::route::RouteSlot {
        offset: [0; 3],
        number: 1,
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [16; 3],
        gradient: 0,
        facing: -2,
        resolution: 16,
    };
    fx.world.routing.slots.insert((LAMP, 0), record);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert!(refused(result));
}

/// Whether Snap found that I do not fit: the false branch.
fn refused(result: Result<bool, sims_vm::Fault>) -> bool {
    result == Ok(false)
}

#[test]
fn not_fitting_sets_words_52_and_54_from_the_placement_error() {
    // The lot's border leaves placement error 1: word 52 becomes 2. My
    // room refusing leaves the 0 Snap cleared it to (0x454d64): word 52
    // becomes 0. Word 54 names no object either way.
    for (place, word_52) in [([8, 40, 1], 2), ([56, 24, 1], 0)] {
        let (mut fx, me) = snapper(0);
        fx.world.objects.get_mut(&me).unwrap().data[54] = 9;
        fx.lamp_mut().position = place;
        assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(false));
        let data = &fx.world.objects[&me].data;
        assert_eq!((data[52], data[54]), (word_52, 0), "{place:?}");
    }
    // Another person stands on the place, in my way: the collision (code
    // 11, 0x433b42) names it, and word 54 becomes its ID.
    let (mut fx, me) = snapper(0);
    fx.lamp_mut().position = [56, 56, 1];
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(create(&mut fx, 0, 0, SIM as i16), Ok(true));
    let other = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    assert_eq!(fx.world.objects[&other].position, [56, 56, 1]);
    let start = fx.world.clone();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(false));
    let data = &fx.world.objects[&me].data;
    assert_eq!((data[52], data[54]), (0, other as i16));
    assert_eq!(fx.world.objects[&me].position, [40, 40, 1]);
    // With flag bit 1 that person would be asked to make way (0x45bd5e),
    // but the catalog has no marker type (0x43ac10): not asked.
    fx.world = start.clone();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 2), Ok(false));
    let data = &fx.world.objects[&me].data;
    assert_eq!((data[52], data[54]), (0, other as i16));
    // With the marker type, whose menu entry 3 moves the person.
    let marker_file = fx.add_type(0x7c4, 4, 0, 0x1006);
    fx.add(
        marker_file,
        0x1006,
        tree(vec![
            assign(scope::PARAMETERS, 0, scope::LITERAL, 100, 1),
            ins(0, 0, 0, [0; 8]),
        ]),
    );
    let library = fx.program.libraries.get_mut(&marker_file).unwrap();
    library.definitions.get_mut(&128).unwrap()[7] = 1007;
    let entry = sims_formats::behavior::Interaction {
        action: 4302,
        test: 0,
        string_index: 3,
        flags: 0,
        autonomy: 50,
        attenuation_code: 0,
        attenuation: 0.0,
        joining_index: -1,
        motives: Vec::new(),
        flags2: 0,
    };
    library.interactions.insert(
        1007,
        Some(sims_formats::behavior::InteractionTable {
            version: 7,
            interactions: vec![entry; 4],
        }),
    );
    fx.world = start;
    fx.world
        .objects
        .get_mut(&me)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .active
        .priority = 77;
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 2), Ok(false));
    let data = &fx.world.objects[&me].data;
    assert_eq!((data[52], data[54]), (1, other as i16));
    // A marker stands where the other person does, its slot written
    // over: the four straight directions, resolution 16.
    let marker = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    assert_eq!(fx.world.objects[&marker].guid, 0x7c4);
    assert_eq!(fx.world.objects[&marker].position, [56, 56, 1]);
    let slot = fx.world.routing.slots[&(marker, 0)];
    assert_eq!(slot.flags, 0x55 | 0x100 | 0x2000);
    assert_eq!((slot.proximity, slot.resolution), ([16, 80, 16], 16));
    // The other person has the marker's move action, at my running
    // action's priority.
    let queue = &fx.world.objects[&other].person.as_ref().unwrap().queue;
    assert!(
        queue.iter().any(|a| a.target == marker as i16
            && a.table_index == 3
            && a.priority == 77
            && a.action == 4302),
        "{queue:?}"
    );
    // Asked once, the person is not asked again.
    let data = fx.world.objects.get_mut(&me).unwrap();
    data.data[52] = 9;
    let count = fx.world.objects.len();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 2), Ok(false));
    assert_eq!(fx.world.objects[&me].data[52], 0);
    assert_eq!(fx.world.objects.len(), count);
}

/// Sets wall segments on a tile of the ground level (lot array 2, 14
/// bytes a cell, the segments in byte 0).
fn wall(fx: &mut Fixture, tile: [usize; 2], segments: u8) {
    let walls = fx.world.arrays.get_mut(&2).unwrap();
    walls.data[(tile[0] * 64 + tile[1]) * 14] |= segments;
}

#[test]
fn without_flag_0x800_the_place_must_be_inside_the_border_and_my_room() {
    // Tile (3, 3) is in my room, with no wall between: I am put there.
    let (mut fx, me) = snapper(0);
    fx.lamp_mut().position = [56, 56, 1];
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].position, [56, 56, 1]);
    // Tile (3, 1) is in room 0, and tile (0, 2) on the lot's border: the
    // border's placement error, 1, makes word 52 2 (0x454e66).
    for (place, word_52) in [([56, 24, 1], 0), ([8, 40, 1], 2)] {
        let (mut fx, me) = snapper(0);
        fx.lamp_mut().position = place;
        fx.world.objects.get_mut(&me).unwrap().data[52] = word_52;
        let start = unmoved(&fx, me);
        assert!(
            refused(snap(&mut fx, me, LAMP as i16, 0, 3, 0)),
            "{place:?}"
        );
        assert_eq!(unmoved(&fx, me), start);
    }
}

#[test]
fn without_flag_0x800_no_wall_may_close_the_way() {
    // From (2, 2) to (2, 3) is direction 4 (the second tile coordinate is
    // the original's first): segment 8 of my tile closes it, and segment
    // 2 of the place's tile closes direction 0 back.
    for (tile, segments) in [([2, 2], 8), ([2, 3], 2)] {
        let (mut fx, me) = snapper(0);
        fx.lamp_mut().position = [40, 56, 1];
        wall(&mut fx, tile, segments);
        assert!(refused(snap(&mut fx, me, LAMP as i16, 0, 3, 0)), "{tile:?}");
    }
    // Another segment closes nothing that way.
    let (mut fx, me) = snapper(0);
    fx.lamp_mut().position = [40, 56, 1];
    wall(&mut fx, [2, 2], 4);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    // A place where I stand is not looked at for walls.
    let (mut fx, me) = snapper(0);
    fx.lamp_mut().position = [40, 40, 1];
    wall(&mut fx, [2, 2], 0xf);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
}

#[test]
fn the_room_is_read_at_the_places_tile_not_its_mirror() {
    // Tile (3, 2) is in room 9 and tile (2, 3) in mine: a lookup with the
    // coordinates swapped would answer the other way.
    let room_at = |fx: &mut Fixture, tile: [usize; 2], room: i16| {
        fx.world.rooms.map[0][tile[0] * 64 + tile[1]] = room;
    };
    let (mut fx, me) = snapper(0);
    room_at(&mut fx, [3, 2], 9);
    room_at(&mut fx, [2, 3], 1);
    fx.lamp_mut().position = [56, 40, 1];
    assert!(refused(snap(&mut fx, me, LAMP as i16, 0, 3, 0)));
    let (mut fx, me) = snapper(0);
    room_at(&mut fx, [3, 2], 9);
    room_at(&mut fx, [2, 3], 1);
    fx.lamp_mut().position = [40, 56, 1];
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
}

#[test]
fn the_walls_back_from_the_place_are_read_on_my_level() {
    // The place is tile (2, 3) on level 2, in my room there, and the
    // person needs no floor (placement flag 0x200). The wall back from
    // it (direction 0, segment 2) is looked for on my level, 1: on level
    // 2 (lot array 102) it closes nothing.
    let upstairs = |walls: u16| {
        let (mut fx, me) = snapper(0);
        fx.world.rooms.map[1][2 * 64 + 3] = 1;
        fx.world.objects.get_mut(&me).unwrap().data[42] |= 0x200;
        fx.lamp_mut().position = [40, 56, 2];
        fx.world.arrays.get_mut(&walls).unwrap().data[(2 * 64 + 3) * 14] |= 2;
        snap(&mut fx, me, LAMP as i16, 0, 3, 0)
    };
    assert_eq!(upstairs(102), Ok(true));
    assert!(refused(upstairs(2)));
}

#[test]
fn routing_slots_are_counted_among_the_routing_entries_only() {
    // The SLOT chunk's first entry holds objects (kind 0), so routing
    // slot 0 is its second entry, and there is no routing slot 1.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    fx.lamp_mut().data[1] = 6;
    let container = Slot { kind: 0, ..slot(0) };
    set_slots(&mut fx, vec![container, slot(SKIP_CHECKS | TURNED)]);
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, 1, 3, 0)), Some(0x1b));
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[1], 6);
}

#[test]
fn snap_checks_its_object_mode_and_slot() {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    let start = unmoved(&fx, me);
    assert_eq!(code(snap(&mut fx, me, 99, 0, 3, 0)), Some(0x15));
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, 0, 5, 0)), Some(0x24));
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, 1, 3, 0)), Some(0x1b));
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, -1, 3, 0)), Some(0x1b));
    assert_eq!(unmoved(&fx, me), start);
}

/// Sets the global file's SLOT 100, whose kind 3 entries are the lot's
/// global routing slots.
fn set_global_slots(fx: &mut Fixture, slots: Vec<Slot>) {
    let global = fx.program.global;
    fx.program
        .libraries
        .get_mut(&global)
        .unwrap()
        .slots
        .insert(100, slots);
}

/// The snapper, with the lamp's one routing slot keeping my facing and
/// the lot's global routing slots: a container entry, then routing
/// slots turning by 1 (flag bit 1) and by 4 (bit 4) with the lamp. The
/// lamp faces 5.
fn global_snapper() -> (Fixture, u16) {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    fx.lamp_mut().data[1] = 5;
    set_global_slots(
        &mut fx,
        vec![
            Slot { kind: 0, ..slot(0) },
            slot(SKIP_CHECKS | TURNED | 2),
            slot(SKIP_CHECKS | TURNED | 0x10),
        ],
    );
    (fx, me)
}

#[test]
fn mode_4_names_one_of_the_lots_global_routing_slots() {
    // Mode 4 reads the lot's table (0x454ab6: the stack object's lot,
    // +0x9c), counting only its routing entries: slot 0 turns me by 1
    // from the lamp's facing, slot 1 by 4. The place is still the stack
    // object's.
    for (index, facing) in [(0, 6), (1, 1)] {
        let (mut fx, me) = global_snapper();
        assert_eq!(snap(&mut fx, me, LAMP as i16, index, 4, 0), Ok(true));
        let person = &fx.world.objects[&me];
        assert_eq!(person.position, [56, 24, 1], "slot {index}");
        assert_eq!(person.data[1], facing, "slot {index}");
        assert_eq!(person.data[52], 0);
    }
    // Mode 3 with the same index reads the lamp's own slot: my facing.
    let (mut fx, me) = global_snapper();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[1], 3);
}

#[test]
fn mode_4_reads_the_lots_flags_for_its_slot_not_the_objects() {
    // The flags the lot's table holds in place of the type's are kept
    // under object 0; the lamp's own for the same index are not read.
    let (mut fx, me) = global_snapper();
    fx.world
        .routing
        .slot_flags
        .insert((LAMP, 0), (SKIP_CHECKS | TURNED | 8) as i32);
    fx.world
        .routing
        .slot_flags
        .insert((0, 0), (SKIP_CHECKS | TURNED | 4) as i32);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 4, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[1], 7);
}

#[test]
fn mode_4_past_the_lots_table_is_native_error_0x1b() {
    // Two global routing slots, and the lamp's table is not looked at.
    let (mut fx, me) = global_snapper();
    set_slots(&mut fx, vec![slot(0), slot(0), slot(0)]);
    let start = unmoved(&fx, me);
    for index in [2, -1] {
        assert_eq!(
            code(snap(&mut fx, me, LAMP as i16, index, 4, 0)),
            Some(0x1b),
            "slot {index}"
        );
    }
    // A lot with no global slots at all.
    set_global_slots(&mut fx, Vec::new());
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, 0, 4, 0)), Some(0x1b));
    assert_eq!(unmoved(&fx, me), start);
}

#[test]
fn mode_4_on_myself_turns_me_where_i_stand() {
    // As the snack order does: I am the stack object, and the global slot
    // (flags 0x1010, with no 0x800) turns me by 4 from my own facing. The
    // place is where I stand, so the walls are not looked at.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    set_global_slots(&mut fx, vec![slot(TURNED | 0x10)]);
    wall(&mut fx, [2, 2], 0xf);
    assert_eq!(snap(&mut fx, me, me as i16, 0, 4, 0), Ok(true));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [40, 40, 1]);
    assert_eq!(person.data[1], 7);
    assert_eq!(person.data[52], 0);
}

#[test]
fn an_offset_turns_with_the_objects_facing() {
    // The original works in its own order, Y first (object +0xf4 is our
    // position[1]): the offset's second coordinate goes along that, its
    // first along X. Turned by the lamp's facing through a matrix in
    // world units and rounded back half away from zero (0x454b93 to
    // 0x454c54), the offset (0, -n) points where the facing's step does:
    // -Y at facing 0, +X at facing 2. Tick 2515 of the Goth house's day
    // has person 21 get up from chair 174 (facing 2) to its slot 1, (0,
    // -16), at (440, 376) from the chair's (424, 376), as the original
    // does.
    // The lamp stands where I do, in the middle of tile (2, 2).
    let cases: [([f32; 2], i16, [i32; 2]); 9] = [
        ([4.0, 0.0], 0, [44, 40]),
        ([4.0, 0.0], 2, [40, 44]),
        ([4.0, 0.0], 4, [36, 40]),
        ([4.0, 0.0], 6, [40, 36]),
        // 0.75 times the root of a half, 0.53033, on 7.5 in world units:
        // 42.828 back in the lot's.
        ([4.0, 0.0], 1, [43, 43]),
        ([0.0, -2.0], 0, [40, 38]),
        ([0.0, -16.0], 0, [40, 24]),
        ([0.0, -16.0], 2, [56, 40]),
        // The facing is the raw data word: 10 turns as 2 does.
        ([4.0, 0.0], 10, [40, 44]),
    ];
    for (offset, facing, place) in cases {
        let (mut fx, me) = snapper(SKIP_CHECKS);
        fx.lamp_mut().position = [40, 40, 1];
        fx.lamp_mut().data[1] = facing;
        set_slots(
            &mut fx,
            vec![Slot {
                offset: [offset[0], offset[1], 5.0],
                ..slot(SKIP_CHECKS)
            }],
        );
        assert_eq!(
            snap(&mut fx, me, LAMP as i16, 0, 3, 0),
            Ok(true),
            "{offset:?} {facing}"
        );
        let person = &fx.world.objects[&me];
        assert_eq!(
            person.position,
            [place[0], place[1], 1],
            "{offset:?} {facing}"
        );
    }
}

#[test]
fn what_snap_does_not_port_fails_loudly() {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 24, 1];
    set_slots(&mut fx, vec![slot(SKIP_CHECKS)]);
    let start = unmoved(&fx, me);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 1);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454c7a")));
    assert_eq!(unmoved(&fx, me), start);
}

#[test]
fn an_offset_of_negative_zero_or_a_height_is_no_offset() {
    // Only the offset's first two coordinates are read, and -0 is zero.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    set_slots(
        &mut fx,
        vec![Slot {
            offset: [-0.0, 0.0, 5.0],
            ..slot(SKIP_CHECKS)
        }],
    );
    fx.lamp_mut().position = [56, 24, 1];
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].position, [56, 24, 1]);
}

#[test]
fn go_to_routing_slot_numbers_its_slot_across_the_whole_chunk() {
    // Kinds 0, 3, 0, 1, 3: routing slots 0 and 1 are the chunk's entries
    // 1 and 4, which the slot table numbers 2 and 5 (0x46e343 numbers
    // every entry), while the flags it keeps are keyed by routing slot.
    for (routing, number) in [(0i16, 2), (1, 5)] {
        let (mut fx, me) = snapper(0);
        let kinds = [0u16, 3, 0, 1, 3];
        set_slots(
            &mut fx,
            kinds.iter().map(|&kind| Slot { kind, ..slot(0) }).collect(),
        );
        fx.lamp_mut().position = [56, 24, 1];
        let [i0, i1] = routing.to_le_bytes();
        let guid = fx.world.objects[&me].guid;
        let file = fx.catalog.object(guid).unwrap().resource;
        fx.add(
            file,
            TREE,
            tree(vec![ins(
                45,
                RETURN_TRUE,
                RETURN_FALSE,
                [i0, i1, 1, 0, 0, 0, 0, 0],
            )]),
        );
        // The route then stops at a step not ported for this person; the
        // context it made first holds the slot's record.
        assert!(fx.run_with(me, TREE, LAMP as i16, &[], 1).is_err());
        let contexts = &fx.world.routing.routes[&me].contexts;
        let numbers: Vec<i32> = contexts.iter().map(|c| c.slot.number).collect();
        assert_eq!(numbers, [number], "routing slot {routing}");
        let keys: Vec<_> = fx.world.routing.slot_flags.keys().copied().collect();
        assert_eq!(keys, [(LAMP, routing as u16)], "routing slot {routing}");
    }
}

type GoTo = (Result<bool, sims_vm::Fault>, Vec<i32>, Vec<(u16, u16)>);

/// Runs Go To Routing Slot as `me` on the lamp with operand words 0 and
/// 1 and the tree's parameters `args`. The lamp's chunk is kinds 0, 3, 0,
/// 1, 3, and the global file's SLOT 100 kinds 3, 0, 3. Returns the
/// result, the slot numbers of the route contexts made and the keys of
/// the slot flags kept.
fn go_to(number: i16, source: i16, args: &[i16]) -> GoTo {
    let (mut fx, me) = snapper(0);
    let kinds = [0u16, 3, 0, 1, 3];
    set_slots(
        &mut fx,
        kinds.iter().map(|&kind| Slot { kind, ..slot(0) }).collect(),
    );
    let global = fx.program.global;
    fx.program.libraries.get_mut(&global).unwrap().slots.insert(
        100,
        [3u16, 0, 3]
            .iter()
            .map(|&kind| Slot { kind, ..slot(0) })
            .collect(),
    );
    fx.lamp_mut().position = [56, 24, 1];
    let [n0, n1] = number.to_le_bytes();
    let [s0, s1] = source.to_le_bytes();
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        tree(vec![ins(
            45,
            RETURN_TRUE,
            RETURN_FALSE,
            [n0, n1, s0, s1, 0, 0, 0, 0],
        )]),
    );
    let result = fx.run_with(me, TREE, LAMP as i16, args, 1);
    let numbers = fx
        .world
        .routing
        .routes
        .get(&me)
        .map(|r| r.contexts.iter().map(|c| c.slot.number).collect())
        .unwrap_or_default();
    let keys = fx.world.routing.slot_flags.keys().copied().collect();
    (result, numbers, keys)
}

#[test]
fn go_to_routing_slot_names_its_slot_by_parameter_or_from_the_global_slots() {
    // Source 0: routing slot 1 of the lamp, from parameter 2. The route
    // stops at a step not ported for this person, after its context.
    let (result, numbers, keys) = go_to(2, 0, &[0, 0, 1, 0]);
    assert!(result.is_err());
    assert_eq!((numbers, keys), (vec![5], vec![(LAMP, 1)]));
    // Source 2: global routing slot 1, the chunk's entry 2, numbered 3;
    // its flags are the lot's, kept under object 0.
    let (result, numbers, keys) = go_to(1, 2, &[]);
    assert!(result.is_err());
    assert_eq!((numbers, keys), (vec![3], vec![(0, 1)]));
}

#[test]
fn a_slot_named_past_its_table_or_from_no_table_fails() {
    let native = |result: Result<bool, sims_vm::Fault>| {
        assert_eq!(result.map_err(|f| f.trap.code()), Err(Some(0x25)));
    };
    // Global slot 2 does not exist; nor does a slot from parameter 0's
    // value, 5; nor table 3.
    native(go_to(2, 2, &[]).0);
    native(go_to(0, 0, &[5]).0);
    native(go_to(0, 3, &[]).0);
    // A parameter past the frame's four reads memory the original does not
    // bound; that fails loudly.
    let fault = go_to(4, 0, &[]).0.unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("parameter 4 of 4")),
        "{fault}"
    );
}

/// The lamp's SLOT chunk: a routing slot that snaps into container slot
/// `into`, then two container slots at its centre, at height 1.
fn into_lamp(fx: &mut Fixture, into: i32) {
    let container = Slot { kind: 0, ..slot(0) };
    set_slots(
        fx,
        vec![
            Slot {
                snap_target: into,
                ..slot(SKIP_CHECKS)
            },
            container.clone(),
            container,
        ],
    );
    fx.lamp_mut().position = [56, 24, 1];
}

/// The snapper, able to go into the lamp: its data word 4 has the bit for
/// height 1, and its weight (data word 27) is the lamp's capacity (data
/// word 28).
fn container_snapper() -> (Fixture, u16) {
    let (mut fx, me) = snapper(SKIP_CHECKS);
    into_lamp(&mut fx, 1);
    let person = fx.world.objects.get_mut(&me).unwrap();
    person.data[4] = 1;
    person.data[27] = 3;
    fx.lamp_mut().data[28] = 3;
    // The fixture's lamp holds objects in its script slots; empty them.
    for held in &mut fx.lamp_mut().slots[1..] {
        held.object = 0;
    }
    (fx, me)
}

#[test]
fn snap_into_a_container_slot_puts_me_in_the_object() {
    let (mut fx, me) = container_snapper();
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    // I stand at the lamp's position, in its script slot 1 (saved slot 2),
    // facing my own way.
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [56, 24, 1]);
    assert_eq!(person.data[1], 3);
    let held: Vec<i16> = fx.world.objects[&LAMP].slots[1..]
        .iter()
        .map(|s| s.object)
        .collect();
    assert_eq!(held, [0, me as i16]);
}

#[test]
fn snap_into_a_container_slot_needs_the_slot_and_a_place_in_it() {
    // One past the lamp's last container slot is native error 0x20.
    let (mut fx, me) = container_snapper();
    into_lamp(&mut fx, 2);
    let start = unmoved(&fx, me);
    assert_eq!(code(snap(&mut fx, me, LAMP as i16, 0, 3, 0)), Some(0x20));
    assert_eq!(unmoved(&fx, me), start);
    // The place test refuses: the slot holds another object, the lamp
    // takes less than I weigh, or my data word 4 lacks the bit for the
    // slot's height. I do not fit, and nothing moves.
    let refusals: [fn(&mut Fixture, u16); 3] = [
        |fx, _| fx.lamp_mut().slots[2].object = LAMP as i16,
        |fx, _| fx.lamp_mut().data[28] = 2,
        |fx, me| fx.world.objects.get_mut(&me).unwrap().data[4] = 2,
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let (mut fx, me) = container_snapper();
        refuse(&mut fx, me);
        unfit(&mut fx, me);
        let start = unmoved(&fx, me);
        let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
        assert_eq!(result, Ok(false), "case {case}");
        assert_eq!(unmoved(&fx, me), start, "case {case}");
    }
}

#[test]
fn flag_bit_2_keeps_me_out_of_a_slot_where_a_wall_crosses_my_footprint() {
    // 0x45522c: my footprint at the lamp (a person 6 wide, so 45 to 51 in
    // the first coordinate at 48), facing the direction, crossed by a
    // wall on the lamp's level (0x476dbc): segment 1 of tile (3, 1), the
    // line 48 in the first coordinate, whose style (0) a person does not
    // cross.
    let crossed = |segments: u8, style: u16, flags: u8, facing: i16| {
        let (mut fx, me) = container_snapper();
        fx.lamp_mut().position = [48, 24, 1];
        let walls = fx.world.arrays.get_mut(&2).unwrap();
        let cell = (3 * 64 + 1) * 14;
        walls.data[cell] |= segments;
        walls.data[cell + 2..cell + 4].copy_from_slice(&style.to_le_bytes());
        let person = fx.world.objects.get_mut(&me).unwrap();
        person.data[1] = facing;
        person.data[54] = 9;
        let result = snap(&mut fx, me, LAMP as i16, 0, 3, flags);
        (result, fx, me)
    };
    // My facing, 11, is the direction 3.
    let (result, fx, me) = crossed(1, 0, 4, 11);
    assert_eq!(result, Ok(false));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [40, 40, 1]);
    // My facing is put back reduced to 0 to 7 (0x45527f), and words 52 and
    // 54 are 0: no placement error and no object collided with.
    assert_eq!(
        (person.data[1], person.data[52], person.data[54]),
        (3, 0, 0)
    );
    assert!(fx.lamp().slots[1..].iter().all(|s| s.object == 0));
    // Without flag bit 2, a style a person crosses (3), or a segment that
    // is not tested (4): I go into the slot.
    for (segments, style, flags) in [(1, 0, 0), (1, 3, 4), (4, 0, 4)] {
        let (result, fx, me) = crossed(segments, style, flags, 3);
        assert_eq!(result, Ok(true), "{segments} {style} {flags}");
        assert_eq!(fx.world.objects[&me].position, [48, 24, 1]);
        assert_eq!(fx.world.objects[&me].data[1], 3);
    }
}

/// The container snapper, with the lamp's container slot 1 (the one it
/// snaps into) at `offset`.
fn offset_container(offset: [f32; 3]) -> (Fixture, u16) {
    let (mut fx, me) = container_snapper();
    let container = Slot { kind: 0, ..slot(0) };
    set_slots(
        &mut fx,
        vec![
            Slot {
                snap_target: 1,
                ..slot(SKIP_CHECKS)
            },
            container.clone(),
            Slot {
                offset,
                ..container
            },
        ],
    );
    (fx, me)
}

#[test]
fn a_container_slot_offset_across_snaps_into_slot_and_guards_stale_offset_mutations() {
    // 0x454ceb..0x454d5e applies the horizontal container-slot offset only to
    // the person's animator offset (`+0x8c/+0x90`, read by `cTSPerson::Place`
    // at `0x5921f0`); the object's lot position and `animator.position` stay at
    // the container's position.
    for offset in [[8.0, 0.0, 0.0], [0.0, -0.5, 0.0], [-0.0, -0.0, 0.0]] {
        let (mut fx, me) = offset_container(offset);
        let lamp_pos = fx.lamp().position;
        assert_eq!(
            snap(&mut fx, me, LAMP as i16, 0, 3, 0),
            Ok(true),
            "{offset:?}"
        );
        assert_eq!(fx.world.objects[&me].position, lamp_pos, "{offset:?}");
        assert_eq!(fx.world.objects[&me].data[2], LAMP as i16, "{offset:?}");
        assert_eq!(fx.world.objects[&me].data[3], 1, "{offset:?}");
    }

    // Rotating the container while it holds a person in a horizontal-offset
    // slot would make the derived offset diverge from the value 0x58933b stored
    // at Snap time, so writing container.data[1] (facing) fails loudly instead
    // of silently altering the model position.
    let (mut fx, me) = offset_container([8.0, 0.0, 0.0]);
    fx.lamp_mut().data[1] = 0;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    let rotate_err = trap(fx.expression(scope::MY_DATA, 1, op::ASSIGN, scope::LITERAL, 2));
    assert!(
        matches!(rotate_err, Trap::Unsupported(ref m) if m.contains("0x454ceb")),
        "expected Unsupported when rotating container holding offset person, got {rotate_err}"
    );
    // Writing the same facing (0) is a no-op and succeeds.
    assert_eq!(
        fx.expression(scope::MY_DATA, 1, op::ASSIGN, scope::LITERAL, 0),
        Ok(true)
    );
    // Snapping the person back out to a non-container routing slot clears
    // containment and zeroes the animator offset, after which the container
    // can rotate freely.
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .get_mut(&SLOTS)
        .unwrap()
        .push(slot(SKIP_CHECKS));
    assert_eq!(snap(&mut fx, me, LAMP as i16, 1, 3, 0), Ok(true));
    assert_eq!(fx.world.objects[&me].data[2], 0);
    assert_eq!(
        fx.expression(scope::MY_DATA, 1, op::ASSIGN, scope::LITERAL, 2),
        Ok(true)
    );

    // Find Location For mode 2 (`turn_one` at 0x433fa1) turning a container
    // holding a person in a horizontal-offset slot fails before mutating facing.
    let (mut fx, me) = offset_container([8.0, 0.0, 0.0]);
    fx.lamp_mut().data[1] = 0;
    fx.lamp_mut().data[23] = 2; // TURN_STEP
    fx.world.objects.get_mut(&SIM).unwrap().data[1] = 2;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    fx.add(
        SIM_FILE,
        4201,
        tree(vec![ins(
            16,
            RETURN_TRUE,
            RETURN_FALSE,
            [2, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    let turn_err = trap(fx.run_with(SIM, 4201, LAMP as i16, &[], 1000));
    assert!(
        matches!(turn_err, Trap::Unsupported(ref m) if m.contains("0x433fa1")),
        "expected Unsupported when Find Location For mode 2 turns container holding offset person, \
         got {turn_err}"
    );
    assert_eq!(fx.lamp().data[1], 0);

    // Find Location For mode 1 (`take_self_off`, 0x4332ed) during the Sim's own
    // tick fails before unlinking the person from a horizontal-offset slot.
    let (mut fx, me) = offset_container([8.0, 0.0, 0.0]);
    fx.lamp_mut().data[1] = 0;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    let widget_file = fx.catalog.objects[fx.catalog.by_guid[&fx.world.objects[&me].guid]].resource;
    fx.add(
        widget_file,
        4202,
        tree(vec![
            assign(
                scope::STACK_OBJECT,
                0,
                scope::MY_DATA,
                11, // object ID
                1,
            ),
            ins(16, RETURN_TRUE, RETURN_FALSE, [1, 0, 0, 0, 0, 0, 0, 0]),
        ]),
    );
    fx.thread(me, 4202, &[]);
    let rates = || sims_sim::motives::Rates::from_lookup(0, |_| None);
    let curve = |motive| sims_sim::motives::Curve::parse(motive, "(-100;1)(100;1)", 8);
    let tuning = sims_sim::motives::Tuning {
        rates: [rates(), rates(), rates()],
        adult_mood: vec![curve(7)],
        child_mood: vec![curve(7)],
    };
    let self_off_err = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream)
        .tick_person(&mut fx.world, me, 0, &tuning)
        .unwrap_err();
    assert!(
        matches!(self_off_err.trap, Trap::Unsupported(ref m) if m.contains("0x4332ed")),
        "expected Unsupported when contained offset person takes itself off, got {self_off_err}"
    );
    assert_eq!(fx.world.objects[&me].data[2], LAMP as i16);

    // A sub-unit offset like [0.5, 0.0, 0.0] truncates to (0, 0) via __ftol at
    // every facing (0x454d4c), so rotating the container does not shift the
    // computed animator offset and succeeds.
    let (mut fx, me) = offset_container([0.5, 0.0, 0.0]);
    fx.lamp_mut().data[1] = 0;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
    assert_eq!(
        fx.expression(scope::MY_DATA, 1, op::ASSIGN, scope::LITERAL, 2),
        Ok(true)
    );
    assert_eq!(fx.lamp().data[1], 2);
}

#[test]
fn a_container_slot_at_height_0_raised_off_the_floor_is_height_5() {
    // Height 0 with a nonzero offset up is height 5: data word 4 needs
    // bit 4 (16), not bit 0.
    let (mut fx, me) = offset_container([0.0, 0.0, 2.0]);
    unfit(&mut fx, me);
    let start = unmoved(&fx, me);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert_eq!(result, Ok(false));
    assert_eq!(unmoved(&fx, me), start);
    let (mut fx, me) = offset_container([0.0, 0.0, 2.0]);
    fx.world.objects.get_mut(&me).unwrap().data[4] = 16;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
}

#[test]
fn nothing_goes_into_an_object_it_holds() {
    // The lamp is in my slot (its data word 26 names me), so I cannot go
    // into it.
    let (mut fx, me) = container_snapper();
    fx.lamp_mut().data[26] = me as i16;
    fx.lamp_mut().data[14] = 1;
    unfit(&mut fx, me);
    let start = unmoved(&fx, me);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert_eq!(result, Ok(false));
    assert_eq!(unmoved(&fx, me), start);
}

/// A widget person made where the lamp stands at `at`.
fn person_at(fx: &mut Fixture, at: [i32; 2]) -> u16 {
    fx.lamp_mut().position = [at[0], at[1], 1];
    let before: Vec<u16> = fx.world.objects.keys().copied().collect();
    assert_eq!(create(fx, 0, 0, SIM as i16), Ok(true));
    let id = *fx
        .world
        .objects
        .keys()
        .find(|id| !before.contains(id))
        .unwrap();
    assert_eq!(fx.world.objects[&id].position, [at[0], at[1], 1]);
    id
}

/// The snapper (on tile (2, 2)) made the lead of a group whose second
/// tile, one tile on in the second coordinate, stands on tile (2, 3); a
/// person in the way on each tile of `blocked`; the lamp, whose slot
/// skips the placement's checks, on tile (3, 2). The group would then
/// stand on tiles (3, 2) and (3, 3).
fn group_snapper(blocked: &[[i32; 2]]) -> (Fixture, u16, Vec<u16>) {
    use sims_formats::saved_objects::Multipart;
    let (mut fx, me) = snapper(SKIP_CHECKS);
    let tile = person_at(&mut fx, [40, 56]);
    let blockers = blocked.iter().map(|&at| person_at(&mut fx, at)).collect();
    for (id, offset, parent) in [(me, [0, 0, 0], 0), (tile, [0, 1, 0], me as i16)] {
        fx.world.objects.get_mut(&id).unwrap().multipart = Some(Multipart {
            offset,
            group: offset,
            parent,
        });
    }
    fx.world.next_parts.insert(me, tile);
    fx.lamp_mut().position = [56, 40, 1];
    (fx, me, blockers)
}

#[test]
fn a_group_that_does_not_fit_names_the_last_object_its_tiles_collided_with() {
    // The group's test goes on through every tile after one refuses
    // (0x422c08), and each collision writes the object collided with over
    // the last (0x433b42): word 54 names the second tile's, though the
    // code kept is the first tile's (0x422b9e).
    let (mut fx, me, blockers) = group_snapper(&[[56, 40], [56, 56]]);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(false));
    let data = &fx.world.objects[&me].data;
    assert_eq!((data[52], data[54]), (0, blockers[1] as i16));
    assert_eq!(fx.world.objects[&me].position, [40, 40, 1]);
    // With only the first tile blocked, it names that one.
    let (mut fx, me, blockers) = group_snapper(&[[56, 40]]);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(false));
    assert_eq!(fx.world.objects[&me].data[54], blockers[0] as i16);
    // The first tile refused for another reason (data word 4 without bit
    // 0, code 0x18), the second by a collision: the group keeps code 0x18,
    // and word 54 still names the object the second tile collided with.
    let (mut fx, me, blockers) = group_snapper(&[[56, 56]]);
    fx.world.objects.get_mut(&me).unwrap().data[4] &= !1;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(false));
    let data = &fx.world.objects[&me].data;
    assert_eq!((data[52], data[54]), (0, blockers[0] as i16));
}

/// Runs Snap in mode 0 as `me` on the lamp, naming local `local`, after
/// setting local 2 to `value`.
/// Snaps in mode 0 with word 0 `parameter`, from a tree with three
/// parameters and four locals: the parameters are `args`, and every local
/// is set to 1, past the lamp's one slot.
fn snap_by_parameter(
    fx: &mut Fixture,
    me: u16,
    parameter: i16,
    args: &[i16],
) -> Result<bool, sims_vm::Fault> {
    let [p0, p1] = parameter.to_le_bytes();
    let guid = fx.world.objects[&me].guid;
    let file = fx.catalog.object(guid).unwrap().resource;
    fx.add(
        file,
        TREE,
        shaped(
            0x8002,
            0,
            3,
            4,
            vec![
                assign(scope::LOCALS, 0, scope::LITERAL, 1, 1),
                assign(scope::LOCALS, 1, scope::LITERAL, 1, 2),
                assign(scope::LOCALS, 2, scope::LITERAL, 1, 3),
                assign(scope::LOCALS, 3, scope::LITERAL, 1, 4),
                ins(SNAP, RETURN_TRUE, RETURN_FALSE, [p0, p1, 0, 0, 0, 0, 0, 0]),
            ],
        ),
    );
    fx.run_with(me, TREE, LAMP as i16, args, 100)
}

#[test]
fn mode_0_reads_the_slots_number_from_a_parameter() {
    // 0x454a4b: word 0 names a parameter (0x47518b is the parameters'
    // start, and frame byte +7, the count it is checked against, is the
    // count of parameters); the parameter holds the number of the stack
    // object's routing slot, which is then taken as mode 3 takes it.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [72, 24, 1];
    let start = unmoved(&fx, me);
    // Parameter 2 holds 1: past the lamp's one slot.
    assert_eq!(
        code(snap_by_parameter(&mut fx, me, 2, &[0, 0, 1])),
        Some(0x1b)
    );
    // A parameter the tree does not have (it has three, and four locals)
    // is native error 8.
    assert_eq!(code(snap_by_parameter(&mut fx, me, 3, &[0, 0, 0])), Some(8));
    assert_eq!(
        code(snap_by_parameter(&mut fx, me, -1, &[0, 0, 0])),
        Some(8)
    );
    assert_eq!(unmoved(&fx, me), start);
    // Parameter 2 holds 0: the lamp's slot 0, though word 0 is 2 and
    // local 2 holds 1.
    assert_eq!(snap_by_parameter(&mut fx, me, 2, &[1, 1, 0]), Ok(true));
    assert_eq!(fx.world.objects[&me].position, [72, 24, 1]);
}

#[test]
fn mode_2_puts_me_one_tile_in_front_of_the_object_after_the_checks() {
    // 0x4549d1: the object's position and level plus one tile's step in
    // its facing (0x42228a), facing my own way, with the place's checks
    // whatever the slot's flags; word 0 names no slot.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 40, 1];
    fx.lamp_mut().data[1] = 4;
    assert_eq!(snap(&mut fx, me, LAMP as i16, 9, 2, 0), Ok(true));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [56, 56, 1]);
    assert_eq!(person.data[1], 3);
    // Facing 0, the tile in front is (3, 1), outside my room: I do not
    // fit, though the lamp's one slot has flag 0x800.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [56, 40, 1];
    fx.lamp_mut().data[1] = 0;
    unfit(&mut fx, me);
    let start = unmoved(&fx, me);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 2, 0), Ok(false));
    assert_eq!(unmoved(&fx, me), start);
}

#[test]
fn mode_1_puts_me_into_the_objects_first_container_slot() {
    // 0x454a1d: the object's container slot 0, as a slot whose snap
    // target is 0 would, with the place's checks; word 0 names no slot.
    let (mut fx, me) = container_snapper();
    fx.lamp_mut().position = [56, 56, 1];
    assert_eq!(snap(&mut fx, me, LAMP as i16, 9, 1, 0), Ok(true));
    let person = &fx.world.objects[&me];
    assert_eq!(person.position, [56, 56, 1]);
    assert_eq!(person.data[1], 3);
    let held: Vec<i16> = fx.world.objects[&LAMP].slots[1..]
        .iter()
        .map(|s| s.object)
        .collect();
    assert_eq!(held, [me as i16, 0]);
    // The checks: the lamp outside my room, and I do not fit.
    let (mut fx, me) = container_snapper();
    unfit(&mut fx, me);
    let start = unmoved(&fx, me);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 1, 0), Ok(false));
    assert_eq!(unmoved(&fx, me), start);
    // Flag bit 2 tests the walls around the object (0x45522c): with none
    // there, I go in.
    let (mut fx, me) = container_snapper();
    fx.lamp_mut().position = [56, 56, 1];
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 1, 4), Ok(true));
    assert_eq!(fx.world.objects[&me].position, [56, 56, 1]);
    // An object with no container slot: mode 1 does not test for one
    // (0x454b19 is not on its way), and what follows is not ported.
    let (mut fx, me) = container_snapper();
    fx.lamp_mut().position = [56, 56, 1];
    fx.lamp_mut().slots.truncate(1);
    let result = snap(&mut fx, me, LAMP as i16, 0, 1, 0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("mode 1")));
}
