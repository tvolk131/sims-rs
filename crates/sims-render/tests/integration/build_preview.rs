//! Integration tests for Build Mode Part 2 visual previews
//! (`crates/sims-render/src/build_preview.rs`) against a real Complete
//! Collection `SIMS_INSTALL`.

use sims_content::Install;
use sims_render::build_preview::{
    DEFAULT_ROOF_PATTERN, RoofCatalog, RoofPitch, RoofPreviewConfig, RoofStyle, TerrainOverlay,
    TerrainTarget, TerrainTool, WaterAction, WaterOverlay, WaterTileStatus, WaterTool,
    detect_roof_footprints, draw_roof_preview, draw_roof_preview_from_catalog,
    draw_terrain_preview, draw_water_preview, validate_terrain_overlay, validate_water_overlay,
};
use sims_render::{Camera, Canvas, Focus, Lot, Walls, Zoom, draw_lot, open_house};
use std::path::PathBuf;

fn install() -> Option<Install> {
    let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    Some(
        Install::detect(&root).expect("SIMS_INSTALL should be a valid Complete Collection install"),
    )
}

#[test]
fn complete_collection_roof_catalog_loads_all_18_bmps_and_default_r1() {
    let Some(install) = install() else {
        return;
    };
    let catalog = RoofCatalog::load(&install).expect("GameData/Roofs should load cleanly");
    assert_eq!(
        catalog.len(),
        18,
        "Complete Collection ships 18 BMP roof patterns in GameData/Roofs"
    );
    let default = catalog
        .default_pattern()
        .expect("default roof pattern r1_ must exist");
    assert_eq!(default.name, DEFAULT_ROOF_PATTERN);
    assert_eq!(default.width, 32);
    assert_eq!(default.height, 32);
    assert_eq!(default.rgba.len(), 32 * 32 * 4);

    // Case-insensitive stem and `.bmp` suffix lookup both work.
    assert!(catalog.get("R1_.BMP").is_some());
    assert!(catalog.get("SuperstarGoldRoof").is_some());
    assert!(catalog.get("mmgypsytentroof.bmp").is_some());
    assert!(catalog.get("gravel1").is_some());
    assert!(catalog.get("tar1").is_some());

    for pattern in catalog.patterns() {
        assert_eq!(
            (pattern.width, pattern.height),
            (32, 32),
            "{}",
            pattern.name
        );
        assert_eq!(pattern.rgba.len(), 32 * 32 * 4, "{}", pattern.name);
    }
}

