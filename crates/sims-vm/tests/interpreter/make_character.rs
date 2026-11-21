//! Make New Character: its locals, the person type it clones, the body
//! strings it writes, and the body and head it draws.

use crate::support::*;
use sims_content::{Profile, Wardrobe};
use sims_core::CrtRng;
use sims_sim::characters::{self, Character};
use sims_sim::neighbors::{Inventory, Item, Row};
use sims_vm::Trap;
use std::collections::BTreeMap;

const TEMPLATE_PERSON: u32 = 0x7fd9_6b54;
const TEMPLATE_DOG: u32 = 0x4a70_df92;
const TEMPLATE_CAT: u32 = 0x7bea_0977;

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
fn with_no_body_or_no_head_for_the_tone_it_cleans_up_and_returns_false() {
    // No child body or head in "med", and no adult female body in "med".
    for (age, sex) in [(10, 0), (30, 1)] {
        let mut fx = fixture(1, age, sex, 0, &TONES);
        fx.world.globals[34] = 99;
        let slots = fx.world.neighbors.slots.len();
        let lot_before = fx.world.lot_rng;
        // Pre-populate a relationship row targeting the soon-to-be-allocated
        // slot (slots + 1) twice, and a pre-existing inventory with kind 3
        // (cleared) and kind 10 (kept), to exercise 0x4259a2 completely.
        let target_id = (slots + 1) as i16;
        fx.world.neighbors.slots[0].as_mut().unwrap().relationships = vec![
            Row {
                target: i32::from(target_id),
                values: vec![10],
            },
            Row {
                target: i32::from(target_id),
                values: vec![20],
            },
        ];
        fx.world.neighbors.inventories.push(Inventory {
            neighbor: target_id,
            items: vec![
                Item {
                    kind: 3,
                    guid: 0x1111,
                    count: 1,
                },
                Item {
                    kind: 10,
                    guid: 0x2222,
                    count: 2,
                },
            ],
        });

        assert_eq!(fx.run(), Ok(false));
        // Type is unregistered (file number 0 is free again), counter stayed
        // advanced, 1 CRT draw happened, 0 game draws happened, stack object
        // was not updated (global 34 untouched).
        assert!(fx.world.characters.made.is_empty());
        assert_eq!(fx.world.characters.next_file, 1);
        assert_eq!(fx.world.lot_rng, lot_before);
        let mut crt = CrtRng::from_state(77);
        crt.next_u15();
        assert_eq!(fx.world.crt_rng, crt);
        assert_eq!(fx.world.globals[34], 99);
        // Slot was vacated in place (len stays slots + 1, slot is None).
        assert_eq!(fx.world.neighbors.slots.len(), slots + 1);
        assert!(fx.world.neighbors.slots[slots].is_none());
        // Only the first relationship row targeting target_id was erased (0x464fa2).
        let rels = &fx.world.neighbors.slots[0].as_ref().unwrap().relationships;
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].values, vec![20]);
        // Inventory kept kind 10 and removed kind 3 (0x41f4e5 for kinds 0..=9).
        assert_eq!(fx.world.neighbors.inventories.len(), 1);
        assert_eq!(fx.world.neighbors.inventories[0].items.len(), 1);
        assert_eq!(fx.world.neighbors.inventories[0].items[0].kind, 10);
    }

    // Without a pre-existing inventory, 0x4259a2 -> 0x42a8a1 appends an empty one.
    let mut fx = fixture(1, 10, 0, 0, &TONES);
    let slots = fx.world.neighbors.slots.len();
    assert_eq!(fx.run(), Ok(false));
    assert_eq!(
        fx.world.neighbors.inventories,
        vec![Inventory {
            neighbor: (slots + 1) as i16,
            items: Vec::new(),
        }]
    );
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
fn a_template_byte_past_two_with_non_negative_age_fails_loudly_after_the_clone() {
    let mut fx = fixture(0, 30, 0, 3, &TONES);
    unsupported(trap(fx.run()), "0x44afa6");
    assert!(fx.world.characters.made.contains_key(&expected_guid()));
}

fn add_pet_templates(fx: &mut Fixture) {
    for (template, gender) in [(TEMPLATE_DOG, "dogmale"), (TEMPLATE_CAT, "catmale")] {
        fx.add_type(template, 2, 0, 0);
        fx.world.neighbors.make_record(template).unwrap();
        fx.program.people.profiles.insert(
            template,
            Profile {
                gender: Some(gender.into()),
                tones: TONES.iter().map(|t| t.to_string()).collect(),
                ..Profile::default()
            },
        );
    }
    let w = fx.program.wardrobe.as_mut().unwrap();
    w.suits.insert("b001dog_lab".into());
    w.textures.insert("b001dog_lab_01.bmp".into());
    w.textures.insert("b001dog_lab_02.bmp".into());
    w.suits.insert("b001cat_tabby".into());
    w.textures.insert("b001cat_tabby_01.bmp".into());
}

