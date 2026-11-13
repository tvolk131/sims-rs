//! Presentation of the bounded chair room; no gameplay advances from a rendered frame.
mod camera;
pub mod camera_device;
pub mod censor;
mod depth_range;
mod decoded_sprites;
pub mod dialog;
mod floors;
pub use floors::active_size as original_lot_size;
pub mod frozen_house;
pub mod gpu;
mod grass;
pub mod object_preview;
pub mod object_speech;
pub mod original_house;
pub mod portrait;
pub mod pose;
pub mod software;
pub mod sprite_depth;
#[cfg(test)]
mod texture_tests;
mod wall_objects;
pub mod wall_view;
mod walls;
use anyhow::{Context, Result, bail};
use glam::{Quat, Vec2, Vec3};
use sims_content::{
    Index,
    avatar::{Avatar, load_adult},
};
use sims_formats::sprite::SpriteSet;
use sims_sim::Simulation;
use sims_vm::Program;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// Published scenes share immutable samples when their texture-vector owner
    /// changes. Writers replace these payloads or use Arc::make_mut, preserving
    /// older scenes and the GPU cache's content-based comparisons.
    pub rgba: Arc<[u8]>,
    pub depth: Arc<[u8]>,
}
impl Texture {
    fn from_avatar(texture: &sims_content::avatar::Texture) -> Self {
        Self {
            width: texture.width,
            height: texture.height,
            rgba: texture.rgba.clone().into(),
            depth: vec![255; (texture.width * texture.height) as usize].into(),
        }
    }

