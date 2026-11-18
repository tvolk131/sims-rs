//! Make New Character: its locals, the person type it clones, the body
//! strings it writes, and the body and head it draws.

use crate::support::*;
use sims_content::{Profile, Wardrobe};
use sims_core::CrtRng;
use sims_sim::characters::{self, Character};
use sims_vm::Trap;
use std::collections::BTreeMap;

const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;

/// A fixture whose lamp's tree 4096 sets its locals 0 to 2 to `skin`,
/// `age` and `sex`, then makes a new character from them with template
/// byte `species`, and on true puts the stack object's ID in global 34.
/// TemplatePerson is registered, with a neighbour record and skin tones
/// `tones`, and the wardrobe gives an adult male of tone "lgt" two bodies
/// and one head ([`wardrobe`]).
fn fixture(skin: i16, age: i16, sex: i16, species: u8, tones: &[&str]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, skin, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, age, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, sex, 3),
            ins(19, 4, RETURN_FALSE, [0, 1, 2, species, 0, 0, 0, 0]),
            assign(scope::GLOBAL, 34, scope::STACK_OBJECT, 0, RETURN_TRUE),
        ]),
    );
    fx.add_type(TEMPLATE_PERSON, 2, 0, 0);
    fx.world.neighbors.make_record(TEMPLATE_PERSON).unwrap();
    fx.program.people.profiles.insert(
        TEMPLATE_PERSON,
        Profile {
            tones: tones.iter().map(|t| t.to_string()).collect(),
            ..Profile::default()
        },
    );
    fx.program.wardrobe = Some(wardrobe());
    fx.world.ticks = 0x0012_3456;
    fx.world.crt_rng = CrtRng::from_state(77);
    fx
}

const TONES: [&str; 3] = ["lgt", "med", "drk"];

/// Suits and textures for every sex and age in tone "lgt" and "drk", but
/// none in "med" for a child.
fn wardrobe() -> Wardrobe {
    let names = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    Wardrobe {
        suits: names(&[
            "b001mafit_01",
            "b002fafit_01",
            "b003mcfit_01",
            "b004fcfit_01",
            "c001ma_bob",
            "c002fa",
            "c003mc",
            "c004fc",
        ]),
        textures: names(&[
            "b001mafitlgt_01.bmp",
            "b001mafitlgt_02.bmp",
            "b001mafitdrk.bmp",
            "b001mafitmed.bmp",
            "b002fafitlgt.bmp",
            "b003mcfitlgt.bmp",
            "b003mcfitdrk.bmp",
            "b004fcfitlgt.bmp",
            "c001malgt_bob.bmp",
            "c001madrk_bob.bmp",
            "c001mamed_bob.bmp",
            "c002falgt.bmp",
            "c003mclgt.bmp",
            "c003mcdrk.bmp",
            "c004fclgt.bmp",
        ]),
        errors: BTreeMap::new(),
    }
}

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
    let mut fx = fixture(0, 10, 1, 0, &TONES);
    let slots = fx.world.neighbors.slots.len();
    let mut lot = fx.world.lot_rng;
    assert_eq!(fx.run(), Ok(true));
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
                (characters::SKIN, "lgt".into()),
            ]),
        }
    );
    // Two game draws: the head, then the body.
    lot.next_u32();
    lot.next_u32();
    assert_eq!(fx.world.lot_rng, lot);
    assert_eq!(fx.world.characters.next_file, 1);
    // One C runtime draw, for the GUID.
    let mut crt = CrtRng::from_state(77);
    crt.next_u15();
    assert_eq!(fx.world.crt_rng, crt);
    // The record goes in a new slot at the end, the slots being full.
    assert_eq!(fx.world.neighbors.slots.len(), slots + 1);
    let record = fx.world.neighbors.first_of(guid).unwrap();
    assert_eq!(usize::try_from(record.id).unwrap(), slots + 1);
    // The stack object is the new record.
    assert_eq!(fx.world.globals[34], record.id);
}

#[test]
fn with_no_body_or_no_head_for_the_tone_it_fails_loudly() {
    // No child body or head in "med", and no adult female body in "med".
    let mut fx = fixture(1, 10, 0, 0, &TONES);
    unsupported(trap(fx.run()), "0x44b203");
    let mut fx = fixture(1, 30, 1, 0, &TONES);
    unsupported(trap(fx.run()), "0x44b203");
}

