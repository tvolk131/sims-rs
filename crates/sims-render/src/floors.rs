//! Original floor images on the imported tile planes. Lighting and terrain
//! shading remain the preview renderer's responsibility, not a native claim.
use crate::{Assets, Camera, Scene, Texture, floor_elevation};
use anyhow::{Context, Result};
use glam::{Vec2, Vec3};
use std::{collections::BTreeMap, sync::Arc};
pub(crate) mod pool;
mod road;

/// The saved arrays have a 64-cell stride even on a smaller active lot.
/// World resize reads global23 (zero means64, values<=3 leave size alone).
/// This is a presentation bound; the simulation's arrays remain untouched.
pub fn active_size(sim: &sims_sim::Simulation) -> Result<usize> {
    let lot = sim
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("active ground bounds require an original lot")?;
    let size = match sim.vm.world.globals.get(&23).copied() {
        Some(0) => 64,
        Some(n) if n > 3 => n as usize,
        _ => lot.size,
    };
    anyhow::ensure!(
        size > 0 && size <= lot.size,
        "active lot exceeds backing arrays"
    );
    Ok(size)
}

///49cecc..49cefc excludes the outer border of the active world, separately
///from the backing stride. Share the same cell range with pointer hit testing.
pub(crate) fn drawn_cells(size: usize) -> Result<std::ops::Range<usize>> {
    anyhow::ensure!(size >= 3, "original ground requires an interior");
    Ok(1..size - 1)
}

fn pieces(
    floor: &sims_sim::lot_geometry::FloorSource,
    cell: usize,
) -> Result<Vec<(u16, [usize; 3])>> {
    let id = floor.coverings[cell];
    Ok(if id == 255 {
        let wall = &floor.walls[cell];
        match wall.segments & 0x30 {
            16 => vec![(wall.styles[0], [0, 1, 3]), (wall.patterns[0], [1, 2, 3])],
            32 => vec![(wall.styles[0], [0, 2, 3]), (wall.patterns[0], [0, 1, 2])],
            _ => anyhow::bail!("split floor without diagonal"),
        }
    } else {
        vec![(id, [0, 1, 2]), (id, [0, 2, 3])]
    })
}
const CORNERS: [Vec3; 4] = [Vec3::ZERO, Vec3::X, Vec3::new(1., 0., 1.), Vec3::Z];

/// Floor SPR2 alpha clips a screen-space diamond. Here the tile triangles
/// already supply that boundary, including diagonal halves and terrain slope.
/// Applying both outlines can discard covered pixels when the camera or Iced
/// viewport has a fractional pixel phase. Extend authored edge texels through
/// the exterior row padding, as a texture border; keep interior pixels/holes
/// and fully transparent rows unchanged. No filtering or source asset edits.
fn extend_floor_padding(width: u32, rgba: &mut [u8]) {
    for row in rgba.chunks_exact_mut(width as usize * 4) {
        let Some(first) = row.chunks_exact(4).position(|p| p[3] != 0) else {
            continue;
        };
        let last = row.chunks_exact(4).rposition(|p| p[3] != 0).unwrap();
        let left: [u8; 4] = row[first * 4..first * 4 + 4].try_into().unwrap();
        let right: [u8; 4] = row[last * 4..last * 4 + 4].try_into().unwrap();
        for pixel in row[..first * 4].chunks_exact_mut(4) {
            pixel.copy_from_slice(&left);
        }
        for pixel in row[(last + 1) * 4..].chunks_exact_mut(4) {
            pixel.copy_from_slice(&right);
        }
    }
}
fn points(lot: &sims_sim::lot_geometry::Lot, level: i32, cell: usize, terrain: bool) -> [Vec3; 4] {
    let base = Vec3::new(
        (cell / lot.size) as f32,
        floor_elevation(level),
        (cell % lot.size) as f32,
    );
    std::array::from_fn(|i| {
        let mut p = base + CORNERS[i];
        if terrain {
            p.y += crate::original_house::terrain_height(f64::from(
                lot.heights[cell][[1, 2, 3, 0][i]],
            ));
        }
        p
    })
}
impl Camera {
    /// Replacement viewport hit testing against the same original floor planes
    /// we draw. This is not a claim about the native protected pixel lookups.
    /// Only the selected floor participates: an upstairs hole is not downstairs.
    pub fn pick_original_ground(
        &self,
        lot: &sims_sim::lot_geometry::Lot,
        pixel: [f32; 2],
    ) -> Result<Option<[i32; 2]>> {
        self.pick_original_ground_in(lot, lot.size, pixel)
    }
    /// Same planes, restricted to the active world size rather than its stride.
    pub fn pick_original_ground_in(
        &self,
        lot: &sims_sim::lot_geometry::Lot,
        size: usize,
        pixel: [f32; 2],
    ) -> Result<Option<[i32; 2]>> {
        anyhow::ensure!(
            size > 0 && size <= lot.size,
            "ground bounds exceed backing arrays"
        );
        let q = Vec2::from_array(pixel);
        if !q.is_finite()
            || q.x < 0.
            || q.y < 0.
            || q.x >= self.width as f32
            || q.y >= self.height as f32
        {
            return Ok(None);
        }
        let floor = lot
            .floor(self.level)
            .context("ground hit-test floor absent")?;
        let mut nearest = f32::INFINITY;
        let mut found = None;
        let range = drawn_cells(size)?;
        for cell in range
            .clone()
            .flat_map(|x| range.clone().map(move |z| x * lot.size + z))
        {
            let corners = points(lot, self.level, cell, true);
            for (material, triangle) in pieces(&floor.source, cell)? {
                if material == 0 && self.level > 1 {
                    continue;
                }
                let world = triangle.map(|i| corners[i]);
                let p = world.map(|v| self.screen(v));
                let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let weights = [
                    (p[1] - q).perp_dot(p[2] - q) / area,
                    (p[2] - q).perp_dot(p[0] - q) / area,
                    (p[0] - q).perp_dot(p[1] - q) / area,
                ];
                if weights.iter().any(|w| *w < 0.) {
                    continue;
                }
                let point = (0..3).fold(Vec3::ZERO, |v, i| v + world[i] * weights[i]);
                let depth = self.depth(point);
                if depth < nearest {
                    nearest = depth;
                    // VM physical words are row/column; renderer uses column/row.
                    found = Some([
                        (point.z * 16.).round() as i32,
                        (point.x * 16.).round() as i32,
                    ]);
                }
            }
        }
        Ok(found)
    }
}

