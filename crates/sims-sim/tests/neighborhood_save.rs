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
use sims_sim::neighborhood_save::{SAVE_INPUTS_FORMAT, SaveInputs, TEMPORARY_FILE};
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

/// A store from before format 6 holds family 0 read with a member list.
/// The migration reads it again, also after a world writeback padded its
/// tail, so the save writes what the original read.
#[test]
fn a_format_five_store_has_family_zero_read_again() {
    let mut tail = vec![0; 16];
    tail[0..4].copy_from_slice(&2i32.to_le_bytes());
    tail[4..8].copy_from_slice(&0x0001_0001u32.to_le_bytes());
    tail[8..12].copy_from_slice(&0x0003_0003u32.to_le_bytes());
    tail[12..16].copy_from_slice(&0x0002_0002u32.to_le_bytes());
    let zero = Family {
        tail,
        ..family(0, 2, Vec::new())
    };
    let chunk = zero.to_chunk(0).unwrap();
    let misread = Family::parse(&chunk).unwrap();
    assert_eq!(misread.members.len(), 2);
    let mut padded = misread.clone();
    padded.upgrade_to_v9();
    assert_ne!(padded.tail, misread.tail);

    for old in [misread, padded] {
        let mut neighborhood = sample();
        neighborhood.families.insert(0, old);
        let mut bytes = neighborhood.encode();
        bytes[..4].copy_from_slice(&5u32.to_le_bytes());
        let (back, report) = StoredNeighborhood::decode_with_report("Old", &bytes).unwrap();
        assert_eq!(report.steps_applied, 1);
        assert_eq!(back.families[&0].to_chunk(0).unwrap(), chunk);
        let saved = back.neighborhood_iff(&SaveInputs::default()).unwrap();
        let iff = Iff::parse(saved).unwrap();
        assert_eq!(iff.data(iff.get(b"FAMI", 0).unwrap()), chunk);
    }
}

/// NGBH option bytes other than 0 and 1, and family names with spaces
/// round them, survive the store, which keeps only the options' truth and
/// trimmed names, when the save is given what it read.
#[test]
fn option_bytes_and_names_read_from_a_save_are_written_again() {
    let neighborhood = sample();
    let mut inputs = SaveInputs::default();
    let plain = neighborhood.neighborhood_iff(&inputs).unwrap();
    let (mut file, uninitialized) = NeighborhoodFile::parse(&plain).unwrap();
    file.neighborhood.option_bytes[0] = 2;
    let odd = file.to_bytes(&uninitialized).unwrap();
    assert_ne!(odd, plain);

    inputs = SaveInputs::from_saved(&odd).unwrap();
    assert_eq!(neighborhood.neighborhood_iff(&inputs).unwrap(), odd);

    // A name read with spaces round it is written as read.
    let (mut file, uninitialized) = NeighborhoodFile::parse(&odd).unwrap();
    let alpha = file
        .families
        .iter_mut()
        .find(|record| record.id == 1)
        .unwrap();
    alpha.name = " Alpha ".to_owned();
    let spaced = file.to_bytes(&uninitialized).unwrap();
    let inputs = SaveInputs::from_saved(&spaced).unwrap();
    assert_eq!(neighborhood.neighborhood_iff(&inputs).unwrap(), spaced);
    let mut renamed = neighborhood.clone();
    renamed.set_family_surname(1, "Delta");
    let (file, _) = NeighborhoodFile::parse(&renamed.neighborhood_iff(&inputs).unwrap()).unwrap();
    let delta = file.families.iter().find(|record| record.id == 1).unwrap();
    assert_eq!(delta.name, "Delta");

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
    // Everything the save needs comes from the store: the record and the
    // save inputs the import kept.
    let out = TempDir::new("ud2-out");
    let path = store.save_neighborhood_iff("UD2", &out.0).unwrap();
    let written = std::fs::read(&path).unwrap();
    let first = written.iter().zip(&recorded).position(|(a, b)| a != b);
    assert!(
        written == recorded,
        "{} bytes written, {} recorded, first difference at {first:?}",
        written.len(),
        recorded.len()
    );
    assert_eq!(
        store.load_save_inputs("UD2").unwrap(),
        SaveInputs::from_saved(&recorded).unwrap()
    );
    assert!(!out.0.join(TEMPORARY_FILE).exists());
}

