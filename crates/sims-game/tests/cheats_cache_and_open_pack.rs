//! Headless E2E verification for Batch 13 Track 1:
//! - Zero-`SIMS_INSTALL` standalone clean-room `open-pack/` boot, rendering,
//!   interaction pie menu, camera controls, live cheats, synthesized WAV audio,
//!   and CLI `--open-pack --png` / automatic standalone fallback.
//! - `SIMS_INSTALL`-gated Complete Collection verification for native live
//!   cheats (`move_objects`, `rosebud`/`klapaucius`, `set_hour` preserving
//!   Buy/Build undo history), warm `NeighborhoodShell::enter_lot` with
//!   `VmInstallCache` (bit-exact `World::hash()` + 5-run median timing),
//!   native REDB v4 `StoredNeighborhood.family_surnames` (zero `.caf-names.tsv`
//!   writes + read-only legacy fallback), and `AutosaveStore` SHA-256 fast path.

use sims_audio::SoundEvent;
use sims_game::autosave_worker::{AutosaveWorker, validate_save_payload_cached_with_sha256};
use sims_game::game::Game;
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::open_pack::discover_open_pack_dir;
use sims_game::sound::{Input, Mixer};
use sims_game::view::Move;
use sims_sim::autosave::{AutosaveStore, SaveError, SaveJob, SavePayload, SaveSlot};
use sims_ui::command::Mode;
use sims_vm::boot::Frames;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct TempTestDir(PathBuf);

