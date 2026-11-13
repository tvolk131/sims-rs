//! Sleep, turns, and waking a sleeping object.

use crate::support::*;
use sims_vm::{Status, Trap};

const BUDGET: usize = 100;

/// The lamp, known to the scheduler and not locked out, with a thread
/// about to sleep `count` turns.
fn asleep(count: i16) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(LAMP_FILE, 4096, sleeper());
    // Flags 1, countdown -1.
    fx.lamp_mut().registry_word = Some(0x0001_ffff);
    fx.lamp_mut().data[25] = 0;
    fx.thread(LAMP, 4096, &[count]);
    fx
}

fn turn(fx: &mut Fixture, object: u16) -> Status {
    let interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter
        .turn(&mut fx.world, object, BUDGET)
        .unwrap_or_else(|f| panic!("{f}"))
}

#[test]
fn an_idle_object_hands_its_count_to_the_scheduler() {
    let mut fx = asleep(5);
    assert_eq!(turn(&mut fx, LAMP), Status::Yielded);
    let lamp = fx.lamp();
    assert_eq!(lamp.countdown(), Some(4));
    assert_eq!(lamp.flags(), 1, "the flags are kept");
    // The frame stays on the Sleep, with its count untouched.
    assert_eq!(
        (lamp.stack[0].node, &lamp.stack[0].parameters[..]),
        (0, &[5][..])
    );
    // The scheduler counts down without running the object. At zero the
    // object runs again and the Sleep is over.
    fx.lamp_mut().set_countdown(0);
    assert_eq!(turn(&mut fx, LAMP), Status::Returned(true));
    assert_eq!(fx.lamp().countdown(), Some(-1));
    assert!(fx.lamp().stack.is_empty());
}

#[test]
fn a_turn_before_the_countdown_ends_starts_the_count_again() {
    // Nothing the scheduler does runs an object whose countdown is above
    // zero, but if it ran, the Sleep would hand over its full count again.
    let mut fx = asleep(5);
    turn(&mut fx, LAMP);
    fx.lamp_mut().set_countdown(2);
    assert_eq!(turn(&mut fx, LAMP), Status::Yielded);
    assert_eq!(fx.lamp().countdown(), Some(4));
}

/// Runs turns until the thread returns and gives how many yielded.
fn yields(fx: &mut Fixture, object: u16) -> usize {
    (0..20)
        .take_while(|_| turn(fx, object) == Status::Yielded)
        .count()
}

#[test]
fn a_locked_out_object_counts_by_itself() {
    let mut fx = asleep(3);
    fx.lamp_mut().data[25] = 1;
    assert_eq!(turn(&mut fx, LAMP), Status::Yielded);
    assert_eq!(fx.lamp().stack[0].parameters, [2]);
    assert_eq!(
        fx.lamp().countdown(),
        Some(-1),
        "the scheduler is not involved"
    );
    assert_eq!(yields(&mut fx, LAMP), 2);
}

#[test]
fn people_and_objects_the_scheduler_does_not_know_count_by_themselves() {
    // A person.
    let mut fx = Fixture::new();
    fx.add(SIM_FILE, 4096, sleeper());
    fx.sim_mut().registry_word = Some(0x0001_ffff);
    fx.sim_mut().data[25] = 0;
    fx.thread(SIM, 4096, &[3]);
    assert_eq!(yields(&mut fx, SIM), 3);
    assert_eq!(fx.sim().countdown(), Some(-1));
    // An object with no registry word.
    let mut fx = asleep(3);
    fx.lamp_mut().registry_word = None;
    assert_eq!(yields(&mut fx, LAMP), 3);
    assert_eq!(fx.lamp().registry_word, None);
}

#[test]
fn a_count_of_zero_or_less_is_over_at_once() {
    for count in [0, -1, i16::MIN] {
        let mut fx = asleep(count);
        assert_eq!(turn(&mut fx, LAMP), Status::Returned(true));
        assert_eq!(fx.lamp().countdown(), Some(-1));
    }
}

#[test]
fn a_synchronous_tree_cannot_wait() {
    let mut fx = asleep(0);
    fx.lamp_mut().stack.clear();
    // No wait is fine.
    assert_eq!(fx.run_with(LAMP, 4096, 0, &[0], BUDGET), Ok(true));
    let fault = fx.run_with(LAMP, 4096, 0, &[5], BUDGET).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x44f));
    assert_eq!(fx.lamp().countdown(), Some(-1));
}

#[test]
fn the_count_must_be_a_parameter_of_the_tree() {
    for index in [1i16, -1] {
        let mut fx = asleep(5);
        let [low, high] = index.to_le_bytes();
        let operands = [low, high, 0, 0, 0, 0, 0, 0];
        fx.add(
            LAMP_FILE,
            4096,
            shaped(
                0x8002,
                0,
                1,
                0,
                vec![ins(0, RETURN_TRUE, RETURN_FALSE, operands)],
            ),
        );
        let interpreter = fx.interpreter();
        let fault = interpreter.turn(&mut fx.world.clone(), LAMP, BUDGET);
        assert_eq!(fault.unwrap_err().trap.code(), Some(8));
    }
}

