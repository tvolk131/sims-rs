//! Unit and Complete Collection install tests for [`sims_ui::album`].

use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_ui::album::{
    AlbumCommand, AlbumEntry, AlbumKey, AlbumSnapshot, AlbumStrings, EditFocus, PhotoAlbumDialog,
    PhotoId, load_art,
};
use sims_ui::art::Art;
use sims_ui::dialog::CARET_BLINK_US;
use sims_ui::frame::{Frame, Rect};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    path.is_dir().then_some(path)
}

fn center(r: Rect) -> [i32; 2] {
    [r.x + r.width / 2, r.y + r.height / 2]
}

fn click(
    dialog: &mut PhotoAlbumDialog,
    rect: Rect,
    screen: [i32; 2],
    snapshot: &AlbumSnapshot,
) -> Vec<AlbumCommand> {
    let pos = center(rect);
    assert!(dialog.handle_pointer_down(pos, screen, snapshot));
    dialog.handle_pointer_up(pos, screen, snapshot)
}

fn solid_image(width: u32, height: u32, rgb: [u8; 3]) -> Image {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Image {
        width,
        height,
        rgba: pixels,
    }
}

fn sample_snapshot() -> AlbumSnapshot {
    AlbumSnapshot {
        family_name: "TestFamily".to_owned(),
        can_capture_snapshot: true,
        entries: vec![
            AlbumEntry {
                id: PhotoId(10),
                title: "First Day".to_owned(),
                caption: "Moved into the starter lot.".to_owned(),
                image: Some(solid_image(64, 48, [200, 80, 60])),
                thumbnail: None,
                sim_day: Some(1),
            },
            AlbumEntry {
                id: PhotoId(20),
                title: "Dinner Party".to_owned(),
                caption: "Neighbors visited for supper and music.".to_owned(),
                image: Some(solid_image(48, 64, [60, 180, 90])),
                thumbnail: Some(solid_image(16, 16, [80, 200, 110])),
                sim_day: Some(3),
            },
            AlbumEntry {
                id: PhotoId(30),
                title: "Garden".to_owned(),
                caption: "Planted hedges along the front walk.".to_owned(),
                image: None,
                thumbnail: None,
                sim_day: Some(5),
            },
        ],
    }
}

#[test]
fn browse_prev_next_first_last_and_thumbnail_strip() {
    let screen = [800, 600];
    let snapshot = sample_snapshot();
    let mut dialog = PhotoAlbumDialog::default();
    assert!(!dialog.is_open());

    dialog.open(&snapshot);
    assert!(dialog.is_open());
    assert_eq!(dialog.selected_id(), Some(PhotoId(10)));
    assert_eq!(dialog.draft_title(), "First Day");

    let layout = dialog.layout(screen[0], screen[1]);

    // Prev/First disabled on index 0
    assert!(click(&mut dialog, layout.btn_prev, screen, &snapshot).is_empty());
    assert!(click(&mut dialog, layout.btn_first, screen, &snapshot).is_empty());

    // Next -> index 1 (PhotoId(20))
    assert_eq!(
        click(&mut dialog, layout.btn_next, screen, &snapshot),
        vec![AlbumCommand::SelectEntry(PhotoId(20))]
    );
    assert_eq!(dialog.selected_id(), Some(PhotoId(20)));
    assert_eq!(dialog.draft_title(), "Dinner Party");

    // Last -> index 2 (PhotoId(30))
    assert_eq!(
        click(&mut dialog, layout.btn_last, screen, &snapshot),
        vec![AlbumCommand::SelectEntry(PhotoId(30))]
    );
    assert_eq!(dialog.selected_id(), Some(PhotoId(30)));
    assert!(click(&mut dialog, layout.btn_next, screen, &snapshot).is_empty());

    // Thumbnail slot 0 -> PhotoId(10)
    assert_eq!(
        click(&mut dialog, layout.thumb_slots[0], screen, &snapshot),
        vec![AlbumCommand::SelectEntry(PhotoId(10))]
    );
    assert_eq!(dialog.selected_id(), Some(PhotoId(10)));
}

