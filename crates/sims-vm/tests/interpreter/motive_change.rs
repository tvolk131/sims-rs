//! Set Motive Change: changes added in order, cleared by flag bit 0, and
//! the motives that take one. Synthetic; no game data.

use crate::support::*;
use sims_vm::Trap;

/// Set Motive Change with the amount and the stop as literals.
fn change(motive: u8, flags: u8, amount: i16, stop: i16) -> [u8; 8] {
    let [a0, a1] = amount.to_le_bytes();
    let [s0, s1] = stop.to_le_bytes();
    [
        scope::LITERAL,
        scope::LITERAL,
        motive,
        flags,
        a0,
        a1,
        s0,
        s1,
    ]
}

/// The Sim runs the given Set Motive Change instructions as its tree 4096.
fn run(fx: &mut Fixture, steps: &[[u8; 8]]) -> Result<bool, sims_vm::Fault> {
    let nodes = steps
        .iter()
        .enumerate()
        .map(|(i, &operands)| {
            let next = if i + 1 == steps.len() {
                RETURN_TRUE
            } else {
                i as u8 + 1
            };
            ins(29, next, RETURN_FALSE, operands)
        })
        .collect();
    fx.add(SIM_FILE, 4096, tree(nodes));
    fx.run_with(SIM, 4096, LAMP as i16, &[], 100)
}

fn changes(fx: &Fixture) -> Vec<(i32, u32, f32)> {
    fx.sim()
        .person
        .as_ref()
        .unwrap()
        .motive_deltas
        .iter()
        .map(|c| (c.motive, c.delta.to_bits(), c.stop_at))
        .collect()
}

#[test]
fn changes_are_added_in_order_as_an_amount_per_tick() {
    let mut fx = Fixture::new();
    assert_eq!(
        run(&mut fx, &[change(5, 0, 1800, 90), change(5, 0, -900, -50)]),
        Ok(true)
    );
    // 1800 an hour is one a tick; -900 is half of one, taken away.
    assert_eq!(
        changes(&fx),
        [(5, 1f32.to_bits(), 90.0), (5, (-0.5f32).to_bits(), -50.0)]
    );
}

/// Operands whose amount and stop name a parameter the tree does not
/// have, so reading either fails.
fn unreadable(motive: u8, flags: u8) -> [u8; 8] {
    let mut operands = change(motive, flags, 5, 6);
    operands[0] = scope::PARAMETERS;
    operands[1] = scope::PARAMETERS;
    operands
}

#[test]
fn flag_bit_0_drops_every_change_and_reads_nothing() {
    let mut fx = Fixture::new();
    run(&mut fx, &[change(7, 0, 100, 10)]).unwrap();
    // A motive that takes no change and operands that cannot be read,
    // with flag bit 0: still a clear.
    assert_eq!(run(&mut fx, &[unreadable(0, 1)]), Ok(true));
    assert!(changes(&fx).is_empty());
}

#[test]
fn the_motive_is_checked_before_the_operands_are_read() {
    let mut fx = Fixture::new();
    let fault = run(&mut fx, &[unreadable(0, 0)]).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x1a), "{fault}");
}

#[test]
fn a_stop_that_cannot_be_read_adds_nothing_and_keeps_the_rest() {
    let mut fx = Fixture::new();
    run(&mut fx, &[change(7, 0, 1800, 10)]).unwrap();
    let mut operands = change(5, 0, 100, 6);
    operands[1] = scope::PARAMETERS;
    assert!(run(&mut fx, &[operands]).is_err());
    assert_eq!(changes(&fx), [(7, 1f32.to_bits(), 10.0)]);
}

#[test]
fn only_some_motives_take_a_change() {
    for motive in [0, 4, 10, 13, 16] {
        let mut fx = Fixture::new();
        let fault = run(&mut fx, &[change(motive, 0, 100, 10)]).unwrap_err();
        assert_eq!(fault.trap.code(), Some(0x1a), "motive {motive}: {fault}");
        assert!(changes(&fx).is_empty());
    }
    for motive in [9, 14, 15] {
        let mut fx = Fixture::new();
        assert_eq!(run(&mut fx, &[change(motive, 0, 100, 10)]), Ok(true));
    }
}

#[test]
fn an_object_that_is_not_a_person_fails_loudly() {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(29, RETURN_TRUE, RETURN_FALSE, change(5, 0, 1, 1))]),
    );
    let fault = fx.run_with(LAMP, 4096, SIM as i16, &[], 100).unwrap_err();
    assert!(matches!(fault.trap, Trap::Unsupported(_)), "{fault}");
}
