//! Reach (opcode 47, 0x45c9ba): a person plays a reach clip picked by the
//! height of the slot it reaches into, and at the clip's event 0 takes
//! the object or puts what it holds there. Synthetic; no game data.
//!
//! The lot is Grab's: the lamp on tile (2, 2) with two container slots at
//! its centre, the Sim on the lamp's tile with its six slots empty, and a
//! widget the lamp has taken into its slot 0.

use crate::create::container;
use crate::find_location::widget;
use crate::support::*;
use sims_content::{Clip, Profile};
use sims_vm::{Status, Trap};

const REACH: u16 = 47;
const GRAB: u16 = 4;
/// The simulator's mode (global 19): 0 is live mode.
const MODE: usize = 19;
/// The lamp's SLOT chunk (the container fixture's).
const LAMP_SLOTS: u16 = 300;
const SKILL: &str = "test-skill";
/// The global file's table of an adult's reach clips, as the fixture
/// names them (the Sim is 174 by person word 58).
const REACHES: u16 = 154;
const NAMES: [&str; 11] = [
    "",
    "floor",
    "floor-sitting",
    "seat",
    "seat-sitting",
    "table",
    "table-sitting",
    "high",
    "high-sitting",
    "under",
    "under-sitting",
];

/// Operands: the mode, whether to move the object, and the parameter
/// that holds the slot.
fn reach(mode: i16, moves: i16, parameter: i16) -> [u8; 8] {
    let [a, b] = mode.to_le_bytes();
    let [c, d] = moves.to_le_bytes();
    let [e, f] = parameter.to_le_bytes();
    [a, b, c, d, e, f, 0, 0]
}

/// The lot, with the widget in the lamp's slot 0 and the Sim about to
/// run one Reach with `operands`, its stack object the widget (mode 0) or
/// the lamp (mode 1), and parameter 0 holding 0.
fn lot(operands: [u8; 8]) -> (Fixture, u16) {
    let (mut fx, _) = container();
    fx.world.globals[MODE] = 0;
    fx.world.ticks = 10;
    let sim = fx.sim_mut();
    sim.position = [40, 40, 1];
    sim.data[29] = 1;
    sim.slots = vec![empty(); 6];
    let id = widget(&mut fx);
    // The widget may go at height 1 and, carried, at height 6.
    fx.world.objects.get_mut(&id).unwrap().data[4] = 0x21;
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![ins(GRAB, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    assert_eq!(fx.run_with(LAMP, 4097, id as i16, &[], 1000), Ok(true));
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
    fx.program
        .libraries
        .get_mut(&GLOBAL)
        .unwrap()
        .strings
        .insert(REACHES, NAMES.iter().map(|n| n.to_string()).collect());
    for name in &NAMES[1..] {
        fx.program.animations.insert(
            "test.far",
            Clip {
                name: (*name).into(),
                skill: SKILL.into(),
                duration: 0x42c8_0000,
                distance: 0,
                moving: false,
                frames: 4,
                motions: 1,
                tracks: Vec::new(),
                channels: Vec::new(),
                samples: [0, 0],
            },
        );
    }
    fx.program.animations.insert_samples(SKILL);
    fx.world.types.insert(8, SIM_GUID);
    fx.program
        .people
        .profiles
        .insert(SIM_GUID, Profile::default());
    fx.add(
        SIM_FILE,
        4096,
        shaped(
            0x8002,
            0,
            1,
            0,
            vec![ins(REACH, RETURN_TRUE, RETURN_FALSE, operands)],
        ),
    );
    fx.thread(SIM, 4096, &[0]);
    fx.sim_mut().stack[0].stack_object = if operands[0] == 1 {
        LAMP as i16
    } else {
        id as i16
    };
    (fx, id)
}

fn empty() -> sims_formats::saved_objects::Slot {
    sims_formats::saved_objects::Slot {
        unknown: 0,
        object: 0,
    }
}

/// What an object's container slots hold.
fn held(object: &sims_sim::Object) -> Vec<i16> {
    object.slots[1..].iter().map(|s| s.object).collect()
}

fn turn(fx: &mut Fixture) -> Result<Status, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, SIM, 100)
}

/// The step Reach keeps in the Sim's frame.
fn step(fx: &Fixture) -> i32 {
    fx.sim().stack[0].primitive_state
}

/// The clip in the Sim's foreground slot, if it has an animator.
fn playing(fx: &Fixture) -> Option<String> {
    fx.world
        .animators
        .get(&SIM)
        .and_then(|a| a.foreground.as_ref())
        .map(|l| l.clip.clone())
}

