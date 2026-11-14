//! The control panel drawn for the Goths' house. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

mod common;

use sha2::{Digest, Sha256};
use sims_ui::command::{Command, Panel};
use sims_ui::frame::Frame;
use sims_ui::hud::{self, Controls};

fn digest(frame: &Frame) -> String {
    let mut hash = Sha256::new();
    for pixel in &frame.rgb {
        hash.update(pixel);
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn the_goths_panel_from_the_original_art() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    assert!(opened.art.missing.is_empty(), "{:?}", opened.art.missing);
    let s = &opened.snapshot;
    // Mortimer, Bella and Cassandra, in the family's order, as the
    // original's portraits are; the clock and funds as it showed them
    // after loading.
    let people: Vec<u16> = s.residents.iter().map(|r| r.object).collect();
    assert_eq!(people, [21, 18, 16]);
    assert_eq!(s.selected, Some(21));
    assert_eq!(
        (s.clock().as_str(), s.money().as_str()),
        ("AM 6:30", "$2,542")
    );
    for r in &s.residents {
        let face = opened.faces.face(r.guid).expect("a face");
        assert_eq!((face.width, face.height), (35, 41));
    }
    let open = Controls {
        expanded: true,
        ..Controls::default()
    };
    // The digests pin the drawing; docs/decisions.md says how it compares
    // with the original's screen.
    let cases = [
        ([800, 600], Controls::default(), "closed 800x600"),
        ([800, 600], open, "open 800x600"),
        ([1280, 720], open, "open 1280x720"),
    ];
    let mut digests = Vec::new();
    for (size, controls, name) in cases {
        let mut frame = Frame::new(size[0] as u32, size[1] as u32, [0, 0, 0]);
        let items = hud::layout(size, s, &controls);
        hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
        digests.push((name, digest(&frame).as_str().to_owned()));
        if name == "open 1280x720" {
            // The needs keep to the right edge; the strip's background
            // fills to it.
            assert_eq!(
                hud::press(&items, [1280 - 387 + 50, 720 - 100 + 15]),
                Some(Command::OpenPanel(Panel::Interests))
            );
            assert_ne!(frame.pixel(1279, 650), Some([0, 0, 0]));
        }
    }
    let expected = [
        (
            "closed 800x600",
            "31c15300c70766950c8fdd609f6193b8e39d143f3fd8db50e4dc2b7c3f3031bc",
        ),
        (
            "open 800x600",
            "4fc31550105bdf347851eb1ad0c365716f71c78a28229e151de427c1d76983bf",
        ),
        (
            "open 1280x720",
            "b06660e2cdbe01705ed96951a1550dc85798ff50659ca1b2f8e8178a66ce2cdc",
        ),
    ];
    let digests: Vec<(&str, &str)> = digests.iter().map(|(n, d)| (*n, d.as_str())).collect();
    assert_eq!(digests, expected);
}
