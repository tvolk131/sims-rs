use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_content::{Catalog, Index, Install, SurfaceCatalog, SurfaceKind};
use sims_ui::art::Art;
use sims_ui::build::{
    self, BuildCatalog, BuildState, BuildSubtoolAction, BuildSwatches, BuildWidgetKind,
    RoofPitchPreset, TerrainSubtool, WaterSubtool,
};
use sims_ui::command::{BuildTool, Command};
use sims_ui::frame::{Frame, Rect};
use sims_ui::thumbnail::Thumbnails;
use std::path::PathBuf;

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

fn assert_layout_has_no_overlaps_and_in_bounds(layout: &build::BuildLayout, width: i32) {
    let mut all: Vec<(&str, Rect)> = Vec::new();
    for w in &layout.widgets {
        all.push(("widget", w.rect));
    }
    for s in &layout.subtools {
        all.push(("subtool", s.rect));
    }
    for &(kind, r) in &all {
        assert!(
            r.x >= 0 && r.x + r.width <= width,
            "{kind} out of horizontal bounds 0..{width}: {r:?}"
        );
    }
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert!(
                !rects_overlap(all[i].1, all[j].1),
                "overlapping Build UI rects at width {width}: {} {:?} vs {} {:?}",
                all[i].0,
                all[i].1,
                all[j].0,
                all[j].1
            );
        }
    }
}

#[test]
fn narrow_viewport_build_strip_has_zero_overlaps_across_tools() {
    let catalog = BuildCatalog::default().with_roof_patterns(&[
        "r1_",
        "r2_",
        "r3_",
        "r4_",
        "gravel1",
        "tar1",
        "MMSuburban1Roof",
        "SuperstarGoldRoof",
    ]);
    let art = Art::default();

    for width in [
        360, 400, 480, 512, 580, 619, 620, 640, 651, 652, 660, 800, 1024,
    ] {
        for tool in [
            None,
            Some(BuildTool::Terrain),
            Some(BuildTool::Water),
            Some(BuildTool::Roof),
            Some(BuildTool::Hand),
        ] {
            let mut state = BuildState::default();
            if let Some(t) = tool {
                state.select_tool(t, &catalog);
            }
            let first = build::layout(
                [width, 384],
                &catalog,
                &state,
                &art,
                1000,
                true,
                false,
                None,
            );
            assert_layout_has_no_overlaps_and_in_bounds(&first, width);
            for p in 0..first.total_pages.max(1) {
                state.page = p;
                let lay = build::layout(
                    [width, 384],
                    &catalog,
                    &state,
                    &art,
                    1000,
                    true,
                    false,
                    None,
                );
                assert_layout_has_no_overlaps_and_in_bounds(&lay, width);
            }
            assert_eq!(first.total_tool_pages, 1);
            // Every single one of the 12 Build tools must be directly clickable
            // through `layout.press(point)` even at 360 px and 400 px (no stranded tool!).
            let mut clickable = std::collections::BTreeSet::new();
            for w in &first.widgets {
                if let Some(Command::BuildTool(t)) = w.command {
                    let c = [w.rect.x + w.rect.width / 2, w.rect.y + w.rect.height / 2];
                    assert_eq!(first.press(c), Some(Command::BuildTool(t)));
                    clickable.insert(t);
                }
            }
            assert_eq!(
                clickable.len(),
                12,
                "all 12 Build tools must remain reachable via press() at width {width} tool {tool:?}"
            );
        }
    }
}