impl Assets {
    pub(crate) fn bind_original_floors(
        &mut self,
        index: &sims_content::Index,
        house: &sims_content::saves::House,
    ) -> Result<()> {
        let materials = sims_content::surfaces::load_floors(index, house)?;
        let textures = Arc::make_mut(&mut self.textures);
        let mut bound = BTreeMap::new();
        for (id, m) in materials {
            let first = textures.len();
            bound.insert(id, [first, first + 1, first + 2]);
            for mut f in m.frames {
                extend_floor_padding(f.width, &mut f.rgba);
                textures.push(Texture {
                    width: f.width,
                    height: f.height,
                    rgba: f.rgba.into(),
                    depth: f.depth.into(),
                });
            }
        }
        road::bind_views(index, &mut bound, textures)?;
        self.pool_materials = pool::bind(index, house, textures)?;
        // Native4a7dfe persists the10/11 artwork phase in the fixed floor words.
        // Keep that saved phase in presentation; camera turns must not edit the
        // retained simulation geometry just to select the matching texture.
        self.saved_road_rotation = (house.view.camera_direction as u16 & 3) as u8;
        let (ground, texture) = crate::grass::Ground::bind(house, textures.len())?;
        textures.push(texture);
        self.ground = Some(ground);
        self.original_background = Some(textures.len());
        textures.push(Texture {
            width: 1,
            height: 1,
            rgba: vec![112, 112, 112, 255].into(),
            depth: vec![255].into(),
        });
        self.floor_materials = Some(bound);
        Ok(())
    }
}
impl Scene {
    pub(crate) fn original_floor(
        &mut self,
        materials: Option<&BTreeMap<u16, [usize; 3]>>,
        ground: Option<&crate::grass::Ground>,
        pool: Option<&pool::Materials>,
        saved_road_rotation: u8,
        sim: &sims_sim::Simulation,
        c: &Camera,
        level: i32,
        x: i32,
        z: i32,
        native_terrain: bool,
    ) -> Result<bool> {
        let Some(materials) = materials else {
            return Ok(false);
        };
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("floor materials require an original lot")?;
        let floor = &lot.floors[(level - 1) as usize].source;
        // Lot arrays and wall geometry address the column before the row.
        // The VM's physical X/Y convention is separately adapted by Bodies.
        let cell = x as usize * lot.size + z as usize;
        let world = points(lot, level, cell, native_terrain);
        if level == 1 {
            if let Some(pool) = pool {
                if let Some(texture) = pool.surface(lot.pool[cell], c) {
                    for triangle in [[0, 1, 2], [0, 2, 3]] {
                        self.original_floor_overlay(c, &world, triangle, texture);
                    }
                    let corners = pool::corner_mask(lot.size, &lot.pool, x, z, c.rotation)?;
                    for corner in 0..4 {
                        if corners & (1 << corner) != 0 {
                            for triangle in [[0, 1, 2], [0, 2, 3]] {
                                self.original_floor_overlay(
                                    c,
                                    &world,
                                    triangle,
                                    pool.corner(corner, c),
                                );
                            }
                        }
                    }
                    return Ok(true);
                }
            }
        }
        for (material, triangle) in pieces(floor, cell)? {
            if material == 0 && level > 1 {
                continue;
            }
            let (texture, tint) = if material == 0 {
                (
                    ground.context("original grass texture absent")?.texture,
                    [1.; 4],
                )
            } else {
                (
                    materials
                        .get(&if level == 1 {
                            road::material(material, saved_road_rotation, c.rotation)
                        } else {
                            material
                        })
                        .with_context(|| format!("floor material {material} not loaded"))?
                        [(c.sprite_zoom().0 - 1) as usize],
                    [1.; 4],
                )
            };
            let start = self.vertices.len() as u32;
            for i in triangle {
                let local = CORNERS[i];
                // The resource is already an isometric diamond. Map screen
                // diamond coordinates directly, retaining affine plane depth.
                let r = c.rotated(local - Vec3::new(0.5, 0., 0.5));
                let uv = if material == 0 {
                    ground.unwrap().uv(x, z, local)
                } else {
                    [(r.x - r.z) * 0.5 + 0.5, (r.x + r.z) * 0.5 + 0.5]
                };
                self.vertices.push(c.vertex(world[i], uv, tint));
            }
            self.draws.push(crate::Draw {
                vertices: start..self.vertices.len() as u32,
                texture,
                object: None,
            });
        }
        Ok(true)
    }

