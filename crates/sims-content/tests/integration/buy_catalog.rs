//! The Buy catalog's eligibility, catalog text and category labels, on
//! synthetic resources. No game data.

use crate::common::{Tree, far};
use sims_content::{
    BuyDefinition, BuyEligibility, BuyLabels, Catalog, CatalogText, Index, Install,
    has_interactions, master_of,
};
use sims_formats::iff::{ChunkData, Iff, write};

/// Little-endian resources (flag 0x10), as object files store them.
const LITTLE: u16 = 0x10;

fn chunk(tag: &[u8; 4], id: u16, label: &str, data: &[u8]) -> ChunkData {
    ChunkData {
        tag: *tag,
        id,
        flags: LITTLE,
        label: label.into(),
        data: data.to_vec(),
    }
}

/// An OBJD payload with the words the Buy catalog reads.
fn objd(guid: u32, words: &[(usize, u16)]) -> Vec<u8> {
    let mut bytes = vec![0; 216];
    bytes[..4].copy_from_slice(&138u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&guid.to_le_bytes());
    for &(offset, value) in words {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

const GRAPHICS: usize = 8;
const TABLE: usize = 0xe;
const KIND: usize = 0x12;
const GROUP: usize = 0x14;
const SUBINDEX: usize = 0x16;
const STRINGS: usize = 0x52;

fn definition(words: &[(usize, u16)]) -> BuyDefinition {
    BuyDefinition::parse(&objd(1, words)).unwrap()
}

/// A TTAB with no entries.
const EMPTY_TTAB: &[u8] = &[0, 0, 5, 0];

/// A version 5 TTAB with one entry and no motives.
fn one_entry_ttab() -> Vec<u8> {
    let mut bytes = vec![1, 0, 5, 0];
    bytes.extend([0; 4]); // action, test
    bytes.extend([0; 4 * 6]); // motives, flags, string, attenuation, autonomy, joining
    bytes
}

/// A version -3 table: (language code, string) pairs, each with an empty
/// comment.
fn localized(entries: &[(u8, &[u8])]) -> Vec<u8> {
    let mut bytes = (-3i16).to_le_bytes().to_vec();
    bytes.extend((entries.len() as u16).to_le_bytes());
    for (language, text) in entries {
        bytes.push(*language);
        bytes.extend(*text);
        bytes.extend([0, 0]);
    }
    bytes
}

#[test]
fn eligibility_follows_the_builders_order() {
    let person = definition(&[(KIND, 2), (GRAPHICS, 1)]);
    let tile = definition(&[(GROUP, 7), (SUBINDEX, 0), (GRAPHICS, 1)]);
    let master = definition(&[(GROUP, 7), (SUBINDEX, 0xffff)]);
    let drawn = definition(&[(GRAPHICS, 3)]);
    let bare = definition(&[]);
    assert_eq!(person.eligibility(), BuyEligibility::Excluded);
    assert_eq!(tile.eligibility(), BuyEligibility::Excluded);
    assert_eq!(master.eligibility(), BuyEligibility::Listed);
    assert_eq!(drawn.eligibility(), BuyEligibility::Listed);
    assert_eq!(bare.eligibility(), BuyEligibility::NeedsInteractions);
    // A person master is still excluded: the person test comes first.
    let person_master = definition(&[(KIND, 2), (GROUP, 7), (SUBINDEX, 0xffff)]);
    assert_eq!(person_master.eligibility(), BuyEligibility::Excluded);
    assert!(BuyDefinition::parse(&[0; 0x53]).is_err());
}

#[test]
fn interaction_table_ids_and_masters() {
    let own = definition(&[(TABLE, 9)]);
    assert_eq!(own.interaction_table(None), Some(9));
    assert_eq!(definition(&[(TABLE, 0)]).interaction_table(None), None);
    assert_eq!(
        definition(&[(TABLE, (-5i16) as u16)]).interaction_table(None),
        None
    );
    // -1 takes the master's word as it is.
    let tile = definition(&[(TABLE, 0xffff), (GROUP, 4), (SUBINDEX, 1)]);
    let master = definition(&[(TABLE, 12), (GROUP, 4), (SUBINDEX, 0xffff)]);
    assert_eq!(tile.interaction_table(Some(&master)), Some(12));
    assert_eq!(tile.interaction_table(None), None);
    let negative_master = definition(&[(TABLE, 0xffff), (GROUP, 4), (SUBINDEX, 0xffff)]);
    assert_eq!(tile.interaction_table(Some(&negative_master)), None);
    // A single-tile definition is its own master, so its -1 loads nothing.
    let single = definition(&[(TABLE, 0xffff)]);
    let found = master_of(&single, &0, [(&0, &master)]).unwrap();
    assert_eq!(found, &single);
    assert_eq!(single.interaction_table(Some(found)), None);
    // The first master of the same group in the same file.
    let other_group = definition(&[(TABLE, 30), (GROUP, 5), (SUBINDEX, 0xffff)]);
    let second = definition(&[(TABLE, 40), (GROUP, 4), (SUBINDEX, 0xffff)]);
    let candidates = [
        (&1, &master),
        (&0, &other_group),
        (&0, &tile),
        (&0, &second),
        (&0, &master),
    ];
    assert_eq!(master_of(&tile, &0, candidates), Some(&second));
    assert_eq!(master_of(&tile, &2, candidates), None);
}

#[test]
fn an_own_table_hides_the_semi_globals_even_when_empty() {
    let full = one_entry_ttab();
    assert!(has_interactions(Some(&full), None).unwrap());
    assert!(!has_interactions(Some(EMPTY_TTAB), Some(&full)).unwrap());
    assert!(has_interactions(None, Some(&full)).unwrap());
    assert!(!has_interactions(None, Some(EMPTY_TTAB)).unwrap());
    assert!(!has_interactions(None, None).unwrap());
    assert!(has_interactions(Some(&[1, 0]), None).is_err());
}

fn text(
    cats: Option<&[u8]>,
    ctss: Option<(&[u8], u16)>,
    language: u8,
    person: bool,
) -> (Vec<u8>, Vec<u8>) {
    let text = CatalogText::read(cats, ctss, b"Label\0tail", language, person).unwrap();
    (text.name, text.description)
}

#[test]
fn cats_holds_the_description_then_the_name() {
    assert_eq!(
        text(
            Some(b"About it\0Chair\0"),
            Some((b"ignored", LITTLE)),
            1,
            false
        ),
        (b"Chair".to_vec(), b"About it".to_vec())
    );
    // No second string before the end: the name falls back to the label.
    assert_eq!(
        text(Some(b"About it\0"), None, 1, false),
        (b"*Label".to_vec(), b"About it".to_vec())
    );
    assert_eq!(
        text(Some(b"About it\0\0"), None, 1, true),
        (b"*Label".to_vec(), b"About it".to_vec())
    );
    assert!(CatalogText::read(Some(b"unterminated"), None, b"", 1, false).is_err());
    assert!(CatalogText::read(Some(b"one\0two"), None, b"", 1, false).is_err());
}

#[test]
fn ctss_is_read_in_the_games_language_only() {
    let table = localized(&[(1, b"Chair"), (1, b"Sit on it"), (2, b"Chaise")]);
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, false),
        (b"Chair".to_vec(), b"Sit on it".to_vec())
    );
    // Language 2 has a name but no description; nothing falls back.
    assert_eq!(
        text(None, Some((&table, LITTLE)), 2, false),
        (b"Chaise".to_vec(), vec![])
    );
    // Language 3 has nothing: the label, not US English.
    assert_eq!(
        text(None, Some((&table, LITTLE)), 3, false),
        (b"*Label".to_vec(), vec![])
    );
    // A stored 0 is the game's language.
    let current = localized(&[(0, b"Any"), (0, b"Text")]);
    assert_eq!(
        text(None, Some((&current, LITTLE)), 5, false),
        (b"Any".to_vec(), b"Text".to_vec())
    );
    // A missing CTSS gives the label and no description.
    assert_eq!(text(None, None, 1, false), (b"*Label".to_vec(), vec![]));
    // Big-endian flags swap the leading word, so this table is unknown and
    // empty.
    assert_eq!(
        text(None, Some((&table, 0)), 1, false),
        (b"*Label".to_vec(), vec![])
    );
}

