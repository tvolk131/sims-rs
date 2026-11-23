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

    // The digest pins the drawing using the original's 9-slice `PopupInfoTiles.bmp`
    // (3008, `0x4efe3b`, `0x524603..0x524645`); see `docs/decisions/20261121-options-and-dialogs.md`.
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    dialog::draw(&mut frame, &art, &message, &layout, None);
    let mut hash = Sha256::new();
    for pixel in frame.rgb() {
        hash.update(pixel);
    }
    assert_eq!(
        format!("{:x}", hash.finalize()),
        "f2a4498bc09ed14faf3b7d198d5e8fd57dec18f52d1545ae8109f27c91885eaf"
    );
}

#[test]
fn native_dialog_chrome_loads_nine_slice_and_sim_stub_art() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to verify dialog chrome art");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    let ids = [
        dialog::BUTTON,
        dialog::BACKGROUND,
        dialog::GENERIC_FRAME,
        dialog::ICON_STUB,
    ];
    let art = Art::load(&ui, US_ENGLISH, &ids).unwrap();
    assert!(art.missing.is_empty(), "{:?}", art.missing);
    let bg = art.image(dialog::BACKGROUND).unwrap();
    assert_eq!((bg.width, bg.height), (36, 36));
    let gen_dlg = art.image(dialog::GENERIC_FRAME).unwrap();
    assert_eq!((gen_dlg.width, gen_dlg.height), (30, 30));
    let stub = art.image(dialog::ICON_STUB).unwrap();
    assert_eq!((stub.width, stub.height), (133, 103));

    // Confirm a 45x45 portrait gets SimStub.BMP backing and prompt renders cTSWinTextEdit2.
    let portrait = sims_formats::bitmap::Image {
        width: 45,
        height: 45,
        rgba: [180, 90, 40, 255].repeat(45 * 45),
    };
    let msg = Message::prompt(
        "Name",
        "Enter a name",
        "Bella",
        vec!["OK".into(), "Cancel".into()],
    )
    .with_icon(Some(portrait));
    let laid = dialog::layout([800, 600], &msg, &art).unwrap();
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let mut state = dialog::DialogState::default();
    state.pressed_button = Some(0);
    dialog::draw_with_state(&mut frame, &art, &msg, &laid, None, &state);
    // Top-left corner of frame matches PopupInfoTiles.bmp's highlight pixel (149, 149, 204).
    assert_eq!(frame.pixel(laid.rect.x, laid.rect.y), Some([149, 149, 204]));
    // 45x45 portrait centered inside 133x103 SimStub.BMP at offset (+44, +29).
    let icon_at = laid.icon.unwrap();
    assert_eq!(
        frame.pixel(icon_at[0] + 44, icon_at[1] + 29),
        Some([180, 90, 40])
    );

    // Native `cTSWinGenDlg` Neighborhood Save prompt (`0x5e542f..0x5e589b`,
    // slot 5 -> `PopupInfoTiles.bmp` 3008 at `0x5e5441`) matches the Complete
    // Collection 1.91 capture at 800x600: exact 396x135 frame with upper-left
    // title at (rect.x + 16, rect.y + 16) and PopupInfoTiles.bmp frame pixels.
    let opt_texts = sims_ui::options::OptionsTexts::load(&install).unwrap();
    let labels = Labels::load(&install).unwrap();
    assert_eq!(labels.yes_key(), 'y');
    assert_eq!(labels.no_key(), 'n');
    let nbhd_confirm = Message::confirm(
        opt_texts.confirm_save.0.clone(),
        opt_texts.confirm_save.1.clone(),
        vec![
            labels.get(3).to_owned(),
            labels.get(4).to_owned(),
            labels.get(2).to_owned(),
        ],
    )
    .with_frame(dialog::FrameStyle::Generic);
    let gen_layout = dialog::layout([800, 600], &nbhd_confirm, &art).unwrap();
    assert_eq!(gen_layout.rect.width, 396);
    assert_eq!(gen_layout.rect.height, 135);
    assert_eq!(
        gen_layout.title,
        [gen_layout.rect.x + 16, gen_layout.rect.y + 16]
    );
    assert_eq!(gen_layout.buttons.len(), 3);
    dialog::draw(&mut frame, &art, &nbhd_confirm, &gen_layout, None);
    // Top-left corner matches PopupInfoTiles.bmp (3008), not GenDlg.bmp (22).
    assert_eq!(
        frame.pixel(gen_layout.rect.x, gen_layout.rect.y),
        Some([149, 149, 204])
    );

    // Edit-mode Save Card (`cWinEditLotInfo`, `0x50d68a..0x50de10`) lays out
    // both Lot Name and multiline Lot Description `cTSWinTextEdit2` fields,
    // includes the 3-button confirmation prompt (`0x50dcc8..0x50dcec`), and
    // supports Tab switching and newline insertion in Lot Description.
    let mut art_with_card = art;
    art_with_card.load_optional(&ui, US_ENGLISH, sims_ui::options::id::SAVE_CARD);
    let card_art = art_with_card
        .image(sims_ui::options::id::SAVE_CARD)
        .cloned();
    assert!(card_art.is_some());
    let mut save_card = Message::save_card(
        "Old Name",
        "Old Desc",
        Some(opt_texts.confirm_save.1.clone()),
        vec![
            labels.get(3).to_owned(),
            labels.get(4).to_owned(),
            labels.get(2).to_owned(),
        ],
        card_art,
    );
    let card_layout = dialog::layout([800, 600], &save_card, &art_with_card).unwrap();
    assert!(card_layout.icon.is_some());
    let name_rect = card_layout.entry.unwrap();
    let card_desc_rect = card_layout.secondary_entry.unwrap();
    // `0x50da8f..0x50dac3` and `0x50dbb2..0x50dc16` with installed 337x164 card art.
    assert_eq!((name_rect.width, name_rect.height), (329, 20));
    assert_eq!((card_desc_rect.width, card_desc_rect.height), (329, 96));
    assert_eq!(card_layout.lines.len(), 1);
    assert_eq!(card_layout.buttons.len(), 3);
    let mut card_state = dialog::DialogState::for_message(&save_card);
    // Initial selection on Lot Name replaces on first typed char.
    assert!(card_state.insert(&mut save_card, 'N'));
    assert!(card_state.insert(&mut save_card, 'e'));
    assert!(card_state.insert(&mut save_card, 'w'));
    assert_eq!(save_card.entry.as_deref(), Some("New"));
    // Tab switches to multiline Lot Description with initial all-selection.
    assert!(card_state.switch_field(&save_card));
    assert!(card_state.is_multiline(&save_card));
    assert!(card_state.insert(&mut save_card, 'L'));
    assert!(card_state.insert(&mut save_card, '1'));
    assert!(card_state.insert(&mut save_card, '\n'));
    assert!(card_state.insert(&mut save_card, 'L'));
    assert!(card_state.insert(&mut save_card, '2'));
    assert_eq!(save_card.secondary_entry.as_deref(), Some("L1\nL2"));
    dialog::draw_with_state(
        &mut frame,
        &art_with_card,
        &save_card,
        &card_layout,
        None,
        &card_state,
    );

    // Row-aware multiline caret hit-testing and navigation (`0x5f2475..0x5f2522`,
    // `0x5f29f2..0x5f2c13`): clicking the start of wrapped row 1 keeps the caret
    // on row 1 (not row 0), and Home/End/Up/Down navigate per visual row.
    let wrapped_text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
    let mut wrap_card = Message::save_card(
        "Wrap Lot",
        wrapped_text,
        None,
        vec![labels.get(1).to_owned()],
        art_with_card
            .image(sims_ui::options::id::SAVE_CARD)
            .cloned(),
    );
    let wrap_layout = dialog::layout([800, 600], &wrap_card, &art_with_card).unwrap();
    let desc_rect = wrap_layout.secondary_entry.unwrap();
    let line_h = art_with_card.font(12).unwrap().line_height;
    let mut wrap_state = dialog::DialogState::for_message(&wrap_card);
    // Click near the left edge of row 1 (second visual wrapped row).
    let row1_click = [desc_rect.x + 2, desc_rect.y + 3 + line_h + 2];
    assert!(wrap_state.click_entry(&wrap_layout, &wrap_card, &art_with_card, row1_click));
    assert_eq!(wrap_state.active_field, 1);
    assert_eq!(wrap_state.caret_line, Some(1));
    let row1_start = wrap_state.caret.unwrap();
    assert!(row1_start > 0);
    // Render with caret visible and verify caret pixels appear on row 1, not row 0.
    let mut before_caret = Frame::new(800, 600, [0, 0, 0]);
    wrap_state.caret_visible = false;
    dialog::draw_with_state(
        &mut before_caret,
        &art_with_card,
        &wrap_card,
        &wrap_layout,
        None,
        &wrap_state,
    );
    let mut with_caret = Frame::new(800, 600, [0, 0, 0]);
    wrap_state.caret_visible = true;
    dialog::draw_with_state(
        &mut with_caret,
        &art_with_card,
        &wrap_card,
        &wrap_layout,
        None,
        &wrap_state,
    );
    let mut row0_diff = 0;
    let mut row1_diff = 0;
    for y in (desc_rect.y + 3)..(desc_rect.y + 3 + line_h) {
        for x in desc_rect.x..(desc_rect.x + desc_rect.width) {
            if before_caret.pixel(x, y) != with_caret.pixel(x, y) {
                row0_diff += 1;
            }
        }
    }
    for y in (desc_rect.y + 3 + line_h)..(desc_rect.y + 3 + line_h * 2) {
        for x in desc_rect.x..(desc_rect.x + desc_rect.width) {
            if before_caret.pixel(x, y) != with_caret.pixel(x, y) {
                row1_diff += 1;
            }
        }
    }
    assert_eq!(row0_diff, 0);
    assert!(row1_diff > 0);

    // End moves to end of row 1 (not end of entire multi-row text); Home returns to row1_start.
    assert!(wrap_state.end_in_layout(&wrap_card, Some(&wrap_layout), Some(&art_with_card)));
    assert_eq!(wrap_state.caret_line, Some(1));
    let row1_end = wrap_state.caret.unwrap();
    assert!(row1_end > row1_start);
    assert!(wrap_state.home_in_layout(&wrap_card, Some(&wrap_layout), Some(&art_with_card)));
    assert_eq!(wrap_state.caret, Some(row1_start));
    assert_eq!(wrap_state.caret_line, Some(1));
    // Up moves from row 1 to row 0, and Down returns to row 1.
    assert!(wrap_state.move_up(&wrap_card, Some(&wrap_layout), &art_with_card));
    assert_eq!(wrap_state.caret_line, Some(0));
    assert!(wrap_state.move_down(&wrap_card, Some(&wrap_layout), &art_with_card));
    assert_eq!(wrap_state.caret_line, Some(1));
    // Inserting at row1_start keeps the rest of the text intact.
    assert!(wrap_state.insert(&mut wrap_card, '!'));
    assert!(wrap_card.secondary_entry.as_deref().unwrap().contains('!'));

    // `0x5f37f4..0x5f38cd`: vertical caret movement carries character column
    // (not pixel distance) across proportional-width lines and clamps to row length.
    let col_card = Message::save_card(
        "Col Lot",
        "WWWW\niiiiiiii",
        None,
        vec![labels.get(1).to_owned()],
        art_with_card
            .image(sims_ui::options::id::SAVE_CARD)
            .cloned(),
    );
    let col_layout = dialog::layout([800, 600], &col_card, &art_with_card).unwrap();
    let mut col_state = dialog::DialogState::for_message(&col_card);
    assert!(col_state.switch_field(&col_card));
    col_state.selection = None;
    col_state.caret = Some(2);
    col_state.caret_line = Some(0);
    assert!(col_state.move_down(&col_card, Some(&col_layout), &art_with_card));
    assert_eq!(col_state.caret_line, Some(1));
    assert_eq!(col_state.caret, Some(5 + 2));

    // `0x5f27ae..0x5f2887`: dragging across text inside `cTSWinTextEdit2`
    // updates the selection range, and typing replaces the selected text.
    let mut drag_card = Message::save_card(
        "abc",
        "line1\nline2",
        None,
        vec![labels.get(1).to_owned()],
        art_with_card
            .image(sims_ui::options::id::SAVE_CARD)
            .cloned(),
    );
    let drag_layout = dialog::layout([800, 600], &drag_card, &art_with_card).unwrap();
    let drag_name = drag_layout.entry.unwrap();
    let drag_desc = drag_layout.secondary_entry.unwrap();
    let mut drag_state = dialog::DialogState::for_message(&drag_card);
    assert!(drag_state.click_entry(
        &drag_layout,
        &drag_card,
        &art_with_card,
        [drag_name.x + 2, drag_name.y + 10],
    ));
    assert_eq!(drag_state.selection, None);
    assert!(drag_state.drag_entry(
        &drag_layout,
        &drag_card,
        &art_with_card,
        [drag_name.x + drag_name.width - 2, drag_name.y + 10],
    ));
    assert_eq!(drag_state.selection, Some((0, 3)));
    assert!(drag_state.release_entry());
    assert!(drag_state.insert(&mut drag_card, 'X'));
    assert_eq!(drag_card.entry.as_deref(), Some("X"));

    // Multiline drag selection across both rows replaces the full range.
    assert!(drag_state.click_entry(
        &drag_layout,
        &drag_card,
        &art_with_card,
        [drag_desc.x + 2, drag_desc.y + 4],
    ));
    assert!(drag_state.drag_entry(
        &drag_layout,
        &drag_card,
        &art_with_card,
        [
            drag_desc.x + drag_desc.width - 2,
            drag_desc.y + drag_desc.height - 4
        ],
    ));
    assert_eq!(drag_state.selection, Some((0, 11)));
    assert!(drag_state.release_entry());
    assert!(drag_state.insert(&mut drag_card, 'Y'));
    assert_eq!(drag_card.secondary_entry.as_deref(), Some("Y"));
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

