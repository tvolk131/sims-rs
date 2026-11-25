//! Headless integration tests for the Complete Collection UI Polish & Fast Save
//! Manager lane (`game/polish-tryon-books-nb-buttons-and-fast-saves`):
//! 1. Fast zero-decompress Save Manager listing via `AutosaveStore::list_saves_metadata`
//!    (plus corruption-handling parity and full `Saved::decode` validation on load).
//! 2. Live 3D Sim preview rendering & rotation for Wardrobe / Charm "Try On"
//!    (`ShopCommand::TryOn`, `ShopCommand::RotatePreview`, graceful asset-missing fallback).
//! 3. Native paginated Spellbook (`Dialog` kind 12) & Cookbook (`Dialog` kind 15)
//!    reference modal (`ReferenceBookDialog`) loaded from `GameData/magic.iff` with
//!    held-ingredient/token highlighting.
//! 4. Neighborhood top-bar secondary actions: `ToggleRezone` (vacant + occupied lot
//!    confirmation & eviction), `ImportFamily` (`.FAM` scan + picker + REDB import),
//!    `OpenCredits` (paginated `Credits.iff` modal), and `OpenWeb` (offline notice).
//! 5. Track 1 UX polish: chained Save & Quit failure notice
//!    (`OptionsDialog::Notice` -> `OptionsDialog::ConfirmSaveBeforeQuit`) and exact
//!    caret `next_blink_us` wakeups on `CheatConsole` and `PhotoAlbumDialog`.

use sims_game::autosave_worker::{
    AutosaveWorker, build_save_manager_snapshot, format_save_label, validate_save_payload,
    validate_save_payload_cached,
};
use sims_game::game::goth_house;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind};
use sims_game::{Frames, Game, Options};
use sims_render::neighborhood::{LotStatus, LotZoning};
use sims_sim::autosave::{SaveSlot, SaveTarget};
use sims_ui::album::{AlbumEntry, AlbumSnapshot, PhotoAlbumDialog, PhotoId};
use sims_ui::art::Art;
use sims_ui::cheat_console::CheatConsole;
use sims_ui::command::{Command, Mode, OptionsAction};
use sims_ui::dialog::CARET_BLINK_US;
use sims_ui::neighborhood::{BarButton, BarLayout, NeighborhoodMode};
use sims_ui::saves::SaveHealth;
use sims_ui::shop::{CurrencyKind, ShopCommand, ShopItem, ShopKind, WardrobeCategory};
use std::path::PathBuf;
use std::time::Instant;

fn install() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

fn temp_dir_and_redb(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sims-game-polish-lane-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let redb = dir.join("saves.redb");
    (dir, redb)
}

fn find_lot_hit_point(shell: &NeighborhoodShell, lot_id: u16) -> Option<[i32; 2]> {
    let viewport = [800u32, 600u32];
    let map = shell.map();
    let lot = map.lot(lot_id)?;
    let [left, top, right, bottom] = map.lot_screen_rect(lot, viewport);
    let y_start = top.max(sims_ui::neighborhood::BAR_HEIGHT + 2);
    for y in (y_start..bottom.min(600)).step_by(2) {
        for x in (left.max(0)..right.min(800)).step_by(2) {
            if map.hit_test(viewport, [x, y]) == Some(lot_id) {
                return Some([x, y]);
            }
        }
    }
    None
}

fn find_command_point(game: &Game, target: &Command) -> [i32; 2] {
    let rect = game
        .panel()
        .iter()
        .find(|i| i.command.as_ref() == Some(target))
        .unwrap_or_else(|| panic!("missing HUD command {target:?}"))
        .rect;
    [rect.x + rect.width / 2, rect.y + rect.height / 2]
}

fn click_top_bar_button(shell: &mut NeighborhoodShell, button: BarButton) {
    let layout = BarLayout::compute(800, 600, shell.neighborhood_state(), &Art::default());
    let rect = layout
        .button(button)
        .unwrap_or_else(|| panic!("missing bar button {button:?}"))
        .hit_rect;
    let pt = [rect.x + rect.width / 2, rect.y + rect.height / 2];
    shell.press(pt);
    shell.release(pt);
}

fn load_ud1(shell: &NeighborhoodShell) -> sims_sim::neighborhood_store::StoredNeighborhood {
    shell
        .store()
        .load_neighborhood("UserData")
        .expect("load UserData")
        .expect("UserData exists")
}

