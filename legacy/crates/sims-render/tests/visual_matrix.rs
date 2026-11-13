//! Read-only presentation matrix. Native gameplay parity is a separate gate.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{original_house::OriginalHouse, software, wall_view::Plan};
use sims_runtime::original_session::Session;
use sims_sim::lot_presenter::{
    hover::History,
    lifecycle, visibility,
    walls::{construction::Graph, cutaway::Mask},
};
use std::path::Path;

#[test]
#[ignore = "requires original immutable assets and three private managed snapshots"]
fn managed_households_render_all_rotations_zooms_floors_and_wall_modes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("visual matrix panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_VISUAL_MATRIX_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    std::fs::create_dir(output.join("states"))?;
    let keep_pairs = std::env::var_os("SIMS_VISUAL_MATRIX_KEEP_PAIRS").is_some();
    if keep_pairs {
        std::fs::create_dir(output.join("repeat"))?;
        std::fs::create_dir(output.join("cold"))?;
    }
    let asset_root = root.join("cache/managed-independent-20261022-third/immutable-assets");
    ensure!(!asset_root.join("UserData").exists() && !asset_root.join("UserData2").exists());
    let index = Index::scan(&asset_root)?;
    let mut cases = vec![];
    let mut households = vec![];
    let mut references = vec![];
    for name in ["newbie", "goth", "mashuga"] {
        let source = format!("cache/managed-independent-20261022-third/{name}.json");
        let saved = serde_json::from_slice(&std::fs::read(root.join(&source))?)?;
        let session = Session::restore(&index, saved)?;
        let baseline = session.save_managed()?;
        let before = serde_json::to_vec(&session.save())?;
        std::fs::write(output.join(format!("states/{name}.json")), &before)?;
        std::fs::write(
            output.join(format!("states/{name}-resources.json")),
            serde_json::to_vec(&session.save().source_hashes)?,
        )?;
        let cold = Session::restore(&index, baseline)?;
        ensure!(
            before == serde_json::to_vec(&cold.save())?,
            "{name}: cold state differs"
        );
        let mut renderer = OriginalHouse::default();
        let mut cold_renderer = OriginalHouse::default();
        let lot = session
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("original lot missing")?;
        let graph = Graph::from_lot(lot).map_err(anyhow::Error::msg)?;
        let mut count = 0;
        for level in 1..=2 {
            for rotation in 0..4 {
                let mut rotated = graph.clone();
                rotated.rotate(rotation).map_err(anyhow::Error::msg)?;
                for zoom in 1..=3 {
                    let camera = OriginalHouse::camera(
                        &session,
                        640,
                        480,
                        rotation,
                        (1u32 << zoom) as f32 / 8.,
                        level,
                    );
                    for mode in ["walls-up", "walls-down", "cutaway-first-room"] {
                        let view = lifecycle::State {
                            level,
                            all_walls: u8::from(mode == "walls-down"),
                            history: History::house_view(),
                            visibility: visibility::State {
                                enabled: u8::from(mode == "cutaway-first-room"),
                                dirty: 1,
                                include_visible: 0,
                                mask: room_mask(lot, level, rotation),
                            },
                        };
                        let plan = Plan::prepare(&rotated, &view, 0, level)?;
                        let scene = renderer.scene_with_wall_plan(&session, camera, Some(&plan))?;
                        let key =
                            format!("{name}-floor{level}-rotation{rotation}-zoom{zoom}-{mode}");
                        validate_scene(&scene).with_context(|| key.clone())?;
                        // Different clear colors expose holes in the backdrop
                        // independently of the normal background color.
                        let frame = software::render(&scene);
                        let alternate =
                            software::render_with_background(&scene, [255, 0, 255, 255]);
                        let inputs = json!({"key":key,"household":name,"source":session.house.source,
                            "floor":level,"rotation":rotation,"native_zoom":zoom,"wall_mode":mode,
                            "camera_center":camera.center.to_array(),"world_scale":camera.zoom,"viewport":[640,480],
                            "tick":session.simulation.vm.tick,"cutaway_mask_rows":view.visibility.mask.rows,
                            "retained_state":format!("states/{name}.json"),"resource_manifest":format!("states/{name}-resources.json")});
                        if frame != alternate {
                            save_failure(
                                &output,
                                &inputs,
                                "uncovered background pixels",
                                &frame,
                                &alternate,
                            )?;
                            anyhow::bail!("{key}: uncovered background pixels");
                        }
                        let repeat = software::render(&scene);
                        if frame != repeat {
                            save_failure(
                                &output,
                                &inputs,
                                "repeat render differs",
                                &frame,
                                &repeat,
                            )?;
                            anyhow::bail!("{key}: repeat render differs");
                        }
                        let restored_scene =
                            cold_renderer.scene_with_wall_plan(&cold, camera, Some(&plan))?;
                        let cold_frame = software::render(&restored_scene);
                        if frame != cold_frame {
                            save_failure(
                                &output,
                                &inputs,
                                "cold render differs",
                                &frame,
                                &cold_frame,
                            )?;
                            anyhow::bail!("{key}: cold render differs");
                        }
                        frame.save(output.join(format!("{key}.png")))?;
                        if keep_pairs {
                            repeat.save(output.join(format!("repeat/{key}.png")))?;
                            cold_frame.save(output.join(format!("cold/{key}.png")))?;
                        }
                        cases.push(json!({
                            "key": key, "household": name, "source": session.house.source,
                            "floor":level, "rotation": rotation, "native_zoom":zoom,
                            "wall_mode":mode, "camera_center":camera.center.to_array(),
                            "capture_inputs":inputs,"pair_images_retained":keep_pairs,
                            "viewport":[640,480], "tick":session.simulation.vm.tick,
                            "finite_geometry":true, "valid_resources":true,
                            "no_uncovered_background":true, "cold_pixels_equal":true,
                            "repeat_pixels_equal":true, "vertices":scene.vertices.len(),
                            "draws":scene.draws.len(), "reference_class":"missing_matched_original"
                        }));
                        count += 1;
                    }
                }
            }
        }
        ensure!(
            before == serde_json::to_vec(&session.save())?,
            "{name}: drawing changed state"
        );
        ensure!(
            before == serde_json::to_vec(&cold.save())?,
            "{name}: cold drawing changed state"
        );
        if name == "newbie" {
            references = original_references(&root, &output, &session, &mut renderer)?;
            ensure!(before == serde_json::to_vec(&session.save())?);
        }
        households.push(json!({"household":name,"source_checkpoint":source,"source_hashes":session.save().source_hashes,
            "cases":count,"full_session_unchanged":true,"cold_session_equal":true,
            "original_starting_save_namespaces_absent":true}));
        eprintln!("{name}: {count} matrix views passed");
    }
    ensure!(cases.len() == 216);
    std::fs::write(
        output.join("matrix.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":1, "scope":"Read-only visual regression, not original pixel or gameplay parity",
            "reference_policy":"A strict reference requires matching source/scene state, camera, zoom, floor and wall inputs. File names alone are not provenance.",
            "cutaway_policy":"Explicit first indoor room corner mask; native wall-plan computation, not a claim of original mouse-mask production.",
            "households":households,"cases":cases,"original_references":references,
            "strict_scene_reference_matches":0,"original_parity_ticks_added":0
        }))?,
    )?;
    Ok(())
}

