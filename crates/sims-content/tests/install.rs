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
            // The file table puts Zebra.iff before the apple directory.
            (3, "Zebra.iff".into()),
            (3, "Apple.iff".into()),
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
            "Downloads",
            "ExpansionPack",
            "Deluxe",
            "ExpansionShared",
            "TemplateNPCs",
            // Makin' Magic is scanned last, after the shared directories.
            "ExpansionPack7",
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
            "Shared.iff",
            "Npc.iff",
            "Wand.iff",
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

/// Mount 3 (Downloads) as names.
fn downloads(tree: &Tree) -> Vec<String> {
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    assert!(index.diagnostics.is_empty(), "{:?}", index.diagnostics);
    index
        .resources
        .iter()
        .filter(|r| r.mount == 3)
        .map(|r| r.name())
        .collect()
}

#[test]
fn a_directory_lists_in_file_table_order() {
    let tree = Tree::new("order");
    base_game(&tree);
    tree.file("Downloads/b.iff", b"")
        .file("Downloads/A/inner.iff", b"")
        .file("Downloads/C.iff", b"")
        .file("Downloads/a.iff", b"");
    // By the hash of each lower-case name, in a table of 193 buckets:
    // c.iff 124, a.iff 138, a 165, b.iff 192. The subdirectory A is
    // searched where its own name falls.
    assert_eq!(downloads(&tree), ["C.iff", "a.iff", "inner.iff", "b.iff"]);
}

#[test]
fn names_sharing_a_bucket_list_newest_first() {
    let tree = Tree::new("bucket");
    base_game(&tree);
    // obj8.iff and obj19.iff both hash to bucket 111. The directory lists
    // obj19.iff first (name order), so obj8.iff is added later and comes
    // out first. An archive's members are added in their stored order, at
    // the point the archive is reached: both land in bucket 111 too.
    tree.file("Downloads/obj8.iff", b"")
        .file("Downloads/obj19.iff", b"");
    assert_eq!(downloads(&tree), ["obj8.iff", "obj19.iff"]);

    let tree = Tree::new("bucket-archive");
    base_game(&tree);
    tree.file(
        "Downloads/pack.far",
        &far(&[("obj8.iff", b"8"), ("obj19.iff", b"19")]),
    );
    assert_eq!(downloads(&tree), ["obj19.iff", "obj8.iff"]);
}

#[test]
fn archive_members_with_paths_go_into_subdirectories() {
    let tree = Tree::new("member-paths");
    base_game(&tree);
    // The archive's People member joins the People directory on disk, and
    // the loose Chair.iff, listed before the archive, wins over the
    // archive's member of the same name.
    tree.file("Downloads/Chair.iff", b"loose")
        .file("Downloads/People/Maid.iff", b"maid")
        .file(
            "Downloads/pack.far",
            &far(&[
                ("chair.IFF", b"packed"),
                ("People\\Cook.iff", b"cook"),
                ("Skins/skin.bmp", b"skin"),
            ]),
        );
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let listed: Vec<_> = index
        .resources
        .iter()
        .filter(|r| r.mount == 3)
        .map(|r| (r.name(), r.read().unwrap()))
        .collect();
    // Buckets: skins 3, chair.iff 78, people 102; inside People,
    // maid.iff 23 and cook.iff 89.
    assert_eq!(
        listed,
        [
            ("Skins/skin.bmp".to_owned(), b"skin".to_vec()),
            ("Chair.iff".to_owned(), b"loose".to_vec()),
            ("Maid.iff".to_owned(), b"maid".to_vec()),
            ("People\\Cook.iff".to_owned(), b"cook".to_vec()),
        ]
    );
}

#[test]
fn directories_named_dir_are_not_searched() {
    let tree = Tree::new("dot-dir");
    base_game(&tree);
    tree.file("Downloads/Old.dir/Hidden.iff", b"")
        .file("Downloads/Seen.iff", b"");
    assert_eq!(downloads(&tree), ["Seen.iff"]);
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