#[test]
fn terrain_water_and_roof_subtool_selection_and_popups() {
    let catalog = BuildCatalog::default()
        .with_roof_patterns(&["r1_", "r2_", "r3_", "r4_", "gravel1", "tar1"]);
    let art = Art::default();
    let mut state = BuildState::default();

    // Terrain subtools, left to right Grass, Level, Lower, Raise with Grass
    // selected (oracle `build-terrain`); click Raise.
    state.select_tool(BuildTool::Terrain, &catalog);
    assert_eq!(state.terrain_subtool, TerrainSubtool::Grass);
    let lay = build::layout([800, 600], &catalog, &state, &art, 1000, false, false, None);
    assert_eq!(lay.subtools.len(), 4);
    let order: Vec<_> = lay.subtools.iter().map(|s| s.action).collect();
    assert_eq!(
        order,
        [
            TerrainSubtool::Grass,
            TerrainSubtool::Level,
            TerrainSubtool::Lower,
            TerrainSubtool::Raise,
        ]
        .map(BuildSubtoolAction::Terrain)
    );
    let raise_btn = lay
        .subtools
        .iter()
        .find(|s| s.action == BuildSubtoolAction::Terrain(TerrainSubtool::Raise))
        .expect("Raise subtool button");
    let pt = [
        raise_btn.rect.x + raise_btn.rect.width / 2,
        raise_btn.rect.y + raise_btn.rect.height / 2,
    ];
    assert_eq!(
        lay.press_subtool(pt),
        Some(BuildSubtoolAction::Terrain(TerrainSubtool::Raise))
    );
    assert!(state.apply_subtool(BuildSubtoolAction::Terrain(TerrainSubtool::Raise), &catalog));
    assert_eq!(state.terrain_subtool, TerrainSubtool::Raise);

    // Hovering a Terrain subtool surfaces its popup card.
    let hovered = build::layout(
        [800, 600],
        &catalog,
        &state,
        &art,
        1000,
        false,
        false,
        Some(pt),
    );
    assert_eq!(
        hovered.subtool_popup.as_ref().map(|p| p.name.as_str()),
        Some("Raise Terrain")
    );

    // Water subtools: Pool ($75) & Water ($15).
    state.select_tool(BuildTool::Water, &catalog);
    let wlay = build::layout([800, 600], &catalog, &state, &art, 1000, false, false, None);
    let water_btn = wlay
        .subtools
        .iter()
        .find(|s| s.action == BuildSubtoolAction::Water(WaterSubtool::Water))
        .expect("Water subtool button");
    assert_eq!(water_btn.price, 15);
    assert!(state.apply_subtool(BuildSubtoolAction::Water(WaterSubtool::Water), &catalog));
    assert_eq!(state.water_subtool, WaterSubtool::Water);

    // Roof tool: 4 pitch presets + enumerated roof patterns.
    state.select_tool(BuildTool::Roof, &catalog);
    let rlay = build::layout([800, 600], &catalog, &state, &art, 1000, false, false, None);
    let steep = rlay
        .subtools
        .iter()
        .find(|s| s.action == BuildSubtoolAction::RoofPitch(RoofPitchPreset::Steep))
        .expect("Steep roof pitch button");
    let steep_pt = [
        steep.rect.x + steep.rect.width / 2,
        steep.rect.y + steep.rect.height / 2,
    ];
    assert_eq!(
        rlay.press_subtool(steep_pt),
        Some(BuildSubtoolAction::RoofPitch(RoofPitchPreset::Steep))
    );
    assert!(state.apply_subtool(
        BuildSubtoolAction::RoofPitch(RoofPitchPreset::Steep),
        &catalog
    ));
    assert_eq!(state.roof_pitch, RoofPitchPreset::Steep);

    let r4_idx = catalog.roof_pattern_index("r4_.bmp").expect("r4_ index");
    assert_eq!(r4_idx, 3);
    let pat_btn = rlay
        .subtools
        .iter()
        .find(|s| s.action == BuildSubtoolAction::RoofPattern(r4_idx))
        .expect("r4_ roof pattern button");
    let pat_pt = [
        pat_btn.rect.x + pat_btn.rect.width / 2,
        pat_btn.rect.y + pat_btn.rect.height / 2,
    ];
    assert_eq!(
        rlay.press_subtool(pat_pt),
        Some(BuildSubtoolAction::RoofPattern(r4_idx))
    );
    assert!(state.apply_subtool(BuildSubtoolAction::RoofPattern(r4_idx), &catalog));
    assert_eq!(state.roof_pattern, r4_idx);
}

