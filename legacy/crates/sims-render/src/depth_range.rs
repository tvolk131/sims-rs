//! Map replacement floating depth into the GPU clip interval without dropping
//! geometry. Native integer sprite lookup/blit semantics remain a separate path.
use crate::{Scene, Vertex};
impl Scene {
    pub(crate) fn fit_depth_range(&mut self) {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for v in &self.vertices {
            for z in [v.position[2], v.position[2] + v.depth_scale] {
                lo = lo.min(z);
                hi = hi.max(z);
            }
        }
        for r in &self.censors {
            lo = lo.min(r.depth);
            hi = hi.max(r.depth);
        }
        if lo >= 0. && hi < 1. || !lo.is_finite() || !hi.is_finite() {
            return;
        }
        // One positive affine transform for every geometry/sprite/censor depth:
        // relative ordering and equal-depth ties have the same meaning.
        let scale = 0.9 / (hi - lo).max(f32::EPSILON);
        let offset = 0.05 - lo * scale;
        for v in &mut self.vertices {
            v.position[2] = v.position[2] * scale + offset;
            v.depth_scale *= scale;
        }
        for r in &mut self.censors {
            r.depth = r.depth * scale + offset;
        }
    }
    pub(crate) fn background(&mut self, texture: usize) {
        let start = self.vertices.len() as u32;
        for [x, y] in [
            [-1., 1.],
            [1., 1.],
            [1., -1.],
            [-1., 1.],
            [1., -1.],
            [-1., -1.],
        ] {
            self.vertices.push(Vertex {
                position: [x, y, 1.],
                uv: [0.5; 2],
                tint: [1.; 4],
                depth_scale: 0.,
            });
        }
        self.draws.insert(
            0,
            crate::Draw {
                vertices: start..self.vertices.len() as u32,
                texture,
                object: None,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn remap_keeps_sprite_occlusion_and_equal_depth_ties() {
        let mut scene = crate::software::rgba_scene(4, 4, [255, 0, 0, 255].repeat(16)).unwrap();
        scene.textures = std::sync::Arc::new(vec![
            crate::Texture {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255].into(),
                depth: vec![0].into(),
            },
            crate::Texture {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 255, 255].into(),
                depth: vec![255].into(),
            },
        ]);
        for v in &mut scene.vertices {
            v.position[2] = -0.5;
            v.depth_scale = 1.;
        }
        let mut far = scene.vertices.clone();
        for v in &mut far {
            v.position[2] = 0.75;
            v.depth_scale = 0.;
        }
        scene.vertices.extend(far);
        scene.draws.push(crate::Draw {
            vertices: 6..12,
            texture: 1,
            object: None,
        });
        let before = crate::software::render(&scene);
        assert!(before.pixels().all(|p| p.0 == [255, 0, 0, 255]));
        scene.censors.push(crate::censor::Region {
            rect: [0, 0, 0, 0],
            depth: -0.25,
            seed: 7,
        });
        scene.fit_depth_range();
        assert!(
            scene
                .vertices
                .iter()
                .all(|v| v.position[2] >= 0. && v.position[2] + v.depth_scale < 1.)
        );
        assert!(scene.censors[0].depth > scene.vertices[0].position[2]);
        assert_eq!(before, crate::software::render(&scene));
        // Equal-depth overwrite keeps the original LessEqual draw order.
        for v in &mut scene.vertices[6..] {
            v.position[2] = scene.censors[0].depth;
        }
        for v in &mut scene.vertices[..6] {
            v.position[2] = scene.censors[0].depth;
            v.depth_scale = 0.;
        }
        assert!(
            crate::software::render(&scene)
                .pixels()
                .all(|p| p.0 == [0, 0, 255, 255])
        );
    }

    #[test]
    fn in_range_depths_stay_bitwise_unchanged_and_background_is_farthest() {
        let mut scene = crate::software::rgba_scene(4, 4, [255, 0, 0, 255].repeat(16)).unwrap();
        let before = bytemuck::cast_slice::<_, u8>(&scene.vertices).to_vec();
        scene.fit_depth_range();
        assert_eq!(before, bytemuck::cast_slice::<_, u8>(&scene.vertices));
        scene.background(0);
        assert_eq!(scene.draws[0].vertices, 6..12);
        assert!(scene.vertices[6..].iter().all(|v| v.position[2] == 1.));
    }
}