#[test]
fn a_turn_is_bounded() {
    let mut fx = Fixture::new();
    // Node 0 branches to itself forever.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![assign(scope::TEMPS, 0, scope::LITERAL, 1, 0)]),
    );
    fx.thread(LAMP, 4096, &[0; 4]);
    let interpreter = fx.interpreter();
    let fault = interpreter
        .turn(&mut fx.world.clone(), LAMP, 50)
        .unwrap_err();
    assert_eq!(fault.trap, Trap::BudgetExhausted { budget: 50 });
}

/// The Sim's tree 4096: read the stack object's data word 25.
fn peek(fx: &mut Fixture) {
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![expression(
            scope::STACK_DATA,
            25,
            op::EQUAL,
            scope::LITERAL,
            0,
        )]),
    );
}

#[test]
fn resolving_data_word_25_wakes_a_sleeping_object() {
    let mut fx = asleep(9);
    turn(&mut fx, LAMP);
    // Six ticks pass: the scheduler has counted 8 down to 2.
    fx.lamp_mut().set_countdown(2);
    peek(&mut fx);
    assert_eq!(fx.run_with(SIM, 4096, LAMP as i16, &[], BUDGET), Ok(true));
    let lamp = fx.lamp();
    // The lamp runs every tick again, and the time it had left is back in
    // the Sleep's count.
    assert_eq!(lamp.countdown(), Some(-2));
    assert_eq!(lamp.flags(), 1);
    assert_eq!(lamp.stack[0].parameters, [2]);
    // Its next turn hands the remaining count back to the scheduler.
    assert_eq!(turn(&mut fx, LAMP), Status::Yielded);
    assert_eq!(fx.lamp().countdown(), Some(1));
}

#[test]
fn waking_leaves_a_thread_that_is_not_in_a_sleep_alone() {
    // A countdown below zero: the count stays as it is.
    let mut fx = asleep(9);
    peek(&mut fx);
    fx.run_with(SIM, 4096, LAMP as i16, &[], BUDGET).unwrap();
    assert_eq!(fx.lamp().countdown(), Some(-2));
    assert_eq!(fx.lamp().stack[0].parameters, [9]);
    // A thread stopped somewhere other than a Sleep.
    let mut fx = asleep(9);
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![assign(
            scope::TEMPS,
            0,
            scope::LITERAL,
            1,
            RETURN_TRUE,
        )]),
    );
    fx.lamp_mut().set_countdown(3);
    peek(&mut fx);
    fx.run_with(SIM, 4096, LAMP as i16, &[], BUDGET).unwrap();
    assert_eq!(fx.lamp().countdown(), Some(-2));
    assert_eq!(fx.lamp().stack[0].parameters, [9]);
    // No thread at all, and no registry word.
    let mut fx = asleep(9);
    fx.lamp_mut().stack.clear();
    fx.lamp_mut().set_countdown(3);
    fx.sim_mut().registry_word = None;
    peek(&mut fx);
    fx.run_with(SIM, 4096, LAMP as i16, &[], BUDGET).unwrap();
    assert_eq!(fx.lamp().countdown(), Some(-2));
    fx.run_with(SIM, 4096, SIM as i16, &[], BUDGET).unwrap();
    assert_eq!(fx.sim().registry_word, None);
}

#[test]
fn a_callback_that_wakes_its_own_object_looks_below_itself() {
    // The lamp sleeps, then a synchronous tree on the lamp itself reads
    // its word 25. The frame that gets the time back is the sleeping one
    // below the callback, not the callback's own.
    let mut fx = asleep(9);
    turn(&mut fx, LAMP);
    fx.lamp_mut().set_countdown(4);
    fx.add(
        LAMP_FILE,
        4097,
        tree(vec![expression(
            scope::MY_DATA,
            25,
            op::EQUAL,
            scope::LITERAL,
            0,
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4097, 0, &[7, 7, 7, 7], BUDGET), Ok(true));
    let lamp = fx.lamp();
    assert_eq!(lamp.countdown(), Some(-2));
    assert_eq!(lamp.stack.len(), 1);
    assert_eq!(lamp.stack[0].parameters, [4]);
}

#[test]
fn a_return_is_the_last_result_too() {
    let mut fx = Fixture::new();
    // The Expression is true but the tree returns false.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            2,
            RETURN_FALSE,
            RETURN_TRUE,
            expr(scope::LITERAL, 1, op::EQUAL, scope::LITERAL, 1),
        )]),
    );
    fx.lamp_mut().last_result = true;
    assert_eq!(fx.run(), Ok(false));
    assert!(!fx.lamp().last_result);
    // A primitive's branch is the last result while the tree goes on.
    fx.add(
        LAMP_FILE,
        4096,
        shaped(
            0x8002,
            0,
            1,
            0,
            vec![
                ins(
                    2,
                    1,
                    1,
                    expr(scope::LITERAL, 1, op::EQUAL, scope::LITERAL, 2),
                ),
                ins(0, RETURN_TRUE, RETURN_TRUE, [0; 8]),
            ],
        ),
    );
    fx.lamp_mut().registry_word = Some(0xffff);
    fx.lamp_mut().data[25] = 0;
    fx.thread(LAMP, 4096, &[3]);
    fx.lamp_mut().last_result = true;
    assert_eq!(turn(&mut fx, LAMP), Status::Yielded);
    assert!(
        !fx.lamp().last_result,
        "a yield leaves the last result alone"
    );
}