#[test]
fn live_install_roof_catalog_subtool_prices_and_narrow_layout_rendering() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let index = Index::build(&install).expect("build index");
    let catalog = Catalog::build(&index).expect("read object catalog");
    let floor_plugins = SurfaceCatalog::plugins(&install, SurfaceKind::Floor).unwrap();
    let wall_plugins = SurfaceCatalog::plugins(&install, SurfaceKind::Wall).unwrap();
    let builtin_floors: Vec<u16> = (1..=29).collect();
    let builtin_walls: Vec<u16> = (1..=30).collect();

    let build_cat = BuildCatalog::load(
        &install,
        &index,
        &catalog,
        &builtin_floors,
        &builtin_walls,
        &floor_plugins,
        &wall_plugins,
    );

    // Complete Collection 1.91 ships 18 BMPs in GameData/Roofs/.
    assert_eq!(
        build_cat.roof_patterns().len(),
        18,
        "Complete Collection GameData/Roofs should enumerate 18 BMP patterns"
    );
    // Plain case-insensitive name order, as the 1.91 oracle lists them.
    assert_eq!(build_cat.roof_pattern(0).unwrap().stem, "gravel1");
    assert_eq!(build_cat.roof_pattern(8).unwrap().stem, "r1_");
    assert_eq!(build_cat.roof_pattern(10).unwrap().stem, "r3_");
    assert_eq!(build_cat.roof_pattern(17).unwrap().stem, "tar1");
    for pat in build_cat.roof_patterns() {
        let swatch = pat
            .swatch
            .as_ref()
            .unwrap_or_else(|| panic!("roof swatch {} should decode", pat.stem));
        assert_eq!((swatch.width, swatch.height), (32, 32));
    }

    // Native Build.iff STR# 128 and STR# 133 pricing.
    assert_eq!(build_cat.terrain_price(TerrainSubtool::Raise), 1);
    assert_eq!(build_cat.terrain_price(TerrainSubtool::Lower), 1);
    assert_eq!(build_cat.terrain_price(TerrainSubtool::Level), 1);
    assert_eq!(build_cat.terrain_price(TerrainSubtool::Grass), 1);
    assert_eq!(build_cat.pool_price(), 75);
    assert_eq!(build_cat.water_price(), 15);

    // Load real UIGraphics Build Mode art (including all new subtool/roof IDs).
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let mut art = Art::default();
    for res_id in build::resource_ids() {
        art.load_optional(&ui, US_ENGLISH, res_id);
    }
    for expected in [
        build::id::SUBTOOL_TERRAIN_UP,
        build::id::SUBTOOL_TERRAIN_DOWN,
        build::id::SUBTOOL_TERRAIN_LEVEL,
        build::id::SUBTOOL_GRASS,
        build::id::SUBTOOL_POOL,
        build::id::SUBTOOL_WATER,
        build::id::SUBTOOL_ROOF_FLAT,
        build::id::SUBTOOL_ROOF_SHALLOW,
        build::id::SUBTOOL_ROOF_MEDIUM,
        build::id::SUBTOOL_ROOF_STEEP,
        build::id::ROOF_PATTERN_TEMPLATE,
    ] {
        assert!(
            art.image(expected).is_some(),
            "UIGraphics Build subtool art {expected} must load"
        );
    }

    // Verify zero overlaps across 360..=800 widths, every tool, and every page with real art!
    let thumbs = Thumbnails::default();
    let swatches = BuildSwatches::default();
    for size in [
        [360, 384],
        [400, 384],
        [480, 384],
        [512, 384],
        [620, 384],
        [640, 384],
        [800, 600],
    ] {
        for tool in [
            BuildTool::Terrain,
            BuildTool::Water,
            BuildTool::Floor,
            BuildTool::Roof,
            BuildTool::Wall,
        ] {
            let mut state = BuildState::default();
            state.select_tool(tool, &build_cat);
            let first = build::layout(size, &build_cat, &state, &art, 5000, true, true, None);
            assert_layout_has_no_overlaps_and_in_bounds(&first, size[0]);
            for p in 0..first.total_pages.clamp(1, 4) {
                state.page = p;
                let lay = build::layout(size, &build_cat, &state, &art, 5000, true, true, None);
                assert_layout_has_no_overlaps_and_in_bounds(&lay, size[0]);
            }
            let mut clickable = std::collections::BTreeSet::new();
            for w in &first.widgets {
                if let Some(Command::BuildTool(t)) = w.command {
                    let c = [w.rect.x + w.rect.width / 2, w.rect.y + w.rect.height / 2];
                    assert_eq!(first.press(c), Some(Command::BuildTool(t)));
                    clickable.insert(t);
                }
            }
            assert_eq!(
                clickable.len(),
                12,
                "all 12 real-art Build tools (including 43px Floor & Water) must be clickable via press() at {size:?} {tool:?}"
            );
            if tool == BuildTool::Floor && size[0] >= 400 {
                assert!(
                    first
                        .widgets
                        .iter()
                        .any(|w| matches!(w.kind, BuildWidgetKind::Slot { .. })),
                    "Floor slots must remain visible at {size:?}"
                );
            }
            let mut frame = Frame::new(size[0] as u32, size[1] as u32, [0, 0, 0]);
            build::draw_with_catalog(
                &mut frame,
                &art,
                &thumbs,
                &swatches,
                Some(&build_cat),
                &first,
                None,
            );
            assert!(
                frame.rgb().iter().any(|&px| px != [0, 0, 0]),
                "drawn Build strip for {tool:?} at {size:?} must not be blank"
            );
        }
    }
}