/// The import keeps each FAMh (and the rest of the save inputs) in the
/// store, and the store's save writes them back through `NTemp.iff`
/// onto `Neighborhood.iff`, replacing the old file.
#[test]
fn the_store_keeps_family_histories_and_saves_through_the_temporary_file() {
    let neighborhood = sample();
    let mut inputs = SaveInputs::default();
    inputs.histories.insert(1, vec![9, 8, 7, 6, 5]);
    inputs.histories.insert(3, vec![1, 2, 3, 4]);
    inputs.uninitialized.neighbor_names = vec![0x5a, 0xa5, 0x3c];
    inputs.uninitialized.family_names.insert(1, 0x77);
    let bytes = neighborhood.neighborhood_iff(&inputs).unwrap();

    let folder = TempDir::new("histories");
    std::fs::write(folder.0.join("Neighborhood.iff"), &bytes).unwrap();
    let db = TempDb::new("histories");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store.import_directory("Hood", &folder.0, 0xff).unwrap();
    let kept = store.load_save_inputs("Hood").unwrap();
    assert_eq!(kept.histories, inputs.histories);
    assert_eq!(kept, SaveInputs::from_saved(&bytes).unwrap());

    let out = TempDir::new("histories-out");
    std::fs::write(out.0.join("Neighborhood.iff"), b"old").unwrap();
    let path = store.save_neighborhood_iff("Hood", &out.0).unwrap();
    assert_eq!(path, out.0.join("Neighborhood.iff"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!out.0.join(TEMPORARY_FILE).exists());

    // A family that moves in from a `.FAM` file brings its FAMh.
    store
        .update_save_inputs("Hood", |inputs| {
            inputs.histories.insert(2, vec![0x42; 6]);
        })
        .unwrap();
    store.save_neighborhood_iff("Hood", &out.0).unwrap();
    let iff = Iff::parse(std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(iff.data(iff.get(b"FAMh", 2).unwrap()), [0x42; 6]);

    // Deleting the neighborhood forgets them; there is none to save to.
    assert!(store.delete_neighborhood("Hood").unwrap());
    assert_eq!(
        store.load_save_inputs("Hood").unwrap(),
        SaveInputs::default()
    );
    assert!(store.update_save_inputs("Hood", |_| {}).is_err());
    assert!(store.save_neighborhood_iff("Hood", &out.0).is_err());
}

/// Save inputs are stored with their version, and another version is
/// refused rather than misread.
#[test]
fn save_inputs_round_trip_and_refuse_another_version() {
    let mut inputs = SaveInputs::default();
    inputs.histories.insert(4, vec![1, 2, 3]);
    inputs.option_bytes = vec![0, 2, 1];
    inputs.family_names.insert(4, " Name ".to_owned());
    inputs.uninitialized.map_padding = vec![7, 7];
    let encoded = inputs.encode();
    assert_eq!(encoded[..4], SAVE_INPUTS_FORMAT.to_le_bytes());
    assert_eq!(SaveInputs::decode(&encoded).unwrap(), inputs);

    let mut other = encoded.clone();
    other[..4].copy_from_slice(&(SAVE_INPUTS_FORMAT + 1).to_le_bytes());
    assert!(SaveInputs::decode(&other).is_err());
    let mut trailing = encoded;
    trailing.push(0);
    assert!(SaveInputs::decode(&trailing).is_err());
    assert!(SaveInputs::decode(&[1, 0]).is_err());
}

/// A `Neighborhood.iff` the save did not write still gives its FAMh, its
/// names and their last bytes.
#[test]
fn a_file_the_save_did_not_write_gives_what_reads() {
    let mut name = vec![0xfd, 0xff, 1, 0, 1];
    name.extend(b"Rho\0\0");
    name.push(0x33);
    let iff = |chunks: &[(&[u8; 4], u16, &[u8])]| {
        let mut bytes =
            b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1\0\0\0\0".to_vec();
        for (tag, id, data) in chunks {
            bytes.extend(*tag);
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend(0x10u16.to_be_bytes());
            bytes.extend([0; 64]);
            bytes.extend(*data);
        }
        bytes
    };
    let bytes = iff(&[(b"FAMh", 6, &[5, 4, 3]), (b"FAMs", 6, &name)]);
    let inputs = SaveInputs::from_file(&bytes).unwrap();
    assert_eq!(inputs.histories[&6], [5, 4, 3]);
    assert_eq!(inputs.family_names[&6], "Rho");
    assert_eq!(inputs.uninitialized.family_names[&6], 0x33);
    assert!(inputs.option_bytes.is_empty());
}

/// A family the store drops takes its FAMh and name with it, so a family
/// that later takes its ID saves none of them.
#[test]
fn a_dropped_family_takes_its_history_and_name_with_it() {
    let mut inputs = SaveInputs::default();
    inputs.histories.insert(1, vec![1, 1]);
    inputs.histories.insert(3, vec![3, 3, 3]);
    inputs.family_names.insert(3, " Gamma ".to_owned());
    inputs.uninitialized.family_names.insert(3, 0x44);
    let bytes = sample().neighborhood_iff(&inputs).unwrap();

    let folder = TempDir::new("dropped");
    std::fs::write(folder.0.join("Neighborhood.iff"), &bytes).unwrap();
    let db = TempDb::new("dropped");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store.import_directory("Hood", &folder.0, 0xff).unwrap();
    assert!(
        store
            .load_save_inputs("Hood")
            .unwrap()
            .histories
            .contains_key(&3)
    );

    // Dropped: family 3's entries go; family 1's stay.
    let mut state = store.load_neighborhood("Hood").unwrap().unwrap();
    let gamma = state.families.remove(&3).unwrap();
    store.save_neighborhood("Hood", &state).unwrap();
    let kept = store.load_save_inputs("Hood").unwrap();
    assert!(!kept.histories.contains_key(&3));
    assert!(!kept.family_names.contains_key(&3));
    assert!(!kept.uninitialized.family_names.contains_key(&3));
    assert_eq!(kept.histories[&1], [1, 1]);

    // A new family 3 saves no FAMh.
    let mut state = store.load_neighborhood("Hood").unwrap().unwrap();
    state.families.insert(3, gamma);
    store.save_neighborhood("Hood", &state).unwrap();
    let out = TempDir::new("dropped-out");
    let path = store.save_neighborhood_iff("Hood", &out.0).unwrap();
    let iff = Iff::parse(std::fs::read(&path).unwrap()).unwrap();
    assert!(iff.get(b"FAMh", 3).is_none());
    assert!(iff.get(b"FAMh", 1).is_some());
}

/// A rename that fails (here onto a folder named `Neighborhood.iff`)
/// leaves no `NTemp.iff` behind.
#[test]
fn a_failed_save_leaves_no_temporary_file() {
    let out = TempDir::new("failed-save");
    std::fs::create_dir(out.0.join("Neighborhood.iff")).unwrap();
    let result = sample().save_neighborhood_iff(&out.0, &SaveInputs::default());
    assert!(result.is_err());
    assert!(!out.0.join(TEMPORARY_FILE).exists());
    assert!(out.0.join("Neighborhood.iff").is_dir());
}

/// A moved-in family's FAMh may be stored before the family is committed:
/// the commit that adds the family keeps it.
#[test]
fn a_history_stored_before_its_family_is_kept() {
    let bytes = sample().neighborhood_iff(&SaveInputs::default()).unwrap();
    let folder = TempDir::new("staged");
    std::fs::write(folder.0.join("Neighborhood.iff"), &bytes).unwrap();
    let db = TempDb::new("staged");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store.import_directory("Hood", &folder.0, 0xff).unwrap();

    store
        .update_save_inputs("Hood", |inputs| {
            inputs.histories.insert(7, vec![7; 5]);
        })
        .unwrap();
    let mut state = store.load_neighborhood("Hood").unwrap().unwrap();
    state.families.insert(7, family(0, 2, vec![0x1000_0007]));
    store.save_neighborhood("Hood", &state).unwrap();
    assert_eq!(
        store.load_save_inputs("Hood").unwrap().histories[&7],
        [7; 5]
    );
}

/// A `NTemp.iff` left as a link to `Neighborhood.iff` is replaced, not
/// written through.
#[cfg(unix)]
#[test]
fn a_linked_temporary_file_is_not_written_through() {
    let out = TempDir::new("linked");
    let target = out.0.join("Neighborhood.iff");
    std::fs::write(&target, b"old").unwrap();
    let elsewhere = out.0.join("Other.iff");
    std::fs::write(&elsewhere, b"other").unwrap();
    std::os::unix::fs::symlink(&elsewhere, out.0.join(TEMPORARY_FILE)).unwrap();
    let inputs = SaveInputs::default();
    let path = sample().save_neighborhood_iff(&out.0, &inputs).unwrap();
    assert_eq!(path, target);
    assert_eq!(std::fs::read(&elsewhere).unwrap(), b"other");
    assert_eq!(
        std::fs::read(&target).unwrap(),
        sample().neighborhood_iff(&inputs).unwrap()
    );
    assert!(!std::fs::symlink_metadata(&target).unwrap().is_symlink());
    assert!(std::fs::symlink_metadata(out.0.join(TEMPORARY_FILE)).is_err());
}
