use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_ui::art::Art;
use sims_ui::book::{
    BookBadge, BookCommand, BookEntry, BookPage, BookStrings, ReferenceBookDialog,
    ReferenceBookSnapshot, load_art, load_credits_pages, load_magic_book_entries,
};
use sims_ui::frame::{Frame, Rect};
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

fn sample_snapshot() -> ReferenceBookSnapshot {
    ReferenceBookSnapshot {
        title: "Reference Book".into(),
        subtitle: Some("Test Sim".into()),
        held_summary: vec![
            BookBadge {
                label: "Token A (x2)".into(),
                owned: true,
            },
            BookBadge {
                label: "Token B".into(),
                owned: false,
            },
        ],
        pages: vec![
            BookPage {
                heading: Some("Chapter 1".into()),
                entries: vec![BookEntry {
                    title: "Entry One".into(),
                    subtitle: Some("Adult".into()),
                    body: "First reference entry body text for pagination verification.".into(),
                    badges: vec![BookBadge {
                        label: "Token A".into(),
                        owned: true,
                    }],
                    highlight: true,
                }],
            },
            BookPage {
                heading: Some("Chapter 2".into()),
                entries: vec![BookEntry {
                    title: "Entry Two".into(),
                    subtitle: Some("Child".into()),
                    body: "Second reference entry body text for pagination verification.".into(),
                    badges: Vec::new(),
                    highlight: false,
                }],
            },
            BookPage {
                heading: Some("Chapter 3".into()),
                entries: vec![BookEntry {
                    title: "Entry Three".into(),
                    subtitle: None,
                    body: "Third reference entry body text.".into(),
                    badges: Vec::new(),
                    highlight: false,
                }],
            },
        ],
    }
}

#[test]
fn reference_book_pagination_bounds_and_buttons() {
    let snap = sample_snapshot();
    let mut dlg = ReferenceBookDialog::new();
    let size = [800, 600];
    dlg.open();
    assert!(dlg.is_open());
    assert_eq!(dlg.page(), 0);
    assert!(!dlg.can_prev(&snap));
    assert!(dlg.can_next(&snap));

    let layout = dlg.layout(size, &snap);
    assert!(!intersects(layout.prev_btn, layout.next_btn));
    assert!(!intersects(layout.next_btn, layout.close_btn));
    assert!(!intersects(layout.title_bar, layout.content_area));
    assert!(!intersects(layout.content_area, layout.prev_btn));

    // Clicking disabled Prev on page 0 is a no-op.
    let (px, py) = center(layout.prev_btn);
    assert!(dlg.pointer_down(size, &snap, px, py));
    assert_eq!(dlg.pointer_up(size, &snap, px, py), None);
    assert_eq!(dlg.page(), 0);

    // Clicking Next advances page 0 -> 1 -> 2, then stops.
    let (nx, ny) = center(layout.next_btn);
    assert!(dlg.pointer_down(size, &snap, nx, ny));
    assert_eq!(
        dlg.pointer_up(size, &snap, nx, ny),
        Some(BookCommand::PageChanged(1))
    );
    assert_eq!(dlg.page(), 1);
    assert!(dlg.can_prev(&snap));
    assert!(dlg.can_next(&snap));

    assert_eq!(dlg.scroll(1, &snap), Some(BookCommand::PageChanged(2)));
    assert_eq!(dlg.page(), 2);
    assert!(!dlg.can_next(&snap));
    assert_eq!(dlg.next_page(&snap), None);

    // Prev returns to page 1.
    assert!(dlg.pointer_down(size, &snap, px, py));
    assert_eq!(
        dlg.pointer_up(size, &snap, px, py),
        Some(BookCommand::PageChanged(1))
    );

    // Close button emits Close request without prematurely closing the dialog
    // so hosts can answer backing VM panels before closing.
    let (cx, cy) = center(layout.close_btn);
    assert!(dlg.pointer_down(size, &snap, cx, cy));
    assert_eq!(
        dlg.pointer_up(size, &snap, cx, cy),
        Some(BookCommand::Close)
    );
    assert!(dlg.is_open());
    dlg.close();
    assert!(!dlg.is_open());
}

