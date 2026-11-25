use crate::common::{Tree, far};
use sims_content::Install;
use sims_content::ui_graphics::{TABLES, US_ENGLISH, UiGraphics};

/// A 1 by 1 24-bit BMP of the given color.
fn bmp(rgb: [u8; 3]) -> Vec<u8> {
    let mut b = b"BM".to_vec();
    b.extend([0; 8]);
    b.extend(54u32.to_le_bytes());
    b.extend(40u32.to_le_bytes());
    b.extend(1i32.to_le_bytes());
    b.extend(1i32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(24u16.to_le_bytes());
    b.extend([0; 24]);
    b.extend([rgb[2], rgb[1], rgb[0], 0]);
    b
}

fn install(tree: &Tree, loose: bool) -> Install {
    let header = b"const Sint32 kA = 7;\nconst Sint32 kFrench = 3;\n";
    let table = b"{INCLUDE,A.h}\n{BITMAP,kA,\"Dir\\A.bmp\"}\n{BITMAP,kA,\"Dir/AFr.bmp\",kFrench}\n{BITMAP,8,\"Dir/AFr.bmp\",kFrench}\n";
    let empty = b"{INCLUDE,A.h}\n";
    let a = bmp([1, 2, 3]);
    let french = bmp([4, 5, 6]);
    let mut members: Vec<(&str, &[u8])> = vec![
        ("A.h", header),
        ("dir\\a.BMP", &a),
        ("Dir\\AFr.bmp", &french),
    ];
    for name in TABLES {
        members.push((
            name,
            if name == "Res_CPanel.RT" {
                table
            } else {
                empty
            },
        ));
    }
    tree.file("GameData/Behavior.iff", b"")
        .dir("GameData/Objects")
        .file("UIGraphics/UIGraphics.far", &far(&members));
    if loose {
        tree.file("UIGraphics/Dir/A.bmp", &bmp([9, 9, 9]));
    }
    Install::detect(&tree.0).unwrap()
}

#[test]
fn resource_ids_resolve_by_language_through_the_archive() {
    let tree = Tree::new("ui-archive");
    let ui = UiGraphics::open(&install(&tree, false)).unwrap();
    // A numeric ID with a language only the included header names.
    assert_eq!(ui.len(), 3);
    assert_eq!(ui.path(8, 3), Some("Dir/AFr.bmp"));
    assert_eq!(ui.path(7, US_ENGLISH), Some("Dir/A.bmp"));
    assert_eq!(ui.path(7, 3), Some("Dir/AFr.bmp"));
    assert_eq!(ui.path(8, US_ENGLISH), None);
    let image = ui.bitmap(7, US_ENGLISH).unwrap().unwrap();
    assert_eq!(image.pixel(0, 0), Some([1, 2, 3, 255]));
    assert_eq!(
        ui.bitmap(7, 3).unwrap().unwrap().pixel(0, 0),
        Some([4, 5, 6, 255])
    );
    assert!(ui.bitmap(8, 1).unwrap().is_none());
    assert!(ui.read("Dir/Missing.bmp").is_err());
    assert!(matches!(
        ui.read("../GameData/Behavior.iff"),
        Err(sims_content::Error::Unsupported(_))
    ));
}

#[test]
fn a_loose_file_wins_over_the_archive() {
    let tree = Tree::new("ui-loose");
    let ui = UiGraphics::open(&install(&tree, true)).unwrap();
    let image = ui.bitmap(7, US_ENGLISH).unwrap().unwrap();
    assert_eq!(image.pixel(0, 0), Some([9, 9, 9, 255]));
}

/// Needs the game: set `SIMS_INSTALL` to the install folder.
#[test]
fn every_installed_ui_bitmap_decodes() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to read the install's UI art");
        return;
    };
    let install = Install::detect(std::path::Path::new(&root)).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    // The one row the original cannot resolve either.
    assert_eq!(ui.unresolved, ["kStudiownBulldoze"]);
    assert_eq!(ui.len(), 647);
    let mut decoded = 0;
    let mut missing = Vec::new();
    for id in 0..10_000 {
        for language in 0..=20 {
            if let Some(path) = ui.path(id, language)
                && (language == 0 || ui.path(id, 0) != Some(path))
            {
                match ui.image(path) {
                    Ok(image) => {
                        assert!(image.width > 0 && image.height > 0);
                        decoded += 1;
                    }
                    Err(sims_content::Error::Missing { .. }) => missing.push(path.to_owned()),
                    Err(e) => panic!("{path}: {e}"),
                }
            }
        }
    }
    // Eleven rows name files the install does not have (leftovers such as
    // `CPanel/Backgrounds/BuyBack.bmp`); every other row decodes.
    assert_eq!(missing.len(), 11, "{missing:?}");
    assert_eq!(decoded, 647 - 11);
    // The control panel's background (kUniversalBkgTGA) has an alpha
    // channel, and the needs button (kMoodBtn) is a strip of four cells.
    let back = ui.bitmap(2037, US_ENGLISH).unwrap().unwrap();
    assert_eq!((back.width, back.height), (220, 183));
    assert!(back.rgba.chunks(4).any(|p| p[3] == 0));
    assert_eq!(ui.path(4500, US_ENGLISH), Some("CPanel/Buttons/Mood.bmp"));
    let mood = ui.bitmap(4500, US_ENGLISH).unwrap().unwrap();
    assert_eq!(mood.width % 4, 0);
}
