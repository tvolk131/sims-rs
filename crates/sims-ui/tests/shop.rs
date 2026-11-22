use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_formats::bitmap::Image;
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::shop::{
    CurrencyKind, ShopCommand, ShopItem, ShopKind, ShopPickerDialog, ShopSnapshot, ShopStrings,
    Wallet, WardrobeCategory, load_art,
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
fn magic_vendor_magicoins_and_negative_price_guard() {
    let snap = ShopSnapshot {
        kind: ShopKind::MagicVendor,
        title_override: None,
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