#[test]
fn reference_book_scrolls_long_wrapped_text_at_short_viewport() {
    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let live_art = load_art(&ui, 1).expect("load book art");
    let live_strings = BookStrings::load(&install, 1).expect("load strings");
    let long_body = (1..=30)
        .map(|i| format!("Line {i}: long reference entry line for short viewport wrap check."))
        .collect::<Vec<_>>()
        .join("\n");
    let snap = ReferenceBookSnapshot {
        title: "Long Reference".into(),
        subtitle: None,
        held_summary: Vec::new(),
        pages: vec![BookPage {
            heading: Some("Tall Page".into()),
            entries: vec![BookEntry {
                title: "Tall Entry".into(),
                subtitle: None,
                body: long_body,
                badges: Vec::new(),
                highlight: false,
            }],
        }],
    };
    let mut dlg = ReferenceBookDialog::new();
    dlg.open();
    let short_size = [320, 240];
    let max_scroll = dlg.max_scroll_lines(short_size, &snap, &live_art);
    assert!(
        max_scroll > 5,
        "tall wrapped entry at 320x240 must expose scrollable overflow, got {max_scroll}"
    );
    let layout = dlg.layout(short_size, &snap);
    assert!(!intersects(layout.content_area, layout.scroll_up_btn));
    assert!(!intersects(layout.scroll_up_btn, layout.scroll_down_btn));

    let (dx, dy) = center(layout.scroll_down_btn);
    assert!(dlg.pointer_down(short_size, &snap, dx, dy));
    assert_eq!(
        dlg.pointer_up(short_size, &snap, dx, dy),
        Some(BookCommand::ScrollLines(3))
    );
    let mut frame_top = Frame::new(short_size[0], short_size[1], [0, 0, 0]);
    dlg.draw(&mut frame_top, &snap, &live_strings, &live_art);
    assert!(dlg.scroll_vertical(3, short_size, &snap, &live_art));
    assert_eq!(dlg.scroll_lines(), 3);
    let mut frame_scrolled = Frame::new(short_size[0], short_size[1], [0, 0, 0]);
    dlg.draw(&mut frame_scrolled, &snap, &live_strings, &live_art);
    assert_ne!(
        frame_top.rgb(),
        frame_scrolled.rgb(),
        "scrolling down within a tall reference page must advance visible lines"
    );
    assert!(dlg.scroll_vertical(max_scroll as i32, short_size, &snap, &live_art));
    assert_eq!(dlg.scroll_lines(), max_scroll);
}

#[test]
fn reference_book_renders_across_viewports_and_live_install() {
    let snap = sample_snapshot();
    let mut dlg = ReferenceBookDialog::new();
    dlg.open();
    let strings = BookStrings::default();
    let art = Art::default();
    for size in [[320, 240], [800, 600], [1280, 720]] {
        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        dlg.draw(&mut frame, &snap, &strings, &art);
        assert!(frame.rgb().iter().any(|&px| px != [0, 0, 0]));
    }

    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let live_art = load_art(&ui, 1).expect("load book art");
    assert!(live_art.missing.is_empty());

    let live_strings = BookStrings::load(&install, 1).expect("load magic.iff strings");
    assert!(!live_strings.back.is_empty());
    assert!(!live_strings.next.is_empty());
    assert!(!live_strings.close.is_empty());

    let spells = load_magic_book_entries(&install, false, 1).expect("load spellbook catalog");
    let recipes = load_magic_book_entries(&install, true, 1).expect("load cookbook catalog");
    assert!(
        spells.len() >= 15,
        "expected real spellbook catalog entries"
    );
    assert!(recipes.len() >= 6, "expected real cookbook catalog entries");

    let credits = load_credits_pages(&install, 1).expect("load credits pages");
    assert!(credits.len() >= 5, "expected multi-page credits catalog");
}
