mod common;

use common::{Tree, far};
use sims_content::{Expansions, Index, Install, Mount};

fn base_game(tree: &Tree) {
    tree.file("GameData/Behavior.iff", b"behavior")
        .file("GameData/Global/Global.iff", b"global")
        .file(
            "GameData/Objects/Objects.far",
            &far(&[("Chair.iff", b"chair"), ("Bed.iff", b"bed")]),
        )
        .dir("GameData/UserObjects");
}

fn names(index: &Index) -> Vec<(usize, String)> {
    index
        .resources
        .iter()
        .map(|r| (r.mount, r.name()))
        .collect()
}

fn mount_paths(mounts: &[Mount]) -> Vec<&str> {
    mounts.iter().map(|m| m.path.as_str()).collect()
}

#[test]
fn rejects_directories_that_are_not_installs() {
    let tree = Tree::new("not-install");
    tree.file("readme.txt", b"");
    assert!(Install::detect(&tree.0).is_err());
    assert!(Install::detect(&tree.0.join("absent")).is_err());
}

#[test]
fn base_game_mounts_and_indexes_in_load_order() {
    let tree = Tree::new("base");
    base_game(&tree);
    tree.file("Downloads/Zebra.iff", b"z")
        .file("Downloads/apple/Apple.iff", b"a");
    let install = Install::detect(&tree.0).unwrap();
    assert_eq!(install.expansions, Expansions(0));
    let index = Index::build(&install).unwrap();
    assert_eq!(
        mount_paths(&index.mounts),
        [
            "GameData/Global",
            "GameData/Objects",
            "GameData/UserObjects",
            "Downloads"
        ]
    );
    assert_eq!(
        names(&index),
        [
            (0, "Global.iff".into()),
            (1, "Chair.iff".into()),
            (1, "Bed.iff".into()),
            (3, "Apple.iff".into()),
            (3, "Zebra.iff".into()),
        ]
    );
    let (_, bed) = index.named("BED.IFF").next().unwrap();
    assert_eq!(bed.read().unwrap(), b"bed");
    assert_eq!(index.resources[0].read().unwrap(), b"global");
    assert!(index.diagnostics.is_empty());
}

#[test]
fn expansions_come_from_pack_directories_holding_archives() {
    let tree = Tree::new("expansions");
    base_game(&tree);
    tree.file(
        "ExpansionPack/ExpansionPack.far",
        &far(&[("Sofa.iff", b"s")]),
    )
    .file("Deluxe/Deluxe.far", &far(&[]))
    .file(
        "ExpansionPack7/ExpansionPack7.far",
        &far(&[("Wand.iff", b"w")]),
    )
    // A pack directory without an archive is not an installed pack.
    .file("ExpansionPack2/readme.txt", b"")
    .file(
        "ExpansionShared/ExpansionShared.far",
        &far(&[("Shared.iff", b"x")]),
    )
    .file("TemplateNPCs/Npc.iff", b"n")
    .file("TemplateNPCs/Nested/Hidden.iff", b"h");
    let install = Install::detect(&tree.0).unwrap();
    assert_eq!(install.expansions, Expansions(1 | 16 | 128));
    assert_eq!(
        install.expansions.to_string(),
        "Livin' Large, Deluxe Edition, Makin' Magic"
    );
    let index = Index::build(&install).unwrap();
    assert_eq!(
        mount_paths(&index.mounts),
        [
            "GameData/Global",
            "GameData/Objects",
            "GameData/UserObjects",
            "ExpansionPack",
            "Deluxe",
            "ExpansionPack7",
            "ExpansionShared",
            "TemplateNPCs",
            "Downloads",
        ]
    );
    let names: Vec<_> = index.resources.iter().map(|r| r.name()).collect();
    // TemplateNPCs is not searched recursively.
    assert_eq!(
        names,
        [
            "Global.iff",
            "Chair.iff",
            "Bed.iff",
            "Sofa.iff",
            "Wand.iff",
            "Shared.iff",
            "Npc.iff"
        ]
    );
}

#[test]
fn only_later_packs_mount_the_shared_directories() {
    let tree = Tree::new("early-packs");
    base_game(&tree);
    tree.file("ExpansionPack/ExpansionPack.far", &far(&[]))
        .file("ExpansionPack2/ExpansionPack2.far", &far(&[]))
        .file(
            "ExpansionShared/ExpansionShared.far",
            &far(&[("Shared.iff", b"x")]),
        );
    let install = Install::detect(&tree.0).unwrap();
    assert_eq!(install.expansions, Expansions(3));
    let mounts = install.mounts();
    assert!(!mount_paths(&mounts).contains(&"ExpansionShared"));
}

#[test]
fn directory_order_ignores_case_and_interleaves_subdirectories() {
    let tree = Tree::new("order");
    base_game(&tree);
    tree.file("Downloads/b.iff", b"")
        .file("Downloads/A/inner.iff", b"")
        .file("Downloads/C.iff", b"")
        .file("Downloads/a.iff", b"");
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let downloads: Vec<_> = index
        .resources
        .iter()
        .filter(|r| r.mount == 3)
        .map(|r| r.name())
        .collect();
    // "A" sorts before "a.iff" ("A" < "A.IFF"), then b, then C.
    assert_eq!(downloads, ["inner.iff", "a.iff", "b.iff", "C.iff"]);
}

#[test]
fn paths_resolve_ignoring_case() {
    let tree = Tree::new("case");
    tree.file("gamedata/behavior.IFF", b"behavior")
        .dir("gamedata/OBJECTS");
    let install = Install::detect(&tree.0).unwrap();
    let path = install.file("GameData/Behavior.iff").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"behavior");
    assert!(install.file("GameData/Missing.iff").is_err());
}

#[test]
fn unreadable_archives_are_skipped_with_a_diagnostic() {
    let tree = Tree::new("bad-far");
    base_game(&tree);
    tree.file("Downloads/Broken.far", b"FAR!byAZ junk")
        .file("Downloads/Good.iff", b"g");
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    assert_eq!(index.named("Good.iff").count(), 1);
    assert_eq!(index.diagnostics.len(), 1);
    assert!(index.diagnostics[0].contains("Broken.far"));
}