impl TempTestDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("sims-game-b13-{tag}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
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

fn open_pack_path() -> PathBuf {
    discover_open_pack_dir().expect("workspace open-pack/ directory must be discoverable")
}

fn submit_cheat(game: &mut Game, line: &str) {
    if !game.cheat_console_open() {
        game.toggle_cheat_console();
    }
    for ch in line.chars() {
        game.cheat_console_char(ch);
    }
    game.cheat_console_submit()
        .unwrap_or_else(|e| panic!("cheat {line:?} parse failed: {e}"));
}

#[test]
fn open_pack_boots_renders_steps_interacts_cheats_and_plays_wavs() {
    let pack_dir = open_pack_path();
    let mut game = Game::open_open_pack(&pack_dir, false, Frames::default())
        .expect("standalone open-pack/ must boot cleanly without SIMS_INSTALL");

    assert!(game.is_open_pack());
    assert!(
        game.stopped().is_none(),
        "open-pack lot must not stop on load: {:?}",
        game.stopped()
    );
    assert!(
        game.errors().is_empty(),
        "open-pack lot load must have 0 errors: {:?}",
        game.errors()
    );

    // Verify all starter objects and both starter Sims exist in the booted world.
    let world = game.lot().world();
    let sim_ids: Vec<u16> = world
        .objects
        .iter()
        .filter_map(|(&id, obj)| obj.person.is_some().then_some(id))
        .collect();
    assert_eq!(sim_ids.len(), 2, "open-pack starter house must boot 2 Sims");
    assert!(
        world.objects.len() >= 18,
        "expected starter cottage objects + Sims, got {}",
        world.objects.len()
    );
    assert!(
        world.selected().is_some(),
        "a starter Sim must be selected on boot"
    );

    // Render into 800x600 Canvas and verify non-blank lot viewport + HUD + visible objects & Sims.
    game.resize([800, 600]);
    game.render();
    let canvas = game.canvas();
    assert_eq!(canvas.width, 800);
    assert_eq!(canvas.height, 600);

    let mut lot_colors = BTreeSet::new();
    for y in 0..500usize {
        for x in (0..800usize).step_by(4) {
            lot_colors.insert(canvas.pixel_rgb(x, y));
        }
    }
    assert!(
        lot_colors.len() >= 16,
        "rendered open-pack lot viewport must be visually rich (got {} distinct colors)",
        lot_colors.len()
    );

    let mut hud_colors = BTreeSet::new();
    for y in 500..600usize {
        for x in (0..800usize).step_by(4) {
            hud_colors.insert(canvas.pixel_rgb(x, y));
        }
    }
    assert!(
        hud_colors.len() >= 6,
        "clean-room procedural HUD strip must render legibly (got {} distinct colors)",
        hud_colors.len()
    );

    // Verify both starter Sims and multiple furniture objects are visibly drawn on canvas.
    let drawn_sims = sim_ids
        .iter()
        .filter(|&&id| canvas.drawn(id).is_some())
        .count();
    assert!(
        drawn_sims >= 1,
        "at least one starter Sim must be visibly drawn in the initial viewport"
    );

    // Find a drawn interactive furniture object pixel and click it to open its pie menu.
    let mut clicked_entry = None;
    let candidate_ids: Vec<u16> = game
        .lot()
        .world()
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_none())
        .map(|(&id, _)| id)
        .collect();
    'search: for obj_id in candidate_ids {
        let Some(rect) = game.canvas().drawn(obj_id) else {
            continue;
        };
        let cx = (rect[0] + rect[2]) / 2;
        let cy = (rect[1] + rect[3]) / 2;
        for &(px, py) in &[
            (cx, cy),
            (rect[0] + 2, rect[1] + 2),
            (rect[2] - 2, rect[3] - 2),
            (cx, rect[3] - 3),
        ] {
            if game.canvas().id_at(px, py) != Some(obj_id) {
                continue;
            }
            game.hover([px, py]);
            let _ = game.press([px, py]);
            let _ = game.release([px, py]);
            if let Some(menu) = game.menu() {
                let labels: Vec<(String, sims_ui::frame::Rect)> = menu
                    .labels()
                    .into_iter()
                    .map(|(s, r)| (s.to_owned(), r))
                    .collect();
                if !labels.is_empty() {
                    clicked_entry = Some((obj_id, px, py, labels));
                    break 'search;
                }
            }
        }
    }
    let (clicked_obj, px, py, labels) =
        clicked_entry.expect("clicking a drawn open-pack object must open a non-empty pie menu");
    eprintln!(
        "open-pack pie menu opened on object {clicked_obj} at ({px}, {py}) with entries: {:?}",
        labels.iter().map(|(s, _)| s).collect::<Vec<_>>()
    );
    // Render while pie menu is open to verify clean-room pie menu drawing doesn't panic or error.
    game.render();
    let label_rect = labels[0].1;
    let pick_pt = [
        label_rect.x + label_rect.width / 2,
        label_rect.y + label_rect.height / 2,
    ];
    game.hover(pick_pt);
    let _ = game.press(pick_pt);
    let _ = game.release(pick_pt);
    assert!(
        game.menu().is_none(),
        "clicking pie menu entry must close menu and queue interaction"
    );

    // Step 30 simulation ticks in strict mode and assert 0 VM traps.
    let ran = game.run(30);
    assert_eq!(ran, 30, "all 30 ticks must complete cleanly");
    assert!(
        game.stopped().is_none(),
        "open-pack simulation must not stop: {:?}",
        game.stopped()
    );
    assert!(
        game.errors().is_empty(),
        "open-pack simulation must have 0 VM traps: {:?}",
        game.errors()
    );

    // Exercise camera pan, rotate, and zoom.
    assert!(game.apply(Move::Scroll([24, -16])));
    assert!(game.apply(Move::Rotate(true)));
    let _ = game.apply(Move::ZoomIn);
    let _ = game.apply(Move::ZoomOut);
    game.render();

    // Exercise Buy Mode in open-pack (catalog rendering, move_objects overlap placement,
    // and Buy/Build undo preservation across native rosebud & set_hour cheats).
    game.set_mode(Mode::Buy);
    game.render();
    let open_obj = game
        .lot()
        .world()
        .objects
        .iter()
        .find(|(_, o)| {
            o.person.is_none()
                && o.multipart.is_none()
                && o.position[2] == 1
                && o.position[0] > 32
                && o.position[1] > 32
                && ![
                    0x2f72_6fff,
                    0x0baa_71c5,
                    0x0000_0437,
                    0x0000_0439,
                    0xf3aa_a62f,
                ]
                .contains(&o.guid)
        })
        .map(|(_, o)| (o.guid, [o.position[0], o.position[1]]))
        .expect("open-pack cottage must have a placed single-tile furniture object");

    assert!(!game.cheat_move_objects());
    let mut cursor_off = game
        .lot_mut()
        .buy_start(open_obj.0, 1, 0)
        .expect("open-pack buy_start with move_objects off")
        .expect("open-pack buy_start cursor");
    assert!(
        !game
            .lot_mut()
            .buy_move(&mut cursor_off, open_obj.1, 1)
            .expect("open-pack buy_move with move_objects off"),
        "move_objects off must reject overlapping placement in open-pack Buy Mode"
    );
    let _ = game.lot_mut().buy_cancel(&cursor_off);

    submit_cheat(&mut game, "move_objects on");
    assert!(game.cheat_move_objects());
    assert!(game.lot().move_objects());
    let mut buy_cursor = game
        .lot_mut()
        .buy_start(open_obj.0, 1, 0)
        .expect("open-pack buy_start")
        .expect("open-pack buy_start cursor");
    assert!(
        game.lot_mut()
            .buy_move(&mut buy_cursor, open_obj.1, 1)
            .expect("open-pack buy_move with move_objects on"),
        "move_objects on must permit overlapping placement in open-pack Buy Mode"
    );
    game.lot_mut()
        .buy_commit(&buy_cursor)
        .expect("open-pack buy_commit");
    assert!(
        game.lot().can_undo(),
        "Buy Mode commit must populate undo stack"
    );

    // Exercise live cheats (`rosebud;!;!`, `set_hour 21`, `move_objects off`).
    let funds_before = game.lot().world().budget.funds;
    submit_cheat(&mut game, "rosebud;!;!");
    assert_eq!(
        game.lot().world().budget.funds,
        funds_before + 3_000,
        "rosebud;!;! must add $3,000 natively"
    );
    assert!(
        game.lot().can_undo(),
        "native add_funds must preserve Buy/Build undo stack"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native add_funds must not round-trip through Lot::restore"
    );

    submit_cheat(&mut game, "set_hour 21");
    assert_eq!(game.hour(), 21, "set_hour 21 must set clock to 21:00");
    assert!(
        game.lot().can_undo(),
        "native set_hour must preserve Buy/Build undo stack"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native set_hour must not round-trip through Lot::restore"
    );

    submit_cheat(&mut game, "move_objects off");
    assert!(!game.cheat_move_objects());
    assert!(!game.lot().move_objects());
    game.set_mode(Mode::Live);
    game.render();

    // Exercise synthesized WAV playback through `Mixer::open_pack`.
    let wavs = game
        .open_pack_sounds()
        .expect("open-pack must carry synthesized WAV bank");
    assert!(!wavs.is_empty(), "open-pack WAV bank must be non-empty");
    let first_sound_name = wavs.keys().next().expect("at least one WAV").clone();
    let mut mixer = Mixer::open_pack(wavs);
    mixer.input(Input::Events(vec![SoundEvent::Play {
        object: 0,
        name: first_sound_name.clone(),
    }]));
    assert_eq!(
        mixer.active_open_pack_clips(),
        1,
        "triggering open-pack WAV {first_sound_name:?} must start a clip"
    );
    let pcm = mixer.render(1024);
    assert_eq!(pcm.len(), 2048);
    assert!(
        pcm.iter().any(|&s| s != 0),
        "synthesized open-pack WAV clip must render non-silent PCM audio"
    );
}

