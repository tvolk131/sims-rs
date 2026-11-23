//! End-to-end Options panel, volume slider -> `sims-audio::Player`, and
//! native confirmation dialog tests on the Goth house.

use sims_game::game::goth_house;
use sims_game::hud::DialogKey;
use sims_game::sound::{Feed, Input, Mixer};
use sims_game::{Game, Options};
use sims_ui::command::{
    Command, DetailLevel, Mode, OptionsAction, OptionsHelp, OptionsRadio, OptionsTab,
    OptionsToggle, VolumeChannel,
};
use sims_ui::hud::{self, Kind};
use sims_vm::boot::Frames;
use std::path::PathBuf;

fn install_root() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn open_goth(root: &std::path::Path) -> Game {
    let mut game = Game::open(&Options {
        install: root.to_path_buf(),
        house: goth_house(root),
        answers: vec![3],
        play: false,
        frames: Frames {
            after: Vec::new(),
            every: Vec::new(),
        },
    })
    .unwrap();
    game.resize([800, 600]);
    game.render();
    game
}

fn find_command_point(game: &Game, target: &Command) -> [i32; 2] {
    let item = game
        .panel()
        .iter()
        .find(|i| i.command.as_ref() == Some(target))
        .unwrap_or_else(|| panic!("expected item for {target:?}"));
    [
        item.rect.x + item.rect.width / 2,
        item.rect.y + item.rect.height / 2,
    ]
}

#[test]
fn options_sound_sliders_wire_into_mixer_and_player_volumes() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run options/audio integration test");
        return;
    };
    let mut game = open_goth(&root);
    let mut mixer = Mixer::open(&root).unwrap();
    let mut feed = Feed::default();
    feed.update(&mut game, |input| mixer.input(input));
    // Installed Complete Collection `UserData/Options.iff` (`Optn` 128) stores `[8, 8, 8]`.
    assert_eq!(mixer.volume_steps(), [8, 8, 8]);

    // Click the cluster's Options mode button.
    let opt_btn = find_command_point(&game, &Command::SetMode(Mode::Options));
    assert!(game.press(opt_btn).redraw);
    game.render();
    assert_eq!(game.mode(), Mode::Options);

    // Entering Options suspends lot simulation (`0x4ce567`, `0x46d566`) and
    // pauses live audio (`0x4ce4c4..0x4ce4d1`).
    assert_eq!(game.advance(500_000), 0);
    let mut mode_inputs = Vec::new();
    feed.update(&mut game, |input| {
        mode_inputs.push(input.clone());
        mixer.input(input);
    });
    assert!(mode_inputs.iter().any(|i| matches!(
        i,
        Input::Events(ev) if ev.iter().any(|e| matches!(e, sims_audio::SoundEvent::Pause))
    )));

    // Clicking the already-selected Options mode button keeps the strip open
    // (`0x53a51b..0x53a53e -> 0x4cea68..0x4cea73`).
    let opt_btn_again = find_command_point(&game, &Command::SetMode(Mode::Options));
    game.press(opt_btn_again);
    game.render();
    assert_eq!(game.mode(), Mode::Options);
    assert!(
        game.panel()
            .iter()
            .any(|i| i.command == Some(Command::OpenOptionsTab(OptionsTab::Sound)))
    );

    // Open the Sound subpanel.
    let sound_tab = find_command_point(&game, &Command::OpenOptionsTab(OptionsTab::Sound));
    assert!(game.press(sound_tab).redraw);
    game.render();

    // Pressing FX slider at min, leaving the window (capture preserved,
    // `0x5f6245..0x5f625b`), dragging outside the window, and releasing at an
    // out-of-window coordinate evaluates the release x-coordinate (`0x5f62ba..0x5f62de`).
    let fx_rect = hud::slider_rect(game.panel(), VolumeChannel::Fx).unwrap();
    game.press([fx_rect.x + 1, fx_rect.y + 7]);
    assert!(game.dragging_slider());
    assert_eq!(game.volumes()[0], 0);
    game.leave();
    assert!(game.dragging_slider());
    assert!(game.drag_slider(fx_rect.x + 65));
    assert_eq!(game.volumes()[0], 5);
    game.release([1200, fx_rect.y + 7]);
    assert!(!game.dragging_slider());
    assert_eq!(game.volumes()[0], 10);

    // Set FX to 0, Music to middle (5), and drag Vox slider.
    game.press([fx_rect.x + 1, fx_rect.y + 7]);
    game.release([fx_rect.x + 1, fx_rect.y + 7]);

    let music_rect = hud::slider_rect(game.panel(), VolumeChannel::Music).unwrap();
    game.press([music_rect.x + 65, music_rect.y + 7]);
    game.release([music_rect.x + 65, music_rect.y + 7]);

    let vox_rect = hud::slider_rect(game.panel(), VolumeChannel::Vox).unwrap();
    game.press([vox_rect.x + 1, vox_rect.y + 7]);
    assert!(game.hover([vox_rect.x + 40, vox_rect.y + 7]));
    game.release([vox_rect.x + 40, vox_rect.y + 7]);
    let expected_vox = sims_ui::options::slider_value_at(vox_rect, vox_rect.x + 40);

    assert_eq!(game.volumes(), [0, 5, expected_vox]);
    let mut sent = Vec::new();
    feed.update(&mut game, |input| {
        sent.push(input.clone());
        mixer.input(input);
    });
    assert!(sent.contains(&Input::FxVolume(0)));
    assert!(sent.contains(&Input::MusicVolume(5)));
    assert!(sent.contains(&Input::VoxVolume(expected_vox)));
    assert_eq!(mixer.volume_steps(), [0, 5, expected_vox]);

    // Switching back to Live unpauses live audio, and reopening Options resets
    // the active tab to None (`0x5179eb..0x5179f6`).
    let live_btn = find_command_point(&game, &Command::SetMode(Mode::Live));
    game.press(live_btn);
    game.render();
    let mut back_inputs = Vec::new();
    feed.update(&mut game, |input| {
        back_inputs.push(input.clone());
        mixer.input(input);
    });
    assert!(back_inputs.iter().any(|i| matches!(
        i,
        Input::Events(ev) if ev.iter().any(|e| matches!(e, sims_audio::SoundEvent::Unpause))
    )));
    let opt_btn = find_command_point(&game, &Command::SetMode(Mode::Options));
    game.press(opt_btn);
    assert_eq!(game.options().tab, None);
}

