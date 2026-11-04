//! Actual native household projection and ordinary visibility, with a neutral
//! diagnostic material/light. Live marker ownership/color remain separate.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_content::{
    Index,
    character_geometry::CharacterGeometry,
    graphic_materials::GraphicMaterials,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_render::{Draw, Scene, Texture, Vertex};
use sims_sim::pose::{
    graphic_frame::State,
    graphic_visibility,
    model::{Bound, Matrix},
    model_visibility,
};

fn frame(
    source: &Value,
    case: &Value,
    index: &Index,
    materials: Option<&GraphicMaterials>,
) -> Result<image::RgbaImage> {
    let bound: Bound = serde_json::from_value(source["bound"].clone())?;
    let geometry: CharacterGeometry = serde_json::from_value(source["geometry"].clone())?;
    let binding: BindingState = serde_json::from_value(source["asset_state"].clone())?;
    let avatar = BoundAvatar::restore(
        index,
        source["skeleton"].as_str().unwrap(),
        source["owner"].as_u64().unwrap(),
        &binding,
    )?;
    let mut state: State = serde_json::from_value(source["frame"].clone())?;
    let camera: Matrix = serde_json::from_value(case["input"]["camera"].clone())?;
    let viewport: [i32; 4] = serde_json::from_value(case["input"]["viewport"].clone())?;
    let visible =
        graphic_visibility::evaluate(&mut state, 0, true, Some(&bound), viewport, || Ok(camera))
            .map_err(anyhow::Error::msg)?;
    ensure!(Value::from(visible) == case["rust"]["visible"]);
    let mut scene = Scene {
        width: 640,
        height: 480,
        vertices: vec![],
        draws: vec![],
        censors: vec![],
        textures: Default::default(),
    };
    if visible == 0 {
        return Ok(sims_render::software::render(&scene));
    }
    let projection = model_visibility::project(
        bound.local_bounds,
        bound.model.scale,
        state.matrix,
        camera,
        viewport,
    )
    .map_err(anyhow::Error::msg)?
    .matrix;
    let mut textures = vec![];
    for (&identity, &appearance) in &geometry.appearances {
        let range = avatar
            .part_ranges
            .get(&identity)
            .context("marker parts absent")?;
        let bindings = &geometry.state.appearances[appearance];
        ensure!(bindings.len() == range.len());
        for (&binding, part) in bindings.iter().zip(range.clone()) {
            let part = &avatar.avatar.parts[part];
            let diffuse = if let Some(materials) = materials {
                ensure!(
                    part.builtin_material.is_some(),
                    "diagnostic material renderer requires built-in colors"
                );
                let words = materials.published_for_binding(binding)?;
                [words[1], words[2], words[3], words[4]].map(f32::from_bits)
            } else {
                [1.; 4]
            };
            let vertices = &geometry.vertices[&binding].vertices;
            let start = scene.vertices.len() as u32;
            for triangle in &part.mesh.triangles {
                for &index in triangle {
                    let vertex = vertices
                        .get(index as usize)
                        .context("marker vertex absent")?;
                    let [x, y, z] = projection
                        .point(vertex.position)
                        .map_err(anyhow::Error::msg)?;
                    let normal = glam::Vec3::from_array(vertex.normal).normalize_or_zero();
                    let light = glam::Vec3::new(-0.4, 0.8, -0.5).normalize();
                    let shade = 0.3 + 0.7 * normal.dot(light).max(0.);
                    scene.vertices.push(Vertex {
                        position: [x / 320. - 1., 1. - y / 240., z / 65535.],
                        uv: vertex.uv,
                        tint: [
                            diffuse[0] * shade,
                            diffuse[1] * shade,
                            diffuse[2] * shade,
                            diffuse[3],
                        ],
                        depth_scale: 0.,
                    });
                }
            }
            scene.draws.push(Draw {
                vertices: start..scene.vertices.len() as u32,
                texture: textures.len(),
                object: None,
            });
            textures.push(if materials.is_some() {
                Texture {
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4].into(),
                    depth: vec![255].into(),
                }
            } else {
                Texture {
                    width: part.texture.width,
                    height: part.texture.height,
                    rgba: part.texture.rgba.clone().into(),
                    depth: vec![255; part.texture.width as usize * part.texture.height as usize].into(),
                }
            });
        }
    }
    scene.textures = std::sync::Arc::new(textures);
    Ok(sims_render::software::render(&scene))
}