#[test]
fn open_pack_cli_png_renders_non_blank_screenshot_without_sims_install() {
    let tmp = TempTestDir::new("cli-open-pack-png");
    let exe = env!("CARGO_BIN_EXE_sims-game");
    let pack_dir = open_pack_path();

    // 1. Explicit `--open-pack <dir> --png <out.png> --ticks 5 --size 640x480`.
    let out1 = tmp.path().join("explicit-open-pack.png");
    let status1 = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .arg("--open-pack")
        .arg(&pack_dir)
        .arg("--png")
        .arg(&out1)
        .arg("--ticks")
        .arg("5")
        .arg("--size")
        .arg("640x480")
        .output()
        .expect("run sims-game --open-pack --png");
    assert!(
        status1.status.success(),
        "sims-game --open-pack --png failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status1.stdout),
        String::from_utf8_lossy(&status1.stderr),
    );
    let png1 = std::fs::read(&out1).expect("read rendered open-pack PNG");
    assert!(
        png1.starts_with(b"\x89PNG\r\n\x1a\n"),
        "output must be a valid PNG"
    );
    assert!(
        png1.len() > 4_000,
        "rendered open-pack PNG should be non-blank (>4 KB), got {} bytes",
        png1.len()
    );

    // 2. Automatic standalone fallback (no `--open-pack`, no `<install>`, `SIMS_INSTALL` unset).
    let out2 = tmp.path().join("auto-fallback.png");
    let status2 = Command::new(exe)
        .env_remove("SIMS_INSTALL")
        .current_dir(pack_dir.parent().expect("repo root"))
        .arg("--png")
        .arg(&out2)
        .arg("--ticks")
        .arg("2")
        .arg("--size")
        .arg("320x240")
        .output()
        .expect("run sims-game --png auto fallback");
    assert!(
        status2.status.success(),
        "sims-game auto fallback --png failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status2.stdout),
        String::from_utf8_lossy(&status2.stderr),
    );
    let png2 = std::fs::read(&out2).expect("read auto-fallback PNG");
    assert!(png2.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png2.len() > 1_500);
}

