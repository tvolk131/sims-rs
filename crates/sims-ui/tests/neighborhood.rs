use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::neighborhood::{
    BAR_HEIGHT, BarButton, BarLayout, ExpansionFlags, LotInfo, LotKind, LotOccupant,
    LotPopupLayout, MAX_NEIGHBORHOOD, MIN_NEIGHBORHOOD, NeighborhoodChrome, NeighborhoodCommand,
    NeighborhoodMode, NeighborhoodState, NeighborhoodStrings, art_id, format_lot_address,
    format_lot_popup_text, format_simoleons, load_art,
};
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

fn synthetic_strings() -> NeighborhoodStrings {
    NeighborhoodStrings {
        bar: (0..14).map(|i| format!("tip_{i}")).collect(),
        popup: vec![
            "Fam %s\r\nNW %s\r\nFr %d".into(),                  // 0
            "%s\r\n%s\r\nCost %s\r\nVacant".into(),             // 1
            "%s\r\n%s\r\nCost %s\r\nNoHouse %s %s".into(),      // 2
            "%s\r\n%s\r\nCost %s\r\nOkHouse %s".into(),         // 3
            "%s\r\n%s\r\nCost %s\r\nNoLot %s %s".into(),        // 4
            "%s\r\n%s\r\nCost %s\r\nOkLot %s".into(),           // 5
            "Tut".into(),                                       // 6
            "%s StreetA".into(),                                // 7
            "TutNoMove".into(),                                 // 8
            "Fam %s\r\nNW %s\r\nFr %d\r\nEvict".into(),         // 9
            "Demolish".into(),                                  // 10
            "Nothing".into(),                                   // 11
            "TutNoDoze".into(),                                 // 12
            "%s StreetB".into(),                                // 13
            "%s StreetC".into(),                                // 14
            "%s StreetD".into(),                                // 15
            "%s StreetE".into(),                                // 16
            "%s\r\n%s\r\nComm".into(),                          // 17
            "%s\r\n%s\r\nNoCommMove".into(),                    // 18
            "%s\r\n%s\r\nRezone".into(),                        // 19
            "%s\r\n%s\r\nNoVisitRes".into(),                    // 20
            "%s\r\n%s\r\nVisitComm".into(),                     // 21
            "Unnamed".into(),                                   // 22
            "NoDesc".into(),                                    // 23
            "%s StreetF".into(),                                // 24
            "%s StreetG".into(),                                // 25
            String::new(),                                      // 26
            "%s StreetH".into(),                                // 27
            "%s\r\n%s\r\nNoVisitMag".into(),                    // 28
            "%s\r\n%s\r\nCost %s MC %s\r\nMagVac".into(),       // 29
            String::new(),                                      // 30
            "%s\r\n%s\r\nCost %s MC %s\r\nMagOk".into(),        // 31
            "1500".into(),                                      // 32
            "1000".into(),                                      // 33
            "Fam %s\r\nNW %s MC %s\r\nFr %d".into(),            // 34
            "Fam %s\r\nNW %s MC %s\r\nFr %d\r\nEvict".into(),   // 35
            "%s\r\n%s\r\nCost %s MC %s\r\nNoMC %s %s".into(),   // 36
            "%s\r\n%s\r\nCost %s MC %s\r\nNoBoth %s %s".into(), // 37
            "%s\r\n%s\r\nCost %s MC %s\r\nNoCash %s %s".into(), // 38
        ],
        expansions: (0..5).map(|i| format!("exp_{i}")).collect(),
    }
}

