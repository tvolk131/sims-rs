//! Original wall-covering/mask resources and retained native cutaway flags on
//! replacement wall planes. Thickness, caps, lighting and rasterization remain
//! preview approximations. Object-provided apertures retain their own resources.
use crate::{Assets, Camera, Scene, Texture, floor_elevation};
use anyhow::{Context, Result};
use glam::Vec3;
use sims_formats::lot::WallTile;
use std::{collections::BTreeMap, sync::Arc};

impl Assets {
    pub(crate) fn bind_original_walls(
        &mut self,
        index: &sims_content::Index,
        house: &sims_content::saves::House,
        program: &sims_vm::Program,
        definitions: &std::collections::BTreeSet<(String, u16)>,
    ) -> Result<()> {
        let materials = sims_content::surfaces::load_walls(index, house)?;
        let styles = house
            .floors
            .iter()
            .flat_map(|f| &f.walls)
            .flat_map(|w| w.styles)
            .map(|s| s as u8);
        let shapes = sims_content::surfaces::load_wall_shapes(index, styles)?;
        let mut object_shapes = BTreeMap::new();
        for (source, id) in definitions {
            let words = &program.modules[source].object_definition_words[id];
            let base = words.get(0xa2 / 2).copied().unwrap_or(0) as u16;
            if base != 0
                && words.get(0x5c / 2).is_some_and(|s| matches!(*s, 22 | 23))
                && !object_shapes.contains_key(&(source.clone(), base))
            {
                object_shapes.insert(
                    (source.clone(), base),
                    sims_content::surfaces::load_object_wall_shapes(index, source, base)?,
                );
            }
        }
        let textures = Arc::make_mut(&mut self.textures);
        let mut bound = BTreeMap::new();
        for (id, m) in materials {
            let mut coverings = [[0; 4]; 3];
            for (zoom, frames) in m.frames.iter().enumerate() {
                coverings[zoom] = std::array::from_fn(|direction| {
                    let f = &frames[direction];
                    let texture = textures.len();
                    textures.push(Texture {
                        width: f.width,
                        height: f.height,
                        rgba: f.rgba.clone().into(),
                        depth: f.depth.clone().into(),
                    });
                    texture
                });
            }
            for (&(style, cut), masks) in &shapes {
                let mut images = [[0; 4]; 3];
                for zoom in 0..3 {
                    for direction in 0..4 {
                        let f = &m.frames[zoom][direction];
                        let mask = &masks[zoom][direction];
                        anyhow::ensure!(
                            f.width == mask.width && f.height == mask.height,
                            "wall shape and covering dimensions differ"
                        );
                        let mut rgba = f.rgba.clone();
                        for (pixel, shape) in
                            rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4))
                        {
                            pixel[3] = ((u16::from(pixel[3]) * u16::from(shape[3])) / 255) as u8;
                        }
                        images[zoom][direction] = textures.len();
                        textures.push(textures[coverings[zoom][direction]].with_rgba(rgba));
                    }
                }
                self.wall_shapes.insert((id, style, cut), images);
            }
            bound.insert(id, coverings);
            for ((source, base), modes) in &object_shapes {
                for (cut, masks) in modes.iter().enumerate() {
                    let mut images = [[0; 4]; 3];
                    for zoom in 0..3 {
                        for direction in 0..4 {
                            let f = &m.frames[zoom][direction];
                            let mask = &masks[zoom][direction];
                            anyhow::ensure!(
                                (f.width, f.height) == (mask.width, mask.height),
                                "object wall mask dimensions differ: {source} #{base}"
                            );
                            let mut rgba = f.rgba.clone();
                            for (p, m) in rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
                                p[3] = ((u16::from(p[3]) * u16::from(m[3])) / 255) as u8;
                            }
                            images[zoom][direction] = textures.len();
                            textures.push(textures[coverings[zoom][direction]].with_rgba(rgba));
                        }
                    }
                    self.object_wall_shapes
                        .insert((id, source.clone(), *base, cut != 0), images);
                }
            }
        }
        self.wall_materials = Some(bound);
        Ok(())
    }
}

/// Positive/negative normal, for n=(b.z-a.z, 0, a.x-b.x).
/// W/N own their style, but the opposite wallpaper lives in the neighbor's
/// E/S field. Diagonal wall-side selectors are not room-triangle IDs:
/// native53f6c1 pairs the0x10 southeast room with pattern3, northwest with2.
fn faces(cells: &[WallTile], size: usize, x: usize, z: usize, bit: u8) -> [u16; 2] {
    let t = &cells[x * size + z];
    match bit {
        1 => [
            t.patterns[0],
            if x > 0 {
                cells[(x - 1) * size + z].patterns[3]
            } else {
                0
            },
        ],
        2 => [
            if z > 0 {
                cells[x * size + z - 1].patterns[2]
            } else {
                0
            },
            t.patterns[1],
        ],
        16 => [t.patterns[3], t.patterns[2]],
        32 => [t.patterns[3], t.patterns[2]],
        _ => unreachable!(),
    }
}

