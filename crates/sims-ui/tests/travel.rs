use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::neighborhood::ExpansionFlags;
use sims_ui::travel::{
    Destination, LotFilter, TravelBarMode, TravelChrome, TravelCommand, TravelControl,
    TravelDialog, TravelDialogCommand, TravelDialogLayout, TravelDialogState, TravelLayout,
    TravelState, TravelStrings, TravelTransport, load_art,
};
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.width > 0
        && a.height > 0
        && b.width > 0
        && b.height > 0
        && a.x < b.x + b.width
        && b.x < a.x + a.width
        && a.y < b.y + b.height
        && b.y < a.y + a.height
}

#[test]
fn all_four_destinations_have_disjoint_controls_across_resolutions() {
    for dest in Destination::ALL {
        for mode in [
            TravelBarMode::Browse,
            TravelBarMode::Visit,
            TravelBarMode::Bulldoze,
        ] {
            let state = TravelState {
                destination: dest,
                mode,
                has_importable_lot: true,
                ..TravelState::default()
            };
            for (w, h) in [(1024u32, 768u32), (800, 600), (640, 480), (320, 240)] {
                let layout = TravelLayout::compute(w, h, &state);
                assert!(
                    !layout.controls.is_empty(),
                    "expected controls for {dest:?} {mode:?} at {w}x{h}"
                );
                for (i, a) in layout.controls.iter().enumerate() {
                    assert!(a.rect.x >= 0 && a.rect.y >= 0);
                    assert!(a.rect.x + a.rect.width <= w as i32);
                    assert!(a.rect.y + a.rect.height <= h as i32);
                    for b in &layout.controls[i + 1..] {
                        assert!(
                            !rects_overlap(a.rect, b.rect),
                            "controls {:?} and {:?} overlapped at {w}x{h}",
                            a.control,
                            b.control
                        );
                    }
                }
            }
        }
    }
}

/// The 1.91 destination screens (`cWinDowntown`, `cWinVacation`,
/// `cWinStudiotown`, `cWinMagicland`) build their bar from one button table
/// each and a shared position table: Sims logo `(5, 0)`, Return `(323, 0)`,
/// Bulldoze `(373, 0)`, Import `(422, 0)`, Sims.com `(762, 0)`, and Magic
/// Town's Move In `(200, 0)`. Nothing else is on the bar: no destination
/// switcher, filter strip, lodging tabs, or status badge.
#[test]
fn destination_bars_hold_exactly_the_native_button_table() {
    use TravelControl::*;
    for dest in Destination::ALL {
        let mut expected = vec![
            (Credits, 5),
            (ReturnHome, 323),
            (Bulldoze, 373),
            (ImportLot, 422),
            (Web, 762),
        ];
        if dest == Destination::MagicTown {
            expected.insert(1, (MoveIn, 200));
        }
        for (w, h, origin) in [(800u32, 600u32, [0, 0]), (1024, 768, [112, 84])] {
            let state = TravelState {
                destination: dest,
                ..TravelState::default()
            };
            let layout = TravelLayout::compute(w, h, &state);
            assert_eq!(layout.origin, origin);
            assert_eq!(
                layout.bar_rect,
                Rect::new(origin[0], origin[1], 800, 52),
                "{dest:?} bar"
            );
            let mut got: Vec<_> = layout
                .controls
                .iter()
                .map(|c| {
                    assert_eq!(c.origin[1], origin[1], "{:?} must sit on row 0", c.control);
                    (c.control, c.origin[0] - origin[0])
                })
                .collect();
            got.sort_by_key(|&(_, x)| x);
            assert_eq!(got, expected, "{dest:?} at {w}x{h}");
            assert!(layout.dialog_rect.is_none() && layout.title_rect.is_none());
        }
    }
}

