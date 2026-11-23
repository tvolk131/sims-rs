//! Asset-backed Buy Mode catalog & popup chrome tests against a real Complete
//! Collection install (`SIMS_INSTALL`).

mod common;

use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, Index, Install};
use sims_ui::art::Art;
use sims_ui::build::{self, BuildCatalog, BuildState};
use sims_ui::buy::{self, BuyCatalog, BuySort, BuyState, BuyZone, RatingLabels, SellLabels};
use sims_ui::command::{BuildTool, Command};
use sims_ui::dialog;
use sims_ui::frame::Frame;
use sims_ui::thumbnail::{TEMPLATE, Thumbnails};

#[test]
fn buy_catalog_loads_residential_and_all_five_expansion_commercial_zones() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test BuyCatalog");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let ratings = RatingLabels::load(&install);
    let sell = SellLabels::load(&install);

    // All 20 `GameData/UIText.iff STR# 160` entries load cleanly.
    let labels = ratings.labels();
    assert_eq!(labels.len(), 20);
    for (idx, text) in labels.iter().enumerate() {
        assert!(!text.is_empty(), "STR# 160[{idx}] must be non-empty");
    }
    assert!(
        labels[14].to_ascii_lowercase().contains("adult"),
        "STR# 160[14] = {:?}",
        labels[14]
    );
    assert!(
        labels[15].to_ascii_lowercase().contains("kid"),
        "STR# 160[15] = {:?}",
        labels[15]
    );
    assert!(
        labels[16].to_ascii_lowercase().contains("group"),
        "STR# 160[16] = {:?}",
        labels[16]
    );
    assert!(
        labels[17].to_ascii_lowercase().contains("pet"),
        "STR# 160[17] = {:?}",
        labels[17]
    );
    assert!(
        labels[18].to_ascii_lowercase().contains("dog"),
        "STR# 160[18] = {:?}",
        labels[18]
    );
    assert!(
        labels[19].to_ascii_lowercase().contains("cat"),
        "STR# 160[19] = {:?}",
        labels[19]
    );

    let buy_cat = BuyCatalog::load(&index, &catalog)
        .with_rating_labels(ratings)
        .with_sell_labels(sell);

    // Residential catalog populated across ByFunction and ByRoom (`720` on Complete Collection).
    let residential = buy_cat.items_for_zone(BuyZone::Residential);
    assert!(
        residential.len() >= 700,
        "expected >= 700 residential items, got {}",
        residential.len()
    );
    for sort in [BuySort::ByFunction, BuySort::ByRoom] {
        for cat in 0..8 {
            let state = BuyState {
                zone: BuyZone::Residential,
                sort,
                category: Some(cat),
                subcategory: 7,
                page: 0,
                selected: None,
            };
            let _ = buy_cat.filtered(&state);
        }
    }

    // All 5 expansion commercial catalogs must be populated (previously dropped!).
    let downtown = buy_cat.items_for_zone(BuyZone::Downtown);
    let vacation = buy_cat.items_for_zone(BuyZone::Vacation);
    let old_town = buy_cat.items_for_zone(BuyZone::Community);
    let studio = buy_cat.items_for_zone(BuyZone::StudioTown);
    let magic = buy_cat.items_for_zone(BuyZone::MagicTown);

    assert!(
        downtown.len() >= 700,
        "expected Downtown >= 700, got {}",
        downtown.len()
    );
    assert!(
        vacation.len() >= 700,
        "expected Vacation >= 700, got {}",
        vacation.len()
    );
    assert!(
        old_town.len() >= 650,
        "expected OldTown >= 650, got {}",
        old_town.len()
    );
    assert!(
        studio.len() >= 550,
        "expected StudioTown >= 550, got {}",
        studio.len()
    );
    assert!(
        magic.len() >= 550,
        "expected MagicTown >= 550, got {}",
        magic.len()
    );

    let has_name = |items: &[&buy::BuyItem], needle: &str| {
        let needle_l = needle.to_ascii_lowercase();
        items
            .iter()
            .any(|i| i.name.to_ascii_lowercase().contains(&needle_l))
    };
    assert!(
        has_name(&downtown, "register") || has_name(&downtown, "checkout"),
        "Downtown catalog must include a cash register"
    );
    assert!(
        has_name(&downtown, "podium") || has_name(&downtown, "maitre"),
        "Downtown catalog must include a dining podium"
    );
    assert!(
        has_name(&vacation, "tent"),
        "Vacation catalog must include a tent"
    );
    assert!(
        has_name(&studio, "runway") || has_name(&studio, "fashion"),
        "StudioTown catalog must include a fashion runway"
    );

    // At least some objects expose TTAB usage restrictions & group activity.
    assert!(
        buy_cat.all().iter().any(|i| i.usage_restriction.is_some()),
        "expected real Complete Collection objects with usage restrictions (STR# 160[14..=19])"
    );
    assert!(
        buy_cat.all().iter().any(|i| i.group_activity),
        "expected real Complete Collection objects with Group Activity (STR# 160[16])"
    );

    // Verify BuyZone::for_lot selects every zone cleanly.
    assert_eq!(BuyZone::for_lot(5, Some(1)), BuyZone::Residential);
    assert_eq!(BuyZone::for_lot(25, Some(2)), BuyZone::Downtown);
    assert_eq!(BuyZone::for_lot(42, Some(2)), BuyZone::Vacation);
    assert_eq!(BuyZone::for_lot(58, Some(2)), BuyZone::Community);
    assert_eq!(BuyZone::for_lot(81, Some(2)), BuyZone::StudioTown);
    assert_eq!(BuyZone::for_lot(95, Some(2)), BuyZone::MagicTown);
    assert_eq!(BuyZone::for_lot(90, Some(1)), BuyZone::Residential);
}

