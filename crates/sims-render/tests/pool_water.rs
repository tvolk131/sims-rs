//! Integration tests for Complete Collection swimming pool (`Arry 9`,
//! `SPR2 0x400..=0x43b`) and pond/natural water (`Arry 10`,
//! `SPR2 0x800..=0x80f`) rendering against a real `SIMS_INSTALL`.

use sims_content::Install;
use sims_render::{CacheHit, Camera, Canvas, Focus, Lot, LotCache, Walls, Zoom, draw, open_house};
use std::path::PathBuf;

fn install() -> Option<Install> {
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Some(
        Install::detect(&root).expect("SIMS_INSTALL should be a valid Complete Collection install"),
    )
}

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
        let Ok(house) = install.file(house_name) else {
            continue;
        };
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
    // Use midday lighting and clear objects so trees/fences cannot occlude the
    // synthetic dug pool & painted pond across all 4 rotations × 3 zooms.
    opened.world.objects.clear();
    if let Some(tod) = opened.world.globals.get_mut(4) {
        *tod = 0;
    }
    let mut base_lot = Lot::from_world(&opened.world)
        .expect("build Goth Lot")
        .with_floor_rotation(opened.rotation);
    base_lot.time_of_day = 0;
    for level in &mut base_lot.levels {
        level.walls.fill(Default::default());
        level.floors.fill(0);
    }
    base_lot.heights.fill([0, 0, 0, 0]);
    let mut edited_lot = base_lot.clone();
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
        edited_lot.pool[Lot::cell(x, y)] = 0;
    }
    recompute_masks(&mut edited_lot.pool, edited_lot.size);
    for (x, y) in [(9, 6), (10, 6), (9, 7), (10, 7)] {
        edited_lot.water[Lot::cell(x, y)] = 0;
    }
    recompute_masks(&mut edited_lot.water, edited_lot.size);

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
            let mut before = Canvas::new(640, 480, [0, 0, 0]);
            let mut after = Canvas::new(640, 480, [0, 0, 0]);
            cache.draw_lot_with_preview(
                &opened.world,
                &base_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut before,
            );
            cache.draw_lot_with_preview(
                &opened.world,
                &edited_lot,
                &mut opened.assets,
                &cam,
                (640, 480),
                None,
                &mut after,
            );
            assert_eq!(
                cache.last_hit(),
                CacheHit::Miss,
                "mutating pool/water must invalidate LotCache at {zoom:?} rot={rotation}"
            );
            assert_ne!(
                before.color, after.color,
                "dug pool & painted pond must change rendered pixels and invalidate LotCache at {zoom:?} rot={rotation}"
            );
            let turquoise = after
                .rgb565()
                .iter()
                .filter(|&&[r, g, b]| b > g && g > r && b >= 160)
                .count();
            let pond_blue = after
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
}