/// Enable rules of `cWinVacation`'s mode switch (`0x56b19e`) and Import's
/// file check (`0x56b870`).
#[test]
fn destination_bar_enable_rules_follow_the_native_mode_switch() {
    use TravelControl::*;
    let enabled = |dest, mode, importable| {
        let state = TravelState {
            destination: dest,
            mode,
            has_importable_lot: importable,
            ..TravelState::default()
        };
        let layout = TravelLayout::compute(800, 600, &state);
        let mut v: Vec<_> = layout
            .controls
            .iter()
            .map(|c| (c.control, c.enabled, c.active))
            .collect();
        v.sort_by_key(|&(c, ..)| format!("{c:?}"));
        v
    };
    let dest = Destination::MagicTown;
    let browse = enabled(dest, TravelBarMode::Browse, false);
    assert!(browse.contains(&(ImportLot, false, false)));
    assert!(browse.contains(&(Bulldoze, true, false)));
    assert!(browse.contains(&(ReturnHome, true, false)));
    assert!(browse.contains(&(Web, true, false)));
    assert!(browse.contains(&(Credits, true, false)));
    assert!(browse.contains(&(MoveIn, true, false)));
    assert!(enabled(dest, TravelBarMode::Browse, true).contains(&(ImportLot, true, false)));

    let doze = enabled(Destination::VacationIsland, TravelBarMode::Bulldoze, true);
    assert_eq!(
        doze,
        vec![
            (Bulldoze, true, true),
            (Credits, false, false),
            (ImportLot, false, false),
            (ReturnHome, false, false),
            (Web, false, false),
        ]
    );
}

#[test]
fn clicking_controls_emits_expected_commands() {
    use TravelControl::*;
    let mut state = TravelState {
        destination: Destination::MagicTown,
        mode: TravelBarMode::Browse,
        expansions: ExpansionFlags(ExpansionFlags::HOT_DATE | ExpansionFlags::MAKIN_MAGIC),
        has_importable_lot: true,
        ..TravelState::default()
    };
    let layout = TravelLayout::compute(800, 600, &state);
    let mut chrome = TravelChrome::new();
    for (control, command) in [
        (ReturnHome, TravelCommand::ReturnToNeighborhood),
        (Bulldoze, TravelCommand::ToggleBulldoze),
        (ImportLot, TravelCommand::ImportLot),
        (Web, TravelCommand::OpenWeb),
        (Credits, TravelCommand::OpenCredits),
        (MoveIn, TravelCommand::OpenFamilyBin),
    ] {
        let c = layout
            .controls
            .iter()
            .find(|c| c.control == control)
            .unwrap_or_else(|| panic!("{control:?} control"));
        let (x, y) = (c.rect.x + c.rect.width / 2, c.rect.y + c.rect.height / 2);
        assert!(chrome.pointer_down(x, y, &layout), "{control:?} press");
        assert_eq!(chrome.pointer_up(x, y, &layout, &state), Some(command));
    }

    // Visit mode (lot picking from a trip) keeps only Return live.
    state.mode = TravelBarMode::Visit;
    state.destination = Destination::Downtown;
    let visit_layout = TravelLayout::compute(800, 600, &state);
    for c in &visit_layout.controls {
        assert_eq!(
            c.enabled,
            c.control == ReturnHome,
            "{:?} in Visit",
            c.control
        );
    }
    let doze = visit_layout
        .controls
        .iter()
        .find(|c| c.control == Bulldoze)
        .expect("Bulldoze control");
    assert!(!chrome.pointer_down(doze.rect.x + 2, doze.rect.y + 2, &visit_layout));
}

/// The Magic Town lot picker keeps the `dlgframe2.bmp` dialog with its
/// 7-filter row at `x = 225 + 50·i, y = 20` (`0x6f54d0`).
#[test]
fn magic_town_picker_dialog_filter_row_matches_native_positions() {
    let state = TravelState {
        destination: Destination::MagicTown,
        mode: TravelBarMode::Visit,
        ..TravelState::default()
    };
    let layout = TravelLayout::compute(800, 600, &state);
    assert!(layout.dialog_rect.is_some());
    let xs: Vec<_> = layout
        .controls
        .iter()
        .map(|c| {
            assert!(matches!(c.control, TravelControl::ToggleFilter(_)));
            assert_eq!(c.rect.y, 20);
            c.rect.x
        })
        .collect();
    assert_eq!(xs, vec![225, 275, 325, 375, 425, 475, 525]);
}

