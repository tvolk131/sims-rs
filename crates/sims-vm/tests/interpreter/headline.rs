//! Set Balloon/Headline (opcode 41, 0x44b724): which object it acts on,
//! and that only an object with a headline is touched. Synthetic; no game
//! data.

use crate::support::*;
use sims_formats::execution::Slot;
use sims_vm::Trap;

const HEADLINE: u16 = 41;
/// The lamp's SLOT chunk.
const SLOTS: u16 = 300;

/// A SLOT entry of `kind` (1 makes a headline, 3 a routing slot).
fn slot(kind: u16) -> Slot {
    Slot {
        kind,
        offset: [0.0, 0.0, 4.0],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [0; 3],
        max_size: 100,
        unknown: 0,
        gradient: 0.0,
        height: 0,
        facing: -2,
        resolution: 16,
    }
}

/// Gives the lamp a SLOT with entries of these kinds.
fn lamp_slots(fx: &mut Fixture, kinds: &[u16]) {
    fx.words(LAMP_FILE)[20] = SLOTS as i16;
    fx.program
        .libraries
        .get_mut(&LAMP_FILE)
        .unwrap()
        .slots
        .insert(SLOTS, kinds.iter().map(|&k| slot(k)).collect());
}

/// Set Balloon/Headline: the stack object's when `stack`, an icon of kind
/// 5 and index 8 for `duration` ticks (0 clears).
fn headline(stack: bool, duration: i16) -> [u8; 8] {
    let [d0, d1] = duration.to_le_bytes();
    [u8::from(stack), 0, 8, 5, d0, d1, 3, 0]
}

/// Object `me` runs one Set Balloon/Headline as its tree 4096, with the
/// stack object `stack`.
fn run(fx: &mut Fixture, me: u16, stack: i16, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    let file = if me == SIM { SIM_FILE } else { LAMP_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(HEADLINE, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(me, 4096, stack, &[], 100)
}

fn showing_fails(result: Result<bool, sims_vm::Fault>) {
    let fault = result.unwrap_err();
    assert!(
        matches!(&fault.trap, Trap::Unsupported(m) if m.contains("0x44b7ed")),
        "{fault}"
    );
}

#[test]
fn an_object_with_no_headline_is_left_alone() {
    let mut fx = Fixture::new();
    lamp_slots(&mut fx, &[3, 0]);
    // Showing an icon, which is not ported, is never reached.
    assert_eq!(
        run(&mut fx, LAMP, SIM as i16, headline(false, -1)),
        Ok(true)
    );
}

#[test]
fn a_slot_of_kind_1_gives_an_object_a_headline() {
    let mut fx = Fixture::new();
    lamp_slots(&mut fx, &[3, 0, 1]);
    assert_eq!(run(&mut fx, LAMP, SIM as i16, headline(false, 0)), Ok(true));
    showing_fails(run(&mut fx, LAMP, SIM as i16, headline(false, 30)));
}

#[test]
fn a_person_has_a_headline() {
    let mut fx = Fixture::new();
    assert_eq!(run(&mut fx, SIM, LAMP as i16, headline(false, 0)), Ok(true));
    showing_fails(run(&mut fx, SIM, LAMP as i16, headline(false, -1)));
}

#[test]
fn byte_0_bit_0_acts_on_the_stack_object() {
    let mut fx = Fixture::new();
    // The person has a headline and the lamp none: showing one on the
    // lamp does nothing, on the person fails.
    assert_eq!(run(&mut fx, SIM, LAMP as i16, headline(true, -1)), Ok(true));
    showing_fails(run(&mut fx, LAMP, SIM as i16, headline(true, -1)));
    let fault = run(&mut fx, SIM, 999, headline(true, 0)).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x15), "{fault}");
}
