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
