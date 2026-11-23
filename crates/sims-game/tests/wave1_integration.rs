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
use sims_sim::autosave::{SaveJob, SavePayload, SaveSlot, SaveTarget};
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

    // 1. Trigger manual save (F5 / Options -> Save) -> strictly fenced commit +
    //    mirrored manual slot on background worker.
    let gen_before_load = shell.lot_generation();
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
    assert!(
        shell.lot_generation() > gen_before_load,
        "in-place Save Manager slot load must bump lot_generation so host resets error indices & audio"
    );
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

    // 8. P1 #3 regression: corrupt the main SAVED_LOTS record so `enter_lot(5)` fails,
    //    then verify Save Manager recovers directly from an intact autosave without
    //    retrying the corrupt SAVED_LOTS record.
    let nh_now = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .expect("UserData present");
    shell
        .store()
        .save_lot_and_neighborhood("UserData", 5, b"corrupt-saved-lot-bytes", &nh_now)
        .expect("inject corrupt SAVED_LOTS record");
    assert!(
        shell.enter_lot(5).is_err(),
        "entering a lot with corrupt SAVED_LOTS must fail"
    );
    let intact_autosave_id = final_slots
        .iter()
        .find_map(|s| match s.slot() {
            SaveSlot::Autosave(seq) => Some(format!("autosave:{seq}")),
            SaveSlot::Manual => None,
        })
        .expect("intact autosave exists");
    assert!(shell.open_save_manager(Some("Corrupt SAVED_LOTS test".into())));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::FallbackToNewestIntact {
        slot_id: intact_autosave_id,
    });
    assert!(
        !shell.save_manager_open(),
        "corrupt SAVED_LOTS recovery via Save Manager must succeed cleanly"
    );
    assert_eq!(shell.screen(), ScreenKind::Lot);

    // 9. P1 #1 regression: advancing the neighborhood revision externally must
    //    cause `save_current_lot()` to fail with a fencing error instead of
    //    overwriting the newer neighborhood state.
    let bumped_nh = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .expect("UserData present");
    shell
        .store()
        .save_neighborhood("UserData", &bumped_nh)
        .expect("externally bump neighborhood revision");
    assert!(
        shell.save_current_lot().is_err(),
        "stale-revision lot save must fail fencing rather than overwriting"
    );
    shell
        .leave_lot(false)
        .expect("leave lot without saving after fencing rejection");
}

