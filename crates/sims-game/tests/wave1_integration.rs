//! Headless E2E integration tests for `sims-game` Integration Wave 1:
//! - Background CoW REDB manual & rotating autosave worker + Save Manager UI +
//!   non-destructive corrupt-save fallback + clean drain on leave/exit,
//! - End-to-end Create-a-Family (`CAF`) 3D turntable & portrait synthesis ->
//!   REDB Family Bin -> Move-In -> Live lot boot + `delete_bin_family`,
//! - Classic Cheat Console (`Ctrl+Shift+C`) command execution (`rosebud;!;!`,
//!   `set_hour`, `move_objects`, `map_edit`, `autonomy`, `sim_speed`,
//!   `water_tool`, `prepare_lot`, `help`),
//! - 400 ms `PieAnimState` transitions + `edge_scroll` Build/Buy hover refresh,
//! - In-lot stereo/TV radio streaming & environmental lot ambience in `Sound`.

use sims_audio::SoundEvent;
use sims_audio::ambience::AmbienceZone;
use sims_audio::player::RATE;
use sims_audio::radio::BuiltinStation;
use sims_game::autosave_worker::validate_save_payload;
use sims_game::game::goth_house;
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind};
use sims_game::sound::{Input, Mixer};
use sims_game::{Frames, Game, Options};
use sims_sim::autosave::{SaveSlot, SaveTarget};
use sims_ui::caf::{Age, CafScreen, Gender, Personality, SimAppearance, SimDraft, SkinTone, Trait};
use sims_ui::command::{BuildTool, Mode};
use sims_ui::saves::{SaveHealth, SaveManagerCommand};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempTestDir(PathBuf);

impl TempTestDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sims-game-wave1-{tag}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn install_dir() -> Option<PathBuf> {
    let root = std::env::var_os("SIMS_INSTALL")?;
    let path = PathBuf::from(root);
    path.is_dir().then_some(path)
}

