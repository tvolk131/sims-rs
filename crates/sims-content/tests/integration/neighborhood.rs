use crate::common::{Tree, far, iff, objd};
use sims_content::{Catalog, House, Index, Install, Neighborhood};

fn saved(magic: &[u8; 4], version: u32) -> Vec<u8> {
    let mut bytes = vec![0; 4];
    bytes.extend(version.to_le_bytes());
    bytes.extend(magic);
    bytes
}

fn family(house: i32, flags: u32, members: &[u32]) -> Vec<u8> {
    let mut bytes = saved(b"IMAF", 7);
    for value in [house, 0, 1000, 0, 0] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(flags.to_le_bytes());
    bytes.extend((members.len() as u32).to_le_bytes());
    for member in members {
        bytes.extend(member.to_le_bytes());
    }
    bytes
}

/// A version 0x3e SIMI with the given tick count and funds.
fn simulator(ticks: u32, funds: i32) -> Vec<u8> {
    let mut bytes = saved(b"IMIS", 0x3e);
    bytes.extend([0; 64]);
    bytes.extend(ticks.to_le_bytes());
    bytes.extend(0i16.to_le_bytes());
    bytes.extend(funds.to_le_bytes());
    bytes.extend([0; 4 * 6]);
    bytes.extend([0; 4 * 6]);
    bytes
}

/// An install with one object, and a neighborhood whose family 1 lives in
/// house 2 with a character defined in `Characters`.
fn setup(tree: &Tree) {
    tree.file("Game/GameData/Behavior.iff", b"")
        .file(
            "Game/GameData/Global/Global.far",
            &far(&[("Global.iff", &iff(&[]))]),
        )
        .file(
            "Game/GameData/Objects/Chair.iff",
            &iff(&[(b"OBJD", 128, &objd(0x1111))]),
        );
    let mut person = objd(0x2222);
    person[18..20].copy_from_slice(&2u16.to_le_bytes());
    let houses = iff(&[
        (b"FAMI", 1, &family(2, 1, &[0x2222])),
        (b"FAMI", 0, &family(0, 0, &[])),
    ]);
    let objects = {
        let mut bytes = saved(b"MjbO", 0x3e);
        bytes.extend([1, 0]); // compressed, empty registration table
        bytes
    };
    tree.file("Hood/Neighborhood.iff", &houses)
        .file(
            "Hood/Characters/User00000.iff",
            &iff(&[(b"OBJD", 128, &person)]),
        )
        .file(
            "Hood/Houses/House02.iff",
            &iff(&[
                (b"SIMI", 1, &simulator(4000, 20000)),
                (b"objt", 0, &saved(b"tjbo", 2)),
                (b"ObjM", 1, &objects),
            ]),
        )
        .file("Hood/Houses/house10.IFF", &iff(&[]))
        .file("Hood/Houses/Notes.txt", b"");
}

fn catalog(tree: &Tree, neighborhood: &Neighborhood) -> (Index, Catalog) {
    let install = Install::detect(&tree.0.join("Game")).unwrap();
    let mut index = Index::build(&install).unwrap();
    let characters = neighborhood.characters().unwrap().unwrap();
    index.add_characters(&characters).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    (index, catalog)
}

/// Family 0 has no member list (0x4105e7): after its fixed fields come
/// the secondary house and the version 9 words. Read with a member list,
/// a secondary house of 5 would ask for five members the chunk lacks.
#[test]
fn family_zero_is_read_without_a_member_list() {
    let mut zero = saved(b"IMAF", 9);
    for value in [0i32, 3, 1000, 0, 0] {
        zero.extend(value.to_le_bytes());
    }
    zero.extend(0u32.to_le_bytes()); // flags
    zero.extend(5i32.to_le_bytes()); // secondary house
    zero.extend([1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0]); // magic words
    let tree = Tree::new("family-zero");
    tree.file(
        "Hood/Neighborhood.iff",
        &iff(&[(b"FAMI", 0, &zero), (b"FAMI", 1, &family(2, 1, &[0x2222]))]),
    );
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    let family = &neighborhood.families[&0];
    assert!(family.members.is_empty());
    assert_eq!(family.secondary_house(), 5);
    assert_eq!(family.tail.len(), 16);
    assert_eq!(family.to_chunk(0).unwrap(), zero);
    assert_eq!(neighborhood.families[&1].members, [0x2222]);
}