#[test]
#[allow(clippy::disallowed_methods)]
fn fast_save_manager_listing_beats_full_decode_and_preserves_corruption_checks() {
    let Some(root) = install() else {
        return;
    };
    let (_tmp, saves) = temp_dir_and_redb("fast-saves");
    let mut shell =
        NeighborhoodShell::open(root.clone(), saves, 1, true, vec![3], Frames::default())
            .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    shell.enter_lot(5).expect("enter Goth house");
    let instance_id = load_ud1(&shell).instance_id;

    let worker = AutosaveWorker::spawn(shell.store().autosaves());
    let saved_manual = shell
        .game()
        .expect("active game")
        .lot()
        .save()
        .expect("save manual");
    worker
        .enqueue_saved_lot(
            "UserData",
            instance_id,
            5,
            SaveTarget::Manual,
            10,
            format_save_label(Some("Goth"), 1, 8, 10),
            saved_manual,
        )
        .expect("enqueue manual");
    for seq in 1..=6u64 {
        shell.game_mut().expect("active game").run(2);
        let saved = shell
            .game()
            .expect("active game")
            .lot()
            .save()
            .expect("save autosave");
        worker
            .enqueue_saved_lot(
                "UserData",
                instance_id,
                5,
                SaveTarget::Autosave,
                10 + seq * 2,
                format_save_label(Some("Goth"), 1, 8, 10 + seq as u8),
                saved,
            )
            .expect("enqueue autosave");
    }
    worker.flush();
    let completions = worker.poll_completions();
    assert_eq!(completions.len(), 7);
    assert!(completions.iter().all(|c| c.result.is_ok()));
    drop(worker);

    let autosaves = shell.store().autosaves();
    // Baseline full decode path (`list_saves_validated` with `validate_save_payload`).
    let t_full = Instant::now();
    let full_entries = autosaves
        .list_saves_validated("UserData", 5, validate_save_payload)
        .expect("full validated listing");
    let full_ms = t_full.elapsed().as_secs_f64() * 1000.0;

    // Fast metadata-only Save Manager listing (`build_save_manager_snapshot` -> `list_saves_metadata`).
    let t_fast = Instant::now();
    let snap = build_save_manager_snapshot(&autosaves, "UserData", 1, 5, "5 Sim Lane", None);
    let fast_ms = t_fast.elapsed().as_secs_f64() * 1000.0;

    // Also measure warm `list_saves_validated(..., validate_save_payload_cached)`.
    let _ = autosaves.list_saves_validated("UserData", 5, validate_save_payload_cached);
    let t_cached = Instant::now();
    let _ = autosaves.list_saves_validated("UserData", 5, validate_save_payload_cached);
    let cached_ms = t_cached.elapsed().as_secs_f64() * 1000.0;

    eprintln!(
        "BENCH SaveManager (6+ slots): full_decode={full_ms:.2} ms, cached_decompress={cached_ms:.2} ms, metadata_only={fast_ms:.2} ms"
    );

    assert_eq!(snap.slots.len(), full_entries.len());
    assert!(snap.slots.len() >= 6);
    assert!(snap.slots.iter().all(|s| s.health == SaveHealth::Intact));
    assert!(
        fast_ms < full_ms,
        "metadata-only listing ({fast_ms:.2} ms) should beat full decode ({full_ms:.2} ms)"
    );

    // Full `Saved::decode` validation still runs cleanly on slot load.
    let loaded = autosaves
        .load_save_validated("UserData", 5, SaveSlot::Manual, validate_save_payload)
        .expect("load_save_validated manual slot succeeds");
    assert_eq!(loaded.metadata.sim_tick, 10);

    // Sol P2 regression: "Newest Intact" fallback must walk candidates in
    // descending tick/timestamp order rather than jumping straight to an older
    // manual save when the highest-tick autosave passes metadata check but fails
    // full `Saved::decode`!
    let (_tmp2, saves2) = temp_dir_and_redb("fast-saves-fallback");
    let mut shell2 = NeighborhoodShell::open(root, saves2, 1, true, vec![3], Frames::default())
        .expect("open shell2");
    shell2.resize([800, 600]);
    shell2.enter_lot(5).expect("enter lot 5");
    let inst2 = load_ud1(&shell2).instance_id;
    let autosaves2 = shell2.store().autosaves();
    // 1) Valid manual save at tick 100.
    let saved_100 = shell2.game().unwrap().lot().save().expect("save 100");
    let manual_job = sims_sim::autosave::SaveJob::manual_for_instance(
        "UserData",
        inst2,
        5,
        100,
        sims_sim::autosave::SavePayload::saved_lot(saved_100.encode()),
    )
    .with_saved_at_unix_secs(1_000)
    .with_label(format_save_label(Some("Goth"), 1, 8, 10));
    autosaves2
        .commit_job_validated(manual_job, validate_save_payload)
        .expect("commit manual 100");
    // 2) Valid autosave at tick 200.
    shell2.game_mut().unwrap().run(4);
    let expected_tick_200 = shell2.game().unwrap().lot().ticks();
    let saved_200 = shell2.game().unwrap().lot().save().expect("save 200");
    let auto200_job = sims_sim::autosave::SaveJob::autosave_for_instance(
        "UserData",
        inst2,
        5,
        200,
        sims_sim::autosave::SavePayload::saved_lot(saved_200.encode()),
    )
    .with_saved_at_unix_secs(2_000)
    .with_label(format_save_label(Some("Goth"), 1, 8, 20));
    autosaves2
        .commit_job_validated(auto200_job, validate_save_payload)
        .expect("commit auto 200");
    // 3) Corrupt-body autosave at tick 300 (valid envelope/metadata, bad Saved bytes).
    let corrupt300_job = sims_sim::autosave::SaveJob::autosave_for_instance(
        "UserData",
        inst2,
        5,
        300,
        sims_sim::autosave::SavePayload::saved_lot(b"corrupt-saved-body".to_vec()),
    )
    .with_saved_at_unix_secs(3_000)
    .with_label(format_save_label(Some("Goth"), 1, 8, 30));
    let meta300 = autosaves2
        .commit_job(corrupt300_job)
        .expect("commit corrupt-body 300");
    let corrupt300_id = sims_game::autosave_worker::slot_id_for(meta300.slot);

    // Advance lot past tick 200, then trigger FallbackToNewestIntact pointing at tick 300.
    shell2.game_mut().unwrap().run(6);
    assert!(shell2.open_save_manager(Some("Simulated load failure".into())));
    let _ = shell2.handle_save_manager_command(
        sims_ui::saves::SaveManagerCommand::FallbackToNewestIntact {
            slot_id: corrupt300_id,
        },
    );
    assert!(
        !shell2.save_manager_open(),
        "fallback must recover from tick-200 autosave and close Save Manager"
    );
    assert_eq!(
        shell2.game().unwrap().lot().ticks(),
        expected_tick_200,
        "Newest Intact fallback must restore tick-200 autosave, not older tick-100 manual save"
    );
}

