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
    let autosaves = sims_sim::AutosaveStore::open(&tmp.path().join("autosaves.redb"))
        .expect("open standalone autosave store");
    let worker = AutosaveWorker::spawn(autosaves);

    let mut bogus_saved = Vec::new();
    bogus_saved.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    bogus_saved.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    bogus_saved.extend_from_slice(b"corrupt-body-bytes");

    worker
        .enqueue_job(sims_sim::autosave::SaveJob::manual(
            "UserData",
            5,
            1,
            sims_sim::autosave::SavePayload::saved_lot(bogus_saved),
        ))
        .expect("enqueue");
    worker.flush();
    assert!(
        worker.has_pending_or_unread(),
        "unread completion must keep wakeup active until drained"
    );
    let done = worker.poll_completions();
    assert_eq!(done.len(), 1);
    assert!(
        matches!(
            done[0].result,
            Err(sims_sim::autosave::SaveError::DecodeFailed { .. })
        ),
        "caller-supplied valid-header corrupt-body bytes must fail with DecodeFailed, got {:?}",
        done[0].result
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
                Some(1),
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
    // Verify `enqueue_saved_lot_bytes` with a real neighborhood `instance_id`
    // rejects a valid-header / corrupt-body payload with `SaveError::DecodeFailed`
    // and never overwrites the existing intact manual save.
    let mut corrupt_body = Vec::new();
    corrupt_body.extend_from_slice(&sims_vm::boot::save::FORMAT.to_le_bytes());
    corrupt_body.extend_from_slice(&sims_sim::SNAPSHOT_FORMAT.to_le_bytes());
    corrupt_body.extend_from_slice(b"corrupt-body-payload");
    worker
        .enqueue_saved_lot_bytes(
            "UserData",
            instance_id,
            5,
            SaveTarget::ManualForceReplace,
            999,
            format_save_label(Some("Goth"), 2, 8, 0),
            corrupt_body,
        )
        .expect("enqueue corrupt bytes");
    worker.flush();
    let bad_done = worker.poll_completions();
    assert_eq!(bad_done.len(), 1);
    assert!(
        matches!(
            bad_done[0].result,
            Err(sims_sim::autosave::SaveError::DecodeFailed { .. })
        ),
        "enqueue_saved_lot_bytes must fail with DecodeFailed on corrupt body, got {:?}",
        bad_done[0].result
    );
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

#[test]
#[ignore]
#[allow(clippy::disallowed_methods)]
fn bench_wave_b_game_draw() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("SIMS_INSTALL is not set; skipping bench_wave_b_game_draw");
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut game = Game::open(&sims_game::Options {
        install: root,
        house,
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("open House05");
    game.resize([1024, 768]);
    game.set_mode(Mode::Live);
    // Warm up Tier-2 LotCache, People poses, HUD layout, and canvases.
    for _ in 0..3 {
        let _ = game.advance(50_000);
        let _ = game.render();
    }

    // 1. Steady-state Tier-2 Game::render (plumb-bob spin while paused, LotCache Tier-2 hit).
    let _ = game.render();
    let steady_iters = 60usize;
    let t0 = Instant::now();
    for _ in 0..steady_iters {
        let _ = game.advance(50_000);
        let c = game.render();
        std::hint::black_box(&c.color);
    }
    let steady_ms = t0.elapsed().as_secs_f64() * 1000.0 / steady_iters as f64;

    // 2. Uncached pan Game::render (camera shifts each frame -> LotCache Miss + People + HUD).
    let pan_iters = 20usize;
    let t1 = Instant::now();
    for i in 0..pan_iters {
        let dx = if i % 2 == 0 { 16 } else { -16 };
        let _ = game.apply(Move::Scroll([dx, 0]));
        let c = game.render();
        std::hint::black_box(&c.color);
    }
    let pan_ms = t1.elapsed().as_secs_f64() * 1000.0 / pan_iters as f64;

    println!(
        "BENCH_GAME_DRAW 1024x768 steady(Tier-2): {steady_ms:.3} ms/frame ({steady_iters} frames), uncached_pan: {pan_ms:.3} ms/frame ({pan_iters} frames)"
    );
}

#[test]
fn wave_b_picking_parity_and_steady_present_damage_completeness() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut game = Game::open(&sims_game::Options {
        install: root.clone(),
        house: house.clone(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("open House05");
    game.resize([1024, 768]);
    game.set_mode(Mode::Live);
    let _ = game.run(100);
    // First render populates LotCache (Miss -> Tier-2 stored); second render hits Tier-2.
    let _ = game.render();
    let _ = game.render();

    assert!(
        game.lot_depth_from_cache(),
        "cached draw must borrow lot depth directly from LotCache without cloning"
    );
    let sparse_bytes = game.people_ids_stored_pixels() * std::mem::size_of::<u16>();
    assert!(
        sparse_bytes < 40_000,
        "sparse people_ids should be <40 KB at 1024x768 (was 3.15 MB), got {sparse_bytes} bytes"
    );

    // Build an independent full-screen reference (cloned lot_depth + full-screen Ids::take).
    let (cw, ch) = (game.canvas().width, game.canvas().height);
    let viewport = game.house_viewport_size();
    let install = Install::detect(&root).expect("detect install");
    let mut index = Index::build(&install).expect("build index");
    let nbhd_dir = house.parent().and_then(|p| p.parent()).expect("nbhd dir");
    let nbhd = sims_content::Neighborhood::open(nbhd_dir).expect("open neighborhood");
    if let Some(chars) = nbhd.characters().expect("characters") {
        index.add_characters(&chars).expect("add characters");
    }
    let catalog = Catalog::build(&index).expect("build catalog");
    let mut ref_assets = sims_render::Assets::new(&install, index, catalog).expect("render assets");
    let mut ref_people = sims_game::people::People::open(&root, &house).expect("open people");
    let world = game.lot().world();
    ref_people.refresh(world);
    let lot_shape = sims_render::Lot::from_world(world)
        .expect("lot shape")
        .with_floor_rotation(game.lot().floor_rotation());
    let cam = game.view.camera;
    let render_options = sims_render::RenderOptions {
        shadows: game.options().graphics.shadows,
        lighting: game.options().graphics.lighting,
    };
    let mut ref_canvas = sims_render::Canvas::new(cw, ch, [0, 0, 0]);
    sims_render::draw_lot_with_preview_and_options(
        world,
        &lot_shape,
        &mut ref_assets,
        &cam,
        viewport,
        None,
        render_options,
        &mut ref_canvas,
    );
    let ref_lot_depth = ref_canvas.depth.clone();
    assert_eq!(
        game.lot_depth_slice(),
        ref_lot_depth.as_slice(),
        "borrowed LotCache depth must be bit-for-bit identical to full-screen lot_depth clone"
    );
    ref_people.set_clock(0);
    ref_people.draw_in_viewport(world, &cam, viewport, &mut ref_canvas);
    let ref_people_ids = ref_canvas.ids();
    sims_render::draw_held_with_options(
        world,
        &lot_shape,
        &mut ref_assets,
        &cam,
        render_options,
        &mut ref_canvas,
    );

    // Verify 100% identical `Game::under` results across:
    // 1. Every single pixel where a Sim or plumb bob is drawn (plus neighbors),
    // 2. Every 4th pixel across the full 1024×768 frame.
    let mut checked_people_pixels = 0usize;
    let mut checked_grid_pixels = 0usize;
    for y in 0..ch as i32 {
        for x in 0..cw as i32 {
            let is_noted = sims_game::pick::noted(world, &cam, &ref_people_ids, [x, y], |id| {
                ref_people.is_bob(world, id)
            })
            .is_some();
            if is_noted || (x % 4 == 0 && y % 4 == 0) {
                let got = game.under([x, y]);
                let want = sims_game::pick::at(
                    game.lot(),
                    &cam,
                    &ref_canvas,
                    &ref_lot_depth,
                    &ref_people_ids,
                    [x, y],
                    |id| ref_people.is_bob(world, id),
                );
                assert_eq!(
                    got, want,
                    "Game::under mismatch at ({x}, {y}) (is_noted={is_noted})"
                );
                if is_noted {
                    checked_people_pixels += 1;
                } else {
                    checked_grid_pixels += 1;
                }
            }
        }
    }
    assert!(
        checked_people_pixels > 500,
        "expected >500 Sim/bob pixels checked, got {checked_people_pixels}"
    );
    assert!(checked_grid_pixels > 40_000);

    // Verify steady-state present_damage completeness:
    // Pause and settle non-zero clock animations, then advance 34 ms (between
    // 42 ms normal-speed ticks, spinning the 30 FPS plumb bob without running a
    // tick), and assert EVERY differing pixel across all 786,432 pixels lies
    // inside the reported `present_damage` rects.
    if !game.paused() {
        game.toggle_pause();
    }
    let _ = game.advance(1_000);
    let _ = game.render();
    let _ = game.advance(1_000);
    let _ = game.render();
    // Restart pace so the next 34 ms step runs 0 simulation ticks while rotating the plumb bob.
    game.pace.restart();
    let before_color = game.canvas().color.clone();
    game.on_presented();
    let (ran, _) = game.advance_while(34_000, 34_000, |_| true);
    assert_eq!(ran, 0, "34 ms step must run 0 ticks at 42 ms/tick");
    let after_color = game.render().color.clone();
    let damage = game
        .peek_present_damage()
        .expect("steady plumb-bob spin frame should produce bounded present_damage");
    assert!(!damage.is_empty(), "damage list must cover dynamic rects");
    let mut diff_pixels = 0usize;
    for y in 0..ch {
        for x in 0..cw {
            let idx = y * cw + x;
            if before_color[idx] != after_color[idx] {
                diff_pixels += 1;
                let (xi, yi) = (x as i32, y as i32);
                let covered = damage
                    .iter()
                    .any(|&[x0, y0, x1, y1]| xi >= x0 && xi < x1 && yi >= y0 && yi < y1);
                assert!(
                    covered,
                    "changed pixel ({x}, {y}) not covered by present_damage {damage:?}"
                );
            }
        }
    }
    assert!(
        diff_pixels > 0,
        "expected spinning plumb bob to change pixels between frames"
    );
    let damage_area: i64 = damage
        .iter()
        .map(|&[x0, y0, x1, y1]| i64::from((x1 - x0).max(0)) * i64::from((y1 - y0).max(0)))
        .sum();
    assert!(
        damage_area < (cw * ch) as i64 / 4,
        "steady-state partial damage area ({damage_area} px) should be a small fraction of full screen"
    );
}
