//! Parity and performance benchmark tests for multi-threaded horizontal-band
//! software rasterization and Neighborhood Map rendering.

use sims_render::neighborhood::{
    HoverFadeState, NeighborhoodKind, NeighborhoodMap, RenderOptions as NbRenderOptions,
};
use sims_render::scene::{RenderOptions, draw_lot_with_band_count};
use sims_render::{
    Camera, Canvas, Focus, Lot, LotCache, Walls, Zoom, draw_lot_with_preview, open_house,
};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("skipping: set SIMS_INSTALL to run House05 / Neighborhood tests");
            None
        }
    }
}

fn assert_canvases_bit_exact(label: &str, a: &Canvas, b: &Canvas, object_ids: &[u16]) {
    assert_eq!(a.width, b.width, "{label}: width mismatch");
    assert_eq!(a.height, b.height, "{label}: height mismatch");
    let color_diffs = a.color.iter().zip(&b.color).filter(|(p, q)| p != q).count();
    assert_eq!(
        color_diffs, 0,
        "{label}: {color_diffs} differing color pixels"
    );
    let depth_diffs = a.depth.iter().zip(&b.depth).filter(|(p, q)| p != q).count();
    assert_eq!(
        depth_diffs, 0,
        "{label}: {depth_diffs} differing depth pixels"
    );
    assert_eq!(a.ids(), b.ids(), "{label}: differing pixel IDs");
    for &id in object_ids {
        assert_eq!(
            a.drawn(id),
            b.drawn(id),
            "{label}: differing drawn rect for object {id}"
        );
    }
}

#[test]
fn single_thread_and_multi_band_render_bit_for_bit_identically() {
    let Some(root) = install() else {
        return;
    };

    // Exercise Goth House (House05: multi-level, furniture, walls, shadows,
    // cutaway/up/down, day/night, terrain grid), House02 (swimming pool +
    // coping + inner corners), and House08 (pond water overlay).
    type ViewConfig = (Zoom, u8, i32, Walls, bool, i16);
    let house_cases: &[(&str, &[ViewConfig])] = &[
        (
            "UserData/Houses/House05.iff",
            &[
                (Zoom::Far, 0, 1, Walls::Cutaway, false, 12),
                (Zoom::Medium, 0, 1, Walls::Cutaway, false, 12),
                (Zoom::Medium, 1, 2, Walls::Up, false, 12),
                (Zoom::Medium, 2, 1, Walls::Down, true, 22),
                (Zoom::Near, 3, 2, Walls::Cutaway, false, 12),
            ],
        ),
        (
            "UserData/Houses/House02.iff",
            &[(Zoom::Medium, 0, 1, Walls::Cutaway, false, 12)],
        ),
        (
            "UserData/Houses/House08.iff",
            &[(Zoom::Medium, 1, 1, Walls::Cutaway, false, 12)],
        ),
    ];

    for &(rel, configs) in house_cases {
        let house = root.join(rel);
        if !house.exists() {
            continue;
        }
        let mut opened = open_house(&root, &house).expect("open house");
        let lot = Lot::from_world(&opened.world)
            .expect("Lot::from_world")
            .with_floor_rotation(opened.rotation);
        let object_ids: Vec<u16> = opened.world.objects.keys().copied().collect();

        let mut single = Canvas::new(1024, 768, [0, 0, 0]);
        let mut bands4 = Canvas::new(1024, 768, [0, 0, 0]);
        let mut bands8 = Canvas::new(1024, 768, [0, 0, 0]);

        for &(zoom, rotation, level, walls, shadows, hour) in configs {
            if opened.world.globals.len() > 4 {
                opened.world.globals[4] = hour;
            }
            let mut camera = Camera {
                zoom,
                rotation,
                level,
                origin: [0, 0],
                walls,
                focus: Focus::default(),
            };
            camera.center_on([22 * 16, 26 * 16], [512, 384]);
            let opts = RenderOptions {
                shadows,
                lighting: true,
            };

            let stats1 = draw_lot_with_band_count(
                &opened.world,
                &lot,
                &mut opened.assets,
                &camera,
                (1024, 768),
                None,
                opts,
                &mut single,
                1,
            );
            let stats4 = draw_lot_with_band_count(
                &opened.world,
                &lot,
                &mut opened.assets,
                &camera,
                (1024, 768),
                None,
                opts,
                &mut bands4,
                4,
            );
            let stats8 = draw_lot_with_band_count(
                &opened.world,
                &lot,
                &mut opened.assets,
                &camera,
                (1024, 768),
                None,
                opts,
                &mut bands8,
                8,
            );

            let tag =
                format!("{rel} {zoom:?} r{rotation} L{level} {walls:?} shadows={shadows} h={hour}");
            assert_eq!(stats1, stats4, "{tag}: stats mismatch 1 vs 4");
            assert_eq!(stats1, stats8, "{tag}: stats mismatch 1 vs 8");
            assert_canvases_bit_exact(
                &format!("{tag} (1 vs 4 bands)"),
                &single,
                &bands4,
                &object_ids,
            );
            assert_canvases_bit_exact(
                &format!("{tag} (1 vs 8 bands)"),
                &single,
                &bands8,
                &object_ids,
            );
        }
    }
}

