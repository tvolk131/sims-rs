mod common;

use common::{Tree, iff};
use sims_content::{Install, Surfaces};

/// A pattern file in a subdirectory is in the catalog (the scan goes into
/// every directory, 0x412699), under its file name alone, and a name with
/// a character outside ASCII is read in Windows-1252, as a house's map
/// names it. Every byte is a character, so a name that starts with the
/// bytes of a byte-order mark stays apart from the name without them.
#[test]
fn a_catalog_holds_nested_files_under_their_windows_1252_names() {
    let pattern = iff(&[(b"SPR2", 1, b""), (b"SPR2", 2, b""), (b"SPR2", 3, b"")]);
    let tree = Tree::new("surfaces-nested");
    tree.file("GameData/Behavior.iff", b"")
        .dir("GameData/Objects")
        .file("GameData/Floors/custom/deeper/tile.flr", &pattern)
        .file("GameData/Floors/caf\u{e9}.flr", &pattern)
        .file("GameData/Floors/plain.flr", &pattern)
        .file("GameData/Floors/\u{ef}\u{bb}\u{bf}plain.flr", &pattern);
    let install = Install::detect(&tree.0).unwrap();
    let surfaces = Surfaces::build(&install).unwrap();
    let floors = &surfaces.floors;
    assert_eq!(floors.files.len(), 5);
    let plain = floors.file("PLAIN.FLR").expect("the plain name");
    let marked = floors
        .file("\u{ef}\u{bb}\u{bf}PLAIN.FLR")
        .expect("the marked name");
    assert_ne!(plain, marked);
    assert_eq!(floors.files[plain].name, "plain.flr");
    let tile = floors.file("TILE.FLR").expect("the nested file");
    let cafe = floors.file("CAF\u{e9}.FLR").expect("the Windows-1252 name");
    assert_eq!(floors.files[cafe].name, "caf\u{e9}.flr");
    let mut keys = [floors.key(tile, 1), floors.key(cafe, 1)];
    keys.sort();
    assert_eq!(keys, [Some(256), Some(257)]);
}

/// Needs the game: set `SIMS_INSTALL` to the install folder.
///
/// The places and keys are the original's, read from its memory while it
/// ran on the Complete Collection. The guest's memory still named 150 of
/// the 208 floor files and 186 of the 272 wall files, all at the places
/// these catalogs give them, and all 480 keys matched.
#[test]
fn the_complete_collections_catalogs_match_the_originals() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to read the install's pattern files");
        return;
    };
    let install = Install::detect(std::path::Path::new(&root)).unwrap();
    let surfaces = Surfaces::build(&install).unwrap();
    assert_eq!(surfaces.floors.files.len(), 209);
    assert_eq!(surfaces.walls.files.len(), 273);
    for (catalog, places) in [
        (
            &surfaces.floors,
            &[
                (1, "starburstrb.flr", 256),
                (158, "floor_castle2.flr", 413),
                (196, "pineboard2.flr", 451),
                (198, "pineboard1.flr", 453),
                (206, "floorjapanesemat2.flr", 461),
            ][..],
        ),
        (
            &surfaces.walls,
            &[
                (1, "decowallwhite.wll", 256),
                (158, "tomatocoulis.wll", 413),
                (272, "wallshojiscreen.wll", 527),
            ][..],
        ),
    ] {
        for &(place, name, key) in places {
            assert_eq!(catalog.files[place].name, name);
            assert_eq!(catalog.file(&name.to_uppercase()), Some(place));
            assert_eq!(catalog.key(place, 1), Some(key));
            assert_eq!(catalog.key(place, 2), None);
        }
        // Every file holds one pattern, and the keys run on from 256.
        for (place, file) in catalog.files.iter().enumerate().skip(1) {
            assert_eq!(file.keys.len(), 1, "{}", file.location);
            assert_eq!(catalog.key(place, 1), Some(255 + place as u16));
        }
    }
    assert_eq!(surfaces.floors.files[0].name, "floors.iff");
    assert_eq!(surfaces.walls.files[0].name, "walls.iff");
}
