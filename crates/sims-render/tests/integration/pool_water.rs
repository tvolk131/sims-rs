//! Integration tests for Complete Collection swimming pool (`Arry 9`,
//! `SPR2 0x400..=0x43b`) and pond/natural water (`Arry 10`,
//! `SPR2 0x800..=0x80f`) rendering against a real `SIMS_INSTALL`.

use crate::common::install;
use sims_formats::lot::Array;
use sims_render::{
    CacheHit, Camera, Canvas, Focus, Lot, LotCache, Walls, Zoom, depth::EMPTY, draw,
    draw_lot_in_viewport, lot::FULL, open_house,
};

/// Recomputes 4-bit cardinal connectivity masks (`1=N(-y), 2=W(-x), 4=S(+y), 8=E(+x)`)
/// for active (`< 16`) tiles in `buf`, matching `Sims.exe` `Arry 9` / `Arry 10`
/// and Build Mode Part 2's `recompute_water_pool_masks_around`.
fn recompute_masks(buf: &mut [u8], size: usize) {
    let active: Vec<bool> = buf.iter().map(|&v| v < 16).collect();
    for x in 1..size - 1 {
        for y in 1..size - 1 {
            let cell = Lot::cell(x, y);
            if !active[cell] {
                continue;
            }
            let mut m = 0u8;
            if active[Lot::cell(x, y - 1)] {
                m |= 1;
            }
            if active[Lot::cell(x - 1, y)] {
                m |= 2;
            }
            if active[Lot::cell(x, y + 1)] {
                m |= 4;
            }
            if active[Lot::cell(x + 1, y)] {
                m |= 8;
            }
            buf[cell] = m;
        }
    }
}

#[test]
fn house02_swimming_pool_renders_turquoise_water_coping_and_inner_corners() {
    let Some(install) = install() else {
        return;
    };
    let house = install
        .file("UserData/Houses/House02.iff")
        .expect("House02.iff exists");
    let mut opened = open_house(&install.root, &house).expect("open House02");
    let lot = Lot::from_world(&opened.world)
        .expect("build House02 Lot")
        .with_floor_rotation(opened.rotation);

    // Same camera as `h02-g.png` and `tests/edge_on_strip.rs`.
    let mut focus = Focus::default();
    focus.visit(2);
    let camera = Camera {
        zoom: Zoom::Far,
        rotation: 0,
        level: 1,
        origin: [511, -246],
        walls: Walls::Cutaway,
        focus,
    };
    let mut canvas = Canvas::new(800, 600, [0, 0, 0]);
    draw(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut canvas,
    );

    let pixels = canvas.rgb565();
    // Sample interior pool water pixels visible in `h02-g.png`:
    // h02-g.png has turquoise water RGB ~(90, 174, 189) (blue > green > red).
    for (x, y) in [(750, 140), (740, 120), (725, 130), (770, 150), (760, 180)] {
        let rgb = pixels[y * 800 + x];
        assert!(
            rgb[2] > rgb[1] && rgb[1] > rgb[0] && rgb[2] >= 160,
            "expected turquoise pool water at ({x}, {y}), got {rgb:?}"
        );
    }

    // Verify all 4 rotations × 3 zooms render House02's pool cleanly.
    for zoom in Zoom::ALL {
        for rotation in 0..4u8 {
            let mut cam = Camera {
                zoom,
                rotation,
                level: 1,
                origin: [0, 0],
                walls: Walls::Up,
                focus: Focus::default(),
            };
            cam.center_on([34 * 16 + 8, 16 * 16 + 8], [200, 150]);
            let mut c = Canvas::new(400, 300, [0, 0, 0]);
            draw(&opened.world, &lot, &mut opened.assets, &cam, &mut c);
            let turquoise = c
                .rgb565()
                .iter()
                .filter(|&&[r, g, b]| b > g && g > r && b >= 160)
                .count();
            assert!(
                turquoise > 200,
                "House02 pool should render turquoise pixels at {zoom:?} rot={rotation} (got {turquoise})"
            );
        }
    }
}