#[test]
fn wardrobe_try_on_renders_live_3d_sim_preview_rotates_and_falls_back_gracefully() {
    let Some(root) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: root.clone(),
        house: goth_house(&root),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    game.run(5);

    // Open interactive boutique shop picker with try-on items.
    let opened = game.open_shop_with_items(
        ShopKind::DowntownBoutique(WardrobeCategory::Formal),
        vec![
            ShopItem {
                id: "outfit_formal_0".into(),
                name: "Formal Outfit A".into(),
                description: "Evening attire".into(),
                price: 150,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: false,
                available: true,
            },
            ShopItem {
                id: "outfit_formal_1".into(),
                name: "Formal Outfit B".into(),
                description: "Gala attire".into(),
                price: 200,
                currency: CurrencyKind::Simoleons,
                icon: None,
                supports_try_on: true,
                owned: false,
                available: true,
            },
        ],
    );
    assert!(opened, "shop picker opens on Goth lot");

    // Click Try On: renders live 3D Sim preview via `sims_people::caf_preview`.
    let out = game.shop_try_on_selected();
    assert!(out.redraw);
    let (img0_w, img0_h, img0_rgba) = {
        let snap = game.shop_snapshot().expect("shop snapshot");
        assert_eq!(snap.tried_on_item_id.as_deref(), Some("outfit_formal_0"));
        let img = snap
            .preview_image
            .as_ref()
            .expect("Try On must render a live 3D Sim preview image");
        assert!(img.width >= 64 && img.height >= 120);
        let visible_px = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] > 0)
            .count();
        assert!(
            visible_px > 400,
            "expected substantial 3D Sim pixels in Try-On preview, got {visible_px}"
        );
        (img.width, img.height, img.rgba.clone())
    };
    assert_eq!(game.shop_preview_yaw_deg(), 0);

    // Render dialog with preview pane visible and verify layout exposes rotate buttons.
    game.render();
    let snap_for_layout = game.shop_snapshot().expect("snapshot").clone();
    let layout = game.shop_dialog().layout([800, 600], &snap_for_layout);
    assert!(layout.preview_pane.is_some());
    assert!(layout.rotate_left_btn.is_some());
    assert!(layout.rotate_right_btn.is_some());

    // Rotate preview by +45° and verify yaw & rendered 3D silhouette update.
    let rot_out = game.shop_rotate_preview(45);
    assert!(rot_out.redraw);
    assert_eq!(game.shop_preview_yaw_deg(), 45);
    let img1_rgba = game
        .shop_snapshot()
        .and_then(|s| s.preview_image.as_ref())
        .expect("rotated preview image")
        .rgba
        .clone();
    assert_eq!(img1_rgba.len(), (img0_w * img0_h * 4) as usize);
    assert_ne!(
        img0_rgba, img1_rgba,
        "rotating the 3D Try-On Sim preview by 45 degrees must re-render from the new yaw"
    );

    // Large rotation deltas (e.g., i16::MAX) must normalize cleanly without overflow.
    let big_rot = game.shop_rotate_preview(i16::MAX);
    assert!(big_rot.redraw);
    assert!((0..360).contains(&game.shop_preview_yaw_deg()));

    // Graceful fallback: trying on an item with no backing asset shows unavailable
    // state (`preview_image.is_none()`) rather than an unrelated everyday body.
    let fallback_out = game.execute_shop_command(ShopCommand::TryOn {
        item_id: "missing_asset_id".into(),
    });
    assert!(fallback_out.redraw);
    let snap_fb = game.shop_snapshot().expect("snapshot after try-on");
    assert_eq!(
        snap_fb.tried_on_item_id.as_deref(),
        Some("missing_asset_id")
    );
    assert!(
        snap_fb.preview_image.is_none(),
        "unbound/missing item must never render an unrelated everyday body"
    );
    assert_eq!(
        snap_fb.preview_label.as_deref(),
        Some("Preview unavailable")
    );
    game.render();
}