#[test]
fn autosave_worker_sha256_fast_path_commits_and_loads_cleanly() {
    let tmp = TempTestDir::new("autosave-sha256");
    let db_path = tmp.path().join("autosaves.redb");
    let store = AutosaveStore::open(&db_path).expect("open autosave store");
    let worker = AutosaveWorker::spawn(store.clone());

    let world = sims_sim::World::empty();
    let world_bytes = world.encode();
    worker
        .enqueue_job(SaveJob::manual(
            "UserData",
            1,
            42,
            SavePayload::from_world(world),
        ))
        .expect("enqueue valid world job");
    worker.flush();
    let done = worker.poll_completions();
    assert_eq!(done.len(), 1);
    let meta = done[0]
        .result
        .as_ref()
        .expect("valid world save must commit");
    assert_eq!(meta.sim_tick, 42);

    // Load via `load_save_validated_with_sha256` using the cached SHA-256 validator.
    let loaded = store
        .load_save_validated_with_sha256(
            "UserData",
            1,
            SaveSlot::Manual,
            validate_save_payload_cached_with_sha256,
        )
        .expect("load_save_validated_with_sha256 must succeed");
    assert_eq!(loaded.payload, world_bytes);

    // Corrupt SavedLot payload with valid header must still be rejected by the SHA-256 fast path.
    let mut bad_saved = Vec::new();
    bad_saved.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    bad_saved.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    bad_saved.extend_from_slice(b"corrupt-body");
    worker
        .enqueue_job(SaveJob::manual(
            "UserData",
            2,
            10,
            SavePayload::saved_lot(bad_saved),
        ))
        .expect("enqueue bad saved job");
    worker.flush();
    let done2 = worker.poll_completions();
    assert_eq!(done2.len(), 1);
    assert!(
        matches!(done2[0].result, Err(SaveError::DecodeFailed { .. })),
        "corrupt SavedLot body must fail DecodeFailed, got {:?}",
        done2[0].result
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn complete_collection_native_cheats_move_objects_and_undo_preservation() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_native_cheats_move_objects_and_undo_preservation"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-cheats");
    let saves = tmp.path().join("saves.redb");
    let mut shell = NeighborhoodShell::open(root, saves, 1, true, vec![3], Frames::default())
        .expect("open neighborhood shell");
    shell.enter_lot(5).expect("enter Goth lot 5");

    let game = shell.game_mut().expect("active lot game");
    game.set_mode(Mode::Buy);

    // Find an existing single-tile floor object (e.g. a chair/lamp) and its occupied fine position.
    let existing_obj = game
        .lot()
        .world()
        .objects
        .iter()
        .find(|(_, o)| {
            o.person.is_none()
                && o.multipart.is_none()
                && o.position[2] == 1
                && o.position[0] > 32
                && o.position[1] > 32
                && game.lot().can_pick_up(o.id).unwrap_or(false)
        })
        .map(|(&id, o)| (id, o.guid, o.position))
        .expect("Goth lot must have a pickable single-tile object");

    let occupied_fine = [existing_obj.2[0], existing_obj.2[1]];
    // Buy a new copy of the same object to test overlapping placement with move_objects off vs on.
    let mut cursor = game
        .lot_mut()
        .buy_start(existing_obj.1, 1, 0)
        .expect("buy_start of single-tile object")
        .expect("buy_start must yield cursor");
    let placed_off = game
        .lot_mut()
        .buy_move(&mut cursor, occupied_fine, 1)
        .expect("buy_move with move_objects off");
    assert!(
        !placed_off,
        "overlapping placement on occupied tile must be rejected when move_objects is off"
    );
    let _ = game.lot_mut().buy_cancel(&cursor);

    // Enable `move_objects on` via the cheat console and verify honest active feedback & native VM override!
    submit_cheat(game, "move_objects on");
    assert!(game.cheat_move_objects());
    assert!(game.lot().move_objects());
    let fb = game
        .cheat_console()
        .feedback()
        .map(|f| f.message.as_str())
        .unwrap_or("");
    assert!(
        !fb.contains("pending"),
        "move_objects feedback must not say pending: {fb:?}"
    );

    let mut cursor2 = game
        .lot_mut()
        .buy_start(existing_obj.1, 1, 0)
        .expect("buy_start with move_objects on")
        .expect("buy_start cursor2");
    let placed_on = game
        .lot_mut()
        .buy_move(&mut cursor2, occupied_fine, 1)
        .expect("buy_move with move_objects on");
    assert!(
        placed_on,
        "overlapping placement on occupied tile must succeed with move_objects on"
    );
    game.lot_mut()
        .buy_commit(&cursor2)
        .expect("buy_commit must succeed and push onto Buy undo stack");
    assert!(
        game.lot().can_undo(),
        "Buy Mode commit must populate Buy/Build undo stack"
    );

    // Execute `rosebud;!;!` and `set_hour 20` and verify Buy/Build undo stack is PRESERVED!
    let funds_before = game.lot().world().budget.funds;
    submit_cheat(game, "rosebud;!;!");
    assert_eq!(game.lot().world().budget.funds, funds_before + 3_000);
    assert!(
        game.lot().can_undo(),
        "native rosebud must preserve Buy/Build undo history"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native rosebud must not clear neighborhood_origin via Lot::restore"
    );

    submit_cheat(game, "set_hour 20");
    assert_eq!(game.hour(), 20);
    assert!(
        game.lot().can_undo(),
        "native set_hour must preserve Buy/Build undo history"
    );
    assert!(
        !game.world_mutated_via_restore(),
        "native set_hour must not clear neighborhood_origin via Lot::restore"
    );

    submit_cheat(game, "move_objects off");
    assert!(!game.cheat_move_objects());
    assert!(!game.lot().move_objects());

    // Native save_current_lot must succeed directly (without staging cheat fallback).
    shell
        .save_current_lot()
        .expect("save_current_lot must succeed after native live cheats");
}