type SegmentKey = [[i32; 2]; 2];
fn segment_key(a: [i32; 2], b: [i32; 2]) -> SegmentKey {
    if a < b { [a, b] } else { [b, a] }
}
/// Front/back multipart objects address the same physical wall from opposite
/// tiles. Resolve their required wall rather than choosing a nearest segment.
fn object_apertures(
    sim: &sims_sim::Simulation,
    program: &sims_vm::Program,
    level: i32,
) -> Result<BTreeMap<SegmentKey, (String, u16)>> {
    let mut result = BTreeMap::new();
    for (&id, o) in &sim.vm.world.objects {
        let Some(d) = &o.definition else {
            continue;
        };
        let Some(p) = sim.vm.world.positions.get(&id).filter(|p| p.level == level) else {
            continue;
        };
        let Some(words) = program
            .modules
            .get(&d.source)
            .and_then(|m| m.object_definition_words.get(&d.object))
        else {
            continue;
        };
        let base = words.get(0xa2 / 2).copied().unwrap_or(0) as u16;
        if base == 0 || !words.get(0x5c / 2).is_some_and(|s| matches!(*s, 22 | 23)) {
            continue;
        }
        let [x, z, _] = sims_vm::loading::coarse_position(*p).map(i32::from);
        let normal = crate::wall_objects::normal(
            o.data.get(&13).copied().unwrap_or(0) as u16,
            o.data.get(&1).copied().unwrap_or(0),
            0,
        );
        let (a, b) = match normal {
            1 => ([x, z], [x, z + 1]),
            2 => ([x, z], [x + 1, z]),
            4 => ([x + 1, z], [x + 1, z + 1]),
            8 => ([x, z + 1], [x + 1, z + 1]),
            _ => continue,
        };
        let value = (d.source.clone(), base);
        if let Some(prior) = result.insert(segment_key(a, b), value.clone()) {
            anyhow::ensure!(
                prior == value,
                "conflicting object wall masks on one segment"
            );
        }
    }
    Ok(result)
}

impl Scene {
    pub(crate) fn original_walls(
        &mut self,
        materials: Option<&BTreeMap<u16, [[usize; 4]; 3]>>,
        shapes: &BTreeMap<(u16, u8, bool), [[usize; 4]; 3]>,
        object_shapes: &BTreeMap<(u16, String, u16, bool), [[usize; 4]; 3]>,
        presentation: Option<&crate::wall_view::Plan>,
        sim: &sims_sim::Simulation,
        program: &sims_vm::Program,
        camera: &Camera,
        level: i32,
        native_terrain: bool,
    ) -> Result<bool> {
        let Some(materials) = materials else {
            return Ok(false);
        };
        let a = sim
            .architecture
            .as_ref()
            .context("wall materials require architecture")?;
        let lot = a
            .original
            .as_ref()
            .context("wall materials require original lot")?;
        let cells = &lot.floors[(level - 1) as usize].source.walls;
        let y = floor_elevation(level);
        let apertures = object_apertures(sim, program, level)?;
        for (i, t) in cells.iter().enumerate() {
            let (x, z) = (i / lot.size, i % lot.size);
            let base = Vec3::new(x as f32, y, z as f32);
            for (bit, p, q) in [
                (1, Vec3::ZERO, Vec3::Z),
                (2, Vec3::ZERO, Vec3::X),
                (16, Vec3::X, Vec3::Z),
                (32, Vec3::ZERO, Vec3::X + Vec3::Z),
            ] {
                if t.segments & bit == 0 {
                    continue;
                }
                let height_at = |p: Vec3| {
                    if native_terrain {
                        let corner = match (p.x as u8, p.z as u8) {
                            (0, 0) => 1,
                            (1, 0) => 2,
                            (1, 1) => 3,
                            _ => 0,
                        };
                        crate::original_house::terrain_height(f64::from(lot.heights[i][corner]))
                    } else {
                        0.
                    }
                };
                let (p, q) = (
                    base + p + Vec3::Y * height_at(p),
                    base + q + Vec3::Y * height_at(q),
                );
                let mid = (p + q) * 0.5;
                let side = Vec3::new(q.z - p.z, 0., p.x - q.x).normalize() * 0.045;
                let exterior = [mid + side * 2., mid - side * 2.].iter().any(|v| {
                    let room = a.room_at(
                        level,
                        sims_sim::Point::new((v.x * 16.) as i32, (v.z * 16.) as i32),
                    );
                    a.outdoors.get(&room) == Some(&true)
                });
                let r = camera.rotated(mid - camera.center);
                let native = presentation
                    .map(|view| {
                        view.segment(
                            level,
                            [p.x.round() as i32, p.z.round() as i32],
                            [q.x.round() as i32, q.z.round() as i32],
                        )
                    })
                    .transpose()?;
                let cut = native.map_or(!(exterior && r.x + r.z < 0.), |s| s.flags & 1 != 0);
                let height = if cut { 0.22 } else { 3. };
                for (id, offset) in faces(cells, lot.size, x, z, bit)
                    .into_iter()
                    .zip([side, -side])
                {
                    let images = materials
                        .get(&id)
                        .with_context(|| format!("wall material {id} not loaded"))?;
                    let object_mask = apertures
                        .get(&segment_key(
                            [p.x.round() as i32, p.z.round() as i32],
                            [q.x.round() as i32, q.z.round() as i32],
                        ))
                        .and_then(|(source, base)| {
                            object_shapes.get(&(id, source.clone(), *base, cut))
                        });
                    let images = object_mask
                        .or_else(|| native.and_then(|n| shapes.get(&(id, n.style, cut))))
                        .unwrap_or(images);
                    self.wall_face(
                        camera,
                        p + offset,
                        q + offset,
                        height,
                        images[(camera.sprite_zoom().0 - 1) as usize],
                    );
                }
                // Native shape transparency already describes fences/openings.
                // A flat preview cap above transparent geometry would float.
                if native.is_none() {
                    let up = Vec3::Y * height;
                    self.surface(
                        camera,
                        [p - side + up, q - side + up, q + side + up, p + side + up],
                        [0.9, 0.88, 0.8, 1.],
                    );
                }
            }
        }
        Ok(true)
    }

