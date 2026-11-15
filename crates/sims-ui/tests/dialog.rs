//! A dialog drawn from a real script's strings. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

mod common;

use sha2::{Digest, Sha256};
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, Index, Install};
use sims_formats::iff::Iff;
use sims_ui::art::Art;
use sims_ui::command::Command;
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::{Frame, Rect};

/// The object whose script shows "Magic Town" a tick after the Goths'
/// house loads, and the operands of that Dialog instruction (read from
/// a snapshot of `sims boot`).
const MAGIC_TOWN: u32 = 0xD557_147B;
const OPERANDS: [u8; 8] = [0, 0, 2, 0, 0, 0, 1, 6];

#[test]
fn magic_town_as_the_original_shows_it() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw a dialog");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let object = catalog.object(MAGIC_TOWN).expect("the Magic Town object");
    let iff = Iff::parse(index.resources[object.resource].read().unwrap()).unwrap();
    let labels = Labels::load(&install).unwrap();
    let message = Message::from_file(&iff, &labels, 360, &OPERANDS)
        .unwrap()
        .expect("a message dialog");
    assert_eq!(message.title, "Magic Town");
    assert_eq!(message.buttons, ["OK"]);
    let icon = message.icon.as_ref().expect("an icon");
    assert_eq!((icon.width, icon.height), (124, 124));

    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &dialog::RESOURCES).unwrap();
    assert!(art.missing.is_empty(), "{:?}", art.missing);
    // Where the original put it at 800 by 600, measured on its screen.
    let layout = dialog::layout([800, 600], &message, &art).unwrap();
    assert_eq!(layout.rect, Rect::new(116, 162, 567, 276));
    assert_eq!(layout.title, [403, 183]);
    assert_eq!(layout.icon, Some([137, 183]));
    let starts: Vec<[i32; 2]> = layout.lines.iter().map(|(_, at)| *at).collect();
    assert_eq!(
        starts,
        [
            [279, 228],
            [279, 251],
            [279, 274],
            [279, 297],
            [137, 320],
            [137, 343]
        ]
    );
    assert_eq!(
        layout.lines[0].0,
        "Recently, there's been talk of strange happenings"
    );
    assert_eq!(layout.buttons, [Rect::new(349, 384, 100, 33)]);
    assert_eq!(
        dialog::press(&layout, [400, 400]),
        Some(Command::AnswerDialog {
            caller: 360,
            button: 0
        })
    );

    // The digest pins the drawing; docs/decisions.md says how it compares
    // with the original's screen.
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    dialog::draw(&mut frame, &art, &message, &layout, None);
    let mut hash = Sha256::new();
    for pixel in frame.rgb() {
        hash.update(pixel);
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "9075999bceded9f98e688ddd10d4c87f6419c145215325c20e3205084cbf7bed"
    );
}
