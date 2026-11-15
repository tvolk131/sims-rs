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
    // (0x4330b7). The person does not fit, and what the original does
    // then is not ported. A floor on tile (2, 3) is not this tile's.
    fx.world.arrays.get_mut(&101).unwrap().data[(2 * 64 + 3) * 2] = 1;
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454e09")));
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

/// Whether Snap failed loudly because its own checks refused the place.
fn refused(result: Result<bool, sims_vm::Fault>) -> bool {
    matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x4550f9"))
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
    // Tile (3, 1) is in room 0, and tile (0, 2) on the lot's border.
    for place in [[56, 24, 1], [8, 40, 1]] {
        let (mut fx, me) = snapper(0);
        fx.lamp_mut().position = place;
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
    for mode in [0, 1, 2] {
        let result = snap(&mut fx, me, LAMP as i16, 0, mode, 0);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains("mode")),
            "mode {mode}"
        );
    }
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
    // The offset's first coordinate goes along the second of the
    // position, its second along the first, turned by the lamp's facing
    // through a matrix in world units and rounded back half away from
    // zero (0x454b93 to 0x454c54).
    // The lamp stands where I do, in the middle of tile (2, 2).
    let cases: [([f32; 2], i16, [i32; 2]); 7] = [
        ([4.0, 0.0], 0, [40, 44]),
        ([4.0, 0.0], 2, [44, 40]),
        ([4.0, 0.0], 4, [40, 36]),
        ([4.0, 0.0], 6, [36, 40]),
        // 0.75 times the root of a half, 0.53033, on 7.5 in world units:
        // 42.828 back in the lot's.
        ([4.0, 0.0], 1, [43, 43]),
        ([0.0, -2.0], 0, [38, 40]),
        // The facing is the raw data word: 10 turns as 2 does.
        ([4.0, 0.0], 10, [44, 40]),
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
    // Flag bit 2 tests the walls around the object instead (0x45522c).
    let (mut fx, me) = container_snapper();
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 4);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x45522c")));
    // The place test refuses: the slot holds another object, the lamp
    // takes less than I weigh, or my data word 4 lacks the bit for the
    // slot's height. Not fitting fails loudly and moves nothing.
    let refusals: [fn(&mut Fixture, u16); 3] = [
        |fx, _| fx.lamp_mut().slots[2].object = LAMP as i16,
        |fx, _| fx.lamp_mut().data[28] = 2,
        |fx, me| fx.world.objects.get_mut(&me).unwrap().data[4] = 2,
    ];
    for (case, refuse) in refusals.into_iter().enumerate() {
        let (mut fx, me) = container_snapper();
        refuse(&mut fx, me);
        let start = unmoved(&fx, me);
        let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454e09")),
            "case {case}"
        );
        assert_eq!(unmoved(&fx, me), start, "case {case}");
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
fn a_container_slot_offset_across_fails_loudly_but_negative_zero_does_not() {
    // The offset across moves my animator (0x454ceb), which is not kept.
    for offset in [[8.0, 0.0, 0.0], [0.0, -0.5, 0.0]] {
        let (mut fx, me) = offset_container(offset);
        let start = unmoved(&fx, me);
        let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454ceb")),
            "{offset:?}"
        );
        assert_eq!(unmoved(&fx, me), start, "{offset:?}");
    }
    let (mut fx, me) = offset_container([-0.0, -0.0, 0.0]);
    assert_eq!(snap(&mut fx, me, LAMP as i16, 0, 3, 0), Ok(true));
}

#[test]
fn a_container_slot_at_height_0_raised_off_the_floor_is_height_5() {
    // Height 0 with a nonzero offset up is height 5: data word 4 needs
    // bit 4 (16), not bit 0.
    let (mut fx, me) = offset_container([0.0, 0.0, 2.0]);
    let start = unmoved(&fx, me);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454e09")));
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
    let start = unmoved(&fx, me);
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454e09")));
    assert_eq!(unmoved(&fx, me), start);
}