#[test]
fn travel_confirm_dialog_enforces_fares_and_handles_taxi_shuttle_magic_hole() {
    let mut dialog = TravelDialog::new();

    // Taxi to Downtown: $50 fare, insufficient funds blocks confirm.
    let mut taxi = TravelDialogState::for_destination(Destination::Downtown, vec!["Sim A".into()]);
    assert_eq!(taxi.transport, TravelTransport::Taxi);
    assert_eq!(taxi.fare_simoleons, 50);
    taxi.household_simoleons = 25;
    assert!(!taxi.can_confirm());
    assert_eq!(dialog.key_press("Enter", &taxi), None);
    assert_eq!(
        dialog.key_press("Escape", &taxi),
        Some(TravelDialogCommand::Cancel)
    );

    // Sufficient funds enables confirm by click and Enter.
    taxi.household_simoleons = 200;
    assert!(taxi.can_confirm());
    let layout = TravelDialogLayout::compute(800, 600);
    assert!(!rects_overlap(layout.confirm_rect, layout.cancel_rect));
    let cx = layout.confirm_rect.x + layout.confirm_rect.width / 2;
    let cy = layout.confirm_rect.y + layout.confirm_rect.height / 2;
    assert!(dialog.pointer_down(cx, cy, &layout, &taxi));
    assert_eq!(
        dialog.pointer_up(cx, cy, &layout, &taxi),
        Some(TravelDialogCommand::Confirm {
            destination: Destination::Downtown,
            transport: TravelTransport::Taxi,
        })
    );

    // Magic Hole to Magic Town with MagiCoin fare.
    let mut hole = TravelDialogState::for_destination(Destination::MagicTown, vec!["Sim B".into()]);
    assert_eq!(hole.transport, TravelTransport::MagicHole);
    hole.fare_magic_coins = 30;
    hole.household_magic_coins = 30;
    assert_eq!(
        dialog.key_press("Enter", &hole),
        Some(TravelDialogCommand::Confirm {
            destination: Destination::MagicTown,
            transport: TravelTransport::MagicHole,
        })
    );

    // Empty or whitespace-only traveler list blocks confirmation even if fare is 0.
    let empty_shuttle = TravelDialogState::for_destination(Destination::StudioTown, Vec::new());
    assert!(!empty_shuttle.can_confirm());
    let blank_shuttle =
        TravelDialogState::for_destination(Destination::StudioTown, vec!["   ".into()]);
    assert!(!blank_shuttle.can_confirm());

    // Negative fares still draw cleanly at tiny viewports (120x100) without panicking.
    let mut neg_fare =
        TravelDialogState::for_destination(Destination::MagicTown, vec!["Sim".into()]);
    neg_fare.fare_simoleons = -50;
    assert!(neg_fare.can_confirm());

    let art = Art::default();
    let strings = TravelStrings::default();
    for (w, h) in [(320u32, 240u32), (120, 100)] {
        let mut frame = Frame::new(w, h, [0, 0, 0]);
        let compact_layout = dialog.draw(&mut frame, &neg_fare, &art, &strings);
        assert!(compact_layout.panel_rect.width > 0);
    }

    // Releasing pointer outside any control clears hovered state so no stale tooltip lingers.
    let bar_state = TravelState::default();
    let bar_layout = TravelLayout::compute(800, 600, &bar_state);
    let mut chrome = TravelChrome::new();
    let home = bar_layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::ReturnHome)
        .expect("ReturnHome");
    chrome.pointer_move(home.rect.x + 2, home.rect.y + 2, &bar_layout);
    assert_eq!(chrome.hovered(), Some(TravelControl::ReturnHome));
    assert_eq!(chrome.pointer_up(799, 599, &bar_layout, &bar_state), None);
    assert_eq!(chrome.hovered(), None);
}

