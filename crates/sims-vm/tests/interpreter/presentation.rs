//! Refresh and Play Sound Event, whose effects are mostly presentation.
//! The lamp runs; its stack object is the Sim.

use crate::support::*;
use sims_vm::{Fault, Interpreter, SoundEvent, Trap};
use std::cell::RefCell;

fn refresh(fx: &mut Fixture, selector: i16, mode: i16, stack_object: i16) -> Result<bool, Fault> {
    let mut operands = [0; 8];
    operands[..2].copy_from_slice(&selector.to_le_bytes());
    operands[2..4].copy_from_slice(&mode.to_le_bytes());
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(7, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(LAMP, 4096, stack_object, &[], 10)
}

/// The lot's hash once the lamp has run a tree that returned true: the
/// only thing such a run leaves behind is the lamp's last result.
fn hash_after_success(fx: &Fixture) -> [u8; 32] {
    let mut world = fx.world.clone();
    world.objects.get_mut(&LAMP).unwrap().last_result = true;
    world.hash()
}

#[test]
fn refreshing_a_drawing_changes_nothing() {
    let mut fx = Fixture::new();
    let before = hash_after_success(&fx);
    assert_eq!(refresh(&mut fx, 0, 0, SIM as i16), Ok(true));
    assert_eq!(refresh(&mut fx, 1, 0, SIM as i16), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), before);
}

#[test]
fn an_object_in_no_room_has_no_room_to_mark() {
    let mut fx = Fixture::new();
    fx.lamp_mut().data[29] = -5;
    fx.sim_mut().data[29] = -5;
    let before = hash_after_success(&fx);
    for selector in [0, 1] {
        for mode in [1, 2] {
            assert_eq!(refresh(&mut fx, selector, mode, SIM as i16), Ok(true));
        }
    }
    assert_eq!(hash_without_frames(&fx.world), before);
}

#[test]
fn modes_1_and_2_mark_the_objects_room_as_not_counted() {
    for mode in [1, 2] {
        let mut fx = Fixture::new();
        let room = |dirty| sims_sim::rooms::Room {
            tiles: Vec::new(),
            outdoors: false,
            dirty,
            half_tiles: 8,
            covered_half_tiles: 0,
            walls: 0,
            covered_walls: 0,
            ..Default::default()
        };
        fx.world.rooms.rooms.insert(1025, room(false));
        fx.world.rooms.rooms.insert(3, room(false));
        fx.sim_mut().data[29] = 1025;
        assert_eq!(refresh(&mut fx, 1, mode, SIM as i16), Ok(true));
        assert!(fx.world.rooms.rooms[&1025].dirty);
        // The room's level now has a room waiting: the upper level's bit.
        assert_eq!(fx.world.rooms.pending, 2);
        // The count itself stays until the next room update, and no other
        // room is touched.
        assert_eq!(fx.world.rooms.rooms[&1025].half_tiles, 8);
        assert!(!fx.world.rooms.rooms[&3].dirty);
        // A room word that names no room does nothing.
        fx.sim_mut().data[29] = 77;
        assert_eq!(refresh(&mut fx, 1, mode, SIM as i16), Ok(true));
    }
}

#[test]
fn refresh_errors() {
    let mut fx = Fixture::new();
    // An invalid selector, a missing stack object, an invalid mode.
    assert_eq!(code(refresh(&mut fx, 2, 0, SIM as i16)), Some(3));
    assert_eq!(code(refresh(&mut fx, -1, 0, SIM as i16)), Some(3));
    assert_eq!(code(refresh(&mut fx, 1, 0, 40)), Some(0x15));
    assert_eq!(code(refresh(&mut fx, 1, 0, -2)), Some(0x15));
    assert_eq!(code(refresh(&mut fx, 0, 3, SIM as i16)), Some(3));
    assert_eq!(code(refresh(&mut fx, 0, -1, SIM as i16)), Some(3));
    // The object is chosen before the mode is checked.
    assert_eq!(code(refresh(&mut fx, 1, 3, 40)), Some(0x15));
    // Mode 0 never looks at the room word.
    fx.lamp_mut().data.truncate(20);
    assert_eq!(refresh(&mut fx, 0, 0, SIM as i16), Ok(true));
    assert!(matches!(
        trap(refresh(&mut fx, 0, 2, SIM as i16)),
        Trap::Invalid(_)
    ));
}

