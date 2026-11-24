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

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("sims-sim-save-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
            // A negative mode has person words too (0x423e8e).
            Some(Neighbor {
                person_mode: -1,
                ..neighbor(3, "Cy", 0x2000_0003)
            }),
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

/// Family 0 has no member list (0x4105e7): the import reads it without
/// one, and the save writes it the same way.
#[test]
fn family_zero_is_imported_and_saved_without_a_member_list() {
    let mut neighborhood = sample();
    // A secondary house of 2 and nonzero version 9 words: read with a
    // member list, these would be two members and a short tail.
    let mut tail = vec![0; 16];
    tail[0..4].copy_from_slice(&2i32.to_le_bytes());
    tail[4..8].copy_from_slice(&0x0001_0001u32.to_le_bytes());
    tail[12..16].copy_from_slice(&0x0002_0002u32.to_le_bytes());
    let zero = Family {
        tail,
        ..family(0, 2, Vec::new())
    };
    neighborhood.families.insert(0, zero.clone());
    let bytes = neighborhood
        .neighborhood_iff(&SaveInputs::default())
        .unwrap();
    let iff = Iff::parse(bytes.clone()).unwrap();
    let chunk = iff.data(iff.get(b"FAMI", 0).unwrap());
    assert_eq!(chunk, zero.to_chunk(0).unwrap());
    assert_eq!(Family::parse_chunk(chunk, 0).unwrap(), zero);

    let folder = TempDir::new("family-zero");
    std::fs::write(folder.0.join("Neighborhood.iff"), &bytes).unwrap();
    let db = TempDb::new("family-zero");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store.import_directory("Zero", &folder.0, 0xff).unwrap();
    let imported = store.load_neighborhood("Zero").unwrap().unwrap();
    assert_eq!(imported.families[&0], zero);
    assert_eq!(
        imported.neighborhood_iff(&SaveInputs::default()).unwrap(),
        bytes
    );
}

/// NGBH option bytes other than 0 and 1 survive the store, which keeps
/// only their truth, when the save is given the bytes it read.
#[test]
fn option_bytes_read_from_a_save_are_written_again() {
    let neighborhood = sample();
    let mut inputs = SaveInputs::default();
    let plain = neighborhood.neighborhood_iff(&inputs).unwrap();
    let (mut file, uninitialized) = NeighborhoodFile::parse(&plain).unwrap();
    file.neighborhood.option_bytes[0] = 2;
    let odd = file.to_bytes(&uninitialized).unwrap();
    assert_ne!(odd, plain);

    inputs = SaveInputs::from_saved(&odd).unwrap();
    assert_eq!(neighborhood.neighborhood_iff(&inputs).unwrap(), odd);

    // An option changed since the read is written as 0 or 1.
    let mut changed = neighborhood.clone();
    changed.options.as_mut().unwrap()[0] = false;
    let (file, _) = NeighborhoodFile::parse(&changed.neighborhood_iff(&inputs).unwrap()).unwrap();
    assert_eq!(file.neighborhood.option_bytes[0], 0);
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
