use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::family_bin::{
    ColumnHeader, FamilyBin, FamilyBinButton, FamilyBinCommand, FamilyBinLayout, FamilyBinMode,
    FamilyBinStrings, FamilyBinView, FamilyEntry, FamilyMemberPortrait, ROW_HEIGHT, ROW_WIDTH,
    VISIBLE_ROWS, art_id, load_art,
};
use sims_ui::frame::{Frame, Rect};
use std::path::PathBuf;

fn install() -> Option<Install> {
    let path = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Install::detect(&path).ok()
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.width > 0
        && a.height > 0
        && b.width > 0
        && b.height > 0
        && a.x < b.x + b.width
        && a.x + a.width > b.x
        && a.y < b.y + b.height
        && a.y + a.height > b.y
}

fn sample_families(count: usize) -> Vec<FamilyEntry> {
    (0..count)
        .map(|i| FamilyEntry {
            id: (i as u32) + 100,
            name: format!("Family{i}"),
            funds: 20_000 + (i as i32) * 1_000,
            friends: i as i32,
            magic_coins: (i as i32) * 50,
            members: vec![FamilyMemberPortrait {
                name: format!("Sim{i}"),
                portrait: Some(Image {
                    width: 16,
                    height: 16,
                    rgba: [180, 140, 110, 255].repeat(16 * 16),
                }),
            }],
        })
        .collect()
}

fn synthetic_strings() -> FamilyBinStrings {
    FamilyBinStrings {
        entries: (0..20).map(|i| format!("fb_str_{i}")).collect(),
    }
}

#[test]
fn native_and_compact_layouts_have_zero_overlaps() {
    for (w, h) in [
        (800u32, 600u32),
        (1024, 768),
        (640, 480),
        (600, 480),
        (500, 480),
    ] {
        for mode in [FamilyBinMode::Standard, FamilyBinMode::MagicTown] {
            let layout = FamilyBinLayout::compute(w, h, 10, 2, mode);
            assert!(layout.visible_rows >= 1 && layout.visible_rows <= VISIBLE_ROWS);
            assert!(
                layout.scrollbar.is_some(),
                "10 families need a scrollbar at {w}x{h}"
            );

            // Collect all interactive rects and verify strict pairwise disjointness
            // and containment within viewport.
            let mut all_rects: Vec<Rect> = Vec::new();
            all_rects.extend(layout.row_rects.iter().copied());
            all_rects.extend(layout.buttons.iter().map(|&(_, r)| r));
            if let Some(sb) = layout.scrollbar {
                all_rects.push(sb.up_arrow);
                all_rects.push(sb.down_arrow);
                all_rects.push(sb.track);
            }
            for (i, &r) in all_rects.iter().enumerate() {
                assert!(r.width > 0 && r.height > 0, "empty rect {i} at {w}x{h}");
                assert!(
                    r.x >= 0 && r.y >= 0 && r.x + r.width <= w as i32 && r.y + r.height <= h as i32,
                    "rect {i} ({r:?}) outside {w}x{h}"
                );
                for (j, &other) in all_rects.iter().enumerate().skip(i + 1) {
                    assert!(
                        !rects_overlap(r, other),
                        "rects {i} ({r:?}) and {j} ({other:?}) overlap at {w}x{h}"
                    );
                }
            }
        }
    }

    // Native 800x600 exact coordinates check (`0x563284..0x5632f9`).
    let native = FamilyBinLayout::compute(800, 600, 3, 0, FamilyBinMode::Standard);
    assert!(
        native.scrollbar.is_none(),
        "3 families do not show scrollbar"
    );
    assert_eq!(
        native.row_rects[0],
        Rect::new(41, 76, ROW_WIDTH, ROW_HEIGHT)
    );
    assert_eq!(
        native.row_rects[5],
        Rect::new(41, 76 + 5 * ROW_HEIGHT, ROW_WIDTH, ROW_HEIGHT)
    );
    assert_eq!(
        native.button_rect(FamilyBinButton::CreateNewFamily),
        Some(Rect::new(322, 530, 92, 62))
    );
    assert_eq!(
        native.button_rect(FamilyBinButton::DeleteFamily),
        Some(Rect::new(414, 530, 92, 62))
    );
    assert_eq!(
        native.button_rect(FamilyBinButton::MoveInFamily),
        Some(Rect::new(506, 530, 164, 62))
    );
}