#[test]
fn options_play_controls_focus_actions_and_mouse_up_dialogs_work() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run options/dialog integration test");
        return;
    };
    let mut game = open_goth(&root);
    let mut feed = Feed::default();
    feed.update(&mut game, |_| {});

    // Switch to Options, then blur BEFORE calling Feed::update: focus changes
    // must NOT swallow the pending Options audio Pause (`0x4ce4c4..0x4ce4d1`,
    // `0x4f762b..0x4f763c`).
    let opt_btn = find_command_point(&game, &Command::SetMode(Mode::Options));
    game.press(opt_btn);
    game.render();
    assert!(!game.options().play.sim_in_background);
    let mut blur_inputs = Vec::new();
    assert!(!feed.set_focused(false, &mut game, |i| blur_inputs.push(i)));
    assert!(!game.paused());
    assert_eq!(blur_inputs.last(), Some(&Input::Active(false)));
    let mut after_blur = Vec::new();
    feed.update(&mut game, |i| after_blur.push(i));
    assert!(after_blur.iter().any(|i| matches!(
        i,
        Input::Events(ev) if ev.iter().any(|e| matches!(e, sims_audio::SoundEvent::Pause))
    )));
    feed.set_focused(true, &mut game, |_| {});
    feed.update(&mut game, |_| {});

    // On a clean ordinary lot, Options action buttons dispatch on mouse-up
    // (`0x514170..0x514176`, `0x5e45d2..0x5e45f3`): pressing Save and dragging
    // off cancels it without saving.
    assert!(!game.dirty());
    let save_btn = find_command_point(&game, &Command::OptionsAction(OptionsAction::Save));
    let quit_btn = find_command_point(&game, &Command::OptionsAction(OptionsAction::Quit));
    let save_down = game.press(save_btn);
    assert!(!save_down.save && !save_down.quit);
    game.hover([5, 5]);
    let save_drag_cancel = game.release([5, 5]);
    assert!(!save_drag_cancel.save && !save_drag_cancel.quit);

    // Clicking Quit (press + release) calls `0x4f290d` -> 2-button Yes/No
    // `STR# 153[1..2]` `cTSWinGenDlg` (`PopupInfoTiles.bmp` 3008). Pressing Save
    // BEFORE the next `render()` MUST be blocked by the modal dialog.
    game.press(quit_btn);
    game.release(quit_btn);
    assert!(game.has_dialog());
    assert!(game.over_panel(save_btn));
    game.press(save_btn);
    let blocked = game.release(save_btn);
    assert!(!blocked.save && !blocked.quit);
    // Localized confirmation keyboard shortcuts (`0x5e5b3b..0x5e5bfd`, `STR# 161`):
    // Escape cancels/declines, 'y' accepts Yes, 'n' declines No.
    let esc_out = game.dialog_confirm_key(DialogKey::Escape);
    assert!(!esc_out.quit && !game.has_dialog());

    // Reopen clean Quit and confirm with 'y' (`0x5e5bdb..0x5e5bd0`).
    game.press(quit_btn);
    game.release(quit_btn);
    let y_quit = game.dialog_confirm_key(DialogKey::Char('y'));
    assert!(y_quit.quit && !y_quit.save && !game.has_dialog());

    // Reopen clean Quit and test mouse-up activation on Yes.
    game.press(quit_btn);
    game.release(quit_btn);
    game.render();
    assert!(game.has_dialog());
    let clean_dlg = game.dialog().expect("Clean Quit dialog shown").clone();
    assert_eq!(clean_dlg.buttons.len(), 2);
    let clean_yes = [clean_dlg.buttons[0].x + 5, clean_dlg.buttons[0].y + 5];
    game.press(clean_yes);
    let clean_quit = game.release(clean_yes);
    assert!(clean_quit.quit && !clean_quit.save);
    game.render();
    assert!(game.dialog().is_none());

    // On an ordinary lot (`!edit_mode`), Save bypasses the edit-mode save card
    // (`0x517c21`) and returns `save: true` on mouse-up (`0x516874 -> 0x4f26cf`).
    game.press(save_btn);
    let direct_save = game.release(save_btn);
    assert!(direct_save.save && !direct_save.quit);
    assert!(game.dialog().is_none());

    // In edit mode (`[lot + 0x50] == 1`), Save opens the 1-button Save card
    // (`0x517c03(0) -> 0x50d68a`, frame 3008 + icon 4994 + Lot Name & multiline
    // Lot Description), Return inserts a newline in Lot Description instead of
    // submitting (`0x5f28b6..0x5f28f9`), accepted edits mark the lot dirty
    // until a save succeeds, and OK persists both fields through save/restore.
    game.set_edit_mode(true);
    assert!(!game.dirty());
    game.press(save_btn);
    game.release(save_btn);
    game.render();
    let card_dlg = game.dialog().expect("Edit-mode Save card shown").clone();
    assert_eq!(card_dlg.buttons.len(), 1);
    assert!(card_dlg.icon.is_some());
    assert!(card_dlg.entry.is_some());
    assert!(card_dlg.secondary_entry.is_some());
    for ch in "Goth Manor".chars() {
        assert!(game.type_dialog_char(ch));
    }
    assert!(game.dialog_switch_field());
    assert!(game.dialog_enter_is_newline());
    for ch in "Spooky".chars() {
        assert!(game.type_dialog_char(ch));
    }
    // Pressing Enter in multiline Lot Description inserts '\n' without closing the card.
    let newline_out = game.dialog_confirm_key(DialogKey::Enter);
    assert!(newline_out.redraw && !newline_out.save && game.has_dialog());
    for ch in "House".chars() {
        assert!(game.type_dialog_char(ch));
    }
    let ok_pt = [card_dlg.buttons[0].x + 5, card_dlg.buttons[0].y + 5];
    game.press(ok_pt);
    let card_out = game.release(ok_pt);
    assert!(card_out.save && !card_out.quit);
    // Accepted Save Card edits mark the lot dirty so a failed snapshot write
    // still takes the dirty-lot Quit path rather than discarding metadata edits.
    assert!(game.dirty());
    assert_eq!(game.lot_name(), "Goth Manor");
    assert_eq!(game.lot_description(), "Spooky\nHouse");
    // Save Card edits survive encode -> decode -> restore in a fresh Game.
    let encoded_save = game.save().unwrap().encode();
    let mut restored_game = open_goth(&root);
    restored_game
        .restore(sims_vm::boot::save::Saved::decode(&encoded_save).unwrap())
        .unwrap();
    assert_eq!(restored_game.lot_name(), "Goth Manor");
    assert_eq!(restored_game.lot_description(), "Spooky\nHouse");
    game.set_edit_mode(false);
    game.render();
    assert!(game.dialog().is_none());

    // Open Play tab: Free Will writes lot global 30 (`0x43222a..0x43223d`),
    // and clicking the Free Will label opens its help balloon with UI_whoosh (`0x516719`).
    let play_tab = find_command_point(&game, &Command::OpenOptionsTab(OptionsTab::Play));
    game.press(play_tab);
    game.render();
    assert_eq!(game.lot().world().globals[30], 1);
    let free_will_cb = find_command_point(&game, &Command::ToggleOption(OptionsToggle::FreeWill));
    game.press(free_will_cb);
    assert!(!game.options().play.free_will);
    assert_eq!(game.lot().world().globals[30], 0);
    assert!(game.dirty());

    let _ = game.take_sounds();
    let free_will_lbl = find_command_point(&game, &Command::OptionsHelp(OptionsHelp::FreeWill));
    game.press(free_will_lbl);
    assert_eq!(game.options().help, Some(OptionsHelp::FreeWill));
    game.render();
    assert!(
        game.panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::OptionsBalloon(_)))
    );
    assert!(game.take_sounds().iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_whoosh"
    )));

    // Clicking the same label again closes the balloon with UI_whoosh.
    game.press(free_will_lbl);
    assert_eq!(game.options().help, None);
    assert!(game.take_sounds().iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_whoosh"
    )));

    // `0x517957..0x5179cd`: hovering outside the Options panel or leaving the
    // window also dismisses an open Options help balloon with UI_whoosh.
    game.press(free_will_lbl);
    assert_eq!(game.options().help, Some(OptionsHelp::FreeWill));
    let _ = game.take_sounds();
    assert!(game.hover([100, 100]));
    assert_eq!(game.options().help, None);
    assert!(game.take_sounds().iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_whoosh"
    )));
    game.press(free_will_lbl);
    assert_eq!(game.options().help, Some(OptionsHelp::FreeWill));
    let _ = game.take_sounds();
    assert!(game.leave());
    assert_eq!(game.options().help, None);
    assert!(game.take_sounds().iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_whoosh"
    )));

    // `0x4f25b4..0x4f25f5 -> 0x4f1a11`: confirming Reset Tutorial on a dirty
    // savable lot chains through the 3-button save confirmation (`STR# 153[3..4]`).
    let reset_tut_btn =
        find_command_point(&game, &Command::OptionsAction(OptionsAction::TutorialReset));
    game.press(reset_tut_btn);
    game.release(reset_tut_btn);
    game.render();
    assert_eq!(game.dialog().unwrap().buttons.len(), 2);
    // Yes on ConfirmResetTutorial opens 3-button ConfirmSaveBeforeResetTutorial.
    let reset_yes = game.dialog_confirm_key(DialogKey::Char('y'));
    assert!(!reset_yes.save && game.has_dialog());
    game.render();
    assert_eq!(game.dialog().unwrap().buttons.len(), 3);
    assert_eq!(
        (
            game.dialog().unwrap().rect.width,
            game.dialog().unwrap().rect.height
        ),
        (396, 135)
    );
    // Cancel aborts Reset Tutorial cleanly.
    let reset_cancel = game.dialog_confirm_key(DialogKey::Escape);
    assert!(!reset_cancel.save && !game.has_dialog());
    assert!(game.note().is_none());
    // Reopen and choose Yes -> Yes: returns `save: true` and host `save_to`
    // preserves the unported Reset Tutorial note alongside the save result.
    game.press(reset_tut_btn);
    game.release(reset_tut_btn);
    let _ = game.dialog_confirm_key(DialogKey::Char('y'));
    let reset_save_yes = game.dialog_confirm_key(DialogKey::Char('y'));
    assert!(reset_save_yes.save && !game.has_dialog());
    {
        let save_dir = std::env::temp_dir().join(format!(
            "sims-game-options-save-note-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&save_dir).unwrap();
        let store = sims_sim::Snapshots::open(&save_dir.join("saves.redb")).unwrap();
        assert!(game.save_to(&store, "reset-note"));
        assert!(
            game.note()
                .is_some_and(|n| n.contains("Saved after tick") && n.contains("Tutorial reset"))
        );
        // Re-dirty so downstream dirty-lot assertions keep running.
        game.press(free_will_cb);
        game.press(free_will_cb);
        assert!(game.dirty());
        let _ = std::fs::remove_dir_all(&save_dir);
    }

    // `0x517904..0x517954` + `0x5175ab..0x5175bb -> 0x50fb7f`: Escape in
    // Options closes the active tab and any open help balloon (emitting both
    // `UI_whoosh` and `UI_click`), and consumes Escape even when no tab is open.
    game.press(free_will_lbl);
    assert_eq!(game.options().help, Some(OptionsHelp::FreeWill));
    let _ = game.take_sounds();
    assert_eq!(game.options().tab, Some(OptionsTab::Play));
    assert!(game.escape_options());
    assert_eq!(game.options().tab, None);
    assert_eq!(game.options().help, None);
    let esc_sounds = game.take_sounds();
    assert!(esc_sounds.iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_whoosh"
    )));
    assert!(esc_sounds.iter().any(|s| matches!(
        s,
        sims_vm::SoundEvent::Play { name, .. } if name == "UI_click"
    )));
    assert!(game.escape_options());
    assert_eq!(game.mode(), Mode::Options);

    // Graphics Shadows/Lighting toggle actually changes rendered lot pixels.
    let gfx_tab = find_command_point(&game, &Command::OpenOptionsTab(OptionsTab::Graphics));
    game.press(gfx_tab);
    game.render();
    let with_shadows = game.canvas().color.clone();
    let shadows_cb = find_command_point(&game, &Command::ToggleOption(OptionsToggle::Shadows));
    game.press(shadows_cb);
    game.render();
    assert_ne!(game.canvas().color, with_shadows);

    // Dirty edit-mode Quit (`0x51681a..0x51686f`):
    // - Clicking No on the Save Card MUST NOT quit directly; it clears `dirty`
    //   temporarily and opens the 2-button ConfirmQuit dialog, restoring `dirty`
    //   when the user declines ConfirmQuit!
    // - Clicking Yes on the Save Card saves (`save: true, quit: false`) and also
    //   opens the 2-button ConfirmQuit dialog.
    game.set_edit_mode(true);
    assert!(game.dirty());
    game.press(quit_btn);
    game.release(quit_btn);
    game.render();
    let edit_quit_card = game.dialog().expect("Edit-mode SaveCardQuit shown").clone();
    assert_eq!(edit_quit_card.buttons.len(), 3);
    // 3-button Save Card draws the STR# 153[5] confirmation prompt above the buttons.
    assert_eq!(edit_quit_card.lines.len(), 1);
    let no_pt = [
        edit_quit_card.buttons[1].x + 5,
        edit_quit_card.buttons[1].y + 5,
    ];
    game.press(no_pt);
    let card_no = game.release(no_pt);
    assert!(!card_no.save && !card_no.quit);
    assert!(game.has_dialog());
    assert!(!game.dirty());
    let final_confirm = game.dialog().expect("Follow-up ConfirmQuit shown").clone();
    assert_eq!(final_confirm.buttons.len(), 2);
    // Decline final ConfirmQuit with 'n' -> stays open and restores `dirty`.
    let declined = game.dialog_confirm_key(DialogKey::Char('n'));
    assert!(!declined.quit && !game.has_dialog());
    assert!(game.dirty());

    // Now test Yes on edit-mode SaveCardQuit -> returns `save: true, quit: false`
    // and opens 2-button ConfirmQuit.
    game.press(quit_btn);
    game.release(quit_btn);
    let card_yes_out = game.dialog_confirm_key(DialogKey::Enter);
    assert!(card_yes_out.save && !card_yes_out.quit);
    assert!(game.has_dialog());
    assert_eq!(game.dialog().unwrap().buttons.len(), 2);
    let confirm_cancel = game.dialog_confirm_key(DialogKey::Escape);
    assert!(!confirm_cancel.quit && !game.has_dialog());
    game.set_edit_mode(false);

    // Dirty ordinary lot Quit opens 3-button Save-before-Quit dialog (`0x516811..0x51686f`).
    // Dialog buttons activate on mouse-up (`0x5e44b4..0x5e45fc`): dragging off cancels.
    game.press(quit_btn);
    game.release(quit_btn);
    game.render();
    let dlg = game
        .dialog()
        .expect("Save-before-Quit confirmation dialog shown")
        .clone();
    assert_eq!(dlg.buttons.len(), 3);
    let yes_pt = [dlg.buttons[0].x + 5, dlg.buttons[0].y + 5];
    let cancel_pt = [dlg.buttons[2].x + 5, dlg.buttons[2].y + 5];
    // Press on Yes, drag off and release -> cancelled, dialog stays open.
    game.press(yes_pt);
    assert!(game.has_ui_dialog());
    game.hover([5, 5]);
    let cancelled = game.release([5, 5]);
    assert!(!cancelled.save && !cancelled.quit);
    assert!(game.has_ui_dialog());
    // Press and release on Cancel -> closes dialog cleanly without quitting.
    game.press(cancel_pt);
    let cancel_out = game.release(cancel_pt);
    assert!(!cancel_out.save && !cancel_out.quit);
    game.render();
    assert!(game.dialog().is_none());

    // Press Neighborhood on dirty ordinary lot -> `0x51712d -> 0x4f1a11` 3-button
    // `cTSWinGenDlg` Save prompt (`STR# 153[3..4]`, 396x135, no save card); Yes returns `save: true`.
    let nbhd_btn = find_command_point(&game, &Command::OptionsAction(OptionsAction::Neighborhood));
    game.press(nbhd_btn);
    game.release(nbhd_btn);
    game.render();
    let nbhd_dlg = game
        .dialog()
        .expect("Neighborhood save dialog shown")
        .clone();
    assert_eq!(nbhd_dlg.buttons.len(), 3);
    assert!(nbhd_dlg.icon.is_none());
    assert_eq!((nbhd_dlg.rect.width, nbhd_dlg.rect.height), (396, 135));
    let yes_pt = [nbhd_dlg.buttons[0].x + 5, nbhd_dlg.buttons[0].y + 5];
    game.press(yes_pt);
    let nbhd_out = game.release(yes_pt);
    assert!(nbhd_out.save && !nbhd_out.quit);
    {
        let save_dir = std::env::temp_dir().join(format!(
            "sims-game-options-nbhd-note-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&save_dir).unwrap();
        let store = sims_sim::Snapshots::open(&save_dir.join("saves.redb")).unwrap();
        assert!(game.save_to(&store, "nbhd-note"));
        assert!(
            game.note()
                .is_some_and(|n| n.contains("Saved after tick") && n.contains("Neighborhood"))
        );
        // A host store-open error calling `set_note` clears any pending
        // follow-up note so a later save does not leak the old action note.
        game.press(play_tab);
        game.render();
        game.press(free_will_cb);
        game.press(free_will_cb);
        assert!(game.dirty());
        game.press(nbhd_btn);
        game.release(nbhd_btn);
        let nbhd_retry = game.dialog_confirm_key(DialogKey::Char('y'));
        assert!(nbhd_retry.save);
        game.set_note("The lot was not saved: store open failed");
        assert!(game.save_to(&store, "after-open-fail"));
        assert!(
            game.note()
                .is_some_and(|n| n.contains("Saved after tick") && !n.contains("Neighborhood"))
        );
        // Restore Graphics tab so downstream assertions stay unchanged.
        game.press(gfx_tab);
        game.render();
        let _ = std::fs::remove_dir_all(&save_dir);
    }
    game.render();
    assert!(game.dialog().is_none());

    // Queuing an interaction on a clean paused lot marks the lot dirty so
    // Options -> Quit opens the 3-button `ConfirmSaveBeforeQuit` dialog
    // (`0x516809..0x51686f -> 0x4f290d`).
    {
        let mut q_game = open_goth(&root);
        assert!(!q_game.dirty());
        q_game.toggle_pause();
        assert!(q_game.paused());
        let mut queued = false;
        'scan: for y in (80..440).step_by(24) {
            for x in (120..700).step_by(24) {
                q_game.hover([x, y]);
                q_game.press([x, y]);
                q_game.release([x, y]);
                q_game.render();
                if let Some(entry_pt) = q_game.menu().and_then(|menu| {
                    menu.labels()
                        .into_iter()
                        .next()
                        .map(|(_, r)| [r.x + r.width / 2, r.y + r.height / 2])
                }) {
                    q_game.press(entry_pt);
                    q_game.release(entry_pt);
                    if q_game.dirty() {
                        queued = true;
                        break 'scan;
                    }
                    q_game.close_menu();
                }
            }
        }
        assert!(queued && q_game.dirty());
        let q_opt = find_command_point(&q_game, &Command::SetMode(Mode::Options));
        q_game.press(q_opt);
        q_game.render();
        let q_quit = find_command_point(&q_game, &Command::OptionsAction(OptionsAction::Quit));
        q_game.press(q_quit);
        q_game.release(q_quit);
        assert_eq!(q_game.dialog().unwrap().buttons.len(), 3);
    }

    // `0x5e45d2..0x5e45f3`: resizing while holding an Options action button
    // hit-tests the button's updated layout bounds on release, so releasing at
    // the pre-resize point does not trigger Save.
    let save_before_resize =
        find_command_point(&game, &Command::OptionsAction(OptionsAction::Save));
    game.press(save_before_resize);
    game.resize([800, 700]);
    let resized_release = game.release(save_before_resize);
    assert!(!resized_release.save);
    game.resize([800, 600]);
    game.render();

    // `0x4e66eb..0x4e6704 -> 0x432220`: restoring an earlier snapshot keeps the
    // player's current Free Will preference instead of rolling it back.
    let mut fw_game = open_goth(&root);
    assert!(fw_game.options().play.free_will);
    assert_eq!(fw_game.lot().world().globals[30], 1);
    let snap_fw_on = fw_game.lot().save().unwrap();
    let fw_opt = find_command_point(&fw_game, &Command::SetMode(Mode::Options));
    fw_game.press(fw_opt);
    fw_game.render();
    let fw_play = find_command_point(&fw_game, &Command::OpenOptionsTab(OptionsTab::Play));
    fw_game.press(fw_play);
    fw_game.render();
    let fw_toggle = find_command_point(&fw_game, &Command::ToggleOption(OptionsToggle::FreeWill));
    fw_game.press(fw_toggle);
    assert!(!fw_game.options().play.free_will);
    assert_eq!(fw_game.lot().world().globals[30], 0);
    let old_quit_pt = find_command_point(&fw_game, &Command::OptionsAction(OptionsAction::Quit));
    fw_game.restore(snap_fw_on).unwrap();
    // Restoring a Live snapshot from Options immediately relayouts the panel so
    // clicking the old Options Quit button before `render()` does not open Quit.
    assert_eq!(fw_game.mode(), Mode::Live);
    assert!(!fw_game.escape_options());
    fw_game.press(old_quit_pt);
    let stale_release = fw_game.release(old_quit_pt);
    assert!(!stale_release.quit && !fw_game.has_dialog());
    assert!(!fw_game.options().play.free_will);
    assert_eq!(fw_game.lot().world().globals[30], 0);
    assert!(
        !sims_ui::options::OptionsState::from_optn(&fw_game.options().to_optn())
            .unwrap()
            .play
            .free_will
    );

    // `0x44b0a8..0x44b0fc`, `0x44b18a..0x44b1cc`: runtime Make New Character
    // names sync into the HUD `Faces` cache so newly created children and pets
    // display their submitted names.
    {
        let house_path = root.join("UserData/Houses/House05.iff");
        let opened = sims_render::open_house(&root, &house_path).unwrap();
        let mut hud = sims_game::hud::Hud::open(
            &root,
            &house_path,
            &opened.assets,
            fw_game.lot().world(),
            fw_game
                .lot()
                .program()
                .autonomy_constants()
                .friendship_threshold,
        )
        .unwrap();
        let new_guid = 0xDEAD_BEEF;
        assert_eq!(hud.name(new_guid), None);
        let names = std::collections::BTreeMap::from([(new_guid, "Sparky".to_owned())]);
        hud.sync_character_names(&names);
        assert_eq!(hud.name(new_guid), Some("Sparky"));
        // Restoring an earlier snapshot without that GUID drops the runtime name.
        hud.reset_character_names(&std::collections::BTreeMap::new());
        assert_eq!(hud.name(new_guid), None);
    }

    // Restoring an answered-awaiting-replay snapshot also keeps Free Will off
    // (`0x432220..0x432242`), both before and after replaying the dialog tick.
    {
        let mut answered_lot = sims_vm::boot::Lot::load(
            &root,
            &root.join("UserData/Houses/House05.iff"),
            &[],
            Frames {
                after: Vec::new(),
                every: Vec::new(),
            },
        )
        .unwrap();
        answered_lot.wait_for_answers();
        answered_lot.step().unwrap();
        assert!(answered_lot.step().is_err());
        assert!(answered_lot.asking().is_some());
        answered_lot.answer(3).unwrap();
        let snap_answered = answered_lot.save().unwrap();
        let mut restored_game = open_goth(&root);
        let opt_p = find_command_point(&restored_game, &Command::SetMode(Mode::Options));
        restored_game.press(opt_p);
        restored_game.render();
        let play_p = find_command_point(&restored_game, &Command::OpenOptionsTab(OptionsTab::Play));
        restored_game.press(play_p);
        restored_game.render();
        let fw_p = find_command_point(
            &restored_game,
            &Command::ToggleOption(OptionsToggle::FreeWill),
        );
        restored_game.press(fw_p);
        assert!(!restored_game.options().play.free_will);
        restored_game.restore(snap_answered).unwrap();
        assert!(!restored_game.options().play.free_will);
        assert_eq!(restored_game.lot().world().globals[30], 0);
        assert_eq!(restored_game.run(1), 1);
        assert_eq!(restored_game.lot().world().globals[30], 0);
    }

    // `0x5e44dd..0x5e44e1`: holding an Options action button draws its pressed
    // cell (distinct from hover pixels) while the pointer stays inside it.
    {
        let save_pt = find_command_point(&game, &Command::OptionsAction(OptionsAction::Save));
        game.hover(save_pt);
        game.render();
        let hovered_pixels = game.canvas().color.clone();
        game.press(save_pt);
        game.render();
        let pressed_pixels = game.canvas().color.clone();
        assert_ne!(hovered_pixels, pressed_pixels);
        // Cancel by dragging off before release so we don't save.
        game.hover([5, 5]);
        let _ = game.release([5, 5]);
    }

    // `0x5e45d2..0x5e45f3`: resizing while holding a modal button relayouts the
    // dialog so releasing at the pre-resize coordinates does not activate it.
    {
        let quit_pt = find_command_point(&game, &Command::OptionsAction(OptionsAction::Quit));
        game.press(quit_pt);
        game.release(quit_pt);
        game.render();
        let dlg = game.dialog().unwrap().clone();
        let yes_pt = [dlg.buttons[0].x + 5, dlg.buttons[0].y + 5];
        game.press(yes_pt);
        game.resize([800, 1000]);
        let out = game.release(yes_pt);
        assert!(!out.quit && !out.save);
        assert!(game.has_ui_dialog());
        game.resize([800, 600]);
        game.dialog_confirm_key(DialogKey::Escape);
        game.render();
        assert!(!game.has_ui_dialog());
    }

    // `0x517c21`, `0x46d8c9`: restoring a snapshot keeps `edit_mode()` synced
    // with restored global 32 so edit mode and the §999,999 budget never split.
    {
        let mut em_game = open_goth(&root);
        assert!(!em_game.edit_mode());
        em_game.set_edit_mode(true);
        assert_eq!(em_game.lot().world().globals[32], 1);
        let snap_edit = em_game.lot().save().unwrap();
        em_game.set_edit_mode(false);
        assert!(!em_game.edit_mode());
        assert_eq!(em_game.lot().world().globals[32], 0);
        em_game.restore(snap_edit).unwrap();
        assert!(em_game.edit_mode());
        assert_eq!(em_game.lot().world().globals[32], 1);
        assert_eq!(sims_vm::available_funds(em_game.lot().world()), 999_999);
    }

    // `0x523825..0x523887`: `Hud::clear_vm_dialog` drops cached VM dialog state
    // so a consecutive identical `(caller, operands)` prompt starts fresh.
    {
        let house_path = root.join("UserData/Houses/House05.iff");
        let opened = sims_render::open_house(&root, &house_path).unwrap();
        let mut world = fw_game.lot().world().clone();
        world.dialogs.records.push(sims_sim::Dialog {
            caller: 77,
            operands: [0, 0, 0, 0, 0, 3, 0, 0],
            status: 1,
        });
        let mut hud = sims_game::hud::Hud::open(
            &root,
            &house_path,
            &opened.assets,
            &world,
            fw_game
                .lot()
                .program()
                .autonomy_constants()
                .friendship_threshold,
        )
        .unwrap();
        hud.read_dialog(&world, &opened.assets);
        assert!(hud.active_dialog_entry().is_some());
        assert!(hud.type_dialog_char('Z'));
        assert!(hud.active_dialog_entry().unwrap().contains('Z'));
        hud.clear_vm_dialog();
        hud.read_dialog(&world, &opened.assets);
        assert!(!hud.active_dialog_entry().unwrap().contains('Z'));
    }

    // Options preferences persist across relaunch via `save_options` / `load_options`.
    let opt_dir = std::env::temp_dir().join(format!("sims-game-options-{}", std::process::id()));
    let opt_file = opt_dir.join("Options.iff");
    game.set_volume(VolumeChannel::Fx, 4);
    game.set_volume(VolumeChannel::Music, 2);
    game.save_options(&opt_file).unwrap();
    let mut relaunched = open_goth(&root);
    relaunched.load_options(&opt_file).unwrap();
    assert_eq!(relaunched.volumes()[0], 4);
    assert_eq!(relaunched.volumes()[1], 2);
    assert!(!relaunched.options().graphics.shadows);
    assert!(!relaunched.options().play.free_will);
    assert_eq!(relaunched.lot().world().globals[30], 0);
    let _ = std::fs::remove_dir_all(&opt_dir);

    // Return to Live mode and verify Sim In Background suppresses blur pause in Live mode too.
    let play_tab = find_command_point(&game, &Command::OpenOptionsTab(OptionsTab::Play));
    game.press(play_tab);
    game.render();
    let sim_bg = find_command_point(
        &game,
        &Command::ToggleOption(OptionsToggle::SimInBackground),
    );
    game.press(sim_bg);
    assert!(game.options().play.sim_in_background);
    let live_btn = find_command_point(&game, &Command::SetMode(Mode::Live));
    game.press(live_btn);
    game.render();
    feed.update(&mut game, |_| {});
    let mut live_blur = Vec::new();
    assert!(!feed.set_focused(false, &mut game, |i| live_blur.push(i)));
    assert!(!game.paused());
    assert_eq!(live_blur.last(), Some(&Input::Active(false)));

    // Edge Scrolling uses the house viewport bottom, 32 px windowed scroll band,
    // and exact wakeup-split-invariant native rates (`562.5` px/s straight edges
    // at `0x4d4839`, `0x4d4946`, `0x4d499d`, `0x4d4a06`; corner vector capped to
    // `562.5` px/s at `0x4d4ab5`; `0x4d50ef..0x4d5160` 100 ms cap):
    // - zero elapsed time moves 0 px
    // - `16_000` us straight edge moves 9 px identically for `1 + 15_999` us
    // - `40 x 1 ms` matches `1 x 40 ms` (22 px)
    // - straight left/right `100 ms` moves 56 px (`562.5` px/s, not 112 px)
    // - corner `100 ms` moves 50 px X and 25 px Y (vector norm <= 56.25 px)
    // - `[400, 480]` and `[400, 498]` pan down while `[400, 598]` over the HUD does not pan.
    let origin_before = game.view.camera.origin;
    game.hover([400, 2]);
    assert!(!game.edge_scroll(0));
    assert_eq!(game.view.camera.origin, origin_before);
    let y_16_single = game.view.camera.origin[1];
    assert!(game.edge_scroll(16_000));
    assert_eq!((game.view.camera.origin[1] - y_16_single).abs(), 9);
    game.hover([400, 200]);
    assert!(!game.edge_scroll(1_000));
    game.hover([400, 2]);
    let y_16_split = game.view.camera.origin[1];
    assert!(!game.edge_scroll(1));
    assert!(game.edge_scroll(15_999));
    assert_eq!((game.view.camera.origin[1] - y_16_split).abs(), 9);

    game.hover([400, 200]);
    assert!(!game.edge_scroll(1_000));
    game.hover([400, 2]);
    let mut moved_by_1ms = 0i32;
    let start_y = game.view.camera.origin[1];
    for _ in 0..40 {
        if game.edge_scroll(1_000) {
            moved_by_1ms = (game.view.camera.origin[1] - start_y).abs();
        }
    }
    assert_eq!(moved_by_1ms, 22);
    let start_y_single = game.view.camera.origin[1];
    game.hover([400, 200]);
    assert!(!game.edge_scroll(1_000));
    game.hover([400, 2]);
    assert!(game.edge_scroll(40_000));
    assert_eq!((game.view.camera.origin[1] - start_y_single).abs(), 22);

    // Straight horizontal edges move 56 px per 100 ms (`562.5` px/s).
    game.hover([400, 200]);
    assert!(!game.edge_scroll(1_000));
    game.hover([2, 200]);
    let x_left = game.view.camera.origin[0];
    assert!(game.edge_scroll(100_000));
    assert_eq!((game.view.camera.origin[0] - x_left).abs(), 56);
    game.hover([798, 200]);
    let x_right = game.view.camera.origin[0];
    assert!(game.edge_scroll(100_000));
    assert_eq!((game.view.camera.origin[0] - x_right).abs(), 56);

    // Corner vector (`0x4d4893`/`0x4d48f9` capped at `0x4d4ab5` to 562.5 px/s):
    // 100 ms moves 50 px X and 25 px Y.
    game.hover([400, 200]);
    assert!(!game.edge_scroll(1_000));
    game.hover([2, 2]);
    let corner_start = game.view.camera.origin;
    assert!(game.edge_scroll(100_000));
    assert_eq!((game.view.camera.origin[0] - corner_start[0]).abs(), 50);
    assert_eq!((game.view.camera.origin[1] - corner_start[1]).abs(), 25);

    let origin_before_hud = game.view.camera.origin;
    game.hover([400, 598]);
    assert!(!game.edge_scroll(40_000));
    assert_eq!(game.view.camera.origin, origin_before_hud);
    game.hover([400, 480]);
    assert!(game.edge_scroll(40_000));
    assert_ne!(game.view.camera.origin, origin_before_hud);
    let origin_after_480 = game.view.camera.origin;
    game.hover([400, 498]);
    assert!(game.edge_scroll(40_000));
    assert_ne!(game.view.camera.origin, origin_after_480);
}