#[test]
fn paginated_spellbook_and_cookbook_load_installed_catalog_and_highlight_held_items() {
    let Some(root) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: root.clone(),
        house: goth_house(&root),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    game.run(5);

    let caller = game.lot().world().selected().expect("selected Sim");
    let neighbor_id = game.lot().world().objects[&caller]
        .person
        .as_ref()
        .and_then(|p| p.data.get(31).copied())
        .expect("caller neighbor id");

    // Seed the real Adult Magic Wand token (`0x1fec_6005`, kind 7) on the caller
    // so both the Spellbook summary strip highlights it AND `SpellBook.iff`'s
    // live `0x1009` ("Look Up Spells") BHAV opens Dialog kind 12 cleanly.
    let saved = game.lot().save().expect("save lot");
    let orig_world_body = saved.world().encode();
    let encoded = saved.encode();
    let tail = encoded[8..]
        .strip_prefix(orig_world_body.as_slice())
        .expect("saved world prefix");
    let mut world = saved.world().clone();
    sims_vm::travel::set_neighbor_token_count(&mut world, neighbor_id, 7, 0x1fec_6005, 1);
    let new_world_body = world.encode();
    let mut rebuilt = Vec::with_capacity(8 + new_world_body.len() + tail.len());
    rebuilt.extend_from_slice(&encoded[..8]);
    rebuilt.extend_from_slice(&new_world_body);
    rebuilt.extend_from_slice(tail);
    let patched = sims_vm::boot::save::Saved::decode(&rebuilt).expect("decode patched saved");
    game.restore(patched).expect("restore seeded world");

    // Open paginated Spellbook (`is_cookbook = false`).
    game.open_reference_book(Some(caller), false);
    assert!(game.book_open());
    assert!(game.has_dialog());
    {
        let snap = game.book_snapshot().expect("spellbook snapshot");
        assert!(
            snap.pages.len() >= 15,
            "Spellbook must load full installed spell/charm catalog, got {} pages",
            snap.pages.len()
        );
        assert!(
            !snap.held_summary.is_empty(),
            "Spellbook must highlight caller's held magic inventory tokens"
        );
        assert!(snap.held_summary.iter().all(|b| b.owned));
    }
    assert_eq!(game.book_dialog().page(), 0);
    assert!(!game.book_prev_page());
    assert!(game.book_next_page());
    assert_eq!(game.book_dialog().page(), 1);
    assert!(game.book_wheel(1.0));
    assert_eq!(game.book_dialog().page(), 0);
    game.render();

    // Close via Escape / book_close.
    let closed = game.dialog_confirm_key(DialogKey::Escape);
    assert!(closed.redraw);
    assert!(!game.book_open());

    // Open paginated Cookbook (`is_cookbook = true`).
    game.open_reference_book(Some(caller), true);
    assert!(game.book_open());
    {
        let snap = game.book_snapshot().expect("cookbook snapshot");
        assert!(
            snap.pages.len() >= 6,
            "Cookbook must load full installed recipe catalog, got {} pages",
            snap.pages.len()
        );
    }
    assert!(game.book_next_page());
    assert_eq!(game.book_dialog().page(), 1);
    game.render();
    // Close via pointer click on the Close button (Sol P1 regression check).
    let close_pt = {
        let snap = game.book_snapshot().expect("cookbook snapshot").clone();
        let layout = game.book_dialog().layout([800, 600], &snap);
        [
            layout.close_btn.x + layout.close_btn.width / 2,
            layout.close_btn.y + layout.close_btn.height / 2,
        ]
    };
    let _ = game.press(close_pt);
    let closed_click = game.release(close_pt);
    assert!(closed_click.redraw);
    assert!(!game.book_open());

    // Live VM SpecialPanel pointer-driven Close test for BOTH kind 12 (SpellBook)
    // and kind 15 (CookBook): clicking the modal's Close button must dismiss the
    // modal AND deliver `SpecialPanelResponse::Dismissed` so the lot never stalls!
    for (label, guid_candidates, expected_is_cookbook) in [
        (
            "SpellBook (kind 12)",
            &[0x8ec5_750du32, 0xce36_15a5][..],
            false,
        ),
        ("CookBook (kind 15)", &[0x2a3d_8714u32][..], true),
    ] {
        let sim_id = game.lot().world().selected().expect("selected Sim");
        let sim_pos = game.lot().world().objects[&sim_id].position;
        let lot = game.lot_mut();
        lot.set_mode(sims_vm::PanelMode::Buy).expect("Buy mode");
        let mut placed_obj = None;
        'guid_loop: for &guid in guid_candidates {
            let Ok(Some(mut cursor)) = lot.buy_start(guid, 0, 0) else {
                continue;
            };
            let held = cursor.held;
            let bx = sim_pos[0] / 16;
            let by = sim_pos[1] / 16;
            for r in 1..=12i32 {
                for dy in -r..=r {
                    for dx in -r..=r {
                        let tx = bx + dx;
                        let ty = by + dy;
                        if !(4..=60).contains(&tx) || !(4..=60).contains(&ty) {
                            continue;
                        }
                        if lot
                            .buy_move(&mut cursor, [tx * 16 + 8, ty * 16 + 8], 1)
                            .unwrap_or(false)
                            && cursor.valid
                            && lot.buy_commit(&cursor).is_ok()
                        {
                            placed_obj = Some(held);
                            break 'guid_loop;
                        }
                    }
                }
            }
            let _ = lot.buy_cancel(&cursor);
        }
        lot.set_mode(sims_vm::PanelMode::Live).expect("Live mode");
        let obj_id = placed_obj.unwrap_or_else(|| panic!("must place {label} on Goth lot"));
        lot.select(sim_id).expect("select Sim");
        let menu = lot
            .pie_menu(sim_id, obj_id)
            .unwrap_or_else(|e| panic!("pie_menu failed for {label}: {e}"));
        let entry = menu
            .entries
            .iter()
            .find(|e| e.action.action == 0x1009)
            .or_else(|| menu.entries.first())
            .unwrap_or_else(|| panic!("expected Read (0x1009) interaction on {label}"));
        let picked_action = entry.action.action;
        let menu_actions: Vec<(u16, Option<String>)> = menu
            .entries
            .iter()
            .map(|e| (e.action.action, e.name.clone()))
            .collect();
        lot.pick(sim_id, entry.action.clone())
            .unwrap_or_else(|e| panic!("pick failed for {label}: {e}"));
        let mut opened = false;
        let mut seen_reqs = Vec::new();
        for _ in 0..800 {
            game.run(1);
            if let Some(asking) = game.asking() {
                let caller = asking.shown.object;
                game.answer(caller, 0);
                continue;
            }
            if let Some(req) = game.active_special_panel_request() {
                seen_reqs.push(format!("{req:?}"));
                let matches_kind = if expected_is_cookbook {
                    matches!(req, sims_vm::SpecialPanelRequest::CookBook { .. })
                } else {
                    matches!(req, sims_vm::SpecialPanelRequest::SpellBook { .. })
                };
                if matches_kind {
                    opened = true;
                    break;
                }
            }
        }
        assert!(
            opened,
            "{label} interaction must open VM special panel (picked={picked_action:#x}, menu={menu_actions:?}, seen={seen_reqs:?}, asking_sp={:?})",
            game.asking_special_panel().map(|a| &a.shown.request)
        );
        assert!(game.book_open(), "{label} must open ReferenceBookDialog");
        game.render();
        let close_pt = {
            let snap = game.book_snapshot().expect("snapshot").clone();
            let layout = game.book_dialog().layout([800, 600], &snap);
            [
                layout.close_btn.x + layout.close_btn.width / 2,
                layout.close_btn.y + layout.close_btn.height / 2,
            ]
        };
        let _ = game.press(close_pt);
        let out = game.release(close_pt);
        assert!(out.redraw, "{label} Close click must redraw");
        assert!(!game.book_open(), "{label} Close click must close modal");
        assert!(
            game.asking_special_panel().is_none(),
            "{label} Close click must answer the VM so the lot does not stall"
        );
        assert!(
            game.run(2) >= 1,
            "{label} lot must resume stepping after Close click"
        );
    }
}