#[test]
fn reads_families_and_finds_houses() {
    let tree = Tree::new("neighborhood");
    setup(&tree);
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    assert_eq!(
        neighborhood.families.keys().copied().collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(neighborhood.families[&1].members, [0x2222]);
    let houses: Vec<_> = neighborhood.houses().unwrap().into_keys().collect();
    assert_eq!(houses, [2, 10]);
    assert_eq!(neighborhood.family_in(2).map(|(id, _)| id), Some(1));
    // Family 0 has house number 0, which means no house.
    assert!(neighborhood.family_in(0).is_none());
    assert!(neighborhood.family_in(10).is_none());
}

#[test]
fn characters_register_after_user_objects() {
    let tree = Tree::new("characters");
    setup(&tree);
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    let (index, catalog) = catalog(&tree, &neighborhood);
    let guids: Vec<_> = catalog.objects.iter().map(|o| o.guid).collect();
    assert_eq!(guids, [0x1111, 0x2222]);
    let person = catalog.object(0x2222).unwrap();
    assert_eq!(person.kind, 2);
    let mount = index.resources[person.resource].mount;
    assert_eq!(index.mounts[mount].path, "Characters");
    // The people come after the user objects and before Downloads.
    assert_eq!(index.mounts[mount - 1].path, "GameData/UserObjects");
    assert_eq!(index.mounts[mount + 1].path, "Downloads");
}

#[test]
fn loads_a_house() {
    let tree = Tree::new("house");
    setup(&tree);
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    let (_, catalog) = catalog(&tree, &neighborhood);
    let houses = neighborhood.houses().unwrap();
    let house = House::load(&houses[&2], &catalog).unwrap();
    assert_eq!(house.simulator.ticks, 4000);
    assert_eq!(house.simulator.funds, 20000);
    assert!(house.objects.instances.is_empty());
    assert!(house.arrays.is_empty() && house.view.is_none() && house.model.is_none());
    // House 10 has no SIMI.
    let error = House::load(&houses[&10], &catalog).unwrap_err().to_string();
    assert!(error.contains("SIMI"), "{error}");
}

#[test]
fn a_neighborhood_needs_its_family_file() {
    let tree = Tree::new("no-neighborhood");
    tree.dir("Hood/Houses");
    let error = Neighborhood::open(&tree.0.join("Hood"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("Neighborhood.iff"), "{error}");
}

/// A string table in the localized format: (language, text) rows.
fn strings(rows: &[(u8, &str)]) -> Vec<u8> {
    let mut bytes = (-3i16).to_le_bytes().to_vec();
    bytes.extend((rows.len() as u16).to_le_bytes());
    for (language, text) in rows {
        bytes.push(*language);
        bytes.extend(text.as_bytes());
        bytes.extend([0, 0]); // end of text, empty comment
    }
    bytes
}

#[test]
fn zoning_comes_from_the_neighborhoods_table() {
    let tree = Tree::new("zoning");
    setup(&tree);
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    // No file, no table.
    assert!(neighborhood.zoning(0xff).unwrap().is_empty());
    let table = strings(&[
        (1, "1, residential"),
        (1, " 5 , Community"),
        (1, "5, residential"), // the first row for a lot wins
        (1, "7, something else"),
        (1, "8, residential community"),
        (1, "no comma here"),
        (1, "x12, community"), // no leading number: lot 0
        (1, "-3, community"),
        (1, "99999, community"), // does not fit a lot number
        (2, "2, community"),     // another language's rows are not read
    ]);
    tree.file("Hood/LotZoning.iff", &iff(&[(b"STR#", 1, &table)]));
    let zoning = neighborhood.zoning(0).unwrap();
    let rows: Vec<_> = zoning.into_iter().collect();
    assert_eq!(rows, [(-3, 2), (0, 2), (1, 1), (5, 2), (7, 1), (8, 1)]);
    // With Makin' Magic installed and no row for its lots, they are added.
    let zoning = neighborhood.zoning(0x80).unwrap();
    assert_eq!(zoning.len(), 15);
    assert_eq!(
        (zoning[&90], zoning[&92], zoning[&93], zoning[&98]),
        (1, 1, 2, 2)
    );
    // A row for one of them means the file already knows the pack.
    let table = strings(&[(1, "91, community")]);
    tree.file("Hood/LotZoning.iff", &iff(&[(b"STR#", 1, &table)]));
    let zoning = neighborhood.zoning(0x80).unwrap();
    assert_eq!(zoning.into_iter().collect::<Vec<_>>(), [(91, 2)]);
}

#[test]
fn reads_the_saved_type_attributes() {
    let tree = Tree::new("type-attributes");
    setup(&tree);
    assert!(
        Neighborhood::open(&tree.0.join("Hood"))
            .unwrap()
            .type_attributes
            .is_none()
    );
    // Unpacked: a record count, then GUID, word count and words.
    let mut tatt = saved(b"TTAT", 0);
    tatt.push(0);
    tatt.extend(1i32.to_le_bytes());
    tatt.extend(0x1111u32.to_le_bytes());
    tatt.extend(2i32.to_le_bytes());
    tatt.extend([5, 0, 0xff, 0xff]);
    let houses = iff(&[(b"FAMI", 0, &family(0, 0, &[])), (b"TATT", 1, &tatt)]);
    tree.file("Hood/Neighborhood.iff", &houses);
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    let saved = neighborhood.type_attributes.unwrap();
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(
        (saved.entries[0].guid, &saved.entries[0].values[..]),
        (0x1111, &[5, -1][..])
    );
}

/// A version 2 type-table entry.
fn type_entry(guid: u32, id: u16, name: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(guid.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(id.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(name.as_bytes());
    bytes.push(0);
    if bytes.len() % 2 != 0 {
        bytes.push(0);
    }
    bytes
}

/// A house whose type table names GUIDs only its `Gtab` maps to defined
/// ones (0x43a6fb): 0x9999 to 0x2222 directly, 0x7777 through 0x8888.
#[test]
fn the_guid_table_translates_the_type_table() {
    let tree = Tree::new("guid-table");
    setup(&tree);
    let mut types = saved(b"tjbo", 2);
    types.extend(type_entry(0x9999, 1, "Exported"));
    types.extend(type_entry(0x7777, 2, "Chained"));
    types.extend(type_entry(0x1111, 3, "Chair"));
    let mut table = saved(b"batG", 0);
    table.extend(3u32.to_le_bytes());
    for (old, new) in [(0x9999u32, 0x2222u32), (0x7777, 0x8888), (0x8888, 0x1111)] {
        table.extend(old.to_le_bytes());
        table.extend(new.to_le_bytes());
    }
    let objects = {
        let mut bytes = saved(b"MjbO", 0x3e);
        bytes.extend([1, 0]);
        bytes
    };
    tree.file(
        "Hood/Houses/House03.iff",
        &iff(&[
            (b"SIMI", 1, &simulator(4000, 20000)),
            (b"Gtab", 1, &table),
            (b"objt", 0, &types),
            (b"ObjM", 1, &objects),
        ]),
    );
    let neighborhood = Neighborhood::open(&tree.0.join("Hood")).unwrap();
    let (_, catalog) = catalog(&tree, &neighborhood);
    let houses = neighborhood.houses().unwrap();
    let house = House::load(&houses[&3], &catalog).unwrap();
    let guids: Vec<_> = house.types.entries.iter().map(|t| t.guid).collect();
    assert_eq!(guids, [0x2222, 0x1111, 0x1111]);
    assert_eq!(house.guid_table.unwrap().pairs.len(), 3);
    // A table under another chunk ID is not read (0x41578e asks for 1).
    tree.file(
        "Hood/Houses/House03.iff",
        &iff(&[
            (b"SIMI", 1, &simulator(4000, 20000)),
            (b"Gtab", 2, &table),
            (b"objt", 0, &types),
            (b"ObjM", 1, &objects),
        ]),
    );
    let house = House::load(&houses[&3], &catalog).unwrap();
    assert!(house.guid_table.is_none());
    assert_eq!(house.types.entries[0].guid, 0x9999);
}

/// Needs the game: set `SIMS_INSTALL` to the install folder.
///
/// The first neighborhood's Old Town homes were saved under their
/// families' exported GUIDs and carry the import's GUID table, which maps
/// them to the characters in `UserData/Characters`. House 69's maps
/// Claire 0x64BCCDFB to 0xBC364BF5 and Luna 0x67FDDFD9 to 0xBE24148E.
#[test]
fn the_old_town_homes_load_through_their_guid_tables() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to read the Old Town houses");
        return;
    };
    let root = std::path::Path::new(&root);
    let install = Install::detect(root).unwrap();
    let neighborhood = Neighborhood::open(&root.join("UserData")).unwrap();
    let mut index = Index::build(&install).unwrap();
    index
        .add_characters(&neighborhood.characters().unwrap().unwrap())
        .unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let houses = neighborhood.houses().unwrap();
    for number in [56, 63, 69, 76, 77] {
        let house = House::load(&houses[&number], &catalog)
            .unwrap_or_else(|error| panic!("house {number}: {error}"));
        let table = house.guid_table.as_ref().expect("a GUID table");
        for (old, _) in &table.pairs {
            assert!(
                house.types.entries.iter().all(|t| t.guid != *old),
                "house {number} still names 0x{old:08X}"
            );
        }
        let (_, family) = neighborhood.family_in(number).unwrap();
        for member in &family.members {
            assert!(
                house.types.entries.iter().any(|t| t.guid == *member),
                "house {number} has no type for member 0x{member:08X}"
            );
        }
    }
    let house = House::load(&houses[&69], &catalog).unwrap();
    assert_eq!(
        house.guid_table.unwrap().pairs,
        [(0x64BC_CDFB, 0xBC36_4BF5), (0x67FD_DFD9, 0xBE24_148E)]
    );
}
