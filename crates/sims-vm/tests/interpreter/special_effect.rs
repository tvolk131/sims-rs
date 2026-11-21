//! Special Effect (opcode 35, 0x44aaa6): a machine on the frame's
//! primitive state. Synthetic; no game data.

use crate::support::*;
use sims_vm::Status;

const SPECIAL_EFFECT: u16 = 35;
/// Global 11, which the first step sets.
const EFFECT: usize = 11;

/// The lamp, known to the scheduler, about to run one Special Effect
/// with `flags` on the Sim; word 0 names the duration (a literal 30).
fn effect(flags: u8) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            SPECIAL_EFFECT,
            RETURN_TRUE,
            RETURN_FALSE,
            [30, 0, 0, 0, flags, 0, 0, 0],
        )]),
    );
    fx.lamp_mut().registry_word = Some(0x0001_ffff);
    fx.lamp_mut().data[25] = 0;
    fx.thread(LAMP, 4096, &[]);
    fx.lamp_mut().stack[0].stack_object = SIM as i16;
    fx.world.globals[EFFECT] = 0;
    fx
}

fn turn(fx: &mut Fixture) -> Result<Status, sims_vm::Fault> {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.turn(&mut fx.world, LAMP, 100)
}

/// Global 10: the lot's number. Lot 0 is always residential (0x40745f);
/// lot 9 has no zoning in the fixture.
const LOT: usize = 10;

#[test]
fn on_a_residential_lot_the_effect_is_shown_and_waits_a_turn_more() {
    // The options' flag is on, as in the running game (0x6f262c), and
    // lot 0 is residential: shown without flag 8. The Goth day recording
    // has objects 12 and 334 wait this extra turn at tick 2748.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 0;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(fx.world.globals[EFFECT], 1);
    assert_eq!(fx.lamp().stack[0].primitive_state, 1);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(fx.lamp().stack[0].primitive_state, 2);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn an_effect_not_shown_yields_once_then_takes_the_true_branch() {
    // Off a residential lot, by an object that is not a person: not
    // shown.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 9;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(fx.lamp().stack[0].primitive_state, 1);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn flag_8_shows_the_effect_on_any_lot() {
    let mut fx = effect(8);
    fx.world.globals[LOT] = 9;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(fx.lamp().stack[0].primitive_state, 2);
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
}

#[test]
fn the_second_step_needs_a_stack_object() {
    let mut fx = effect(0);
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    fx.lamp_mut().stack[0].stack_object = 77;
    assert_eq!(turn(&mut fx).unwrap_err().trap.code(), Some(0x17));
}

#[test]
fn a_shown_effect_without_flag_0x40_resets_speed_to_normal() {
    // 0x44abda..0x44abee → 0x4f684c..0x4f6873: message 0x10c sets speed
    // to NORMAL (760) through 0x46d697, updating global 18 when non-negative.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 0;
    fx.world.globals[16] = 1000;
    fx.world.globals[18] = 924;
    // State 0 yields without changing speed.
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (1000, 924));
    // State 1 shows the effect and drops speed to 760.
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (760, 760));
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));

    // Negative return speed stays negative when 0x46d697 runs.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 0;
    fx.world.globals[16] = 994;
    fx.world.globals[18] = -1;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (760, -1));
}

#[test]
fn flag_0x40_or_unshown_or_already_normal_leaves_speed_alone() {
    // Flag 0x40 suppresses message 0x10c.
    let mut fx = effect(0x40);
    fx.world.globals[LOT] = 0;
    fx.world.globals[16] = 1000;
    fx.world.globals[18] = 924;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (1000, 924));

    // Unshown effect does not broadcast 0x10c.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 9;
    fx.world.globals[16] = 1000;
    fx.world.globals[18] = 924;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(turn(&mut fx), Ok(Status::Returned(true)));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (1000, 924));

    // Already at NORMAL (760): 0x4f686c skips 0x46d697, so global 18 stays.
    let mut fx = effect(0);
    fx.world.globals[LOT] = 0;
    fx.world.globals[16] = 760;
    fx.world.globals[18] = 924;
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!(turn(&mut fx), Ok(Status::Yielded));
    assert_eq!((fx.world.speed(), fx.world.return_speed()), (760, 924));
}