#[test]
fn roof_page_arrows_and_page_size_follow_roofpanelstrs() {
    // `cWinRoofPanel` (`0x52911a`, `STR# 147`): 16 patterns a page with Next
    // Page at panel + 433, or 8 with it at panel + 253 on an 800-wide screen;
    // Previous Page at panel + 58; both 20 below the panel's top. The panel
    // is at (538, strip_y + 5) here, and the 1.91 oracle's Next Page sits at
    // (971, 693) on 1024×768.
    let names: Vec<String> = (0..18).map(|i| format!("p{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let catalog = BuildCatalog::default().with_roof_patterns(&refs);
    let art = Art::default();
    for (width, per_page, next_x) in [(800, 8, 791), (1024, 16, 971)] {
        let height = width * 3 / 4;
        let strip_y = height - 100;
        for page in [0, 1] {
            let mut state = BuildState::default();
            state.select_tool(BuildTool::Roof, &catalog);
            state.page = page;
            let layout = build::layout(
                [width, height],
                &catalog,
                &state,
                &art,
                1000,
                false,
                false,
                None,
            );
            let arrow = |delta: i32| {
                layout
                    .widgets
                    .iter()
                    .find(|w| w.command == Some(Command::BuildPage(delta)))
                    .map(|w| [w.rect.x, w.rect.y])
            };
            let patterns = layout
                .subtools
                .iter()
                .filter(|s| matches!(s.action, BuildSubtoolAction::RoofPattern(_)))
                .count();
            assert_eq!(
                patterns,
                per_page.min(18 - page * per_page),
                "{width} page {page}"
            );
            let has_next = (page + 1) * per_page < 18;
            assert_eq!(
                arrow(1),
                has_next.then_some([next_x, strip_y + 25]),
                "{width} page {page}"
            );
            assert_eq!(
                arrow(-1),
                (page > 0).then_some([596, strip_y + 25]),
                "{width} page {page}"
            );
            assert_layout_has_no_overlaps_and_in_bounds(&layout, width);
        }
    }
}