#[test]
#[ignore]
#[allow(clippy::disallowed_methods)]
fn bench_render_and_neighborhood() {
    let Some(root) = install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = open_house(&root, &house).expect("open House05");
    let lot = Lot::from_world(&opened.world)
        .expect("Lot::from_world")
        .with_floor_rotation(opened.rotation);
    let world = &opened.world;
    let assets = &mut opened.assets;

    let mut canvas = Canvas::new(1024, 768, [0, 0, 0]);
    for (label, zoom, level, walls) in [
        ("1024x768 Far L1 Cutaway", Zoom::Far, 1, Walls::Cutaway),
        ("1024x768 Med L1 Cutaway", Zoom::Medium, 1, Walls::Cutaway),
        ("1024x768 Near L1 Cutaway", Zoom::Near, 1, Walls::Cutaway),
        ("1024x768 Med L2 Up", Zoom::Medium, 2, Walls::Up),
    ] {
        let mut camera = Camera {
            zoom,
            rotation: 0,
            level,
            origin: [0, 0],
            walls,
            focus: Focus::default(),
        };
        camera.center_on([22 * 16, 26 * 16], [512, 384]);
        // Warm up assets + caches first
        let mut cache = LotCache::new();
        for _ in 0..4 {
            camera.origin[0] += 8;
            cache.draw_lot_with_preview(
                world,
                &lot,
                assets,
                &camera,
                (1024, 768),
                None,
                &mut canvas,
            );
        }
        // Measure uncached pan frames (camera.origin shifts each frame, Grass cached in LotCache)
        let iters = 20usize;
        let t0 = std::time::Instant::now();
        for i in 0..iters {
            camera.origin[0] += if i % 2 == 0 { 4 } else { -4 };
            cache.draw_lot_with_preview(
                world,
                &lot,
                assets,
                &camera,
                (1024, 768),
                None,
                &mut canvas,
            );
        }
        let elapsed = t0.elapsed();
        let ms = elapsed.as_secs_f64() * 1000.0 / iters as f64;
        println!("BENCH_LOT {label}: {ms:.3} ms/frame ({iters} pan frames)");
    }

    // Also measure direct draw_lot_with_preview (cold Grass + full draw)
    let mut camera = Camera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 1,
        origin: [0, 0],
        walls: Walls::Cutaway,
        focus: Focus::default(),
    };
    camera.center_on([22 * 16, 26 * 16], [512, 384]);
    draw_lot_with_preview(world, &lot, assets, &camera, (1024, 768), None, &mut canvas);
    let iters = 15usize;
    let t0 = std::time::Instant::now();
    for _ in 0..iters {
        draw_lot_with_preview(world, &lot, assets, &camera, (1024, 768), None, &mut canvas);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("BENCH_LOT_DIRECT 1024x768 Med L1 Cutaway: {ms:.3} ms/frame ({iters} frames)");

    // Measure Neighborhood Map 1024x768 hover cross-fade + highlighted lot
    let nb = NeighborhoodMap::load(
        &sims_content::Install::detect(&root).unwrap(),
        1,
        NeighborhoodKind::Unleashed,
        false,
    )
    .expect("load Unleashed neighborhood");
    let mut fade = HoverFadeState::new();
    fade.set_hovered(Some(5), 0);
    let mut opts = NbRenderOptions {
        viewport: [1024, 768],
        water_frame: Some(1),
        highlight_hovered: true,
        ..NbRenderOptions::default()
    };
    let mut nb_canvas = Canvas::new(1024, 768, [0, 0, 0]);
    // Warm up
    fade.apply_to(&mut opts, 50);
    nb.render_into(&mut nb_canvas, &opts);
    let iters = 30usize;
    let t0 = std::time::Instant::now();
    for i in 0..iters {
        let now_ms = 10 + (i as u64 * 9) % 280;
        fade.apply_to(&mut opts, now_ms);
        nb.render_into(&mut nb_canvas, &opts);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("BENCH_NB 1024x768 HoverFade+Highlight: {ms:.3} ms/frame ({iters} frames)");

    opts.highlight_hovered = false;
    fade.apply_to(&mut opts, 50);
    nb.render_into(&mut nb_canvas, &opts);
    let t0 = std::time::Instant::now();
    for i in 0..iters {
        let now_ms = 10 + (i as u64 * 9) % 280;
        fade.apply_to(&mut opts, now_ms);
        nb.render_into(&mut nb_canvas, &opts);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("BENCH_NB 1024x768 HoverFade: {ms:.3} ms/frame ({iters} frames)");
}
