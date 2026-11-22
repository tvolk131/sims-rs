use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::saves::{
    SaveHealth, SaveManagerCommand, SaveManagerDialog, SaveManagerSnapshot, SaveSlotKind,
    SaveSlotSummary, load_art,
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

fn sample_snapshot() -> SaveManagerSnapshot {
    SaveManagerSnapshot {
        neighborhood: 1,
        lot: 1,
        lot_label: "Neighborhood 1 - Lot 1".into(),
        slots: vec![
            SaveSlotSummary {
                slot_id: "n1-l1-auto-1".into(),
                neighborhood: 1,
                lot: 1,
                label: "Autosave #1".into(),
                kind: SaveSlotKind::Autosave { rotation_index: 1 },
                sim_day: 2,
                sim_hour: 9,
                sim_minute: 15,
                tick: 4_200,
                saved_at_epoch_secs: Some(1_700_000_100),
                household_name: Some("Goth".into()),
                health: SaveHealth::Intact,
            },
            SaveSlotSummary {
                slot_id: "n1-l1-auto-3".into(),
                neighborhood: 1,
                lot: 1,
                label: "Autosave #3".into(),
                kind: SaveSlotKind::Autosave { rotation_index: 3 },
                sim_day: 3,
                sim_hour: 18,
                sim_minute: 45,
                tick: 9_600,
                saved_at_epoch_secs: Some(1_700_000_300),
                household_name: Some("Goth".into()),
                health: SaveHealth::Corrupt {
                    reason: "BLAKE3 checksum mismatch".into(),
                },
            },
            SaveSlotSummary {
                slot_id: "n1-l1-manual".into(),
                neighborhood: 1,
                lot: 1,
                label: "Manual Save".into(),
                kind: SaveSlotKind::Manual,
                sim_day: 3,
                sim_hour: 12,
                sim_minute: 0,
                tick: 8_000,
                saved_at_epoch_secs: Some(1_700_000_200),
                household_name: Some("Goth".into()),
                health: SaveHealth::Intact,
            },
            SaveSlotSummary {
                slot_id: "n1-l1-auto-2".into(),
                neighborhood: 1,
                lot: 1,
                label: "Autosave #2".into(),
                kind: SaveSlotKind::Autosave { rotation_index: 2 },
                sim_day: 3,
                sim_hour: 15,
                sim_minute: 30,
                tick: 8_900,
                saved_at_epoch_secs: Some(1_700_000_250),
                household_name: Some("Goth".into()),
                health: SaveHealth::Intact,
            },
            SaveSlotSummary {
                slot_id: "n1-l1-auto-0".into(),
                neighborhood: 1,
                lot: 1,
                label: "Autosave #0".into(),
                kind: SaveSlotKind::Autosave { rotation_index: 0 },
                sim_day: 1,
                sim_hour: 8,
                sim_minute: 0,
                tick: 1_000,
                saved_at_epoch_secs: Some(1_700_000_050),
                household_name: Some("Goth".into()),
                health: SaveHealth::Unsupported {
                    schema_version: 99,
                    reason: "Schema v99 not supported".into(),
                },
            },
        ],
        active_error: None,
    }
}

#[test]
fn manual_first_then_newest_first_autosaves_and_newest_intact_resolution() {
    let snap = sample_snapshot();
    let ordered: Vec<&str> = snap
        .ordered_slots()
        .into_iter()
        .map(|s| s.slot_id.as_str())
        .collect();
    assert_eq!(
        ordered,
        vec![
            "n1-l1-manual",
            "n1-l1-auto-3",
            "n1-l1-auto-2",
            "n1-l1-auto-1",
            "n1-l1-auto-0",
        ]
    );
    assert_eq!(
        snap.newest_intact().map(|s| s.slot_id.as_str()),
        Some("n1-l1-auto-2")
    );
    let warning = snap.warning_banner_text().expect("warning banner");
    assert!(warning.contains("corrupt"));
    assert!(warning.contains("unsupported"));
    assert!(warning.contains("not modified or deleted"));
}

#[test]
fn load_fallback_delete_and_cancel_actions_respect_health_badges() {
    let snap = sample_snapshot();
    let mut dialog = SaveManagerDialog::new();
    dialog.open();
    assert!(dialog.is_open());

    let size = [800, 600];
    // Default selection is first ordered slot (Manual Save, Intact).
    assert_eq!(
        dialog.selected_slot(&snap).map(|s| s.slot_id.as_str()),
        Some("n1-l1-manual")
    );
    assert!(dialog.can_load(&snap));
    assert!(dialog.can_fallback(&snap));

    let layout = dialog.layout(size, &snap);
    let (lx, ly) = center(layout.load_btn);
    assert!(dialog.pointer_down(size, &snap, lx, ly));
    assert_eq!(
        dialog.pointer_up(size, &snap, lx, ly),
        Some(SaveManagerCommand::LoadSlot {
            slot_id: "n1-l1-manual".into()
        })
    );

    // Select the corrupt Autosave #3 (index 1 in ordered list).
    assert!(dialog.select_next(&snap));
    assert_eq!(
        dialog.selected_slot(&snap).map(|s| s.slot_id.as_str()),
        Some("n1-l1-auto-3")
    );
    assert!(!dialog.can_load(&snap));
    assert!(dialog.can_fallback(&snap));

    // Clicking Load on a corrupt slot emits nothing.
    dialog.pointer_down(size, &snap, lx, ly);
    assert_eq!(dialog.pointer_up(size, &snap, lx, ly), None);

    // Clicking Fallback (or pressing Enter) falls back to newest intact (Autosave #2).
    let (fx, fy) = center(layout.fallback_btn);
    dialog.pointer_down(size, &snap, fx, fy);
    assert_eq!(
        dialog.pointer_up(size, &snap, fx, fy),
        Some(SaveManagerCommand::FallbackToNewestIntact {
            slot_id: "n1-l1-auto-2".into()
        })
    );
    assert_eq!(
        dialog.submit(&snap),
        Some(SaveManagerCommand::FallbackToNewestIntact {
            slot_id: "n1-l1-auto-2".into()
        })
    );

    // Delete works on the selected corrupt slot.
    let (dx, dy) = center(layout.delete_btn);
    dialog.pointer_down(size, &snap, dx, dy);
    assert_eq!(
        dialog.pointer_up(size, &snap, dx, dy),
        Some(SaveManagerCommand::DeleteSlot {
            slot_id: "n1-l1-auto-3".into()
        })
    );

    // Cancel closes the dialog.
    let (cx, cy) = center(layout.cancel_btn);
    dialog.pointer_down(size, &snap, cx, cy);
    assert_eq!(
        dialog.pointer_up(size, &snap, cx, cy),
        Some(SaveManagerCommand::Cancel)
    );
    assert!(!dialog.is_open());
}

#[test]
fn scrolling_and_responsive_layout_never_overlap_across_viewports() {
    let mut snap = sample_snapshot();
    for i in 4..14u32 {
        snap.slots.push(SaveSlotSummary {
            slot_id: format!("n1-l1-auto-{i}"),
            neighborhood: 1,
            lot: 1,
            label: format!("Autosave #{i}"),
            kind: SaveSlotKind::Autosave { rotation_index: i },
            sim_day: i,
            sim_hour: 10,
            sim_minute: 0,
            tick: u64::from(i) * 1000,
            saved_at_epoch_secs: Some(1_700_000_000 + u64::from(i)),
            household_name: Some("Goth".into()),
            health: SaveHealth::Intact,
        });
    }

    let mut dialog = SaveManagerDialog::new();
    dialog.open();
    let art = Art::default();

    for size in [
        [320, 240],
        [640, 480],
        [800, 600],
        [1280, 720],
        [1920, 1080],
    ] {
        let layout = dialog.layout(size, &snap);
        assert!(layout.scroll_up.is_some());
        assert!(layout.scroll_down.is_some());
        if let Some(banner) = layout.warning_banner {
            assert!(!intersects(layout.title_bar, banner));
            assert!(!intersects(banner, layout.list_area));
        }
        assert!(!intersects(layout.list_area, layout.load_btn));
        assert!(!intersects(layout.load_btn, layout.fallback_btn));
        assert!(!intersects(layout.fallback_btn, layout.delete_btn));
        assert!(!intersects(layout.delete_btn, layout.cancel_btn));

        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        dialog.draw(&mut frame, &snap, &art);
        assert!(frame.rgb().iter().any(|&px| px != [0, 0, 0]));
    }

    assert!(dialog.scroll(2, [800, 600], &snap));
    assert_eq!(dialog.scroll_offset(), 2);
    assert!(dialog.scroll(-1, [800, 600], &snap));
    assert_eq!(dialog.scroll_offset(), 1);
}

#[test]
fn complete_collection_live_install_renders_save_manager_with_native_chrome() {
    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, 1).expect("load save manager art");
    assert!(
        art.missing.is_empty(),
        "missing native 1.91 save manager resources: {:?}",
        art.missing
    );

    let snap = sample_snapshot();
    let mut dialog = SaveManagerDialog::new();
    dialog.open();
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    dialog.draw(&mut frame, &snap, &art);

    let non_black = frame.rgb().iter().filter(|&&px| px != [0, 0, 0]).count();
    assert!(
        non_black > 25_000,
        "expected rich 1.91 9-slice save dialog rendering, got {non_black} non-black pixels"
    );
}
