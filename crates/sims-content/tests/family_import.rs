mod common;

use sims_content::family_import::{
    ImportInput, ImportOptions, TEMPLATE_CAT, TEMPLATE_DOG, TEMPLATE_PERSON, clone_character,
    import, refresh_outfits,
};
use sims_content::{Catalog, ImportWardrobe, Index, Install, Neighborhood, Object};
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::{Chunk, Iff, write};
use sims_formats::lot::{GuidTable, ObjectTypes};
use sims_formats::neighborhood::{Family, Neighbor};
use sims_formats::strings::StringTable;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const PERSON: u32 = 0x1111_1111;
const DOG: u32 = 0x2222_2222;
const FIXTURE: u32 = 0x3333_3333;
const OLD_DOG: u32 = 0x4444_4444;
const MADE_DOG: u32 = 0x1000_0000;
const MADE_PERSON: u32 = MADE_DOG + 1;
const PETS: ImportOptions = ImportOptions {
    pets_enabled: true,
    language: 1,
};

fn words(bytes: &mut Vec<u8>, values: &[u32]) {
    for word in values {
        bytes.extend(word.to_le_bytes());
    }
}

fn shorts(bytes: &mut Vec<u8>, values: &[u16]) {
    for word in values {
        bytes.extend(word.to_le_bytes());
    }
}

fn saved_header(tag: &[u8; 4], version: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    words(&mut bytes, &[0, version]);
    bytes.extend(tag);
    bytes
}

fn text(entries: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    shorts(&mut bytes, &[(-3_i16) as u16, entries.len() as u16]);
    for entry in entries {
        bytes.push(1);
        bytes.extend(entry.as_bytes());
        bytes.extend([0, 0]);
    }
    bytes
}

fn chunk(file: &mut Vec<u8>, tag: &[u8; 4], id: u16, data: &[u8]) {
    file.extend(tag);
    file.extend(((76 + data.len()) as u32).to_be_bytes());
    file.extend(id.to_be_bytes());
    file.extend([0; 66]);
    file.extend(data);
}

fn synthetic_file() -> FamilyFile {
    let mut iff = vec![0; 64];
    iff[..13].copy_from_slice(b"IFF FILE 2.5:");
    let mut family = saved_header(b"IMAF", 7);
    words(&mut family, &[42, 17, 200, 300, 99, 17, 2, PERSON, DOG]);
    chunk(&mut iff, b"FAMI", 17, &family);
    chunk(&mut iff, b"FAMs", 17, &text(&["Export Name"]));
    chunk(&mut iff, b"CTSS", 1000, &text(&["Family Name"]));
    chunk(&mut iff, b"FAMh", 17, b"history");

    let mut export = saved_header(b"iPXE", 0x3f);
    shorts(&mut export, &[17, 2, 1, 0, 0, 0, 0, 0, 0]);
    export.push(1);
    chunk(&mut iff, b"EXPi", 0, &export);

    let mut neighbors = saved_header(b"SRBN", 63);
    words(&mut neighbors, &[2]);
    for (slot, guid, gender) in [(1, PERSON, 0), (2, DOG, 3)] {
        words(&mut neighbors, &[1, 4]);
        neighbors.extend(b"exported\0\0");
        words(&mut neighbors, &[42, 7]); // Old private field, person revision.
        let mut data = [0; 80];
        data[2] = 700;
        data[31] = 123;
        data[61] = 17;
        data[65] = gender;
        data[75] = 20;
        shorts(&mut neighbors, &data);
        // The relationship key comes from EXPi, independent of the serialized ID.
        shorts(&mut neighbors, &[slot + 40]);
        words(&mut neighbors, &[guid, u32::MAX, 2]);
        words(&mut neighbors, &[1, 3 - u32::from(slot), 2, 35, 15]);
        words(&mut neighbors, &[1, 23, 1, 98]);
    }
    chunk(&mut iff, b"NBRS", 1, &neighbors);

    for (id, skeleton, body, gender) in [
        (1, "adult", "person-body", "female"),
        (2, "dog", "dog-body", "dogfemale"),
    ] {
        let mut strings = [""; 13];
        strings[0] = skeleton;
        strings[1] = body;
        strings[12] = gender;
        chunk(&mut iff, b"uChr", id, &text(&strings));
    }
    chunk(&mut iff, b"CTSS", 1, &text(&["Person", "Person bio"]));
    chunk(&mut iff, b"CTSS", 2, &text(&["Fallback dog"]));
    chunk(&mut iff, b"CTSS", 2002, &text(&["Dog", "Dog bio"]));

    let mut types = saved_header(b"tjbo", 3);
    for (id, guid, kind) in [(5, PERSON, 2), (7, OLD_DOG, 2), (9, FIXTURE, 0)] {
        words(&mut types, &[guid, 19, 31]);
        shorts(&mut types, &[id, kind]);
        types.extend(b"Type\0\0");
        words(&mut types, &[73]);
    }
    chunk(&mut iff, b"objt", 0, &types);
    let mut gtab = saved_header(b"batG", 0);
    words(&mut gtab, &[1, OLD_DOG, DOG]);
    chunk(&mut iff, b"Gtab", 1, &gtab);
    chunk(&mut iff, b"ObjM", 1, b"lot payload");
    FamilyFile::parse(&iff).unwrap()
}