    /// Pool interiors and sparse corner caps use the same tile plane/depth and
    /// screen-diamond UVs as floors. Original pool SPR2s have no depth channel;
    /// no invented volume, depth bias, tint, simulation movement or clock.
    fn original_floor_overlay(
        &mut self,
        c: &Camera,
        world: &[Vec3; 4],
        triangle: [usize; 3],
        texture: usize,
    ) {
        let start = self.vertices.len() as u32;
        for i in triangle {
            let r = c.rotated(CORNERS[i] - Vec3::new(0.5, 0., 0.5));
            let uv = [(r.x - r.z) * 0.5 + 0.5, (r.x + r.z) * 0.5 + 0.5];
            self.vertices.push(c.vertex(world[i], uv, [1.; 4]));
        }
        self.draws.push(crate::Draw {
            vertices: start..self.vertices.len() as u32,
            texture,
            object: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_sim::{
        Room, Simulation,
        lot_geometry::{FloorSource, Lot},
    };
    use sims_vm::{Program, Vm};
    #[test]
    fn floor_padding_extends_edges_without_changing_authored_pixels_or_holes() {
        let clear = [0, 0, 0, 0];
        let left = [15, 18, 22, 255];
        let right = [41, 36, 29, 127];
        let mut pixels = [
            clear, left, clear, right, clear, clear, clear, clear, clear, clear,
        ]
        .concat();
        extend_floor_padding(5, &mut pixels);
        assert_eq!(
            pixels,
            [
                left, left, clear, right, right, clear, clear, clear, clear, clear
            ]
            .concat()
        );
    }
    #[test]
    fn diagonal_upper_floor_holes_and_picking_stay_on_the_authored_side() {
        let blank = FloorSource {
            coverings: vec![0; 16],
            walls: vec![Default::default(); 16],
            object_heads: vec![0; 16],
            flags: vec![0; 16],
        };
        for (bit, filled, hole) in [(16, [1.2, 1.2], [1.8, 1.8]), (32, [1.2, 1.8], [1.8, 1.2])] {
            let mut upper = blank.clone();
            upper.coverings[5] = 255;
            upper.walls[5].segments = bit;
            upper.walls[5].styles = [1, 1];
            let lot = Lot::rebuild(
                4,
                vec![[10; 4]; 16],
                vec![255; 16],
                vec![255; 16],
                vec![blank.clone(), upper],
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
            let before = serde_json::to_vec(&sim).unwrap();
            let lot = sim
                .architecture
                .as_ref()
                .unwrap()
                .original
                .as_ref()
                .unwrap();
            let materials = BTreeMap::from([(1, [0; 3])]);
            for rotation in 0..4 {
                let mut camera = Camera::room(&sim, 400, 300, rotation, 1.).floor(2);
                camera.center.y += crate::original_house::terrain_height(10.);
                let mut scene = Scene {
                    vertices: vec![],
                    draws: vec![],
                    censors: vec![],
                    textures: Arc::new(vec![Texture {
                        width: 1,
                        height: 1,
                        rgba: vec![255, 0, 0, 255].into(),
                        depth: vec![255].into(),
                    }]),
                    width: 400,
                    height: 300,
                };
                scene
                    .original_floor(
                        Some(&materials),
                        None,
                        None,
                        0,
                        &sim,
                        &camera,
                        2,
                        1,
                        1,
                        true,
                    )
                    .unwrap();
                let frame = crate::software::render(&scene);
                for (point, filled) in [(filled, true), (hole, false)] {
                    let world = Vec3::new(point[0], camera.center.y, point[1]);
                    let screen = camera.screen(world);
                    assert_eq!(
                        frame.get_pixel(screen.x as u32, screen.y as u32).0,
                        if filled {
                            [255, 0, 0, 255]
                        } else {
                            [24, 32, 37, 255]
                        },
                        "bit{bit}, rotation{rotation}, filled={filled}"
                    );
                    assert_eq!(
                        camera
                            .pick_original_ground(lot, screen.to_array())
                            .unwrap()
                            .is_some(),
                        filled,
                        "pick bit{bit}, rotation{rotation}, filled={filled}"
                    );
                }
            }
            assert_eq!(before, serde_json::to_vec(&sim).unwrap());
        }
    }
    #[test]
    fn active_world_bound_preserves_backing_stride_and_excludes_phantom_ground_hits() {
        let floor = FloorSource {
            coverings: vec![0; 64],
            walls: vec![Default::default(); 64],
            object_heads: vec![0; 64],
            flags: vec![0; 64],
        };
        let lot = Lot::rebuild(
            8,
            vec![[0; 4]; 64],
            vec![255; 64],
            vec![255; 64],
            vec![floor],
        )
        .unwrap();
        let mut sim = Simulation::new(
            Vm::new(Default::default(), 1),
            Room {
                width: 128,
                height: 128,
                obstacles: vec![],
            },
            Default::default(),
        )
        .unwrap();
        sim.set_original_lot(&Program::default(), lot).unwrap();
        sim.vm.world.globals.insert(23, 4);
        assert_eq!(active_size(&sim).unwrap(), 4);
        let lot = sim
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        assert_eq!(lot.size, 8);
        for rotation in 0..4 {
            let c = Camera {
                level: 1,
                width: 1000,
                height: 740,
                rotation,
                zoom: 1.,
                center: Vec3::new(4., 0., 4.),
            };
            let good = Vec3::new(2.5, 0., 2.5);
            let unused = Vec3::new(6.5, 0., 2.5);
            assert_eq!(
                c.pick_original_ground_in(lot, 4, c.screen(good).to_array())
                    .unwrap(),
                Some([40, 40])
            );
            assert!(
                c.pick_original_ground(lot, c.screen(unused).to_array())
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                c.pick_original_ground_in(lot, 4, c.screen(unused).to_array())
                    .unwrap(),
                None
            );
            for border in [
                Vec3::new(0.5, 0., 2.5),
                Vec3::new(3.5, 0., 2.5),
                Vec3::new(2.5, 0., 0.5),
                Vec3::new(2.5, 0., 3.5),
            ] {
                assert_eq!(
                    c.pick_original_ground_in(lot, 4, c.screen(border).to_array())
                        .unwrap(),
                    None,
                    "outer world border must not be drawn or pickable, rotation{rotation}: {border}"
                );
            }
        }
        assert_eq!(lot.size, 8);
        sim.vm.world.globals.insert(23, 3);
        assert_eq!(active_size(&sim).unwrap(), 8);
        sim.vm.world.globals.insert(23, 9);
        assert!(active_size(&sim).is_err());
    }
    #[test]
    fn ground_pick_matches_rendered_slopes_rotations_zoom_and_upper_holes() {
        let floor = FloorSource {
            coverings: vec![1; 16],
            walls: vec![Default::default(); 16],
            object_heads: vec![0; 16],
            flags: vec![0; 16],
        };
        let mut upper = floor.clone();
        upper.coverings.fill(0);
        upper.coverings[5] = 255;
        upper.walls[5].segments = 32;
        upper.walls[5].styles = [1, 1];
        upper.walls[5].patterns[0] = 0;
        let heights = (0..16)
            .map(|i| {
                let x = (i / 4) as u8;
                let z = (i % 4) as u8;
                [x + z + 1, x + z, x + z + 1, x + z + 2]
            })
            .collect();
        let lot =
            Lot::rebuild(4, heights, vec![255; 16], vec![255; 16], vec![floor, upper]).unwrap();
        for level in [1, 2] {
            for rotation in 0..4 {
                for zoom in [0.25, 0.5, 1.] {
                    let c = Camera {
                        level,
                        width: 1000,
                        height: 740,
                        rotation,
                        zoom,
                        center: Vec3::new(2., floor_elevation(level), 2.),
                    };
                    let corners = points(&lot, level, 5, true);
                    for (material, face) in pieces(&lot.floor(level).unwrap().source, 5).unwrap() {
                        let point = face.into_iter().fold(Vec3::ZERO, |v, i| v + corners[i]) / 3.;
                        let hit = c
                            .pick_original_ground(&lot, c.screen(point).to_array())
                            .unwrap();
                        assert_eq!(
                            hit,
                            (material != 0 || level == 1).then_some([
                                (point.z * 16.).round() as i32,
                                (point.x * 16.).round() as i32
                            ]),
                            "floor{level} rotation{rotation} zoom{zoom}"
                        );
                    }
                    for bad in [[-1., 0.], [1000., 0.], [0., 740.], [f32::NAN, 10.]] {
                        assert_eq!(c.pick_original_ground(&lot, bad).unwrap(), None);
                    }
                }
            }
        }
    }
    #[test]
    fn original_floor_pixels_follow_columns_and_both_diagonal_halves_in_all_rotations() {
        let blank = FloorSource {
            coverings: vec![0; 16],
            walls: vec![Default::default(); 16],
            object_heads: vec![0; 16],
            flags: vec![0; 16],
        };
        let mut upper = blank.clone();
        upper.coverings[1 * 4 + 2] = 1;
        upper.coverings[2 * 4 + 1] = 2;
        upper.coverings[1 * 4 + 1] = 255;
        upper.walls[1 * 4 + 1].segments = 32;
        upper.walls[1 * 4 + 1].styles = [1, 1];
        upper.walls[1 * 4 + 1].patterns[0] = 2;
        upper.coverings[2 * 4 + 2] = 255;
        upper.walls[2 * 4 + 2].segments = 16;
        upper.walls[2 * 4 + 2].styles = [1, 1];
        upper.walls[2 * 4 + 2].patterns[0] = 2;
        let lot = Lot::rebuild(
            4,
            vec![[10; 4]; 16],
            vec![255; 16],
            vec![255; 16],
            vec![blank, upper],
        )
        .unwrap();
        let p = Program::default();
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
        sim.set_original_lot(&p, lot).unwrap();
        let materials = BTreeMap::from([(1, [1; 3]), (2, [2; 3])]);
        let colors = [[255, 255, 255, 255], [255, 0, 0, 255], [0, 0, 255, 255]];
        let textures = Arc::new(
            colors
                .iter()
                .map(|color| Texture {
                    width: 1,
                    height: 1,
                    rgba: color.to_vec().into(),
                    depth: vec![255].into(),
                })
                .collect(),
        );
        for (rotation, native_terrain) in (0..4).flat_map(|r| [(r, false), (r, true)]) {
            let ground = if native_terrain {
                crate::original_house::terrain_height(10.)
            } else {
                0.
            };
            let mut c = Camera::room(&sim, 400, 300, rotation, 0.7).floor(2);
            c.center.y += ground;
            let mut scene = Scene {
                vertices: vec![],
                draws: vec![],
                censors: vec![],
                textures: Arc::clone(&textures),
                width: 400,
                height: 300,
            };
            for x in 0..4 {
                for z in 0..4 {
                    scene
                        .original_floor(
                            Some(&materials),
                            None,
                            None,
                            0,
                            &sim,
                            &c,
                            2,
                            x,
                            z,
                            native_terrain,
                        )
                        .unwrap();
                }
            }
            let frame = crate::software::render(&scene);
            for (x, z, expected) in [
                (1.5, 2.5, 1),
                (2.5, 1.5, 2),
                (1.25, 1.75, 1),
                (1.75, 1.25, 2),
                (2.75, 2.75, 2),
                (2.25, 2.25, 1),
            ] {
                let screen = c.screen(Vec3::new(x, floor_elevation(2) + ground, z));
                assert_eq!(
                    frame.get_pixel(screen.x as u32, screen.y as u32).0,
                    colors[expected],
                    "rotation {rotation}, tile point ({x},{z})"
                );
            }
        }
    }
}
