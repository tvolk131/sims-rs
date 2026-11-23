use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::shop::{
    CHOICE_VIEW, ChoiceDialog, CurrencyKind, ShopCommand, ShopItem, ShopKind, ShopPickerDialog,
    ShopSnapshot, ShopStrings, Wallet, WardrobeCategory, load_art,
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

fn sample_icon(rgb: [u8; 3]) -> Image {
    let mut rgba = Vec::with_capacity(16 * 12 * 4);
    for _ in 0..(16 * 12) {
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Image {
        width: 16,
        height: 12,
        rgba,
    }
}

fn sample_boutique_snapshot() -> ShopSnapshot {
    ShopSnapshot {
        kind: ShopKind::DowntownBoutique(WardrobeCategory::Formal),
        title_override: None,
        buy_label_override: None,
        shopper_name: Some("Bella".into()),
        wallet: Wallet {
            simoleons: 350,
            magic_coins: 40,
        },
        items: vec![
            ShopItem {
                id: "formal_01".into(),
                name: "Evening Gown".into(),
                description: "Classic formal outfit for Downtown dinners.".into(),
                price: 250,
                currency: CurrencyKind::Simoleons,
                icon: Some(sample_icon([200, 40, 60])),
                supports_try_on: true,
                owned: false,
                available: true,
            },
            ShopItem {
                id: "formal_02".into(),
                name: "Designer Tuxedo".into(),
                description: "High-fashion formal wear beyond current budget.".into(),
                price: 500,
                currency: CurrencyKind::Simoleons,
                icon: Some(sample_icon([40, 60, 180])),
                supports_try_on: true,
                owned: false,
                available: true,
            },
            ShopItem {
                id: "formal_03".into(),
                name: "Owned Jacket".into(),
                description: "Already in household wardrobe.".into(),
                price: 120,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: true,
                available: true,
            },
        ],
        tried_on_item_id: None,
        preview_image: None,
        preview_label: None,
        hide_wallet_and_prices: false,
        choice: None,
        body_shapes: Vec::new(),
    }
}

#[test]
fn boutique_try_on_buy_affordability_and_cancel_flow() {
    let snap = sample_boutique_snapshot();
    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let size = [800, 600];

    // First item is affordable and supports Try On.
    assert!(dialog.can_try_on_selected(&snap));
    assert!(dialog.can_buy_selected(&snap));
    assert_eq!(dialog.selected_status_hint(&snap), None);

    let layout = dialog.layout(size, &snap);
    let (tx, ty) = center(layout.try_on_btn);
    dialog.pointer_down(size, &snap, tx, ty);
    assert_eq!(
        dialog.pointer_up(size, &snap, tx, ty),
        Some(ShopCommand::TryOn {
            item_id: "formal_01".into()
        })
    );

    let (bx, by) = center(layout.buy_btn);
    dialog.pointer_down(size, &snap, bx, by);
    assert_eq!(
        dialog.pointer_up(size, &snap, bx, by),
        Some(ShopCommand::Buy {
            item_id: "formal_01".into(),
            currency: CurrencyKind::Simoleons,
            price: 250,
        })
    );

    // Second item is unaffordable (§500 > §350): Try On works, Buy is blocked.
    assert!(dialog.select_next(&snap));
    assert!(dialog.can_try_on_selected(&snap));
    assert!(!dialog.can_buy_selected(&snap));
    assert_eq!(
        dialog.selected_status_hint(&snap),
        Some("Insufficient Simoleons")
    );
    dialog.pointer_down(size, &snap, bx, by);
    assert_eq!(dialog.pointer_up(size, &snap, bx, by), None);

    // Third item is already owned: Buy is blocked.
    assert!(dialog.select_next(&snap));
    assert!(!dialog.can_buy_selected(&snap));
    assert_eq!(dialog.selected_status_hint(&snap), Some("Already owned"));

    // Escape closes dialog and emits Cancel.
    assert_eq!(dialog.escape(), Some(ShopCommand::Cancel));
    assert!(!dialog.is_open());
}

#[test]
fn choice_dialog_is_a_view_between_arrows_with_ok_and_cancel() {
    // `cWinClothingChoiceDlg` (`0x5090d0`): no list, a 200×300 3D view with
    // the 300/301 arrows beside it, OK and Cancel below.
    let mut snap = sample_boutique_snapshot();
    snap.choice = Some(ChoiceDialog::Clothing);
    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let size = [800, 600];
    let layout = dialog.layout(size, &snap);
    let view = layout.preview_pane.expect("3D view");
    assert_eq!([view.width as u32, view.height as u32], CHOICE_VIEW);
    assert!(layout.visible_cards.is_empty(), "no list");
    let previous = layout.previous_btn.expect("previous arrow");
    let next = layout.next_btn.expect("next arrow");
    assert!(previous.x + previous.width <= view.x);
    assert!(next.x >= view.x + view.width);
    assert!(!intersects(layout.buy_btn, layout.cancel_btn));
    assert!(layout.buy_btn.y >= view.y + view.height);

    let try_on = |id: &str| Some(ShopCommand::TryOn { item_id: id.into() });
    // The arrows wrap (`0x40d360`, `0x40d229`): previous on the first
    // outfit shows the last, next on the last shows the first.
    let (px, py) = center(previous);
    dialog.pointer_down(size, &snap, px, py);
    assert_eq!(dialog.pointer_up(size, &snap, px, py), try_on("formal_03"));
    let (nx, ny) = center(next);
    dialog.pointer_down(size, &snap, nx, ny);
    assert_eq!(dialog.pointer_up(size, &snap, nx, ny), try_on("formal_01"));
    dialog.pointer_down(size, &snap, nx, ny);
    assert_eq!(dialog.pointer_up(size, &snap, nx, ny), try_on("formal_02"));
    assert_eq!(dialog.choose(&snap, false), try_on("formal_01"));
    // An item that cannot be tried on is still shown (as its fallback).
    let mut plain = snap.clone();
    plain.items[1].supports_try_on = false;
    assert_eq!(dialog.choose(&plain, true), try_on("formal_02"));
    assert_eq!(dialog.choose(&plain, false), try_on("formal_01"));
    // One choice: the arrows have nowhere to go.
    let mut single = snap.clone();
    single.items.truncate(1);
    assert_eq!(dialog.choose(&single, true), None);
    // The wardrobe's arrows pass over outfits of another body shape
    // (`0x40d6fa..0x40d714`), keeping each outfit's own place in the list.
    let mut shaped = snap.clone();
    shaped.body_shapes = vec!["fit".into(), "fat".into(), "fit".into()];
    let mut arrows = dialog.clone();
    arrows.select_item("formal_01");
    assert_eq!(arrows.choose(&shaped, true), try_on("formal_03"));
    assert_eq!(arrows.choose(&shaped, true), try_on("formal_01"));
    assert_eq!(arrows.choose(&shaped, false), try_on("formal_03"));
    // The only `fat` outfit: an arrow comes back to it and nothing moves.
    arrows.select_item("formal_02");
    assert_eq!(arrows.choose(&shaped, true), None);
    assert_eq!(arrows.choose(&shaped, false), None);

    // Cancel closes without buying.
    let mut cancelled = dialog.clone();
    let (cx, cy) = center(layout.cancel_btn);
    cancelled.pointer_down(size, &snap, cx, cy);
    assert_eq!(
        cancelled.pointer_up(size, &snap, cx, cy),
        Some(ShopCommand::Cancel)
    );
    assert!(!cancelled.is_open());

    // OK buys the outfit on show.
    let (bx, by) = center(layout.buy_btn);
    dialog.pointer_down(size, &snap, bx, by);
    assert_eq!(
        dialog.pointer_up(size, &snap, bx, by),
        Some(ShopCommand::Buy {
            item_id: "formal_01".into(),
            currency: CurrencyKind::Simoleons,
            price: 250,
        })
    );
}

#[test]
fn buy_cancels_if_item_removed_or_repriced_between_press_and_release() {
    let snap = sample_boutique_snapshot();
    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let size = [800, 600];

    // Press Buy on formal_01 at §250.
    let layout = dialog.layout(size, &snap);
    let (bx, by) = center(layout.buy_btn);
    assert!(dialog.pointer_down(size, &snap, bx, by));

    // Reprice formal_01 to §300 before release -> must NOT silently accept the new price.
    let mut repriced = snap.clone();
    repriced.items[0].price = 300;
    assert_eq!(dialog.pointer_up(size, &repriced, bx, by), None);

    // Press Buy again, then remove formal_01 before release -> must NOT buy a replacement item.
    assert!(dialog.pointer_down(size, &snap, bx, by));
    let mut removed = snap.clone();
    removed.items.remove(0);
    removed.items[0].price = 100; // even if the replacement is affordable
    assert_eq!(dialog.pointer_up(size, &removed, bx, by), None);
}

#[test]
fn keyboard_selection_scrolls_card_into_view_at_compact_viewport() {
    let snap = sample_boutique_snapshot();
    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let size = [320, 240];

    for expected_idx in 0..snap.items.len() {
        let layout = dialog.layout(size, &snap);
        assert!(
            layout
                .visible_cards
                .iter()
                .any(|&(idx, _)| idx == expected_idx),
            "selected card {expected_idx} must be visible in {:?}",
            layout.visible_cards
        );
        if expected_idx + 1 < snap.items.len() {
            assert!(dialog.select_next(&snap));
        }
    }
}

#[test]
fn magic_vendor_magicoins_and_negative_price_guard() {
    let snap = ShopSnapshot {
        kind: ShopKind::MagicVendor,
        title_override: None,
        buy_label_override: None,
        shopper_name: Some("Mortimer".into()),
        wallet: Wallet {
            simoleons: 1_000,
            magic_coins: 20,
        },
        items: vec![
            ShopItem {
                id: "dragon_scales".into(),
                name: "Dragon Scales".into(),
                description: "Spell ingredient sold for MagiCoins.".into(),
                price: 25,
                currency: CurrencyKind::MagiCoins,
                icon: Some(sample_icon([160, 80, 220])),
                supports_try_on: false,
                owned: false,
                available: true,
            },
            ShopItem {
                id: "toadstool".into(),
                name: "Toadstools".into(),
                description: "Affordable MagiCoin ingredient.".into(),
                price: 15,
                currency: CurrencyKind::MagiCoins,
                icon: None,
                supports_try_on: false,
                owned: false,
                available: true,
            },
            ShopItem {
                id: "corrupt_price".into(),
                name: "Glitched Charm".into(),
                description: "Invalid negative price must be rejected.".into(),
                price: -5,
                currency: CurrencyKind::MagiCoins,
                icon: None,
                supports_try_on: false,
                owned: false,
                available: true,
            },
        ],
        tried_on_item_id: None,
        preview_image: None,
        preview_label: None,
        hide_wallet_and_prices: false,
        choice: None,
        body_shapes: Vec::new(),
    };

    let mut dialog = ShopPickerDialog::new();
    dialog.open();

    // 25 MC > 20 MC (even though Simoleons = 1000).
    assert!(!dialog.can_try_on_selected(&snap));
    assert!(!dialog.can_buy_selected(&snap));
    assert_eq!(
        dialog.selected_status_hint(&snap),
        Some("Insufficient MagiCoins")
    );

    // 15 MC <= 20 MC: affordable.
    assert!(dialog.select_next(&snap));
    assert!(dialog.can_buy_selected(&snap));
    assert_eq!(
        dialog.submit(&snap),
        Some(ShopCommand::Buy {
            item_id: "toadstool".into(),
            currency: CurrencyKind::MagiCoins,
            price: 15,
        })
    );

    // Negative price is never buyable.
    assert!(dialog.select_next(&snap));
    assert!(!dialog.can_buy_selected(&snap));
    assert_eq!(dialog.selected_status_hint(&snap), Some("Invalid price"));
    assert_eq!(CurrencyKind::Simoleons.format_price(120), "\u{00a7}120");
    assert_eq!(CurrencyKind::MagiCoins.format_price(25), "25 MC");
}

#[test]
fn responsive_layout_and_scrolling_cover_all_shop_kinds() {
    let mut snap = sample_boutique_snapshot();
    for i in 4..12 {
        snap.items.push(ShopItem {
            id: format!("item_{i}"),
            name: format!("Catalog Item {i}"),
            description: "Scrollable item card description.".into(),
            price: 50 * i,
            currency: CurrencyKind::Simoleons,
            icon: Some(sample_icon([80, 140, 200])),
            supports_try_on: true,
            owned: false,
            available: true,
        });
    }

    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let strings = ShopStrings::default();
    let art = Art::default();

    for kind in [
        ShopKind::DowntownBoutique(WardrobeCategory::Swimwear),
        ShopKind::DowntownBoutique(WardrobeCategory::Pajamas),
        ShopKind::DowntownBoutique(WardrobeCategory::Winterwear),
        ShopKind::VacationKiosk,
        ShopKind::PetAdoption,
        ShopKind::StudioWardrobe,
        ShopKind::MagicVendor,
    ] {
        assert!(!strings.title_for(kind).is_empty());
        assert!(!strings.buy_label_for(kind).is_empty());
    }

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
        assert!(!intersects(layout.title_bar, layout.wallet_bar));
        assert!(!intersects(layout.wallet_bar, layout.list_area));
        assert!(!intersects(layout.list_area, layout.detail_bar));
        assert!(!intersects(layout.detail_bar, layout.buy_btn));
        assert!(!intersects(layout.try_on_btn, layout.buy_btn));
        assert!(!intersects(layout.buy_btn, layout.cancel_btn));

        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        dialog.draw(&mut frame, &snap, &strings, &art);
        assert!(frame.rgb().iter().any(|&px| px != [0, 0, 0]));
    }

    assert!(dialog.scroll(2, [800, 600], &snap));
    assert_eq!(dialog.scroll_offset(), 2);
}

#[test]
fn try_on_preview_pane_renders_image_rotate_buttons_and_fallback() {
    let mut snap = sample_boutique_snapshot();
    snap.tried_on_item_id = Some("formal_01".into());
    snap.preview_image = Some(sample_icon([220, 180, 140]));
    snap.preview_label = Some("Evening Gown".into());

    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let size = [800, 600];
    let layout = dialog.layout(size, &snap);
    let pane = layout.preview_pane.expect("preview pane laid out");
    let left = layout.rotate_left_btn.expect("rotate left button");
    let right = layout.rotate_right_btn.expect("rotate right button");
    assert!(!intersects(layout.list_area, pane));
    assert!(!intersects(left, right));

    let (lx, ly) = center(left);
    dialog.pointer_down(size, &snap, lx, ly);
    assert_eq!(
        dialog.pointer_up(size, &snap, lx, ly),
        Some(ShopCommand::RotatePreview(-45))
    );

    let (rx, ry) = center(right);
    dialog.pointer_down(size, &snap, rx, ry);
    assert_eq!(
        dialog.pointer_up(size, &snap, rx, ry),
        Some(ShopCommand::RotatePreview(45))
    );

    // Fallback when preview_image is unavailable disables rotate commands cleanly.
    snap.preview_image = None;
    snap.preview_label = Some("Preview unavailable".into());
    dialog.pointer_down(size, &snap, rx, ry);
    assert_eq!(dialog.pointer_up(size, &snap, rx, ry), None);
    let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
    dialog.draw(&mut frame, &snap, &ShopStrings::default(), &Art::default());
    assert!(frame.rgb().iter().any(|&px| px != [0, 0, 0]));
}

#[test]
fn complete_collection_live_install_loads_uitext_and_renders_shop_picker() {
    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, 1).expect("load shop art");
    assert!(
        art.missing.is_empty(),
        "missing native 1.91 shop resources: {:?}",
        art.missing
    );

    let strings = ShopStrings::load(&install, 1).expect("load UIText.iff shop strings");
    assert!(!strings.buy_clothing.is_empty());
    assert!(!strings.buy_pet.is_empty());
    assert!(!strings.cancel.is_empty());
    assert!(!strings.title_formalwear.is_empty());
    assert!(!strings.title_swimwear.is_empty());
    assert!(!strings.title_pajamas.is_empty());
    assert!(!strings.title_pet.is_empty());
    // Each choice dialog is titled from its own table: `STR# 220[4]`,
    // `STR# 221[4]` and the Charm's `STR# 273[4]` (`0x536c26`).
    assert_eq!(
        strings.choice_title(ChoiceDialog::Clothing),
        "Please Select Outfit"
    );
    assert_eq!(
        strings.choice_title(ChoiceDialog::Charm),
        "Choose the New You"
    );
    // The English defaults match those entries, so a localized table pins
    // which resource each title comes from.
    let german = ShopStrings::load(&install, 4).expect("load German UIText.iff strings");
    assert_eq!(
        german.choice_title(ChoiceDialog::Charm),
        "Wähle Dein neues Ich"
    );
    assert_eq!(
        german.choice_title(ChoiceDialog::Clothing),
        "Bitte ein Outfit auswählen."
    );

    let snap = sample_boutique_snapshot();
    let mut dialog = ShopPickerDialog::new();
    dialog.open();
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    dialog.draw(&mut frame, &snap, &strings, &art);

    let non_black = frame.rgb().iter().filter(|&&px| px != [0, 0, 0]).count();
    assert!(
        non_black > 25_000,
        "expected rich 1.91 9-slice shop dialog rendering, got {non_black} non-black pixels"
    );
}