#[test]
fn a_languages_later_run_goes_in_front() {
    let table = localized(&[(1, b"A"), (1, b"B"), (2, b"X"), (1, b"C"), (1, b"D")]);
    // English is filed C, D, A, B.
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, false),
        (b"C".to_vec(), b"D".to_vec())
    );
}

#[test]
fn other_ctss_layouts() {
    // -1: NUL-terminated strings, every language.
    let mut terminated = (-1i16).to_le_bytes().to_vec();
    terminated.extend(2u16.to_le_bytes());
    terminated.extend(b"Lamp\0Bright\0");
    assert_eq!(
        text(None, Some((&terminated, LITTLE)), 4, false),
        (b"Lamp".to_vec(), b"Bright".to_vec())
    );
    // Big-endian -1 swaps its count too.
    let mut big = (-1i16).to_be_bytes().to_vec();
    big.extend(2u16.to_be_bytes());
    big.extend(b"Lamp\0Bright\0");
    assert_eq!(
        text(None, Some((&big, 0)), 1, false),
        (b"Lamp".to_vec(), b"Bright".to_vec())
    );
    // -2: each string has a comment.
    let mut commented = (-2i16).to_le_bytes().to_vec();
    commented.extend(2u16.to_le_bytes());
    commented.extend(b"Rug\0note\0Soft\0note\0");
    assert_eq!(
        text(None, Some((&commented, LITTLE)), 1, false),
        (b"Rug".to_vec(), b"Soft".to_vec())
    );
    // A positive leading word counts length-prefixed strings, with 0xa1
    // and 0xd5 changed; a NUL ends the copied string.
    let mut counted = 2i16.to_le_bytes().to_vec();
    counted.extend(b"\x04B\xa1x\xd5\x04Up\0x");
    assert_eq!(
        text(None, Some((&counted, LITTLE)), 1, false),
        (b"Box'".to_vec(), b"Up".to_vec())
    );
    // A leading word of 0 is an empty table.
    assert_eq!(
        text(None, Some((&[0, 0, 1, 0, 1, b'Z'], LITTLE)), 1, false),
        (b"*Label".to_vec(), vec![])
    );
    // A table that ends early is an error.
    let mut short = localized(&[(1, b"Chair")]);
    short.truncate(short.len() - 1);
    assert!(CatalogText::read(None, Some((&short, LITTLE)), b"", 1, false).is_err());
}