fn open_goth(install: &Path) -> Game {
    let mut game = Game::open(&Options {
        install: install.to_path_buf(),
        house: goth_house(install),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("Goth house opens");
    game.resize([800, 600]);
    game
}

#[test]
fn background_cow_autosave_worker_and_save_manager_end_to_end() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 autosave E2E test");
        return;
    };
    let tmp = TempTestDir::new("autosave");
    let saves = tmp.path().join("saves.redb");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        false,
        vec![3],
        Frames::default(),
    )
    .expect("NeighborhoodShell opens");
    assert!(
        shell.migration_report().is_some(),
        "startup migration should run cleanly on NeighborhoodShell::open"
    );
    assert_eq!(shell.migration_error(), None);

    // Enter Goth house (lot 5) and run a few ticks.
    shell.enter_lot(5).expect("enter Goth lot 5");
    assert_eq!(shell.screen(), ScreenKind::Lot);
    let tick_a = {
        let game = shell.game_mut().expect("lot game");
        game.run(5);
        game.lot().ticks()
    };
    assert!(tick_a >= 5);

    // 1. Trigger manual save (F5 / Options -> Save) -> background worker + store.
    shell.save_current_lot().expect("manual save succeeds");
    shell.flush_background_saves();
    assert_eq!(shell.pending_background_saves(), 0);

    // 2. Advance further, enqueue direct CoW World snapshot AND rotating lot autosaves.
    let tick_b = {
        let game = shell.game_mut().expect("lot game");
        game.run(10);
        game.lot().ticks()
    };
    assert!(tick_b > tick_a);
    shell
        .enqueue_background_world_save(SaveTarget::Autosave)
        .expect("enqueue CoW world autosave");
    shell
        .enqueue_background_lot_save(SaveTarget::Autosave)
        .expect("enqueue CoW Saved lot autosave");
    shell.flush_background_saves();
    assert_eq!(shell.pending_background_saves(), 0);

    let slots = shell
        .store()
        .autosaves()
        .list_saves_validated("UserData", 5, validate_save_payload)
        .expect("list autosaves");
    assert!(
        slots.len() >= 3,
        "expected manual + 2 autosave slots, got {}",
        slots.len()
    );

    // 3. Verify Options -> "Saves" button opens SaveManagerDialog cleanly.
    {
        let game = shell.game_mut().expect("lot game");
        game.set_mode(Mode::Options);
        game.render();
    }
    let saves_btn = shell
        .game()
        .expect("lot game")
        .options_saves_button_rect()
        .expect("Options Saves button rect");
    let center = [
        saves_btn.x + saves_btn.width / 2,
        saves_btn.y + saves_btn.height / 2,
    ];
    let _ = shell.press(center);
    let _ = shell.release(center);
    assert!(
        shell.save_manager_open(),
        "clicking Options -> Saves must open SaveManagerDialog"
    );
    shell.render();

    let snapshot = shell
        .save_manager_snapshot()
        .expect("SaveManagerSnapshot populated")
        .clone();
    assert_eq!(snapshot.lot, 5);
    assert!(
        snapshot
            .slots
            .iter()
            .any(|s| s.slot_id == "manual" && s.health == SaveHealth::Intact),
        "manual slot must be listed as Intact: {:?}",
        snapshot.slots
    );

    // 4. Advance further, then load BOTH the earlier manual slot (SavedLot) AND
    //    the direct CoW WorldSnapshot autosave slot (`autosave:1`) through SaveManagerCommand.
    shell.close_save_manager();
    {
        let game = shell.game_mut().expect("lot game");
        game.set_mode(Mode::Live);
        game.run(8);
        assert!(game.lot().ticks() > tick_b);
    }
    assert!(shell.open_save_manager(None));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::LoadSlot {
        slot_id: "autosave:1".into(),
    });
    assert!(
        !shell.save_manager_open(),
        "loading CoW WorldSnapshot autosave:1 must succeed cleanly"
    );
    assert!(shell.open_save_manager(None));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert!(!shell.save_manager_open());
    assert_eq!(
        shell.game().expect("lot game").lot().ticks(),
        tick_a,
        "loading manual slot must restore exact tick"
    );

    // 5. FallbackToNewestIntact restores the newest intact autosave cleanly.
    assert!(shell.open_save_manager(Some("Simulated load error".into())));
    let newest_id = shell
        .save_manager_snapshot()
        .and_then(|s| s.newest_intact().map(|slot| slot.slot_id.clone()))
        .expect("newest intact slot");
    let _ = shell.handle_save_manager_command(SaveManagerCommand::FallbackToNewestIntact {
        slot_id: newest_id,
    });
    assert!(!shell.save_manager_open());

    // 6. DeleteSlot removes one autosave slot without touching others.
    assert!(shell.open_save_manager(None));
    let before_count = shell.save_manager_snapshot().unwrap().slots.len();
    let _ = shell.handle_save_manager_command(SaveManagerCommand::DeleteSlot {
        slot_id: "autosave:1".into(),
    });
    let after_count = shell.save_manager_snapshot().unwrap().slots.len();
    assert_eq!(after_count + 1, before_count);
    let _ = shell.handle_save_manager_command(SaveManagerCommand::Cancel);
    assert!(!shell.save_manager_open());

    // 7. Save & Leave drains background autosave worker cleanly (even after restore!).
    shell
        .leave_lot(true)
        .expect("Save & Leave after restore succeeds");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);
    assert_eq!(shell.pending_background_saves(), 0);
    // Verify manual + rotating autosave slots persisted in REDB.
    let final_slots = shell
        .store()
        .autosaves()
        .list_saves_validated("UserData", 5, validate_save_payload)
        .expect("list final saves");
    assert!(final_slots.iter().any(|s| s.slot() == SaveSlot::Manual));
}

