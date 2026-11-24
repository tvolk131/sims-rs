//! Generic Sims Call routine 24 (0x44ff6e), the outfit editor, against a
//! synthetic wardrobe: made-up suit and texture names only.

use crate::support::*;
use std::collections::BTreeMap;

const BODY_0: &str = "b100mafit_x,BODY=b100mafitlgt_one";
const BODY_1: &str = "b100mafit_x,BODY=b100mafitlgt_two";
const HEAD: &str = "c100ma_x,HEAD-HEAD=c100malgt_face";
const FORMAL_0: &str = "f100mafit_x,BODY=f100mafitlgt_one";
const FORMAL_1: &str = "f100mafit_x,BODY=f100mafitlgt_two";
const SWIM: &str = "s100mafit_x,BODY=s100mafitlgt_one";
const PAJAMAS: &str = "l100mafit_x,BODY=l100mafitlgt_one";
const WINTER: &str = "w100mafit_x,BODY=w100mafitlgt_one";
const FASHION: &str = "h100mafit_x,BODY=h100mafitlgt_one";

/// The Sim as an adult man of light skin wearing the first body and the
/// second formal outfit, with one suit of every other category, so the
/// commit's check (0x40a588) keeps every row.
fn dressed_sim() -> Fixture {
    let mut fx = Fixture::new();
    for suit in [
        "b100mafit_x",
        "c100ma_x",
        "f100mafit_x",
        "s100mafit_x",
        "l100mafit_x",
        "w100mafit_x",
        "h100mafit_x",
    ] {
        fx.program.people.eager_suits.insert(suit.into());
    }
    fx.program.people.textures = [
        "b100mafitlgt_one",
        "b100mafitlgt_two",
        "c100malgt_face",
        "f100mafitlgt_one",
        "f100mafitlgt_two",
        "s100mafitlgt_one",
        "l100mafitlgt_one",
        "w100mafitlgt_one",
        "h100mafitlgt_one",
    ]
    .map(String::from)
    .to_vec();
    fx.program.people.profiles.insert(
        SIM_GUID,
        sims_content::Profile {
            body: Some(BODY_0.into()),
            head: Some(HEAD.into()),
            gender: Some("male".into()),
            age: Some("30".into()),
            skin: Some("lgt".into()),
            suit_rows: BTreeMap::from([
                (30, FORMAL_1.into()),
                (31, SWIM.into()),
                (32, PAJAMAS.into()),
                (33, WINTER.into()),
                (34, FASHION.into()),
            ]),
            ..Default::default()
        },
    );
    fx
}

/// Runs routine 24 as `me` with temps 0 and 1 at `category` and `index`.
fn generic_24(
    fx: &mut Fixture,
    me: u16,
    category: i16,
    index: i16,
) -> Result<bool, sims_vm::Fault> {
    let file = if me == SIM { SIM_FILE } else { LAMP_FILE };
    let object = fx.world.objects.get_mut(&me).unwrap();
    (object.temps[0], object.temps[1]) = (category, index);
    fx.add(
        file,
        4096,
        tree(vec![ins(
            1,
            RETURN_TRUE,
            RETURN_FALSE,
            [24, 0, 0, 0, 0, 0, 0, 0],
        )]),
    );
    fx.run_with(me, 4096, 0, &[], 10)
}

fn row(fx: &Fixture, idx: usize) -> Option<&str> {
    fx.world
        .made_outfits
        .get(&SIM_GUID)
        .and_then(|rows| rows.get(&idx))
        .map(String::as_str)
}

#[test]
fn generic_call_24_puts_on_a_body_and_answers_the_one_worn() {
    // Category 0 writes slot 2, row 1 (0x40d10b), and temp 1 becomes the
    // body worn before (0x40d0a4).
    let mut fx = dressed_sim();
    assert_eq!(generic_24(&mut fx, SIM, 0, 1), Ok(true));
    assert_eq!(row(&fx, 1), Some(BODY_1));
    assert_eq!(fx.sim().temps[..2], [0, 0]);
    // Back again: the kept index is now 1.
    assert_eq!(generic_24(&mut fx, SIM, 0, 0), Ok(true));
    assert_eq!(row(&fx, 1), Some(BODY_0));
    assert_eq!(fx.sim().temps[..2], [0, 1]);
}