#[test]
fn a_nameless_person_takes_the_first_language_with_a_name() {
    let table = localized(&[(3, b"Third"), (2, b""), (4, b"Fourth")]);
    // Language 2's empty name is found first, so the label is used.
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, true),
        (b"*Label".to_vec(), vec![])
    );
    let table = localized(&[(4, b"Fourth"), (3, b"Third")]);
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, true),
        (b"Third".to_vec(), vec![])
    );
    // Not a person: the label.
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, false),
        (b"*Label".to_vec(), vec![])
    );
    // Language code 21 is past the probe.
    let table = localized(&[(21, b"Far")]);
    assert_eq!(
        text(None, Some((&table, LITTLE)), 1, true),
        (b"*Label".to_vec(), vec![])
    );
}

/// A synthetic UIText.iff: each table's strings are `T<id>.<n>` in
/// language `language`.
fn ui_text(language: u8, ids: impl IntoIterator<Item = (u16, usize)>) -> Vec<u8> {
    write(&ui_text_chunks(language, ids)).unwrap()
}

fn ui_text_chunks(language: u8, ids: impl IntoIterator<Item = (u16, usize)>) -> Vec<ChunkData> {
    ids.into_iter()
        .map(|(id, count)| {
            let strings: Vec<Vec<u8>> = (1..=count)
                .map(|n| format!("T{id}.{n}").into_bytes())
                .collect();
            let entries: Vec<(u8, &[u8])> = strings.iter().map(|s| (language, &s[..])).collect();
            chunk(b"STR#", id, "", &localized(&entries))
        })
        .collect()
}