    fn with_rgba(&self, rgba: Vec<u8>) -> Self {
        Self {
            width: self.width,
            height: self.height,
            rgba: rgba.into(),
            depth: Arc::clone(&self.depth),
        }
    }
}
#[derive(Debug, Clone)]
struct Sprite {
    id: u16,
    texture: usize,
    offset: Vec2,
    object: Vec3,
    flip: bool,
}
#[derive(Debug)]
struct ResidentAvatar {
    avatar: Avatar,
    texture_base: usize,
    outfit: i16,
    idle: String,
}
#[derive(Debug)]
pub struct Assets {
    floor_materials: Option<BTreeMap<u16, [usize; 3]>>,
    pool_materials: Option<floors::pool::Materials>,
    saved_road_rotation: u8,
    ground: Option<grass::Ground>,
    original_background: Option<usize>,
    wall_materials: Option<BTreeMap<u16, [[usize; 4]; 3]>>,
    wall_shapes: BTreeMap<(u16, u8, bool), [[usize; 4]; 3]>,
    object_wall_shapes: BTreeMap<(u16, String, u16, bool), [[usize; 4]; 3]>,
    residents: BTreeMap<i16, ResidentAvatar>,
    pub avatar: Avatar,
    alternate: Avatar,
    alternate_texture: usize,
    nude: Avatar,
    nude_texture: usize,
    sleepwear: Avatar,
    sleepwear_texture: usize,
    pub textures: Arc<Vec<Texture>>,
    // Source, DGRP, direction, original zoom (1 distant / 2 medium / 3 near).
    sprites: BTreeMap<(String, u16, u32, u32), Vec<Sprite>>,
    object_graphics: BTreeMap<(String, u16), (u16, u16)>,
    accessories: BTreeMap<String, Vec<(sims_content::avatar::Part, usize)>>,
}
impl Assets {
    /// Bind explicit resident assets, keeping the skeleton and outfit together.
    /// The caller chooses a preview idle; this does not restore animator clocks.
    fn bind_resident(&mut self, id: i16, avatar: Avatar, outfit: i16, idle: String) -> Result<()> {
        anyhow::ensure!(
            !self.residents.contains_key(&id),
            "resident avatar already bound"
        );
        let textures = Arc::make_mut(&mut self.textures);
        let texture_base = textures.len();
        for p in &avatar.parts {
            textures.push(Texture::from_avatar(&p.texture));
        }
        self.residents.insert(
            id,
            ResidentAvatar {
                avatar,
                texture_base,
                outfit,
                idle,
            },
        );
        Ok(())
    }
    fn skeleton(&self, id: i16) -> &sims_formats::avatar::Skeleton {
        &self
            .residents
            .get(&id)
            .map_or(&self.avatar, |p| &p.avatar)
            .skeleton
    }
    pub fn load(index: &Index) -> Result<Self> {
        Self::load_objects(index, &[("DiningChairs.iff", 16807)])
    }
    /// Definitions actually placed, plus literal Create Object dependencies in
    /// linked bytecode. Resolve groups to their renderable parts, not catalogs.
    pub fn load_simulation(index: &Index, program: &Program, sim: &Simulation) -> Result<Self> {
        let mut objects: std::collections::BTreeSet<(String, u16)> = sim
            .vm
            .world
            .objects
            .values()
            .filter(|o| o.person.is_none())
            .filter_map(|o| o.definition.as_ref().map(|d| (d.source.clone(), d.object)))
            .collect();
        let guids: std::collections::BTreeSet<_> = program
            .modules
            .values()
            .flat_map(|m| m.routines.values())
            .flat_map(|r| &r.body.instructions)
            .filter(|i| i.opcode == 0x2a)
            .map(|i| u32::from_le_bytes(i.operands[..4].try_into().unwrap()))
            .collect();
        for module in program.modules.values() {
            for (&id, w) in &module.object_definition_words {
                if w.len() <= 15
                    || !guids.contains(&(w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16)))
                {
                    continue;
                }
                if w[10] != 0 && w[11] == -1 {
                    for part in sims_vm::multitile::construction_plan(module, id)
                        .map_err(anyhow::Error::msg)?
                    {
                        objects.insert((module.source.clone(), part));
                    }
                } else {
                    objects.insert((module.source.clone(), id));
                }
            }
        }
        Self::load_objects(
            index,
            &objects
                .iter()
                .map(|(s, id)| (s.as_str(), *id))
                .collect::<Vec<_>>(),
        )
    }
    pub fn load_household(index: &Index) -> Result<Self> {
        let (p, s) = sims_content::household::autonomous_household(index)?;
        Self::load_simulation(index, &p, &s)
    }
    /// Explicit resource definitions for headless household frames and clients.
    pub fn load_objects(index: &Index, objects: &[(&str, u16)]) -> Result<Self> {
        Self::load_objects_profile(index, objects, None)
    }
    fn load_objects_profile(
        index: &Index,
        objects: &[(&str, u16)],
        requested: Option<&std::collections::BTreeSet<String>>,
    ) -> Result<Self> {
        let avatar = load_adult(index)?;
        let mut textures = vec![Texture {
            width: 1,
            height: 1,
            rgba: vec![255; 4].into(),
            depth: vec![255].into(),
        }];
        for p in &avatar.parts {
            textures.push(Texture::from_avatar(&p.texture));
        }
        let nude = sims_content::avatar::load_adult_outfit(index, true)?;
        let nude_texture = textures.len();
        for p in &nude.parts {
            textures.push(Texture::from_avatar(&p.texture));
        }
        let alternate = sims_content::avatar::load_adult_profile(index, false, true)?;
        let alternate_texture = textures.len();
        for p in &alternate.parts {
            textures.push(Texture::from_avatar(&p.texture));
        }
        let sleepwear = sims_content::avatar::load_adult_sleepwear(index)?;
        let sleepwear_texture = textures.len();
        for p in &sleepwear.parts {
            textures.push(Texture::from_avatar(&p.texture));
        }
        let mut accessories = BTreeMap::new();
        let layers = if let Some(requested) = requested {
            let (layers, report) = sims_content::avatar::load_named_accessories(index, requested)?;
            anyhow::ensure!(
                report.unresolved_labels.is_empty(),
                "missing requested accessories: {:?}",
                report.unresolved_labels
            );
            layers
        } else {
            let mut program = Program::default();
            for (selector, _) in objects {
                program
                    .modules
                    .extend(sims_content::vm::load_program(index, selector, "Global.iff")?.modules);
            }
            sims_content::avatar::load_linked_accessories(index, &program)?
        };
        for (name, asset) in layers {
            let mut parts = vec![];
            for part in asset.parts {
                for bone in &part.mesh.bones {
                    if !avatar
                        .skeleton
                        .bones
                        .iter()
                        .any(|b| b.name.eq_ignore_ascii_case(bone))
                    {
                        bail!("accessory references absent skeleton bone {bone}");
                    }
                }
                let texture = textures.len();
                textures.push(Texture::from_avatar(&part.texture));
                parts.push((part, texture));
            }
            accessories.insert(name, parts);
        }
        let mut sprites = BTreeMap::new();
        let mut object_graphics = BTreeMap::new();
        for &(selector, definition) in objects {
            let doc = index.document(index.find(selector)?)?;
            let mut decoded = decoded_sprites::Frames::default();
            let obj = doc
                .objects
                .iter()
                .find(|o| o.id == definition)
                .context("object definition missing")?;
            object_graphics.insert(
                (doc.source.clone(), definition),
                (obj.value.graphic, obj.value.graphics),
            );
            let end = obj
                .value
                .graphic
                .checked_add(obj.value.graphics)
                .context("object graphics overflow")?;
            for group_id in obj.value.graphic..end {
                let Some(group) = doc.groups.iter().find(|g| g.id == group_id) else {
                    // OBJD declares a range; unused entries may have no DGRP.
                    // A runtime selection of an absent entry still errors.
                    continue;
                };
                let group = &group.value;
                for view in &group.views {
                    anyhow::ensure!(
                        (1..=3).contains(&view.zoom),
                        "unsupported object sprite zoom {} in {} DGRP {group_id}",
                        view.zoom,
                        doc.source
                    );
                    let mut layers = Vec::new();
                    for s in &view.sprites {
                        // Bit 4 is the luminous layer. This authored room uses
                        // unlit sprite colors, so it already has full brightness.
                        if s.flags & !5 != 0 {
                            bail!(
                                "unsupported object sprite flags {:#x} in {} DGRP {group_id}, SPR2 {}",
                                s.flags,
                                doc.source,
                                s.id
                            );
                        }
                        let identifier = u16::try_from(s.id)?;
                        let texture = decoded.get((identifier, s.frame as usize), || {
                            let chunk = doc.iff.get(b"SPR2", identifier).with_context(|| {
                                format!(
                                    "missing SPR2 {} for {} DGRP {group_id}, frame {}",
                                    s.id, doc.source, s.frame
                                )
                            })?;
                            let raw = doc.iff.data(chunk);
                            Ok(SpriteSet::parse(raw)?.decode(raw, s.frame as usize, &doc.iff)?)
                        })?;
                        // Original household objects include empty SPR2 frames.
                        // They contribute no pixels or pick geometry, and wgpu
                        // cannot allocate a zero-sized texture for them.
                        let Some(texture) = texture else {
                            continue;
                        };
                        // DGRP screen offsets use the bottom of the sprite, independently of SPR2's preview origin.
                        layers.push(Sprite {
                            id: identifier,
                            texture: textures.len(),
                            offset: Vec2::new(s.x as f32, s.y as f32 - texture.height as f32),
                            object: Vec3::new(
                                s.object[0] / 16.,
                                s.object[2] / 5.,
                                s.object[1] / 16.,
                            ),
                            flip: s.flags & 1 != 0,
                        });
                        textures.push(texture);
                    }
                    sprites.insert(
                        (doc.source.clone(), group_id, view.direction, view.zoom),
                        layers,
                    );
                }
            }
        }
        Ok(Self {
            floor_materials: None,
            pool_materials: None,
            saved_road_rotation: 0,
            ground: None,
            original_background: None,
            wall_materials: None,
            wall_shapes: BTreeMap::new(),
            object_wall_shapes: BTreeMap::new(),
            residents: BTreeMap::new(),
            alternate,
            alternate_texture,
            avatar,
            nude,
            nude_texture,
            sleepwear,
            sleepwear_texture,
            textures: Arc::new(textures),
            sprites,
            object_graphics,
            accessories,
        })
    }
}
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub tint: [f32; 4],
    pub depth_scale: f32,
}
#[derive(Debug, Clone)]
pub struct Draw {
    pub vertices: Range<u32>,
    pub texture: usize,
    pub object: Option<sims_vm::ObjectId>,
}
#[derive(Debug, Clone)]
pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub draws: Vec<Draw>,
    pub censors: Vec<censor::Region>,
    pub textures: Arc<Vec<Texture>>,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Highest visible storey; lower floors remain visible through openings.
    pub level: i32,
    pub width: u32,
    pub height: u32,
    pub rotation: u8,
    pub zoom: f32,
    pub center: Vec3,
}
impl Camera {
    /// Use authored pixels at each original zoom. Continuous zoom is a client
    /// extension: choose the closest original size, then scale its pixels and
    /// DGRP screen offsets together. World offsets are already in tile units.
    fn sprite_zoom(&self) -> (u32, f32) {
        let level = if self.zoom < 0.375 {
            1
        } else if self.zoom < 0.75 {
            2
        } else {
            3
        };
        (level, self.zoom / ((1u32 << level) as f32 / 8.))
    }
    pub fn room(sim: &Simulation, width: u32, height: u32, rotation: u8, zoom: f32) -> Self {
        Self {
            level: 1,
            width,
            height,
            rotation: rotation % 4,
            zoom,
            center: Vec3::new(
                sim.room.width as f32 / 32.,
                0.,
                sim.room.height as f32 / 32.,
            ),
        }
    }
    pub fn floor(mut self, level: i32) -> Self {
        self.level = level;
        self.center.y = floor_elevation(level);
        self
    }
    pub fn rotated(&self, p: Vec3) -> Vec3 {
        match self.rotation {
            0 => p,
            1 => Vec3::new(-p.z, p.y, p.x),
            2 => Vec3::new(-p.x, p.y, -p.z),
            _ => Vec3::new(p.z, p.y, -p.x),
        }
    }
    pub fn screen(&self, p: Vec3) -> Vec2 {
        let p = self.rotated(p - self.center);
        Vec2::new((p.x - p.z) * 64., (p.x + p.z) * 32. - p.y * 78.38367) * self.zoom
            + Vec2::new(self.width as f32 * 0.5, self.height as f32 * 0.57)
    }
    pub fn depth(&self, p: Vec3) -> f32 {
        let p = self.rotated(p - self.center);
        0.5 - ((p.x + p.z) * 0.61237246 + p.y * 0.5) * 0.02
    }
    fn vertex(&self, p: Vec3, uv: [f32; 2], tint: [f32; 4]) -> Vertex {
        self.screen_vertex(self.screen(p), self.depth(p), uv, tint, 0.)
    }
    fn screen_vertex(
        &self,
        p: Vec2,
        z: f32,
        uv: [f32; 2],
        tint: [f32; 4],
        depth_scale: f32,
    ) -> Vertex {
        Vertex {
            position: [
                p.x / self.width as f32 * 2. - 1.,
                1. - p.y / self.height as f32 * 2.,
                z,
            ],
            uv,
            tint,
            depth_scale,
        }
    }
}
impl Scene {
    /// Pick the nearest opaque fragment, including sprite alpha/depth and geometry occlusion.
    /// Coordinates are in scene pixels; the client handles viewport letterboxing and DPI.
    pub fn pick(&self, point: [f32; 2]) -> Option<sims_vm::ObjectId> {
        let q = Vec2::from_array(point);
        if !q.is_finite()
            || q.x < 0.
            || q.y < 0.
            || q.x >= self.width as f32
            || q.y >= self.height as f32
        {
            return None;
        }
        let mut nearest = 1.;
        let mut object = None;
        for draw in &self.draws {
            let texture = &self.textures[draw.texture];
            for tri in self.vertices[draw.vertices.start as usize..draw.vertices.end as usize]
                .as_chunks::<3>()
                .0
            {
                let p = tri.map(|v| {
                    Vec2::new(
                        (v.position[0] + 1.) * self.width as f32 / 2.,
                        (1. - v.position[1]) * self.height as f32 / 2.,
                    )
                });
                let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
                if area.abs() < 1e-6 {
                    continue;
                }
                let b = [
                    (p[1] - q).perp_dot(p[2] - q) / area,
                    (p[2] - q).perp_dot(p[0] - q) / area,
                    (p[0] - q).perp_dot(p[1] - q) / area,
                ];
                if b.iter().any(|w| *w < 0.) {
                    continue;
                }
                let uv = (0..3).fold(Vec2::ZERO, |uv, i| uv + Vec2::from_array(tri[i].uv) * b[i]);
                let x = (uv.x.clamp(0., 1.) * texture.width as f32)
                    .floor()
                    .min(texture.width as f32 - 1.) as usize;
                let y = (uv.y.clamp(0., 1.) * texture.height as f32)
                    .floor()
                    .min(texture.height as f32 - 1.) as usize;
                let texel = y * texture.width as usize + x;
                if texture.rgba[texel * 4 + 3] == 0 {
                    continue;
                }
                let depth = (0..3).map(|i| tri[i].position[2] * b[i]).sum::<f32>()
                    + (1. - texture.depth[texel] as f32 / 255.) * tri[0].depth_scale;
                if depth <= nearest {
                    nearest = depth;
                    object = draw.object;
                }
            }
        }
        object
    }
    fn quad(&mut self, vertices: [Vertex; 4], texture: usize) {
        let start = self.vertices.len() as u32;
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(vertices[i]);
        }
        self.draws.push(Draw {
            vertices: start..self.vertices.len() as u32,
            texture,
            object: None,
        });
    }
    fn surface(&mut self, c: &Camera, p: [Vec3; 4], tint: [f32; 4]) {
        self.quad(p.map(|p| c.vertex(p, [0.5; 2], tint)), 0);
    }
}
/// `fraction` interpolates presentation into the next simulation tick, without executing it.
pub fn room_scene(
    assets: &Assets,
    program: &Program,
    sim: &Simulation,
    camera: Camera,
    fraction: f32,
) -> Result<Scene> {
    room_scene_inner(assets, program, sim, camera, fraction, false, None)
}

