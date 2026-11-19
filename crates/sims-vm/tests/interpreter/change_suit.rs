//! Change Suit/Accessory: the suit and its variant, and accessories put on
//! and taken off by name. Synthetic; no game data.

use crate::support::*;
use sims_core::GameRng;
use sims_formats::saved_objects::{Accessory, Frame};
use sims_vm::{Status, Stream};

const SUIT: usize = 8;
const VARIANT: usize = 21;
const AGE: usize = 58;

/// Accessory names in table 304 of the Sim's file, the lamp's and the
/// global file, every one a suit some skeleton file defines.
fn name_accessories(fx: &mut Fixture) {
    for (file, names) in [
        (SIM_FILE, ["Suit-Book101", "", "hat"]),
        (LAMP_FILE, ["lampshade", "", ""]),
        (GLOBAL, ["cane", "x", "y"]),
    ] {
        if let Some(library) = fx.program.libraries.get_mut(&file) {
            library
                .strings
                .insert(0x130, names.map(String::from).to_vec());
        }
    }
    fx.program.people.suits = ["suit-book101", "hat", "lampshade", "cane"]
        .map(String::from)
        .into();
}

/// The Sim runs one Change Suit/Accessory with operands `index`, `scope`
/// and `flags` as its own tree 4096.
fn run_change(fx: &mut Fixture, index: u8, scope: u8, flags: u8) -> Result<bool, sims_vm::Fault> {
    fx.add(SIM_FILE, 4096, change_tree(index, scope, flags));
    fx.run_with(SIM, 4096, LAMP as i16, &[], 100)
}

fn change_tree(index: u8, scope: u8, flags: u8) -> sims_formats::behavior::Behavior {
    tree(vec![ins(
        6,
        RETURN_TRUE,
        RETURN_FALSE,
        [index, scope, flags, 0, 0, 0, 0, 0],
    )])
}

/// `run_change` with the accessory names in place.
fn change(fx: &mut Fixture, index: u8, scope: u8, flags: u8) -> Result<bool, sims_vm::Fault> {
    name_accessories(fx);
    run_change(fx, index, scope, flags)
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

#[test]
fn misses_with_no_decode_errors_append_unnamed_entries() {
    let mut fx = Fixture::new();
    name_accessories(&mut fx);
    assert!(fx.program.people.suit_errors.is_empty());
    assert!(run_change(&mut fx, 2, 2, 0).unwrap());
    assert!(run_change(&mut fx, 0, 0, 0).unwrap());
    fx.program.people.suits.remove("hat");
    assert!(run_change(&mut fx, 2, 2, 0).unwrap());
    assert_eq!(
        worn(&fx),
        [("cane", ""), ("", "")].map(|(n, b)| (n.to_string(), b.to_string()))
    );

    assert!(run_change(&mut fx, 2, 2, 0).unwrap());
    let expected = [("cane", ""), ("", ""), ("", "")].map(|(n, b)| (n.to_string(), b.to_string()));
    assert_eq!(worn(&fx), expected);

    fx.world = sims_sim::World::decode(&fx.world.encode()).unwrap();
    assert_eq!(worn(&fx), expected);
    assert!(run_change(&mut fx, 2, 2, 1).unwrap());
    assert_eq!(worn(&fx), expected);
}

#[test]
fn a_miss_with_decode_errors_traps_before_changing_the_list() {
    let mut fx = Fixture::new();
    name_accessories(&mut fx);
    fx.program.people.suit_errors.extend([
        ("GameData/x.bcf".into(), "truncated".into()),
        ("GameData/y.cmx".into(), "invalid".into()),
    ]);
    assert!(run_change(&mut fx, 2, 2, 0).unwrap());
    let before = worn(&fx);
    fx.program.people.suits.remove("hat");

    let error = trap(run_change(&mut fx, 2, 2, 0));
    let sims_vm::Trap::Unsupported(reason) = error else {
        panic!("{error}");
    };
    assert!(reason.contains("accessory \"hat\""), "{reason}");
    assert!(
        reason.contains("2 avatar files stopped decoding (GameData/x.bcf, GameData/y.cmx)"),
        "{reason}"
    );
    assert_eq!(worn(&fx), before);

    assert!(run_change(&mut fx, 2, 2, 1).unwrap());
    assert!(worn(&fx).is_empty());
}

#[test]
fn scope_2_reads_the_running_code_s_file_not_the_person_s() {
    // The Sim runs the lamp's tree: the lamp's table names the accessory.
    let mut fx = Fixture::new();
    name_accessories(&mut fx);
    fx.add(LAMP_FILE, 4096, change_tree(0, 2, 0));
    fx.sim_mut().stack = vec![Frame {
        stack_object: 0,
        tree: 4096,
        node: 0,
        parameters: vec![0; 4],
        locals: vec![0; 4],
        primitive_state: 0,
        code_owner_type: LAMP_TYPE,
    }];
    let interpreter = fx.interpreter();
    let mut world = fx.world.clone();
    assert_eq!(
        interpreter.step(&mut world, SIM),
        Ok(Status::Returned(true))
    );
    let person = world.objects[&SIM].person.as_ref().unwrap();
    let names: Vec<_> = person.accessories.iter().map(|a| &*a.name).collect();
    assert_eq!(names, ["lampshade"]);
}

#[test]
fn a_missing_table_or_global_file_is_a_native_error() {
    let mut fx = Fixture::new();
    name_accessories(&mut fx);
    fx.program
        .libraries
        .get_mut(&SIM_FILE)
        .unwrap()
        .strings
        .remove(&0x130);
    assert_eq!(code(run_change(&mut fx, 0, 2, 0)), Some(0x30));
    fx.program.libraries.remove(&GLOBAL);
    assert_eq!(code(run_change(&mut fx, 0, 0, 0)), Some(0x2f));
    assert!(worn(&fx).is_empty());
}

#[test]
fn suit_8_is_the_first_with_a_drawn_variant() {
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 27);
    let rng = fx.world.lot_rng.state();
    assert!(change(&mut fx, 7, 1, 0).unwrap());
    assert_eq!(fx.world.lot_rng.state(), rng);
    let mut expected = GameRng::from_state(rng);
    let variant = (expected.next_u32() % 10) as i16;
    assert!(change(&mut fx, 8, 1, 0).unwrap());
    assert_eq!((data(&fx)[SUIT], data(&fx)[VARIANT]), (8, variant));
    assert_eq!(fx.world.lot_rng.state(), expected.state());
}