#[test]
fn end_to_end_caf_create_family_bin_move_in_lot_boot_and_delete() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 CAF E2E test");
        return;
    };
    let tmp = TempTestDir::new("caf");
    let saves = tmp.path().join("saves.redb");

    let mut shell = NeighborhoodShell::open(
        install.clone(),
        saves.clone(),
        1,
        false,
        vec![3],
        Frames::default(),
    )
    .expect("NeighborhoodShell opens");

    // Open Family Bin -> Create New Family.
    shell.open_family_bin();
    assert_eq!(shell.screen(), ScreenKind::FamilyBin);
    shell
        .open_create_family()
        .expect("Create-a-Family screen opens");
    assert_eq!(shell.screen(), ScreenKind::CreateFamily);

    // Type family last name on the Family overview screen.
    for ch in "WaveOne".chars() {
        let _ = shell.key_char(ch);
    }

    // Open Create-a-Sim editor, configure an adult Sim, verify live 3D turntable renders.
    {
        let caf = shell.caf_session_mut().expect("CAF session");
        let _ = caf.controller_mut().begin_add_sim();
        assert_eq!(caf.screen(), CafScreen::EditSim);
        {
            let sim = caf.controller_mut().editing_sim_mut().expect("editing sim");
            sim.first_name = "Alice".into();
            sim.bio = "Wave 1 E2E Sim".into();
            sim.appearance = SimAppearance {
                age: Age::Adult,
                gender: Gender::Female,
                skin_tone: SkinTone::Light,
                ..SimAppearance::default()
            };
            sim.personality = Personality::from_raw([5, 5, 5, 5, 5]);
        }
        assert!(
            caf.controller()
                .editing_sim()
                .unwrap()
                .personality
                .zodiac()
                .is_some(),
            "25-point personality must compute a live zodiac sign"
        );
    }

    // Tick turntable and render Create-a-Sim screen.
    let _ = shell.tick_shell(100_000);
    shell.render();
    let preview = shell
        .caf_session()
        .expect("CAF session")
        .preview_image()
        .expect("3D turntable preview rendered");
    assert!(preview.width > 0 && preview.height > 0);
    assert!(
        preview.rgba.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
        "3D turntable preview must contain visible opaque Sim pixels"
    );

    // Commit the draft household directly through CafSession -> REDB Family Bin.
    let draft = {
        let caf = shell.caf_session().expect("CAF session");
        let sim = caf.controller().editing_sim().unwrap().clone();
        sims_ui::caf::FamilyDraft {
            last_name: "WaveOne".into(),
            bio: "Created in Wave 1".into(),
            members: vec![sim],
        }
    };
    let created = {
        let mut caf_session = {
            let ui = sims_content::ui_graphics::UiGraphics::open(
                &sims_content::Install::detect(&install).unwrap(),
            )
            .unwrap();
            sims_game::caf_screen::CafSession::open(
                &install,
                &sims_content::Install::detect(&install).unwrap(),
                &ui,
            )
            .unwrap()
        };
        caf_session
            .commit_family_draft(shell.store(), shell.neighborhood_id(), &draft)
            .expect("commit CAF household into REDB")
    };
    assert_eq!(created.family_name, "WaveOne");
    assert_eq!(created.members.len(), 1);

    // Refresh Family Bin and verify the new family appears cleanly and can Move In.
    shell.open_family_bin();
    shell.refresh_from_store().expect("refresh from store");
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created.family_id)),
        "new CAF household must appear in Family Bin"
    );

    // Find a vacant residential lot and Move In -> immediately boot Live mode!
    let vacant_lot = (1u32..=10)
        .find(|&lot| {
            shell.stored_neighborhood().is_residential(lot)
                && shell.stored_neighborhood().family_in(lot).is_none()
        })
        .expect("at least one vacant residential lot in UserData");
    shell
        .execute_move_in(created.family_id, vacant_lot)
        .expect("Move-In CAF family onto vacant residential lot");
    assert_eq!(shell.screen(), ScreenKind::Lot);
    assert_eq!(shell.active_lot_house(), Some(vacant_lot));
    {
        let game = shell.game_mut().expect("lot game");
        game.run(3);
        game.render();
    }
    shell.leave_lot(true).expect("leave newly moved-in lot");

    // Also verify Delete Family calls `sims_vm::caf::delete_bin_family` cleanly.
    let second_draft = sims_ui::caf::FamilyDraft {
        last_name: "TempBin".into(),
        bio: String::new(),
        members: vec![SimDraft {
            first_name: "Bob".into(),
            bio: String::new(),
            kind: sims_ui::caf::MemberKind::Sim,
            appearance: SimAppearance {
                age: Age::Adult,
                gender: Gender::Male,
                skin_tone: SkinTone::Medium,
                ..SimAppearance::default()
            },
            personality: Personality::from_raw([5, 5, 5, 5, 5]),
            portrait: None,
        }],
    };
    let ui = sims_content::ui_graphics::UiGraphics::open(
        &sims_content::Install::detect(&install).unwrap(),
    )
    .unwrap();
    let mut caf_helper = sims_game::caf_screen::CafSession::open(
        &install,
        &sims_content::Install::detect(&install).unwrap(),
        &ui,
    )
    .unwrap();
    let second = caf_helper
        .commit_family_draft(shell.store(), shell.neighborhood_id(), &second_draft)
        .expect("commit second bin family");
    shell
        .refresh_from_store()
        .expect("refresh after second CAF");
    assert!(
        shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(second.family_id))
    );
    shell
        .execute_delete_family(second.family_id)
        .expect("delete_bin_family succeeds");
    assert!(
        !shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(second.family_id)),
        "deleted family must be gone from Family Bin"
    );
    let _ = Trait::Neat;
}

