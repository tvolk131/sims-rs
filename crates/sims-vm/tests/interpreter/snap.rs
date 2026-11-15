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
const TREE: u16 = 4200;
/// The lamp's SLOT chunk.
const SLOTS: u16 = 300;
/// Slot flags: the placement skips its checks, the direction follows the
/// object's facing.
const SKIP_CHECKS: u32 = 0x800;
const TURNED: u32 = 0x1000;

/// A routing slot at the object, with the given flags.
fn slot(flags: u32) -> Slot {
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
fn snapper(flags: u32) -> (Fixture, u16) {
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

fn set_slots(fx: &mut Fixture, slots: Vec<Slot>) {
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
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x4550d8")));
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
    for mode in [0, 1, 2, 4] {
        let result = snap(&mut fx, me, LAMP as i16, 0, mode, 0);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains("mode")),
            "mode {mode}"
        );
    }
    assert_eq!(unmoved(&fx, me), start);
}

#[test]
fn what_snap_does_not_port_fails_loudly() {
    let cases: [(Slot, u8, &str); 5] = [
        (
            Slot {
                snap_target: 0,
                ..slot(SKIP_CHECKS)
            },
            0,
            "0x454b19",
        ),
        (
            Slot {
                offset: [8.0, 0.0, 0.0],
                ..slot(SKIP_CHECKS)
            },
            0,
            "0x455070",
        ),
        (
            Slot {
                offset: [0.0, -2.0, 0.0],
                ..slot(SKIP_CHECKS)
            },
            0,
            "0x455070",
        ),
        (slot(SKIP_CHECKS), 1, "0x454c7a"),
        (slot(0), 0, "0x4550d8"),
    ];
    for (entry, flags, address) in cases {
        let (mut fx, me) = snapper(SKIP_CHECKS);
        fx.lamp_mut().position = [56, 24, 1];
        set_slots(&mut fx, vec![entry]);
        let start = unmoved(&fx, me);
        let result = snap(&mut fx, me, LAMP as i16, 0, 3, flags);
        assert!(
            matches!(trap(result), Trap::Unsupported(ref m) if m.contains(address)),
            "{address}"
        );
        assert_eq!(unmoved(&fx, me), start, "{address}");
    }
    // A position further out than the round trip through world units is
    // known to keep.
    let (mut fx, me) = snapper(SKIP_CHECKS);
    fx.lamp_mut().position = [0x8001, 24, 1];
    let result = snap(&mut fx, me, LAMP as i16, 0, 3, 0);
    assert!(matches!(trap(result), Trap::Unsupported(ref m) if m.contains("0x454fb9")));
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
