//! Performance Part 4 baseline/after timing harness and correctness-neutrality
//! verification for Track 1 (`perf/assets-audio-and-app-loop`).
//!
//! Requires `SIMS_INSTALL` for full install benchmarks; skipped cleanly when unset.

use sims_audio::SoundEvent;
use sims_audio::player::{BLOCK, RATE};
use sims_content::{ArchivePool, Catalog, Index, Install};
use sims_game::autosave_worker::{
    AutosaveWorker, build_save_manager_snapshot, format_save_label, validate_save_payload_header,
};
use sims_game::game::{Game, InstallAssets};
use sims_game::neighborhood::{NeighborhoodShell, ScreenKind, WATER_STEP_US};
use sims_game::sound::{Feed, Input, Mixer};
use sims_game::view::Move;
use sims_sim::autosave::{SavePayloadKind, SaveTarget};
use sims_ui::command::{Mode, Speed};
use sims_vm::boot::Frames;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

struct TempTestDir(PathBuf);

impl TempTestDir {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("sims-game-perf-part4-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn validate_save_payload_header_rejects_short_and_bad_versions() {
    assert!(validate_save_payload_header(&SavePayloadKind::SavedLot, b"short").is_err());
    let mut bad_fmt = Vec::new();
    bad_fmt.extend_from_slice(&999u32.to_le_bytes());
    bad_fmt.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    bad_fmt.push(0);
    assert!(validate_save_payload_header(&SavePayloadKind::SavedLot, &bad_fmt).is_err());

    let mut bad_snap = Vec::new();
    bad_snap.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    bad_snap.extend_from_slice(&999_999u32.to_le_bytes());
    bad_snap.push(0);
    assert!(validate_save_payload_header(&SavePayloadKind::SavedLot, &bad_snap).is_err());

    let mut ok = Vec::new();
    ok.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    ok.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    ok.push(0);
    assert!(validate_save_payload_header(&SavePayloadKind::SavedLot, &ok).is_ok());
    // Full cached validation MUST reject valid-header / corrupt-body payloads
    // so Save Manager listings and overwrite/pruning safeguards always agree
    // with `load_save_validated`.
    assert!(
        sims_game::autosave_worker::validate_save_payload_cached(&SavePayloadKind::SavedLot, &ok)
            .is_err(),
        "valid-header corrupt-body SavedLot must be rejected by cached full validator"
    );
}

#[test]
fn autosave_worker_rejects_corrupt_body_bytes_and_tracks_unread_completions() {
    let tmp = TempTestDir::new("sol-r1-autosave");
    let store =
        sims_sim::NeighborhoodStore::open(&tmp.path().join("saves.redb")).expect("open redb store");
    let autosaves = store.autosaves();
    let worker = AutosaveWorker::spawn(autosaves.clone());

    let mut bogus_saved = Vec::new();
    bogus_saved.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    bogus_saved.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    bogus_saved.extend_from_slice(b"corrupt-body-bytes");

    worker
        .enqueue_saved_lot_bytes(
            "UserData",
            0,
            5,
            SaveTarget::Manual,
            1,
            format_save_label(Some("Goth"), 1, 8, 0),
            bogus_saved,
        )
        .expect("enqueue");
    worker.flush();
    assert!(
        worker.has_pending_or_unread(),
        "unread completion must keep wakeup active until drained"
    );
    let done = worker.poll_completions();
    assert_eq!(done.len(), 1);
    assert!(
        done[0].result.is_err(),
        "caller-supplied valid-header corrupt-body bytes must be rejected"
    );
    assert!(
        !worker.has_pending_or_unread(),
        "after draining completions with no pending jobs, worker must be idle"
    );
}

#[test]
#[allow(clippy::disallowed_methods)]
fn perf_part4_track1_measurements_and_parity() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("SIMS_INSTALL is not set; skipping perf_part4_track1_measurements_and_parity");
        return;
    };

    // 1. Catalog::build_with_pool (warm runs).
    let install = Install::detect(&root).expect("install detect");
    let index = Index::build(&install).expect("index build");
    let mut cat_ms = Vec::new();
    let mut cat_objects = 0usize;
    let mut cat_diags = 0usize;
    for _ in 0..3 {
        let pool = ArchivePool::new();
        let t0 = Instant::now();
        let cat = Catalog::build_with_pool(&index, &pool).expect("catalog build");
        cat_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        cat_objects = cat.objects.len();
        cat_diags = cat.diagnostics.len();
    }
    assert_eq!(cat_objects, 5289, "Catalog object count must be bit-exact");
    assert_eq!(cat_diags, 0, "Catalog diagnostics must stay zero");
    eprintln!(
        "BENCH Catalog::build_with_pool: {:?} ms (objects={cat_objects}, diags={cat_diags})",
        cat_ms
    );

    // 2. NeighborhoodShell::open (cold with REDB import + warm pre-imported).
    let tmp = TempTestDir::new("bench");
    let saves = tmp.path().join("saves.redb");
    let t_cold_nb = Instant::now();
    let shell_cold = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("cold NeighborhoodShell::open");
    let cold_nb_ms = t_cold_nb.elapsed().as_secs_f64() * 1000.0;
    drop(shell_cold);

    let t_warm_nb = Instant::now();
    let mut shell = NeighborhoodShell::open(
        root.clone(),
        saves.clone(),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("warm NeighborhoodShell::open");
    let warm_nb_ms = t_warm_nb.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "BENCH NeighborhoodShell::open: cold(with import)={cold_nb_ms:.2} ms, warm={warm_nb_ms:.2} ms"
    );

    // Item 3.1: check exact event-loop deadlines on Neighborhood & FamilyBin.
    let nb_wake = shell.next_wake_us(false, 16_667);
    if let Some(us) = nb_wake {
        assert!(
            (100_000..=WATER_STEP_US).contains(&us),
            "idle neighborhood with water should wake near 200ms WATER_STEP_US, got {us} us"
        );
    }

    // 3. First lot entry + SaveManager commit/list + warm lot re-entry.
    let t_enter1 = Instant::now();
    shell.enter_lot(5).expect("enter lot 5 first");
    let enter1_ms = t_enter1.elapsed().as_secs_f64() * 1000.0;

    // Item 3.1: check Lot deadlines (Normal 42ms, Fast 14ms, Ultra 2ms, Options idle).
    {
        let game = shell.game_mut().expect("game");
        game.set_speed(Speed::Normal);
        let wake_normal = game.next_wake_us(false, 16_667).expect("live wake");
        assert_eq!(wake_normal, 42_000, "Normal speed wake must be 42ms");
        game.set_speed(Speed::Fast);
        let wake_fast = game.next_wake_us(false, 16_667).expect("fast wake");
        assert_eq!(wake_fast, 14_000, "Fast speed wake must be 14ms");
        game.set_speed(Speed::Ultra);
        let wake_ultra = game.next_wake_us(false, 16_667).expect("ultra wake");
        assert_eq!(wake_ultra, 2_000, "Ultra speed wake must be 2ms");
        game.set_speed(Speed::Normal);

        // Sol Round 1 P1: plumb-bob deadline must never busy-spin at 0 us when
        // the window cannot draw or when a frame is already dirty.
        game.set_mode(Mode::Options);
        assert_eq!(
            game.next_wake_us_with_window(false, 16_667, false, false),
            None,
            "zero-sized/undrawable window in Options mode must not schedule presentation wakes"
        );
        let _ = game.advance(200_000); // push clock_us past SPIN_FRAME_US without drawing
        if let Some(us) = game.next_wake_us_with_window(false, 16_667, true, true) {
            assert!(
                us > 0,
                "dirty frame with overdue plumb bob must not emit a 0 us busy-spin deadline"
            );
        }
        game.set_mode(Mode::Live);

        // Item 3.2: Feed::update dirty-gating skips duplicate Snapshot rebuilds
        // on idle wakeups and updates screen on pure camera pan.
        let mut feed = Feed::default();
        let mut first_sources = 0usize;
        feed.update(game, |input| {
            if matches!(input, Input::Sources(_)) {
                first_sources += 1;
            }
        });
        assert_eq!(first_sources, 1, "initial Feed::update must send Sources");

        let mut idle_sources = 0usize;
        for _ in 0..10 {
            feed.update(game, |input| {
                if matches!(input, Input::Sources(_)) {
                    idle_sources += 1;
                }
            });
        }
        assert_eq!(
            idle_sources, 0,
            "repeated non-tick wakeups must not resend Sources"
        );

        assert!(
            game.apply(Move::Scroll([32, 0])),
            "camera scroll must change camera"
        );
        let mut pan_sources = None;
        feed.update(game, |input| {
            if let Input::Sources(snap) = input {
                pan_sources = Some(snap);
            }
        });
        let pan_snap = pan_sources.expect("camera pan must send updated Sources");
        assert_eq!(
            *pan_snap,
            game.sound_sources(),
            "camera-pan incremental Snapshot must match full game.sound_sources() bit-for-bit"
        );

        // Item 3.3: HUD draw with selected_override must be pixel-identical to cloning World.
        game.resize([800, 600]);
        game.set_mode(Mode::Live);
        let alt_sim = game
            .lot()
            .world()
            .objects
            .values()
            .find(|o| o.person.is_some() && !o.selected)
            .map(|o| o.id);
        if let Some(alt_id) = alt_sim {
            let mut cloned_world = game.lot().world().clone();
            for obj in cloned_world.objects.values_mut() {
                if obj.person.is_some() {
                    obj.selected = obj.id == alt_id;
                }
            }
            assert_ne!(cloned_world.selected(), game.lot().world().selected());
            let view = game.view;
            let pace = sims_game::pace::Pace::default();
            let (world, _, assets) = game.render_parts_mut();
            let members: Vec<u32> = world
                .objects
                .values()
                .filter_map(|o| o.person.is_some().then_some(o.guid))
                .collect();
            let mut hud_a =
                sims_game::hud::Hud::open_with_members(&root, members.clone(), assets, world, 50)
                    .expect("hud_a");
            let mut hud_b =
                sims_game::hud::Hud::open_with_members(&root, members, assets, world, 50)
                    .expect("hud_b");
            let mut canvas_override = sims_render::Canvas::new(800, 600, [0, 0, 0]);
            let mut canvas_cloned = sims_render::Canvas::new(800, 600, [0, 0, 0]);
            let vis = sims_ui::options::LotVisibility::default();
            hud_a.draw_for_lot_at_with_selection(
                world,
                true,
                vis,
                assets,
                &view,
                &pace,
                None,
                None,
                0,
                Some(Some(alt_id)),
                &mut canvas_override,
            );
            hud_b.draw_for_lot_at(
                &cloned_world,
                true,
                vis,
                assets,
                &view,
                &pace,
                None,
                None,
                0,
                &mut canvas_cloned,
            );
            assert_eq!(
                canvas_override.color, canvas_cloned.color,
                "HUD draw with selected_override must be 100% pixel-identical to cloned World"
            );
        }
    }

    // Seed 1 manual + 5 rotating autosaves for Save Manager benchmark.
    let saved = shell.game().expect("game").save().expect("save");
    let instance_id = shell
        .store()
        .load_neighborhood("UserData")
        .unwrap()
        .unwrap()
        .instance_id;
    let worker = AutosaveWorker::spawn(shell.store().autosaves());
    let t_commit = Instant::now();
    worker
        .enqueue_saved_lot(
            "UserData",
            instance_id,
            5,
            SaveTarget::ManualForceReplace,
            10,
            format_save_label(Some("Goth"), 1, 8, 0),
            saved.clone(),
        )
        .expect("enqueue manual");
    for i in 1..=5u64 {
        worker
            .enqueue_saved_lot(
                "UserData",
                instance_id,
                5,
                SaveTarget::Autosave,
                10 + i * 20,
                format_save_label(Some("Goth"), 1, 8, (i * 5) as u8),
                saved.clone(),
            )
            .expect("enqueue autosave");
    }
    worker.flush();
    let commit_6_ms = t_commit.elapsed().as_secs_f64() * 1000.0;
    let completions = worker.poll_completions();
    assert_eq!(completions.len(), 6);
    for c in &completions {
        assert!(c.result.is_ok(), "save failed: {:?}", c.result);
    }
    drop(worker);

    let autosaves = shell.store().autosaves();
    let mut list_ms = Vec::new();
    for _ in 0..5 {
        let t0 = Instant::now();
        let snap = build_save_manager_snapshot(&autosaves, "UserData", 1, 5, "5 Sim Lane", None);
        list_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(snap.slots.len(), 6);
    }
    eprintln!("BENCH SaveManager: commit_6_slots={commit_6_ms:.2} ms, list_6_slots={list_ms:?} ms");

    shell.leave_lot(false).expect("leave lot 5");
    assert_eq!(shell.screen(), ScreenKind::Neighborhood);

    let t_enter2 = Instant::now();
    shell.enter_lot(5).expect("enter lot 5 warm");
    let enter2_ms = t_enter2.elapsed().as_secs_f64() * 1000.0;
    shell.leave_lot(false).expect("leave lot 5 again");

    let t_enter7 = Instant::now();
    shell.enter_lot(7).expect("enter lot 7 warm");
    let enter7_ms = t_enter7.elapsed().as_secs_f64() * 1000.0;
    shell.leave_lot(false).expect("leave lot 7");
    eprintln!(
        "BENCH Lot entry (incl. sims-vm enter_lot): first(lot5)={enter1_ms:.2} ms, warm(lot5)={enter2_ms:.2} ms, warm(lot7)={enter7_ms:.2} ms"
    );

    // Measure pure sims-game lot assembly (`from_loaded_lot` uncached vs `InstallAssets` cached).
    let lot_a =
        sims_vm::lifecycle::enter_lot(&root, shell.store(), "UserData", 5, &[3], Frames::default())
            .expect("vm enter_lot a");
    let lot_b =
        sims_vm::lifecycle::enter_lot(&root, shell.store(), "UserData", 5, &[3], Frames::default())
            .expect("vm enter_lot b");
    let snap = shell
        .store()
        .load_lot_snapshot("UserData", 5)
        .unwrap()
        .unwrap();
    let members = snap
        .neighborhood
        .family_in(5)
        .map(|(_, f)| f.members.clone())
        .unwrap_or_default();
    let t_uncached = Instant::now();
    let _g1 = Game::from_loaded_lot(
        &root,
        lot_a,
        members.clone(),
        &snap.character_files,
        snap.base_house.as_deref(),
        true,
    )
    .expect("uncached from_loaded_lot");
    let uncached_wrap_ms = t_uncached.elapsed().as_secs_f64() * 1000.0;

    let cached_assets = Arc::new(
        InstallAssets::load(&install, Arc::new(ArchivePool::new()), None, None)
            .expect("InstallAssets::load"),
    );
    let t_cached = Instant::now();
    let _g2 = Game::from_loaded_lot_cached(
        &root,
        lot_b,
        members,
        &snap.character_files,
        snap.base_house.as_deref(),
        true,
        Some(&cached_assets),
    )
    .expect("cached from_loaded_lot");
    let cached_wrap_ms = t_cached.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "BENCH Game::from_loaded_lot (sims-game asset wrap): uncached={uncached_wrap_ms:.2} ms, cached(InstallAssets)={cached_wrap_ms:.2} ms"
    );

    // 4. Audio Mixer::open & render_into parity + timing.
    let mut mixer_open_ms = Vec::new();
    for _ in 0..3 {
        let t0 = Instant::now();
        let _m = Mixer::open(&root).expect("Mixer::open");
        mixer_open_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    let mut mixer_alloc = Mixer::open(&root).expect("Mixer::open");
    let mut mixer_into = Mixer::open(&root).expect("Mixer::open");
    let ring = Input::Events(vec![SoundEvent::Play {
        object: 1,
        name: "Phone_ring".into(),
    }]);
    mixer_alloc.input(ring.clone());
    mixer_into.input(ring);
    let mut scratch = Vec::with_capacity(2 * BLOCK);
    let initial_ptr = scratch.as_ptr();
    let initial_cap = scratch.capacity();
    let t_render = Instant::now();
    for _ in 0..500 {
        let expected = mixer_alloc.render(BLOCK);
        mixer_into.render_into(BLOCK, &mut scratch);
        assert_eq!(
            scratch, expected,
            "Mixer::render_into must be bit-identical to Mixer::render"
        );
    }
    let render_500_ms = t_render.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(
        scratch.as_ptr(),
        initial_ptr,
        "render_into must not reallocate its output buffer"
    );
    assert_eq!(scratch.capacity(), initial_cap);
    let _ = RATE;
    eprintln!(
        "BENCH Audio: Mixer::open={mixer_open_ms:?} ms, 500x dual render+compare={render_500_ms:.2} ms (0 output buffer reallocs)"
    );
}