#[test]
fn play_sound_takes_the_true_branch_and_changes_nothing() {
    let mut fx = Fixture::new();
    let before = hash_after_success(&fx);
    for operands in [[0; 8], [5, 0, 0, 1, 3, 0, 0, 0], [0xff; 8]] {
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(23, RETURN_TRUE, RETURN_FALSE, operands)]),
        );
        assert_eq!(fx.run(), Ok(true));
    }
    assert_eq!(hash_without_frames(&fx.world), before);
}

/// Runs the lamp's tree 4096, one instruction of `opcode` with
/// `operands`, with the Sim as stack object, and returns the sound events
/// it made.
fn sounds(fx: &mut Fixture, opcode: u16, operands: [u8; 8]) -> Vec<SoundEvent> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(opcode, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    let sink = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.sounds = Some(&sink);
    assert_eq!(
        interpreter.run_tree(&mut fx.world, LAMP, 4096, SIM as i16, &[], 10),
        Ok(true)
    );
    sink.into_inner()
}

fn play(object: u16, name: &str) -> SoundEvent {
    SoundEvent::Play {
        object: object as i16,
        name: name.into(),
    }
}

#[test]
fn play_sound_plays_the_event_its_fwav_names_in_the_codes_file_then_the_global_one() {
    let mut fx = Fixture::new();
    let before = hash_after_success(&fx);
    for (file, id, name) in [
        (LAMP_FILE, 5, "lamp_on"),
        (GLOBAL, 5, "global_five"),
        (GLOBAL, 6, "global_six"),
        // The semi-global file is not searched.
        (SEMI_FILE, 7, "semi_seven"),
    ] {
        let library = fx.program.libraries.entry(file).or_default();
        library.sounds.insert(id, name.into());
    }
    // Me, unless byte 4 has bit 1 and is not 0xff.
    assert_eq!(
        sounds(&mut fx, 23, [5, 0, 0, 0, 0, 0, 0, 0]),
        [play(LAMP, "lamp_on")]
    );
    assert_eq!(
        sounds(&mut fx, 23, [5, 0, 0, 0, 2, 0, 0, 0]),
        [play(SIM, "lamp_on")]
    );
    assert_eq!(
        sounds(&mut fx, 23, [5, 0, 0, 0, 0xff, 0, 0, 0]),
        [play(LAMP, "lamp_on")]
    );
    assert_eq!(
        sounds(&mut fx, 23, [6, 0, 0, 0, 0, 0, 0, 0]),
        [play(LAMP, "global_six")]
    );
    // A name found nowhere plays nothing, and the branch is still true.
    assert_eq!(sounds(&mut fx, 23, [7, 0, 0, 0, 0, 0, 0, 0]), []);
    // The stack object's ID is passed as it is, naming an object or not.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            23,
            RETURN_TRUE,
            RETURN_FALSE,
            [5, 0, 0, 0, 2, 0, 0, 0],
        )]),
    );
    let sink = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.sounds = Some(&sink);
    assert_eq!(
        interpreter.run_tree(&mut fx.world, LAMP, 4096, 999, &[], 10),
        Ok(true)
    );
    assert_eq!(sink.into_inner(), [play(999, "lamp_on")]);
    // Sound changes nothing the lot keeps.
    assert_eq!(hash_without_frames(&fx.world), before);
}

#[test]
fn stop_all_sounds_stops_me_or_the_stack_object() {
    let mut fx = Fixture::new();
    let stop = |object: u16| SoundEvent::Stop {
        object: object as i16,
    };
    assert_eq!(sounds(&mut fx, 48, [0; 8]), [stop(LAMP)]);
    assert_eq!(sounds(&mut fx, 48, [1, 0, 0, 0, 0, 0, 0, 0]), [stop(SIM)]);
}