#[test]
fn selection_buttons_double_click_and_empty_slots() {
    let families = sample_families(3);
    let view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &families,
    };
    let size = [800, 600];
    let layout = FamilyBinLayout::compute(800, 600, families.len(), 0, view.mode);
    let mut bin = FamilyBin::new();

    let click_btn = |bin: &mut FamilyBin, btn: FamilyBinButton| {
        let r = layout.button_rect(btn).unwrap();
        let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
        assert_eq!(bin.press(cx, cy, size, &view), None);
        bin.release(cx, cy, size, &view)
    };

    // Without selection, Delete and MoveIn are disabled; Create & Cancel work.
    assert_eq!(click_btn(&mut bin, FamilyBinButton::DeleteFamily), None);
    assert_eq!(click_btn(&mut bin, FamilyBinButton::MoveInFamily), None);
    assert_eq!(
        click_btn(&mut bin, FamilyBinButton::CreateNewFamily),
        Some(FamilyBinCommand::CreateNewFamily)
    );
    assert_eq!(
        click_btn(&mut bin, FamilyBinButton::Cancel),
        Some(FamilyBinCommand::Cancel)
    );

    // Clicking empty slot #4 (index 4 >= len 3) selects nothing.
    let empty_row = layout.row_rects[4];
    assert_eq!(
        bin.press(empty_row.x + 20, empty_row.y + 20, size, &view),
        None
    );
    assert_eq!(bin.selected_index(&families), None);

    // Clicking populated row #2 selects Family2 (id 102).
    let row2 = layout.row_rects[2];
    assert_eq!(
        bin.press(row2.x + 20, row2.y + 20, size, &view),
        Some(FamilyBinCommand::SelectFamily(102))
    );
    assert_eq!(bin.selected_index(&families), Some(2));
    assert_eq!(bin.selected_id(), Some(102));
    assert_eq!(
        click_btn(&mut bin, FamilyBinButton::MoveInFamily),
        Some(FamilyBinCommand::MoveInFamily(102))
    );
    assert_eq!(
        click_btn(&mut bin, FamilyBinButton::DeleteFamily),
        Some(FamilyBinCommand::DeleteFamily(102))
    );

    // Removing a preceding row keeps selection pinned to stable ID 102 (never retargets!).
    let shifted: Vec<FamilyEntry> = families[1..].to_vec(); // ids 101, 102
    let shifted_view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &shifted,
    };
    assert_eq!(bin.selected_index(&shifted), Some(1));
    let del_r = layout.button_rect(FamilyBinButton::DeleteFamily).unwrap();
    let (dx, dy) = (del_r.x + del_r.width / 2, del_r.y + del_r.height / 2);
    assert_eq!(bin.press(dx, dy, size, &shifted_view), None);
    assert_eq!(
        bin.release(dx, dy, size, &shifted_view),
        Some(FamilyBinCommand::DeleteFamily(102))
    );

    // Removing the selected household itself clears selection so Delete does nothing.
    let without_102: Vec<FamilyEntry> = vec![families[0].clone(), families[1].clone()];
    let without_view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &without_102,
    };
    assert_eq!(bin.press(dx, dy, size, &without_view), None);
    assert_eq!(bin.release(dx, dy, size, &without_view), None);
    assert_eq!(bin.selected_id(), None);

    // Double-clicking row #1 immediately emits MoveInFamily(101).
    let row1 = layout.row_rects[1];
    assert_eq!(
        bin.double_click(row1.x + 20, row1.y + 20, size, &view),
        Some(FamilyBinCommand::MoveInFamily(101))
    );
    assert_eq!(bin.selected_index(&families), Some(1));

    // Escape cancels cleanly.
    assert_eq!(bin.cancel(), FamilyBinCommand::Cancel);
}

