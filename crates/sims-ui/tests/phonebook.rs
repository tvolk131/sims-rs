use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::phonebook::{
    NATIVE_HEIGHT, NATIVE_WIDTH, OPTIONAL_RESOURCES, PHONE_BOOK_BKG, PHONE_BOOK_ICON,
    PhoneBookCommand, PhoneBookDialog, PhoneBookEntry, PhoneBookSnapshot, load_art,
};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    path.is_dir().then_some(path)
}

fn center(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn intersects(a: Rect, b: Rect) -> bool {
    let ax1 = i64::from(a.x) + i64::from(a.width);
    let ay1 = i64::from(a.y) + i64::from(a.height);
    let bx1 = i64::from(b.x) + i64::from(b.width);
    let by1 = i64::from(b.y) + i64::from(b.height);
    i64::from(a.x) < bx1 && ax1 > i64::from(b.x) && i64::from(a.y) < by1 && ay1 > i64::from(b.y)
}

fn solid_portrait(rgb: [u8; 3]) -> Image {
    let mut rgba = Vec::with_capacity(45 * 45 * 4);
    for _ in 0..(45 * 45) {
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Image {
        width: 45,
        height: 45,
        rgba,
    }
}

fn sample_snapshot(count: usize) -> PhoneBookSnapshot {
    let entries = (0..count)
        .map(|i| PhoneBookEntry {
            neighbor_id: (i as i16) + 10,
            name: format!("Neighbor {}", i + 1),
            daily: ((i as i16) * 15 - 30).clamp(-100, 100),
            lifetime: ((i as i16) * 12 - 10).clamp(-100, 100),
            portrait: Some(solid_portrait([
                (40 + i * 17) as u8,
                (90 + i * 11) as u8,
                (140 + i * 9) as u8,
            ])),
            is_friend: i >= 5,
        })
        .collect();
    PhoneBookSnapshot {
        title: "Call Neighbor".into(),
        caller_name: Some("Mortimer".into()),
        entries,
        cancel_label: "Cancel".into(),
        prev_label: "Prev".into(),
        next_label: "Next".into(),
    }
}

#[test]
fn phonebook_paging_disjoint_layout_and_call_cancel_commands() {
    let snap = sample_snapshot(12);
    let mut dlg = PhoneBookDialog::new();
    dlg.open();
    assert!(dlg.is_open());

    let size = [800, 600];
    let layout = dlg.layout(size, &snap);
    assert!(layout.native_art);
    assert_eq!(layout.dialog.width, NATIVE_WIDTH);
    assert_eq!(layout.dialog.height, NATIVE_HEIGHT);
    assert_eq!(layout.per_page, 8);
    assert_eq!(layout.entry_slots.len(), 8);
    assert_eq!(dlg.page_count(size, &snap), 2);
    assert!(!dlg.can_prev(size, &snap));
    assert!(dlg.can_next(size, &snap));

    // All slots and footer controls must be pairwise disjoint.
    for (i, &(_, a)) in layout.entry_slots.iter().enumerate() {
        assert!(!intersects(a, layout.prev_btn));
        assert!(!intersects(a, layout.next_btn));
        assert!(!intersects(a, layout.cancel_btn));
        for &(_, b) in &layout.entry_slots[i + 1..] {
            assert!(!intersects(a, b), "slots overlapped: {a:?} vs {b:?}");
        }
    }
    assert!(!intersects(layout.prev_btn, layout.next_btn));
    assert!(!intersects(layout.next_btn, layout.cancel_btn));

    // Next page advances to page 1 showing the remaining 4 neighbors (indices 8..12).
    let (nx, ny) = center(layout.next_btn);
    assert!(dlg.pointer_down(size, &snap, nx, ny));
    assert_eq!(dlg.pointer_up(size, &snap, nx, ny), None);
    assert_eq!(dlg.page(), 1);
    let p1 = dlg.layout(size, &snap);
    assert_eq!(p1.entry_slots.len(), 4);
    assert_eq!(p1.entry_slots[0].0, 8);

    // Clicking first slot on page 1 emits Call(neighbor_id = 18).
    let (ex, ey) = center(p1.entry_slots[0].1);
    assert!(dlg.pointer_down(size, &snap, ex, ey));
    assert_eq!(
        dlg.pointer_up(size, &snap, ex, ey),
        Some(PhoneBookCommand::Call(18))
    );

    // Compact viewport (< 620x356) falls back cleanly to non-native responsive layout.
    let compact = dlg.layout([480, 320], &snap);
    assert!(!compact.native_art);
    assert!(compact.dialog.width <= 480 && compact.dialog.height <= 320);
    assert!(!compact.entry_slots.is_empty());

    // Cancel closes the modal and emits Cancel.
    assert_eq!(dlg.escape(), Some(PhoneBookCommand::Cancel));
    assert!(!dlg.is_open());
}

#[test]
fn complete_collection_loads_native_phonebook_bitmaps_and_renders() {
    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, US_ENGLISH).expect("load phonebook art");
    assert!(art.missing.is_empty());
    assert!(art.image(PHONE_BOOK_BKG).is_some());
    assert!(art.image(PHONE_BOOK_ICON).is_some());
    for &id in &OPTIONAL_RESOURCES {
        assert!(art.image(id).is_some(), "missing phonebook resource {id}");
    }

    let snap = sample_snapshot(10);
    let mut dlg = PhoneBookDialog::new();
    dlg.open();
    for size in [[800u32, 600u32], [480, 320]] {
        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        dlg.draw(&mut frame, &snap, &art);
        let non_black = frame.rgb().iter().filter(|&&px| px != [0, 0, 0]).count();
        assert!(
            non_black > 40_000,
            "expected rich phonebook render at {size:?}, got {non_black}"
        );
    }
    // Synthetic Art::default() fallback renders without panicking too.
    let mut fallback = Frame::new(800, 600, [0, 0, 0]);
    dlg.draw(&mut fallback, &snap, &Art::default());
    assert!(fallback.rgb().iter().any(|&px| px != [0, 0, 0]));
}
