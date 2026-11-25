//! Opening a template neighborhood: house 0's reload, then the import of
//! its `Patch` families (0x557fd3, 0x42c0ff), against the first
//! neighborhood, into which the original imported the same files.

use sims_content::family_import::{TEMPLATE_CAT, TEMPLATE_DOG};
use sims_content::{Catalog, Index, Install, Neighborhood, PENDING_IMPORT};
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::Iff;
use sims_formats::lot::GuidTable;
use sims_formats::neighborhood::{Family, Neighbor};
use sims_sim::neighborhood_save::SaveInputs;
use sims_sim::neighbors::{Registered, registry};
use sims_sim::patch_import::{Context, PETS, Patch, open};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn sorted(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn file_number(path: &Path) -> Option<i32> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    name.strip_prefix("user")?
        .strip_suffix(".iff")?
        .parse()
        .ok()
}

/// The GUID of each person type in the first neighborhood's
/// `Characters/User%05d.iff`, by file number.
fn character_guids(directory: &Path) -> BTreeMap<i32, u32> {
    let mut guids = BTreeMap::new();
    for path in sorted(&directory.join("Characters")) {
        let Some(number) = file_number(&path) else {
            continue;
        };
        let iff = Iff::parse(fs::read(&path).unwrap()).unwrap();
        let mut objds = iff.all(b"OBJD");
        let objd = iff.data(objds.next().unwrap());
        assert!(objds.next().is_none(), "{}", path.display());
        guids.insert(
            number,
            u32::from_le_bytes(objd[0x1c..0x20].try_into().unwrap()),
        );
    }
    guids
}

/// The template each of the first neighborhood's characters was copied
/// from, by its GUID.
fn character_templates(directory: &Path) -> BTreeMap<u32, u32> {
    let mut templates = BTreeMap::new();
    for path in sorted(&directory.join("Characters")) {
        if file_number(&path).is_none() {
            continue;
        }
        let iff = Iff::parse(fs::read(&path).unwrap()).unwrap();
        for chunk in iff.all(b"OBJD") {
            let objd = iff.data(chunk);
            let word = |at: usize| u32::from_le_bytes(objd[at..at + 4].try_into().unwrap());
            templates.insert(word(0x1c), word(0x8c));
        }
    }
    templates
}

fn guid_table(house: &Path) -> GuidTable {
    let iff = Iff::parse(fs::read(house).unwrap()).unwrap();
    GuidTable::parse(iff.data(iff.get(b"Gtab", 1).unwrap())).unwrap()
}