#[test]
fn thumbnail_strip_scroll_persists_across_draw_without_snapping_back() {
    let screen = [800, 600];
    let art = Art::default();
    let mut frame = Frame::new(800, 600, [20, 30, 50]);
    let snapshot = AlbumSnapshot {
        family_name: "LargeAlbum".to_owned(),
        can_capture_snapshot: true,
        entries: (0..12)
            .map(|i| AlbumEntry {
                id: PhotoId(100 + i),
                title: format!("Photo {i}"),
                caption: String::new(),
                image: None,
                thumbnail: None,
                sim_day: Some(i as u32 + 1),
            })
            .collect(),
    };

    let mut dialog = PhotoAlbumDialog::default();
    dialog.open(&snapshot);
    let layout = dialog.layout(screen[0], screen[1]);
    let slots = layout.thumb_slots.len();
    assert!(slots < snapshot.entries.len());

    // Scroll strip right while entry 0 remains selected, then draw.
    assert!(click(&mut dialog, layout.strip_next, screen, &snapshot).is_empty());
    dialog.draw(&mut frame, &snapshot, &art);

    // Clicking the first visible slot must select entry 1 (PhotoId(101)), proving
    // draw did not snap strip_scroll back to entry 0.
    assert_eq!(
        click(&mut dialog, layout.thumb_slots[0], screen, &snapshot),
        vec![AlbumCommand::SelectEntry(PhotoId(101))]
    );
}

#[test]
fn edit_title_and_wrapped_caption_and_auto_commit_on_nav_or_done() {
    let screen = [800, 600];
    let snapshot = sample_snapshot();
    let art = Art::default();
    let mut dialog = PhotoAlbumDialog::default();
    dialog.open(&snapshot);
    let layout = dialog.layout(screen[0], screen[1]);

    // Focus title box and append text
    assert!(click(&mut dialog, layout.title_box, screen, &snapshot).is_empty());
    assert_eq!(dialog.focus(), Some(EditFocus::Title));
    for ch in "!".chars() {
        assert!(
            dialog
                .handle_key(AlbumKey::Char(ch), screen, &snapshot, &art)
                .is_empty()
        );
    }
    assert!(dialog.is_dirty());
    assert_eq!(dialog.draft_title(), "First Day!");

    // Tab moves focus to Caption; type newline + multi-word wrapped line
    assert!(
        dialog
            .handle_key(AlbumKey::Tab, screen, &snapshot, &art)
            .is_empty()
    );
    assert_eq!(dialog.focus(), Some(EditFocus::Caption));
    dialog.handle_key(AlbumKey::Enter, screen, &snapshot, &art);
    for ch in "Second line with wrapping words for testing.".chars() {
        dialog.handle_key(AlbumKey::Char(ch), screen, &snapshot, &art);
    }
    assert!(dialog.draft_caption().contains('\n'));

    // Switching to Next auto-commits UpdateEntryText before SelectEntry
    let cmds = click(&mut dialog, layout.btn_next, screen, &snapshot);
    assert_eq!(cmds.len(), 2);
    assert_eq!(
        cmds[0],
        AlbumCommand::UpdateEntryText {
            id: PhotoId(10),
            title: "First Day!".to_owned(),
            caption: "Moved into the starter lot.\nSecond line with wrapping words for testing."
                .to_owned(),
        }
    );
    assert_eq!(cmds[1], AlbumCommand::SelectEntry(PhotoId(20)));
    assert!(!dialog.is_dirty());

    // Edit entry 20 title and commit via Done -> emits [UpdateEntryText, Close]
    click(&mut dialog, layout.title_box, screen, &snapshot);
    dialog.handle_key(AlbumKey::Backspace, screen, &snapshot, &art);
    assert!(dialog.is_dirty());
    let done_cmds = click(&mut dialog, layout.btn_done, screen, &snapshot);
    assert_eq!(
        done_cmds,
        vec![
            AlbumCommand::UpdateEntryText {
                id: PhotoId(20),
                title: "Dinner Part".to_owned(),
                caption: "Neighbors visited for supper and music.".to_owned(),
            },
            AlbumCommand::Close,
        ]
    );
    assert!(!dialog.is_open());
}

