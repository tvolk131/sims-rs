//! End-to-end headless integration tests for the playable Neighborhood ↔
//! Family Bin ↔ Lot Save & Leave ↔ Evict/Bulldoze ↔ Streaming Mode Music loop.
//!
//! Requires `SIMS_INSTALL`; skipped cleanly when unset. Never mutates any
//! install file — all state persists inside a temporary REDB file.

use sims_audio::SoundEvent;
use sims_audio::music::MusicMode;
use sims_audio::player::RATE;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind};
use sims_game::sound::{self, Input, Mixer};
use sims_render::neighborhood::LotStatus;
use sims_ui::command::{Command, Mode, OptionsAction};
use sims_ui::neighborhood::{BarButton, BarLayout, NeighborhoodMode};
use sims_vm::boot::Frames;
use std::path::{Path, PathBuf};

fn temp_redb(tag: &str) -> (PathBuf, PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("sims-game-nbhd-shell-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let redb = dir.join("saves.redb");
    (dir, redb)
}

fn find_lot_hit_point(shell: &NeighborhoodShell, lot_id: u16) -> Option<[i32; 2]> {
    let viewport = [800u32, 600u32];
    let lot = shell.map().lot(lot_id)?;
    let [left, top, right, bottom] = shell.map().lot_screen_rect(lot, viewport);
    let y_start = top.max(sims_ui::neighborhood::BAR_HEIGHT);
    for y in (y_start..bottom.min(600)).step_by(2) {
        for x in (left.max(0)..right.min(800)).step_by(2) {
            if shell.map().hit_test(viewport, [x, y]) == Some(lot_id) {
                return Some([x, y]);
            }
        }
    }
    None
}

fn find_bar_button_point(shell: &NeighborhoodShell, button: BarButton) -> [i32; 2] {
    let art = sims_ui::art::Art::default();
    let layout = BarLayout::compute(800, 600, shell.neighborhood_state(), &art);
    let rect = layout
        .button(button)
        .expect("button in 800x600 bar")
        .hit_rect;
    [rect.x + rect.width / 2, rect.y + rect.height / 2]
}

fn find_game_command_point(game: &sims_game::Game, target: &Command) -> [i32; 2] {
    let item = game
        .panel()
        .iter()
        .find(|i| i.command.as_ref() == Some(target))
        .unwrap_or_else(|| panic!("HUD item {target:?} not found"));
    [
        item.rect.x + item.rect.width / 2,
        item.rect.y + item.rect.height / 2,
    ]
}

#[test]
fn end_to_end_neighborhood_lot_save_move_in_evict_bulldoze_loop() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping neighborhood shell integration test");
        return;
    };
    let install = PathBuf::from(root);
    let (tmp_dir, saves_path) = temp_redb("e2e");

    // 1. Open Neighborhood Shell on slot 1 (imports UserData into REDB).
    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves_path.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("NeighborhoodShell opens");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);
    assert_eq!(shell.neighborhood_slot(), 1);
    assert_eq!(shell.neighborhood_id(), "UserData");

    // Initial audio queue sets volumes and Neighborhood mode music.
    let initial_audio = shell.take_audio_inputs();
    assert!(initial_audio.contains(&Input::MusicMode(MusicMode::Neighborhood)));

    // Render Neighborhood map and verify non-blank output.
    shell.resize([800, 600]);
    shell.render();
    assert!(shell.canvas().color.iter().any(|&px| px != [0, 0, 0]));

    // 2. Hover Goth lot (House 5) and verify StreetNames.iff address & popup metadata.
    let goth_pt = find_lot_hit_point(&shell, 5).expect("Goth lot 5 is hit-testable on map");
    assert!(shell.hover(goth_pt));
    assert_eq!(shell.hovered_lot_id(), Some(5));
    let hovered = shell
        .neighborhood_state()
        .hovered_lot
        .as_ref()
        .expect("hovered lot popup populated");
    assert_eq!(hovered.lot_number, 5);
    assert!(
        hovered
            .address
            .as_deref()
            .is_some_and(|a| a.starts_with("5 ")),
        "expected StreetNames.iff address for lot 5, got {:?}",
        hovered.address
    );
    let goth_occ = hovered.occupant.as_ref().expect("Goth lot is occupied");
    assert!(!goth_occ.family_name.is_empty());
    assert!(
        !goth_occ.portraits.is_empty(),
        "Goth hover popup should include occupant portraits"
    );
    shell.render();

    // 3. Click Goth lot to enter live Game via sims_vm::lifecycle::enter_lot.
    let before_revision = shell.stored_neighborhood().revision;
    shell.press(goth_pt);
    shell.release(goth_pt);
    assert_eq!(shell.screen(), ScreenKind::Lot);
    assert_eq!(shell.active_lot_house(), Some(5));
    {
        let game = shell.game_mut().expect("lot is open");
        assert_eq!(game.lot().lot_id(), 5);
        assert_eq!(game.mode(), Mode::Live);
        let ran = game.run(30);
        assert_eq!(ran, 30);
        assert!(game.dirty(), "ticking the lot marks it dirty");
    }

    // 4. Open Options -> Neighborhood confirmation dialog (Save / Don't Save / Cancel).
    let nbhd_btn = {
        let game = shell.game_mut().expect("lot open");
        game.set_mode(Mode::Options);
        game.render();
        find_game_command_point(game, &Command::OptionsAction(OptionsAction::Neighborhood))
    };
    shell.press(nbhd_btn);
    shell.release(nbhd_btn);
    assert!(
        shell.has_dialog(),
        "dirty lot opens Neighborhood save dialog"
    );

    // Cancel (Escape) keeps the player on the lot.
    shell.key_dialog(DialogKey::Escape);
    assert!(!shell.has_dialog());
    assert_eq!(shell.screen(), ScreenKind::Lot);

    // Trigger Options -> Neighborhood again and accept Yes (Save & Return).
    shell.press(nbhd_btn);
    shell.release(nbhd_btn);
    assert!(shell.has_dialog());
    shell.key_dialog(DialogKey::Enter);
    assert_eq!(
        shell.screen(),
        ScreenKind::Neighborhood,
        "Yes saves and returns cleanly to Neighborhood"
    );
    assert!(
        shell.stored_neighborhood().revision > before_revision,
        "REDB neighborhood revision should advance after Save & Leave"
    );
    assert!(
        shell
            .store()
            .load_saved_lot("UserData", 5)
            .expect("query saved lot")
            .is_some(),
        "House 5 saved lot blob should be persisted in REDB"
    );
    let return_audio = shell.take_audio_inputs();
    assert!(return_audio.contains(&Input::Events(vec![SoundEvent::StopAll])));
    assert!(return_audio.contains(&Input::MusicMode(MusicMode::Neighborhood)));

    // 5. Switch neighborhood 1 -> 2 -> 1 using top-bar switcher buttons.
    let next_btn = find_bar_button_point(&shell, BarButton::NextNeighborhood);
    shell.press(next_btn);
    shell.release(next_btn);
    assert_eq!(shell.neighborhood_slot(), 2);
    assert_eq!(shell.neighborhood_id(), "UserData2");
    assert!(
        shell
            .store()
            .list_neighborhoods()
            .expect("list neighborhoods")
            .contains(&"UserData2".to_owned())
    );
    let prev_btn = find_bar_button_point(&shell, BarButton::PreviousNeighborhood);
    shell.press(prev_btn);
    shell.release(prev_btn);
    assert_eq!(shell.neighborhood_slot(), 1);
    assert_eq!(shell.neighborhood_id(), "UserData");

    // 6. Open Family Bin, pick a bin household, enter Move-In mode, and move onto an affordable vacant residential lot.
    let bin_btn = find_bar_button_point(&shell, BarButton::SelectFamily);
    shell.press(bin_btn);
    shell.release(bin_btn);
    assert_eq!(shell.screen(), ScreenKind::FamilyBin);
    assert!(
        shell
            .take_audio_inputs()
            .contains(&Input::MusicMode(MusicMode::FamilyBin))
    );
    shell.render();
    assert!(
        !shell.family_bin_entries().is_empty(),
        "UserData should have Family Bin households"
    );
    let chosen = shell.family_bin_entries()[0].clone();
    let chosen_id = chosen.id as u16;
    assert!(!chosen.name.is_empty());
    assert!(chosen.funds > 0);

    // Find an affordable vacant residential lot on the map.
    shell
        .start_move_in(chosen_id)
        .expect("start Move-In placement mode");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);
    assert!(matches!(
        shell.neighborhood_state().mode,
        NeighborhoodMode::MoveIn { .. }
    ));

    let target_lot = shell
        .map()
        .lots
        .iter()
        .find(|lot| {
            lot.status == LotStatus::Vacant
                && shell
                    .stored_neighborhood()
                    .is_residential(u32::from(lot.id))
                && sims_vm::lifecycle::lot_price(
                    shell.store(),
                    "UserData",
                    u32::from(lot.id),
                    sims_sim::neighborhood_store::DEFAULT_ARCH_VALUE_MULTIPLIER,
                )
                .is_ok_and(|price| price <= chosen.funds)
                && find_lot_hit_point(&shell, lot.id).is_some()
        })
        .expect("an affordable vacant residential lot exists")
        .id;

    let target_pt = find_lot_hit_point(&shell, target_lot).expect("target lot hit point");
    shell.press(target_pt);
    shell.release(target_pt);
    assert!(
        shell.has_dialog(),
        "Move-In should open confirmation dialog"
    );

    // Confirm Move-In (Yes) -> moves household onto lot in REDB and enters the lot!
    shell.key_dialog(DialogKey::Enter);
    assert_eq!(shell.screen(), ScreenKind::Lot);
    assert_eq!(shell.active_lot_house(), Some(u32::from(target_lot)));
    {
        let stored_after = shell
            .store()
            .load_neighborhood("UserData")
            .unwrap()
            .unwrap();
        let (occ_id, occ_fam) = stored_after
            .family_in(u32::from(target_lot))
            .expect("moved-in family occupies target lot");
        assert_eq!(occ_id, chosen_id);
        assert!(
            occ_fam.budget < chosen.funds,
            "lot price deducted from budget"
        );
    }

    // Leave the newly moved-in lot back to Neighborhood (saving it).
    shell
        .leave_lot(true)
        .expect("leave newly moved-in lot with save");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);
    assert!(
        !shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(chosen_id)),
        "moved-in family should no longer appear in Family Bin"
    );

    // 7. Evict the household back to Family Bin via Bulldoze tool + confirmation dialog.
    let bdoze_btn = find_bar_button_point(&shell, BarButton::Bulldoze);
    shell.press(bdoze_btn);
    shell.release(bdoze_btn);
    assert_eq!(shell.neighborhood_state().mode, NeighborhoodMode::Bulldoze);

    let target_pt = find_lot_hit_point(&shell, target_lot).expect("target lot hit point");
    shell.press(target_pt);
    shell.release(target_pt);
    assert!(
        shell.has_dialog(),
        "clicking occupied lot in Bulldoze opens Evict confirmation"
    );
    shell.key_dialog(DialogKey::Enter);
    assert!(!shell.has_dialog());
    assert!(
        shell
            .stored_neighborhood()
            .family_in(u32::from(target_lot))
            .is_none(),
        "lot should be vacant after Evict"
    );
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(chosen_id)),
        "evicted household should return immediately to Family Bin"
    );
    assert_eq!(
        shell.map().lot(target_lot).unwrap().status,
        LotStatus::Vacant
    );

    // 8. Bulldoze a built vacant house (e.g. House 2 if built, else target_lot if built).
    let built_vacant = [target_lot, 2, 3, 4]
        .into_iter()
        .find(|&id| {
            shell
                .map()
                .lot(id)
                .is_some_and(|l| l.status == LotStatus::Vacant)
                && find_lot_hit_point(&shell, id).is_some()
        })
        .expect("a vacant lot exists for bulldoze check");
    let bdoze_report = shell
        .execute_bulldoze(u32::from(built_vacant))
        .expect("bulldoze vacant lot succeeds");
    assert_eq!(bdoze_report.house, u32::from(built_vacant));
    assert_eq!(bdoze_report.evicted_family, None);
    shell.render();

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn streaming_mode_music_and_volume_slider_wire_through_mixer_and_feed() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping mode music integration test");
        return;
    };
    let install = Path::new(&root);
    let mut mixer = Mixer::open(install).expect("Mixer opens");

    // Starts silent with MusicMode::None.
    assert_eq!(mixer.music_mode(), MusicMode::None);
    assert!(!mixer.music_playing());
    assert!(mixer.render(512).iter().all(|&s| s == 0));

    // Neighborhood music streams non-zero stereo audio (2 s clears ~1 s MP3 lead-in).
    mixer.input(Input::MusicMode(MusicMode::Neighborhood));
    assert_eq!(mixer.music_mode(), MusicMode::Neighborhood);
    assert!(mixer.music_playing());
    let pcm = mixer.render(2 * RATE as usize);
    assert!(
        pcm.iter().any(|&s| s.unsigned_abs() > 64),
        "Neighborhood mode music should stream audible PCM"
    );

    // Music volume slider step 0 (kind 38) silences music immediately; non-zero restores it.
    mixer.input(Input::MusicVolume(0));
    assert_eq!(mixer.volume_steps()[1], 0);
    assert!(
        mixer.render(1024).iter().all(|&s| s == 0),
        "MusicVolume(0) should mute mode music output"
    );
    mixer.input(Input::MusicVolume(8));
    assert_eq!(mixer.volume_steps()[1], 8);

    // Switching to Buy and Build updates active station; Buy -> Live stops immediately.
    mixer.input(Input::MusicMode(MusicMode::Buy));
    assert_eq!(mixer.music_mode(), MusicMode::Buy);
    assert!(mixer.music_playing());
    mixer.input(Input::MusicMode(MusicMode::Build));
    assert_eq!(mixer.music_mode(), MusicMode::Build);
    assert!(mixer.music_playing());
    mixer.input(Input::MusicMode(MusicMode::Live));
    assert_eq!(mixer.music_mode(), MusicMode::Live);
    assert!(
        !mixer.music_playing(),
        "Build -> Live should stop music immediately per 1.91"
    );

    // Feed::lot_music_mode maps standard and Magic Town modes accurately.
    assert_eq!(sound::lot_music_mode(Mode::Live, 5), MusicMode::Live);
    assert_eq!(sound::lot_music_mode(Mode::Buy, 5), MusicMode::Buy);
    assert_eq!(sound::lot_music_mode(Mode::Build, 5), MusicMode::Build);
    assert_eq!(sound::lot_music_mode(Mode::Options, 5), MusicMode::Options);
    assert_eq!(
        sound::lot_music_mode(Mode::Buy, 91),
        MusicMode::MagicTownBuy
    );
    assert_eq!(
        sound::lot_music_mode(Mode::Build, 91),
        MusicMode::MagicTownBuild
    );

    // StopAll stops streaming mode music cleanly too.
    mixer.input(Input::MusicMode(MusicMode::Neighborhood));
    assert!(mixer.music_playing());
    mixer.input(Input::Events(vec![SoundEvent::StopAll]));
    assert!(!mixer.music_playing());
}