fn event(fx: &mut Fixture, event: i32) {
    fx.world.animators.get_mut(&SIM).unwrap().events.push(event);
}

fn end_clip(fx: &mut Fixture) {
    let a = fx.world.animators.get_mut(&SIM).unwrap();
    a.foreground.as_mut().unwrap().ended = true;
}

#[test]
fn reach_takes_the_object_at_the_clips_event_and_ends_with_the_clip() {
    let (mut fx, id) = lot(reach(0, 1, 0));
    // Step 0: the lamp's slot is at height 0, so the clip is number 1.
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 1);
    assert_eq!(playing(&fx).as_deref(), Some("floor"));
    let a = &fx.world.animators[&SIM];
    assert_eq!((a.state, a.pickup, a.selection), (0, 1, -1));
    // Step 1 waits for event 0; other events are dropped.
    event(&mut fx, 3);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 1);
    assert!(fx.world.animators[&SIM].events.is_empty());
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
    // Event 0: the widget goes into the Sim's slot 0.
    event(&mut fx, 0);
    event(&mut fx, 5);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 2);
    assert!(fx.world.animators[&SIM].events.is_empty());
    assert_eq!(held(fx.lamp()), [0, 0]);
    assert_eq!(held(fx.sim()), [id as i16, 0, 0, 0, 0]);
    assert_eq!(fx.world.objects[&id].data[26], SIM as i16);
    // Step 2 waits for the clip to end, then takes the true branch.
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    end_clip(&mut fx);
    event(&mut fx, 7);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
    let a = &fx.world.animators[&SIM];
    assert!(a.foreground.is_none());
    assert!(a.events.is_empty());
    assert_eq!((a.state, a.pickup), (3, 0));
}

#[test]
fn the_moved_objects_ct_grab_tree_runs_on_my_stack() {
    // The widget's "CT - Grab" copies its stack object's ID into my temp
    // 0: it runs on the Sim's stack, with the widget as stack object.
    let (mut fx, id) = lot(reach(0, 1, 0));
    let file = fx.catalog.object(crate::factory::WIDGET).unwrap().resource;
    fx.add(
        file,
        4200,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::STACK_OBJECT,
            0,
            RETURN_TRUE,
        )]),
    );
    let library = fx.program.libraries.get_mut(&file).unwrap();
    library.names.insert(4200, "CT - Grab".to_owned());
    fx.sim_mut().temps[0] = -1;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(fx.sim().temps[0], id as i16);
    assert_eq!(fx.sim().stack.len(), 1);
}

#[test]
fn the_end_of_the_clip_also_lets_the_reach_go_on() {
    let (mut fx, id) = lot(reach(0, 1, 0));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(held(fx.sim())[0], id as i16);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn a_refused_object_ends_the_reach_on_the_false_branch() {
    let (mut fx, id) = lot(reach(0, 1, 0));
    // The widget may not go at height 6, where the Sim carries.
    fx.world.objects.get_mut(&id).unwrap().data[4] = 1;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 3);
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(false)));
}

#[test]
fn word_1_at_0_plays_the_reach_without_moving_anything() {
    let (mut fx, id) = lot(reach(0, 0, 0));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 2);
    assert_eq!(held(fx.lamp()), [id as i16, 0]);
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn reach_puts_what_it_holds_into_the_slot_a_parameter_names() {
    let (mut fx, id) = lot(reach(1, 1, 0));
    // The Sim takes the widget first; the lamp's slot 1 is to take it.
    fx.add(
        SIM_FILE,
        4097,
        tree(vec![ins(GRAB, RETURN_TRUE, RETURN_FALSE, [0; 8])]),
    );
    assert_eq!(fx.run_with(SIM, 4097, id as i16, &[], 1000), Ok(true));
    assert_eq!(held(fx.sim())[0], id as i16);
    fx.sim_mut().stack[0].parameters[0] = 1;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(playing(&fx).as_deref(), Some("floor"));
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 2);
    assert_eq!(held(fx.lamp()), [0, id as i16]);
    assert_eq!(held(fx.sim())[0], 0);
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn putting_with_empty_hands_ends_on_the_false_branch() {
    let (mut fx, _) = lot(reach(1, 1, 0));
    fx.sim_mut().stack[0].parameters[0] = 1;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 3);
    end_clip(&mut fx);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(false)));
}

