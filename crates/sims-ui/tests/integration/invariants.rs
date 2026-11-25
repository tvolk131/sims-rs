//! Resize-continuity, radio-group mutual exclusion, Buy strip height, and
//! real-install Build Mode catalog asset-sanity invariant tests.

use crate::common;

use sims_content::{Catalog, Index, Install};
use sims_ui::art::Art;
use sims_ui::build::{self, BuildCatalog, BuildState};
use sims_ui::buy::{self, BuyCatalog, BuySort, BuyState};
use sims_ui::camera::{CameraLayout, CameraState, CameraStrings};
use sims_ui::command::{BuildItemId, BuildTool, Mode, OptionsTab, Panel, Walls};
use sims_ui::frame::Rect;
use sims_ui::household::Snapshot;
use sims_ui::hud::{self, CLUSTER, CameraState as HudCamera, Controls, Kind, STRIP, id};
use sims_ui::options::{OptionsState, OptionsTexts};
use std::collections::BTreeMap;

const VIEWPORTS: [[i32; 2]; 6] = [
    [800, 600],
    [1024, 768],
    [1025, 768],
    [1280, 720],
    [1440, 900],
    [1600, 900],
];

fn sample_snapshot() -> Snapshot {
    let mut world = sims_sim::World::empty();
    world.budget.funds = 2500;
    let mut s = Snapshot::read(&world, &[]);
    s.show_money = true;
    s.residential = true;
    s
}