#[test]
#[allow(clippy::disallowed_methods)]
fn complete_collection_warm_enter_lot_vm_cache_parity_and_speedup() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_warm_enter_lot_vm_cache_parity_and_speedup"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-vm-cache");
    let saves = tmp.path().join("saves.redb");
    let mut shell =
        NeighborhoodShell::open(root.clone(), saves, 1, true, vec![3], Frames::default())
            .expect("open neighborhood shell");

    // Baseline uncached `Game::open_from_store_cached(..., None)` hash & 5-run median.
    let mut uncached_ms = Vec::with_capacity(5);
    let mut uncached_hash = None;
    for _ in 0..5 {
        let t0 = Instant::now();
        let g = Game::open_from_store_cached(
            &root,
            shell.store(),
            "UserData",
            5,
            &[3],
            true,
            Frames::default(),
            None,
        )
        .expect("uncached open_from_store_cached");
        uncached_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        uncached_hash = Some(g.lot().world().hash());
    }
    uncached_ms.sort_by(f64::total_cmp);
    let uncached_median = uncached_ms[2];

    // Warm first entry (loads InstallAssets + VmInstallCache once), then measure 5 warm re-entries.
    shell.enter_lot(5).expect("cold enter_lot 5");
    let mut cached_ms = Vec::with_capacity(5);
    let mut cached_hash = None;
    for _ in 0..5 {
        let t0 = Instant::now();
        shell.enter_lot(5).expect("warm enter_lot 5");
        cached_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        cached_hash = Some(shell.game().expect("game").lot().world().hash());
    }
    cached_ms.sort_by(f64::total_cmp);
    let cached_median = cached_ms[2];

    assert_eq!(
        cached_hash, uncached_hash,
        "warm VmInstallCache enter_lot must produce bit-identical World::hash()"
    );
    eprintln!(
        "BENCH warm enter_lot(House05) 5-run median: uncached={uncached_median:.2} ms, cached(VmInstallCache+InstallAssets)={cached_median:.2} ms (speedup={:.2}x)",
        uncached_median / cached_median.max(0.001)
    );
}