fn template_record(id: i16, guid: u32) -> Neighbor {
    Neighbor {
        version: 4,
        version_extra: None,
        name: String::new(),
        unknown_zero: 0,
        person_mode: 0,
        person_data: Vec::new(),
        id,
        guid,
        unknown_negative: -1,
        relationships: Vec::new(),
    }
}

fn synthetic_catalog() -> Catalog {
    let mut catalog = Catalog {
        global: 0,
        objects: Vec::new(),
        by_guid: BTreeMap::new(),
        semi_globals: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    for (resource, guid) in [TEMPLATE_PERSON, TEMPLATE_DOG].into_iter().enumerate() {
        catalog.objects.push(Object {
            guid,
            resource,
            chunk: 1,
            kind: 2,
            master: 0,
        });
        catalog.by_guid.insert(guid, resource);
    }
    catalog
}

#[test]
fn clones_members_in_export_order_and_translates_their_links() {
    let file = synthetic_file();
    let catalog = synthetic_catalog();
    let neighbors = [
        Some(template_record(1, TEMPLATE_PERSON)),
        Some(template_record(2, TEMPLATE_DOG)),
        None,
        Some(template_record(4, FIXTURE)),
        None,
    ];
    let mut prior_family: Family = file.family.clone();
    prior_family.members.clear();
    let input = ImportInput {
        catalog: &catalog,
        neighbors: &neighbors,
        families: &BTreeMap::from([(17, prior_family)]),
        next_file: 31_998,
        character_files: &BTreeSet::from([31_998, 0]),
        character_guids: &BTreeSet::new(),
        house_files: &BTreeSet::from([42]),
        zoning: &BTreeMap::from([(42, 1)]),
        neighborhood_data: [42; 16],
    };
    let mut guids = MADE_DOG..;
    let imported = import(&input, &file, PETS, &mut || guids.next().unwrap()).unwrap();
    assert_eq!(guids.next(), Some(MADE_PERSON + 1));
    assert_eq!(imported.next_file, 2);
    let made = &imported.made;
    assert_eq!(made[&MADE_DOG].template, TEMPLATE_DOG);
    assert_eq!(made[&MADE_DOG].strings[&1], "dog-body");
    assert_eq!(made[&MADE_DOG].file, 31_999);
    assert_eq!(made[&MADE_DOG].name, "Dog");
    assert_eq!(made[&MADE_DOG].description, "Dog bio");
    assert_eq!(made[&MADE_PERSON].template, TEMPLATE_PERSON);
    assert_eq!(made[&MADE_PERSON].strings[&0], "adult");
    assert_eq!(made[&MADE_PERSON].strings[&1], "person-body");
    assert_eq!(made[&MADE_PERSON].file, 1);
    assert_eq!(made[&MADE_PERSON].name, "Person");
    assert_eq!(made[&MADE_PERSON].description, "Person bio");

    assert_eq!(imported.neighbor_ids, BTreeMap::from([(2, 5), (1, 3)]));
    assert_eq!(
        imported.guid_map,
        BTreeMap::from([(PERSON, MADE_PERSON), (DOG, MADE_DOG)])
    );
    for (id, target, guid, gender, name) in [
        (3, 5, MADE_PERSON, 0, "user00001"),
        (5, 3, MADE_DOG, 3, "user31999"),
    ] {
        let record = &imported.neighbors[&id];
        assert_eq!((record.id, record.guid), (id, guid));
        assert_eq!(record.name, name);
        assert_eq!(record.unknown_zero, 0);
        assert_eq!(record.person_mode, 7);
        assert_eq!(record.person_data[2], 700);
        assert_eq!(record.person_data[31], 0);
        assert_eq!(record.person_data[61], 1);
        assert_eq!(record.person_data[65], gender);
        assert_eq!(record.person_data[75], 20);
        assert_eq!(record.relationships.len(), 1);
        assert_eq!(record.relationships[0].target, target);
        assert_eq!(record.relationships[0].values, [35, 15]);
    }
    assert_eq!(imported.family_id, 1);
    assert_eq!(imported.family.number, 18);
    assert_eq!(imported.family.budget, 200);
    assert_eq!(imported.family.architecture_value, 300);
    assert_eq!(imported.family.flags, 1);
    assert_eq!(imported.family.friends, 0);
    assert_eq!(imported.family.members, [MADE_DOG, MADE_PERSON]);
    assert_eq!(imported.family_name, "Family Name");
    assert_eq!(
        imported.family_history.as_deref(),
        Some(b"history".as_slice())
    );
    assert_eq!(imported.neighborhood_data[2..4], [0, 42]);
    let (displaced_id, displaced) = imported.displaced_family.unwrap();
    assert_eq!(
        (displaced_id, displaced.house, displaced.architecture_value),
        (17, 0, 0)
    );

    let lot = imported.lot.unwrap();
    assert_eq!(lot.house, 42);
    let resolved: Vec<_> = lot
        .object_types
        .entries
        .iter()
        .map(|entry| (entry.id, entry.guid))
        .collect();
    assert_eq!(resolved, [(5, MADE_PERSON), (7, MADE_DOG), (9, FIXTURE)]);
    assert_eq!(lot.chunks[b"ObjM"][0].data, b"lot payload");
    let gtab = &lot.chunks[b"Gtab"][0];
    assert_eq!(gtab.id, 1);
    let mut expected = saved_header(b"batG", 0);
    words(
        &mut expected,
        &[3, OLD_DOG, DOG, DOG, MADE_DOG, PERSON, MADE_PERSON],
    );
    assert_eq!(gtab.data, expected);
    assert_eq!(
        ObjectTypes::parse(&lot.chunks[b"objt"][0].data)
            .unwrap()
            .entries[0]
            .guid,
        PERSON
    );
}

#[test]
fn missing_template_record_and_disabled_pets_report_native_addresses() {
    let file = synthetic_file();
    let catalog = synthetic_catalog();
    let input = ImportInput {
        catalog: &catalog,
        neighbors: &[Some(template_record(1, TEMPLATE_PERSON))],
        families: &BTreeMap::new(),
        next_file: 0,
        character_files: &BTreeSet::new(),
        character_guids: &BTreeSet::new(),
        house_files: &BTreeSet::from([42]),
        zoning: &BTreeMap::from([(42, 1)]),
        neighborhood_data: [0; 16],
    };
    let mut guid = || panic!("a rejected template should consume no GUID");
    let message = import(&input, &file, PETS, &mut guid)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("0x426335 missing template record"),
        "{message}"
    );
    let message = import(&input, &file, ImportOptions::default(), &mut guid)
        .unwrap_err()
        .to_string();
    assert!(message.contains("0x42601e drop of a pet"), "{message}");
}