#[test]
fn opening_userdata2_imports_its_patch_families_as_the_first_neighborhood_has_them() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to open UserData2");
        return;
    };
    let root = PathBuf::from(root);
    let install = Install::detect(&root).unwrap();
    let first = root.join("UserData");
    let directory = root.join("UserData2");

    let mut index = crate::common::installed_index(&install);
    index.add_characters(&directory.join("Characters")).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let registry = registry(&index, &catalog);
    let character_mount = index
        .mounts
        .iter()
        .position(|mount| mount.path == Index::CHARACTERS)
        .unwrap();
    let mut neighborhood = Neighborhood::open(&directory).unwrap();
    assert!(neighborhood.options.unwrap()[PENDING_IMPORT]);
    let context = Context {
        catalog: &catalog,
        registry: &registry,
        character_files: sorted(&directory.join("Characters"))
            .iter()
            .filter_map(|path| file_number(path))
            .collect(),
        character_guids: catalog
            .objects
            .iter()
            .filter(|object| index.resources[object.resource].mount == character_mount)
            .map(|object| object.guid)
            .collect(),
        house_files: neighborhood.houses().unwrap().into_keys().collect(),
        zoning: neighborhood
            .zoning(u16::from(install.expansions.0))
            .unwrap(),
        lots: neighborhood.lot_rows().unwrap(),
        expansions: install.expansions.0,
        language: 1,
    };

    // The first neighborhood's files were imported in name order, the order
    // NTFS lists them: Burb's people are User00012, 16, 18 and 19, the
    // Charmings' 24 and 25, the Goths' 26 to 29, the Hicks' 30 to 34, the
    // Kats' 35 to 39 and the strays 40 to 47. Each housed family's GUID
    // table pairs its people's exported GUIDs with the new ones.
    let names = [
        "Burb_11.FAM",
        "Charming_13.FAM",
        "Goth Sr_12.FAM",
        "Hick_10.FAM",
        "Kat_6.FAM",
        "Strays_4000.FAM",
    ];
    let listed: Vec<String> = sorted(&directory.join("Patch"))
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(listed, names);
    let patches: Vec<Patch> = names
        .iter()
        .map(|name| Patch {
            name: (*name).to_owned(),
            file: FamilyFile::parse(&fs::read(directory.join("Patch").join(name)).unwrap())
                .unwrap(),
        })
        .collect();
    let first_characters = character_guids(&first);
    let mut original = Vec::new();
    for patch in &patches {
        let house = patch.file.family.house;
        if house != 0 {
            let table = guid_table(&first.join(format!("Houses/House{house:02}.iff")));
            original.extend(table.pairs.iter().map(|&(_, new)| new));
        } else {
            original.extend((40..48).map(|number| first_characters[&number]));
        }
    }
    let mut draws = original.clone().into_iter();
    let opened = open(&mut neighborhood, &patches, &context, &mut || {
        draws.next().expect("more draws than the original made")
    })
    .unwrap();
    assert!(draws.next().is_none());
    assert!(!neighborhood.options.unwrap()[PENDING_IMPORT]);

    // House 0's reload made the template pets' records.
    let made: BTreeSet<u32> = opened
        .made_records
        .iter()
        .map(|&id| {
            neighborhood.neighbors[id as usize - 1]
                .as_ref()
                .unwrap()
                .guid
        })
        .collect();
    assert!(made.contains(&TEMPLATE_DOG) && made.contains(&TEMPLATE_CAT));
    // The file's 26 empty slots, from the last, then new slots after its 49.
    assert_eq!(opened.made_records.len(), 270);
    assert_eq!(opened.made_records[..2], [46, 45]);
    assert_eq!(opened.made_records[26..28], [50, 51]);

    // The first neighborhood had more families and records before its
    // import: 8 more family numbers and 18 more record slots.
    const NUMBERS: i32 = 8;
    const SLOTS: i16 = 18;
    let first_neighborhood = Neighborhood::open(&first).unwrap();
    // The view's set-up made inventories for the one housed family's
    // records, 48 and 47; each housed import, for its people, in import
    // order. The first neighborhood's import made the same ones, SLOTS
    // further on, after its two housed families' own.
    let inventories: Vec<i16> = neighborhood
        .inventories
        .iter()
        .map(|inventory| {
            let inventory = inventory.as_ref().unwrap();
            assert!(inventory.items.is_empty());
            inventory.neighbor
        })
        .collect();
    assert_eq!(inventories[..2], [48, 47]);
    assert_eq!(inventories[2..], (294..314).collect::<Vec<_>>());
    let first_inventories: Vec<i16> = first_neighborhood
        .inventories
        .iter()
        .map(|inventory| inventory.as_ref().unwrap().neighbor)
        .collect();
    assert_eq!(
        first_inventories[first_inventories.len() - 20..],
        inventories[2..]
            .iter()
            .map(|&id| id + SLOTS)
            .collect::<Vec<_>>()
    );
    let first_records: BTreeMap<u32, _> = first_neighborhood
        .neighbors
        .iter()
        .flatten()
        .map(|record| (record.guid, record))
        .collect();
    let first_files: BTreeMap<u32, i32> = first_characters
        .iter()
        .map(|(&number, &guid)| (guid, number))
        .collect();
    let mut made_types = BTreeMap::new();
    // Compared as the open left the neighborhood, after every file.
    assert_eq!(opened.imports.len(), names.len());
    for (name, import) in &opened.imports {
        let ours = &neighborhood.families[&import.family_id];
        // Every housed family has its lot, with the same GUID table, byte
        // for byte.
        assert_eq!(import.lot.is_some(), ours.house != 0, "{name}");
        if let Some(lot) = &import.lot {
            let house = first.join(format!("Houses/House{:02}.iff", lot.house));
            let iff = Iff::parse(fs::read(&house).unwrap()).unwrap();
            assert_eq!(
                lot.chunks[b"Gtab"][0].data,
                iff.data(iff.get(b"Gtab", 1).unwrap()),
                "{name}"
            );
        }
        // The same family, under the same ID.
        let theirs = &first_neighborhood.families[&import.family_id];
        assert_eq!(
            (
                ours.version,
                ours.house,
                ours.budget,
                ours.architecture_value
            ),
            (
                theirs.version,
                theirs.house,
                theirs.budget,
                theirs.architecture_value
            ),
            "{name}"
        );
        assert_eq!(
            (ours.friends, ours.flags, &ours.members, &ours.tail),
            (theirs.friends, theirs.flags, &theirs.members, &theirs.tail),
            "{name}"
        );
        assert_eq!(ours.number + NUMBERS, theirs.number, "{name}");
        // The same records.
        assert_eq!(import.neighbors.len(), ours.members.len(), "{name}");
        for &id in import.neighbors.keys() {
            let record = neighborhood.neighbors[id as usize - 1].as_ref().unwrap();
            let theirs = first_records[&record.guid];
            assert_eq!(id, record.id);
            assert_eq!(id + SLOTS, theirs.id, "{name}");
            assert_eq!(
                (record.version, record.version_extra),
                (theirs.version, theirs.version_extra),
                "{name} {id}"
            );
            // Each is named for its own file: this neighborhood already
            // holds User00000 and User00001.
            let file = import.made[&record.guid].file;
            assert_eq!(record.name, format!("user{file:05}"), "{name} {id}");
            assert_eq!(
                theirs.name,
                format!("user{:05}", first_files[&record.guid]),
                "{name} {id}"
            );
            assert_eq!(
                (
                    record.unknown_zero,
                    record.person_mode,
                    record.unknown_negative
                ),
                (
                    theirs.unknown_zero,
                    theirs.person_mode,
                    theirs.unknown_negative
                ),
                "{name} {id}"
            );
            assert_eq!(record.person_data, theirs.person_data, "{name} {id}");
            let rows = |record: &Neighbor, offset: i32| -> Vec<(i32, Vec<i32>)> {
                record
                    .relationships
                    .iter()
                    .map(|row| (row.target + offset, row.values.clone()))
                    .collect()
            };
            assert_eq!(
                rows(record, i32::from(SLOTS)),
                rows(theirs, 0),
                "{name} {id}"
            );
        }
        made_types.extend(
            import
                .made
                .iter()
                .map(|(&guid, made)| (guid, made.template)),
        );
    }
    // Every made type is one of the first neighborhood's characters, copied
    // from the same template (OBJD byte 0x8c keeps the GUID it was copied
    // from, 0x4725e4).
    let templates = character_templates(&first);
    assert_eq!(made_types.len(), 28);
    for (guid, template) in made_types {
        assert_eq!(templates.get(&guid), Some(&template), "{guid:#010x}");
    }
}