#[test]
fn restoring_pre_caf_lot_slot_preserves_newer_caf_family_and_neighbors() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping wave1 pre-CAF reconciliation test");
        return;
    };
    let tmp = TempTestDir::new("pre-caf-reconcile");
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

    // 1. Enter Lot 5, run a few ticks, save manual slot (pre-CAF!), and leave.
    shell.enter_lot(5).expect("enter lot 5");
    shell.game_mut().unwrap().run(4);
    shell.save_current_lot().expect("pre-CAF manual save");
    shell.flush_background_saves();
    shell.leave_lot(false).expect("leave lot 5");

    // 2. Create a brand-new CAF household in REDB (advancing neighborhood + NBRS).
    let ui = sims_content::ui_graphics::UiGraphics::open(
        &sims_content::Install::detect(&install).unwrap(),
    )
    .unwrap();
    let mut caf = sims_game::caf_screen::CafSession::open(
        &install,
        &sims_content::Install::detect(&install).unwrap(),
        &ui,
    )
    .unwrap();
    let draft = sims_ui::caf::FamilyDraft {
        last_name: "Persisted".into(),
        bio: "Created after lot 5 save.".into(),
        members: vec![SimDraft {
            first_name: "Nova".into(),
            bio: "Test Sim".into(),
            kind: sims_ui::caf::MemberKind::Sim,
            appearance: SimAppearance {
                age: Age::Adult,
                gender: Gender::Female,
                skin_tone: SkinTone::Light,
                ..SimAppearance::default()
            },
            personality: Personality::from_raw([5, 5, 5, 5, 5]),
            portrait: None,
        }],
    };
    let created = caf
        .commit_family_draft(shell.store(), shell.neighborhood_id(), &draft)
        .expect("commit CAF household");
    let caf_guid = created.members[0].guid;
    shell.refresh_from_store().expect("refresh after CAF");

    // 3. Also mutate the Goth family's current funds in REDB (simulating an
    //    updated household balance), then re-enter Lot 5, load the OLDER
    //    pre-CAF manual slot via Save Manager, run a few ticks, and Save & Leave!
    let mut updated_nh = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .expect("UserData present");
    let expected_funds = {
        let (_, fam) = updated_nh.family_in_mut(5).expect("Goth family on lot 5");
        fam.budget = fam.budget.saturating_add(777);
        fam.budget
    };
    shell
        .store()
        .save_neighborhood("UserData", &updated_nh)
        .expect("persist updated Goth budget");
    shell
        .refresh_from_store()
        .expect("refresh after budget edit");

    shell.enter_lot(5).expect("re-enter lot 5");
    assert!(shell.open_save_manager(None));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert!(!shell.save_manager_open());
    assert_eq!(
        shell.game().unwrap().lot().world().budget.funds,
        expected_funds,
        "restoring an older lot slot must preserve the live neighborhood's current household budget"
    );
    shell.game_mut().unwrap().run(3);
    shell
        .leave_lot(true)
        .expect("Save & Leave after loading pre-CAF slot");

    // 4. Verify BOTH the newer CAF family, its NBRS record, AND the updated
    //    household budget survived!
    let final_nh = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .expect("UserData present");
    assert!(
        final_nh.families.contains_key(&created.family_id),
        "newer CAF family must survive restoring and saving an older lot slot"
    );
    assert!(
        final_nh
            .neighbors
            .iter()
            .flatten()
            .any(|n| n.guid == caf_guid),
        "newer CAF Sim NBRS record must survive restoring and saving an older lot slot"
    );
    assert_eq!(
        final_nh.family_in(5).map(|(_, f)| f.budget),
        Some(expected_funds),
        "current household budget must not roll back when saving a restored older slot"
    );

    // 5. Verify a cheat-mutated lot save (`rosebud`) persists cleanly via the
    //    full lifecycle save pipeline and reloads with the cheated funds intact.
    shell
        .enter_lot(5)
        .expect("enter lot 5 for cheat save check");
    {
        let game = shell.game_mut().unwrap();
        game.toggle_cheat_console();
        for ch in "rosebud".chars() {
            game.cheat_console_char(ch);
        }
        game.cheat_console_submit().expect("rosebud executes");
    }
    shell
        .leave_lot(true)
        .expect("Save & Leave after rosebud cheat");
    let after_cheat_nh = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .expect("UserData present");
    assert_eq!(
        after_cheat_nh.family_in(5).map(|(_, f)| f.budget),
        Some(expected_funds + 1_000),
        "rosebud cheat funds must persist through Save & Leave"
    );
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
    let expected_funds = before_funds + 3_000;
    assert_eq!(
        game.lot().world().budget.funds,
        expected_funds,
        "rosebud;!;! must add $3,000"
    );
    assert_eq!(
        game.lot().world().globals.get(14).copied(),
        Some((expected_funds / 10_000) as i16),
        "split funds high global[14] must match World::set_funds"
    );
    assert_eq!(
        game.lot().world().globals.get(13).copied(),
        Some((expected_funds % 10_000) as i16),
        "split funds low global[13] must match World::set_funds"
    );

    // Test set_hour (including exact VM period boundaries & zeroed seconds),
    // move_objects, map_edit, autonomy, sim_speed, water_tool, prepare_lot, help.
    for ch in "set_hour 6".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game.cheat_console_submit().expect("set_hour 6 parses");
    assert_eq!(game.hour(), 6);
    assert_eq!(game.lot().world().globals.get(4).copied(), Some(3)); // dawn
    assert_eq!(game.lot().world().globals.get(6).copied(), Some(0)); // seconds cleared

    for ch in "set_hour 18".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game.cheat_console_submit().expect("set_hour 18 parses");
    assert_eq!(game.hour(), 18);
    assert_eq!(game.lot().world().globals.get(4).copied(), Some(1)); // dusk

    for ch in "set_hour 22; move_objects on; map_edit on; autonomy 0; sim_speed 800".chars() {
        assert!(game.cheat_console_char(ch));
    }
    let _ = game.cheat_console_submit().expect("chained cheats parse");
    assert_eq!(game.hour(), 22);
    assert_eq!(game.lot().world().globals.get(4).copied(), Some(2));
    assert_eq!(game.lot().world().globals.get(6).copied(), Some(0));
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

    // 2. Native SimAntics HIT TV play & turn-off routing (`tv_action_cheap`, `tv_turn_off`).
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 77,
        name: "tv_action_cheap".into(),
    }]));
    assert_eq!(mixer.radio().active_emitters(), 2);
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 77,
        name: "tv_turn_off".into(),
    }]));
    assert_eq!(mixer.radio().active_emitters(), 1);

    // 3. Activate environmental lot ambience bed and verify 1..=3 zoom mapping, zone, hour, pause.
    assert_eq!(
        sims_game::sound::zoom_level_from_u32(1),
        sims_audio::ambience::ZoomLevel::Far
    );
    assert_eq!(
        sims_game::sound::zoom_level_from_u32(2),
        sims_audio::ambience::ZoomLevel::Medium
    );
    assert_eq!(
        sims_game::sound::zoom_level_from_u32(3),
        sims_audio::ambience::ZoomLevel::Near
    );
    mixer.input(Input::Zoom(3));
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

/// Sets the open lot's pending indicator refreshes and star pools, through
/// a save of the lot and its restore.
fn set_open_queues(shell: &mut NeighborhoodShell, queues: sims_sim::IndicatorQueues) {
    let game = shell.game_mut().expect("lot game");
    let mut saved = game.save().expect("save the open lot");
    saved.set_indicator_queues(queues);
    game.lot_mut().restore(saved).expect("restore the open lot");
}