#[test]
fn scrolling_arrows_track_drag_and_list_shrink() {
    let families = sample_families(10);
    let view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &families,
    };
    let size = [800, 600];
    let mut bin = FamilyBin::new();

    let layout = FamilyBinLayout::compute(800, 600, families.len(), 0, view.mode);
    let sb = layout.scrollbar.unwrap();

    // Down arrow increments scroll_offset.
    bin.press(
        sb.down_arrow.x + sb.down_arrow.width / 2,
        sb.down_arrow.y + sb.down_arrow.height / 2,
        size,
        &view,
    );
    assert_eq!(bin.scroll_offset(), 1);

    // Clicking slot 0 now selects family index 1 (id 101).
    let row0 = layout.row_rects[0];
    assert_eq!(
        bin.press(row0.x + 30, row0.y + 30, size, &view),
        Some(FamilyBinCommand::SelectFamily(101))
    );

    // Up arrow decrements scroll_offset back to 0.
    bin.press(
        sb.up_arrow.x + sb.up_arrow.width / 2,
        sb.up_arrow.y + sb.up_arrow.height / 2,
        size,
        &view,
    );
    assert_eq!(bin.scroll_offset(), 0);

    // Track click below thumb pages down and clamps at max (10 - 6 = 4).
    bin.press(
        sb.track.x + sb.track.width / 2,
        sb.track.y + sb.track.height - 4,
        size,
        &view,
    );
    assert_eq!(bin.scroll_offset(), 4);

    // Drag thumb back to top.
    let layout_bottom = FamilyBinLayout::compute(800, 600, families.len(), 4, view.mode);
    let sb_b = layout_bottom.scrollbar.unwrap();
    bin.press(sb_b.thumb.x + 4, sb_b.thumb.y + 4, size, &view);
    bin.hover(sb_b.thumb.x + 4, sb_b.track.y, size, &view);
    bin.release(sb_b.thumb.x + 4, sb_b.track.y, size, &view);
    assert_eq!(bin.scroll_offset(), 0);

    // Wheel scroll and sync after shrinking list.
    bin.scroll(100, families.len(), VISIBLE_ROWS);
    assert_eq!(bin.scroll_offset(), 4);
    bin.select(Some(9), &families);
    bin.sync(&families[..3], VISIBLE_ROWS);
    assert_eq!(bin.selected_id(), None);
    assert_eq!(bin.scroll_offset(), 0);
}

#[test]
fn tiny_and_extreme_viewports_do_not_overlap_or_overflow() {
    // 190x240 Magic Town: column headers must not overlap.
    let tiny = FamilyBinLayout::compute(190, 240, 10, 0, FamilyBinMode::MagicTown);
    for i in 0..tiny.headers.len() {
        for j in (i + 1)..tiny.headers.len() {
            assert!(
                !rects_overlap(tiny.headers[i].1, tiny.headers[j].1),
                "headers overlap at 190x240: {:?} vs {:?}",
                tiny.headers[i],
                tiny.headers[j]
            );
        }
    }
    // 190x110: scrollbar either stays absent or leaves draggable travel.
    let short = FamilyBinLayout::compute(190, 110, 10, 0, FamilyBinMode::Standard);
    if let Some(sb) = short.scrollbar {
        assert!(sb.thumb.height < sb.track.height);
        assert!(!rects_overlap(sb.up_arrow, sb.track));
        assert!(!rects_overlap(sb.down_arrow, sb.track));
    }
    // Extreme width u32::MAX must not overflow in debug builds.
    let _ = FamilyBinLayout::compute(u32::MAX, 599, 10, 0, FamilyBinMode::MagicTown);
}