const ANNE: u32 = 0xa;
const BOB: u32 = 0xb;
const PET: u32 = 0xc;

fn record(id: i16, guid: u32) -> Neighbor {
    Neighbor {
        version: 4,
        version_extra: None,
        name: String::new(),
        unknown_zero: 0,
        person_mode: 0,
        person_data: vec![0; 256],
        id,
        guid,
        unknown_negative: -1,
        relationships: Vec::new(),
    }
}

fn family(number: i32, members: &[u32]) -> Family {
    Family {
        version: 9,
        house: 0,
        number,
        budget: 0,
        architecture_value: 0,
        friends: 0,
        flags: 0,
        members: members.to_vec(),
        tail: Vec::new(),
    }
}

/// Anne, an empty slot, Bob; family 0, Anne's family 4000 and Bob's 7;
/// an import pending.
fn small() -> Neighborhood {
    Neighborhood {
        directory: PathBuf::new(),
        families: BTreeMap::from([
            (0, family(-1, &[])),
            (4000, family(1, &[ANNE])),
            (7, family(2, &[BOB, PET])),
        ]),
        type_attributes: None,
        neighbors: vec![Some(record(1, ANNE)), None, Some(record(3, BOB))],
        inventories: Vec::new(),
        data: [0; 16],
        options: Some([true; 11]),
    }
}