fn save_failure(
    output: &Path,
    inputs: &serde_json::Value,
    reason: &str,
    expected: &image::RgbaImage,
    actual: &image::RgbaImage,
) -> Result<()> {
    let directory = output.join("failure");
    std::fs::create_dir(&directory)?;
    expected.save(directory.join("expected.png"))?;
    actual.save(directory.join("actual.png"))?;
    std::fs::write(
        directory.join("failure.json"),
        serde_json::to_vec_pretty(&json!({"case":inputs,"reason":reason}))?,
    )?;
    Ok(())
}

fn original_references(
    root: &Path,
    output: &Path,
    session: &Session,
    renderer: &mut OriginalHouse,
) -> Result<Vec<serde_json::Value>> {
    let lot = session
        .simulation
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap();
    let mut references = vec![];
    for (name, context) in [
        ("walls-up", "original-walls-up.json"),
        ("walls-down", "original-walls-last.json"),
        ("cutaway", "original-live2.json"),
    ] {
        let context_path = format!("cache/visual-geometry-20261022/{context}");
        let reference: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(&context_path))?)?;
        let input = serde_json::from_value(reference["camera"].clone())?;
        let level = reference["level"].as_i64().context("reference level")? as i32;
        let camera = sims_render::Camera::from_native(input, level)?;
        let mut graph = Graph::from_lot(lot).map_err(anyhow::Error::msg)?;
        graph.rotate(camera.rotation).map_err(anyhow::Error::msg)?;
        let view = lifecycle::State {
            level,
            all_walls: reference["all_walls"].as_u64().context("reference walls")? as u8,
            history: History::house_view(),
            visibility: visibility::State {
                enabled: reference["cutaway"].as_u64().context("reference cutaway")? as u8,
                dirty: 1,
                include_visible: 0,
                mask: Mask {
                    first_row: 0,
                    rows: serde_json::from_value(reference["cutaway_mask"].clone())?,
                },
            },
        };
        let storage = reference["view_storage"]
            .as_str()
            .context("reference storage")?;
        let roof = u8::from_str_radix(&storage[0x51 * 2..0x52 * 2], 16)?;
        let plan = Plan::prepare(&graph, &view, roof, level)?;
        let directions = [
            [0, -1],
            [0, 1],
            [-1, 0],
            [1, 0],
            [-1, -1],
            [1, 1],
            [1, -1],
            [-1, 1],
        ];
        ensure!(
            camera.rotation == 0,
            "reference wall coordinate adapter requires rotation0"
        );
        let mut segments = 0;
        for floor in reference["floors"].as_array().context("reference floors")? {
            for edge in floor["edges"].as_array().context("reference edges")? {
                let direction = directions[edge["kind"].as_u64().unwrap() as usize];
                let mut a: [i32; 3] = serde_json::from_value(edge["ends"][0].clone())?;
                for segment in edge["segments"].as_array().unwrap() {
                    let b = [a[0] + direction[0], a[1] + direction[1], a[2]];
                    let actual = plan.segment(a[2], [a[0], a[1]], [b[0], b[1]])?;
                    ensure!(
                        u64::from(actual.style) == segment["style"].as_u64().unwrap()
                            && u64::from(actual.flags) == segment["flags"].as_u64().unwrap(),
                        "{name}: original wall flags differ at {a:?}"
                    );
                    segments += 1;
                    a = b;
                }
            }
        }
        let scene = renderer.scene_with_wall_plan(session, camera, Some(&plan))?;
        validate_scene(&scene)?;
        software::render(&scene).save(output.join(format!("reference-newbie-{name}-rust.png")))?;
        let image_path = format!("cache/visual-geometry-20261022/original-{name}.png");
        std::fs::copy(
            root.join(&image_path),
            output.join(format!("reference-newbie-{name}-original.png")),
        )?;
        references.push(json!({"household":"newbie","wall_mode":name,"context":context_path,
            "original_image":image_path,"camera":reference["camera"],"floor":level,
            "classification":"matched_presentation_inputs_only",
            "original_tick":reference["tick"],"rust_tick":session.simulation.vm.tick,
            "reason":"Source household/camera/zoom/floor/wall inputs match, but gameplay states and ticks differ; this is not strict whole-scene pixel equality.",
            "native_wall_segments_compared":segments,"native_wall_flags_equal":true}));
    }
    Ok(references)
}