#[test]
fn unoccupied_magic_and_studio_lots_boot_into_edit_mode_with_native_save_card() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test unoccupied edit-mode lots");
        return;
    };
    let open_house = |rel: &str| {
        let mut game = Game::open(&Options {
            install: root.clone(),
            house: root.join(rel),
            answers: Vec::new(),
            play: false,
            frames: Frames {
                after: Vec::new(),
                every: Vec::new(),
            },
        })
        .unwrap();
        game.resize([800, 600]);
        game.render();
        game
    };

    // Magic Town lot 90 (`MTDesc.iff STR# 2090`) boots cleanly into edit mode
    // with global 32 = 1 -> §999,999 edit budget (`0x4f1bd9..0x4f1be5`,
    // `0x46d8c9`) and Buy mode (`0x4f2510..0x4f2520`), survives window ticks
    // without halting, and opens the native Save Card with its lot name/desc.
    let mut lot90 = open_house("UserData/Houses/House90.iff");
    assert!(lot90.edit_mode());
    assert_eq!(lot90.lot().world().globals[32], 1);
    assert_eq!(sims_vm::available_funds(lot90.lot().world()), 999_999);
    assert_eq!(lot90.mode(), Mode::Buy);
    assert_eq!(lot90.advance(50_000), 0);
    assert!(lot90.stopped().is_none());
    assert!(!lot90.lot_name().is_empty());
    let original_name = lot90.lot_name().to_owned();
    let opt_btn = find_command_point(&lot90, &Command::SetMode(Mode::Options));
    lot90.press(opt_btn);
    lot90.render();
    assert_eq!(lot90.mode(), Mode::Options);
    let save_btn = find_command_point(&lot90, &Command::OptionsAction(OptionsAction::Save));
    lot90.press(save_btn);
    lot90.release(save_btn);
    assert!(lot90.has_dialog());
    assert_eq!(lot90.dialog_entry(), Some(original_name.as_str()));
    assert_eq!(lot90.dialog().unwrap().buttons.len(), 1);
    for ch in "abc".chars() {
        assert!(lot90.type_dialog_char(ch));
    }
    assert_eq!(lot90.dialog_entry(), Some("abc"));
    // `0x5f27ae..0x5f2887`: mouse-drag across Save Card text selects it so the
    // next typed character replaces the dragged range instead of prepending.
    let name_rect = lot90.dialog().unwrap().entry.unwrap();
    lot90.press([name_rect.x + name_rect.width - 2, name_rect.y + 10]);
    assert!(lot90.hover([name_rect.x + 2, name_rect.y + 10]));
    lot90.release([name_rect.x + 2, name_rect.y + 10]);
    assert!(lot90.type_dialog_char('X'));
    assert_eq!(lot90.dialog_entry(), Some("X"));
    assert!(lot90.dialog_backspace());
    // `0x5f27a6`: stationary click (including macOS/winit same-position
    // `CursorMoved` emitted right before mouse-up) in a scrolled 49-char Lot
    // Name or scrolled multiline Lot Description must NOT create a selection
    // or delete characters when typing next.
    let long_name = "WWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWWW";
    assert_eq!(long_name.len(), 48);
    for ch in long_name.chars() {
        assert!(lot90.type_dialog_char(ch));
    }
    let click_pt = [name_rect.x + 12, name_rect.y + 10];
    lot90.press(click_pt);
    lot90.hover(click_pt);
    lot90.release(click_pt);
    assert!(lot90.type_dialog_char('X'));
    assert_eq!(lot90.dialog_entry().unwrap().len(), 49);
    assert!(lot90.dialog_entry().unwrap().contains('X'));

    // Same stationary-click + same-position `hover` check in scrolled multiline Lot Description.
    assert!(lot90.dialog_switch_field());
    let desc_rect = lot90.dialog().unwrap().secondary_entry.unwrap();
    let long_desc = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8";
    for ch in long_desc.chars() {
        assert!(lot90.type_dialog_char(ch));
    }
    let desc_click = [desc_rect.x + 12, desc_rect.y + 8];
    lot90.press(desc_click);
    lot90.hover(desc_click);
    lot90.release(desc_click);
    assert!(lot90.type_dialog_char('Y'));
    assert_eq!(
        lot90.dialog_secondary_entry().unwrap().len(),
        long_desc.len() + 1
    );
    assert!(lot90.dialog_secondary_entry().unwrap().contains('Y'));

    // Switch back to Lot Name (selects all) and submit final edited name.
    assert!(lot90.dialog_switch_field());
    for ch in "Edited Lot 90".chars() {
        assert!(lot90.type_dialog_char(ch));
    }
    let ok_out = lot90.dialog_submit();
    assert!(ok_out.save && lot90.dirty());
    assert_eq!(lot90.lot_name(), "Edited Lot 90");

    // Vacant residential lot (`House02.iff`, zoning 1, `0x4f1b5f..0x4f1b76`),
    // Downtown (25), Vacation Island (all 9 lots 40..=48, whose controllers
    // create people during `reconcile_controllers` before family selection),
    // Studio Town (81), and Magic Town (93) all boot cleanly into edit mode
    // with global 32 = 1 (§999,999 budget), survive `advance(50_000)` without
    // halting, and open the native Save Card.
    for lot_num in [2u32, 25, 40, 41, 42, 43, 44, 45, 46, 47, 48, 81, 93] {
        let rel = format!("UserData/Houses/House{lot_num:02}.iff");
        let mut game = open_house(&rel);
        assert!(game.edit_mode(), "lot {lot_num} should boot in edit mode");
        assert_eq!(
            game.lot().world().globals[32],
            1,
            "lot {lot_num} should enable edit budget global 32"
        );
        assert_eq!(
            sims_vm::available_funds(game.lot().world()),
            999_999,
            "lot {lot_num} should have §999,999 edit budget"
        );
        assert_eq!(
            game.mode(),
            Mode::Buy,
            "lot {lot_num} should open in Buy mode"
        );
        assert_eq!(
            game.advance(50_000),
            0,
            "lot {lot_num} should not advance in edit mode"
        );
        assert!(
            game.stopped().is_none(),
            "lot {lot_num} halted after advance: {:?}",
            game.stopped()
        );
        // `0x4f2482..0x4f248b` -> `0x53b233..0x53b254`: edit-mode lots disable
        // the Live button and stay out of Live mode.
        let live_item = game
            .panel()
            .iter()
            .find(|i| {
                matches!(
                    i.kind,
                    sims_ui::hud::Kind::Button {
                        id: sims_ui::hud::id::LIVE,
                        ..
                    }
                )
            })
            .expect("Live button present");
        assert!(
            live_item.command.is_none(),
            "lot {lot_num} should disable Live button in edit mode"
        );
        game.set_mode(Mode::Live);
        assert_eq!(
            game.mode(),
            Mode::Buy,
            "lot {lot_num} should refuse Live mode in edit mode"
        );
        if lot_num == 2 {
            // `NeighborhoodDesc.iff STR# 2002` has empty strings on disk.
            assert!(game.lot_name().is_empty());
            // `0x4d25b6..0x4d25c7` -> `0x4d45f3`: Edge Scrolling also pans in Buy mode.
            let origin_before = game.view.camera.origin;
            game.hover([400, 480]);
            assert!(game.edge_scroll(40_000));
            assert_ne!(game.view.camera.origin, origin_before);
        } else {
            assert!(
                !game.lot_name().is_empty(),
                "lot {lot_num} missing *Desc.iff name"
            );
        }
        let opt_btn = find_command_point(&game, &Command::SetMode(Mode::Options));
        game.press(opt_btn);
        game.render();
        assert_eq!(
            game.mode(),
            Mode::Options,
            "lot {lot_num} should switch to Options"
        );
        let save_btn = find_command_point(&game, &Command::OptionsAction(OptionsAction::Save));
        game.press(save_btn);
        game.release(save_btn);
        assert!(game.has_dialog(), "lot {lot_num} Save Card did not open");
        assert_eq!(game.dialog_entry(), Some(game.lot_name()));
    }
}

