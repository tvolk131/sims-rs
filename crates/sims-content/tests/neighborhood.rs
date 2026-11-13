mod common;

use common::{Tree, far, iff, objd};
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