/// A `WorldSnapshot` slot saved before the indicator queues were kept (the
/// world's encoding with nothing after it) restores with its star pools
/// taken from where its stars stand, as a `Saved` of that age does, not
/// with both pools empty: otherwise the next famous Sim would make four new
/// stars while one star is held and three are free.
#[test]
fn an_old_world_snapshot_slot_adopts_its_star_pools() {
    const STAR_MARKER: u32 = 0xE933_7020;
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping the old snapshot star pools test");
        return;
    };
    let tmp = TempTestDir::new("old-snapshot-pools");
    let saves = tmp.path().join("saves.redb");
    let mut shell = NeighborhoodShell::open(install, saves, 1, false, vec![3], Frames::default())
        .expect("NeighborhoodShell opens");
    shell.enter_lot(5).expect("enter Goth lot 5");
    let game = shell.game().expect("lot game");
    let mut world = game.lot().world().clone();
    let tick = u64::from(game.lot().ticks());
    let person = world.people[0];

    // Four stars: one held by a Sim, three free off the lot.
    let template = world
        .order
        .iter()
        .map(|id| &world.objects[id])
        .find(|o| o.person.is_none() && o.multipart.is_none() && o.data.len() > 2)
        .expect("an object to copy")
        .clone();
    let first = world.objects.keys().max().unwrap() + 1;
    let stars: Vec<u16> = (first..first + 4).collect();
    for (n, &id) in stars.iter().enumerate() {
        let mut star = template.clone();
        star.id = id;
        star.guid = STAR_MARKER;
        star.position = [-16, -16, 1];
        star.slots.truncate(1);
        star.data[2] = if n == 0 { person as i16 } else { 0 };
        world.order.push(id);
        world.objects.insert(id, star);
    }
    let expected = sims_vm::boot::save::queues_saved_before_kept(&world);
    assert_eq!(expected.stars_in_use, [stars[0]]);
    assert_eq!(expected.available_stars, [stars[3], stars[2], stars[1]]);

    shell
        .store()
        .autosaves()
        .commit_job(SaveJob::manual_for_instance(
            shell.neighborhood_id(),
            shell.stored_neighborhood().instance_id,
            5,
            tick,
            SavePayload::world_bytes(world.encode()),
        ))
        .expect("save the old-style slot");

    assert!(shell.open_save_manager(None));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert!(
        !shell.save_manager_open(),
        "loading the WorldSnapshot slot must succeed"
    );
    assert_eq!(
        shell
            .game()
            .expect("lot game")
            .lot()
            .world()
            .indicator_queues,
        expected,
        "the pools from where the slot's stars stand"
    );
}

/// Loading a `WorldSnapshot` slot through the Save Manager restores the
/// slot's own indicator refreshes and star pools, not the open lot's: the
/// restore splices the snapshot's world into the open lot's `Saved`, whose
/// queues may name people the restored world lacks.
#[test]
fn a_world_snapshot_slot_restores_its_own_indicator_queues() {
    let Some(install) = install_dir() else {
        eprintln!("SIMS_INSTALL is not set; skipping the snapshot queues restore test");
        return;
    };
    let tmp = TempTestDir::new("snapshot-queues");
    let saves = tmp.path().join("saves.redb");
    let mut shell = NeighborhoodShell::open(install, saves, 1, false, vec![3], Frames::default())
        .expect("NeighborhoodShell opens");
    shell.enter_lot(5).expect("enter Goth lot 5");
    let people = shell.game().expect("lot game").lot().world().people.clone();
    let [first, second, ..] = people[..] else {
        panic!("the Goth house has two people: {people:?}");
    };

    // The slot: the world with `saved` pending.
    let mut saved = sims_sim::IndicatorQueues::default();
    saved.queue_date(first, 1);
    saved.queue_star(first);
    set_open_queues(&mut shell, saved.clone());
    assert_eq!(shell.game().unwrap().lot().world().indicator_queues, saved);
    shell
        .enqueue_background_world_save(SaveTarget::Manual)
        .expect("enqueue a WorldSnapshot manual save");
    shell.flush_background_saves();
    assert_eq!(shell.pending_background_saves(), 0);

    // The open lot then has others pending.
    let mut open = sims_sim::IndicatorQueues::default();
    open.queue_star(second);
    open.queue_star(second);
    set_open_queues(&mut shell, open.clone());
    assert_ne!(open, saved);

    assert!(shell.open_save_manager(None));
    let _ = shell.handle_save_manager_command(SaveManagerCommand::LoadSlot {
        slot_id: "manual".into(),
    });
    assert!(
        !shell.save_manager_open(),
        "loading the WorldSnapshot slot must succeed"
    );
    assert_eq!(
        shell
            .game()
            .expect("lot game")
            .lot()
            .world()
            .indicator_queues,
        saved,
        "the slot's queues, not the open lot's"
    );
}