fn catalog() -> Catalog {
    Catalog {
        global: 0,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    }
}

fn registered(guid: u32, kind: u16) -> Registered {
    Registered {
        guid,
        kind,
        resolves: true,
    }
}

fn small_context<'a>(
    catalog: &'a Catalog,
    registry: &'a [Registered],
    expansions: u8,
) -> Context<'a> {
    Context {
        catalog,
        registry,
        character_files: BTreeSet::new(),
        character_guids: BTreeSet::new(),
        house_files: BTreeSet::new(),
        zoning: BTreeMap::new(),
        lots: Vec::new(),
        expansions,
        language: 1,
    }
}

fn no_draws() -> u32 {
    panic!("no file was imported")
}

#[test]
fn house_zeros_reload_makes_records_and_drops_members_without_one() {
    let catalog = catalog();
    // The pet type has no record: the reload makes one in the empty slot.
    // A type of a kind it does not record (a plain object) gets none.
    let registry = [
        registered(ANNE, 2),
        registered(BOB, 2),
        registered(PET, 2),
        registered(0xd, 0),
        registered(0xe, 0x200),
    ];
    let mut neighborhood = small();
    let opened = open(
        &mut neighborhood,
        &[],
        &small_context(&catalog, &registry, 0),
        &mut no_draws,
    )
    .unwrap();
    assert_eq!(opened.made_records, [2, 4]);
    let guids: Vec<_> = neighborhood
        .neighbors
        .iter()
        .map(|record| record.as_ref().map(|r| (r.id, r.guid)))
        .collect();
    assert_eq!(
        guids,
        [
            Some((1, ANNE)),
            Some((2, PET)),
            Some((3, BOB)),
            Some((4, 0xe))
        ]
    );
    // Family 7's pet had no record when the reload checked the members.
    assert_eq!(neighborhood.families[&7].members, [BOB]);
    // Word 61 names each member's family; the made records save none.
    let family_words: Vec<_> = neighborhood
        .neighbors
        .iter()
        .flatten()
        .map(|record| record.person_data.get(61).copied())
        .collect();
    assert_eq!(family_words, [Some(4000), None, Some(7), None]);
    // Without Unleashed, nothing is imported and the option stays.
    assert_eq!(opened.renumbered, None);
    assert!(neighborhood.options.unwrap()[PENDING_IMPORT]);
}

#[test]
fn freeing_family_4000_past_the_largest_signed_id_fails() {
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let mut neighborhood = small();
    // Family 7 as 32767, the largest signed ID.
    let family = neighborhood.families.remove(&7).unwrap();
    neighborhood.families.insert(32767, family);
    let error = open(
        &mut neighborhood,
        &[],
        &small_context(&catalog, &registry, PETS),
        &mut no_draws,
    )
    .unwrap_err();
    assert!(error.to_string().contains("0x42c1ad"), "{error}");
}