#[test]
fn cleanup_reads_the_lot_word_of_displaced_records() {
    let file = synthetic_file();
    let catalog = synthetic_catalog();
    let mut orphan = template_record(3, FIXTURE);
    orphan.person_data = vec![0; 80];
    orphan.person_data[67] = 42;
    let input = ImportInput {
        catalog: &catalog,
        neighbors: &[
            Some(template_record(1, TEMPLATE_PERSON)),
            Some(template_record(2, TEMPLATE_DOG)),
            Some(orphan),
        ],
        families: &BTreeMap::new(),
        next_file: 0,
        character_files: &BTreeSet::new(),
        character_guids: &BTreeSet::from([FIXTURE]),
        house_files: &BTreeSet::from([42]),
        zoning: &BTreeMap::from([(42, 1)]),
        neighborhood_data: [0; 16],
    };
    let message = import(&input, &file, PETS, &mut || {
        panic!("rejected before copying")
    })
    .unwrap_err()
    .to_string();
    assert!(
        message.contains("0x426fcd cleanup of former residents"),
        "{message}"
    );
}

#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn paths(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn read_iff(path: &Path) -> Iff {
    Iff::parse(fs::read(path).unwrap()).unwrap()
}

fn character_guid(iff: &Iff) -> u32 {
    let definitions = iff.resource_order(b"OBJD").unwrap();
    assert_eq!(definitions.len(), 1);
    u32::from_le_bytes(iff.data(definitions[0])[0x1c..0x20].try_into().unwrap())
}