fn dummy_portrait(rgb: [u8; 3]) -> Image {
    let mut rgba = Vec::with_capacity(32 * 32 * 4);
    for _ in 0..(32 * 32) {
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Image {
        width: 32,
        height: 32,
        rgba,
    }
}

#[test]
fn native_and_wide_bar_layouts_match_191_coords_without_overlapping_hits() {
    let art = Art::default();
    let state = NeighborhoodState::default();

    for (w, h) in [(800u32, 600u32), (1024, 768), (1280, 720)] {
        let layout = BarLayout::compute(w, h, &state, &art);
        let ox = (w as i32 - 800) / 2;
        assert_eq!(layout.bar_rect, Rect::new(ox, 0, 800, BAR_HEIGHT));
        assert_eq!(
            layout
                .button(BarButton::PreviousNeighborhood)
                .unwrap()
                .draw_rect
                .x,
            ox + 106
        );
        assert_eq!(
            layout
                .button(BarButton::NextNeighborhood)
                .unwrap()
                .draw_rect
                .x,
            ox + 141
        );
        assert_eq!(
            layout.button(BarButton::SelectFamily).unwrap().draw_rect.x,
            ox + 200
        );
        assert_eq!(
            layout.button(BarButton::Quit).unwrap().draw_rect.x,
            ox + 600
        );
        assert_eq!(layout.button(BarButton::Web).unwrap().draw_rect.x, ox + 762);
        assert_eq!(layout.buttons.len(), 13);

        for i in 0..layout.buttons.len() {
            let a = layout.buttons[i].hit_rect;
            assert!(
                a.x >= 0 && a.x + a.width <= w as i32,
                "button {i} out of bounds"
            );
            assert!(a.y >= 0 && a.y + a.height <= h as i32);
            for j in (i + 1)..layout.buttons.len() {
                let b = layout.buttons[j].hit_rect;
                assert!(
                    !rects_overlap(a, b),
                    "buttons {:?} and {:?} overlap at {w}x{h}: {a:?} vs {b:?}",
                    layout.buttons[i].button,
                    layout.buttons[j].button
                );
            }
        }
    }
}

#[test]
fn compact_widths_have_zero_overlapping_hit_targets() {
    let art = Art::default();
    let state = NeighborhoodState::default();
    for w in (480..=799).step_by(7) {
        let layout = BarLayout::compute(w, 480, &state, &art);
        assert!(layout.buttons.len() >= 12);
        for i in 0..layout.buttons.len() {
            let a = layout.buttons[i].hit_rect;
            assert!(a.width > 0 && a.height > 0);
            assert!(
                a.x >= 0 && a.x + a.width <= w as i32,
                "out of bounds at width {w}: {a:?}"
            );
            for j in (i + 1)..layout.buttons.len() {
                let b = layout.buttons[j].hit_rect;
                assert!(
                    !rects_overlap(a, b),
                    "overlap at width {w} between {:?} ({a:?}) and {:?} ({b:?})",
                    layout.buttons[i].button,
                    layout.buttons[j].button
                );
            }
        }
    }
}

#[test]
fn neighborhood_number_cycles_1_through_8() {
    let mut state = NeighborhoodState {
        neighborhood_number: 0,
        ..Default::default()
    };
    assert_eq!(state.clamped_neighborhood(), MIN_NEIGHBORHOOD);
    assert_eq!(state.previous_neighborhood(), MAX_NEIGHBORHOOD);
    assert_eq!(state.next_neighborhood(), 2);

    state.neighborhood_number = 8;
    assert_eq!(state.previous_neighborhood(), 7);
    assert_eq!(state.next_neighborhood(), MIN_NEIGHBORHOOD);

    state.neighborhood_number = 99;
    assert_eq!(state.clamped_neighborhood(), MAX_NEIGHBORHOOD);
}

#[test]
fn bar_button_click_semantics_respect_modes_and_expansions() {
    let art = Art::default();
    let mut chrome = NeighborhoodChrome::new();
    let mut state = NeighborhoodState::default();
    let size = [800, 600];
    let layout = BarLayout::compute(size[0], size[1], &state, &art);

    let click = |chrome: &mut NeighborhoodChrome, state: &NeighborhoodState, btn: BarButton| {
        let r = layout.button(btn).unwrap().hit_rect;
        let cx = r.x + r.width / 2;
        let cy = r.y + r.height / 2;
        assert!(chrome.press(cx, cy, size, state, &art));
        chrome.release(cx, cy, size, state, &art)
    };

    assert_eq!(
        click(&mut chrome, &state, BarButton::PreviousNeighborhood),
        Some(NeighborhoodCommand::PreviousNeighborhood)
    );
    assert_eq!(
        click(&mut chrome, &state, BarButton::NextNeighborhood),
        Some(NeighborhoodCommand::NextNeighborhood)
    );
    assert_eq!(
        click(&mut chrome, &state, BarButton::SelectFamily),
        Some(NeighborhoodCommand::OpenFamilyBin)
    );
    // Import disabled when no importable family exists.
    assert_eq!(click(&mut chrome, &state, BarButton::ImportFamily), None);
    state.has_importable_family = true;
    assert_eq!(
        click(&mut chrome, &state, BarButton::ImportFamily),
        Some(NeighborhoodCommand::ImportFamily)
    );

    // Move-In mode turns SelectFamily into CancelMoveIn.
    state.mode = NeighborhoodMode::MoveIn {
        family_name: "Alpha".into(),
        family_funds: 20_000,
        family_magic_coins: 0,
    };
    assert!(BarButton::SelectFamily.is_toggled(&state));
    assert_eq!(
        click(&mut chrome, &state, BarButton::SelectFamily),
        Some(NeighborhoodCommand::CancelMoveIn)
    );

    // Bulldoze mode disables other buttons while keeping Bulldoze active.
    state.mode = NeighborhoodMode::Bulldoze;
    assert!(BarButton::Bulldoze.is_toggled(&state));
    assert_eq!(click(&mut chrome, &state, BarButton::Quit), None);
    assert_eq!(
        click(&mut chrome, &state, BarButton::Bulldoze),
        Some(NeighborhoodCommand::ToggleBulldoze)
    );

    // Missing Hot Date disables Downtown and selects its expansion tooltip.
    state.mode = NeighborhoodMode::Normal;
    state.expansions = ExpansionFlags(ExpansionFlags::ALL.0 & !ExpansionFlags::HOT_DATE);
    assert_eq!(click(&mut chrome, &state, BarButton::Downtown), None);
    let strings = synthetic_strings();
    assert_eq!(
        strings.button_tooltip(BarButton::Downtown, &state),
        Some("exp_0")
    );
    assert_eq!(
        click(&mut chrome, &state, BarButton::VacationIsland),
        Some(NeighborhoodCommand::OpenVacationIsland)
    );

    // Dragging off a button cancels activation.
    let q = layout.button(BarButton::Quit).unwrap().hit_rect;
    assert!(chrome.press(q.x + 10, q.y + 10, size, &state, &art));
    assert_eq!(chrome.release(10, 200, size, &state, &art), None);
}

#[test]
fn lot_popup_formats_occupied_vacant_movein_cant_afford_and_clamps_below_bar() {
    let art = Art::default();
    let strings = synthetic_strings();
    let mut state = NeighborhoodState::default();

    // Occupied residential lot with 7 portraits (wraps to 2 rows at narrow width).
    let occupied = LotInfo {
        lot_number: 5,
        kind: LotKind::Residential,
        occupant: Some(LotOccupant {
            family_name: "Goth".into(),
            funds: 5_000,
            house_value: 30_658,
            net_worth: 35_658,
            friends: 2,
            magic_coins: 0,
            portraits: (0..7).map(|_| dummy_portrait([100, 120, 140])).collect(),
        }),
        anchor: [760, 10], // deliberately top-right above BAR_HEIGHT
        ..Default::default()
    };
    assert_eq!(format_lot_address(&occupied, &strings), "5 StreetA");
    let (text, cant_afford) = format_lot_popup_text(&occupied, &state, &strings);
    assert!(!cant_afford);
    assert!(text.contains("5 StreetA"));
    assert!(text.contains("Fam Goth"));
    assert!(text.contains("$35,658"));
    assert!(text.contains("Fr 2"));

    let popup = LotPopupLayout::compute(&occupied, &state, [800, 600], &art, &strings);
    assert!(
        popup.rect.y >= BAR_HEIGHT,
        "popup must never overlap top bar"
    );
    assert!(popup.rect.x + popup.rect.width <= 800);
    assert!(popup.rect.y + popup.rect.height <= 600);
    assert_eq!(popup.portrait_rects.len(), 7);
    for &pr in &popup.portrait_rects {
        assert!(!rects_overlap(popup.text_rect, pr));
    }

    // Tiny viewport (160x100): portraits must never overlap text_rect.
    let tiny_popup = LotPopupLayout::compute(&occupied, &state, [160, 100], &art, &strings);
    for &pr in &tiny_popup.portrait_rects {
        assert!(
            !rects_overlap(tiny_popup.text_rect, pr),
            "portrait {pr:?} overlaps text_rect {:?} at 160x100",
            tiny_popup.text_rect
        );
    }

    // Vacant house in Move-In mode: unaffordable vs affordable.
    let vacant = LotInfo {
        lot_number: 78,
        house_name: "Villa".into(),
        house_description: "Big house".into(),
        kind: LotKind::Residential,
        has_house: true,
        house_cost: 32_415,
        anchor: [400, 300],
        ..Default::default()
    };
    state.mode = NeighborhoodMode::MoveIn {
        family_name: "Bachelor".into(),
        family_funds: 20_000,
        family_magic_coins: 0,
    };
    let (text_no, cant) = format_lot_popup_text(&vacant, &state, &strings);
    assert!(cant);
    assert!(text_no.contains("NoHouse Bachelor $20,000"));

    state.mode = NeighborhoodMode::MoveIn {
        family_name: "Rich".into(),
        family_funds: 50_000,
        family_magic_coins: 0,
    };
    let (text_ok, cant_ok) = format_lot_popup_text(&vacant, &state, &strings);
    assert!(!cant_ok);
    assert!(text_ok.contains("OkHouse $17,585"));

    // Magic Town residential lot checks both cash and MagiCoins.
    let magic_lot = LotInfo {
        lot_number: 90,
        house_name: "Tower".into(),
        house_description: "Spooky".into(),
        kind: LotKind::MagicResidential,
        has_house: true,
        house_cost: 10_000,
        magic_cost: 1_000,
        ..Default::default()
    };
    state.mode = NeighborhoodMode::MoveIn {
        family_name: "Wizard".into(),
        family_funds: 20_000,
        family_magic_coins: 250,
    };
    let (text_mc, cant_mc) = format_lot_popup_text(&magic_lot, &state, &strings);
    assert!(cant_mc);
    assert!(text_mc.contains("NoMC Wizard 250"));

    // Bulldoze mode on built vacant lot vs empty vacant lot.
    state.mode = NeighborhoodMode::Bulldoze;
    let (text_doze, _) = format_lot_popup_text(&vacant, &state, &strings);
    assert!(text_doze.contains("Demolish"));
    let empty_lot = LotInfo {
        has_house: false,
        ..vacant
    };
    let (text_empty_doze, _) = format_lot_popup_text(&empty_lot, &state, &strings);
    assert!(text_empty_doze.contains("Nothing"));
}

#[test]
fn simoleons_and_zero_size_frame_safety() {
    assert_eq!(format_simoleons(0), "$0");
    assert_eq!(format_simoleons(999), "$999");
    assert_eq!(format_simoleons(20_000), "$20,000");
    assert_eq!(format_simoleons(1_234_567), "$1,234,567");
    assert_eq!(format_simoleons(-5_500), "-$5,500");

    let art = Art::default();
    let strings = synthetic_strings();
    let state = NeighborhoodState::default();
    let chrome = NeighborhoodChrome::new();
    let mut empty_frame = Frame::new(0, 0, [0, 0, 0]);
    let layout = chrome.draw(&mut empty_frame, &state, &art, &strings);
    assert!(layout.buttons.is_empty());
}

#[test]
fn install_neighborhood_art_strings_and_rendering() {
    let Some(install) = install() else { return };
    let ui = UiGraphics::open(&install).unwrap();
    let art = load_art(&ui, 1).unwrap();
    assert!(
        art.missing.is_empty(),
        "missing neighborhood resources: {:?}",
        art.missing
    );
    assert_eq!(art.image(art_id::BAR_BKG).unwrap().width, 800);
    assert_eq!(art.image(art_id::BAR_BKG).unwrap().height, 52);
    assert_eq!(art.image(art_id::POPUP_TILER).unwrap().width, 111);
    assert_eq!(art.image(art_id::POPUP_TILER).unwrap().height, 111);

    let strings = NeighborhoodStrings::load(&install, 1).unwrap();
    assert!(strings.bar.len() >= 13);
    assert!(strings.popup.len() >= 39);
    assert!(strings.expansions.len() >= 5);

    // Render clean 800x600 neighborhood chrome with occupied lot hover card.
    let mut state = NeighborhoodState {
        neighborhood_number: 1,
        hovered_lot: Some(LotInfo {
            lot_number: 5,
            kind: LotKind::Residential,
            occupant: Some(LotOccupant {
                family_name: "Goth".into(),
                funds: 5_000,
                house_value: 30_658,
                net_worth: 35_658,
                friends: 0,
                magic_coins: 0,
                portraits: vec![
                    dummy_portrait([180, 120, 90]),
                    dummy_portrait([150, 100, 80]),
                    dummy_portrait([120, 140, 170]),
                ],
            }),
            anchor: [500, 260],
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut chrome = NeighborhoodChrome::new();
    let mut frame = Frame::new(800, 600, [40, 90, 30]);
    let layout = chrome.draw(&mut frame, &state, &art, &strings);
    assert_eq!(layout.buttons.len(), 13);

    // Top bar and popup card must both paint over the green background.
    let bar_painted = (0..800)
        .filter(|&x| frame.pixel(x, 20) != Some([40, 90, 30]))
        .count();
    assert!(bar_painted > 700, "top bar should span 800 px");
    let popup_painted = (260..400)
        .flat_map(|y| (500..750).map(move |x| (x, y)))
        .filter(|&(x, y)| frame.pixel(x, y) != Some([40, 90, 30]))
        .count();
    assert!(popup_painted > 5_000, "lot hover popup should paint card");

    // Hovering Move-In button shows tooltip, and active Move-In draws cyan ring.
    state.mode = NeighborhoodMode::MoveIn {
        family_name: "Bachelor".into(),
        family_funds: 20_000,
        family_magic_coins: 0,
    };
    let sel_rect = layout.button(BarButton::SelectFamily).unwrap().hit_rect;
    chrome.hover(
        sel_rect.x + sel_rect.width / 2,
        sel_rect.y + sel_rect.height / 2,
        [800, 600],
        &state,
        &art,
    );
    let mut frame_active = Frame::new(800, 600, [40, 90, 30]);
    chrome.draw(&mut frame_active, &state, &art, &strings);
    let fa = &frame_active;
    let cyan_pixels = (0..52)
        .flat_map(|y| (200..253).map(move |x| fa.pixel(x, y).unwrap()))
        .filter(|&[r, g, b]| r < 40 && g > 200 && b > 200)
        .count();
    assert!(
        cyan_pixels > 50,
        "active Move-In button should show cyan selection ring, got {cyan_pixels}"
    );
    let tooltip_cream = (52..90)
        .flat_map(|y| (100..400).map(move |x| fa.pixel(x, y).unwrap()))
        .filter(|&px| px == [255, 255, 230])
        .count();
    assert!(
        tooltip_cream > 100,
        "hovering SelectFamily should draw tooltip box, got {tooltip_cream}"
    );
}