#[test]
fn resize_continuity_across_1024_to_1025_never_teleports_hud_buy_build_options_or_camera() {
    let snap = sample_snapshot();
    let art = Art::default();
    let opt_texts = OptionsTexts::default();

    // 1. HUD across Live (every subpanel), Buy, Build, Options (every tab).
    let mut configs: Vec<(&str, Controls)> = vec![
        (
            "live_needs",
            Controls {
                mode: Mode::Live,
                expanded: true,
                panel: Some(Panel::Needs),
                ..Controls::default()
            },
        ),
        (
            "live_personality",
            Controls {
                mode: Mode::Live,
                expanded: true,
                panel: Some(Panel::Personality),
                ..Controls::default()
            },
        ),
        (
            "live_job",
            Controls {
                mode: Mode::Live,
                expanded: true,
                panel: Some(Panel::Job),
                ..Controls::default()
            },
        ),
        (
            "live_relationships",
            Controls {
                mode: Mode::Live,
                expanded: true,
                panel: Some(Panel::Relationships),
                ..Controls::default()
            },
        ),
        (
            "live_house",
            Controls {
                mode: Mode::Live,
                expanded: true,
                panel: Some(Panel::House),
                ..Controls::default()
            },
        ),
        (
            "buy_expanded",
            Controls {
                mode: Mode::Buy,
                expanded: true,
                ..Controls::default()
            },
        ),
        (
            "build_expanded",
            Controls {
                mode: Mode::Build,
                expanded: true,
                ..Controls::default()
            },
        ),
    ];
    for tab in [
        None,
        Some(OptionsTab::Graphics),
        Some(OptionsTab::Sound),
        Some(OptionsTab::Play),
    ] {
        let options = OptionsState {
            tab,
            ..OptionsState::default()
        };
        configs.push((
            "options_tab",
            Controls {
                mode: Mode::Options,
                expanded: true,
                options,
                ..Controls::default()
            },
        ));
    }

    for (label, controls) in &configs {
        let items_a =
            hud::layout_with_options([1024, 768], &snap, controls, &opt_texts, Some(&art));
        let items_b =
            hud::layout_with_options([1025, 768], &snap, controls, &opt_texts, Some(&art));
        // Map every button/image/gauge/toggle/slider by stable key and assert dx/dy <= 2 px.
        let key_of = |i: &hud::Item| -> Option<(u8, i32)> {
            match i.kind {
                Kind::Image(id) => Some((1, id)),
                Kind::Button { id, .. } => Some((2, id)),
                Kind::Toggle { id, .. } => Some((3, id)),
                Kind::Pause { .. } => Some((4, id::PAUSE)),
                Kind::Portrait { guid, .. } => Some((5, guid as i32)),
                _ => None,
            }
        };
        let mut map_a: BTreeMap<(u8, i32), Vec<Rect>> = BTreeMap::new();
        for item in &items_a {
            if let Some(k) = key_of(item) {
                map_a.entry(k).or_default().push(item.rect);
            }
        }
        let mut map_b: BTreeMap<(u8, i32), Vec<Rect>> = BTreeMap::new();
        for item in &items_b {
            if let Some(k) = key_of(item) {
                map_b.entry(k).or_default().push(item.rect);
            }
        }
        assert_eq!(
            map_a.keys().collect::<Vec<_>>(),
            map_b.keys().collect::<Vec<_>>(),
            "{label}: widget set changed between 1024x768 and 1025x768"
        );
        for (key, rects_a) in &map_a {
            let rects_b = &map_b[key];
            assert_eq!(rects_a.len(), rects_b.len());
            for (ra, rb) in rects_a.iter().zip(rects_b.iter()) {
                assert!(
                    (ra.x - rb.x).abs() <= 2 && (ra.y - rb.y).abs() <= 2,
                    "{label} widget {key:?} teleported on 1024->1025: {ra:?} -> {rb:?}"
                );
            }
        }
    }

    // 2. Buy Mode top-level & subcategory widgets across 1024 -> 1025.
    let buy_cat = BuyCatalog::default();
    for sort in [BuySort::ByFunction, BuySort::ByRoom] {
        for category in [None, Some(0)] {
            let state = BuyState {
                zone: sims_ui::buy::BuyZone::Residential,
                sort,
                category,
                subcategory: 0,
                page: 0,
                selected: None,
            };
            let la = buy::layout([1024, 768], &buy_cat, &state, &art, 5000);
            let lb = buy::layout([1025, 768], &buy_cat, &state, &art, 5000);
            assert_eq!(la.widgets.len(), lb.widgets.len());
            for (wa, wb) in la.widgets.iter().zip(lb.widgets.iter()) {
                assert!(
                    (wa.rect.x - wb.rect.x).abs() <= 2 && (wa.rect.y - wb.rect.y).abs() <= 2,
                    "Buy widget teleported on 1024->1025 ({sort:?}, {category:?}): {:?} -> {:?}",
                    wa.rect,
                    wb.rect
                );
            }
        }
    }

    // 3. Build Mode widgets & subtools across 1024 -> 1025.
    let build_cat = BuildCatalog::default().with_roof_patterns(&["r1_", "r2_", "r3_", "r4_"]);
    for tool in [
        None,
        Some(BuildTool::Terrain),
        Some(BuildTool::Water),
        Some(BuildTool::Wall),
        Some(BuildTool::Doors),
        Some(BuildTool::Windows),
        Some(BuildTool::Stairs),
        Some(BuildTool::Plants),
        Some(BuildTool::Fireplace),
        Some(BuildTool::Roof),
    ] {
        let state = BuildState {
            tool,
            ..BuildState::default()
        };
        let la = build::layout(
            [1024, 768],
            &build_cat,
            &state,
            &art,
            5000,
            true,
            true,
            None,
        );
        let lb = build::layout(
            [1025, 768],
            &build_cat,
            &state,
            &art,
            5000,
            true,
            true,
            None,
        );
        assert_eq!(la.widgets.len(), lb.widgets.len());
        for (wa, wb) in la.widgets.iter().zip(lb.widgets.iter()) {
            assert!(
                (wa.rect.x - wb.rect.x).abs() <= 2 && (wa.rect.y - wb.rect.y).abs() <= 2,
                "Build widget teleported on 1024->1025 ({tool:?}): {:?} -> {:?}",
                wa.rect,
                wb.rect
            );
        }
        assert_eq!(la.subtools.len(), lb.subtools.len());
        for (sa, sb) in la.subtools.iter().zip(lb.subtools.iter()) {
            assert!(
                (sa.rect.x - sb.rect.x).abs() <= 2 && (sa.rect.y - sb.rect.y).abs() <= 2,
                "Build subtool teleported on 1024->1025 ({tool:?}): {:?} -> {:?}",
                sa.rect,
                sb.rect
            );
        }
    }

    // 4. Camera subpanel across 1024 -> 1025.
    let cam_state = CameraState::default();
    let cam_strings = CameraStrings::default();
    let strip_a = Rect::new(CLUSTER[0], 768 - STRIP, 1024 - CLUSTER[0], STRIP);
    let strip_b = Rect::new(CLUSTER[0], 768 - STRIP, 1025 - CLUSTER[0], STRIP);
    let ca = CameraLayout::compute(strip_a, &cam_state, &cam_strings, &art);
    let cb = CameraLayout::compute(strip_b, &cam_state, &cam_strings, &art);
    assert_eq!(ca.buttons.len(), cb.buttons.len());
    for (ba, bb) in ca.buttons.iter().zip(cb.buttons.iter()) {
        assert_eq!(ba.button, bb.button);
        assert!(
            (ba.rect.x - bb.rect.x).abs() <= 2 && (ba.rect.y - bb.rect.y).abs() <= 2,
            "Camera button {:?} teleported on 1024->1025: {:?} -> {:?}",
            ba.button,
            ba.rect,
            bb.rect
        );
    }

    // 5. Help balloon picture-slot vertical layout (`balloon::slot_vertical`)
    // preserves exact 800x600 & 1024x768 parity and stays continuous across
    // 1024 -> 1025 -> 1280 -> 1600 -> 1920 (no 50 px jump past 1024).
    assert_eq!(sims_ui::balloon::slot_vertical(0, 200, 800), (5, 189));
    let wide_ref = sims_ui::balloon::slot_vertical(0, 200, 1024);
    assert_eq!(wide_ref, (5, 139));
    for w in [1025u32, 1280, 1440, 1600, 1920] {
        assert_eq!(
            sims_ui::balloon::slot_vertical(0, 200, w),
            wide_ref,
            "balloon slot 0 must stay continuous at width {w}"
        );
        assert_eq!(
            sims_ui::balloon::slot_vertical(1, 200, w),
            (5, 189),
            "balloon slot 1 must stay stable at width {w}"
        );
    }
}