#[test]
fn with_no_suit_names_it_fails_loudly() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.program.wardrobe = None;
    unsupported(trap(fx.run()), "not read");
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    let mut wardrobe = wardrobe();
    wardrobe.errors.insert("b.cmx".into(), "bad".into());
    fx.program.wardrobe = Some(wardrobe);
    unsupported(trap(fx.run()), "b.cmx");
    // The boot keeps a person file it could not list for this call.
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.program.wardrobe = Some(Wardrobe::unread("Downloads/damaged.far: bad".into()));
    unsupported(trap(fx.run()), "damaged.far");
}

#[test]
fn an_adult_male_is_written_as_one() {
    let mut fx = fixture(2, 18, 0, 0, &TONES);
    assert_eq!(fx.run(), Ok(true));
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
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(fx.world.characters.made[&expected_guid()].file, 5);
    assert_eq!(fx.world.characters.next_file, 6);
}

#[test]
fn the_file_number_skips_the_files_the_characters_folder_held() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.program.character_files = (0..172).collect();
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(fx.world.characters.made[&expected_guid()].file, 172);
    assert_eq!(fx.world.characters.next_file, 173);
}

#[test]
fn with_the_last_number_taken_the_clone_fails_and_the_counter_goes_back_to_0() {
    // Numbers below 31999 are free, but the counter is past them.
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.program.character_files = [31999].into();
    fx.world.characters.next_file = 31999;
    unsupported(trap(fx.run()), "0x4262c0");
    assert_eq!(fx.world.characters.next_file, 0);
    assert!(fx.world.characters.made.is_empty());
    assert_eq!(fx.world.crt_rng, CrtRng::from_state(77));
}

#[test]
fn the_file_counter_goes_back_to_0_after_the_last_number() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.world.characters.next_file = 31999;
    assert_eq!(fx.run(), Ok(true));
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
        fx.world.neighbors.make_record(template).unwrap();
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
fn a_template_with_no_record_or_two_definitions_fails_loudly_before_any_draw() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.world.neighbors.slots.iter_mut().for_each(|slot| {
        if slot.as_ref().is_some_and(|n| n.guid == TEMPLATE_PERSON) {
            *slot = None;
        }
    });
    unsupported(trap(fx.run()), "no neighbour record");
    assert_eq!(fx.world.characters.next_file, 0);
    assert_eq!(fx.world.crt_rng, CrtRng::from_state(77));
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.add_type(TEMPLATE_PERSON, 2, 0, 0);
    unsupported(trap(fx.run()), "0x42571f");
    assert_eq!(fx.world.crt_rng, CrtRng::from_state(77));
}

#[test]
fn a_local_that_starts_as_half_of_a_pointer_fails_loudly_before_the_clone() {
    // Local 3 is never written: the skin (b0), then the sex (b2), read it.
    for (operands, address) in [
        ([3, 1, 2, 0, 0, 0, 0, 0], "0x44ae23"),
        ([0, 1, 3, 0, 0, 0, 0, 0], "0x44ae49"),
        ([0, 3, 2, 0, 0, 0, 0, 0], "0x44ae70"),
    ] {
        let mut fx = fixture(0, 30, 0, 0, &TONES);
        fx.add(
            LAMP_FILE,
            4096,
            tree(vec![ins(19, RETURN_TRUE, RETURN_FALSE, operands)]),
        );
        let mut storage = sims_sim::FrameStorage::default();
        storage.unknown.insert((0, 3));
        fx.world.frame_storage.insert(LAMP, storage);
        let trap = trap(fx.run());
        unsupported(trap.clone(), address);
        unsupported(trap, "half of a pointer");
        assert!(fx.world.characters.made.is_empty());
        assert_eq!(fx.world.crt_rng, CrtRng::from_state(77));
    }
}

#[test]
fn a_new_guid_another_type_has_fails_loudly() {
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.add_type(expected_guid(), 7, 0, 0);
    unsupported(trap(fx.run()), "0x43bda9");
}
