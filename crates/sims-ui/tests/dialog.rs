//! A dialog drawn from a real script's strings. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

mod common;

use sha2::{Digest, Sha256};
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, Index, Install, Neighborhood};
use sims_formats::iff::Iff;
use sims_ui::art::Art;
use sims_ui::command::Command;
use sims_ui::dialog::{self, Labels, Message};
use sims_ui::frame::{Frame, Rect};
use sims_vm::boot::{Frames, Lot, StepError};

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
    // A line keeps the space it breaks after, as the original's do.
    assert_eq!(
        layout.lines[0].0,
        "Recently, there's been talk of strange happenings "
    );
    assert_eq!(layout.buttons, [Rect::new(349, 384, 100, 33)]);
    assert_eq!(
        dialog::press(&layout, [400, 400]),
        Some(Command::AnswerDialog {
            caller: 360,
            button: 0
        })
    );

    // The digest pins the drawing; docs/decisions/ says how it compares
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

/// The Yes/No "Help System" dialog the Goths' house asks in its second
/// lot update, as the original laid it out at 800 by 600 (measured on its
/// screen: docs/decisions/20261115-help-system-dialog-matches-original.md).
#[test]
fn the_help_system_as_the_original_shows_it() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw a dialog");
        return;
    };
    let house = root.join(common::GOTH_HOUSE);
    let frames = Frames {
        after: Vec::new(),
        every: Vec::new(),
    };
    let mut lot = Lot::load(&root, &house, &[], frames).unwrap();
    lot.wait_for_answers();
    // The first update runs through; the second waits for the player.
    lot.step().expect("the first update");
    let stop = lot.step().expect_err("the second update asks");
    assert!(matches!(stop, StepError::Asking { tick: 2 }), "{stop:?}");
    let asked = &lot.asking().unwrap().world;
    let record = dialog::pending(asked).expect("the dialog asked");
    let install = Install::detect(&root).unwrap();
    let mut index = Index::build(&install).unwrap();
    let neighborhood = Neighborhood::open(house.parent().unwrap().parent().unwrap()).unwrap();
    if let Some(characters) = neighborhood.characters().unwrap() {
        index.add_characters(&characters).unwrap();
    }
    let labels = Labels::load(&install).unwrap();
    let message = Message::read(asked, &index, lot.catalog(), &labels, record)
        .unwrap()
        .expect("a message dialog");
    assert_eq!(message.title, "Help System");
    assert_eq!(message.buttons, ["Yes", "No"]);
    let icon = message.icon.as_ref().expect("an icon");
    assert_eq!((icon.width, icon.height), (133, 103));

    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &dialog::RESOURCES).unwrap();
    let layout = dialog::layout([800, 600], &message, &art).unwrap();
    assert_eq!(layout.rect, Rect::new(156, 185, 488, 230));
    assert_eq!(layout.title, [404, 206]);
    assert_eq!(layout.icon, Some([177, 206]));
    let starts: Vec<[i32; 2]> = layout.lines.iter().map(|(_, at)| *at).collect();
    assert_eq!(starts, [[328, 251], [328, 274], [328, 297], [177, 320]]);
    // The last line, under the icon, by its digest: the game's prose is
    // not kept in the repository (docs/legal-and-provenance.md).
    assert_eq!(
        format!("{:x}", Sha256::digest(layout.lines[3].0.as_bytes())),
        "c6da3959e6ad3235004d9662458a497876cafddada3bd520d475c581b39337d0"
    );
    assert_eq!(
        layout.buttons,
        [Rect::new(259, 361, 100, 33), Rect::new(441, 361, 100, 33)]
    );
    assert_eq!(
        dialog::press(&layout, [450, 370]),
        Some(Command::AnswerDialog {
            caller: record.caller,
            button: 1
        })
    );
}
