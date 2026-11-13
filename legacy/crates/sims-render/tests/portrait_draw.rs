use anyhow::{Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::portrait::{Drawable, Surface, resize::Format};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::pose::{
    mesh::Pose,
    model::Matrix,
    portrait::{Camera, view::Input},
};

#[test]
#[ignore = "requires private original portrait-preparation capture and assets; CPU rendering"]
fn original_head_draw_restores_geometry_and_presentation_after_success_and_failure() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved: Saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-portrait-prepared-session.json"),
    )?)?;
    let mut gallery = image::RgbaImage::new(800, 200);
    let mut native = vec![];
    let mut resize_inputs = vec![];
    let mut resize_results = vec![];
    let mut portraits = image::RgbaImage::new(5 * 110, 2 * 50);
    for rotation in 0..5 {
        let mut session = Session::restore(&index, saved.clone())?;
        let id = 196;
        let selected =
            session
                .host
                .prepare_portrait(&mut session.simulation, &session.program, id)?;
        ensure!(selected.len() == 1);
        let (binding, part) = selected[0];
        let before = session.save();
        let mut resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&before)?)?,
        )?;
        let input = Input {
            viewport: [0, 0, 800, 600],
            center: [344, 232],
            zoom: 1,
            rotation: rotation % 4,
        };
        let view = input.build().map_err(anyhow::Error::msg)?;
        let camera = Camera {
            inverse: view.inverse,
            rect: [0, 0, 200, 200],
            zoom: input.zoom,
            rotation: input.rotation as i32,
            angles: [0., 0.],
        };
        let transform = camera.transform().map_err(anyhow::Error::msg)?.matrix;
        let mut initial = Matrix::IDENTITY;
        initial.values[12] = 123.;
        initial.kind = 2;
        let states = [(
            binding,
            Drawable {
                matrix: initial,
                mode: 0xf000_0001,
                cached: false,
            },
        )]
        .into_iter()
        .collect();
        // Original view/device matrix and light-direction builders, with an
        // explicit viewport/scroll input. Rasterization and pixel conversion
        // remain software services, not original Windows pixel equivalence.
        let mut surface =
            Surface::for_portrait(states, view, [24, 32, 37, 255]).map_err(anyhow::Error::msg)?;
        if rotation == 4 {
            surface.draw_status.insert(binding, -7);
        }
        let original_states = surface.drawables.clone();
        let mut second = surface.clone();
        let assets = &session.host.characters[&id];
        let mesh = &assets.avatar.avatar.parts[part].mesh;
        let g = &assets.geometry.state.bindings[binding];
        let published = &assets.geometry.vertices[&binding];
        let hierarchy = &session.simulation.characters[&id].character.hierarchy;
        let poses = mesh
            .bindings
            .iter()
            .map(|r| {
                let b = g.bones[r.bone];
                Pose::from_absolute(hierarchy.absolute[b], hierarchy.updates[b])
                    .map_err(anyhow::Error::msg)
            })
            .collect::<Result<Vec<_>>>()?;
        let input = json!({"begin":0,"end":0,"order":[0],"models":[{"matrix":initial,"mode":0xf000_0001u32}],"draws":[if rotation==4{-7}else{0}],"matrix":transform,
            "vertices":{"rest":false,"cached_drawable":false,"flags":g.drawable.flags,"buffer_flags":published.buffer_flags,
                "binding_bounds":g.bounds.minimum.iter().chain(&g.bounds.maximum).copied().collect::<Vec<_>>(),
                "samples":mesh.vertices.iter().map(|v|v.position.iter().chain(&v.normal).copied().collect::<Vec<_>>()).chain(mesh.blends.iter().map(|v|v.position.iter().chain(&v.normal).copied().collect::<Vec<_>>())).collect::<Vec<_>>(),
                "initial_vertices":published.vertices.iter().map(|v|v.position.iter().chain(&v.normal).chain(&v.uv).copied().collect::<Vec<_>>()).collect::<Vec<_>>(),
                "bindings":mesh.bindings.iter().enumerate().map(|(i,b)|[b.bone,b.first_vertex,b.vertex_count,b.first_blend,b.blend_count,i]).collect::<Vec<_>>(),
                "bones":poses.iter().map(|p|json!({"position":p.position,"rotation":p.rotation})).collect::<Vec<_>>(),
                "blends":mesh.blends.iter().map(|b|[(b.weight*32768.) as i32,b.vertex as i32]).collect::<Vec<_>>()}});
        let outcome = session.host.draw_portrait(
            &mut session.simulation,
            &session.program,
            id,
            transform,
            &mut surface,
        )?;
        ensure!(
            outcome
                == resumed.host.draw_portrait(
                    &mut resumed.simulation,
                    &resumed.program,
                    id,
                    transform,
                    &mut second
                )?
        );
        ensure!(outcome.copy && outcome.success == (rotation != 4));
        ensure!(surface.drawables == original_states && second.drawables == original_states);
        ensure!(surface.uploads == vec![binding, binding]);
        ensure!(surface.frame == second.frame);
        for (row, format) in [Format::Rgb555, Format::Rgb565].into_iter().enumerate() {
            let native_format = row + 1;
            // The source display surface key flag is independent of the clear
            // color. Exercise both states without assuming captured UI state.
            let background = format.encode([24, 32, 37]);
            for key in [None, Some(background)] {
                let (pixels, input) = surface
                    .image_input(format, [105, 41], None, [2; 4])
                    .map_err(anyhow::Error::msg)?;
                let output = surface
                    .image(format, key, [105, 41], None, [2; 4])
                    .map_err(anyhow::Error::msg)?
                    .unwrap();
                ensure!(
                    Some(output.clone())
                        == second
                            .image(format, key, [105, 41], None, [2; 4])
                            .map_err(anyhow::Error::msg)?
                );
                resize_inputs.push(json!({"format":native_format,"width":200,"height":200,"padding":0,"pixels":pixels,
                    "scale":input.scale,"rect":input.rect,"key":key,"alpha":false}));
                resize_results.push(output.clone());
                if key.is_none() {
                    ensure!(output.width <= 110 && output.height <= 50);
                    for y in 0..output.height {
                        for x in 0..output.width {
                            let [r, g, b] = format.decode(output.pixels[y * output.width + x]);
                            portraits.put_pixel(
                                rotation * 110 + x as u32,
                                row as u32 * 50 + y as u32,
                                image::Rgba([r, g, b, 255]),
                            );
                        }
                    }
                }
            }
        }
        let crop = surface
            .crop([105, 41], None, [2; 4])
            .map_err(anyhow::Error::msg)?;
        ensure!(
            crop == second
                .crop([105, 41], None, [2; 4])
                .map_err(anyhow::Error::msg)?
        );
        if rotation == 4 {
            // An empty draw fails the native border scan and retains the full
            // scratch surface. The crop's aspect expansion cannot enlarge it.
            ensure!(crop.rect == [0, 0, 200, 200]);
        } else {
            // The wide target aspect may expand the crop to the full scratch
            // width. Check retained content and removed vertical margins.
            ensure!(crop.rect[3] - crop.rect[1] < 200);
            for (x, y, pixel) in surface.frame.as_ref().unwrap().enumerate_pixels() {
                if pixel.0 != surface.background {
                    ensure!((crop.rect[0]..crop.rect[2]).contains(&(x as i32)));
                    ensure!((crop.rect[1]..crop.rect[3]).contains(&(y as i32)));
                }
            }
        }
        ensure!(
            session.simulation == before.simulation.state
                && resumed.simulation == session.simulation
        );
        ensure!(session.host.snapshot() == resumed.host.snapshot());
        let current = &session.host.characters[&id].geometry;
        for (&other, vertices) in &before.assets.characters[&id].geometry.vertices {
            if other != binding {
                ensure!(&current.vertices[&other] == vertices);
            }
        }
        let final_saved = session.save();
        let final_restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&final_saved)?)?,
        )?;
        ensure!(
            final_restored.simulation == session.simulation
                && final_restored.host.snapshot() == session.host.snapshot()
        );
        let g = &current.state.bindings[binding];
        let vertices = &current.vertices[&binding];
        native.push(json!({"label":format!("view-{rotation}"),"input":input,"rust_outcome":outcome,
            "rust_final":[{"matrix":surface.drawables[&binding].matrix,"mode":surface.drawables[&binding].mode,"flags":g.drawable.flags,"buffer_flags":vertices.buffer_flags,
                "bounds":g.drawable.bounds.minimum.iter().chain(&g.drawable.bounds.maximum).copied().collect::<Vec<_>>(),
                "vertices":vertices.vertices.iter().map(|v|v.position.iter().chain(&v.normal).chain(&v.uv).copied().collect::<Vec<_>>()).collect::<Vec<_>>()}]}));
        if rotation == 4 {
            let assets = &session.host.characters[&id];
            let mut no_head = assets.avatar.clone();
            for p in &mut no_head.avatar.parts {
                if p.attachment == "HEAD" {
                    p.attachment = "Head".into();
                }
            }
            let mut geometry = assets.geometry.clone();
            let frame = surface.frame.clone();
            let uploads = surface.uploads.clone();
            let result = sims_content::character_portrait::render(
                &mut geometry,
                &no_head,
                &session.simulation.characters[&id],
                transform,
                &mut surface,
            )?;
            ensure!(!result.success && !result.copy && geometry == assets.geometry);
            ensure!(
                surface.frame == frame
                    && surface.uploads == uploads
                    && surface.drawables == original_states
            );
        }
        if rotation < 4 {
            let frame = surface.frame.as_ref().unwrap();
            ensure!(frame.pixels().any(|p| p.0 != [24, 32, 37, 255]));
            image::imageops::replace(&mut gallery, frame, i64::from(rotation) * 200, 0);
        }
        println!(
            "view {rotation}: native rest/restore sequence, draw status, state, pixels and crop {:?} survive restore",
            crop.rect
        );
    }
    gallery.save(root.join("cache/original-portrait-rest-draw.png"))?;
    portraits.save(root.join("cache/original-portrait-resized.png"))?;
    std::fs::write(
        root.join("cache/original-portrait-resize-inputs.json"),
        serde_json::to_vec(&resize_inputs)?,
    )?;
    std::fs::write(
        root.join("cache/original-portrait-resize-results.json"),
        serde_json::to_vec(&resize_results)?,
    )?;
    std::fs::write(
        root.join("cache/original-portrait-draw-inputs.json"),
        serde_json::to_vec(&native)?,
    )?;
    Ok(())
}