#[test]
fn add_snapshot_reorder_and_two_step_delete_confirmation() {
    let screen = [800, 600];
    let snapshot = sample_snapshot();
    let mut dialog = PhotoAlbumDialog::default();
    dialog.open(&snapshot);
    let layout = dialog.layout(screen[0], screen[1]);

    // Add blank page and trigger camera snapshot
    assert_eq!(
        click(&mut dialog, layout.btn_add, screen, &snapshot),
        vec![AlbumCommand::AddBlankPage]
    );
    assert_eq!(
        click(&mut dialog, layout.btn_snapshot, screen, &snapshot),
        vec![AlbumCommand::CaptureSnapshot]
    );

    // Reorder: MoveEarlier disabled at index 0, MoveLater moves 10 -> index 1
    assert!(click(&mut dialog, layout.btn_move_earlier, screen, &snapshot).is_empty());
    assert_eq!(
        click(&mut dialog, layout.btn_move_later, screen, &snapshot),
        vec![AlbumCommand::MoveEntry {
            id: PhotoId(10),
            new_index: 1,
        }]
    );

    // Clicking Delete enters inline confirmation without emitting DeleteEntry yet
    assert!(click(&mut dialog, layout.btn_delete, screen, &snapshot).is_empty());
    assert_eq!(dialog.pending_delete(), Some(PhotoId(10)));

    // Clicking No cancels delete
    assert!(click(&mut dialog, layout.btn_confirm_no, screen, &snapshot).is_empty());
    assert_eq!(dialog.pending_delete(), None);

    // Clicking Delete then Yes emits DeleteEntry(PhotoId(10))
    assert!(click(&mut dialog, layout.btn_delete, screen, &snapshot).is_empty());
    assert_eq!(
        click(&mut dialog, layout.btn_confirm_yes, screen, &snapshot),
        vec![AlbumCommand::DeleteEntry(PhotoId(10))]
    );
    assert_eq!(dialog.pending_delete(), None);
}

#[test]
fn press_bound_pointer_guard_rejects_drag_off_and_disabled_snapshot() {
    let screen = [800, 600];
    let mut snapshot = sample_snapshot();
    snapshot.can_capture_snapshot = false;
    let mut dialog = PhotoAlbumDialog::default();
    dialog.open(&snapshot);
    let layout = dialog.layout(screen[0], screen[1]);

    // Snapshot button disabled when can_capture_snapshot == false
    assert!(click(&mut dialog, layout.btn_snapshot, screen, &snapshot).is_empty());

    // Press on Next, release on Done -> neither fires
    assert!(dialog.handle_pointer_down(center(layout.btn_next), screen, &snapshot));
    assert!(
        dialog
            .handle_pointer_up(center(layout.btn_done), screen, &snapshot)
            .is_empty()
    );
    assert!(dialog.is_open());
    assert_eq!(dialog.selected_id(), Some(PhotoId(10)));
}

#[test]
fn draws_empty_and_populated_album_without_panicking_and_blinks_caret() {
    let mut frame = Frame::new(800, 600, [40, 60, 80]);
    let art = Art::default();
    let mut dialog = PhotoAlbumDialog::default();

    // Empty album
    let empty = AlbumSnapshot::default();
    dialog.open(&empty);
    dialog.draw(&mut frame, &empty, &art);

    // Populated album with focused caption & blink cycle
    let snapshot = sample_snapshot();
    dialog.open(&snapshot);
    let layout = dialog.layout(800, 600);
    click(&mut dialog, layout.caption_box, [800, 600], &snapshot);
    dialog.draw(&mut frame, &snapshot, &art);
    dialog.advance_time(CARET_BLINK_US);
    dialog.draw(&mut frame, &snapshot, &art);
}

#[test]
fn loads_complete_collection_art_and_uitext_strings() {
    let Some(path) = install() else {
        return;
    };
    let install = Install::detect(&path).expect("open Complete Collection install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, 1).expect("load album UIGraphics art");
    let strings = AlbumStrings::load_from_install(&install, 1).expect("load UIText.iff strings");

    assert!(!strings.title.is_empty());
    assert!(!strings.done.is_empty());
    assert!(!strings.delete.is_empty());
    assert!(!strings.empty_caption.is_empty());
    assert!(!strings.done.starts_with('&'));
    assert!(!strings.delete.starts_with('*'));

    let mut dialog = PhotoAlbumDialog::new(strings);
    let snapshot = sample_snapshot();
    dialog.open(&snapshot);
    let mut frame = Frame::new(800, 600, [20, 40, 60]);
    dialog.draw(&mut frame, &snapshot, &art);
}
