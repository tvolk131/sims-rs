//! Make New Character: its locals, the person type it clones, and the
//! body strings it writes.

use crate::support::*;
use sims_content::Profile;
use sims_core::CrtRng;
use sims_sim::characters::{self, Character};
use sims_vm::Trap;
use std::collections::BTreeMap;

const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;

/// A fixture whose lamp's tree 4096 sets its locals 0 to 2 to `skin`,
/// `age` and `sex`, then makes a new character from them with template
/// byte `species`. TemplatePerson is registered, with skin tones `tones`.
fn fixture(skin: i16, age: i16, sex: i16, species: u8, tones: &[&str]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, skin, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, age, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, sex, 3),
            ins(
                19,
                RETURN_TRUE,
                RETURN_FALSE,
                [0, 1, 2, species, 0, 0, 0, 0],
            ),
        ]),
    );
    fx.add_type(TEMPLATE_PERSON, 2, 0, 0);
    fx.program.people.profiles.insert(
        TEMPLATE_PERSON,
        Profile {
            tones: tones.iter().map(|t| t.to_string()).collect(),
            ..Profile::default()
        },
    );
    fx.world.ticks = 0x0012_3456;
    fx.world.crt_rng = CrtRng::from_state(77);
    fx
}

const TONES: [&str; 3] = ["lgt", "med", "drk"];

/// The GUID the clone gives with the fixture's clock and C runtime state.
fn expected_guid() -> u32 {
    let mut crt = CrtRng::from_state(77);
    (0x3456 << 16) ^ u32::from(crt.next_u15())
}

fn unsupported(trap: Trap, address: &str) {
    assert!(
        matches!(&trap, Trap::Unsupported(m) if m.contains(address)),
        "{trap:?}"
    );
}

#[test]
fn a_local_past_the_trees_locals_throws_native_error_8() {
    for (skin, age, sex) in [(4, 0, 0), (0, 4, 0), (0, 0, 4), (255, 255, 255)] {
        let mut fx = Fixture::new();
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(
                19,
                RETURN_TRUE,
                RETURN_FALSE,
                [skin, age, sex, 0, 0, 0, 0, 0],
            )]),
        );
        assert_eq!(code(fx.run()), Some(8), "{skin} {age} {sex}");
        assert!(fx.world.characters.made.is_empty());
    }
}

#[test]
fn a_child_gets_a_type_cloned_from_the_template_with_a_record_and_its_body_strings() {
    let mut fx = fixture(1, 10, 1, 0, &TONES);
    let slots = fx.world.neighbors.slots.len();
    // The body and head lists are the next part to port.
    unsupported(trap(fx.run()), "0x409663");
    let guid = expected_guid();
    let made = &fx.world.characters.made;
    assert_eq!(made.keys().copied().collect::<Vec<_>>(), [guid]);
    assert_eq!(
        made[&guid],
        Character {
            template: TEMPLATE_PERSON,
            file: 0,
            strings: BTreeMap::from([
                (characters::SKELETON, "child".into()),
                (characters::GENDER, "female".into()),
                (characters::AGE, "9".into()),
                (characters::SKIN, "med".into()),
            ]),
        }
    );
    assert_eq!(fx.world.characters.next_file, 1);
    // One C runtime draw, for the GUID.
    let mut crt = CrtRng::from_state(77);
    crt.next_u15();
    assert_eq!(fx.world.crt_rng, crt);
    // The record goes in a new slot at the end, the slots being full.
    assert_eq!(fx.world.neighbors.slots.len(), slots + 1);
    let record = fx.world.neighbors.first_of(guid).unwrap();
    assert_eq!(usize::try_from(record.id).unwrap(), slots + 1);
}

#[test]
fn an_adult_male_is_written_as_one() {
    let mut fx = fixture(2, 18, 0, 0, &TONES);
    unsupported(trap(fx.run()), "0x409663");
    let strings = &fx.world.characters.made[&expected_guid()].strings;
    assert_eq!(strings[&characters::SKELETON], "adult");
    assert_eq!(strings[&characters::GENDER], "male");
    assert_eq!(strings[&characters::AGE], "27");
    assert_eq!(strings[&characters::SKIN], "drk");
}

#[test]
fn the_file_number_skips_those_made_types_hold() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    for (guid, file) in [(5, 3), (6, 4)] {
        fx.world.characters.made.insert(
            guid,
            Character {
                template: TEMPLATE_PERSON,
                file,
                strings: BTreeMap::new(),
            },
        );
    }
    fx.world.characters.next_file = 3;
    unsupported(trap(fx.run()), "0x409663");
    assert_eq!(fx.world.characters.made[&expected_guid()].file, 5);
    assert_eq!(fx.world.characters.next_file, 6);
}

#[test]
fn the_file_counter_goes_back_to_0_after_the_last_number() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.world.characters.next_file = 31999;
    unsupported(trap(fx.run()), "0x409663");
    assert_eq!(fx.world.characters.made[&expected_guid()].file, 31999);
    assert_eq!(fx.world.characters.next_file, 0);
}

#[test]
fn a_skin_tone_with_no_name_throws_native_error_0x3b_after_the_clone() {
    for (skin, tones) in [(3, &TONES[..]), (-1, &TONES[..]), (1, &["lgt", ""][..])] {
        let mut fx = fixture(skin, 30, 0, 0, tones);
        let slots = fx.world.neighbors.slots.len();
        assert_eq!(code(fx.run()), Some(0x3b), "{skin} {tones:?}");
        // The type and its record stay, with the template's strings.
        let made = &fx.world.characters.made[&expected_guid()];
        assert!(made.strings.is_empty());
        assert_eq!(fx.world.neighbors.slots.len(), slots + 1);
    }
}

#[test]
fn the_unported_paths_fail_loudly_after_the_clone() {
    for (age, species, address) in [(-1, 0, "0x44b23c"), (30, 3, "0x44afa6")] {
        let mut fx = fixture(0, age, 0, species, &TONES);
        unsupported(trap(fx.run()), address);
        assert!(fx.world.characters.made.contains_key(&expected_guid()));
    }
}

#[test]
fn a_pet_fails_loudly() {
    for (species, template) in [(1, 0x4a70_df92), (2, 0x7bea_0977)] {
        let mut fx = fixture(0, 30, 0, species, &TONES);
        // The pet template's file is a clone of TemplatePerson's here.
        fx.add_type(template, 2, 0, 0);
        fx.program.people.profiles.insert(
            template,
            fx.program.people.profiles[&TEMPLATE_PERSON].clone(),
        );
        unsupported(trap(fx.run()), "0x44b051");
    }
}

#[test]
fn a_missing_template_fails_loudly_before_any_draw() {
    let mut fx = fixture(0, 30, 0, 1, &TONES);
    unsupported(trap(fx.run()), "0x426335");
    assert!(fx.world.characters.made.is_empty());
    assert_eq!(fx.world.crt_rng, CrtRng::from_state(77));
}

#[test]
fn a_new_guid_another_type_has_fails_loudly() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.add_type(expected_guid(), 7, 0, 0);
    unsupported(trap(fx.run()), "0x43bda9");
}