#[test]
fn a_pending_import_first_frees_family_4000_and_then_clears_the_option() {
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let mut neighborhood = small();
    let opened = open(
        &mut neighborhood,
        &[],
        &small_context(&catalog, &registry, PETS),
        &mut no_draws,
    )
    .unwrap();
    // The largest ID is 4000 itself, so the family becomes 4001.
    assert_eq!(opened.renumbered, Some((4000, 4001)));
    assert_eq!(
        neighborhood.families.keys().copied().collect::<Vec<_>>(),
        [0, 7, 4001]
    );
    assert_eq!(neighborhood.families[&4001].members, [ANNE]);
    assert!(!neighborhood.options.unwrap()[PENDING_IMPORT]);
}

#[test]
fn no_import_is_pending_once_the_option_is_clear() {
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let mut neighborhood = small();
    neighborhood.options.as_mut().unwrap()[PENDING_IMPORT] = false;
    let opened = open(
        &mut neighborhood,
        &[],
        &small_context(&catalog, &registry, PETS),
        &mut no_draws,
    )
    .unwrap();
    assert_eq!(opened.renumbered, None);
    assert!(neighborhood.families.contains_key(&4000));
}

#[test]
fn what_the_import_path_does_not_port_fails_loudly() {
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let context = small_context(&catalog, &registry, PETS);
    let fails = |change: &dyn Fn(&mut Neighborhood), address: &str| {
        let mut neighborhood = small();
        change(&mut neighborhood);
        let error = open(&mut neighborhood, &[], &context, &mut no_draws)
            .unwrap_err()
            .to_string();
        assert!(error.contains(address), "{error}");
    };
    fails(&|n| n.options = None, "0x4244bc");
    fails(&|n| n.neighbors.push(Some(record(4, 0x99))), "0x4259a2");
    fails(
        &|n| {
            n.families.remove(&0);
        },
        "0x425209",
    );
    fails(
        &|n| {
            n.families.insert(0x8000, family(3, &[BOB]));
        },
        "0x424bb6",
    );
    // The pet has no record, so its family is left empty.
    fails(
        &|n| {
            n.families.insert(9, family(3, &[PET]));
        },
        "0x425efb",
    );
    fails(
        &|n| {
            n.families.insert(9, family(3, &[ANNE]));
        },
        "0x425244",
    );
    fails(
        &|n| n.neighbors[0].as_mut().unwrap().person_data.clear(),
        "0x425244",
    );
}

/// A `Patch` file of a houseless family `id` whose `FAMI` lists `members`
/// and which exports nobody (`EXPi` names no member).
fn patch_listing(name: &str, id: u16, members: &[u32]) -> Patch {
    patch_file(name, id, members, None)
}

/// [`patch_listing`] with a FAMh, when `history` is given.
fn patch_file(name: &str, id: u16, members: &[u32], history: Option<&[u8]>) -> Patch {
    fn header(tag: &[u8; 4], version: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(version.to_le_bytes());
        bytes.extend(tag);
        bytes
    }
    fn chunk(file: &mut Vec<u8>, tag: &[u8; 4], id: u16, data: &[u8]) {
        file.extend(tag);
        file.extend(((76 + data.len()) as u32).to_be_bytes());
        file.extend(id.to_be_bytes());
        file.extend([0; 66]);
        file.extend(data);
    }
    let mut iff = vec![0; 64];
    iff[..13].copy_from_slice(b"IFF FILE 2.5:");
    let mut family = header(b"IMAF", 7);
    for word in [0, 0, 0, 0, 0, 0, members.len() as u32] {
        family.extend(word.to_le_bytes());
    }
    for guid in members {
        family.extend(guid.to_le_bytes());
    }
    chunk(&mut iff, b"FAMI", id, &family);
    let mut name_table = Vec::new();
    name_table.extend((-3_i16).to_le_bytes());
    name_table.extend(1u16.to_le_bytes());
    name_table.push(1);
    name_table.extend(b"Name\0\0");
    chunk(&mut iff, b"FAMs", id, &name_table);
    if let Some(history) = history {
        chunk(&mut iff, b"FAMh", id, history);
    }
    let mut export = header(b"iPXE", 0x3f);
    for word in [id, 0, 0, 0, 0, 0, 0, 0, 0] {
        export.extend(word.to_le_bytes());
    }
    export.push(1);
    chunk(&mut iff, b"EXPi", 0, &export);
    let mut neighbors = header(b"SRBN", 63);
    neighbors.extend(0u32.to_le_bytes());
    chunk(&mut iff, b"NBRS", 1, &neighbors);
    Patch {
        name: name.to_owned(),
        file: FamilyFile::parse(&iff).unwrap(),
    }
}

