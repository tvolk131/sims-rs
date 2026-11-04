//! Original lot materials and live native character poses. Rendering is read-only:
//! the session owns animation sampling, root movement and appearance changes.
use crate::{Assets, Camera, Scene, floor_elevation, pose, room_scene_inner};
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Quat, Vec3};
use sims_content::saved_avatar::BindingState;
use sims_runtime::original_session::Session;
use std::collections::{BTreeMap, BTreeSet};

pub struct OriginalHouse {
    assets: Option<Assets>,
    definitions: BTreeSet<(String, u16)>,
    bindings: BTreeMap<i16, BindingState>,
    static_textures: usize,
    resource_scope: Option<ResourceScope>,
}
///Exact inputs of the decoded resource binding, separate from live geometry.
///Compared only at the original reclamation boundary, never each VM slice.
#[derive(PartialEq)]
struct ResourceScope {
    root: std::path::PathBuf,
    source: String,
    sources: BTreeMap<String, String>,
    dimensions: [usize; 2],
    floors: Vec<(Vec<u16>, Vec<sims_formats::lot::WallTile>)>,
    floor_map: Vec<sims_formats::lot::SurfaceMapping>,
    wall_map: Vec<sims_formats::lot::SurfaceMapping>,
    arrays: BTreeMap<u16, sims_formats::lot::Array>,
    road_rotation: i16,
    definitions: BTreeMap<(String, u16), Vec<i16>>,
}
impl ResourceScope {
    fn new(s: &Session, definitions: &BTreeSet<(String, u16)>) -> Option<Self> {
        let definitions = definitions
            .iter()
            .map(|(source, id)| {
                Some((
                    (source.clone(), *id),
                    s.program
                        .modules
                        .get(source)?
                        .object_definition_words
                        .get(id)?
                        .clone(),
                ))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(Self {
            root: s.index().root.clone(),
            source: s.house.source.clone(),
            sources: s.house.sources.clone(),
            dimensions: [s.house.width, s.house.height],
            floors: s
                .house
                .floors
                .iter()
                .map(|f| (f.coverings.clone(), f.walls.clone()))
                .collect(),
            floor_map: s.house.floor_map.clone(),
            wall_map: s.house.wall_map.clone(),
            //Only Arry6 grass and Arry9 pool participate in binding. Tile
            //heads and unrelated saved arrays can change during Live/Buy.
            arrays: s
                .house
                .arrays
                .iter()
                .filter(|(id, _)| matches!(**id, 6 | 9))
                .map(|(&id, a)| (id, a.clone()))
                .collect(),
            road_rotation: s.house.view.camera_direction,
            definitions,
        })
    }
}
impl Default for OriginalHouse {
    fn default() -> Self {
        Self {
            assets: None,
            definitions: BTreeSet::new(),
            bindings: BTreeMap::new(),
            static_textures: 0,
            resource_scope: None,
        }
    }
}
impl OriginalHouse {
    ///Collect excessive unused decoded assets; retain a bounded immutable spare
    ///cache so selection indicators do not force a whole-house decode. Build a
    ///replacement first on invalidation. VM clips/skeletons remain Session owned.
    pub fn rebuild_resources(&mut self, session: &Session, camera: Camera) -> Result<Scene> {
        let live = Self::live_definitions(session);
        let scope = ResourceScope::new(session, &self.definitions);
        let spare_bytes = self.spare_sprite_bytes(&live);
        if std::env::var_os("SIMS_PROFILE_RENDER_RESOURCES").is_some() {
            eprintln!(
                "renderer reclamation: live={} retained={} same_scope={} spare_bytes={} removed={:?} added={:?}",
                live.len(),
                self.definitions.len(),
                scope.is_some() && self.resource_scope == scope,
                spare_bytes,
                self.definitions.difference(&live).collect::<Vec<_>>(),
                live.difference(&self.definitions).collect::<Vec<_>>()
            );
        }
        if self.assets.is_some()
            && live.is_subset(&self.definitions)
            && self.definitions.len() - live.len() <= 32
            && spare_bytes <= 8 * 1024 * 1024
            && scope.is_some()
            && self.resource_scope == scope
        {
            //Reconcile current appearances and rebuild geometry/lighting without decoding the
            //same immutable sprites, wall masks and grass texture again.
            return self.scene(session, camera);
        }
        let mut replacement = Self::default();
        let scene = replacement.scene(session, camera)?;
        *self = replacement;
        Ok(scene)
    }
    fn live_definitions(session: &Session) -> BTreeSet<(String, u16)> {
        let sim = &session.simulation;
        sim.vm
            .world
            .objects
            .iter()
            .filter(|(id, o)| {
                sim.bodies.contains_key(id)
                    && o.person.is_none()
                    && o.data.get(&34).is_none_or(|v| *v == 0)
            })
            .filter_map(|(_, o)| o.definition.as_ref().map(|d| (d.source.clone(), d.object)))
            .collect()
    }
    fn spare_sprite_bytes(&self, live: &BTreeSet<(String, u16)>) -> usize {
        let Some(assets) = &self.assets else {
            return 0;
        };
        let mut groups = BTreeSet::new();
        for definition in live {
            if let Some(&(base, count)) = assets.object_graphics.get(definition) {
                for id in u32::from(base)..u32::from(base) + u32::from(count) {
                    groups.insert((definition.0.as_str(), id as u16));
                }
            }
        }
        let mut used = BTreeSet::new();
        let mut unused = BTreeSet::new();
        for ((source, group, _, _), sprites) in &assets.sprites {
            let textures = if groups.contains(&(source.as_str(), *group)) {
                &mut used
            } else {
                &mut unused
            };
            textures.extend(sprites.iter().map(|s| s.texture));
        }
        let mut apertures = BTreeSet::new();
        if let Some(scope) = &self.resource_scope {
            for definition in live {
                if let Some(words) = scope.definitions.get(definition) {
                    if words
                        .get(0x5c / 2)
                        .is_some_and(|kind| matches!(*kind, 22 | 23))
                    {
                        apertures.insert((
                            definition.0.as_str(),
                            words.get(0xa2 / 2).copied().unwrap_or(0) as u16,
                        ));
                    }
                }
            }
        }
        for ((_, source, base, _), frames) in &assets.object_wall_shapes {
            let textures = if apertures.contains(&(source.as_str(), *base)) {
                &mut used
            } else {
                &mut unused
            };
            textures.extend(frames.iter().flatten().copied());
        }
        unused
            .difference(&used)
            .map(|&i| assets.textures[i].rgba.len() + assets.textures[i].depth.len())
            .sum()
    }
    /// Replacement renderer point for a native physical coordinate, including
    /// the actual lot terrain. Forward pointer projection must use this same
    /// surface as the inverse ground hit.
    pub fn ground_point(session: &Session, position: [i32; 2], level: i32) -> Result<Vec3> {
        let x = position[1] as f32 / 16.;
        let z = position[0] as f32 / 16.;
        Ok(Vec3::new(
            x,
            floor_elevation(level) + ground_elevation(&session.simulation, x, z)?,
            z,
        ))
    }
    /// Actual retained native origin/zoom; does not automatically recenter on
    /// the building or the selected person.
    pub fn native_camera(session: &Session) -> Result<Camera> {
        let p = session
            .host
            .presentation
            .presenter
            .as_ref()
            .context("native house view absent")?;
        let settings = &p
            .camera
            .as_ref()
            .context("native camera settings absent")?
            .settings;
        let [left, top, right, bottom] = settings.viewport;
        ensure!(
            left == 0 && top == 0 && right > 0 && bottom > 0,
            "native scene viewport unavailable"
        );
        Camera::from_native(
            sims_sim::pose::portrait::view::Input {
                viewport: [0, 0, right as u32, bottom as u32],
                center: p.view.origin,
                zoom: p.view.zoom,
                rotation: p.view.display_rotation,
            },
            p.view.floor.level,
        )
    }
    pub fn camera(
        session: &Session,
        width: u32,
        height: u32,
        rotation: u8,
        zoom: f32,
        level: i32,
    ) -> Camera {
        let mut camera =
            Camera::room(&session.simulation, width, height, rotation, zoom).floor(level);
        if let Some(lot) = session
            .simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
        {
            let points: Vec<_> = lot.floors[0]
                .rooms
                .primary
                .iter()
                .enumerate()
                .filter(|(_, r)| lot.floors[0].metrics.get(r).is_some_and(|m| !m.outdoors))
                .map(|(i, _)| [(i / lot.size) as f32, (i % lot.size) as f32])
                .collect();
            if !points.is_empty() {
                for axis in [0, 1] {
                    let lo = points.iter().map(|p| p[axis]).reduce(f32::min).unwrap();
                    let hi = points.iter().map(|p| p[axis]).reduce(f32::max).unwrap() + 1.;
                    if axis == 0 {
                        camera.center.x = (lo + hi) * 0.5;
                    } else {
                        camera.center.z = (lo + hi) * 0.5;
                    }
                }
            }
        }
        camera.center.y += ground_elevation(&session.simulation, camera.center.x, camera.center.z)
            .expect("loaded lot has valid terrain");
        camera
    }
    pub fn scene(&mut self, session: &Session, camera: Camera) -> Result<Scene> {
        // Native flags describe the retained view. Free preview cameras keep
        // the old heuristic rather than reusing another floor/rotation's mask.
        let retained_view = session
            .host
            .presentation
            .presenter
            .as_ref()
            .is_some_and(|p| {
                p.view.display_rotation == u32::from(camera.rotation)
                    && p.view.floor.level == camera.level
            });
        let walls = retained_view
            .then(|| crate::wall_view::Plan::from_session(session))
            .transpose()?
            .flatten();
        self.scene_with_wall_plan(session, camera, walls.as_ref())
    }
    /// Read-only reference-rendering seam. A supplied plan changes presentation
    /// inputs only; it is never imported into the retained simulation state.
    pub fn scene_with_wall_plan(
        &mut self,
        session: &Session,
        camera: Camera,
        walls: Option<&crate::wall_view::Plan>,
    ) -> Result<Scene> {
        let sim = &session.simulation;
        let definitions = Self::live_definitions(session);
        if self.assets.is_none() || !definitions.is_subset(&self.definitions) {
            let definitions: BTreeSet<_> = self.definitions.union(&definitions).cloned().collect();
            let mut assets = Assets::load_objects_profile(
                session.index(),
                &definitions
                    .iter()
                    .map(|(s, id)| (s.as_str(), *id))
                    .collect::<Vec<_>>(),
                Some(&BTreeSet::new()),
            )?;
            assets.bind_original_floors(session.index(), &session.house)?;
            assets.bind_original_walls(
                session.index(),
                &session.house,
                &session.program,
                &definitions,
            )?;
            self.static_textures = assets.textures.len();
            self.resource_scope = ResourceScope::new(session, &definitions);
            self.assets = Some(assets);
            self.definitions = definitions;
            self.bindings.clear();
        }
        let bindings: BTreeMap<_, _> = session
            .host
            .characters
            .iter()
            .filter(|(id, _)| sim.characters.contains_key(id))
            .map(|(&id, c)| (id, c.avatar.snapshot()))
            .collect();
        let assets = self.assets.as_mut().unwrap();
        if self.bindings != bindings {
            std::sync::Arc::make_mut(&mut assets.textures).truncate(self.static_textures);
            assets.residents.clear();
            for &id in bindings.keys() {
                let outfit = sim.vm.world.objects[&id]
                    .person
                    .as_ref()
                    .context("native person fields absent")?[&8];
                assets.bind_resident(
                    id,
                    session.host.characters[&id].avatar.avatar.clone(),
                    outfit,
                    String::new(),
                )?;
            }
            self.bindings = bindings;
        }
        ensure!(sim.vm.world.objects.iter().filter(|(_,o)|o.person.is_some()).all(|(id,_)|
            sim.characters.contains_key(id) && assets.residents.contains_key(id)),"native person has no completed presentation owner");
        // An outfit word can change before the appearance operation finishes;
        // bound meshes remain authoritative until that operation completes.
        for (&id, r) in &mut assets.residents {
            r.outfit = sim.vm.world.objects[&id].person.as_ref().unwrap()[&8];
        }
        room_scene_inner(assets, &session.program, sim, camera, 0., true, walls)
    }
}

/// Convert Arry-0 height into room tiles using the animator's spilled scale.
pub(crate) fn terrain_height(height: f64) -> f32 {
    let scale = (1. / (2. * 6_f64.sqrt())) as f32;
    (f64::from(scale) * 3. * height) as f32 / 3.
}

pub(crate) fn ground_elevation(sim: &sims_sim::Simulation, x: f32, z: f32) -> Result<f32> {
    let lot = sim
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("native scene requires original terrain")?;
    let terrain = sims_sim::pose::transform::Terrain {
        size: lot.size,
        corners: &lot.heights,
    };
    // Renderer columns/rows are the inverse of the native physical word order.
    Ok(terrain
        .height([(z * 16.).round() as i32, (x * 16.).round() as i32])
        .map_err(anyhow::Error::msg)?
        .map_or(0., terrain_height))
}

pub(crate) fn actor_pose(sim: &sims_sim::Simulation, id: i16) -> Result<(Vec3, Vec<Mat4>, Quat)> {
    let resident = sim.characters.get(&id).context("native character absent")?;
    let character = &resident.character;
    let bones = character.displayed_pose().map_err(anyhow::Error::msg)?;
    let bones = pose::from_placed_absolute_bones(&bones, character.model.bounds_transform)?;
    // Native placement uses [physical row, height, physical column], in three
    // Vitaboy units per tile. After handedness conversion, this rotation maps
    // it into the room renderer's [column, height, row] coordinates. Do not add
    // a second body translation or heading: both are already in the model.
    let basis = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let bones = bones.into_iter().map(|m| basis * m).collect();
    let level = sim.bodies.get(&id).context("native body absent")?.level;
    Ok((
        Vec3::new(0., floor_elevation(level), 0.),
        bones,
        Quat::IDENTITY,
    ))
}
