//! A stored neighborhood saved as `Neighborhood.iff` (0x4252cc).
//!
//! The synthetic tests use made-up families, names and GUIDs. The parity
//! test reads `SIMS_UD2_AFTER`, a recorded neighborhood folder whose
//! `Neighborhood.iff` the original's save wrote last; it is local and never
//! committed, and without it that test does nothing.

use sims_formats::iff::Iff;
use sims_formats::neighborhood::{Family, Inventory, InventoryItem, Neighbor, Relationship};
use sims_formats::neighborhood_file::{NeighborhoodFile, parse_family_name};
use sims_formats::tatt::{Entry, Tatt};
use sims_sim::characters::Characters;
use sims_sim::neighborhood_save::SaveInputs;
use sims_sim::{NeighborhoodStore, StoredNeighborhood};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempDb(PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-sim-save-{label}-{}-{n}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn family(house: i32, number: i32, members: Vec<u32>) -> Family {
    Family {
        version: 9,
        house,
        number,
        budget: 20_000,
        architecture_value: 0,
        friends: 0,
        flags: 0,
        members,
        tail: vec![0; 16],
    }
}

fn neighbor(id: i16, name: &str, guid: u32) -> Neighbor {
    Neighbor {
        version: 10,
        version_extra: Some(9),
        name: name.to_owned(),
        unknown_zero: 0,
        person_mode: 1,
        person_data: (0..256).map(|i| i as i16 - 7).collect(),
        id,
        guid,
        unknown_negative: -1,
        relationships: vec![Relationship {
            target: 2,
            values: vec![40, -12],
        }],
    }
}

fn sample() -> StoredNeighborhood {
    let mut families = BTreeMap::new();
    families.insert(1, family(5, 1, vec![0x1000_0001, 0x1000_0002]));
    families.insert(2, family(0, 1, vec![0x1000_0003]));
    families.insert(3, family(7, 4, vec![0x1000_0004]));
    let mut family_surnames = BTreeMap::new();
    family_surnames.insert(1, "Alpha".to_owned());
    family_surnames.insert(3, "Gamma".to_owned());
    StoredNeighborhood {
        name: "Synthetic".to_owned(),
        families,
        family_surnames,
        neighbors: vec![
            None,
            Some(neighbor(1, "Ana", 0x2000_0001)),
            Some(neighbor(2, "Bo", 0x2000_0002)),
        ],
        inventories: vec![
            None,
            Some(Inventory {
                neighbor: 1,
                items: vec![InventoryItem {
                    kind: 1,
                    guid: 0x3000_0001,
                    count: 2,
                }],
            }),
        ],
        data: [3, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9],
        options: Some([
            true, false, true, false, true, false, true, false, true, false, true,
        ]),
        type_attributes: Some(Tatt {
            version: 1,
            packed: true,
            entries: vec![Entry {
                guid: 0x4000_0001,
                values: vec![0, -1, 15, 300, -32768],
                count_end: 0,
                end: 0,
            }],
            count_end: 0,
            consumed: 0,
        }),
        zoning: BTreeMap::new(),
        lot_rows: Vec::new(),
        made_characters: Characters::default(),
        deleted_characters: BTreeSet::new(),
        deletion_epoch: 0,
        deleted_at_epoch: BTreeMap::new(),
        instance_id: 1,
        revision: 0,
    }
}

/// The family vector's order (0x425209): number descending, then ID
/// descending, the ID compared as a signed word.
#[test]
fn families_are_saved_by_number_then_id_descending() {
    let mut neighborhood = sample();
    neighborhood
        .families
        .insert(0xffff, family(0, 4, Vec::new()));
    neighborhood
        .families
        .insert(0x7fff, family(0, 1, Vec::new()));
    assert_eq!(neighborhood.family_order(), [3, 0xffff, 0x7fff, 2, 1]);
}

/// Everything the store keeps comes back from the saved file.
#[test]
fn a_saved_neighborhood_reads_back_as_stored() {
    let neighborhood = sample();
    let mut inputs = SaveInputs::default();
    inputs.histories.insert(3, vec![1, 2, 3, 4]);
    let bytes = neighborhood.neighborhood_iff(&inputs).unwrap();
    let (file, _) = NeighborhoodFile::parse(&bytes).unwrap();

    assert_eq!(file.neighborhood.data, neighborhood.data);
    assert_eq!(file.neighborhood.inventories, neighborhood.inventories);
    assert_eq!(Some(file.neighborhood.options), neighborhood.options);
    assert_eq!(file.neighbors.entries, neighborhood.neighbors);
    let tatt = file.type_attributes;
    let stored = neighborhood.type_attributes.as_ref().unwrap();
    assert_eq!(tatt.entries.len(), 1);
    assert_eq!(tatt.entries[0].guid, stored.entries[0].guid);
    assert_eq!(tatt.entries[0].values, stored.entries[0].values);

    let ids: Vec<_> = file.families.iter().map(|record| record.id).collect();
    assert_eq!(ids, [3, 2, 1]);
    for record in &file.families {
        assert_eq!(record.family, neighborhood.families[&record.id]);
        assert_eq!(
            record.name,
            neighborhood.family_surname(record.id).unwrap_or_default()
        );
        assert_eq!(record.history.as_ref(), inputs.histories.get(&record.id));
    }

    // The same inputs give the same bytes again.
    let again = SaveInputs::from_saved(&bytes).unwrap();
    assert_eq!(neighborhood.neighborhood_iff(&again).unwrap(), bytes);
}

/// A family with ID -1 is never written (0x4106a0), and one without a
/// surname gets an empty name table.
#[test]
fn the_unset_family_is_skipped_and_a_nameless_one_has_an_empty_name() {
    let mut neighborhood = sample();
    neighborhood
        .families
        .insert(0xffff, family(0, 9, Vec::new()));
    let bytes = neighborhood
        .neighborhood_iff(&SaveInputs::default())
        .unwrap();
    let iff = Iff::parse(bytes).unwrap();
    let ids: Vec<_> = iff.all(b"FAMI").map(|chunk| chunk.id).collect();
    assert_eq!(ids, [3, 2, 1]);
    let name = iff.get(b"FAMs", 2).unwrap();
    assert_eq!(
        parse_family_name(iff.data(name)).unwrap(),
        (String::new(), 0)
    );
    assert!(iff.all(b"FAMh").next().is_none());
}

/// The import reads family 0 with a member list, which the original's
/// reader skips for that ID (0x4105e7); the save writes it as the
/// original would have read it.
#[test]
fn family_zero_read_with_a_member_list_is_saved_without_one() {
    let mut neighborhood = sample();
    // As the original wrote it: no member list, so the secondary house
    // (0) and the version 9 words follow the fixed fields.
    let native = family(0, 2, Vec::new());
    let chunk = native.to_chunk(0).unwrap();
    // As the import reads it: the secondary house taken for a count.
    let misread = Family::parse(&chunk).unwrap();
    assert_ne!(misread, native);
    neighborhood.families.insert(0, misread);
    let bytes = neighborhood
        .neighborhood_iff(&SaveInputs::default())
        .unwrap();
    let iff = Iff::parse(bytes).unwrap();
    let saved = iff.get(b"FAMI", 0).unwrap();
    assert_eq!(iff.data(saved), chunk);
}

/// A neighborhood with no NGBH options or TATT read still saves, with the
/// reader's defaults.
#[test]
fn missing_options_and_type_attributes_save_as_the_defaults() {
    let mut neighborhood = sample();
    neighborhood.options = None;
    neighborhood.type_attributes = None;
    let bytes = neighborhood
        .neighborhood_iff(&SaveInputs::default())
        .unwrap();
    let (file, _) = NeighborhoodFile::parse(&bytes).unwrap();
    assert!(file.type_attributes.entries.is_empty());
    assert_eq!(
        file.neighborhood.options,
        [
            false, true, false, true, false, true, false, true, false, true, true
        ]
    );
}

/// The recorded save, imported into the store and saved from it, gives
/// the recorded file back.
#[test]
fn an_imported_save_is_saved_again_byte_for_byte() {
    let Some(folder) = std::env::var_os("SIMS_UD2_AFTER").map(PathBuf::from) else {
        eprintln!("SIMS_UD2_AFTER not set; skipping");
        return;
    };
    let recorded = std::fs::read(folder.join("Neighborhood.iff")).unwrap();
    let db = TempDb::new("ud2");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store.import_directory("UD2", &folder, 0xff).unwrap();
    let neighborhood = store.load_neighborhood("UD2").unwrap().unwrap();
    let inputs = SaveInputs::from_saved(&recorded).unwrap();
    let written = neighborhood.neighborhood_iff(&inputs).unwrap();
    let first = written.iter().zip(&recorded).position(|(a, b)| a != b);
    assert!(
        written == recorded,
        "{} bytes written, {} recorded, first difference at {first:?}",
        written.len(),
        recorded.len()
    );
}