#[test]
fn ordinary_dialogs_format_tokens_resolve_icon_modes_and_caller_title_fallback() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test dialog formatting and icon modes");
        return;
    };
    let house = root.join(common::GOTH_HOUSE);
    let frames = Frames {
        after: Vec::new(),
        every: Vec::new(),
    };
    let lot = Lot::load(&root, &house, &[], frames).unwrap();
    let install = Install::detect(&root).unwrap();
    let mut index = Index::build(&install).unwrap();
    let neighborhood = Neighborhood::open(house.parent().unwrap().parent().unwrap()).unwrap();
    if let Some(characters) = neighborhood.characters().unwrap() {
        index.add_characters(&characters).unwrap();
    }
    let labels = Labels::load(&install).unwrap();
    let careers = sims_ui::household::Careers::load(&index).unwrap();
    let fame = sims_ui::household::Fame::load(&install).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    let art = Art::load(&ui, US_ENGLISH, &dialog::RESOURCES).unwrap();

    // Pick a real Goth household Sim (Mortimer) and a real buyable object on the lot.
    let base_world = lot.world();
    let sim_obj = base_world
        .objects
        .values()
        .find(|o| o.person.is_some())
        .expect("a Sim on Goth lot");
    let sim_id = sim_obj.id;
    let sim_guid = sim_obj.guid;
    let other_neighbor = base_world
        .neighbors
        .slots
        .iter()
        .flatten()
        .find(|n| n.guid != sim_guid)
        .expect("another neighbor record");
    let mut probe_thumbs = sims_ui::thumbnail::Thumbnails::default();
    let non_person_obj = base_world
        .objects
        .values()
        .find(|o| {
            o.person.is_none()
                && lot.catalog().object(o.guid).is_some()
                && probe_thumbs.read(&index, lot.catalog(), o.guid).is_ok()
                && probe_thumbs
                    .item(o.guid)
                    .and_then(|i| i.thumbnail.as_ref())
                    .is_some()
        })
        .expect("a non-person object with a thumbnail on Goth lot");

    let faces =
        sims_ui::household::Faces::load(&index, lot.catalog(), &[sim_guid, other_neighbor.guid])
            .unwrap();
    let sim_name = faces
        .name(sim_guid)
        .expect("Sim has a localized name")
        .to_owned();
    let other_name = faces
        .name(other_neighbor.guid)
        .expect("Neighbor has a localized name")
        .to_owned();

    // Find a real STR# 301 entry in the install that uses `$Me` / `$Local` / `$Object` / `$TimeLocal` / `$Job`!
    let mut found_token_dialog = None;
    for (&guid, &idx) in &lot.catalog().by_guid {
        let obj = &lot.catalog().objects[idx];
        let Some(res) = index.resources.get(obj.resource) else {
            continue;
        };
        let Ok(bytes) = res.read() else { continue };
        let Ok(iff) = Iff::parse(bytes) else { continue };
        let Some(chunk) = iff.get(b"STR#", dialog::STRINGS) else {
            continue;
        };
        let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk)) else {
            continue;
        };
        for (i, entry) in table.entries.iter().enumerate() {
            if entry.language == US_ENGLISH
                && (entry.text.contains("$Me") || entry.text.contains("$Local:"))
                && entry.text.contains('$')
                && i < 250
            {
                found_token_dialog = Some((guid, (i + 1) as u8, entry.text.clone()));
                break;
            }
        }
        if found_token_dialog.is_some() {
            break;
        }
    }
    let (code_owner_guid, msg_str_id, raw_template) = found_token_dialog
        .expect("Complete Collection must contain a real STR# 301 dialog with $Me/$Local tokens");

    // Clone world (read-only w.r.t. simulation) to stage synthetic dialog records
    // pointing to real Complete Collection objects/characters and verify formatting,
    // caller title fallback (operands[6] == 0), and all icon modes 0, 1, 2, 4!
    let mut world = base_world.clone();
    let type_slot: u16 = 9999;
    world.types.insert(type_slot, code_owner_guid);
    if let Some(caller) = world.objects.get_mut(&sim_id) {
        if let Some(person) = caller.person.as_mut()
            && person.data.len() > 65
        {
            person.data[56] = 1; // Business career
            person.data[57] = 1;
        }
        caller.stack.push(sims_formats::saved_objects::Frame {
            stack_object: non_person_obj.id as i16,
            tree: 4096,
            node: 0,
            parameters: Vec::new(),
            locals: vec![42, 500, other_neighbor.id, 9, 1, 2],
            primitive_state: 0,
            code_owner_type: type_slot as i16,
        });
    }

    let mut families = std::collections::BTreeMap::new();
    let other_fam_id = i32::from(other_neighbor.data.get(61).copied().unwrap_or(0)).max(1);
    families.insert(
        other_fam_id,
        dialog::FamilyInfo {
            name: "Newbie".into(),
            assets: 12_345,
        },
    );
    let mut thumbs = sims_ui::thumbnail::Thumbnails::default();
    let mut ctx = dialog::DialogContext {
        faces: Some(&faces),
        thumbnails: Some(&mut thumbs),
        art: Some(&art),
        careers: Some(&careers),
        fame: Some(&fame),
        families: Some(&families),
    };

    // 1. Real VM-emitted dialog check: step the Goth house to tick 2 where the
    //    real VM executes a Dialog instruction ("Help System"), and verify
    //    `Message::read_with_context` produces the exact title, body, and icon.
    let mut live_lot = Lot::load(
        &root,
        &house,
        &[],
        Frames {
            after: Vec::new(),
            every: Vec::new(),
        },
    )
    .unwrap();
    live_lot.wait_for_answers();
    live_lot.step().expect("tick 1");
    let _ = live_lot.step().expect_err("tick 2 asks Help System");
    let live_world = &live_lot.asking().unwrap().world;
    let live_rec = dialog::pending(live_world).expect("VM-emitted Help System dialog");
    let live_msg = Message::read_with_context(
        live_world,
        &index,
        live_lot.catalog(),
        &labels,
        live_rec,
        &mut ctx,
    )
    .unwrap()
    .expect("VM-emitted dialog read via read_with_context");
    assert_eq!(live_msg.title, "Help System");
    assert_eq!(live_msg.buttons, ["Yes", "No"]);
    assert_eq!(
        live_msg.icon.as_ref().map(|i| (i.width, i.height)),
        Some((133, 103))
    );

    // 2. Native 1.91 token semantics (`0x44bf66`, `0x44c034`, `0x44c861`, `0x44c9c4`):
    //    - `$FamilyAssets:N` uses family ID in local `N` and consumes `:N`
    //    - `$Family` resolves stack neighbor's family name (not caller display name)
    //    - `$Grade:N` validates `N < locals.len()` and passes `N` to `Careers::grade`
    //    - `$Job` / `$Job:0` (single suffix -> caller career+level) vs `$Job:4:5` (both locals)
    let mut fmt_world = world.clone();
    if let Some(nb) = fmt_world.neighbors.get_mut(other_neighbor.id)
        && nb.data.len() > 61
    {
        nb.data[61] = other_fam_id as i16;
    }
    if let Some(caller) = fmt_world.objects.get_mut(&sim_id)
        && let Some(frame) = caller.stack.last_mut()
    {
        frame.stack_object = other_neighbor.id;
        // locals: [0]=other_fam_id, [1]=500, [2]=other_neighbor.id, [3]=13 (1pm), [4]=2 (Entertainment), [5]=3
        frame.locals = vec![other_fam_id as i16, 500, other_neighbor.id, 13, 2, 3];
    }
    let caller_job = careers.title(1, 1, false).expect("career 1 level 1");
    let local_job = careers.title(2, 3, false).expect("career 2 level 3");
    let grade_2 = careers.grade(2);
    let formatted_all = dialog::format_text(
        "fam=$Family assets=$FamilyAssets:0 grade=$Grade:2 time=$TimeLocal:3 cost=$$Local:1 cur=$Job single=$Job:4 both=$Job:4:5",
        sim_id,
        &fmt_world,
        &index,
        lot.catalog(),
        &ctx,
    );
    assert_eq!(
        formatted_all,
        format!(
            "fam=Newbie assets=12345 grade={grade_2} time= 1:00 pm cost=$500 cur={caller_job} single={caller_job} both={local_job}"
        )
    );

    // 3. Exact 1.91 icon-mode dispatch (`0x436279..0x4363b5`) + caller title
    //    fallback (`operands[6] == 0`):
    //    - mode 0: resolves `$Object`/`$Neighbor` or stack object's master icon,
    //      and leaves `None` when neither token nor stack object supplies one.
    //    - mode 1: leaves `icon == None`.
    //    - mode 2: resolves `world.neighbors.get(stack_object)` DIRECTLY (`0x4362f4`)
    //      even when an unrelated live object shares the EXACT SAME numeric ID!
    //    - mode 4: formats operand 1 text and leaves `icon == None`.
    for mode in [0u8, 1, 2, 4] {
        let mut test_world = world.clone();
        if mode == 2 {
            let colliding_id = other_neighbor.id as u16;
            let mut collider = non_person_obj.clone();
            collider.id = colliding_id;
            test_world.objects.insert(colliding_id, collider);
            if let Some(caller) = test_world.objects.get_mut(&sim_id)
                && let Some(frame) = caller.stack.last_mut()
            {
                frame.stack_object = other_neighbor.id;
            }
        }
        let mut operands = [0u8; 8];
        operands[2] = msg_str_id;
        operands[5] = 0; // kind 0 (OK)
        operands[6] = 0; // title fallback -> caller's display name (`sim_name`)
        operands[7] = (mode & 7) << 1;
        let rec = sims_sim::Dialog {
            caller: sim_id,
            operands,
            status: 0,
        };
        let msg =
            Message::read_with_context(&test_world, &index, lot.catalog(), &labels, &rec, &mut ctx)
                .unwrap()
                .expect("dialog message");
        assert_eq!(
            msg.title, sim_name,
            "operands[6]==0 must fall back to caller display name (mode {mode})"
        );
        assert!(
            !msg.text.contains("$Me")
                && !msg.text.contains("$Local")
                && !msg.text.contains("$Object")
                && !msg.text.contains("$Neighbor")
                && !msg.text.contains("$TimeLocal")
                && !msg.text.contains("$Job"),
            "raw token leaked in mode {mode}: raw={raw_template:?} -> formatted={:?}",
            msg.text
        );
        match mode {
            0 => assert!(
                msg.icon.is_some(),
                "icon mode 0 with valid stack object must resolve an icon"
            ),
            1 | 4 => assert!(
                msg.icon.is_none(),
                "native 1.91 icon mode {mode} leaves icon None (0x436279)"
            ),
            2 => {
                let icon = msg
                    .icon
                    .as_ref()
                    .expect("icon mode 2 must resolve neighbor portrait");
                assert_eq!(
                    (icon.width, icon.height),
                    (45, 45),
                    "mode 2 must return 45x45 neighbor portrait even on object/neighbor ID collision"
                );
                let laid = dialog::layout([800, 600], &msg, &art).unwrap();
                assert!(laid.icon.is_some());
                if let Some(dir) =
                    std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from)
                {
                    let mut frame = Frame::new(800, 600, [24, 34, 48]);
                    dialog::draw(&mut frame, &art, &msg, &laid, None);
                    let _ = std::fs::create_dir_all(&dir);
                    let _ =
                        std::fs::write(dir.join("dialog_mode2_portrait_800x600.png"), frame.png());
                }
            }
            _ => unreachable!(),
        }
    }

    // Mode 0 with NO `$Object`/`$Neighbor` token and NO stack object (stack_object = 0)
    // must leave `icon == None` (no spurious fallback to caller!).
    let mut empty_stack_world = world.clone();
    if let Some(caller) = empty_stack_world.objects.get_mut(&sim_id)
        && let Some(frame) = caller.stack.last_mut()
    {
        frame.stack_object = 0;
    }
    let rec_mode0_empty = sims_sim::Dialog {
        caller: sim_id,
        operands: [0, 0, msg_str_id, 0, 0, 0, 0, 0],
        status: 0,
    };
    let msg_empty = Message::read_with_context(
        &empty_stack_world,
        &index,
        lot.catalog(),
        &labels,
        &rec_mode0_empty,
        &mut ctx,
    )
    .unwrap()
    .expect("mode 0 empty stack dialog");
    assert!(
        msg_empty.icon.is_none(),
        "mode 0 with no token portrait and stack_object=0 must leave icon None"
    );
    assert!(!other_name.is_empty());
}
