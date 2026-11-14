//! Change Suit/Accessory: the suit and its variant, and accessories put on
//! and taken off by name. Synthetic; no game data.

use crate::support::*;
use sims_core::GameRng;
use sims_formats::saved_objects::Accessory;

const SUIT: usize = 8;
const VARIANT: usize = 21;
const AGE: usize = 58;

/// The Sim runs one Change Suit/Accessory with operands `index`, `scope`
/// and `flags`, with accessory names in table 304 of its own file and of
/// the global file.
fn change(fx: &mut Fixture, index: u8, scope: u8, flags: u8) -> Result<bool, sims_vm::Fault> {
    for (file, names) in [
        (SIM_FILE, ["Suit-Book101", "", "hat"]),
        (GLOBAL, ["cane", "x", "y"]),
    ] {
        fx.program
            .libraries
            .get_mut(&file)
            .unwrap()
            .strings
            .insert(0x130, names.map(String::from).to_vec());
    }
    fx.add(
        SIM_FILE,
        4096,
        tree(vec![ins(
            6,
            RETURN_TRUE,
            RETURN_FALSE,
            [index, scope, flags, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(SIM, 4096, LAMP as i16, &[], 100)
}

fn data(fx: &Fixture) -> &[i16] {
    &fx.sim().person.as_ref().unwrap().data
}

fn worn(fx: &Fixture) -> Vec<(String, String)> {
    let person = fx.sim().person.as_ref().unwrap();
    person
        .accessories
        .iter()
        .map(|a| (a.name.clone(), a.binding.clone()))
        .collect()
}

fn set(fx: &mut Fixture, word: usize, value: i16) {
    fx.sim_mut().person.as_mut().unwrap().data[word] = value;
}

#[test]
fn a_suit_below_8_has_variant_0_and_draws_nothing() {
    let mut fx = Fixture::new();
    let rng = fx.world.lot_rng.state();
    assert!(change(&mut fx, 5, 1, 0).unwrap());
    assert_eq!((data(&fx)[SUIT], data(&fx)[VARIANT]), (5, 0));
    assert_eq!(fx.world.lot_rng.state(), rng);
}

#[test]
fn an_adult_s_new_suit_draws_its_variant_modulo_10() {
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 27);
    set(&mut fx, SUIT, 3);
    let mut expected = GameRng::from_state(fx.world.lot_rng.state());
    let variant = (expected.next_u32() % 10) as i16;
    assert!(change(&mut fx, 9, 1, 0).unwrap());
    assert_eq!((data(&fx)[SUIT], data(&fx)[VARIANT]), (9, variant));
    assert_eq!(fx.world.lot_rng.state(), expected.state());
}

#[test]
fn an_adult_s_same_suit_draws_until_the_variant_differs() {
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 0);
    set(&mut fx, SUIT, 9);
    // The variant the first draw would give, so at least two draws.
    let mut expected = GameRng::from_state(fx.world.lot_rng.state());
    let first = (expected.clone().next_u32() % 10) as i16;
    set(&mut fx, VARIANT, first);
    let variant = loop {
        let drawn = (expected.next_u32() % 10) as i16;
        if drawn != first {
            break drawn;
        }
    };
    assert!(change(&mut fx, 9, 1, 0).unwrap());
    assert_eq!(data(&fx)[VARIANT], variant);
    assert_ne!(variant, first);
    assert_eq!(fx.world.lot_rng.state(), expected.state());
}

#[test]
fn a_child_s_variant_is_0_or_1() {
    // The same suit: the other variant, with no draw.
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 9);
    set(&mut fx, SUIT, 12);
    set(&mut fx, VARIANT, 0);
    let rng = fx.world.lot_rng.state();
    assert!(change(&mut fx, 12, 1, 0).unwrap());
    assert_eq!(data(&fx)[VARIANT], 1);
    assert_eq!(fx.world.lot_rng.state(), rng);
    // A new suit: a draw's low bit.
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 17);
    set(&mut fx, SUIT, 12);
    let mut expected = GameRng::from_state(fx.world.lot_rng.state());
    let variant = (expected.next_u32() & 1) as i16;
    assert!(change(&mut fx, 13, 1, 0).unwrap());
    assert_eq!((data(&fx)[SUIT], data(&fx)[VARIANT]), (13, variant));
    assert_eq!(fx.world.lot_rng.state(), expected.state());
}

#[test]
fn taking_a_suit_off_does_nothing() {
    let mut fx = Fixture::new();
    let before = data(&fx).to_vec();
    assert!(change(&mut fx, 9, 1, 1).unwrap());
    assert_eq!(data(&fx), before);
}

#[test]
fn an_accessory_is_named_by_the_running_code_s_file_or_the_global_file() {
    let mut fx = Fixture::new();
    fx.sim_mut().person.as_mut().unwrap().accessories = vec![Accessory {
        name: "suit-book101".into(),
        binding: "R_HAND".into(),
    }];
    // Its own file's string 0, lowered: the old entry of that name goes,
    // and the new one comes last with no binding.
    assert!(change(&mut fx, 2, 2, 0).unwrap());
    assert!(change(&mut fx, 0, 2, 0).unwrap());
    assert!(change(&mut fx, 0, 0, 0).unwrap());
    assert_eq!(
        worn(&fx),
        [("hat", ""), ("suit-book101", ""), ("cane", "")]
            .map(|(n, b)| (n.to_string(), b.to_string()))
    );
    // Taking it off drops it.
    assert!(change(&mut fx, 0, 2, 1).unwrap());
    assert_eq!(
        worn(&fx),
        [("hat", ""), ("cane", "")].map(|(n, b)| (n.to_string(), b.to_string()))
    );
}

#[test]
fn an_empty_or_missing_name_and_another_table_are_native_errors() {
    let mut fx = Fixture::new();
    assert_eq!(code(change(&mut fx, 1, 2, 0)), Some(0x30));
    assert_eq!(code(change(&mut fx, 7, 2, 0)), Some(0x30));
    assert_eq!(code(change(&mut fx, 0, 3, 0)), Some(0x2f));
    assert!(worn(&fx).is_empty());
}