fn all_tables() -> Vec<(u16, usize)> {
    let mut tables = vec![(150, 56), (210, 5)];
    tables.extend((200..=207).map(|id| (id, 4)));
    tables
}

#[test]
fn labels_read_their_tables_slots() {
    let iff = Iff::parse(ui_text(1, all_tables())).unwrap();
    let labels = BuyLabels::from_iff(&iff, 1).unwrap();
    let s = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
    // Subcatalog 0: each catalog's own row; 5 and 6 share row 5.
    assert_eq!(s(labels.label(0, 0, 0, 3)), "T150.4");
    assert_eq!(s(labels.label(4, 0, 0, 7)), "T150.40");
    assert_eq!(s(labels.label(5, 0, 0, 1)), "T150.42");
    assert_eq!(s(labels.label(6, 0, 0, 1)), "T150.42");
    // Catalog 6 remaps buttons 2 and 3.
    assert_eq!(s(labels.label(6, 0, 0, 2)), "T150.45");
    assert_eq!(s(labels.label(6, 0, 0, 3)), "T150.35");
    assert_eq!(s(labels.root(6, 2)), "T150.51");
    // In a subcatalog, catalog 1 reads its function row and STR# 210.
    assert_eq!(s(labels.label(1, 3, 2, 0)), "T202.1");
    assert_eq!(s(labels.label(1, 3, 7, 3)), "T207.4");
    let special: Vec<_> = (4..8).map(|c| s(labels.label(1, 3, 0, c))).collect();
    assert_eq!(special, ["T210.4", "T210.5", "T210.2", "T210.3"]);
    // Every other catalog reads row 1.
    assert_eq!(s(labels.label(3, 2, 0, 5)), "T150.14");
    assert!(BuyLabels::shown(1, 0, 5));
    assert!(BuyLabels::shown(3, 1, 5));
    assert!(!BuyLabels::shown(3, 0, 5));
    assert!(BuyLabels::shown(6, 0, 7));
    // Past the last catalog, row or column there is no button.
    assert!(!BuyLabels::shown(1, 1, 8));
    assert!(!BuyLabels::shown(7, 0, 0));
    assert_eq!(labels.label(1, 1, 0, 8), b"");
    assert_eq!(labels.label(1, 1, 8, 0), b"");
    assert_eq!(labels.label(7, 0, 0, 0), b"");
    assert_eq!(labels.root(7, 0), b"");
}

#[test]
fn labels_fall_back_to_us_english_only_for_a_language_with_no_strings() {
    let iff = Iff::parse(ui_text(1, all_tables())).unwrap();
    let german = BuyLabels::from_iff(&iff, 3).unwrap();
    assert_eq!(german.label(0, 0, 0, 0), b"T150.1");
    // A table with a few strings in the language does not fall back for
    // the ones it lacks.
    let mut chunks = ui_text_chunks(1, all_tables().into_iter().filter(|&(id, _)| id != 210));
    chunks.push(chunk(
        b"STR#",
        210,
        "",
        &localized(&[(1, b"e1"), (1, b"e2"), (3, b"g1"), (3, b"g2")]),
    ));
    let mixed = Iff::parse(write(&chunks).unwrap()).unwrap();
    let german = BuyLabels::from_iff(&mixed, 3).unwrap();
    assert_eq!(german.label(1, 1, 0, 6), b"g2");
    assert_eq!(german.label(1, 1, 0, 4), b"");
    // A missing table fails.
    let missing = Iff::parse(ui_text(1, [(150, 56)])).unwrap();
    assert!(BuyLabels::from_iff(&missing, 1).is_err());
}

