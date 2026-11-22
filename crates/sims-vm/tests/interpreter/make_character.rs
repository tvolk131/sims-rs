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
/// `age` and `sex` and `stack_object` to 77, then makes a new character
/// from them with template byte `species`. On true it puts `stack_object`
/// in global 34; on false it puts `stack_object` in temp 0.
fn fixture(skin: i16, age: i16, sex: i16, species: u8, tones: &[&str]) -> Fixture {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![
            assign(scope::LOCALS, 0, scope::LITERAL, skin, 1),
            assign(scope::LOCALS, 1, scope::LITERAL, age, 2),
            assign(scope::LOCALS, 2, scope::LITERAL, sex, 3),
            assign(scope::STACK_OBJECT, 0, scope::LITERAL, 77, 4),
            ins(19, 5, 6, [0, 1, 2, species, 0, 0, 0, 0]),
            assign(scope::GLOBAL, 34, scope::STACK_OBJECT, 0, RETURN_TRUE),
            assign(scope::TEMPS, 0, scope::STACK_OBJECT, 0, RETURN_FALSE),
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
    // Test body-only miss (bodies == 0, heads > 0) AND head-only miss
    // (bodies > 0, heads == 0) separately so neither check can be dropped.
    for remove_head in [false, true] {
        let mut fx = fixture(0, 30, 0, 0, &TONES);
        fx.world.globals[34] = 99;
        let w = fx.program.wardrobe.as_mut().unwrap();
        if remove_head {
            w.textures.remove("c001malgt_bob.bmp");
        } else {
            w.textures.remove("b001mafitlgt_01.bmp");
            w.textures.remove("b001mafitlgt_02.bmp");
        }
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
        // stayed at 77 (written to global 35 on false branch).
        assert!(fx.world.characters.made.is_empty());
        assert_eq!(fx.world.characters.next_file, 1);
        assert_eq!(fx.world.lot_rng, lot_before);
        let mut crt = CrtRng::from_state(77);
        crt.next_u15();
        assert_eq!(fx.world.crt_rng, crt);
        assert_eq!(fx.world.globals[34], 99);
        assert_eq!(fx.world.objects[&LAMP].temps[0], 77);
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

    // If a family protects the cloned GUID, 0x409492(0) still discards the
    // uncommitted body-string edits before 0x426fcd returns early.
    let mut fx = fixture(1, 10, 0, 0, &TONES);
    fx.world.families.push(sims_sim::neighbors::Family {
        id: 1,
        house: 0,
        members: vec![expected_guid()],
        friends: 0,
        stale: true,
    });
    assert_eq!(fx.run(), Ok(false));
    assert!(
        fx.world.characters.made[&expected_guid()]
            .strings
            .is_empty()
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
    fx.program.expansions |= 0x20;
    crate::rooms::build(fx);
    fx.world.arrays.insert(3, crate::rooms::array(2, 0));
    fx.world.arrays.insert(103, crate::rooms::array(2, 0));
    fx.world.arrays.insert(9, crate::rooms::array(1, 0xff));
    fx.world.arrays.insert(10, crate::rooms::array(1, 0xff));
    fx.world.indicators = [LAMP, SIM];
    let init = tree(vec![
        assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
        assign(scope::MY_DATA, 42, scope::LITERAL, 2, RETURN_TRUE),
    ]);
    let main = tree(vec![
        assign(scope::PARAMETERS, 0, scope::LITERAL, 100, 1),
        ins(0, 0, 0, [0; 8]),
    ]);
    for (template, skeleton, gender) in [
        (TEMPLATE_DOG, "dog", "dogmale"),
        (TEMPLATE_CAT, "kat", "catmale"),
    ] {
        let file = fx.add_type(template, 2, 4096, 4097);
        fx.add(file, 4096, init.clone());
        fx.add(file, 4097, main.clone());
        crate::person_factory::equip(fx, template, skeleton, 12);
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
    for suit in ["b001dog_lab", "b001cat_tabby"] {
        fx.program.people.suits.insert(suit.into());
        fx.program.people.head_suits.insert(suit.into());
    }
}

#[test]
fn pets_force_adult_age_set_pet_strings_and_use_declined_or_draw() {
    for (species, template, sex, skeleton, gender) in [
        (1, TEMPLATE_DOG, 0, "dog", "dogmale"),
        (1, TEMPLATE_DOG, 1, "dog", "dogfemale"),
        (2, TEMPLATE_CAT, 0, "kat", "catmale"),
        (2, TEMPLATE_CAT, 1, "kat", "catfemale"),
    ] {
        // Default registry word 0x100 is -1 (0x43f72d): draws 1 game RNG,
        // then runs 0x44b126 → 0x4e8c5d(index, 0x1f, 1) populating slot 1
        // and saving BMP_ 2002 without leaving the temporary pet behind.
        let mut fx = fixture(0, 5, sex, species, &TONES);
        add_pet_templates(&mut fx);
        assert_eq!(fx.world.dialogs.declined, -1);
        let people_before = fx.world.people.clone();
        let objects_before = fx.world.objects.len();
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
        // 0x44b126 calls 0x4e8c5d(index, 0x1f, 1): bit 0x01 renders & saves
        // BMP_ 2002 (`saved_portraits`), then bits 0x08/0x02/0x04/0x10 run
        // forced dresses (`0x58d4e1 -> 0x4eb0cc -> 0x4e897e`) which clear the
        // in-memory portrait slot when the pet's head slot is empty ("NONE").
        assert!(fx.world.portraits.is_empty());
        assert_eq!(fx.world.saved_portraits, [guid].into());
        assert!(fx.world.dirty_portraits.is_empty());
        assert_eq!(fx.world.people, people_before);
        assert_eq!(fx.world.objects.len(), objects_before);
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
        assert!(fx.world.portraits.is_empty());
        assert_eq!(fx.world.saved_portraits, [guid].into());
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
fn skin_tone_truncates_to_three_bytes_and_negative_age_truncates_to_fifteen_bytes() {
    // Non-negative human: "lgtExtra" truncates to "lgt" (0x40bf8b) before
    // wardrobe lookup and writing body string 14.
    let mut fx = fixture(0, 30, 0, 0, &["lgtExtra"]);
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(
        fx.world.characters.made[&expected_guid()].strings[&characters::SKIN],
        "lgt"
    );

    // Non-negative pet: shipped "unknown" tone truncates to "unk" (0x40bf8b).
    let mut fx = fixture(0, 5, 0, 1, &TONES);
    add_pet_templates(&mut fx);
    fx.program
        .people
        .profiles
        .get_mut(&TEMPLATE_DOG)
        .unwrap()
        .tones = vec!["unknown".into()];
    assert_eq!(fx.run(), Ok(true));
    assert_eq!(
        fx.world.characters.made[&expected_guid()].strings[&characters::SKIN],
        "unk"
    );

    // Negative age: "0000000000000018" truncates to 15 bytes ("000000000000001")
    // at 0x40cbdd -> atoi == 1 <= 17 (child "9"), and "drkExtra" truncates to
    // "drk" at 0x40bf8b.
    let mut fx = fixture(0, -1, 0, 0, &TONES);
    fx.catalog.objects[0].kind = 2;
    let lamp_guid = fx.world.objects[&LAMP].guid;
    fx.program.people.profiles.insert(
        lamp_guid,
        Profile {
            gender: Some("male".into()),
            age: Some("0000000000000018".into()),
            skin: Some("drkExtra".into()),
            ..Profile::default()
        },
    );
    assert_eq!(fx.run(), Ok(true));
    let strings = &fx.world.characters.made[&expected_guid()].strings;
    assert_eq!(strings[&characters::AGE], "9");
    assert_eq!(strings[&characters::SKELETON], "child");
    assert_eq!(strings[&characters::SKIN], "drk");
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

#[test]
fn pet_creation_runs_0x4e8c5d_portrait_build_even_when_template_has_bmp_2002() {
    // 0x44b126 → 0x4e8c5d(index, 0x1f, 1) creates a temporary pet and runs
    // Init unconditionally even when the cloned template already carries
    // BMP_ 2002 (as shipped TemplateCat.iff / TemplateDog.iff do).
    let setup = |has_bmp: bool, size: i16| {
        let mut fx = fixture(0, 5, 0, 2, &TONES);
        add_pet_templates(&mut fx);
        fx.world.size = size;
        let cat_file = fx.catalog.object(TEMPLATE_CAT).unwrap().resource;
        let lib = fx.program.libraries.get_mut(&cat_file).unwrap();
        lib.has_portrait_bitmap = has_bmp;
        lib.sounds.insert(5, "pet_init_ping".into());
        fx.add(
            cat_file,
            4096,
            tree(vec![
                assign(scope::MY_DATA, 4, scope::LITERAL, 1, 1),
                assign(scope::MY_DATA, 42, scope::LITERAL, 2, 2),
                ins(23, RETURN_TRUE, ERROR, [5, 0, 0, 0, 0, 0, 0, 0]),
            ]),
        );
        fx
    };
    let count_pings = |events: &[sims_vm::SoundEvent]| {
        events
            .iter()
            .filter(
                |e| matches!(e, sims_vm::SoundEvent::Play { name, .. } if name == "pet_init_ping"),
            )
            .count()
    };

    // 1. Placeable lot with template carrying BMP_ 2002: still runs Init
    //    (create + remove's reset = 2 pings) and populates slot 1 + saved.
    let mut fx = setup(true, 6);
    let sounds = std::cell::RefCell::new(Vec::new());
    let mut interpreter = sims_vm::Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.sounds = Some(&sounds);
    let res = interpreter.run_tree(&mut fx.world, LAMP, 4096, 0, &[], 1000);
    assert_eq!(res, Ok(true));
    assert_eq!(count_pings(&sounds.borrow()), 2);
    let guid = expected_guid();
    assert!(fx.world.portraits.is_empty());
    assert_eq!(fx.world.saved_portraits, [guid].into());

    // 2. Blocked lot (size = 2, no free tile): 0x4e9706 removes unput and
    //    returns NULL → 0x4e8c5d returns false (ignored by 0x44b126), so
    //    Make New Character still succeeds with 2 Init pings, leaving slot 1
    //    empty and saved_portraits untouched.
    let mut fx_blocked = setup(false, 2);
    let sounds = std::cell::RefCell::new(Vec::new());
    let mut interpreter =
        sims_vm::Interpreter::new(&fx_blocked.program, &fx_blocked.catalog, fx_blocked.stream);
    interpreter.sounds = Some(&sounds);
    let res = interpreter.run_tree(&mut fx_blocked.world, LAMP, 4096, 0, &[], 1000);
    assert_eq!(res, Ok(true));
    assert_eq!(count_pings(&sounds.borrow()), 2);
    assert!(fx_blocked.world.portraits.is_empty());
    assert!(fx_blocked.world.saved_portraits.is_empty());

    // 3. Mixed starts with view_centre = None: placing populates slot 1 and
    //    saves BMP_ 2002 while removing unput does neither, so fail loudly.
    let mut fx_mixed = setup(false, 6);
    fx_mixed.world.rooms.map[0][64 + 1] = 9;
    let grid = &mut fx_mixed.world.arrays.get_mut(&3).unwrap().data;
    grid[(64 + 1) * 2..((64 + 1) * 2 + 2)].copy_from_slice(&(LAMP as i16).to_le_bytes());
    unsupported(trap(fx_mixed.run()), "0x44b126");

    // 4. Reset writing person data word 68 runs at 0x4e8fdb AFTER 0x4e8d2e
    //    (save BMP_ 2002) and 0x4e8d6d (populate slot 1), leaving slot 1 empty
    //    and dirty while keeping BMP_ 2002 saved.
    let mut fx_reset = setup(false, 6);
    let cat_file = fx_reset.catalog.object(TEMPLATE_CAT).unwrap().resource;
    // Register Reset callback (OBJf entry 3, tree 4100) writing My Person
    // Data 68 := 1 when remove_now runs Reset at 0x4e8fdb.
    crate::visits::function(&mut fx_reset, cat_file, 3, 4100);
    fx_reset.add(
        cat_file,
        4100,
        tree(vec![assign(
            scope::MY_PERSON,
            68,
            scope::LITERAL,
            1,
            RETURN_TRUE,
        )]),
    );
    assert_eq!(fx_reset.run(), Ok(true));
    let guid = expected_guid();
    assert_eq!(fx_reset.world.saved_portraits, [guid].into());
    assert!(!fx_reset.world.portraits.contains(&guid));
    assert!(fx_reset.world.dirty_portraits.contains(&guid));
}

#[test]
fn cloning_a_template_inherits_bmp_2002_saved_during_play() {
    // 0x472356..0x472519 copies every resource currently in the template's
    // file into the new `User%05d.iff`, including any BMP_ 2002 saved during
    // play. A clone inherits a bitmap saved into its template BEFORE cloning,
    // does NOT retroactively inherit one saved AFTER cloning, and failed
    // wardrobe cleanup drops the discarded clone's entry from saved_portraits.
    let mut fx = fixture(0, 30, 0, 0, &TONES);
    fx.world.saved_portraits.insert(TEMPLATE_PERSON);
    assert_eq!(fx.run(), Ok(true));
    let guid = expected_guid();
    assert!(
        fx.world.saved_portraits.contains(&guid),
        "clone inherits BMP_ 2002 saved into template before cloning"
    );

    // Failed wardrobe cleanup (bodies == 0) clears saved_portraits for the
    // discarded clone while leaving the template's saved_portraits intact.
    let mut fx_fail = fixture(1, 10, 0, 0, &TONES);
    fx_fail.world.saved_portraits.insert(TEMPLATE_PERSON);
    assert_eq!(fx_fail.run(), Ok(false));
    assert!(fx_fail.world.saved_portraits.contains(&TEMPLATE_PERSON));
    assert!(!fx_fail.world.saved_portraits.contains(&expected_guid()));

    // Saving BMP_ 2002 into the template AFTER cloning does not affect an
    // already-cloned character's separate User%05d.iff.
    let mut fx_after = fixture(0, 30, 0, 0, &TONES);
    assert_eq!(fx_after.run(), Ok(true));
    let guid_after = expected_guid();
    assert!(!fx_after.world.saved_portraits.contains(&guid_after));
    fx_after.world.saved_portraits.insert(TEMPLATE_PERSON);
    assert!(!fx_after.world.saved_portraits.contains(&guid_after));
}

#[test]
fn pet_creation_without_pet_expansions_skips_temporary_portrait_pet_and_keeps_character() {
    // With pet resources present and both Unleashed (0x20) and Makin' Magic
    // (0x80) flags clear, native 0x440dd5 returns NULL -> 0x4e8c5d returns
    // false (ignored by 0x44b126), so Make New Character still succeeds and
    // keeps the newly cloned character without populating slot 1 or saving.
    let mut fx = fixture(0, 5, 0, 2, &TONES);
    add_pet_templates(&mut fx);
    fx.program.expansions = 0;
    assert_eq!(fx.run(), Ok(true));
    let guid = expected_guid();
    assert!(fx.world.characters.made.contains_key(&guid));
    assert!(fx.world.portraits.is_empty());
    assert!(fx.world.saved_portraits.is_empty());
}

#[test]
fn make_new_character_names_new_child_and_pet_from_last_kind3_dialog_text() {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    // 0x44b0a8..0x44b0fc: Make New Character reads global [0x6e4ac0] (populated
    // by the last kind 3 text dialog) and writes it to the created character's
    // catalog name.
    let dialog_text = RefCell::new("Bella".to_owned());
    let character_names = RefCell::new(BTreeMap::new());
    let mut fx = fixture(0, 5, 1, 0, &TONES);
    let mut interp = sims_vm::Interpreter::new(&fx.program, &fx.catalog, sims_vm::Stream::Global);
    interp.dialog_text = Some(&dialog_text);
    interp.character_names = Some(&character_names);
    assert_eq!(
        interp.run_tree(&mut fx.world, LAMP, 4096, SIM as i16, &[], 1000),
        Ok(true)
    );
    let child_guid = expected_guid();
    assert_eq!(
        character_names
            .borrow()
            .get(&child_guid)
            .map(String::as_str),
        Some("Bella")
    );

    // Pet creation also receives the submitted dialog text.
    *dialog_text.borrow_mut() = "Fido".to_owned();
    let mut fx_pet = fixture(0, 5, 0, 1, &TONES);
    add_pet_templates(&mut fx_pet);
    fx_pet.program.expansions = 0;
    let mut interp_pet =
        sims_vm::Interpreter::new(&fx_pet.program, &fx_pet.catalog, sims_vm::Stream::Global);
    interp_pet.dialog_text = Some(&dialog_text);
    interp_pet.character_names = Some(&character_names);
    assert_eq!(
        interp_pet.run_tree(&mut fx_pet.world, LAMP, 4096, SIM as i16, &[], 1000),
        Ok(true)
    );
    let pet_guid = expected_guid();
    assert_eq!(
        character_names.borrow().get(&pet_guid).map(String::as_str),
        Some("Fido")
    );

    // Empty kind-3 submission falls back to localized STR# 142[5] default name
    // (`0x44b0bf..0x44b0d6`).
    dialog_text.borrow_mut().clear();
    let mut fx_default = fixture(0, 5, 1, 0, &TONES);
    let mut interp_default = sims_vm::Interpreter::new(
        &fx_default.program,
        &fx_default.catalog,
        sims_vm::Stream::Global,
    );
    interp_default.dialog_text = Some(&dialog_text);
    interp_default.character_names = Some(&character_names);
    interp_default.default_character_name = Some("DefaultKid");
    assert_eq!(
        interp_default.run_tree(&mut fx_default.world, LAMP, 4096, SIM as i16, &[], 1000),
        Ok(true)
    );
    assert_eq!(
        character_names
            .borrow()
            .get(&expected_guid())
            .map(String::as_str),
        Some("DefaultKid")
    );
}