#[test]
fn house08_and_house28_pond_water_and_pool_deck_render_without_chroma_key_bleed() {
    let Some(install) = install() else {
        return;
    };
    for house_name in ["UserData/Houses/House08.iff", "UserData/Houses/House28.iff"] {
        let house = install
            .file(house_name)
            .unwrap_or_else(|e| panic!("{house_name} must exist in SIMS_INSTALL: {e}"));
        let mut opened = open_house(&install.root, &house).expect("open house");
        let lot = Lot::from_world(&opened.world)
            .expect("build Lot")
            .with_floor_rotation(opened.rotation);
        let water_tiles: Vec<[i32; 2]> = (1..lot.size - 1)
            .flat_map(|x| (1..lot.size - 1).map(move |y| [x as i32, y as i32]))
            .filter(|&[x, y]| lot.water[Lot::cell(x as usize, y as usize)] != 0xff)
            .collect();
        assert!(
            !water_tiles.is_empty(),
            "{house_name} should contain Arry 10 pond water tiles"
        );
        // Center on the first interior (mask == 15) pond tile so water fills the center.
        let focus_tile = water_tiles
            .iter()
            .copied()
            .find(|&[x, y]| lot.water[Lot::cell(x as usize, y as usize)] == 0x0f)
            .unwrap_or(water_tiles[0]);
        for zoom in Zoom::ALL {
            for rotation in 0..4u8 {
                let mut camera = Camera {
                    zoom,
                    rotation,
                    level: 1,
                    origin: [0, 0],
                    walls: Walls::Up,
                    focus: Focus::default(),
                };
                camera.center_on([focus_tile[0] * 16 + 8, focus_tile[1] * 16 + 8], [200, 150]);
                let mut canvas = Canvas::new(400, 300, [0, 0, 0]);
                draw(
                    &opened.world,
                    &lot,
                    &mut opened.assets,
                    &camera,
                    &mut canvas,
                );
                let px = canvas.rgb565();
                let blue_water = px
                    .iter()
                    .filter(|&&[r, g, b]| b > g && g > r && b >= 60 && r <= 60)
                    .count();
                assert!(
                    blue_water > 100,
                    "{house_name} pond should render blue water at {zoom:?} rot={rotation} (got {blue_water})"
                );
                // Center pixel of a full-water tile must never show chroma-key magenta.
                let center = px[150 * 400 + 200];
                assert!(
                    !(center[0] > 200 && center[1] < 50 && center[2] > 180),
                    "{house_name} pond center leaked magenta {center:?} at {zoom:?} rot={rotation}"
                );
            }
        }

        // Verify House28's Arry 9 `0x10` pool deck tiles render floor 12
        // (and change when cleared to `0xff`).
        if house_name == "UserData/Houses/House28.iff" {
            let deck_tiles: Vec<[i32; 2]> = (1..lot.size - 1)
                .flat_map(|x| (1..lot.size - 1).map(move |y| [x as i32, y as i32]))
                .filter(|&[x, y]| lot.pool[Lot::cell(x as usize, y as usize)] == 0x10)
                .collect();
            assert!(
                !deck_tiles.is_empty(),
                "House28.iff should contain Arry 9 0x10 pool deck tiles"
            );
            let deck = deck_tiles[0];
            let mut cam = Camera {
                zoom: Zoom::Medium,
                rotation: 0,
                level: 1,
                origin: [0, 0],
                walls: Walls::Down,
                focus: Focus::default(),
            };
            cam.center_on([deck[0] * 16 + 8, deck[1] * 16 + 8], [100, 75]);
            let mut with_deck = Canvas::new(200, 150, [0, 0, 0]);
            draw(
                &opened.world,
                &lot,
                &mut opened.assets,
                &cam,
                &mut with_deck,
            );
            let mut cleared = lot.clone();
            cleared.pool[Lot::cell(deck[0] as usize, deck[1] as usize)] = 0xff;
            let mut without_deck = Canvas::new(200, 150, [0, 0, 0]);
            draw(
                &opened.world,
                &cleared,
                &mut opened.assets,
                &cam,
                &mut without_deck,
            );
            assert_ne!(
                with_deck.color, without_deck.color,
                "House28 0x10 pool deck tile {deck:?} must render floor 12 distinct from bare terrain"
            );
            let mut explicit_floor12 = cleared;
            explicit_floor12.levels[0].floors[Lot::cell(deck[0] as usize, deck[1] as usize)] = 12;
            let mut floor12_canvas = Canvas::new(200, 150, [0, 0, 0]);
            draw(
                &opened.world,
                &explicit_floor12,
                &mut opened.assets,
                &cam,
                &mut floor12_canvas,
            );
            assert_eq!(
                with_deck.color[75 * 200 + 100],
                floor12_canvas.color[75 * 200 + 100],
                "House28 0x10 pool deck tile center pixel must match built-in floor 12"
            );
        }
    }
}

