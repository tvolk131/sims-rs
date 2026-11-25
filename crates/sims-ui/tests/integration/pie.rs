//! The pie menu placed as the original placed it. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

use crate::common;

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::pie::{self, Entry};

/// Two menus the input track read from the running original at 800 by
/// 600 on the Goth house (`tools/harness/input.py`): where the click was,
/// the entry, and the window's origin and the entry's box as the
/// original's menu window held them.
const LOGGED: [([i32; 2], &str, [i32; 2], Rect); 2] = [
    ([432, 230], "Sit", [320, 118], Rect::new(418, 141, 29, 29)),
    (
        [432, 254],
        "Go Here",
        [320, 142],
        Rect::new(397, 165, 70, 29),
    ),
];

#[test]
fn menus_open_where_the_original_opened_them() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to lay out a pie menu");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &pie::RESOURCES).unwrap();
    assert!(art.missing.is_empty(), "{:?}", art.missing);
    let face = art.image(pie::FACE).unwrap();
    assert_eq!([face.width, face.height], [201, 201]);
    for (at, label, origin, rect) in LOGGED {
        let entries = [Entry {
            label: label.into(),
            enabled: true,
        }];
        let layout = pie::layout([800, 600], at, "", &entries, &art).unwrap();
        assert_eq!([layout.window.x, layout.window.y], origin, "{label}");
        assert_eq!(layout.labels[0].rect, rect, "{label}");
        assert_eq!(layout.center, at);
        assert!(!layout.moved);
        // Pointing up at it highlights it; the middle does not.
        assert_eq!(pie::hit(&layout, [at[0], at[1] - 50]), Some(0));
        assert_eq!(pie::hit(&layout, at), None);
        let mut frame = Frame::new(800, 600, [0; 3]);
        pie::draw(&mut frame, &art, &layout, None, Some(0), false);
        // The box is the people panel's dark blue.
        let inside = frame.pixel(rect.x + 2, rect.y + rect.height - 3).unwrap();
        assert_eq!(inside, [0, 40, 140]);
    }
}

/// The fridge's menu in track 2's recording of a snack order
/// (goth-snack-order-20261115): pressed at (176, 198), its four entries
/// reach further left than the press is from the screen's edge, so the
/// window is moved to x 0 (0x5ebb10) and its middle lands 24 pixels right
/// of the press. The boxes are the original's menu window's.
#[test]
fn a_menu_near_the_edge_moves_onto_the_screen_as_the_original_moved_it() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to lay out a pie menu");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &pie::RESOURCES).unwrap();
    let logged = [
        ("Have A Snack", Rect::new(144, 109, 113, 29)),
        ("Have Quick Breakfast", Rect::new(260, 184, 180, 29)),
        ("Have Breakfast", Rect::new(135, 258, 130, 29)),
        ("Serve Breakfast", Rect::new(4, 184, 136, 29)),
    ];
    let entries: Vec<Entry> = logged
        .iter()
        .map(|(label, _)| Entry {
            label: (*label).into(),
            enabled: true,
        })
        .collect();
    let layout = pie::layout([800, 600], [176, 198], "", &entries, &art).unwrap();
    assert_eq!([layout.window.x, layout.window.y], [0, 86]);
    assert!(layout.moved);
    assert_eq!(layout.center, [200, 198]);
    let rects: Vec<Rect> = layout.labels.iter().map(|l| l.rect).collect();
    assert_eq!(rects, logged.map(|(_, rect)| rect));
}