#[test]
fn options_saves_and_coop_buttons_never_overlap_any_subpanel_widget_across_resolutions() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test Options full-composite zero-overlap");
        return;
    };
    let mut game = open_goth(&root);

    let rects_overlap = |a: sims_ui::frame::Rect, b: sims_ui::frame::Rect| {
        a.width > 0
            && a.height > 0
            && b.width > 0
            && b.height > 0
            && a.x < b.x + b.width
            && a.x + a.width > b.x
            && a.y < b.y + b.height
            && a.y + a.height > b.y
    };

    for [w, h] in [[640, 480], [800, 600], [1024, 768], [1280, 720]] {
        game.resize([w, h]);
        game.set_mode(Mode::Options);
        game.render();

        for tab in [
            None,
            Some(OptionsTab::Graphics),
            Some(OptionsTab::Sound),
            Some(OptionsTab::Play),
        ] {
            if let Some(t) = tab {
                if game.options().tab != Some(t) {
                    let pt = find_command_point(&game, &Command::OpenOptionsTab(t));
                    game.press(pt);
                }
            } else if game.options().tab.is_some() {
                assert!(game.escape_options());
            }
            let canvas = game.render();
            if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(PathBuf::from)
                && matches!([w, h], [800, 600] | [1024, 768])
                && matches!(tab, Some(OptionsTab::Play | OptionsTab::Graphics))
            {
                let tag = match tab {
                    Some(OptionsTab::Play) => "play",
                    Some(OptionsTab::Graphics) => "graphics",
                    _ => unreachable!(),
                };
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(
                    dir.join(format!("options_{tag}_{w}x{h}.png")),
                    sims_render::png::encode(canvas),
                );
            }

            let saves = game
                .options_saves_button_rect()
                .expect("Saves button laid out in Options");
            let coop = game
                .options_coop_button_rect()
                .expect("Co-op button laid out in Options");
            assert!(
                !rects_overlap(saves, coop),
                "Saves {saves:?} and Co-op {coop:?} overlap at {w}x{h} tab={tab:?}"
            );
            for (name, r) in [("Saves", saves), ("Co-op", coop)] {
                assert!(
                    r.x >= 0 && r.y >= 0 && r.x + r.width <= w && r.y + r.height <= h,
                    "{name} {r:?} clips outside {w}x{h} tab={tab:?}"
                );
                let cluster = sims_ui::frame::Rect::new(
                    0,
                    (h - sims_ui::hud::CLUSTER[1]).max(0),
                    sims_ui::hud::CLUSTER[0],
                    sims_ui::hud::CLUSTER[1],
                );
                assert!(
                    !rects_overlap(r, cluster),
                    "{name} {r:?} overlaps cluster {cluster:?} at {w}x{h} tab={tab:?}"
                );
                for item in game.panel() {
                    if matches!(
                        item.kind,
                        sims_ui::hud::Kind::Stretched(_)
                            | sims_ui::hud::Kind::Image(
                                sims_ui::hud::id::STRIP | sims_ui::hud::id::BACK_PATCH
                            )
                    ) {
                        continue;
                    }
                    assert!(
                        !rects_overlap(r, item.rect),
                        "{name} {r:?} overlaps Options item {:?} ({:?}) at {w}x{h} tab={tab:?}",
                        item.kind,
                        item.rect
                    );
                }
            }

            // Confirm Export HTML (Play tab) and Character Detail High radio (Graphics tab)
            // receive clicks directly without being intercepted by Saves/Co-op!
            if tab == Some(OptionsTab::Play) {
                let before = game.options().play.export_html;
                let pt =
                    find_command_point(&game, &Command::ToggleOption(OptionsToggle::ExportHtml));
                game.press(pt);
                assert_ne!(
                    game.options().play.export_html,
                    before,
                    "Export HTML click must not be intercepted at {w}x{h}"
                );
                game.press(pt); // restore
            }
            if tab == Some(OptionsTab::Graphics) {
                let pt = find_command_point(
                    &game,
                    &Command::SetDetail {
                        radio: OptionsRadio::CharacterDetail,
                        level: DetailLevel::Med,
                    },
                );
                game.press(pt);
                assert_eq!(
                    game.options().graphics.character_detail,
                    DetailLevel::Med,
                    "Character Detail Med click must not be intercepted at {w}x{h}"
                );
                let high = find_command_point(
                    &game,
                    &Command::SetDetail {
                        radio: OptionsRadio::CharacterDetail,
                        level: DetailLevel::High,
                    },
                );
                game.press(high);
                assert_eq!(
                    game.options().graphics.character_detail,
                    DetailLevel::High,
                    "Character Detail High click must not be intercepted at {w}x{h}"
                );
            }
        }
    }

    // Synchronous relayout check (`P2 #8`): switching Options tab updates
    // Saves/Co-op rects immediately BEFORE `render()` is called.
    game.resize([800, 600]);
    game.set_mode(Mode::Options);
    if game.options().tab.is_some() {
        assert!(game.escape_options());
    }
    let before_tab_saves = game
        .options_saves_button_rect()
        .expect("Saves laid out synchronously on base Options");
    let play_btn = find_command_point(&game, &Command::OpenOptionsTab(OptionsTab::Play));
    game.press(play_btn);
    let after_tab_saves = game
        .options_saves_button_rect()
        .expect("Saves updated synchronously on OpenOptionsTab before render()");
    assert_ne!(
        before_tab_saves, after_tab_saves,
        "Saves button rect must update synchronously before draw when opening Play tab at 800x600"
    );

    // Help balloon + Quick Tips hover tooltip at 800x600 must NEVER overlap Saves/Co-op.
    let help_pt = find_command_point(&game, &Command::OptionsHelp(OptionsHelp::FreeWill));
    game.press(help_pt);
    let save_action = find_command_point(&game, &Command::OptionsAction(OptionsAction::Save));
    game.hover(save_action);
    game.render();
    if let (Some(saves), Some(coop)) = (
        game.options_saves_button_rect(),
        game.options_coop_button_rect(),
    ) {
        assert!(!rects_overlap(saves, coop));
        for r in [saves, coop] {
            for item in game.panel() {
                if matches!(
                    item.kind,
                    sims_ui::hud::Kind::Stretched(_)
                        | sims_ui::hud::Kind::Image(
                            sims_ui::hud::id::STRIP | sims_ui::hud::id::BACK_PATCH
                        )
                ) {
                    continue;
                }
                let obstacle = match &item.kind {
                    sims_ui::hud::Kind::OptionsTooltip(_) => {
                        // Quick Tips tooltip draws immediately above its anchor rect (`0x517a6f`).
                        sims_ui::frame::Rect::new(
                            (item.rect.x - 40).max(0),
                            (item.rect.y - 26).max(0),
                            item.rect.width + 80,
                            24,
                        )
                    }
                    _ => item.rect,
                };
                assert!(
                    !rects_overlap(r, obstacle),
                    "Saves/Co-op {r:?} overlapped balloon/tooltip/widget {:?} ({obstacle:?})",
                    item.kind
                );
            }
        }
    }

    // Extreme narrow viewport (`320x240`, `P3 #1`): when the pair cannot fit
    // cleanly within bounds without overlap, return `None` instead of clipping.
    game.resize([320, 240]);
    game.render();
    assert_eq!(
        game.options_saves_button_rect(),
        None,
        "Saves button must be hidden rather than clip out of a 320x240 viewport"
    );
    assert_eq!(
        game.options_coop_button_rect(),
        None,
        "Co-op button must be hidden rather than clip out of a 320x240 viewport"
    );

    // Resizing in Buy Mode on an Old Town community lot (`House58.iff`, P2 #4)
    // must keep `BuyZone::Community` and preserve category/selection across resize + render.
    let old_town_house = root.join("UserData/Houses/House58.iff");
    if old_town_house.is_file() {
        let mut community = Game::open(&Options {
            install: root.clone(),
            house: old_town_house,
            answers: vec![3],
            play: false,
            frames: Frames {
                after: Vec::new(),
                every: Vec::new(),
            },
        })
        .expect("open Old Town community lot House58");
        community.resize([800, 600]);
        community.set_mode(Mode::Buy);
        community.render();
        assert_eq!(community.buy_state().zone, sims_ui::buy::BuyZone::Community);
        let cat0 = community
            .buy_layout()
            .widgets
            .iter()
            .find(|w| w.command == Some(Command::BuyCategory(0)))
            .expect("Community BuyCategory(0)")
            .rect;
        community.press([cat0.x + cat0.width / 2, cat0.y + cat0.height / 2]);
        community.render();
        assert_eq!(community.buy_state().category, Some(0));
        // Resize from 800x600 -> 1024x768 -> render: zone & category MUST stay intact!
        community.resize([1024, 768]);
        assert_eq!(community.buy_state().zone, sims_ui::buy::BuyZone::Community);
        assert_eq!(community.buy_state().category, Some(0));
        community.render();
        assert_eq!(community.buy_state().zone, sims_ui::buy::BuyZone::Community);
        assert_eq!(community.buy_state().category, Some(0));
    }
}