#[test]
fn neighborhood_rezone_import_family_credits_and_web_actions_work_end_to_end() {
    let Some(root) = install() else {
        return;
    };
    let (tmp_dir, saves) = temp_dir_and_redb("nb-actions");
    let mut shell =
        NeighborhoodShell::open(root.clone(), saves, 1, true, vec![3], Frames::default())
            .expect("open NeighborhoodShell");
    shell.resize([800, 600]);
    shell.render();
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);

    // 1. ToggleRezone: activate Rezone mode, verify hovered lot highlight activates,
    //    rezone vacant lot 2 (Residential -> Community) and occupied lot 5 (Goth -> Community,
    //    confirming eviction back to Family Bin).
    click_top_bar_button(&mut shell, BarButton::Rezone);
    assert_eq!(shell.neighborhood_state().mode, NeighborhoodMode::Rezone);
    assert!(shell.should_highlight_hovered_lot());

    // Rezone vacant lot 2 via map click -> confirmation dialog -> Enter (Yes).
    let lot2_pt = find_lot_hit_point(&shell, 2).expect("lot 2 hit point");
    shell.press(lot2_pt);
    shell.release(lot2_pt);
    assert!(
        shell.has_dialog(),
        "clicking lot 2 in Rezone mode opens confirmation"
    );
    shell.key_dialog(DialogKey::Enter);
    assert_eq!(load_ud1(&shell).zoning.get(&2), Some(&2));
    assert_eq!(
        shell.map().lot(2).expect("lot 2").zoning,
        LotZoning::Community
    );
    assert!(matches!(
        shell.map().lot(2).expect("lot 2").status,
        LotStatus::Community { .. }
    ));

    // Rezone occupied lot 5 (Goth household): must show confirmation & evict Goth to Family Bin!
    let goth_fam_id = load_ud1(&shell)
        .family_in(5)
        .expect("Goth occupies lot 5 initially")
        .0;
    // Rezone confirmation fence regression (Sol P1): if the neighborhood revision
    // changes between opening the Rezone confirmation and pressing Enter, the
    // stale confirmation must be rejected cleanly without evicting or rezoning!
    let lot5_pt = find_lot_hit_point(&shell, 5).expect("lot 5 hit point");
    shell.press(lot5_pt);
    shell.release(lot5_pt);
    assert!(
        shell.has_dialog(),
        "rezoning occupied lot 5 must open confirmation dialog"
    );
    {
        let mut bumped = load_ud1(&shell);
        let inst = bumped.instance_id;
        let rev = bumped.revision;
        bumped.revision = rev + 1;
        shell
            .store()
            .save_neighborhood_checked("UserData", inst, rev, &bumped)
            .expect("bump revision")
            .expect("revision bump succeeds");
    }
    shell.key_dialog(DialogKey::Enter);
    assert!(
        shell.has_dialog(),
        "stale Rezone confirmation must open an error notice instead of committing"
    );
    shell.key_dialog(DialogKey::Enter);
    let after_stale = load_ud1(&shell);
    assert_ne!(
        after_stale.zoning.get(&5),
        Some(&2),
        "stale confirmation must not rezone lot 5"
    );
    assert!(
        after_stale.family_in(5).is_some(),
        "stale confirmation must not evict lot 5"
    );

    // Sol P2 active-trip regression: rezoning either endpoint (home or secondary
    // destination) of an active Old Town trip must be refused cleanly!
    {
        let mut with_trip = load_ud1(&shell);
        let inst = with_trip.instance_id;
        let rev = with_trip.revision;
        let fam = with_trip
            .families
            .get_mut(&goth_fam_id)
            .expect("Goth family");
        if fam.version < 8 {
            fam.version = 9;
        }
        if fam.tail.len() < 4 {
            fam.tail.resize(16, 0);
        }
        fam.tail[0..4].copy_from_slice(&2i32.to_le_bytes());
        with_trip.revision = rev + 1;
        shell
            .store()
            .save_neighborhood_checked("UserData", inst, rev, &with_trip)
            .expect("inject active trip")
            .expect("trip commit succeeds");
        shell.refresh_from_store().expect("refresh with trip");
    }
    assert!(
        shell.execute_rezone(5, 2).is_err(),
        "rezoning home lot 5 during an active trip must be refused"
    );
    assert!(
        shell.execute_rezone(2, 1).is_err(),
        "rezoning destination lot 2 during an active trip must be refused"
    );
    // Clear the synthetic active trip before testing atomic occupied rezone.
    {
        let mut cleared = load_ud1(&shell);
        let inst = cleared.instance_id;
        let rev = cleared.revision;
        let fam = cleared.families.get_mut(&goth_fam_id).expect("Goth family");
        fam.tail[0..4].fill(0);
        cleared.revision = rev + 1;
        shell
            .store()
            .save_neighborhood_checked("UserData", inst, rev, &cleared)
            .expect("clear synthetic trip")
            .expect("clear trip succeeds");
        shell.refresh_from_store().expect("refresh cleared trip");
    }

    // Rezone occupied lot 5 (Goth household): must show confirmation & atomically
    // evict Goth to Family Bin + commit Community zoning under the confirmation fence!
    shell.press(lot5_pt);
    shell.release(lot5_pt);
    assert!(
        shell.has_dialog(),
        "rezoning occupied lot 5 must open confirmation dialog"
    );
    shell.key_dialog(DialogKey::Enter);
    let after_rezone = load_ud1(&shell);
    assert_eq!(after_rezone.zoning.get(&5), Some(&2));
    assert!(
        after_rezone.family_in(5).is_none(),
        "rezoning occupied lot 5 must evict the household"
    );
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(goth_fam_id)),
        "evicted household must appear in the Family Bin"
    );

    // Exit Rezone mode via Rezone button toggle.
    click_top_bar_button(&mut shell, BarButton::Rezone);
    assert_eq!(shell.neighborhood_state().mode, NeighborhoodMode::Normal);

    // 2. ImportFamily: export a Family Bin household into a temp Import dir, register it,
    //    open the native Import picker modal, and import cleanly into REDB Family Bin!
    let import_dir = tmp_dir.join("Import");
    std::fs::create_dir_all(&import_dir).expect("create temp Import dir");
    let chars = shell
        .store()
        .load_character_files("UserData")
        .expect("load characters");
    let fam_bytes = sims_vm::caf::export_family_fam(
        &after_rezone,
        &chars,
        goth_fam_id,
        "ImportedTest",
        1,
        true,
    )
    .expect("export test .FAM");
    std::fs::write(import_dir.join("ImportedTest_99.FAM"), &fam_bytes).expect("write .FAM");
    shell.add_import_search_dir(import_dir);
    assert!(shell.has_importable_family());

    let bin_before = shell.family_bin_entries().len();
    click_top_bar_button(&mut shell, BarButton::ImportFamily);
    assert!(
        shell.import_picker_open(),
        "ImportFamily opens native picker modal"
    );
    let imp_snap = shell.import_picker_snapshot().expect("import snapshot");
    assert!(
        imp_snap.items.iter().any(|i| i.name == "ImportedTest"),
        "import picker must list our .FAM family: {:?}",
        imp_snap.items.iter().map(|i| &i.name).collect::<Vec<_>>()
    );
    shell.render();
    // Press Enter to import the top selected `.FAM` family (`ImportedTest`).
    shell.key_enter();
    assert!(!shell.import_picker_open());
    assert!(
        shell.has_dialog(),
        "successful import shows confirmation notice"
    );
    shell.key_dialog(DialogKey::Enter);
    assert_eq!(
        shell.family_bin_entries().len(),
        bin_before + 1,
        "imported family must be added to the Family Bin (note={:?})",
        shell.note()
    );
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.name == "ImportedTest")
    );

    // 3. OpenCredits: opens paginated Credits/About modal from `GameData/Credits.iff`.
    click_top_bar_button(&mut shell, BarButton::Credits);
    assert!(shell.credits_open(), "Credits button opens paginated modal");
    let credits_pages = shell
        .credits_snapshot()
        .expect("credits snapshot")
        .pages
        .len();
    assert!(
        credits_pages >= 5,
        "Credits.iff should produce multiple pages"
    );
    assert_eq!(shell.credits_dialog().page(), 0);
    shell.key_right();
    assert_eq!(shell.credits_dialog().page(), 1);
    shell.key_left();
    assert_eq!(shell.credits_dialog().page(), 0);
    shell.render();
    shell.escape();
    assert!(!shell.credits_open());

    // 4. OpenWeb: opens clean native offline notice modal.
    click_top_bar_button(&mut shell, BarButton::Web);
    assert!(
        shell.has_dialog(),
        "OpenWeb opens native offline notice dialog"
    );
    shell.render();
    shell.key_dialog(DialogKey::Enter);
    assert!(!shell.has_dialog());
}