fn room_scene_inner(
    assets: &Assets,
    program: &Program,
    sim: &Simulation,
    camera: Camera,
    fraction: f32,
    native_characters: bool,
    wall_view: Option<&wall_view::Plan>,
) -> Result<Scene> {
    if !(1..=2).contains(&camera.level)
        || camera.width == 0
        || camera.height == 0
        || camera.width > 4096
        || camera.height > 4096
        || !camera.zoom.is_finite()
        || camera.zoom <= 0.
        || !fraction.is_finite()
    {
        bail!("invalid viewport");
    }
    let fraction = fraction.clamp(0., 0.9999);
    let mut scene = Scene {
        vertices: vec![],
        draws: vec![],
        censors: vec![],
        textures: assets.textures.clone(),
        width: camera.width,
        height: camera.height,
    };
    let ground_size = assets
        .floor_materials
        .as_ref()
        .map(|_| floors::active_size(sim))
        .transpose()?;
    for level in 1..=camera.level {
        let floor_y = floor_elevation(level);
        let columns = ground_size
            .map(|n| floors::drawn_cells(n).map(|r| r.start as i32..r.end as i32))
            .transpose()?
            .unwrap_or(0..sim.room.width / 16);
        let rows = ground_size
            .map(|n| floors::drawn_cells(n).map(|r| r.start as i32..r.end as i32))
            .transpose()?
            .unwrap_or(0..sim.room.height / 16);
        for x in columns {
            for z in rows.clone() {
                if sim.architecture.as_ref().is_some_and(|a| {
                    !a.supported(level, sims_sim::Point::new(x * 16 + 8, z * 16 + 8))
                }) {
                    continue;
                }
                if scene.original_floor(
                    assets.floor_materials.as_ref(),
                    assets.ground.as_ref(),
                    assets.pool_materials.as_ref(),
                    assets.saved_road_rotation,
                    sim,
                    &camera,
                    level,
                    x,
                    z,
                    native_characters,
                )? {
                    continue;
                }
                let (x, z) = (x as f32, z as f32);
                let shade = if (x as i32 + z as i32) % 2 == 0 {
                    0.74
                } else {
                    0.69
                };
                let tint = if let Some(a) = &sim.architecture {
                    let room = a.room_at(
                        level,
                        sims_sim::Point::new(x as i32 * 16 + 8, z as i32 * 16 + 8),
                    );
                    let color = match room {
                        0 => [0.26, 0.37, 0.24],
                        1 => [0.59, 0.44, 0.28],
                        2 => [0.66, 0.78, 0.77],
                        _ => [0.48, 0.56, 0.60],
                    };
                    let shade = shade / 0.74;
                    [color[0] * shade, color[1] * shade, color[2] * shade, 1.]
                } else {
                    [shade, shade * 0.94, shade * 0.82, 1.]
                };
                let e = 0.013;
                scene.surface(
                    &camera,
                    [
                        Vec3::new(x + e, floor_y, z + e),
                        Vec3::new(x + 1. - e, floor_y, z + e),
                        Vec3::new(x + 1. - e, floor_y, z + 1. - e),
                        Vec3::new(x + e, floor_y, z + 1. - e),
                    ],
                    tint,
                );
            }
        }
        if scene.original_walls(
            assets.wall_materials.as_ref(),
            &assets.wall_shapes,
            &assets.object_wall_shapes,
            wall_view,
            sim,
            program,
            &camera,
            level,
            native_characters,
        )? {
            continue;
        }
        if let Some(a) = &sim.architecture {
            for wall in a.walls(level) {
                if wall.door.is_some() {
                    continue;
                }
                let p = Vec3::new(wall.a.x as f32 / 16., floor_y, wall.a.y as f32 / 16.);
                let q = Vec3::new(wall.b.x as f32 / 16., floor_y, wall.b.y as f32 / 16.);
                let mid = (p + q) * 0.5;
                let side = Vec3::new(q.z - p.z, 0., p.x - q.x).normalize_or_zero() * 0.045;
                let room = |v: Vec3| {
                    a.room_at(
                        level,
                        sims_sim::Point::new((v.x * 16.) as i32, (v.z * 16.) as i32),
                    )
                };
                let exterior = [room(mid + side * 2.), room(mid - side * 2.)]
                    .iter()
                    .any(|id| a.outdoors.get(id) == Some(&true));
                let rotated = camera.rotated(mid - camera.center);
                let height = if exterior && rotated.x + rotated.z < 0. {
                    2.5
                } else {
                    0.22
                };
                let up = Vec3::Y * height;
                scene.surface(
                    &camera,
                    [p - side, q - side, q - side + up, p - side + up],
                    [0.79, 0.76, 0.67, 1.],
                );
                scene.surface(
                    &camera,
                    [p + side, q + side, q + side + up, p + side + up],
                    [0.69, 0.72, 0.69, 1.],
                );
                scene.surface(
                    &camera,
                    [p - side + up, q - side + up, q + side + up, p + side + up],
                    [0.9, 0.88, 0.80, 1.],
                );
            }
        }
    }
    for o in &sim.room.obstacles {
        let (x, z, max_x, max_z) = (
            o.min.x as f32 / 16.,
            o.min.y as f32 / 16.,
            o.max.x as f32 / 16.,
            o.max.y as f32 / 16.,
        );
        let h = 0.8;
        scene.surface(
            &camera,
            [
                Vec3::new(x, h, z),
                Vec3::new(max_x, h, z),
                Vec3::new(max_x, h, max_z),
                Vec3::new(x, h, max_z),
            ],
            [0.36, 0.53, 0.53, 1.],
        );
        for (a, b) in [
            ((x, z), (max_x, z)),
            ((max_x, z), (max_x, max_z)),
            ((max_x, max_z), (x, max_z)),
            ((x, max_z), (x, z)),
        ] {
            scene.surface(
                &camera,
                [
                    Vec3::new(a.0, 0., a.1),
                    Vec3::new(b.0, 0., b.1),
                    Vec3::new(b.0, h, b.1),
                    Vec3::new(a.0, h, a.1),
                ],
                [0.25, 0.39, 0.4, 1.],
            );
        }
    }
    let mut residents = BTreeMap::new();
    for (&id, o) in &sim.vm.world.objects {
        if o.person.is_some() && sim.bodies.contains_key(&id) {
            residents.insert(
                id,
                if native_characters {
                    original_house::actor_pose(sim, id)?
                } else {
                    actor_pose(assets, program, sim, id, fraction)?
                },
            );
        }
    }
    for (&id, chair) in &sim.bodies {
        let object = &sim.vm.world.objects[&id];
        let mut root = id;
        while let Some((parent, _)) = sim.vm.world.objects[&root].container {
            root = parent;
        }
        if object.data.get(&34).is_some_and(|v| *v != 0)
            || !visible_on_floor(sim, root, camera.level)
        {
            continue;
        }
        let Some(def) = &object.definition else {
            continue;
        };
        if object.person.is_some()
            || program.modules[&def.source]
                .object_definition_words
                .get(&def.object)
                .and_then(|w| w.get(3))
                == Some(&0)
        {
            continue;
        }
        let Some(&(base, count)) = assets
            .object_graphics
            .get(&(def.source.clone(), def.object))
        else {
            bail!(
                "missing sprite assets for object #{id}: {} OBJD {}",
                def.source,
                def.object
            );
        };
        let Some(graphic) = graphic_group(base, count, object.data.get(&0).copied().unwrap_or(0))
        else {
            // Native 0x45764f returns null outside its cached DGRP vector.
            // There is no sprite to draw. Editor fallback bounds are separate.
            continue;
        };
        // Native 0x437fdd recursively draws contained children after adding the
        // parent's rotated SLOT XYZ offset. Height is a placement category, not
        // a rendering whitelist or literal elevation. Bone attachment terminates
        // the ordinary-container chain at a person.
        let mut cp = Vec3::ZERO;
        let mut cursor = id;
        let mut hidden = false;
        let mut seen = std::collections::BTreeSet::new();
        loop {
            if !seen.insert(cursor) {
                bail!("container presentation cycle");
            }
            let current = &sim.vm.world.objects[&cursor];
            hidden |= current.data.get(&34).is_some_and(|v| *v != 0)
                || wall_view
                    .map(|v| v.hides_object(sim, cursor))
                    .transpose()?
                    .unwrap_or(false);
            let Some((parent, slot)) = current.container else {
                let body = &sim.bodies[&cursor];
                cp += Vec3::new(
                    body.position.x as f32 / 16.,
                    floor_elevation(body.level)
                        + if native_characters {
                            original_house::ground_elevation(
                                sim,
                                body.position.x as f32 / 16.,
                                body.position.y as f32 / 16.,
                            )?
                        } else {
                            0.
                        },
                    body.position.y as f32 / 16.,
                );
                break;
            };
            let container = &sim.vm.world.objects[&parent];
            if container.person.is_some() {
                if slot != 0 {
                    bail!("unsupported person presentation slot {slot}");
                }
                let index = assets
                    .skeleton(parent)
                    .bones
                    .iter()
                    .position(|b| b.name.eq_ignore_ascii_case("R_FINGER0"))
                    .context("adult carry bone missing")?;
                let (pos, bones, turn) = residents
                    .get(&parent)
                    .context("held object parent pose missing")?;
                cp += *pos + *turn * bones[index].transform_point3(Vec3::ZERO) / 3.;
                hidden |= container.data.get(&34).is_some_and(|v| *v != 0);
                break;
            }
            let d = container
                .definition
                .as_ref()
                .context("container definition missing")?;
            let slot = program.modules[&d.source]
                .slots
                .get(&d.slots)
                .and_then(|s| s.entries.iter().filter(|s| s.kind == 0).nth(slot as usize))
                .context("container presentation slot missing")?;
            // 0x454f29 converts native vertical units to tiles; the character
            // animator multiplies by three to obtain Vitaboy model units.
            let offset = Vec3::new(
                slot.offset[0] / 16.,
                sims_vm::routing::containment_height(slot.height, slot.offset[2]).1
                    / (2. * 6.0f32.sqrt()),
                slot.offset[1] / 16.,
            );
            cp += Quat::from_rotation_y(-direction(sim.bodies[&parent].facing)) * offset;
            cursor = parent;
        }
        if hidden {
            continue;
        }
        let bits = (chair.facing as u32).rotate_left(camera.rotation as u32 * 2);
        let dir = (bits & 255) | (bits >> 8);
        let rot = Quat::from_rotation_y(-direction(chair.facing));
        let (sprite_zoom, pixel_scale) = camera.sprite_zoom();
        for s in assets
            .sprites
            .get(&(def.source.clone(), graphic, dir, sprite_zoom))
            .with_context(|| {
                format!(
                    "missing object view: {} DGRP {graphic}, direction {dir}, zoom {sprite_zoom}",
                    def.source
                )
            })?
        {
            let base = program.modules[&def.source].object_definition_words[&def.object]
                .get(0x80 / 2)
                .copied()
                .unwrap_or(0);
            if !sprite_visible(s.id as i16, base, &object.dynamic_sprite_flags) {
                continue;
            }
            let t = &assets.textures[s.texture];
            let object = rot * s.object;
            let center = cp + object;
            let origin = camera.screen(center) + s.offset * pixel_scale;
            let front = match camera.rotation {
                0 => Vec3::new(1., 0., 1.),
                1 => Vec3::new(1., 0., -1.),
                2 => Vec3::new(-1., 0., -1.),
                _ => Vec3::new(-1., 0., 1.),
            };
            let back = center - front * 0.45;
            let scale = (camera.depth(back + front) - camera.depth(back)) / 0.4;
            let left = if s.flip { 1. } else { 0. };
            let right = 1. - left;
            let points = [
                (Vec2::ZERO, [left, 0.]),
                (Vec2::new(t.width as f32, 0.), [right, 0.]),
                (Vec2::new(t.width as f32, t.height as f32), [right, 1.]),
                (Vec2::new(0., t.height as f32), [left, 1.]),
            ];
            scene.quad(
                points.map(|(p, uv)| {
                    camera.screen_vertex(
                        origin + p * pixel_scale,
                        camera.depth(back),
                        uv,
                        [1.; 4],
                        scale,
                    )
                }),
                s.texture,
            );
            scene.draws.last_mut().unwrap().object = Some(id);
        }
    }
    for (&id, (pos, bones, turn)) in &residents {
        if !visible_on_floor(sim, id, camera.level) {
            continue;
        }
        if native_characters
            && sim.vm.world.objects[&id]
                .data
                .get(&34)
                .is_some_and(|v| *v != 0)
        {
            continue;
        }
        let (pos, turn) = (*pos, *turn);
        let outfit = sim.vm.world.objects[&id]
            .person
            .as_ref()
            .and_then(|p| p.get(&8))
            .copied()
            .unwrap_or(0);
        let (appearance, texture_base) = if let Some(resident) = assets.residents.get(&id) {
            anyhow::ensure!(
                outfit == resident.outfit,
                "resident #{id} changed outfit; original appearance reload is required"
            );
            (&resident.avatar, resident.texture_base)
        } else {
            match outfit {
                0 if id != 1 => (&assets.alternate, assets.alternate_texture),
                0 => (&assets.avatar, 1),
                1 => (&assets.nude, assets.nude_texture),
                5 => (&assets.sleepwear, assets.sleepwear_texture),
                _ => bail!("adult appearance profile does not include outfit {outfit}"),
            }
        };
        let mut parts: Vec<_> = appearance
            .parts
            .iter()
            .enumerate()
            .map(|(i, p)| (p, texture_base + i))
            .collect();
        for name in sim.vm.world.objects[&id]
            .accessories
            .iter()
            .filter(|_| !native_characters)
        {
            let accessory = assets
                .accessories
                .get(name)
                .with_context(|| format!("missing accessory appearance {name}"))?;
            parts.extend(accessory.iter().map(|(p, texture)| (p, *texture)));
        }
        let mut censor_points = vec![];
        for (part, texture) in parts {
            let verts = pose::skin(&part.mesh, &appearance.skeleton, &bones)?;
            if part.censor {
                for (p, _) in &verts[..part.mesh.vertices.len()] {
                    let p = pos + turn * *p / 3.;
                    let screen = camera.screen(p);
                    censor_points.push([screen.x, screen.y, camera.depth(p)]);
                }
                continue;
            }
            let start = scene.vertices.len() as u32;
            for face in &part.mesh.triangles {
                for &j in face {
                    let (p, n) = verts[j as usize];
                    let n = (turn * n).normalize_or_zero();
                    let shade =
                        0.72 + 0.28 * n.dot(Vec3::new(-0.35, 0.8, -0.4).normalize()).max(0.);
                    scene.vertices.push(camera.vertex(
                        pos + turn * p / 3.,
                        part.mesh.vertices[j as usize].uv,
                        [shade, shade, shade, 1.],
                    ));
                }
            }
            scene.draws.push(Draw {
                vertices: start..scene.vertices.len() as u32,
                texture,
                object: Some(id),
            });
        }
        if let Some(region) = censor::Region::projected(
            &censor_points,
            camera.zoom,
            sim.vm.tick as u32 ^ u32::from(id as u16),
        ) {
            scene.censors.push(region);
        }
    }
    if assets.floor_materials.is_some() {
        scene.fit_depth_range();
    }
    if let Some(texture) = assets.original_background {
        scene.background(texture);
    }
    Ok(scene)
}
/// One storey spans nine Vitaboy model units. Both original adult stair clips
/// encode that rise; model geometry is scaled by three into the renderer's tiles.
pub fn floor_elevation(level: i32) -> f32 {
    (level - 1) as f32 * 3.
}
fn visible_on_floor(sim: &Simulation, id: i16, selected: i32) -> bool {
    let body = &sim.bodies[&id];
    // The original ascending tree sets person field 40 until the climb ends.
    // We use that interval for lower-floor visibility; native draw filtering
    // itself has not yet been recovered for this flag.
    let ascending = sim.vm.world.objects[&id]
        .person
        .as_ref()
        .is_some_and(|p| p.get(&40).is_some_and(|v| *v != 0));
    body.level - i32::from(ascending) <= selected
}
fn actor_pose(
    assets: &Assets,
    program: &Program,
    sim: &Simulation,
    id: i16,
    fraction: f32,
) -> Result<(Vec3, Vec<glam::Mat4>, Quat)> {
    let actor = sim.bodies.get(&id).context("resident body missing")?;
    let mut pos = Vec3::new(
        actor.position.x as f32 / 16.,
        floor_elevation(actor.level),
        actor.position.y as f32 / 16.,
    );
    let (mut bones, heading) = if let Some(route) = sim.routes.values().find(|r| r.caller == id) {
        let sample = route.locomotion.sample(&route.path, fraction);
        pos = Vec3::new(
            sample.position[0] / 16.,
            floor_elevation(actor.level),
            sample.position[1] / 16.,
        );
        (
            pose::locomotion(assets.skeleton(id), program, &route.locomotion, sample)?,
            sample.heading,
        )
    } else if let Some((clip, frame, heading)) =
        sim.turns.get(&id).and_then(|t| t.sample(program, fraction))
    {
        (
            pose::evaluate(assets.skeleton(id), &program.animations[clip], frame, false)?,
            heading,
        )
    } else if let Some(p) = sim.animations.get(&id) {
        let a = program
            .animations
            .get(&p.clip)
            .context("active animation missing")?;
        (
            pose::evaluate(
                assets.skeleton(id),
                a,
                if p.reverse {
                    ((a.clip.frames - 1) as f32
                        - p.frame_fifths as f32 / 5.
                        - fraction * p.speed as f32 / 5.)
                        .max(0.)
                } else {
                    p.frame_fifths as f32 / 5. + fraction * p.speed as f32 / 5.
                },
                p.looping,
            )?,
            direction(actor.facing),
        )
    } else {
        let a = program
            .animations
            .get(
                assets
                    .residents
                    .get(&id)
                    .map_or(sims_sim::locomotion::STAND, |r| r.idle.as_str()),
            )
            .context("standing animation missing")?;
        (
            pose::evaluate(assets.skeleton(id), a, sim.vm.tick as f32 * 1.2, true)?,
            direction(actor.facing),
        )
    };
    // Locomotion and standing turns provide a full body pose. Otherwise compose
    // the looped background underneath the current foreground's authored channels.
    if !sim.routes.values().any(|r| r.caller == id) && !sim.turns.contains_key(&id) {
        if let Some(layer) = sim.backgrounds.get(&id) {
            bones = pose::background_underlay(
                assets.skeleton(id),
                &bones,
                program
                    .animations
                    .get(&layer.clip)
                    .context("background animation missing")?,
                ((sim.vm.tick - layer.started_tick) as f32 + fraction) * 1.2,
                sim.animations
                    .get(&id)
                    .and_then(|a| program.animations.get(&a.clip)),
            )?;
        }
    }
    if let Some((clip, frame)) = sim.carry_frame(id, fraction) {
        bones = pose::carry_overlay(
            assets.skeleton(id),
            &bones,
            program
                .animations
                .get(clip)
                .context("carry animation missing")?,
            frame,
        )?;
    }
    let turn = Quat::from_rotation_y(std::f32::consts::PI - heading);
    Ok((pos, bones, turn))
}
/// Native initialized DGRP-vector selection. The loader restricts the base
/// resource range to non-overflowing IDs; an invalid selector has no sprite.
pub fn graphic_group(base: u16, count: u16, index: i16) -> Option<u16> {
    let index = u16::try_from(index).ok()?;
    (index < count).then(|| base.checked_add(index)).flatten()
}
fn direction(facing: u8) -> f32 {
    facing.trailing_zeros() as f32 * std::f32::consts::FRAC_PI_4
}

/// Original 0x437f99 uses one signed word per dynamic sprite, not a bitset.
pub fn sprite_visible(sprite: i16, base: i16, flags: &[i16]) -> bool {
    let index = i32::from(sprite) - i32::from(base);
    if flags.is_empty() || index < 0 || index as usize >= flags.len() {
        true
    } else {
        flags[index as usize] != 0
    }
}