#[test]
fn pets_force_adult_age_set_pet_strings_and_use_declined_or_draw() {
    for (species, template, sex, skeleton, gender) in [
        (1, TEMPLATE_DOG, 0, "dog", "dogmale"),
        (1, TEMPLATE_DOG, 1, "dog", "dogfemale"),
        (2, TEMPLATE_CAT, 0, "kat", "catmale"),
        (2, TEMPLATE_CAT, 1, "kat", "catfemale"),
    ] {
        // Default registry word 0x100 is -1 (0x43f72d): draws 1 game RNG.
        let mut fx = fixture(0, 5, sex, species, &TONES);
        add_pet_templates(&mut fx);
        assert_eq!(fx.world.dialogs.declined, -1);
        let mut lot = fx.world.lot_rng;
        assert_eq!(fx.run(), Ok(true));
        let guid = expected_guid();
        let made = &fx.world.characters.made[&guid];
        assert_eq!(made.template, template);
        assert_eq!(
            made.strings,
            BTreeMap::from([
                (characters::SKELETON, skeleton.into()),
                (characters::GENDER, gender.into()),
                (characters::AGE, "27".into()),
                (characters::SKIN, "lgt".into()),
            ])
        );
        lot.next_u32();
        assert_eq!(fx.world.lot_rng, lot);
        assert_eq!(fx.world.dialogs.declined, -1);
        let record = fx.world.neighbors.first_of(guid).unwrap();
        assert_eq!(fx.world.globals[34], record.id);

        // Dialog-selected pet index (>= 0): draws 0 game RNG and resets word
        // 0x100 to -1 (0x44b06f, 0x44b09b).
        let mut fx = fixture(1, 5, sex, species, &TONES);
        add_pet_templates(&mut fx);
        fx.world.dialogs.declined = 0;
        let lot_before = fx.world.lot_rng;
        assert_eq!(fx.run(), Ok(true));
        assert_eq!(fx.world.lot_rng, lot_before);
        assert_eq!(fx.world.dialogs.declined, -1);
    }
}

#[test]
fn a_pet_with_zero_bodies_cleans_up_without_clearing_declined() {
    let mut fx = fixture(0, 5, 0, 1, &TONES);
    add_pet_templates(&mut fx);
    // Remove all dog bodies from the wardrobe.
    fx.program
        .wardrobe
        .as_mut()
        .unwrap()
        .suits
        .retain(|s| !s.contains("dog"));
    fx.world.dialogs.declined = 2;
    let slots = fx.world.neighbors.slots.len();
    let lot_before = fx.world.lot_rng;
    assert_eq!(fx.run(), Ok(false));
    assert!(fx.world.characters.made.is_empty());
    assert_eq!(fx.world.lot_rng, lot_before);
    assert_eq!(fx.world.dialogs.declined, 2);
    assert_eq!(fx.world.neighbors.slots.len(), slots + 1);
    assert!(fx.world.neighbors.slots[slots].is_none());
    assert_eq!(
        fx.world.neighbors.inventories,
        vec![Inventory {
            neighbor: (slots + 1) as i16,
            items: Vec::new(),
        }]
    );
}

#[test]
fn negative_age_copies_running_human_without_drawing_or_checking_skin_tone() {
    // Make LAMP a person definition (kind 2) with profile strings.
    let mut fx = fixture(-1, -1, 0, 3, &TONES); // out-of-range skin & species >= 3 both skipped
    fx.catalog.objects[0].kind = 2;
    let lamp_guid = fx.world.objects[&LAMP].guid;
    fx.program.people.profiles.insert(
        lamp_guid,
        Profile {
            gender: Some("Female".into()),
            age: Some("12".into()),
            skin: None, // 0x40d007 maps null to "???"
            ..Profile::default()
        },
    );
    let lot_before = fx.world.lot_rng;
    assert_eq!(fx.run(), Ok(true));
    let guid = expected_guid();
    let made = &fx.world.characters.made[&guid];
    assert_eq!(made.template, TEMPLATE_PERSON);
    assert_eq!(
        made.strings,
        BTreeMap::from([
            (characters::SKELETON, "child".into()),
            (characters::GENDER, "female".into()),
            (characters::AGE, "9".into()),
            (characters::SKIN, "???".into()),
        ])
    );
    assert_eq!(fx.world.lot_rng, lot_before);
    assert_eq!(
        fx.world.globals[34],
        fx.world.neighbors.first_of(guid).unwrap().id
    );

    // Adult male running object copying onto TemplateDog (dst_is_pet == true):
    // 0x40cc2a writes AGE = "27" and skips SKELETON (0x40cee4), then 0x40ce41
    // writes GENDER = "male" without overwriting SKELETON.
    let mut fx = fixture(0, -5, 1, 1, &TONES);
    add_pet_templates(&mut fx);
    fx.catalog.objects[0].kind = 2;
    fx.program.people.profiles.insert(
        lamp_guid,
        Profile {
            gender: Some("male".into()),
            age: Some("27".into()),
            skin: Some("med".into()),
            ..Profile::default()
        },
    );
    assert_eq!(fx.run(), Ok(true));
    let made = &fx.world.characters.made[&expected_guid()];
    assert_eq!(made.template, TEMPLATE_DOG);
    assert_eq!(
        made.strings,
        BTreeMap::from([
            (characters::GENDER, "male".into()),
            (characters::AGE, "27".into()),
            (characters::SKIN, "med".into()),
        ])
    );
}

#[test]
fn negative_age_fails_loudly_for_non_person_or_pet_running_object() {
    // Non-person running object (LAMP has kind 4): 0x457f76 returns null.
    let mut fx = fixture(0, -1, 0, 0, &TONES);
    unsupported(trap(fx.run()), "0x457f76");
    assert!(fx.world.characters.made.contains_key(&expected_guid()));

    // Pet or unknown-gender running person: 0x40d408 dereferences null head list.
    for gender in [Some("dogmale"), Some("unknown"), None] {
        let mut fx = fixture(0, -1, 0, 0, &TONES);
        fx.catalog.objects[0].kind = 2;
        let lamp_guid = fx.world.objects[&LAMP].guid;
        fx.program.people.profiles.insert(
            lamp_guid,
            Profile {
                gender: gender.map(str::to_owned),
                ..Profile::default()
            },
        );
        unsupported(trap(fx.run()), "0x40d42e");
        assert!(fx.world.characters.made.contains_key(&expected_guid()));
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
