use sims_render::{Scene, Texture, software::rgba_scene};
use std::sync::Arc;
#[path = "support/offscreen.rs"]
#[allow(dead_code)]
mod offscreen;

#[test]
#[ignore = "offscreen GPU; no original assets or window required"]
fn changed_texture_slots_render_exactly_like_a_fresh_gpu() -> anyhow::Result<()> {
    let mut base = rgba_scene(8, 8, [255, 0, 0, 255].repeat(64))?;
    base.vertices.iter_mut().for_each(|v| v.position[2] = 0.1);
    Arc::make_mut(&mut base.textures).push(Texture {
        width: 1,
        height: 1,
        rgba: vec![0, 255, 0, 255].into(),
        depth: vec![255].into(),
    });
    let mut behind = base.vertices.clone();
    behind.iter_mut().for_each(|v| v.position[2] = 0.3);
    base.vertices.extend(behind);
    base.draws.push(sims_render::Draw {
        vertices: 6..12,
        texture: 1,
        object: None,
    });
    let mut scenes = vec![base.clone(), base.clone()];
    // New Vec owner with byte-identical entries.
    scenes[1].textures = Arc::new(base.textures.as_ref().clone());
    let mut color = base.clone();
    Arc::make_mut(&mut color.textures)[0].rgba = [0, 0, 255, 255].repeat(64).into();
    scenes.push(color.clone());
    let mut depth = color.clone();
    Arc::make_mut(&mut depth.textures)[0].depth = vec![0; 64].into();
    depth.vertices.iter_mut().for_each(|v| v.depth_scale = 0.4);
    scenes.push(depth);
    let mut swap = base.clone();
    Arc::make_mut(&mut swap.textures).swap(0, 1);
    scenes.push(swap);
    let mut shrink = color.clone();
    Arc::make_mut(&mut shrink.textures).truncate(1);
    shrink.draws.truncate(1);
    scenes.push(shrink);
    let mut grow = color;
    Arc::make_mut(&mut grow.textures).push(Texture {
        width: 1,
        height: 2,
        rgba: [255, 255, 0, 255].repeat(2).into(),
        depth: vec![255; 2].into(),
    });
    grow.draws[0].texture = 2;
    scenes.push(grow);
    let mut reallocated = base.clone();
    reallocated.textures = Arc::new(
        base.textures
            .iter()
            .map(|texture| Texture {
                width: texture.width,
                height: texture.height,
                rgba: texture.rgba.to_vec().into(),
                depth: texture.depth.to_vec().into(),
            })
            .collect(),
    );
    assert!(!Arc::ptr_eq(
        &reallocated.textures[0].rgba,
        &base.textures[0].rgba
    ));
    scenes.push(reallocated);
    scenes.push(base);
    let refs: Vec<&Scene> = scenes.iter().collect();
    let warm = offscreen::render_sequence_at(&refs, [8, 8], [0., 0., 8., 8.])?;
    for (i, scene) in scenes.iter().enumerate() {
        assert_eq!(warm[i], offscreen::render(scene)?, "texture transition {i}");
    }
    assert_eq!(
        warm[0], warm[7],
        "fresh payload allocations with equal bytes must render identically"
    );
    assert_eq!(
        warm[0], warm[8],
        "retained original scene must survive all transitions"
    );
    assert_ne!(warm[0], warm[2], "color change must be visible");
    assert_ne!(warm[2], warm[3], "depth change must change occlusion");
    assert_ne!(warm[0], warm[4], "slot reorder must be visible");
    assert_ne!(warm[5], warm[6], "new drawn slot must be visible");
    Ok(())
}