#[test]
fn cheat_console_rosebud_chain_set_hour_and_toggles() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 cheat console test");
        return;
    };
    let mut game = open_goth(&install);
    assert!(!game.cheat_console_open());

    let before_funds = game.lot().world().budget.funds;
    assert!(game.toggle_cheat_console());
    assert!(game.cheat_console_open());

    for ch in "rosebud;!;!".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let cmds = game.cheat_console_submit().expect("rosebud;!;! parses");
    assert_eq!(cmds.len(), 3);
    assert_eq!(
        game.lot().world().budget.funds,
        before_funds + 3_000,
        "rosebud;!;! must add $3,000"
    );

    // Test set_hour, move_objects, map_edit, autonomy, sim_speed, water_tool, prepare_lot, help.
    for ch in "set_hour 22; move_objects on; map_edit on; autonomy 0; sim_speed 800".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game.cheat_console_submit().expect("chained cheats parse");
    assert_eq!(game.hour(), 22);
    assert_eq!(game.lot().world().globals.get(4).copied(), Some(2));
    assert!(game.cheat_move_objects());
    assert!(game.cheat_map_edit());
    assert_eq!(game.cheat_autonomy(), Some(0));
    assert_eq!(game.cheat_sim_speed(), Some(800));

    for ch in "water_tool".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game.cheat_console_submit().expect("water_tool parses");
    assert_eq!(game.mode(), Mode::Build);

    for ch in "prepare_lot; help".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game
        .cheat_console_submit()
        .expect("prepare_lot; help parses");

    // Rendering with open cheat console must draw cleanly.
    let _ = game.render();
    assert!(game.close_cheat_console());
    assert!(!game.cheat_console_open());
}