#[test]
fn goth_house_build_previews_render_cleanly_without_altering_static_lot_pixels() {
    let Some(install) = install() else {
        return;
    };
    let roof_catalog = RoofCatalog::load(&install).expect("load roof catalog");
    let house_path = install
        .file("UserData/Houses/House05.iff")
        .expect("House05.iff exists");
    let mut opened = open_house(&install.root, &house_path).expect("open Goth house");
    let lot = Lot::from_world(&opened.world)
        .expect("build Lot")
        .with_floor_rotation(opened.rotation);

    let mut camera = Camera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 2,
        origin: [0, 0],
        walls: Walls::Up,
        focus: Focus::default(),
    };
    camera.center_on(lot.built_center(), [400, 300]);

    // Render static lot baseline twice to prove zero mutation / bit identity.
    let mut base_a = Canvas::new(800, 600, sims_render::scene::BACKGROUND);
    let mut base_b = Canvas::new(800, 600, sims_render::scene::BACKGROUND);
    draw_lot(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut base_a,
    );
    draw_lot(
        &opened.world,
        &lot,
        &mut opened.assets,
        &camera,
        &mut base_b,
    );
    assert_eq!(base_a.color, base_b.color);
    assert_eq!(base_a.depth, base_b.depth);

    // 1. Terrain overlay on an open yard vertex centered in the camera view.
    let open_vertex = (2..lot.size - 2)
        .flat_map(|x| (2..lot.size - 2).map(move |y| [x as i32, y as i32]))
        .find(|&v| {
            validate_terrain_overlay(
                &lot,
                Some(&opened.world),
                &TerrainOverlay {
                    tool: TerrainTool::Raise,
                    target: TerrainTarget::Vertex(v),
                },
            )
            .valid
        })
        .expect("Goth house has an editable open yard vertex");
    let mut yard_cam = camera;
    yard_cam.center_on([open_vertex[0] * 16, open_vertex[1] * 16], [400, 300]);
    let mut yard_base = Canvas::new(800, 600, sims_render::scene::BACKGROUND);
    draw_lot(
        &opened.world,
        &lot,
        &mut opened.assets,
        &yard_cam,
        &mut yard_base,
    );
    let mut terrain_canvas = yard_base.clone();
    let (terrain_plan, terrain_stats) = draw_terrain_preview(
        &lot,
        Some(&opened.world),
        &yard_cam,
        &TerrainOverlay {
            tool: TerrainTool::Raise,
            target: TerrainTarget::Vertex(open_vertex),
        },
        &mut terrain_canvas,
    );
    assert!(terrain_plan.valid);
    assert_eq!(terrain_plan.tiles.len(), 4);
    assert!(terrain_stats.pixels_written > 0);
    assert_ne!(terrain_canvas.color, yard_base.color);
    // Ground overlays do not clobber the lot's depth buffer.
    assert_eq!(terrain_canvas.depth, yard_base.depth);

    // Road tiles on the Goth lot front must be rejected by terrain validation.
    let road_tile = (0..lot.size)
        .flat_map(|x| (0..lot.size).map(move |y| [x as i32, y as i32]))
        .find(|&[x, y]| {
            let f = lot.levels[0].floors[Lot::cell(x as usize, y as usize)];
            (9..=13).contains(&f)
        })
        .expect("Goth house has road tiles");
    let road_plan = validate_terrain_overlay(
        &lot,
        Some(&opened.world),
        &TerrainOverlay {
            tool: TerrainTool::Grass,
            target: TerrainTarget::Brush {
                center: road_tile,
                radius: 0,
            },
        },
    );
    assert!(!road_plan.valid);

    // 2. Water / Pool preview centered on an open flat yard rectangle.
    let pool_start = (2..lot.size - 5)
        .flat_map(|x| (2..lot.size - 5).map(move |y| [x as i32, y as i32]))
        .find(|&[x, y]| {
            validate_water_overlay(
                &lot,
                Some(&opened.world),
                &WaterOverlay {
                    tool: WaterTool::Pool,
                    action: WaterAction::Dig,
                    start: [x, y],
                    end: [x + 2, y + 2],
                },
            )
            .valid
        })
        .expect("Goth house has a flat open 3x3 yard region for a pool");
    let mut pool_cam = camera;
    pool_cam.center_on(
        [(pool_start[0] + 1) * 16, (pool_start[1] + 1) * 16],
        [400, 300],
    );
    let mut pool_canvas = Canvas::new(800, 600, sims_render::scene::BACKGROUND);
    draw_lot(
        &opened.world,
        &lot,
        &mut opened.assets,
        &pool_cam,
        &mut pool_canvas,
    );
    let (pool_plan, pool_stats) = draw_water_preview(
        &lot,
        Some(&opened.world),
        &pool_cam,
        &WaterOverlay {
            tool: WaterTool::Pool,
            action: WaterAction::Dig,
            start: pool_start,
            end: [pool_start[0] + 2, pool_start[1] + 2],
        },
        &mut pool_canvas,
    );
    assert!(pool_plan.valid);
    assert_eq!(pool_plan.tiles.len(), 9);
    assert_eq!(pool_plan.coping_tiles.len(), 16);
    assert!(pool_stats.pixels_written > 0);

    // Verify a pool dragged over a sloped backyard tile flags InvalidSlope.
    let sloped_tile = (2..lot.size - 2)
        .flat_map(|x| (2..lot.size - 2).map(move |y| [x as i32, y as i32]))
        .find(|&[x, y]| {
            let h = lot.heights[Lot::cell(x as usize, y as usize)];
            h[0] != h[1] || h[1] != h[2] || h[2] != h[3]
        });
    if let Some(st) = sloped_tile {
        let sloped_pool = validate_water_overlay(
            &lot,
            Some(&opened.world),
            &WaterOverlay {
                tool: WaterTool::Pool,
                action: WaterAction::Dig,
                start: st,
                end: st,
            },
        );
        assert!(!sloped_pool.valid);
        assert!(
            sloped_pool
                .tiles
                .iter()
                .any(|t| t.status == WaterTileStatus::InvalidSlope)
        );
    }

    // 3. Roof preview over the Goth House's enclosed rooms across styles & pitches.
    let [l1, l2] = detect_roof_footprints(&lot);
    assert!(
        !l1.is_empty() || !l2.is_empty(),
        "Goth house must have enclosed indoor rooms to roof"
    );

    let mut roof_medium = base_a.clone();
    let medium_stats = draw_roof_preview_from_catalog(
        &lot,
        &camera,
        &roof_catalog,
        &RoofPreviewConfig {
            pattern_name: "r2_".to_owned(),
            style: RoofStyle::Hip,
            pitch: RoofPitch::Medium,
            ..RoofPreviewConfig::default()
        },
        &mut roof_medium,
    )
    .expect("render medium hip roof");
    assert!(medium_stats.footprint_tiles > 20);
    assert!(medium_stats.pixels_written > 1000);
    assert_ne!(roof_medium.color, base_a.color);

    let mut roof_steep = base_a.clone();
    let steep_pattern = roof_catalog
        .get("SuperstarGoldRoof")
        .expect("SuperstarGoldRoof present");
    let steep_stats = draw_roof_preview(
        &lot,
        &camera,
        steep_pattern,
        &RoofPreviewConfig {
            pattern_name: steep_pattern.name.clone(),
            style: RoofStyle::Gable,
            pitch: RoofPitch::Steep,
            ..RoofPreviewConfig::default()
        },
        &mut roof_steep,
    );
    assert!(steep_stats.pixels_written > 1000);
    assert_ne!(roof_steep.color, roof_medium.color);

    // Confirm all 4 camera rotations and all 3 zooms render cleanly without panic.
    for zoom in Zoom::ALL {
        for rot in 0..4u8 {
            let mut cam = Camera {
                zoom,
                rotation: rot,
                level: 2,
                origin: [0, 0],
                walls: Walls::Up,
                focus: Focus::default(),
            };
            // Frame the roof height above the house's built center so the roof
            // stays inside the 400x300 test viewport even at Near zoom (232 px/storey).
            cam.center_on(lot.built_center(), [200, 150 + zoom.floor_height() * 2]);
            let mut c = Canvas::new(400, 300, sims_render::scene::BACKGROUND);
            let s = draw_roof_preview_from_catalog(
                &lot,
                &cam,
                &roof_catalog,
                &RoofPreviewConfig::default(),
                &mut c,
            )
            .expect("render roof across zoom/rotation");
            assert!(s.pixels_written > 0, "{zoom:?} rot {rot}");
        }
    }
}