    fn wall_face(&mut self, c: &Camera, mut p: Vec3, mut q: Vec3, height: f32, images: [usize; 4]) {
        if c.screen(p).x > c.screen(q).x {
            std::mem::swap(&mut p, &mut q);
        }
        let slope = (c.screen(q).y - c.screen(p).y) / c.zoom;
        let direction = if slope < -1. {
            0
        } else if slope > 1. {
            1
        } else {
            2
        };
        let texture = images[direction];
        let t = &self.textures[texture];
        let tilt = (4u32 << c.sprite_zoom().0) as f32 / t.height as f32;
        let (bottom, top) = match direction {
            0 => ([1., 1. - tilt], [tilt, 0.]),
            1 => ([1. - tilt, 1.], [0., tilt]),
            _ => ([1., 1.], [0., 0.]),
        };
        let top = [0, 1].map(|i| bottom[i] + (top[i] - bottom[i]) * (height / 3.));
        let up = Vec3::Y * height;
        self.quad(
            [
                c.vertex(p, [0., bottom[0]], [1.; 4]),
                c.vertex(q, [1., bottom[1]], [1.; 4]),
                c.vertex(q + up, [1., top[1]], [1.; 4]),
                c.vertex(p + up, [0., top[0]], [1.; 4]),
            ],
            texture,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn diagonal_faces_match_native_wall_graph_room_ownership() {
        // Native540619/53f6c1 emits both wallpaper and room identity for each
        // edge side. In particular, wall-side selector4 is not room triangle4.
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../sims-sim/tests/fixtures/wall-construction.json"
        ))
        .unwrap();
        for case in &report["cases"].as_array().unwrap()[46..50] {
            let input = &case["input"];
            let walls: Vec<WallTile> = serde_json::from_value(input["walls"].clone()).unwrap();
            let split: sims_sim::lot_rooms::Split =
                serde_json::from_value(input["rooms"]["halves"][0].clone()).unwrap();
            let size = input["size"].as_u64().unwrap() as usize;
            let [x, z] = split.tile;
            let bit = walls[x * size + z].segments;
            // For the renderer's p->q winding, positive normal is southeast
            // for0x10 and northeast for0x20 (native room triangle2 and3).
            let positive_room = split.rooms[&if bit == 16 { 2 } else { 3 }];
            let native_rooms: Vec<Option<i16>> =
                serde_json::from_value(case["original"]["geometry"]["rooms"][0].clone()).unwrap();
            let native_patterns: [u16; 2] = serde_json::from_value(
                case["original"]["before"]["edges"][0]["segments"][0]["patterns"].clone(),
            )
            .unwrap();
            let positive = native_rooms
                .iter()
                .position(|room| *room == Some(positive_room))
                .unwrap();
            assert_eq!(
                faces(&walls, size, x, z, bit),
                [native_patterns[positive], native_patterns[1 - positive]],
                "wall{bit}: positive-normal room{positive_room}"
            );
        }
    }
    #[test]
    fn headless_pixels_show_each_side_of_orthogonal_and_diagonal_walls() {
        use sims_sim::{
            Room, Simulation,
            lot_geometry::{FloorSource, Lot},
        };
        use sims_vm::{Program, Vm};
        let colors = [[255, 255, 255, 255], [255, 0, 0, 255], [0, 0, 255, 255]];
        let textures = Arc::new(
            colors
                .iter()
                .map(|v| Texture {
                    width: 1,
                    height: 1,
                    rgba: v.to_vec().into(),
                    depth: vec![255].into(),
                })
                .collect(),
        );
        let materials = BTreeMap::from([(1, [[1; 4]; 3]), (2, [[2; 4]; 3])]);
        for (bit, p, q, positive) in [
            (1, Vec3::ZERO, Vec3::Z, 1),
            (2, Vec3::ZERO, Vec3::X, 2),
            (16, Vec3::X, Vec3::Z, 2),
            (32, Vec3::ZERO, Vec3::X + Vec3::Z, 2),
        ] {
            let mut cells = vec![WallTile::default(); 16];
            cells[5].segments = bit;
            cells[5].styles = [1, 1];
            cells[5].patterns = [1, 1, 1, 2];
            if bit == 1 {
                cells[1].segments = 4;
                cells[1].patterns[3] = 2;
            }
            if bit == 2 {
                cells[4].segments = 8;
                cells[4].patterns[2] = 2;
            }
            let blank = FloorSource {
                coverings: vec![0; 16],
                walls: vec![WallTile::default(); 16],
                object_heads: vec![0; 16],
                flags: vec![0; 16],
            };
            let floor = FloorSource {
                walls: cells,
                ..blank.clone()
            };
            let lot = Lot::rebuild(
                4,
                vec![[10; 4]; 16],
                vec![255; 16],
                vec![255; 16],
                vec![floor, blank],
            )
            .unwrap();
            let mut sim = Simulation::new(
                Vm::new(Default::default(), 1),
                Room {
                    width: 64,
                    height: 64,
                    obstacles: vec![],
                },
                Default::default(),
            )
            .unwrap();
            sim.set_original_lot(&Program::default(), lot).unwrap();
            let (p, q) = (p + Vec3::new(1., 0., 1.), q + Vec3::new(1., 0., 1.));
            let normal = Vec3::new(q.z - p.z, 0., p.x - q.x).normalize();
            let mid = (p + q) * 0.5;
            for rotation in 0..4 {
                let mut c = Camera::room(&sim, 400, 300, rotation, 2.);
                c.center = mid;
                if (c.screen(p).x - c.screen(q).x).abs() < 1. {
                    continue;
                }
                let n = c.rotated(normal);
                let sign = if n.x + n.z > 0. { 1. } else { -1. };
                let expected = if sign > 0. { positive } else { 3 - positive };
                let mut scene = Scene {
                    vertices: vec![],
                    draws: vec![],
                    censors: vec![],
                    textures: Arc::clone(&textures),
                    width: 400,
                    height: 300,
                };
                scene
                    .original_walls(
                        Some(&materials),
                        &BTreeMap::new(),
                        &BTreeMap::new(),
                        None,
                        &sim,
                        &Program::default(),
                        &c,
                        1,
                        false,
                    )
                    .unwrap();
                let image = crate::software::render(&scene);
                let s = c.screen(mid + normal * sign * 0.046 + Vec3::Y * 0.1);
                assert_eq!(
                    image.get_pixel(s.x as u32, s.y as u32).0,
                    colors[expected],
                    "wall {bit}, view {rotation}"
                );
                // Translating wall and camera onto native terrain must preserve
                // projected geometry and the visible face. Float translation
                // can move raster edges by a few ulps; this is not a saved-pixel
                // equality check (which uses the exact same world coordinates).
                let old_vertices = scene.vertices.clone();
                c.center.y += crate::original_house::terrain_height(10.);
                scene.vertices.clear();
                scene.draws.clear();
                scene
                    .original_walls(
                        Some(&materials),
                        &BTreeMap::new(),
                        &BTreeMap::new(),
                        None,
                        &sim,
                        &Program::default(),
                        &c,
                        1,
                        true,
                    )
                    .unwrap();
                let actual = crate::software::render(&scene);
                let delta = scene
                    .vertices
                    .iter()
                    .zip(&old_vertices)
                    .flat_map(|(a, b)| {
                        a.position
                            .iter()
                            .zip(b.position)
                            .map(|(a, b)| (a - b).abs())
                    })
                    .fold(0.0f32, f32::max);
                assert!(
                    delta <= 8. * f32::EPSILON,
                    "terrain wall {bit}, view {rotation}: vertex delta {delta}"
                );
                assert_eq!(
                    actual.get_pixel(s.x as u32, s.y as u32).0,
                    colors[expected],
                    "terrain wall {bit}, view {rotation}"
                );
            }
        }
    }
}