#[test]
fn age_18_is_an_adult_and_a_negative_age_a_child() {
    for (age, child) in [(18, false), (-1, true), (0, false)] {
        let mut fx = Fixture::new();
        set(&mut fx, AGE, age);
        set(&mut fx, SUIT, 3);
        let mut expected = GameRng::from_state(fx.world.lot_rng.state());
        let draw = expected.next_u32();
        let variant = if child { draw & 1 } else { draw % 10 } as i16;
        assert!(change(&mut fx, 9, 1, 0).unwrap());
        assert_eq!(data(&fx)[VARIANT], variant, "age {age}");
    }
}

#[test]
fn a_child_on_variant_1_of_its_suit_goes_to_0() {
    let mut fx = Fixture::new();
    set(&mut fx, AGE, 5);
    set(&mut fx, SUIT, 12);
    set(&mut fx, VARIANT, 1);
    assert!(change(&mut fx, 12, 1, 0).unwrap());
    assert_eq!(data(&fx)[VARIANT], 0);
}

#[test]
fn the_variant_draws_from_the_active_stream() {
    let mut fx = Fixture::new();
    fx.stream = Stream::Global;
    set(&mut fx, AGE, 30);
    set(&mut fx, SUIT, 3);
    let lot = fx.world.lot_rng.state();
    let mut expected = GameRng::from_state(fx.world.global_rng.state());
    let variant = (expected.next_u32() % 10) as i16;
    assert!(change(&mut fx, 9, 1, 0).unwrap());
    assert_eq!(data(&fx)[VARIANT], variant);
    assert_eq!(fx.world.global_rng.state(), expected.state());
    assert_eq!(fx.world.lot_rng.state(), lot);
}

#[test]
fn accessories_draw_nothing() {
    let mut fx = Fixture::new();
    let rng = fx.world.lot_rng.state();
    assert!(change(&mut fx, 0, 0, 0).unwrap());
    assert!(change(&mut fx, 0, 0, 1).unwrap());
    assert_eq!(fx.world.lot_rng.state(), rng);
}

#[test]
fn only_the_words_a_branch_reads_must_be_known() {
    let unknown = |words: &[usize]| {
        let mut fx = Fixture::new();
        set(&mut fx, AGE, 30);
        set(&mut fx, SUIT, 3);
        for &word in words {
            fx.world.routing.unknown_words.insert((SIM, word));
        }
        fx
    };
    // Suits below 8 read nothing, and the writes make both words known.
    let mut fx = unknown(&[SUIT, VARIANT, AGE]);
    assert!(change(&mut fx, 4, 1, 0).unwrap());
    assert_eq!(
        fx.world.routing.unknown_words,
        [(SIM, AGE)].into_iter().collect()
    );
    // A new suit reads the suit and age, not the variant.
    let mut fx = unknown(&[VARIANT]);
    assert!(change(&mut fx, 9, 1, 0).unwrap());
    assert!(fx.world.routing.unknown_words.is_empty());
    for word in [SUIT, AGE] {
        let mut fx = unknown(&[word]);
        let before = data(&fx).to_vec();
        let error = trap(change(&mut fx, 9, 1, 0)).to_string();
        assert!(error.contains(&format!("word {word}")), "{error}");
        assert_eq!(data(&fx), before);
    }
    // The suit it wears reads the variant too.
    let mut fx = unknown(&[VARIANT]);
    set(&mut fx, SUIT, 9);
    assert!(
        trap(change(&mut fx, 9, 1, 0))
            .to_string()
            .contains("word 21")
    );
}
