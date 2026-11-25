use crate::common::{Tree, far};
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
fn a_resolved_path_is_spelled_as_on_disk() {
    // On a case-insensitive filesystem the spelling asked for would also
    // open; the result must still be the one on disk, the same on every
    // machine.
    let tree = Tree::new("spelling");
    tree.file("gamedata/behavior.IFF", b"behavior")
        .dir("gamedata/OBJECTS");
    let install = Install::detect(&tree.0).unwrap();
    let path = install.file("GameData/Behavior.iff").unwrap();
    assert_eq!(path, tree.0.join("gamedata").join("behavior.IFF"));
}

#[test]
fn dot_components_must_lead_somewhere() {
    let tree = Tree::new("dots");
    base_game(&tree);
    let install = Install::detect(&tree.0).unwrap();
    assert!(install.file("GameData/Behavior.iff/..").is_err());
    assert!(install.file("GameData/./Behavior.iff").is_ok());
}

#[test]
fn names_differing_only_in_case_resolve_the_same_everywhere() {
    let tree = Tree::new("collide");
    base_game(&tree);
    tree.file("GameData/x.iff", b"lower")
        .file("GameData/X.IFF", b"upper");
    // Only counted, so the listing's order doesn't matter.
    #[allow(clippy::disallowed_methods)]
    let listed = std::fs::read_dir(tree.0.join("GameData"))
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name.eq_ignore_ascii_case("x.iff")
        })
        .count();
    if listed < 2 {
        // A case-insensitive filesystem holds only one of them.
        return;
    }
    let install = Install::detect(&tree.0).unwrap();
    let read = |path: &str| std::fs::read(install.file(path).unwrap()).unwrap();
    // An exact match wins; otherwise the first in sorted order, where
    // "X.IFF" comes before "x.iff".
    assert_eq!(read("GameData/x.iff"), b"lower");
    assert_eq!(read("GameData/X.IFF"), b"upper");
    assert_eq!(read("GameData/x.IFF"), b"upper");
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

#[test]
fn an_archive_claiming_a_huge_directory_is_skipped_unread() {
    let tree = Tree::new("huge-directory");
    base_game(&tree);
    // The directory starts right after the header, so the whole sparse
    // remainder would be the directory.
    let mut header = b"FAR!byAZ".to_vec();
    header.extend(1u32.to_le_bytes());
    header.extend(16u32.to_le_bytes());
    header.extend(1u32.to_le_bytes());
    tree.file("Downloads/Huge.far", &header)
        .file("Downloads/Good.iff", b"g");
    std::fs::OpenOptions::new()
        .write(true)
        .open(tree.0.join("Downloads/Huge.far"))
        .unwrap()
        .set_len(sims_formats::far::MAX_DIRECTORY + 17)
        .unwrap();
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    assert_eq!(index.named("Good.iff").count(), 1);
    assert_eq!(index.diagnostics.len(), 1);
    assert!(index.diagnostics[0].contains("64 MiB"));
}

#[test]
fn a_compressed_member_is_left_out_with_a_diagnostic() {
    let tree = Tree::new("compressed-member");
    base_game(&tree);
    let mut archive = far(&[("Packed.iff", &[7; 64]), ("Plain.iff", b"plain")]);
    // The first member's unpacked size, the first word of its entry.
    let manifest = u32::from_le_bytes(archive[12..16].try_into().unwrap()) as usize;
    archive[manifest + 4..manifest + 8].copy_from_slice(&65u32.to_le_bytes());
    tree.file("Downloads/pack.far", &archive);
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    assert_eq!(index.named("Packed.iff").count(), 0);
    let (_, plain) = index.named("Plain.iff").next().unwrap();
    assert_eq!(plain.read().unwrap(), b"plain");
    assert_eq!(index.diagnostics.len(), 1);
    assert!(index.diagnostics[0].contains("compressed member Packed.iff"));
}

#[test]
fn a_read_checks_the_member_against_the_archive_again() {
    let tree = Tree::new("stale-member");
    base_game(&tree);
    tree.file("Downloads/pack.far", &far(&[("Gone.iff", b"payload")]));
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let (_, member) = index.named("Gone.iff").next().unwrap();
    assert_eq!(member.read().unwrap(), b"payload");
    // The archive shrinks after indexing.
    tree.file("Downloads/pack.far", b"FAR!byAZ");
    assert!(member.read().unwrap_err().to_string().contains("outside"));
}