#[test]
fn buy_and_build_hover_popups_render_native_popup_info_tiles_chrome() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test Buy/Build popup chrome");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let index = Index::build(&install).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let ui = UiGraphics::open(&install).unwrap();

    let mut art = Art::load(&ui, US_ENGLISH, &[TEMPLATE, dialog::BACKGROUND]).unwrap();
    for id in buy::resource_ids() {
        art.load_optional(&ui, US_ENGLISH, id);
    }
    for id in build::resource_ids() {
        art.load_optional(&ui, US_ENGLISH, id);
    }
    assert!(art.image(dialog::BACKGROUND).is_some());

    // Commercial Buy Mode icons load cleanly and lay out 5 top-level buttons.
    let buy_cat = BuyCatalog::load(&index, &catalog)
        .with_rating_labels(RatingLabels::load(&install))
        .with_sell_labels(SellLabels::load(&install));
    for zone in [
        BuyZone::Downtown,
        BuyZone::Vacation,
        BuyZone::Community,
        BuyZone::StudioTown,
        BuyZone::MagicTown,
    ] {
        let mut st = BuyState::default();
        st.set_zone(zone);
        let top = buy::layout([800, 600], &buy_cat, &st, &art, 50_000);
        assert_eq!(top.widgets.len(), 5, "{zone:?} top-level category count");
        assert!(st.apply(&Command::BuyCategory(0)));
        assert!(st.apply(&Command::BuySubcategory(7)));
        let sub = buy::layout([800, 600], &buy_cat, &st, &art, 50_000);
        let slot_count = sub
            .widgets
            .iter()
            .filter(|w| matches!(w.command, Some(Command::BuyItem(_))))
            .count();
        assert!(
            slot_count > 0,
            "{zone:?} sub-view must populate real Command::BuyItem product slots (got {slot_count})"
        );
    }

    // Buy Mode PopupCard renders full PopupInfoTiles.bmp (3008) 9-slice chrome
    // (all 4 corners, all 4 edges, and tiled interior — not just top-left).
    let sheet = art
        .image(sims_ui::dialog::BACKGROUND)
        .expect("PopupInfoTiles.bmp (3008) loaded");
    let cell_w = sheet.width / 3;
    let cell_h = sheet.height / 3;
    let sheet_rgb = |sx: u32, sy: u32| -> [u8; 3] {
        let p = sheet.pixel(sx, sy).unwrap();
        [p[0], p[1], p[2]]
    };

    let mut buy_state = BuyState::default();
    buy_state.apply(&Command::BuyCategory(0));
    buy_state.apply(&Command::BuySubcategory(7));
    let first_guid = buy_cat.filtered(&buy_state).first().unwrap().guid;
    buy_state.selected = Some(first_guid);
    let buy_layout = buy::layout([800, 600], &buy_cat, &buy_state, &art, 50_000);
    let popup = buy_layout.popup.as_ref().expect("Buy popup card");
    let mut thumbs = Thumbnails::default();
    let _ = thumbs.read(&index, &catalog, first_guid);
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    buy::draw(&mut frame, &art, &thumbs, &buy_layout, None);
    let pr = popup.rect;
    assert_eq!(
        frame.pixel(pr.x, pr.y),
        Some(sheet_rgb(0, 0)),
        "Buy TL corner"
    );
    assert_eq!(
        frame.pixel(pr.x + pr.width - 1, pr.y),
        Some(sheet_rgb(sheet.width - 1, 0)),
        "Buy TR corner"
    );
    assert_eq!(
        frame.pixel(pr.x, pr.y + pr.height - 1),
        Some(sheet_rgb(0, sheet.height - 1)),
        "Buy BL corner"
    );
    assert_eq!(
        frame.pixel(pr.x + pr.width - 1, pr.y + pr.height - 1),
        Some(sheet_rgb(sheet.width - 1, sheet.height - 1)),
        "Buy BR corner"
    );
    assert_eq!(
        frame.pixel(pr.x + pr.width / 2, pr.y),
        Some(sheet_rgb(cell_w, 0)),
        "Buy top edge"
    );
    assert_eq!(
        frame.pixel(pr.x + pr.width / 2, pr.y + pr.height - 1),
        Some(sheet_rgb(cell_w, sheet.height - 1)),
        "Buy bottom edge"
    );
    assert_eq!(
        frame.pixel(pr.x + pr.width - 1, pr.y + pr.height / 2),
        Some(sheet_rgb(sheet.width - 1, cell_h)),
        "Buy right edge"
    );
    assert_eq!(
        frame.pixel(pr.x, pr.y + pr.height / 2),
        Some(sheet_rgb(0, cell_h)),
        "Buy left edge"
    );
    assert_eq!(
        frame.pixel(pr.x + 20, pr.y + pr.height - 16),
        Some(sheet_rgb(cell_w, cell_h)),
        "Buy tiled interior"
    );
    if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("buy_popup_800x600.png"), frame.png());
        // Also capture Downtown & MagicTown commercial Buy Mode strips.
        for (zone, file_name) in [
            (BuyZone::Downtown, "buy_downtown_800x600.png"),
            (BuyZone::MagicTown, "buy_magictown_800x600.png"),
        ] {
            let mut zstate = BuyState::default();
            zstate.set_zone(zone);
            zstate.apply(&Command::BuyCategory(0));
            zstate.apply(&Command::BuySubcategory(7));
            let items = buy_cat.filtered(&zstate);
            let mut zthumbs = Thumbnails::default();
            for item in items.iter().take(12) {
                let _ = zthumbs.read(&index, &catalog, item.guid);
            }
            if let Some(first) = items.first() {
                zstate.selected = Some(first.guid);
            }
            let zlayout = buy::layout([800, 600], &buy_cat, &zstate, &art, 50_000);
            let mut zframe = Frame::new(800, 600, [0, 0, 0]);
            buy::draw(&mut zframe, &art, &zthumbs, &zlayout, None);
            let _ = std::fs::write(dir.join(file_name), zframe.png());
        }
    }

    // Build Mode PopupCard also renders PopupInfoTiles.bmp (3008) chrome.
    let empty_plugins = std::collections::BTreeMap::new();
    let build_cat = BuildCatalog::load(
        &install,
        &index,
        &catalog,
        &[],
        &[],
        &empty_plugins,
        &empty_plugins,
    );
    let mut build_state = BuildState {
        tool: Some(BuildTool::Doors),
        ..BuildState::default()
    };
    let door_id = build_cat.items_for(BuildTool::Doors).first().unwrap().id;
    build_state.selected = Some(door_id);
    let build_layout_base = build::layout(
        [800, 600],
        &build_cat,
        &build_state,
        &art,
        50_000,
        false,
        false,
        None,
    );
    let slot_rect = build_layout_base
        .widgets
        .iter()
        .find(|w| matches!(w.kind, build::BuildWidgetKind::Slot { .. }))
        .expect("at least one Build catalog slot")
        .rect;
    let build_layout = build::layout(
        [800, 600],
        &build_cat,
        &build_state,
        &art,
        50_000,
        false,
        false,
        Some([slot_rect.x + 5, slot_rect.y + 5]),
    );
    let bpopup = build_layout.popup.as_ref().expect("Build popup card");
    let mut bframe = Frame::new(800, 600, [0, 0, 0]);
    build::draw_with_catalog(
        &mut bframe,
        &art,
        &thumbs,
        &Default::default(),
        Some(&build_cat),
        &build_layout,
        None,
    );
    let br = bpopup.rect;
    assert_eq!(
        bframe.pixel(br.x, br.y),
        Some(sheet_rgb(0, 0)),
        "Build TL corner"
    );
    assert_eq!(
        bframe.pixel(br.x + br.width - 1, br.y),
        Some(sheet_rgb(sheet.width - 1, 0)),
        "Build TR corner"
    );
    assert_eq!(
        bframe.pixel(br.x, br.y + br.height - 1),
        Some(sheet_rgb(0, sheet.height - 1)),
        "Build BL corner"
    );
    assert_eq!(
        bframe.pixel(br.x + br.width - 1, br.y + br.height - 1),
        Some(sheet_rgb(sheet.width - 1, sheet.height - 1)),
        "Build BR corner"
    );
    assert_eq!(
        bframe.pixel(br.x + br.width / 2, br.y),
        Some(sheet_rgb(cell_w, 0)),
        "Build top edge"
    );
    assert_eq!(
        bframe.pixel(br.x + br.width / 2, br.y + br.height - 1),
        Some(sheet_rgb(cell_w, sheet.height - 1)),
        "Build bottom edge"
    );
    assert_eq!(
        bframe.pixel(br.x, br.y + br.height / 2),
        Some(sheet_rgb(0, cell_h)),
        "Build left edge"
    );
    assert_eq!(
        bframe.pixel(br.x + br.width - 1, br.y + br.height / 2),
        Some(sheet_rgb(sheet.width - 1, cell_h)),
        "Build right edge"
    );
    assert_eq!(
        bframe.pixel(br.x + 20, br.y + br.height - 16),
        Some(sheet_rgb(cell_w, cell_h)),
        "Build tiled interior"
    );
    if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("build_popup_800x600.png"), bframe.png());
    }
}
