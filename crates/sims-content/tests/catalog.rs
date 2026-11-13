mod common;

use common::{Tree, far, iff, objd};
use sims_content::{Catalog, Index, Install};

fn catalog(tree: &Tree) -> (Index, Catalog) {
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    (index, catalog)
}

fn name(index: &Index, resource: usize) -> String {
    index.resources[resource].name()
}

/// A base game whose Global.far holds Global.iff and the given semi-globals.
fn base(tree: &Tree, semi_globals: &[(&str, &[u8])]) {
    let global = iff(&[]);
    let mut members = vec![("Global.iff", &global[..])];
    members.extend_from_slice(semi_globals);
    tree.file("GameData/Behavior.iff", b"")
        .file("GameData/Global/Global.far", &far(&members))
        .dir("GameData/Objects");
}

#[test]
fn registers_definitions_in_load_order_and_later_guids_win() {
    let tree = Tree::new("register");
    base(&tree, &[]);
    let mut tile = objd(0x2222);
    tile[20..22].copy_from_slice(&7u16.to_le_bytes()); // multi-tile group 7
    let chair = iff(&[(b"OBJD", 128, &objd(0x1111)), (b"OBJD", 129, &tile)]);
    let replacement = iff(&[(b"OBJD", 300, &objd(0x1111))]);
    tree.file(
        "GameData/Objects/Objects.far",
        &far(&[("Chair.iff", &chair)]),
    )
    .file("Downloads/Better.iff", &replacement);
    let (index, catalog) = catalog(&tree);
    assert_eq!(name(&index, catalog.global), "Global.iff");
    let registered: Vec<_> = catalog
        .objects
        .iter()
        .map(|o| (o.guid, name(&index, o.resource), o.chunk))
        .collect();
    assert_eq!(
        registered,
        [
            (0x1111, "Chair.iff".into(), 128),
            (0x2222, "Chair.iff".into(), 129),
            (0x1111, "Better.iff".into(), 300),
        ]
    );
    assert_eq!(catalog.object(0x1111).unwrap().chunk, 300);
    assert_eq!(catalog.object(0x2222).unwrap().chunk, 129);
    assert_eq!(catalog.object(0x2222).unwrap().master, 7);
    assert_eq!(catalog.object(0x1111).unwrap().master, 0);
    assert!(catalog.object(0x3333).is_none());
    assert_eq!(catalog.diagnostics.len(), 1);
    assert!(
        catalog.diagnostics[0].contains("0x00001111"),
        "{:?}",
        catalog.diagnostics
    );
}

#[test]
fn resolves_semi_global_links_by_file_name() {
    let tree = Tree::new("links");
    let sofa_globals = iff(&[]);
    base(&tree, &[("SofaGlobals.iff", &sofa_globals)]);
    let sofa = iff(&[(b"OBJD", 128, &objd(1)), (b"GLOB", 128, b"\x0bsofaglobals")]);
    let rocker = iff(&[(b"OBJD", 128, &objd(2)), (b"GLOB", 128, b"\0")]);
    let lamp = iff(&[(b"OBJD", 128, &objd(3)), (b"GLOB", 128, b"\x0bLampGlobals")]);
    tree.file("Downloads/Sofa.iff", &sofa)
        .file("Downloads/Rocker.iff", &rocker)
        .file("Downloads/Lamp.iff", &lamp);
    let (index, catalog) = catalog(&tree);
    let link = |file: &str| {
        let (resource, _) = index.named(file).next().unwrap();
        catalog.semi_globals.get(&resource).copied()
    };
    let target = link("Sofa.iff").unwrap().unwrap();
    assert_eq!(name(&index, target), "SofaGlobals.iff");
    assert_eq!(link("Rocker.iff"), Some(None));
    assert_eq!(link("Lamp.iff"), None);
    assert_eq!(catalog.diagnostics.len(), 1);
    assert!(catalog.diagnostics[0].contains("LampGlobals.iff not found"));
}

#[test]
fn bad_files_and_definitions_become_diagnostics() {
    let tree = Tree::new("bad");
    base(&tree, &[]);
    let mut short = objd(5);
    short.truncate(40);
    tree.file("Downloads/Broken.iff", b"IFF FILE 2.5: truncated")
        .file(
            "Downloads/Short.iff",
            &iff(&[(b"OBJD", 128, &short), (b"OBJD", 129, &objd(6))]),
        );
    let (_, catalog) = catalog(&tree);
    let guids: Vec<_> = catalog.objects.iter().map(|o| o.guid).collect();
    assert_eq!(guids, [6]);
    assert_eq!(catalog.diagnostics.len(), 2, "{:?}", catalog.diagnostics);
    assert!(catalog.diagnostics[0].contains("Broken.iff"));
    assert!(catalog.diagnostics[1].contains("Short.iff OBJD #128"));
}

#[test]
fn the_global_file_is_required() {
    let tree = Tree::new("no-global");
    tree.file("GameData/Behavior.iff", b"")
        .dir("GameData/Objects")
        .file("GameData/Global/Other.iff", &iff(&[]));
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let error = Catalog::build(&index).unwrap_err().to_string();
    assert!(error.contains("Global.iff"), "{error}");
}