#[test]
fn moving_into_previously_edited_vacant_lot_enters_live_mode_and_preserves_funds() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping edited-lot Move-In regression test");
        return;
    };
    let install = PathBuf::from(root);
    let (tmp_dir, saves_path) = temp_redb("p0-edit-movein");

    let mut shell =
        NeighborhoodShell::open(install, saves_path, 1, true, vec![3], Frames::default())
            .expect("NeighborhoodShell opens");
    shell.resize([800, 600]);

    let chosen = shell.family_bin_entries()[0].clone();
    let chosen_id = chosen.id as u16;
    let target_lot = shell
        .map()
        .lots
        .iter()
        .find(|lot| {
            lot.status == LotStatus::Vacant
                && shell
                    .stored_neighborhood()
                    .is_residential(u32::from(lot.id))
                && sims_vm::lifecycle::lot_price(
                    shell.store(),
                    "UserData",
                    u32::from(lot.id),
                    sims_sim::neighborhood_store::DEFAULT_ARCH_VALUE_MULTIPLIER,
                )
                .is_ok_and(|price| price <= chosen.funds)
        })
        .expect("affordable vacant residential lot exists")
        .id;

    // 1. Enter vacant residential lot in edit mode, save via F5 path (also
    // updating thumbnail), then leave without re-saving.
    shell
        .enter_lot(u32::from(target_lot))
        .expect("enter vacant lot");
    {
        let game = shell.game().expect("vacant lot open");
        assert_eq!(
            game.lot().world().globals.get(32).copied().unwrap_or(0),
            1,
            "vacant residential lot opens with global[32] == 1 edit mode"
        );
        assert_ne!(game.mode(), Mode::Live);
    }
    shell.save_current_lot().expect("F5 save on vacant lot");
    shell
        .leave_lot(false)
        .expect("leave vacant lot after F5 save");

    // Verify saved vacant lot persisted global[32] == 1.
    let saved_bytes = shell
        .store()
        .load_saved_lot("UserData", u32::from(target_lot))
        .unwrap()
        .expect("vacant lot saved in REDB");
    let saved = sims_vm::boot::save::Saved::decode(&saved_bytes).unwrap();
    assert_eq!(saved.world().globals.get(32).copied().unwrap_or(0), 1);

    // 2. Move a Family Bin household into that previously edited lot.
    let move_report = shell
        .execute_move_in(chosen_id, u32::from(target_lot))
        .expect("move into previously edited lot succeeds");
    assert_eq!(shell.screen(), ScreenKind::Lot);
    {
        let game = shell.game().expect("occupied lot open");
        assert_eq!(
            game.mode(),
            Mode::Live,
            "occupied residential lot must enter Live mode, not remain locked in edit mode"
        );
        assert_eq!(
            game.lot().world().globals.get(32).copied().unwrap_or(0),
            0,
            "global[32] must be cleared for an occupied residential lot"
        );
        assert_eq!(
            game.lot().world().budget.funds,
            move_report.budget_after,
            "moving-in family funds must be applied to the live world"
        );
    }

    // 3. Save & Leave and verify the household's funds were NOT overwritten by
    // the vacant lot's old balance.
    shell
        .leave_lot(true)
        .expect("Save & Leave after moving into edited lot");
    let after = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    let (_, fam) = after
        .family_in(u32::from(target_lot))
        .expect("family still occupies target lot");
    assert_eq!(
        fam.budget, move_report.budget_after,
        "Save & Leave must preserve moved-in household funds"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn move_in_rollback_magic_town_magicoins_pace_sync_and_family_bin_clicks() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("SIMS_INSTALL is not set; skipping Round 1 regression suite");
        return;
    };
    let install = PathBuf::from(root);
    let (tmp_dir, saves_path) = temp_redb("r1-fixes");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves_path,
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("NeighborhoodShell opens");
    shell.resize([800, 600]);

    // 1. Magic Town residential MagiCoin cost & validation (P1 #4) + MTDesc.iff lookup (P2 #2).
    assert_eq!(
        sims_vm::lifecycle::magic_town_lot_cost(Some(&install), 90),
        1000
    );
    assert_eq!(
        sims_vm::lifecycle::magic_town_lot_cost(Some(&install), 91),
        1500
    );
    assert_eq!(
        sims_vm::lifecycle::magic_town_lot_cost(Some(&install), 92),
        1000
    );
    assert_eq!(
        sims_vm::lifecycle::magic_town_lot_cost(Some(&install), 5),
        0
    );

    shell
        .switch_kind(sims_render::neighborhood::NeighborhoodKind::MagicTown)
        .expect("switch to Magic Town");
    let lot92_pt = find_lot_hit_point(&shell, 92).expect("Magic Town lot 92 hit point");
    shell.hover(lot92_pt);
    let hovered92 = shell
        .neighborhood_state()
        .hovered_lot
        .as_ref()
        .expect("lot 92 hover card");
    assert_eq!(hovered92.magic_cost, 1000);
    assert!(
        !hovered92.house_name.is_empty(),
        "MTDesc.iff STR# 2092 lot name should populate hover card"
    );

    let chosen = shell.family_bin_entries()[0].clone();
    let chosen_id = chosen.id as u16;
    // Give chosen household plenty of Simoleons, 0 MagiCoins.
    {
        let mut nh = shell
            .store()
            .load_neighborhood("UserData")
            .unwrap()
            .unwrap();
        let origin = (nh.instance_id, nh.revision);
        nh.families.get_mut(&chosen_id).unwrap().budget = 50_000;
        shell
            .store()
            .save_neighborhood_checked("UserData", origin.0, origin.1, &nh)
            .unwrap()
            .expect("commit boosted budget");
    }
    shell.refresh_from_store().unwrap();

    // Insufficient MagiCoins must be rejected both by shell Move-In and by lifecycle::move_in.
    shell.start_move_in(chosen_id).unwrap();
    shell.press(lot92_pt);
    shell.release(lot92_pt);
    assert!(
        !shell.has_dialog(),
        "Magic Town lot 92 must not open Move-In confirmation when family lacks MagiCoins"
    );
    let err = sims_vm::lifecycle::move_in(&install, shell.store(), "UserData", chosen_id, 92)
        .expect_err("lifecycle::move_in must reject insufficient MagiCoins");
    assert!(
        matches!(
            err,
            sims_vm::lifecycle::LifecycleError::InsufficientMagicCoins {
                available: 0,
                required: 1000,
                ..
            }
        ),
        "expected InsufficientMagicCoins, got {err}"
    );

    // Grant 1,250 MagiCoins to the family's first member in NGBH inventories.
    {
        let mut nh = shell
            .store()
            .load_neighborhood("UserData")
            .unwrap()
            .unwrap();
        let origin = (nh.instance_id, nh.revision);
        let first_guid = nh.families[&chosen_id].members[0];
        let neighbor_id = nh
            .neighbors
            .iter()
            .flatten()
            .find(|r| r.guid == first_guid)
            .expect("member neighbor record")
            .id;
        if let Some(inv) = nh
            .inventories
            .iter_mut()
            .flatten()
            .find(|inv| inv.neighbor == neighbor_id)
        {
            inv.items.push(sims_formats::neighborhood::InventoryItem {
                kind: sims_vm::lifecycle::MAGIC_COIN_TOKEN_KIND,
                guid: sims_vm::lifecycle::MAGIC_COIN_TOKEN_GUID,
                count: 1250,
            });
        } else {
            nh.inventories
                .push(Some(sims_formats::neighborhood::Inventory {
                    neighbor: neighbor_id,
                    items: vec![sims_formats::neighborhood::InventoryItem {
                        kind: sims_vm::lifecycle::MAGIC_COIN_TOKEN_KIND,
                        guid: sims_vm::lifecycle::MAGIC_COIN_TOKEN_GUID,
                        count: 1250,
                    }],
                }));
        }
        shell
            .store()
            .save_neighborhood_checked("UserData", origin.0, origin.1, &nh)
            .unwrap()
            .expect("commit MagiCoins");
    }
    shell.refresh_from_store().unwrap();
    assert_eq!(
        sims_vm::lifecycle::family_magic_coins(shell.stored_neighborhood(), chosen_id),
        1250
    );
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(chosen_id) && e.magic_coins == 1250)
    );

    // Now Move-In to Magic Town lot 92 succeeds and deducts 1,000 MagiCoins.
    sims_vm::lifecycle::move_in(&install, shell.store(), "UserData", chosen_id, 92)
        .expect("Magic Town Move-In succeeds with sufficient MagiCoins");
    shell.refresh_from_store().unwrap();
    assert_eq!(
        sims_vm::lifecycle::family_magic_coins(shell.stored_neighborhood(), chosen_id),
        250,
        "Magic Town lot 92 purchase must deduct 1,000 MagiCoins"
    );

    // Evict back to bin for the rollback test.
    shell.execute_evict(92).expect("evict from lot 92");

    // 2. Move-In rollback on failed lot boot (P1 #2).
    shell
        .switch_kind(sims_render::neighborhood::NeighborhoodKind::Unleashed)
        .expect("switch back to main neighborhood");
    let bad_lot = shell
        .map()
        .lots
        .iter()
        .find(|lot| {
            lot.status == LotStatus::Vacant
                && shell
                    .stored_neighborhood()
                    .is_residential(u32::from(lot.id))
                && sims_vm::lifecycle::lot_price(
                    shell.store(),
                    "UserData",
                    u32::from(lot.id),
                    sims_sim::neighborhood_store::DEFAULT_ARCH_VALUE_MULTIPLIER,
                )
                .is_ok_and(|price| price <= 50_000)
        })
        .expect("vacant residential lot for rollback test")
        .id;
    let budget_before = shell.stored_neighborhood().families[&chosen_id].budget;
    // Corrupt base house bytes AFTER price check won't run if corrupt, so keep
    // valid SIMI or break enter_lot via a bogus saved lot blob so lot_price
    // fails? Wait: lot_price reads saved_lot first! Instead, store a minimal
    // SIMI-only IFF (passes lot_price, fails House::open during enter_lot!).
    let base_bytes = shell
        .store()
        .load_base_house("UserData", u32::from(bad_lot))
        .unwrap()
        .unwrap();
    let iff = sims_formats::iff::Iff::parse(base_bytes).unwrap();
    let simi_chunk = iff
        .chunks
        .iter()
        .find(|c| &c.tag == b"SIMI")
        .expect("SIMI chunk");
    let simi_only = sims_formats::iff::write(&[sims_formats::iff::ChunkData {
        tag: *b"SIMI",
        id: simi_chunk.id,
        flags: 0,
        label: String::new(),
        data: iff.data(simi_chunk).to_vec(),
    }])
    .unwrap();
    shell
        .store()
        .save_base_house("UserData", u32::from(bad_lot), &simi_only)
        .unwrap();
    let err = shell
        .execute_move_in(chosen_id, u32::from(bad_lot))
        .expect_err("move-in must fail when lot boot fails");
    assert!(
        err.contains("rolled back"),
        "error should report rollback: {err}"
    );
    let after_rb = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    assert!(after_rb.family_in(u32::from(bad_lot)).is_none());
    assert_eq!(after_rb.families[&chosen_id].house, 0);
    assert_eq!(after_rb.families[&chosen_id].budget, budget_before);

    // 3. F9 / reopen pace sync (P2 #3).
    shell.enter_lot(5).expect("enter Goth house");
    {
        let game = shell.game_mut().unwrap();
        game.set_speed(sims_ui::command::Speed::Fast);
    }
    shell.save_current_lot().expect("save at Fast speed");
    shell.restore_current_lot().expect("F9 restore");
    let mut feed = sound::Feed::default();
    let mut inputs = Vec::new();
    feed.update(shell.game_mut().unwrap(), |input| inputs.push(input));
    assert!(
        inputs.contains(&Input::Speed(sims_ui::command::Speed::Fast)),
        "restoring a Fast-speed save must sync Pace/audio speed to Fast: {inputs:?}"
    );
    shell.leave_lot(false).unwrap();

    // 4. Family Bin rapid double-click on Cancel button is NOT swallowed (P2 #5).
    shell.open_family_bin();
    assert_eq!(shell.screen(), ScreenKind::FamilyBin);
    let bin_layout = sims_ui::family_bin::FamilyBinLayout::compute(
        800,
        600,
        shell.family_bin_entries().len(),
        0,
        sims_ui::family_bin::FamilyBinMode::Standard,
    );
    let cancel_rect = bin_layout
        .button_rect(sims_ui::family_bin::FamilyBinButton::Cancel)
        .expect("Cancel button rect");
    let cancel_pt = [
        cancel_rect.x + cancel_rect.width / 2,
        cancel_rect.y + cancel_rect.height / 2,
    ];
    shell.double_click(cancel_pt);
    shell.release(cancel_pt);
    assert_eq!(
        shell.screen(),
        ScreenKind::Neighborhood,
        "rapid second click on Family Bin Cancel button must fall through to press/release"
    );

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