#[test]
fn buy_entries_list_what_the_builder_keeps() {
    let tree = Tree::new("buy-entries");
    let semi = write(&[
        chunk(b"TTAB", 50, "", &one_entry_ttab()),
        chunk(b"TTAB", 51, "", &one_entry_ttab()),
    ])
    .unwrap();
    tree.file("GameData/Behavior.iff", b"").file(
        "GameData/Global/Global.far",
        &far(&[
            ("Global.iff", &write(&[]).unwrap()),
            ("ThingGlobals.iff", &semi),
        ]),
    );
    let cats = b"Old words\0Couch\0";
    let names = localized(&[(1, b"Lamp"), (1, b"Lights up")]);
    let object = write(&[
        // Drawn, named by CATS.
        chunk(
            b"OBJD",
            128,
            "couch",
            &objd(1, &[(GRAPHICS, 1), (STRINGS, 10)]),
        ),
        chunk(b"CATS", 10, "", cats),
        // Drawn, named by CTSS.
        chunk(
            b"OBJD",
            129,
            "lamp",
            &objd(2, &[(GRAPHICS, 1), (STRINGS, 11)]),
        ),
        chunk(b"CTSS", 11, "", &names),
        // No graphics; its own TTAB 50 is empty and hides the semi-global's.
        chunk(b"OBJD", 130, "hidden", &objd(3, &[(TABLE, 50)])),
        chunk(b"TTAB", 50, "", EMPTY_TTAB),
        // No graphics; TTAB 51 only in the semi-global, nameless.
        chunk(
            b"OBJD",
            131,
            "helper",
            &objd(4, &[(TABLE, 51), (STRINGS, 99)]),
        ),
        // A tile and a person, never listed.
        chunk(
            b"OBJD",
            132,
            "tile",
            &objd(5, &[(GROUP, 3), (SUBINDEX, 1), (GRAPHICS, 1)]),
        ),
        chunk(
            b"OBJD",
            133,
            "person",
            &objd(6, &[(KIND, 2), (GRAPHICS, 1)]),
        ),
        // A single-tile -1 table loads nothing.
        chunk(b"OBJD", 134, "orphan", &objd(7, &[(TABLE, 0xffff)])),
        chunk(b"GLOB", 1, "", b"\x0cThingGlobals"),
    ])
    .unwrap();
    tree.file(
        "GameData/Objects/Objects.far",
        &far(&[("Thing.iff", &object)]),
    );
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let buy = catalog.buy_entries(&index, 1).unwrap();
    assert!(buy.diagnostics.is_empty(), "{:?}", buy.diagnostics);
    let listed: Vec<_> = buy
        .entries
        .iter()
        .map(|e| {
            (
                catalog.objects[e.object].guid,
                String::from_utf8(e.text.name.clone()).unwrap(),
                String::from_utf8(e.text.description.clone()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            (1, "Couch".into(), "Old words".into()),
            (2, "Lamp".into(), "Lights up".into()),
            (4, "*helper".into(), String::new()),
        ]
    );
}

/// Needs the game: set `SIMS_INSTALL` to the install folder. Prints
/// counts only.
#[test]
fn installed_buy_catalog_reads_cleanly() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to list the install's Buy catalog");
        return;
    };
    let install = Install::detect(std::path::Path::new(&root)).unwrap();
    let index = crate::common::installed_index(&install);
    let catalog = Catalog::build(&index).unwrap();
    let buy = catalog.buy_entries(&index, 1).unwrap();
    assert!(buy.diagnostics.is_empty(), "{:?}", buy.diagnostics);
    let fallback = buy
        .entries
        .iter()
        .filter(|e| e.text.name.starts_with(b"*"))
        .count();
    eprintln!(
        "{} of {} definitions listed, {fallback} by label",
        buy.entries.len(),
        catalog.objects.len()
    );
    assert!(buy.entries.len() > 100);
    let labels = BuyLabels::load(&install, 1).unwrap();
    for catalog in 0..7 {
        for column in 0..8 {
            if BuyLabels::shown(catalog, 0, column) {
                assert!(
                    !labels.label(catalog, 0, 0, column).is_empty(),
                    "{catalog} {column}"
                );
            }
        }
    }
}