#[test]
fn the_preview_gets_an_inventory_for_a_member_who_has_a_record() {
    // The preview (0x4e29f5) counts the file's family's MagiCoins in the
    // open neighborhood (0x4e1b93 → 0x410282): Bob has a record (3) and no
    // inventory, so he gets an empty one at the end of the vector. The
    // member with no record is skipped.
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let mut neighborhood = small();
    neighborhood.families.remove(&4000);
    neighborhood.families.insert(8, family(1, &[ANNE]));
    let patch = patch_listing("Visitor_9.FAM", 9, &[0x0bad_0001, BOB]);
    let result = open(
        &mut neighborhood,
        std::slice::from_ref(&patch),
        &small_context(&catalog, &registry, PETS),
        &mut no_draws,
    );
    let inventories: Vec<(i16, usize)> = neighborhood
        .inventories
        .iter()
        .map(|inventory| {
            let inventory = inventory.as_ref().unwrap();
            (inventory.neighbor, inventory.items.len())
        })
        .collect();
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(inventories, [(3, 0)]);
}

/// The save after the import (0x4252cc) copies each imported family's
/// FAMh from its file (0x4291e6, 0x4107a8), with no fallback to a stored
/// history when the file has none, and the renumbered family 4000 keeps
/// its own under its new ID.
#[test]
fn imported_families_take_their_files_histories_and_family_4000_keeps_its_own() {
    let catalog = catalog();
    let registry = [registered(ANNE, 2), registered(BOB, 2), registered(PET, 2)];
    let mut neighborhood = small();
    let patches = [
        patch_file("Visitor_9.FAM", 9, &[0x0bad_0001], Some(&[9, 8, 7, 6])),
        patch_file("Other_5.FAM", 5, &[0x0bad_0002], None),
    ];
    let opened = open(
        &mut neighborhood,
        &patches,
        &small_context(&catalog, &registry, PETS),
        &mut no_draws,
    )
    .unwrap();
    assert_eq!(opened.renumbered, Some((4000, 4001)));
    let [(_, first), (_, second)] = &opened.imports[..] else {
        panic!("two imports: {:?}", opened.imports.len());
    };
    assert_ne!(first.family_id, second.family_id);

    let mut inputs = SaveInputs::default();
    inputs.histories.insert(4000, vec![0x40; 3]);
    inputs.family_names.insert(4000, "Strays ".to_owned());
    inputs.uninitialized.family_names.insert(4000, 0x11);
    inputs.histories.insert(7, vec![7]);
    // Left under the second import's ID by an earlier family.
    inputs.histories.insert(second.family_id, vec![0xee; 2]);
    inputs
        .family_names
        .insert(second.family_id, "Old".to_owned());
    opened.update_save_inputs(&mut inputs);

    assert_eq!(inputs.histories.get(&4000), None);
    assert_eq!(inputs.histories[&4001], [0x40; 3]);
    assert_eq!(inputs.family_names[&4001], "Strays ");
    assert_eq!(inputs.uninitialized.family_names[&4001], 0x11);
    assert_eq!(inputs.histories[&7], [7]);
    assert_eq!(inputs.histories[&first.family_id], [9, 8, 7, 6]);
    assert_eq!(inputs.histories.get(&second.family_id), None);
    assert_eq!(inputs.family_names.get(&second.family_id), None);
}