fn main() -> Result<()> {
    let report: Value =
        serde_json::from_slice(&std::fs::read("cache/original-indicator-visibility.json")?)?;
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let sources = report["sources"]
        .as_array()
        .context("marker sources absent")?;
    let cases = report["cases"].as_array().context("marker cases absent")?;
    if std::env::args().any(|arg| arg == "--materials") {
        let materials: Value =
            serde_json::from_slice(&std::fs::read("cache/original-indicator-materials.json")?)?;
        let mut gallery = image::RgbaImage::new(9 * 240, 3 * 240);
        let mut checks = vec![];
        for (row, source) in sources.iter().enumerate() {
            let camera = cases
                .iter()
                .filter(|c| c["name"] == source["name"] && c["rust"]["visible"] == 1)
                .max_by_key(|c| c["camera_input"]["zoom"].as_u64().unwrap())
                .context("visible marker camera absent")?;
            let mut previous = None;
            let mut first = None;
            for (column, case) in materials["cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["name"] == source["name"] && c["mode"] == 2)
                .enumerate()
            {
                let owner: GraphicMaterials = serde_json::from_value(case["owner"].clone())?;
                let image = frame(source, camera, &index, Some(&owner))?;
                let restored = serde_json::from_slice(&serde_json::to_vec(&owner)?)?;
                ensure!(
                    image == frame(source, camera, &index, Some(&restored))?,
                    "restored material pixels differ"
                );
                let pixels = image.pixels().filter(|p| p.0 != [24, 32, 37, 255]).count();
                ensure!(pixels > 0);
                if column == 0 {
                    first = Some(image.clone());
                }
                if column == 1 || column == 2 || column == 3 {
                    ensure!(
                        previous.as_ref() != Some(&image),
                        "different native effects produced identical pixels"
                    );
                }
                if column == 8 {
                    ensure!(
                        first.as_ref() == Some(&image),
                        "clear did not restore neutral pixels"
                    );
                }
                checks.push(serde_json::json!({"name":source["name"],"controls":case["input"]["requested"],"pixels":pixels,"restored_identical":true}));
                let crop = image::imageops::crop_imm(&image, 260, 180, 120, 120).to_image();
                let enlarged =
                    image::imageops::resize(&crop, 240, 240, image::imageops::FilterType::Nearest);
                image::imageops::replace(
                    &mut gallery,
                    &enlarged,
                    column as i64 * 240,
                    row as i64 * 240,
                );
                previous = Some(image);
            }
        }
        ensure!(checks.len() == 27);
        gallery.save("cache/original-indicator-materials-gallery.png")?;
        std::fs::write(
            "cache/original-indicator-materials-pixels.json",
            serde_json::to_vec_pretty(&checks)?,
        )?;
        println!(
            "27 native-material diagnostic views; restored pixels identical; effect changes and clear verified"
        );
        return Ok(());
    }
    let mut gallery = image::RgbaImage::new(640 * sources.len() as u32, 960);
    let mut checks = vec![];
    for (column, source) in sources.iter().enumerate() {
        let visible = cases
            .iter()
            .filter(|c| c["name"] == source["name"] && c["rust"]["visible"] == 1)
            .max_by_key(|c| c["camera_input"]["zoom"].as_u64().unwrap())
            .context("visible marker case absent")?;
        let hidden = cases
            .iter()
            .find(|c| {
                c["name"] == source["name"]
                    && c["camera_input"]["rotation"] == visible["camera_input"]["rotation"]
                    && c["camera_input"]["zoom"] == visible["camera_input"]["zoom"]
                    && c["rust"]["visible"] == 0
            })
            .context("hidden marker case absent")?;
        for (row, case) in [visible, hidden].into_iter().enumerate() {
            let image = frame(source, case, &index, None)?;
            let restored_source = serde_json::from_slice(&serde_json::to_vec(source)?)?;
            let restored_case = serde_json::from_slice(&serde_json::to_vec(case)?)?;
            ensure!(
                image == frame(&restored_source, &restored_case, &index, None)?,
                "restored native camera pixels differ"
            );
            let pixels = image.pixels().filter(|p| p.0 != [24, 32, 37, 255]).count();
            ensure!(
                if row == 0 { pixels > 0 } else { pixels == 0 },
                "unexpected culling pixels {pixels}"
            );
            checks.push(serde_json::json!({"name":source["name"],"camera":case["camera_input"],"visible":case["rust"]["visible"],"pixels":pixels,"restored_identical":true}));
            image::imageops::replace(&mut gallery, &image, column as i64 * 640, row as i64 * 480);
        }
    }
    gallery.save("cache/original-indicator-visibility-gallery.png")?;
    std::fs::write(
        "cache/original-indicator-visibility-pixels.json",
        serde_json::to_vec_pretty(&checks)?,
    )?;
    println!(
        "6 native-camera marker views; restored pixels identical; visible draws and offscreen culls verified"
    );
    Ok(())
}
