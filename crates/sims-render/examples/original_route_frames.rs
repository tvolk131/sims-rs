//! Headless route diagnostics over original furnished homes. Gold is the
//! inferred complete plan; teal dots are native physical positions recorded
//! during live execution. These overlays do not claim recovered path selection
//! or a fully imported live avatar/animator.
use anyhow::{Context, Result};
use glam::{Vec2, Vec3};
use sims_content::{Index, saves::House};
use sims_render::{Camera, Draw, Scene, Vertex, frozen_house::FrozenHouse, software};
use sims_sim::{Point, Route};
use std::collections::BTreeMap;
fn ribbon(scene: &mut Scene, camera: Camera, a: Point, b: Point, color: [f32; 4], level: i32) {
    let project = |p: Point| {
        camera.screen(Vec3::new(
            p.x as f32 / 16.,
            (level - 1) as f32 * 3.,
            p.y as f32 / 16.,
        ))
    };
    let a = project(a);
    let b = project(b);
    let delta = b - a;
    let normal = if delta.length() > 0.01 {
        Vec2::new(-delta.y, delta.x).normalize() * 2.
    } else {
        Vec2::new(3., 0.)
    };
    let b = if delta.length() > 0.01 {
        b
    } else {
        b + Vec2::new(0., 6.)
    };
    let start = scene.vertices.len() as u32;
    for p in [
        a - normal,
        a + normal,
        b + normal,
        a - normal,
        b + normal,
        b - normal,
    ] {
        scene.vertices.push(Vertex {
            position: [
                p.x / scene.width as f32 * 2. - 1.,
                1. - p.y / scene.height as f32 * 2.,
                0.001,
            ],
            uv: [0., 0.],
            tint: color,
            depth_scale: 0.,
        });
    }
    // Texture zero is the renderer's untextured white geometry material.
    scene.draws.push(Draw {
        vertices: start..scene.vertices.len() as u32,
        texture: 0,
        object: None,
    });
}
fn main() -> Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        "cache/original-house-inferred-walking.json",
    )?)?;
    let mut sheet = image::RgbaImage::new(3600, 900);
    for (column, row) in report
        .as_array()
        .context("house report array")?
        .iter()
        .enumerate()
    {
        let source = row["source"].as_str().context("house source")?;
        let house = House::load(&index, source)?;
        let frozen = house.load_frozen(
            &index,
            sims_vm::loading::HouseContext {
                simulator: sims_vm::loading::SimulatorContext {
                    game_mode: 0,
                    retained_global25: 0,
                    global30: 1,
                },
                installed_expansions: 255,
                performance_class: 2,
            },
        )?;
        let preview = FrozenHouse::load(&index, &house, &frozen)?;
        let routes: BTreeMap<u64, Route> = serde_json::from_value(row["planned_routes"].clone())?;
        let residents: Vec<_> = house
            .household
            .as_ref()
            .context("household")?
            .residents
            .iter()
            .map(|r| r.object as i16)
            .collect();
        for level in [1, 2] {
            let mut shown: Vec<_> = routes
                .values()
                .filter(|r| residents.contains(&r.caller) && r.target_body.level == level)
                .collect();
            if shown.is_empty() {
                if level == 2 {
                    continue;
                }
                shown.push(routes.values().next().context("planned route")?);
            }
            let lot = frozen.architecture.original.as_ref().unwrap();
            let mut points: Vec<_> = shown.iter().flat_map(|r| r.path.iter().copied()).collect();
            points.extend(
                lot.floors[level as usize - 1]
                    .rooms
                    .primary
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        lot.floors[level as usize - 1]
                            .metrics
                            .get(r)
                            .is_some_and(|m| !m.outdoors)
                    })
                    .map(|(i, _)| {
                        Point::new((i / lot.size) as i32 * 16, (i % lot.size) as i32 * 16)
                    }),
            );
            let minx = points.iter().map(|p| p.x).min().unwrap();
            let maxx = points.iter().map(|p| p.x).max().unwrap();
            let miny = points.iter().map(|p| p.y).min().unwrap();
            let maxy = points.iter().map(|p| p.y).max().unwrap();
            let span = ((maxx - minx + maxy - miny) as f32 / 16. + 4.).max(1.);
            let mut camera = preview.camera(1200, 900, 0, (1100. / (span * 64.)).min(0.6), level);
            camera.center = Vec3::new(
                (minx + maxx) as f32 / 32.,
                (level - 1) as f32 * 3.,
                (miny + maxy) as f32 / 32.,
            );
            let mut scene = preview.scene(camera)?;
            for route in &shown {
                for w in route.path.windows(2) {
                    ribbon(&mut scene, camera, w[0], w[1], [1., 0.74, 0.2, 1.], level);
                }
            }
            let mut shown_samples = 0;
            for sample in row["movement_samples"]
                .as_array()
                .context("movement samples")?
            {
                let mut recorded = false;
                for actor in sample["people"].as_array().unwrap() {
                    if !shown
                        .iter()
                        .any(|r| i64::from(r.caller) == actor["caller"].as_i64().unwrap())
                        || actor["position"]["level"] != level
                    {
                        continue;
                    }
                    let p = &actor["position"];
                    let point = Point::new(
                        p["y"].as_i64().unwrap() as i32,
                        p["x"].as_i64().unwrap() as i32,
                    );
                    ribbon(&mut scene, camera, point, point, [0.1, 1., 0.7, 1.], level);
                    recorded = true;
                }
                shown_samples += usize::from(recorded);
            }
            let frame = software::render(&scene);
            frame.save(format!("cache/original-route-{column}-floor-{level}.png"))?;
            if level == 1 {
                frame.save(format!("cache/original-route-{column}.png"))?;
                image::imageops::replace(&mut sheet, &frame, column as i64 * 1200, 0);
            }
            println!(
                "{source} floor {level}: {} inferred plans, {} path points, {} matching sampled live ticks (frozen avatar preview)",
                shown.len(),
                shown.iter().map(|r| r.path.len()).sum::<usize>(),
                shown_samples
            );
        }
    }
    sheet.save("cache/original-route-scenes.png")?;
    Ok(())
}