#[test]
fn the_slots_height_picks_the_clip() {
    // Height codes (0x46def8) and the clip each gives: 2 is 2.5 high, 3
    // and 8 are 4, 4 is 4.05; times 3/(2√6) that is 1.53, 2.45 and 2.48.
    for (height, z, sitting, clip) in [
        (2, 0.0, false, "seat"),
        (3, 0.0, false, "table"),
        (8, 0.0, true, "table-sitting"),
        (4, 0.0, false, "table"),
        (1, 9.0, false, "floor"),
        // Any other code keeps the entry's third offset: 4 is 2.45 high,
        // 6 is 3.67.
        (5, 4.0, false, "table"),
        (6, 6.0, false, "high"),
    ] {
        let (mut fx, _) = lot(reach(0, 1, 0));
        let slots = fx
            .program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .slots
            .get_mut(&LAMP_SLOTS)
            .unwrap();
        slots[0].height = height;
        slots[0].offset[2] = z;
        if sitting {
            fx.sim_mut().person.as_mut().unwrap().data[0] = 1;
        }
        assert_eq!(turn(&mut fx), Ok(Status::Yielded), "height {height}");
        assert_eq!(playing(&fx).as_deref(), Some(clip), "height {height}");
    }
}

#[test]
fn too_high_a_slot_starts_no_clip() {
    // 7 is 4.29 high, above every band: the false branch, or the true
    // one when nothing is to move (0x45ca7f).
    for (moves, branch) in [(1, false), (0, true)] {
        let (mut fx, id) = lot(reach(0, moves, 0));
        let slots = fx
            .program
            .libraries
            .get_mut(&LAMP_FILE)
            .unwrap()
            .slots
            .get_mut(&LAMP_SLOTS)
            .unwrap();
        slots[0].height = 5;
        slots[0].offset[2] = 7.0;
        assert_eq!(turn(&mut fx), Ok(Status::Returned(branch)));
        assert!(playing(&fx).is_none());
        assert_eq!(held(fx.lamp()), [id as i16, 0]);
    }
}

#[test]
fn an_unlisted_or_empty_reach_name_starts_no_clip() {
    // The widget sits at slot height 0, clip 1 ("floor"). With that entry
    // empty, or the table too short, 0x59dcd4 gives no name and the start
    // fails (0x589f45): the false branch, or the true one when nothing is
    // to move.
    for short in [false, true] {
        for (moves, branch) in [(1, false), (0, true)] {
            let (mut fx, id) = lot(reach(0, moves, 0));
            let names = fx
                .program
                .libraries
                .get_mut(&GLOBAL)
                .unwrap()
                .strings
                .get_mut(&REACHES)
                .unwrap();
            if short {
                names.truncate(1);
            } else {
                names[1].clear();
            }
            assert_eq!(turn(&mut fx), Ok(Status::Returned(branch)));
            assert!(playing(&fx).is_none());
            assert_eq!(held(fx.lamp()), [id as i16, 0]);
        }
    }
}

#[test]
fn mode_2_reaches_under_with_nothing_to_move() {
    let (mut fx, _) = lot(reach(2, 1, 0));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(playing(&fx).as_deref(), Some("under"));
    // At the event a mode it does not move for leaves the step at 1.
    event(&mut fx, 0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(step(&fx), 1);
}

#[test]
fn reach_without_a_stack_object_takes_the_false_branch() {
    for mode in [0, 1] {
        let (mut fx, _) = lot(reach(mode, 1, 0));
        fx.sim_mut().stack[0].stack_object = 77;
        assert_eq!(turn(&mut fx), Ok(Status::Returned(false)));
        assert!(playing(&fx).is_none());
    }
}

#[test]
fn reach_by_an_object_that_is_not_a_person_is_native_error_5() {
    let (mut fx, id) = lot(reach(0, 1, 0));
    fx.add(
        LAMP_FILE,
        4098,
        tree(vec![ins(REACH, RETURN_TRUE, RETURN_FALSE, reach(0, 1, 0))]),
    );
    assert_eq!(code(fx.run_with(LAMP, 4098, id as i16, &[], 1000)), Some(5));
}

#[test]
fn reach_naming_no_parameter_is_native_error_8() {
    let (mut fx, _) = lot(reach(1, 1, 3));
    let fault = turn(&mut fx).unwrap_err();
    assert_eq!(fault.trap.code(), Some(8), "{fault:?}");
    assert!(matches!(fault.trap, Trap::Native { .. }));
}