#[test]
fn build_mode_dug_pool_and_painted_pond_render_and_invalidate_cache() {
    let Some(install) = install() else {
        return;
    };
    let house = install
        .file("UserData/Houses/House05.iff")
        .expect("House05.iff exists");
    let mut opened = open_house(&install.root, &house).expect("open Goth house");
    // Use midday lighting and flatten/clear the world arrays so we exercise the
    // full World -> Lot::from_world -> LotCache edit-to-render pipeline with
    // zero mansion wall or tree occlusion across all 4 rotations × 3 zooms.
    opened.world.objects.clear();
    if let Some(tod) = opened.world.globals.get_mut(4) {
        *tod = 0;
    }
    for id in [0u16, 1, 2, 101, 102] {
        if let Some(arr) = opened.world.arrays.get_mut(&id) {
            arr.data.fill(0);
        }
    }
    // Raise flat terrain to height 2 so pond painting can depress the interior
    // basin vertex in Arry 0 just like Build Mode Part 2 (`0x4984cf`).
    if let Some(heights) = opened.world.arrays.get_mut(&0) {
        heights.data.fill(2);
    }
    for id in [9u16, 10] {
        opened.world.arrays.insert(
            id,
            Array {
                width: FULL,
                height: FULL,
                stride: 1,
                encoding: 1,
                data: vec![0xff; FULL * FULL],
            },
        );
    }

    // Wrong-dimension stride-1 Arry 9 / Arry 10 (32×128) must fall back to 0xff
    // instead of silently reindexing as 64×64.
    {
        let mut malformed = opened.world.clone();
        malformed.arrays.insert(
            9,
            Array {
                width: 32,
                height: 128,
                stride: 1,
                encoding: 1,
                data: vec![0x0f; FULL * FULL],
            },
        );
        malformed.arrays.insert(
            10,
            Array {
                width: 32,
                height: 128,
                stride: 1,
                encoding: 1,
                data: vec![0x0f; FULL * FULL],
            },
        );
        let fallback_lot = Lot::from_world(&malformed).expect("build malformed fallback Lot");
        assert!(fallback_lot.pool.iter().all(|&b| b == 0xff));
        assert!(fallback_lot.water.iter().all(|&b| b == 0xff));
    }

    // Stage 0: dry yard World -> Lot.
    let dry_world = opened.world.clone();
    let dry_lot = Lot::from_world(&dry_world)
        .expect("build dry Lot")
        .with_floor_rotation(opened.rotation);

    // Stage 1: dig an L-shaped 3x3-minus-1 swimming pool in World's Arry 9 and
    // rebuild Lot from World.
    let mut pool_world = dry_world.clone();
    {
        let pool_arr = pool_world.arrays.get_mut(&9).expect("Arry 9 exists");
        for (x, y) in [
            (6, 6),
            (7, 6),
            (8, 6),
            (6, 7),
            (7, 7),
            (8, 7),
            (6, 8),
            (7, 8),
        ] {
            pool_arr.data[Lot::cell(x, y)] = 0;
        }
        recompute_masks(&mut pool_arr.data, dry_lot.size);
    }
    let pool_lot = Lot::from_world(&pool_world)
        .expect("build pool Lot")
        .with_floor_rotation(opened.rotation);

    // Stage 2: independently paint a 2x2 pond in World's Arry 10 and depress
    // the shared interior pond corner (10, 7) in Arry 0, then rebuild Lot.
    let mut both_world = pool_world.clone();
    {
        let water_arr = both_world.arrays.get_mut(&10).expect("Arry 10 exists");
        for (x, y) in [(9, 6), (10, 6), (9, 7), (10, 7)] {
            water_arr.data[Lot::cell(x, y)] = 0;
        }
        recompute_masks(&mut water_arr.data, dry_lot.size);
        let heights = both_world.arrays.get_mut(&0).expect("Arry 0 exists");
        // Lower shared interior vertex (10, 7) across its 4 incident tiles
        // (matching `write_vertex_height` in `sims-vm::build`).
        for (tx, ty, corner) in [(9, 6, 3), (10, 6, 0), (9, 7, 2), (10, 7, 1)] {
            heights.data[Lot::cell(tx, ty) * 4 + corner] = 1;
        }
    }
    let both_lot = Lot::from_world(&both_world)
        .expect("build pool+pond Lot")
        .with_floor_rotation(opened.rotation);

    let mut cache = LotCache::new();
    for zoom in Zoom::ALL {
        for rotation in 0..4u8 {
            let mut cam = Camera {
                zoom,
                rotation,
                level: 1,
                origin: [0, 0],
                walls: Walls::Up,
                focus: Focus::default(),
            };
            cam.center_on([8 * 16 + 8, 7 * 16 + 8], [320, 240]);
            let mut dry_c = Canvas::new(640, 480, [0, 0, 0]);
            let mut pool_c = Canvas::new(640, 480, [0, 0, 0]);
            let mut both_c = Canvas::new(640, 480, [0, 0, 0]);

            // Warm cache on dry yard before digging pool.
            cache.draw_lot_with_preview(
                &dry_world,
                &dry_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut dry_c,
            );
            cache.draw_lot_with_preview(
                &dry_world,
                &dry_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut dry_c,
            );
            assert_eq!(cache.last_hit(), CacheHit::Full);

            // Edit 1: dug pool invalidates warmed cache and draws turquoise water.
            cache.draw_lot_with_preview(
                &pool_world,
                &pool_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut pool_c,
            );
            assert_eq!(
                cache.last_hit(),
                CacheHit::Miss,
                "dug pool must invalidate warmed LotCache at {zoom:?} rot={rotation}"
            );
            assert_ne!(dry_c.color, pool_c.color);

            // Warm cache on pool lot before painting pond.
            cache.draw_lot_with_preview(
                &pool_world,
                &pool_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut pool_c,
            );
            assert_eq!(cache.last_hit(), CacheHit::Full);

            // Edit 2: painted pond + basin depression invalidates warmed cache.
            cache.draw_lot_with_preview(
                &both_world,
                &both_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut both_c,
            );
            assert_eq!(
                cache.last_hit(),
                CacheHit::Miss,
                "painted pond must invalidate warmed LotCache at {zoom:?} rot={rotation}"
            );
            assert_ne!(pool_c.color, both_c.color);

            let turquoise = both_c
                .rgb565()
                .iter()
                .filter(|&&[r, g, b]| b > g && g > r && b >= 160)
                .count();
            let pond_blue = both_c
                .rgb565()
                .iter()
                .filter(|&&[r, g, b]| b > g && g > r && b >= 60 && r <= 60)
                .count();
            assert!(
                turquoise > 50 && pond_blue > 30,
                "expected both pool ({turquoise}) and pond ({pond_blue}) pixels at {zoom:?} rot={rotation}"
            );
        }
    }

    // Viewport clipping (`0x4c1a4a`–`0x4c1a8c`, `0x4c81b1`–`0x4c81c0`):
    // partially visible pool & pond tiles must never write color, depth, or
    // IDs below `viewport.1` on a taller canvas.
    let mut clip_cam = Camera {
        zoom: Zoom::Near,
        rotation: 0,
        level: 1,
        origin: [0, 0],
        walls: Walls::Up,
        focus: Focus::default(),
    };
    clip_cam.center_on([8 * 16 + 8, 7 * 16 + 8], [320, 150]);
    let sentinel = [0x11, 0x22, 0x33];
    let mut clipped = Canvas::new(640, 300, sentinel);
    draw_lot_in_viewport(
        &both_world,
        &both_lot,
        &mut opened.assets,
        &clip_cam,
        (640, 150),
        &mut clipped,
    );
    // Above the viewport boundary, pool & pond water were drawn.
    assert!(
        clipped.depth[..150 * 640]
            .iter()
            .any(|&d| i32::from(d) < EMPTY),
        "pool/pond should draw above clip_bottom"
    );
    // Below the viewport boundary, color/depth/ids must remain strictly untouched
    // (clear color is written by `paint`, so verify pool/pond did not write depth
    // or non-zero IDs and color equals the flat background fill).
    let bg_below = clipped.color[299 * 640];
    for row in 150..300 {
        for col in 0..640 {
            let idx = row * 640 + col;
            assert_eq!(
                clipped.color[idx], bg_below,
                "pool/pond leaked color below clip_bottom at ({col}, {row})"
            );
            assert_eq!(
                i32::from(clipped.depth[idx]),
                EMPTY,
                "pool/pond leaked depth below clip_bottom at ({col}, {row})"
            );
            assert_eq!(
                clipped.id_at(col as i32, row as i32),
                None,
                "pool/pond leaked id below clip_bottom at ({col}, {row})"
            );
        }
    }
}