#[test]
fn magic_town_mode_and_zero_size_frame_safety() {
    let layout = FamilyBinLayout::compute(800, 600, 2, 0, FamilyBinMode::MagicTown);
    assert!(
        layout
            .headers
            .iter()
            .any(|&(h, _)| h == ColumnHeader::MagicCoins)
    );

    let families = sample_families(2);
    let view = FamilyBinView {
        mode: FamilyBinMode::MagicTown,
        families: &families,
    };
    let bin = FamilyBin::new();
    let art = Art::default();
    let strings = synthetic_strings();
    let mut zero = Frame::new(0, 0, [0, 0, 0]);
    let zero_layout = bin.draw(&mut zero, &view, &art, &strings);
    assert_eq!(zero_layout.visible_rows, 0);
}

#[test]
fn install_family_bin_art_strings_and_rendering() {
    let Some(install) = install() else { return };
    let ui = UiGraphics::open(&install).unwrap();
    let art = load_art(&ui, 1).unwrap();
    assert!(
        art.missing.is_empty(),
        "missing family bin resources: {:?}",
        art.missing
    );
    assert!(art.font(16).is_some(), "variablesans_16 should load");
    assert_eq!(art.image(art_id::PICK_BKG).unwrap().width, 800);
    assert_eq!(art.image(art_id::PICK_BKG).unwrap().height, 600);
    assert_eq!(art.image(art_id::ITEM_BKG).unwrap().width, 690);
    assert_eq!(art.image(art_id::ITEM_BKG).unwrap().height, 204);

    let strings = FamilyBinStrings::load(&install, 1).unwrap();
    assert!(strings.entries.len() >= 18);
    assert!(!strings.title().is_empty());
    assert!(!strings.cancel_label().is_empty());

    let families = sample_families(9);
    let view = FamilyBinView {
        mode: FamilyBinMode::Standard,
        families: &families,
    };
    let mut bin = FamilyBin::new();
    bin.select(Some(2), &families);

    // Hover Create button to verify tooltip rendering too.
    let layout_preview = FamilyBinLayout::compute(800, 600, families.len(), 0, view.mode);
    let create_r = layout_preview
        .button_rect(FamilyBinButton::CreateNewFamily)
        .unwrap();
    bin.hover(
        create_r.x + create_r.width / 2,
        create_r.y + create_r.height / 2,
        [800, 600],
        &view,
    );

    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let layout = bin.draw(&mut frame, &view, &art, &strings);
    assert!(layout.scrollbar.is_some());

    // Selected row 2 must render bright cyan pixels (`[0, 255, 255]`) matching 10-family-selected.png.
    let f = &frame;
    let row2 = layout.row_rects[2];
    let cyan_count = (row2.y..(row2.y + row2.height))
        .flat_map(|y| (row2.x..(row2.x + row2.width)).map(move |x| f.pixel(x, y).unwrap()))
        .filter(|&[r, g, b]| r < 32 && g > 200 && b > 200)
        .count();
    assert!(
        cyan_count > 100,
        "selected family row should have cyan highlight & ink, got {cyan_count}"
    );

    // Cream tooltip must render above the hovered Create button.
    let tooltip_cream = (480..540)
        .flat_map(|y| (220..520).map(move |x| f.pixel(x, y).unwrap()))
        .filter(|&px| px == [255, 255, 230])
        .count();
    assert!(
        tooltip_cream > 100,
        "hovering Create button should render cream tooltip, got {tooltip_cream}"
    );

    // Magic Town variant renders cleanly with `PickBkgMagic.bmp`.
    let magic_view = FamilyBinView {
        mode: FamilyBinMode::MagicTown,
        families: &families[..3],
    };
    let mut magic_frame = Frame::new(800, 600, [0, 0, 0]);
    bin.draw(&mut magic_frame, &magic_view, &art, &strings);
    assert_ne!(frame.rgb(), magic_frame.rgb());
}