fn room_mask(lot: &sims_sim::lot_geometry::Lot, level: i32, rotation: u8) -> Mask {
    let floor = &lot.floors[(level - 1) as usize];
    let room = floor
        .metrics
        .iter()
        .find(|(_, value)| !value.outdoors)
        .map(|(&id, _)| id);
    let mut mask = Mask {
        first_row: 0,
        rows: vec![0; 64],
    };
    if let Some(room) = room {
        for (cell, &id) in floor
            .rooms
            .primary
            .iter()
            .enumerate()
            .filter(|(_, id)| **id == room)
        {
            let _ = id;
            let x = cell / lot.size;
            let y = cell % lot.size;
            for (x, y) in [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)] {
                let (x, y) = match rotation {
                    1 => (64 - y, x),
                    2 => (64 - x, 64 - y),
                    3 => (y, 64 - x),
                    _ => (x, y),
                };
                if x < 64 && y < 64 {
                    mask.rows[y] |= 1u64 << x;
                }
            }
        }
    }
    mask
}

fn validate_scene(scene: &sims_render::Scene) -> Result<()> {
    ensure!(scene.width > 0 && scene.height > 0);
    for vertex in &scene.vertices {
        ensure!(
            vertex
                .position
                .iter()
                .chain(&vertex.uv)
                .chain(&vertex.tint)
                .all(|x| x.is_finite())
                && vertex.depth_scale.is_finite(),
            "nonfinite vertex"
        );
        for depth in [vertex.position[2], vertex.position[2] + vertex.depth_scale] {
            ensure!(
                (-f32::EPSILON..=1. + f32::EPSILON).contains(&depth),
                "depth outside clip interval"
            );
        }
    }
    for draw in &scene.draws {
        ensure!(
            draw.vertices.start <= draw.vertices.end
                && draw.vertices.end as usize <= scene.vertices.len()
                && (draw.vertices.end - draw.vertices.start) % 3 == 0,
            "invalid triangle range"
        );
        let texture = scene
            .textures
            .get(draw.texture)
            .context("missing draw texture")?;
        let pixels = texture.width as usize * texture.height as usize;
        ensure!(
            pixels > 0 && texture.rgba.len() == pixels * 4 && texture.depth.len() == pixels,
            "invalid texture storage"
        );
    }
    Ok(())
}