#[test]
fn radio_groups_and_buy_strip_height_invariants_hold_across_all_viewports() {
    let snap = sample_snapshot();
    for size in VIEWPORTS {
        for mode in [Mode::Live, Mode::Buy, Mode::Build, Mode::Options] {
            for roof in [false, true] {
                for walls in [Walls::Down, Walls::Cutaway, Walls::Up] {
                    let controls = Controls {
                        mode,
                        expanded: true,
                        camera: HudCamera {
                            roof,
                            walls,
                            ..HudCamera::default()
                        },
                        ..Controls::default()
                    };
                    let items = hud::layout(size, &snap, &controls);

                    // 1. `{ROOF, WALLS_UP, CUTAWAY, WALLS_DOWN}` always has EXACTLY 1 selected.
                    let view_selected: Vec<i32> = items
                        .iter()
                        .filter_map(|i| match i.kind {
                            Kind::Button {
                                id, selected: true, ..
                            } if matches!(
                                id,
                                id::ROOF | id::WALLS_UP | id::CUTAWAY | id::WALLS_DOWN
                            ) =>
                            {
                                Some(id)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        view_selected.len(),
                        1,
                        "expected exactly 1 selected view button at {size:?} mode={mode:?} roof={roof} walls={walls:?}, got {view_selected:?}"
                    );

                    // 2. `{LIVE, BUY, BUILD, OPTIONS}` always has <= 1 selected (in fact 1).
                    let mode_selected: Vec<i32> = items
                        .iter()
                        .filter_map(|i| match i.kind {
                            Kind::Button {
                                id, selected: true, ..
                            } if matches!(id, id::LIVE | id::BUY | id::BUILD | id::OPTIONS) => {
                                Some(id)
                            }
                            _ => None,
                        })
                        .collect();
                    assert!(
                        mode_selected.len() <= 1,
                        "expected <= 1 selected mode button at {size:?} mode={mode:?}, got {mode_selected:?}"
                    );

                    // 3. Expanded strip is always standard 100 px `id::STRIP` (4910) with zero 50 px overhang.
                    let strip = items
                        .iter()
                        .find(|i| matches!(i.kind, Kind::Stretched(_)))
                        .expect("expanded strip background");
                    assert_eq!(
                        strip.kind,
                        Kind::Stretched(id::STRIP),
                        "mode {mode:?} at {size:?} must use standard 4910 strip, never 4918"
                    );
                    assert_eq!(strip.rect.height, STRIP);
                    assert_eq!(strip.rect.y, size[1] - STRIP);
                }
            }
        }
    }
}

#[test]
fn real_install_build_catalog_maps_every_objd_word_69_bucket_accurately() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to verify Complete Collection Build catalog");
        return;
    };
    let install = Install::detect(&root).expect("detect Complete Collection install");
    let index = Index::build(&install).expect("build install index");
    let catalog = Catalog::build(&index).expect("build object catalog");
    let build_catalog = BuildCatalog::load(
        &install,
        &index,
        &catalog,
        &[],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );

    let names_for = |tool: BuildTool| -> Vec<String> {
        build_catalog
            .items_for(tool)
            .into_iter()
            .filter(|item| matches!(item.id, BuildItemId::Object(_)))
            .map(|item| item.name.to_ascii_lowercase())
            .collect()
    };

    let doors = names_for(BuildTool::Doors);
    let windows = names_for(BuildTool::Windows);
    let stairs = names_for(BuildTool::Stairs);
    let plants = names_for(BuildTool::Plants);
    let fireplaces = names_for(BuildTool::Fireplace);
    let walls = names_for(BuildTool::Wall);
    let water = names_for(BuildTool::Water);

    assert!(
        doors.len() >= 10
            && doors
                .iter()
                .any(|n| n.contains("door") || n.contains("arch")),
        "BuildTool::Doors must contain real doors, got {} items: {:?}",
        doors.len(),
        &doors[..doors.len().min(8)]
    );
    assert!(
        !doors
            .iter()
            .any(|n| n.contains("stair") || n.contains("fireplace")),
        "BuildTool::Doors must never contain stairs or fireplaces"
    );

    assert!(
        windows.len() >= 10 && windows.iter().any(|n| n.contains("window")),
        "BuildTool::Windows must contain real windows, got {} items: {:?}",
        windows.len(),
        &windows[..windows.len().min(8)]
    );
    assert!(
        !windows.iter().any(|n| n.contains("sycamore")
            || n.contains("willow")
            || n.contains("shrub")
            || n.contains("cypress")),
        "BuildTool::Windows must never contain outdoor plants: {:?}",
        windows
    );

    assert!(
        stairs.len() >= 5 && stairs.iter().any(|n| n.contains("stair")),
        "BuildTool::Stairs must contain real stairs, got {} items: {:?}",
        stairs.len(),
        &stairs[..stairs.len().min(8)]
    );
    assert!(
        !stairs.iter().any(|n| n.contains("fireplace")),
        "BuildTool::Stairs must never contain fireplaces"
    );

    assert!(
        plants.len() >= 15
            && plants.iter().any(|n| n.contains("tree")
                || n.contains("shrub")
                || n.contains("flower")
                || n.contains("cypress")
                || n.contains("oak")
                || n.contains("pine")),
        "BuildTool::Plants must contain real trees/shrubs/flowers, got {} items: {:?}",
        plants.len(),
        &plants[..plants.len().min(8)]
    );

    assert!(
        fireplaces.len() >= 3
            && fireplaces.iter().any(|n| n.contains("fireplace")
                || n.contains("hearth")
                || n.contains("chimney")
                || n.contains("fire")),
        "BuildTool::Fireplace must contain real fireplaces, got {} items: {:?}",
        fireplaces.len(),
        &fireplaces[..fireplaces.len().min(8)]
    );
    assert!(
        !fireplaces
            .iter()
            .any(|n| n.contains("pool") || n.contains("diving") || n.contains("ladder")),
        "BuildTool::Fireplace must never contain pool objects"
    );

    assert!(
        walls.len() >= 5
            && walls
                .iter()
                .any(|n| n.contains("column") || n.contains("fence") || n.contains("arch")),
        "BuildTool::Wall must contain real columns/object fences, got {} items: {:?}",
        walls.len(),
        &walls[..walls.len().min(8)]
    );

    assert!(
        water.len() >= 2
            && water.iter().any(|n| n.contains("ladder")
                || n.contains("diving")
                || n.contains("pool")
                || n.contains("slide")),
        "BuildTool::Water must contain real pool ladders/diving boards/slides, got {} items: {:?}",
        water.len(),
        water
    );

    // Verify OBJD_CTSS (`0x52`), OBJD_GRAPHICS (`0x08`), and OBJD_TTAB (`0x0e`)
    // load real localized CTSS catalog names & descriptions rather than falling
    // back to raw internal IFF chunk labels with empty descriptions.
    let mut total_obj_items = 0usize;
    let mut described_obj_items = 0usize;
    for tool in [
        BuildTool::Doors,
        BuildTool::Windows,
        BuildTool::Stairs,
        BuildTool::Plants,
        BuildTool::Fireplace,
        BuildTool::Wall,
        BuildTool::Water,
    ] {
        for item in build_catalog.items_for(tool) {
            if matches!(item.id, BuildItemId::Object(_)) {
                total_obj_items += 1;
                if !item.description.trim().is_empty() {
                    described_obj_items += 1;
                }
            }
        }
    }
    assert!(
        total_obj_items >= 150,
        "expected >= 150 Complete Collection Build Mode catalog objects, got {total_obj_items}"
    );
    assert!(
        described_obj_items * 100 >= total_obj_items * 95,
        "expected >= 95% of Build Mode objects to have non-empty CTSS descriptions (got {described_obj_items}/{total_obj_items})"
    );
    // Real localized CTSS display names appear (e.g. "Monticello Door",
    // "Privacy Window", "Pool Ladder") rather than internal IFF labels
    // like "Door - Roman Arch".
    assert!(
        doors.iter().any(|n| !n.starts_with("door - ")),
        "expected real CTSS door display names, got: {:?}",
        &doors[..doors.len().min(10)]
    );
    assert!(
        windows.iter().any(|n| !n.starts_with("window - ")),
        "expected real CTSS window display names, got: {:?}",
        &windows[..windows.len().min(10)]
    );
}