fn comparable_resources(iff: &Iff) -> BTreeMap<([u8; 4], u16), &Chunk> {
    let tags: BTreeSet<_> = iff.chunks.iter().map(|chunk| chunk.tag).collect();
    let mut resources = BTreeMap::new();
    for tag in tags {
        for chunk in iff.resource_order(&tag).unwrap() {
            if tag == *b"XXXX" || (tag == *b"BMP_" && (2002..=2007).contains(&chunk.id)) {
                continue;
            }
            assert!(resources.insert((tag, chunk.id), chunk).is_none());
        }
    }
    resources
}

fn string_rows(data: &[u8]) -> Vec<(u8, Vec<u8>, Vec<u8>)> {
    StringTable::parse(data)
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| (entry.language, entry.raw_text, entry.raw_comment))
        .collect()
}

fn compare_clone(clone: &Iff, oracle: &Iff, number: i32) {
    assert!(
        clone
            .all(b"BMP_")
            .all(|chunk| !(2002..=2007).contains(&chunk.id))
    );
    let actual = comparable_resources(clone);
    let expected = comparable_resources(oracle);
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (key, chunk) in actual {
        let original = expected[&key];
        let resource_name = format!("User{number:05} {} #{}", chunk.kind(), chunk.id);
        assert_eq!(
            (chunk.flags, &chunk.label),
            (original.flags, &original.label),
            "{resource_name}"
        );
        let bytes = clone.data(chunk);
        let original = oracle.data(original);
        match (&key.0, key.1) {
            (b"STR#", 200) => {
                let rows = string_rows(bytes);
                let saved = string_rows(original);
                assert_eq!((rows.len(), saved.len()), (35, 35), "{resource_name}");
                for (index, (row, saved)) in rows.iter().zip(&saved).enumerate() {
                    assert_eq!(row, saved, "{resource_name} row {}", index + 1);
                }
                // 0x46fc9f leaves one unused byte per declared row.
                assert_eq!(bytes.len(), original.len(), "{resource_name} size");
                let end = bytes.len() - rows.len();
                assert_eq!(&bytes[..end], &original[..end], "{resource_name}");
                assert!(bytes[end..].iter().all(|&byte| byte == 0));
            }
            (b"CTSS", 2000) => {
                // 0x47090e allocates one unused byte per declared row.
                let rows = string_rows(original).len();
                assert_eq!(bytes.len(), original.len(), "{resource_name}");
                let end = original.len() - rows;
                assert_eq!(&bytes[..end], &original[..end], "{resource_name}");
                assert!(bytes[end..].iter().all(|&byte| byte == 0));
            }
            _ => assert!(bytes == original, "{resource_name}: payload differs"),
        }
    }
}