#[test]
fn a_resource_over_the_size_limit_fails_to_read() {
    let tree = Tree::new("huge-resource");
    base_game(&tree);
    tree.file("Downloads/Huge.iff", b"");
    std::fs::OpenOptions::new()
        .write(true)
        .open(tree.0.join("Downloads/Huge.iff"))
        .unwrap()
        .set_len(sims_formats::far::MAX_MEMBER + 1)
        .unwrap();
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let (_, huge) = index.named("Huge.iff").next().unwrap();
    assert!(huge.read().unwrap_err().to_string().contains("128 MiB"));
}

#[test]
fn a_member_nested_too_deeply_is_left_out() {
    let tree = Tree::new("deep-member");
    base_game(&tree);
    let deep = format!("{}x.iff", "a\\".repeat(33));
    let shallow = format!("{}y.iff", "a\\".repeat(32));
    tree.file(
        "Downloads/pack.far",
        &far(&[(&deep, b"deep"), (&shallow, b"shallow")]),
    );
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let listed = |name: &str| index.resources.iter().any(|r| r.name() == name);
    assert!(listed(&shallow));
    assert!(!listed(&deep));
    assert_eq!(index.diagnostics.len(), 1);
    assert!(index.diagnostics[0].contains("in 33 directories"));
}

#[test]
fn a_compressed_member_of_the_animation_archives_fails_the_load() {
    let tree = Tree::new("compressed-animation");
    base_game(&tree);
    let mut archive = far(&[("clips.bcf", &[0; 8])]);
    let manifest = u32::from_le_bytes(archive[12..16].try_into().unwrap()) as usize;
    archive[manifest + 4..manifest + 8].copy_from_slice(&9u32.to_le_bytes());
    tree.file("GameData/Animation/Animation.far", &archive);
    let install = Install::detect(&tree.0).unwrap();
    let index = Index::build(&install).unwrap();
    let error = sims_content::Animations::load(&install, &index).unwrap_err();
    assert!(error.to_string().contains("compressed member clips.bcf"));
}

/// A FAR whose member names are given as raw bytes.
fn far_raw(members: &[(&[u8], &[u8])]) -> Vec<u8> {
    let named: Vec<(String, &[u8])> = members
        .iter()
        .map(|(name, data)| ("x".repeat(name.len()), *data))
        .collect();
    let named: Vec<(&str, &[u8])> = named.iter().map(|(n, d)| (n.as_str(), *d)).collect();
    let mut bytes = far(&named);
    // Each placeholder name is as long as its raw name, so the directory
    // keeps its layout; overwrite the names in order.
    let mut at = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize + 4;
    for (name, _) in members {
        at += 16;
        bytes[at..at + name.len()].copy_from_slice(name);
        at += name.len();
    }
    bytes
}

#[test]
fn archive_member_names_hash_as_stored_bytes() {
    let tree = Tree::new("member-code-page");
    base_game(&tree);
    // "café.iff" in Windows-1252, as an archive stores it. Its bytes hash
    // to bucket 158 of 193, after obj0.iff's 43; the name's UTF-8 bytes
    // would hash to bucket 0 and list it first.
    tree.file(
        "Downloads/p.far",
        &far_raw(&[(b"obj0.iff", b"ascii"), (b"caf\xe9.iff", b"archive")]),
    );
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let names: Vec<_> = index
        .resources
        .iter()
        .filter(|r| r.mount == 3)
        .map(|r| r.name())
        .collect();
    assert_eq!(names, ["obj0.iff", "café.iff"]);
}

#[test]
fn a_loose_file_replaces_the_member_its_name_encodes_to() {
    let tree = Tree::new("loose-code-page");
    base_game(&tree);
    // The member's name is Windows-1252; the loose file's, on disk, UTF-8.
    tree.file("Downloads/p.far", &far_raw(&[(b"Caf\xe9.iff", b"archive")]))
        .file("Downloads/Café.iff", b"loose");
    let index = Index::build(&Install::detect(&tree.0).unwrap()).unwrap();
    let found: Vec<_> = index
        .named("Café.iff")
        .map(|(_, r)| r.read().unwrap())
        .collect();
    assert_eq!(found, [b"loose".to_vec()]);
}