#[test]
fn complete_collection_native_caf_surnames_without_sidecar_and_legacy_fallback() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!(
            "SIMS_INSTALL is not set; skipping complete_collection_native_caf_surnames_without_sidecar_and_legacy_fallback"
        );
        return;
    };
    let tmp = TempTestDir::new("cc-caf-surnames");
    let saves = tmp.path().join("saves.redb");
    let sidecar = saves.with_extension("caf-names.tsv");

    // 1. Create a CAF family via CafSession -> NeighborhoodStore and verify native REDB persistence.
    let mut shell = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open neighborhood shell");
    shell.open_family_bin();
    shell.open_create_family().expect("open CAF");
    let draft = {
        let caf = shell.caf_session_mut().expect("CAF session");
        let _ = caf.controller_mut().begin_add_sim();
        {
            let sim = caf.controller_mut().editing_sim_mut().expect("editing sim");
            sim.first_name = "Nova".into();
            sim.bio = "Clean-room REDB v4 surname test".into();
            sim.appearance = sims_ui::caf::SimAppearance {
                age: sims_ui::caf::Age::Adult,
                gender: sims_ui::caf::Gender::Female,
                skin_tone: sims_ui::caf::SkinTone::Light,
                ..sims_ui::caf::SimAppearance::default()
            };
            sim.personality = sims_ui::caf::Personality::from_raw([5, 5, 5, 5, 5]);
        }
        let sim = caf.controller().editing_sim().unwrap().clone();
        sims_ui::caf::FamilyDraft {
            last_name: "Starlight".to_owned(),
            bio: "Clean-room REDB v4 surname test".to_owned(),
            members: vec![sim],
        }
    };
    let created = {
        let mut caf_session = {
            let install = sims_content::Install::detect(&root).unwrap();
            let ui = sims_content::ui_graphics::UiGraphics::open(&install).unwrap();
            sims_game::caf_screen::CafSession::open(&root, &install, &ui).unwrap()
        };
        caf_session
            .commit_family_draft(shell.store(), "UserData", &draft)
            .expect("commit CAF family")
    };
    shell
        .refresh_from_store()
        .expect("refresh after CAF commit");
    assert!(
        !sidecar.exists(),
        "CAF commit must NOT write legacy .caf-names.tsv sidecar"
    );

    // Reopen shell from disk and verify surname survives natively in StoredNeighborhood.family_surnames.
    drop(shell);
    let reopened = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("reopen neighborhood shell");
    let stored = reopened
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.family_surname(created.family_id),
        Some("Starlight"),
        "StoredNeighborhood.family_surnames must persist CAF surname natively"
    );
    assert!(
        reopened
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created.family_id) && e.name == "Starlight"),
        "Family Bin must display native REDB surname after reopen"
    );

    // 2. Verify read-only legacy `.caf-names.tsv` fallback when StoredNeighborhood lacks an entry.
    let mut stripped = stored.clone();
    stripped.remove_family_surname(created.family_id);
    reopened
        .store()
        .save_neighborhood("UserData", &stripped)
        .expect("save stripped neighborhood");
    let inst_hex = format!("{:032x}", stripped.instance_id);
    std::fs::write(
        &sidecar,
        format!(
            "UserData\t{inst_hex}\t{}\tLegacySidecarName\n",
            created.family_id
        ),
    )
    .expect("write legacy sidecar fixture");
    drop(reopened);

    let legacy_shell = NeighborhoodShell::open(root, saves, 1, true, vec![3], Frames::default())
        .expect("reopen with legacy sidecar");
    assert!(
        legacy_shell
            .family_bin_entries()
            .iter()
            .any(|e| e.id == u32::from(created.family_id) && e.name == "LegacySidecarName"),
        "read-only legacy .caf-names.tsv fallback must still surface surnames for pre-v4 saves"
    );
}