#[test]
fn imports_and_writes_the_complete_collections_userdata2_patches() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to import UserData2's Patch families");
        return;
    };
    let root = PathBuf::from(root);
    let directory = root.join("UserData2");
    let first = root.join("UserData");
    let originals: BTreeMap<_, _> = (12..48)
        .map(|number| {
            let iff = read_iff(&first.join(format!("Characters/User{number:05}.iff")));
            (character_guid(&iff), (number, iff))
        })
        .collect();
    assert_eq!(originals.len(), 36);
    let stray_guids: BTreeMap<_, _> = originals
        .iter()
        .filter_map(|(&guid, &(number, _))| (number >= 40).then_some((number, guid)))
        .collect();
    let install = Install::detect(&root).unwrap();
    let wardrobe = ImportWardrobe::load(&install).unwrap();
    let mut index = Index::build(&install).unwrap();
    index.add_characters(&directory.join("Characters")).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let character_mount = index
        .mounts
        .iter()
        .position(|mount| mount.path == Index::CHARACTERS)
        .unwrap();
    let mut character_guids: BTreeSet<_> = catalog
        .objects
        .iter()
        .filter(|object| index.resources[object.resource].mount == character_mount)
        .map(|object| object.guid)
        .collect();
    let neighborhood = Neighborhood::open(&directory).unwrap();
    let mut neighbors = neighborhood.neighbors.clone();
    // 0x424e72 registers people missing from the old on-disk NBRS.
    // Supply the three template records the import path looks up.
    let mut templates = BTreeMap::new();
    for guid in [TEMPLATE_PERSON, TEMPLATE_DOG, TEMPLATE_CAT] {
        let definition = catalog
            .object(guid)
            .unwrap_or_else(|| panic!("template {guid:#010x}"));
        let template = Iff::parse(index.resources[definition.resource].read().unwrap()).unwrap();
        templates.insert(guid, template);
        if !neighbors.iter().flatten().any(|record| record.guid == guid) {
            let slot = neighbors
                .iter()
                .rposition(Option::is_none)
                .unwrap_or(neighbors.len());
            let record = Some(template_record((slot + 1) as i16, guid));
            if slot == neighbors.len() {
                neighbors.push(record);
            } else {
                neighbors[slot] = record;
            }
        }
    }
    let mut character_files: BTreeSet<_> = paths(&directory.join("Characters"))
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_ascii_lowercase();
            name.strip_prefix("user")?
                .strip_suffix(".iff")?
                .parse()
                .ok()
        })
        .collect();
    let house_files = neighborhood.houses().unwrap().into_keys().collect();
    let zoning = neighborhood
        .zoning(u16::from(install.expansions.0))
        .unwrap();
    let mut neighborhood_data = neighborhood.data;
    let mut families = neighborhood.families;
    let mut next_file = 0;
    let patches: Vec<_> = paths(&directory.join("Patch"))
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("FAM"))
        })
        .collect();
    assert_eq!(patches.len(), 6);
    assert!(patches.iter().any(|path| path.ends_with("Charming_13.FAM")));
    let output = common::Tree::new("import-clones");
    let mut written = BTreeMap::new();
    for path in patches {
        let input = ImportInput {
            catalog: &catalog,
            neighbors: &neighbors,
            families: &families,
            next_file,
            character_files: &character_files,
            character_guids: &character_guids,
            house_files: &house_files,
            zoning: &zoning,
            neighborhood_data,
        };
        let file = FamilyFile::parse(&fs::read(&path).unwrap()).unwrap();
        let member_ids: Vec<_> = file
            .export_info
            .member_ids
            .into_iter()
            .take_while(|&id| id != 0)
            .collect();
        let mut guids = if file.family.house == 0 {
            stray_guids.values().copied().collect::<Vec<_>>()
        } else {
            let house = read_iff(&first.join(format!("Houses/House{:02}.iff", file.family.house)));
            let table = GuidTable::parse(house.data(house.get(b"Gtab", 1).unwrap())).unwrap();
            let exported: Vec<_> = member_ids
                .iter()
                .map(|&id| {
                    file.neighbors.entries[usize::from(id) - 1]
                        .as_ref()
                        .unwrap()
                        .guid
                })
                .collect();
            assert_eq!(
                table.pairs.iter().map(|&(old, _)| old).collect::<Vec<_>>(),
                exported,
                "{}",
                path.display()
            );
            table.pairs.into_iter().map(|(_, new)| new).collect()
        }
        .into_iter();
        let imported = import(&input, &file, PETS, &mut || guids.next().unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(guids.next().is_none());
        for &guid in &imported.family.members {
            let (number, oracle) = &originals[&guid];
            let mut character = imported.made[&guid].clone();
            character.file = *number;
            let mut resources =
                clone_character(&templates[&character.template], &character, guid, &file)
                    .unwrap_or_else(|error| panic!("User{number:05}: {error}"));
            refresh_outfits(&mut resources, &wardrobe)
                .unwrap_or_else(|error| panic!("User{number:05}: {error}"));
            let bytes = write(&resources).unwrap();
            let clone = Iff::parse(bytes.clone()).unwrap();
            compare_clone(&clone, oracle, *number);
            output.file(&format!("Characters/User{number:05}.iff"), &bytes);
            assert!(written.insert(guid, *number).is_none());
        }
        if path.ends_with("Charming_13.FAM") {
            assert_eq!(imported.made.len(), 2);
            let templates: BTreeSet<_> = imported.made.values().map(|made| made.template).collect();
            assert_eq!(templates, BTreeSet::from([TEMPLATE_PERSON, TEMPLATE_DOG]));
            assert_eq!(imported.family_id, 13);
            assert_eq!(
                imported.neighbor_ids,
                BTreeMap::from([(126, 126), (127, 127)])
            );
            let lot = imported.lot.as_ref().unwrap();
            assert_eq!(lot.house, 69);
            for entry in &lot.object_types.entries {
                assert!(
                    catalog.object(entry.guid).is_some() || imported.made.contains_key(&entry.guid),
                    "House69: {} {:#010x} has no definition",
                    entry.name,
                    entry.guid,
                );
            }
        }
        if path.ends_with("Kat_6.FAM") {
            assert_eq!(
                imported
                    .made
                    .values()
                    .filter(|made| made.template == TEMPLATE_CAT)
                    .count(),
                4
            );
        }
        if path.ends_with("Strays_4000.FAM") {
            assert_eq!(imported.family_id, 4000);
            assert!(imported.lot.is_none());
            assert!(imported.global_rng.is_none());
        }
        for (id, record) in imported.neighbors {
            let id = usize::try_from(id).unwrap();
            if neighbors.len() < id {
                neighbors.resize(id, None);
            }
            assert!(neighbors[id - 1].replace(record).is_none());
        }
        character_files.extend(imported.made.values().map(|made| made.file));
        character_guids.extend(imported.made.keys().copied());
        if let Some((id, family)) = imported.displaced_family {
            families.insert(id, family);
        }
        assert!(
            families
                .insert(imported.family_id, imported.family)
                .is_none()
        );
        neighborhood_data = imported.neighborhood_data;
        next_file = imported.next_file;
    }
    assert_eq!(written.len(), 28);
    let unmapped: BTreeSet<_> = originals
        .iter()
        .filter_map(|(guid, (number, _))| (!written.contains_key(guid)).then_some(*number))
        .collect();
    assert_eq!(unmapped, BTreeSet::from([13, 14, 15, 17, 20, 21, 22, 23]));

    let mut index = Index::build(&install).unwrap();
    index.add_characters(&output.0.join("Characters")).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    for (guid, number) in written {
        let definition = catalog.object(guid).unwrap();
        assert_eq!(definition.kind, 2);
        assert_eq!(definition.chunk, 128);
        let resource = &index.resources[definition.resource];
        assert_eq!(
            resource.path,
            output.0.join(format!("Characters/User{number:05}.iff"))
        );
    }
}