#[test]
fn chained_save_quit_failure_notice_and_exact_caret_blink_deadlines() {
    // 1. Exact `next_blink_us` on `CheatConsole` and `PhotoAlbumDialog` (unit-level, no install needed).
    let mut console = CheatConsole::new();
    assert_eq!(console.next_blink_us(0), None);
    console.open();
    assert_eq!(console.next_blink_us(0), Some(CARET_BLINK_US));
    assert!(!console.sync_blink(100_000));
    assert_eq!(
        console.next_blink_us(250_000),
        Some(CARET_BLINK_US - 150_000)
    );
    assert!(console.sync_blink(100_000 + CARET_BLINK_US));
    assert_eq!(
        console.next_blink_us(100_000 + CARET_BLINK_US),
        Some(CARET_BLINK_US)
    );

    let mut album = PhotoAlbumDialog::default();
    let album_snap = AlbumSnapshot {
        family_name: "Goth".into(),
        entries: vec![AlbumEntry {
            id: PhotoId(1),
            title: "First".into(),
            caption: "Caption".into(),
            image: None,
            sim_day: Some(1),
        }],
    };
    album.open(&album_snap);
    // Open but unfocused -> zero idle wakeups!
    assert_eq!(album.next_blink_us(), None);
    assert!(!album.sync_blink(CARET_BLINK_US));
    // Focusing Title box activates exact 500 ms caret deadlines.
    let layout = album.layout(800, 600);
    let pt = [
        layout.title_box.x + 8,
        layout.title_box.y + layout.title_box.height / 2,
    ];
    album.handle_pointer_down(pt, [800, 600], &album_snap);
    let _ = album.handle_pointer_up(pt, [800, 600], &album_snap);
    assert_eq!(album.next_blink_us(), Some(CARET_BLINK_US));
    assert!(!album.sync_blink(120_000));
    assert_eq!(album.next_blink_us(), Some(CARET_BLINK_US - 120_000));
    assert!(album.sync_blink(CARET_BLINK_US - 120_000));
    assert_eq!(album.next_blink_us(), Some(CARET_BLINK_US));

    // 2. Live `Game` chained SaveCardQuit failure notice (`Notice` -> `ConfirmSaveBeforeQuit`)
    //    and exact `Game::next_wake_us` with open CheatConsole.
    let Some(root) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        install: root.clone(),
        house: goth_house(&root),
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    game.set_edit_mode(true);
    game.run(1); // mark dirty
    game.render();

    // Open Quit from edit mode -> SaveCardQuit -> choose Yes (0) -> queues ConfirmQuit + Outcome.save.
    let opt_pt = find_command_point(&game, &Command::SetMode(Mode::Options));
    game.press(opt_pt);
    game.render();
    let quit_pt = find_command_point(&game, &Command::OptionsAction(OptionsAction::Quit));
    game.press(quit_pt);
    game.release(quit_pt);
    let save_out = game.dialog_confirm_key(DialogKey::Enter);
    assert!(save_out.save);

    // Simulate host save failure with chained notice:
    // Step 1: opens OptionsDialog::Notice explaining the failure.
    game.after_save_failed_with_notice("Save Failed", "The lot was not saved: read-only disk");
    assert!(game.has_ui_dialog());
    assert_eq!(game.note(), Some("The lot was not saved: read-only disk"));
    // Step 2: dismissing Notice via Enter chains directly into ConfirmSaveBeforeQuit
    //         (which offers Yes=save+quit, No=quit without saving, Cancel).
    let dismiss = game.dialog_confirm_key(DialogKey::Enter);
    assert!(dismiss.redraw);
    assert!(!dismiss.quit);
    assert!(
        game.has_ui_dialog(),
        "dismissing chained Save Failed notice must open ConfirmSaveBeforeQuit"
    );
    // Choosing button 1 (No = quit without saving) on ConfirmSaveBeforeQuit emits quit=true, save=false!
    let quit_no_save = game.dialog_confirm_key(DialogKey::Char('n'));
    assert!(quit_no_save.quit && !quit_no_save.save);

    // Exact cheat-console caret wakeup on paused lot (no 100 ms poll!).
    game.set_edit_mode(false);
    game.toggle_pause();
    game.toggle_cheat_console();
    let wake = game
        .next_wake_us(false, 16_667)
        .expect("open cheat console schedules caret blink");
    assert!(
        wake >= 200_000 || wake == 50_000,
        "exact caret blink should schedule ~500 ms (or 50 ms plumb bob spin), got {wake} us"
    );
}