#[test]
fn complete_collection_loads_travel_art_and_localized_tooltips() {
    let Some(root) = sims_install() else {
        return;
    };
    let install = Install::detect(&root).expect("open Complete Collection install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, US_ENGLISH).expect("load travel art");
    let strings = TravelStrings::load(&install, US_ENGLISH).expect("load UIText.iff strings");

    assert!(
        art.missing.is_empty(),
        "unexpected missing travel art IDs: {:?}",
        art.missing
    );
    assert!(!strings.downtown_tips.is_empty());
    assert!(!strings.vacation_tips.is_empty());
    assert!(!strings.studio_town_tips.is_empty());
    assert!(!strings.magic_town_tips.is_empty());
    assert!(strings.filter_tips.len() >= 13);
    assert!(!strings.go_downtown_dialog.is_empty());
    assert!(!strings.go_vacation_dialog.is_empty());
    assert!(!strings.visit_neighborhood.is_empty());
    assert!(!strings.visit_magic_town.is_empty());

    let chrome = TravelChrome::new();
    let dialog = TravelDialog::new();
    for dest in Destination::ALL {
        assert!(
            strings.tooltip(dest, TravelControl::ReturnHome).is_some(),
            "missing return tooltip for {dest:?}"
        );
        for control in [
            TravelControl::Web,
            TravelControl::Credits,
            TravelControl::ImportLot,
            TravelControl::Bulldoze,
        ] {
            assert!(
                strings.tooltip(dest, control).is_some(),
                "missing {control:?} tooltip for {dest:?}"
            );
        }
        let state = TravelState {
            destination: dest,
            ..TravelState::default()
        };
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        let layout = chrome.draw(&mut frame, &state, &art, &strings);
        assert_eq!(layout.controls.len(), dest.bar_buttons().len());

        let dstate = TravelDialogState::for_destination(dest, vec!["Sim".into()]);
        dialog.draw(&mut frame, &dstate, &art, &strings);
    }

    assert!(
        strings
            .tooltip(Destination::MagicTown, TravelControl::MoveIn)
            .is_some(),
        "Magic Town's STR# 174 has a sixth (Move In) tooltip"
    );
    assert!(
        strings
            .tooltip(
                Destination::StudioTown,
                TravelControl::ToggleFilter(LotFilter::Music)
            )
            .is_some()
    );
    assert_ne!(
        strings.tooltip(Destination::Downtown, TravelControl::Bulldoze),
        strings.downtown_tips.get(1).map(String::as_str),
        "Bulldoze tooltip must map to STR# entry 4, not entry 1"
    );
    assert_ne!(
        strings.tooltip(
            Destination::VacationIsland,
            TravelControl::ToggleFilter(LotFilter::Lodging)
        ),
        strings.tooltip(
            Destination::VacationIsland,
            TravelControl::ToggleFilter(LotFilter::DogCat)
        )
    );

    // Destination::OldTown has its own distinct identity, PHONE_NEIGHBORHOOD (5433)
    // transport icon, Trolley transport, full 50..=79 range, Unleashed community filters, and travel dialog.
    assert_ne!(Destination::OldTown, Destination::Downtown);
    assert_eq!(Destination::OldTown.short_label(), "Old Town");
    assert_eq!(Destination::OldTown.lot_range(), (50, 79));
    assert_eq!(
        Destination::OldTown.default_transport(),
        sims_ui::travel::TravelTransport::Trolley
    );
    assert_eq!(Destination::OldTown.default_transport().label(), "Trolley");
    assert_eq!(
        Destination::OldTown
            .default_transport()
            .icon_art_id(Destination::OldTown),
        sims_ui::travel::art_id::PHONE_NEIGHBORHOOD
    );
    let old_town_dlg = TravelDialogState::for_destination(Destination::OldTown, vec!["Sim".into()]);
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let layout = dialog.draw(&mut frame, &old_town_dlg, &art, &strings);
    assert!(layout.panel_rect.width > 100);
}

/// Pixel guard: with the real art, the browse bar is `banner_downtown.bmp`
/// plus the native button sprites at their native origins, and nothing is
/// drawn below row 52. Any invented label, tab, badge, or strip changes a
/// banner pixel outside the sprites or paints the map area.
#[test]
fn browse_bar_draws_only_the_banner_and_native_sprites() {
    let Some(root) = sims_install() else {
        return;
    };
    let install = Install::detect(&root).expect("open Complete Collection install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, US_ENGLISH).expect("load travel art");
    let strings = TravelStrings::load(&install, US_ENGLISH).expect("load UIText.iff strings");
    let banner = art
        .image(sims_ui::travel::art_id::DOWNTOWN_BAR_BKG)
        .expect("banner_downtown.bmp");
    assert_eq!((banner.width, banner.height), (800, 52));
    const CLEAR: [u8; 3] = [1, 2, 3];
    for dest in Destination::ALL {
        for mode in [TravelBarMode::Browse, TravelBarMode::Bulldoze] {
            let state = TravelState {
                destination: dest,
                mode,
                ..TravelState::default()
            };
            let mut frame = Frame::new(1024, 768, CLEAR);
            let layout = TravelChrome::new().draw(&mut frame, &state, &art, &strings);
            let [ox, oy] = layout.origin;
            // Expected bar: the banner, then each native cell in table order
            // (enabled 0, held 1, disabled 3; the logo is one cell), clipped
            // to the bar. Nothing else may differ anywhere in the frame.
            let mut expected = Frame::new(1024, 768, CLEAR);
            expected.draw_all(banner, [ox, oy]);
            for c in &layout.controls {
                let id = match c.control {
                    TravelControl::ReturnHome => sims_ui::travel::art_id::DOWNTOWN_RETURN,
                    TravelControl::Web => sims_ui::travel::art_id::INET,
                    TravelControl::Credits => sims_ui::travel::art_id::DOWNTOWN_LOGO,
                    TravelControl::ImportLot => dest.import_art_id(),
                    TravelControl::Bulldoze => sims_ui::travel::art_id::DOWNTOWN_BULLDOZE,
                    TravelControl::MoveIn => sims_ui::travel::art_id::MOVE_IN,
                    TravelControl::ToggleFilter(_) => unreachable!("no filters on the bar"),
                };
                let cell = if !c.enabled {
                    3
                } else if c.active {
                    1
                } else {
                    0
                };
                let sprite = if c.control == TravelControl::Credits {
                    art.image(id).cloned()
                } else {
                    art.cell(id, 4, cell)
                }
                .expect("button art");
                let visible = (sprite.width as i32).min(ox + 800 - c.origin[0]);
                expected.draw(
                    &sprite,
                    Rect::new(0, 0, visible, sprite.height as i32),
                    c.origin,
                );
            }
            for y in 0..768 {
                for x in 0..1024 {
                    assert_eq!(
                        frame.pixel(x, y),
                        expected.pixel(x, y),
                        "{dest:?} {mode:?} drew a non-native pixel at ({x}, {y})"
                    );
                }
            }
        }
    }
}

/// Bulldoze (`373`) draws after Return (`323`) and Import (`422`), covering
/// their edges: each visible column hits the button drawn there.
#[test]
fn bar_hit_rects_follow_the_visible_sprites() {
    let state = TravelState {
        destination: Destination::Downtown,
        has_importable_lot: true,
        ..TravelState::default()
    };
    let layout = TravelLayout::compute(800, 600, &state);
    let hit = |x| layout.hit(x, 26).map(|c| c.control);
    assert_eq!(hit(323), Some(TravelControl::ReturnHome));
    assert_eq!(hit(372), Some(TravelControl::ReturnHome));
    assert_eq!(hit(373), Some(TravelControl::Bulldoze));
    assert_eq!(hit(425), Some(TravelControl::Bulldoze));
    assert_eq!(hit(426), Some(TravelControl::ImportLot));
    assert_eq!(hit(474), Some(TravelControl::ImportLot));
    assert_eq!(hit(475), None);
    assert_eq!(hit(799), Some(TravelControl::Web));
    assert_eq!(hit(5), Some(TravelControl::Credits));
    assert_eq!(hit(74), Some(TravelControl::Credits));
    assert_eq!(hit(75), None);
}

/// Like `cWinBtn`, a held button shows its down frame only while the pointer
/// is over it, and the click still lands when released back on it.
#[test]
fn held_button_shows_down_frame_only_while_hovered() {
    let state = TravelState::default();
    let art = Art::default();
    let strings = TravelStrings::default();
    let layout = TravelLayout::compute(800, 600, &state);
    let ret = layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::ReturnHome)
        .copied()
        .expect("Return");
    let (cx, cy) = (
        ret.rect.x + ret.rect.width / 2,
        ret.rect.y + ret.rect.height / 2,
    );
    let mut chrome = TravelChrome::new();
    let center = |chrome: &TravelChrome| {
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        chrome.draw(&mut frame, &state, &art, &strings);
        frame.pixel(cx, cy).unwrap()
    };
    let idle = center(&chrome);
    assert!(chrome.pointer_down(cx, cy, &layout));
    let down = center(&chrome);
    assert_ne!(idle, down, "pressed button shows its down frame");
    chrome.pointer_move(400, 300, &layout);
    assert_eq!(center(&chrome), idle, "dragging off drops the down frame");
    chrome.pointer_move(cx, cy, &layout);
    assert_eq!(center(&chrome), down, "dragging back restores it");
    assert_eq!(
        chrome.pointer_up(cx, cy, &layout, &state),
        Some(TravelCommand::ReturnToNeighborhood)
    );
}