#[test]
fn generic_call_24_changes_a_clothing_category() {
    // Category 1 is formal wear, slot 0x1f (row 30, 0x40d599).
    let mut fx = dressed_sim();
    assert_eq!(generic_24(&mut fx, SIM, 1, 0), Ok(true));
    assert_eq!(row(&fx, 30), Some(FORMAL_0));
    assert_eq!(fx.sim().temps[..2], [1, 1]);
    // The other rows pass the commit's check, so they stay the profile's.
    for idx in [1, 2, 31, 32, 33, 34] {
        assert_eq!(row(&fx, idx), None, "row {idx}");
    }
}

#[test]
fn generic_call_24_writes_nothing_for_a_negative_or_unlisted_index() {
    // A negative temp 1 skips the category (0x44ffd0), an index past the
    // list's end fails its count (0x44fff2 and kin), and so does a
    // category past 5. Temp 1 is then left as it was (natively it gets
    // the low word of a pointer).
    for (category, index) in [(0, -1), (0, 2), (1, 2), (6, 0), (-1, 0)] {
        let mut fx = dressed_sim();
        assert_eq!(generic_24(&mut fx, SIM, category, index), Ok(true));
        assert_eq!(row(&fx, 1), None, "category {category} index {index}");
        assert_eq!(row(&fx, 30), None, "category {category} index {index}");
        assert_eq!(fx.sim().temps[..2], [category, index]);
    }
}

#[test]
fn generic_call_24_replaces_a_body_its_list_does_not_hold() {
    // The commit (0x40ab7f): a body row in no place of the list becomes
    // the first body (0x40ac34) once nothing is written.
    let mut fx = dressed_sim();
    fx.program.people.profiles.get_mut(&SIM_GUID).unwrap().body =
        Some("b100mafit_x,BODY=b100mafitlgt_gone".into());
    assert_eq!(generic_24(&mut fx, SIM, 0, -1), Ok(true));
    assert_eq!(row(&fx, 1), Some(BODY_0));
}

#[test]
fn generic_call_24_needs_a_person() {
    let mut fx = dressed_sim();
    assert_eq!(trap(generic_24(&mut fx, LAMP, 0, 1)).code(), Some(0x1c));
    assert!(fx.world.made_outfits.is_empty());
}

#[test]
fn generic_call_24_dresses_a_pet_from_its_species_list() {
    // A dog (gender code 2) takes bodies `b???dog` with no skin tone
    // (0x408bf5 → 0x6e488c); the commit writes NONE into rows 2 and 30
    // to 34 (0x40a5d5).
    let mut fx = Fixture::new();
    fx.program.people.eager_suits.insert("b100dog_x".into());
    fx.program.people.textures = ["b100dog_one", "b100dog_two"].map(String::from).to_vec();
    fx.program.people.profiles.insert(
        SIM_GUID,
        sims_content::Profile {
            body: Some("b100dog_x,BODY=b100dog_one".into()),
            gender: Some("dogmale".into()),
            age: Some("5".into()),
            ..Default::default()
        },
    );
    assert_eq!(generic_24(&mut fx, SIM, 0, 1), Ok(true));
    assert_eq!(row(&fx, 1), Some("b100dog_x,BODY=b100dog_two"));
    for idx in [2, 30, 31, 32, 33, 34] {
        assert_eq!(row(&fx, idx), Some("NONE"), "row {idx}");
    }
    assert_eq!(fx.sim().temps[..2], [0, 0]);
    // A pet's clothing categories are refused before anything changes.
    let mut fx2 = Fixture::new();
    fx2.program = fx.program.clone();
    let text = trap(generic_24(&mut fx2, SIM, 1, 0)).to_string();
    assert!(text.contains("category 1 for a pet"), "{text}");
    assert!(fx2.world.made_outfits.is_empty());
}