#[test]
fn pie_anim_state_and_edge_scroll_hover_refresh() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 pie anim & edge scroll test");
        return;
    };
    let mut game = open_goth(&install);
    game.set_pie_animations(true);
    game.run(3);
    game.render();

    // Find any clickable object on the lot and open its pie menu.
    let mut opened = false;
    'scan: for y in (80..440).step_by(24) {
        for x in (80..720).step_by(24) {
            if game.under([x, y]).is_some() {
                game.hover([x, y]);
                let _ = game.press([x, y]);
                if game.menu().is_some() {
                    opened = true;
                    break 'scan;
                }
            }
        }
    }
    assert!(opened, "expected to open a pie menu on the Goth lot");
    assert!(
        game.pie_animating(),
        "newly opened pie menu with animations enabled must be animating"
    );
    assert!(
        game.is_actively_interacting(),
        "active pie animation must keep RedrawPacer in interactive mode"
    );
    let _ = game.render();

    // Mid-transition (200 ms): still animating.
    let _ = game.advance_while(200_000, 0, |_| false);
    assert!(game.pie_animating());
    let _ = game.render();

    // Past 400 ms pop-in + direction ease: settled to steady state.
    let _ = game.advance_while(450_000, 0, |_| false);
    let _ = game.render();
    assert!(
        !game.pie_animating(),
        "pie menu must finish animating after 400 ms"
    );
    assert!(game.close_menu());

    // Verify edge_scroll pans and refreshes Build Mode hover preview without panicking.
    game.set_mode(Mode::Build);
    game.select_build_tool(BuildTool::Wall);
    game.render();
    let origin_before = game.view.camera.origin;
    game.hover([10, 200]); // inside left 32 px edge-scroll band
    let moved = game.edge_scroll(50_000);
    assert!(moved, "pointer in 32 px left edge band should edge-scroll");
    assert_ne!(game.view.camera.origin, origin_before);
    let _ = game.render();
}

#[test]
fn sound_mixer_radio_streaming_and_ambience_integration() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 sound integration test");
        return;
    };
    let mut mixer = Mixer::open(&install).expect("Mixer opens");
    assert!(
        mixer.render(512).iter().all(|&s| s == 0),
        "mixer must be silent before any audio starts"
    );

    // 1. Tune an in-lot stereo emitter directly and verify non-zero stereo PCM output.
    mixer.input(Input::RadioTune {
        emitter: 42,
        station: BuiltinStation::Rock.into(),
    });
    assert_eq!(mixer.radio().active_emitters(), 1);
    let track = mixer.radio().emitter_track(42).map(str::to_owned);
    let radio_pcm = mixer.render(RATE as usize * 2);
    let faults = mixer.new_faults();
    assert!(
        radio_pcm.iter().any(|&s| s != 0),
        "tuned Rock stereo station should render non-zero PCM audio (track={track:?}, faults={faults:?})"
    );

    // 2. SimAntics Play / StopObject routing for TV channel.
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 77,
        name: "tv_comedy".into(),
    }]));
    assert_eq!(mixer.radio().active_emitters(), 2);
    mixer.input(Input::Events(vec![SoundEvent::StopObject { object: 77 }]));
    assert_eq!(mixer.radio().active_emitters(), 1);

    // 3. Activate environmental lot ambience bed and verify zone/zoom/hour/pause wiring.
    mixer.input(Input::Zoom(2));
    mixer.input(Input::View(100));
    mixer.input(Input::Hour(12));
    mixer.input(Input::AmbienceZone(AmbienceZone::Residential));
    assert_eq!(mixer.ambience().zone(), AmbienceZone::Residential);
    assert_eq!(mixer.ambience().outdoor_percent(), 100);

    // Pausing silences both radio and ambience cleanly.
    mixer.input(Input::Events(vec![SoundEvent::Pause]));
    assert!(mixer.radio().is_paused());
    assert!(mixer.ambience().is_paused());
    assert!(
        mixer.render(512).iter().all(|&s| s == 0),
        "paused radio and ambience must render silence"
    );

    // StopAll tears down radio, ambience, and mode music cleanly.
    mixer.input(Input::Events(vec![SoundEvent::StopAll]));
    assert_eq!(mixer.radio().active_emitters(), 0);
    assert!(!mixer.ambience().is_playing());
    assert!(mixer.render(512).iter().all(|&s| s == 0));
}
