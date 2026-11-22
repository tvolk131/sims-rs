//! Two-tier static lot-surface cache for `Pass::Lot` ([`crate::draw_lot_with_preview`]).
//!
//! - **Tier 2 (`Full`)** caches the completed `Pass::Lot` [`Canvas`] (`color`,
//!   `depth`, `ids`, `pen`, and `rects`) plus [`Stats`], and restores it via
//!   fast slice copies whenever every visual input to `Pass::Lot` matches
//!   bit-for-bit (for example while Sims animate, the plumb bob spins, or a pie
//!   menu opens).
//! - **Tier 1 (`Environment`)** caches the level-1 static environment right
//!   after `trims(1)` and before `objects(1)` (background, terrain/grass,
//!   Buy/Build grid, level-1 floors and floor shadows, level-1 walls with their
//!   wall-shaping cutouts, and trims/caps). When only non-environment object
//!   state changes (for example an appliance/TV animating, a lamp's glow word
//!   toggling, a door swinging open/closed without moving its aperture, or a
//!   non-shadowing Buy preview moving), [`LotCache`] restores Tier 1 via fast
//!   slice copies and repaints only the object pass (and level 2 when viewing
//!   floor 2) on top.
//! - It also caches the computed [`Grass`] tables across camera pans (since
//!   `Grass` depends on terrain/grass/level-1 walls & floors/time-of-day/
//!   rotation/zoom, not `camera.origin`).

use crate::assets::Assets;
use crate::camera::{Camera, Walls, Zoom};
use crate::canvas::Canvas;
use crate::cutaway::{Focus, Mask};
use crate::grass::Grass;
use crate::lot::{Lot, WallCell};
use crate::scene::{
    Edge, RenderOptions, Stats, background, holds_person, paint_lot_from_env,
    paint_lot_with_env_hook, shadow_of_chain, wall_objects_on_level,
};
use sims_sim::{Object, World};
use std::collections::{BTreeMap, BTreeSet};

/// Which tier satisfied the most recent [`LotCache::draw_lot_with_preview`] call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheHit {
    /// Tier 2 hit: restored the full `Pass::Lot` canvas directly.
    Full,
    /// Tier 1 hit: restored level-1 static environment and painted only objects
    /// (plus level 2 if active) on top.
    Environment,
    /// Full cache miss: painted terrain/floors/walls/trims and objects.
    #[default]
    Miss,
}

/// Two-tier cache for the static `Pass::Lot` surface and per-zoom/rotation
/// grass tables.
///
/// Surface entries are bound to the [`Assets`] instance so switching installs
/// or catalogs invalidates automatically; call [`LotCache::clear`] to drop
/// cached memory immediately.
#[derive(Default)]
pub struct LotCache {
    /// Tier 2: complete `Pass::Lot` surface including non-held objects.
    entry: Option<Entry>,
    /// Tier 1: level-1 static environment (before `objects(1)`).
    env: Option<EnvEntry>,
    grass: Option<GrassEntry>,
    last_hit: CacheHit,
}

struct Entry {
    key: LotKey,
    canvas: Canvas,
    stats: Stats,
}

struct EnvEntry {
    key: EnvKey,
    canvas: Canvas,
    stats: Stats,
}

struct GrassEntry {
    key: GrassKey,
    grass: Grass,
}

struct GrassKey {
    size: usize,
    time_of_day: i16,
    rotation: u8,
    zoom: Zoom,
    heights: Vec<[u8; 4]>,
    grass: Vec<i8>,
    floors0: Vec<u16>,
    walls0: Vec<WallCell>,
}

impl GrassKey {
    fn from(lot: &Lot, camera: &Camera) -> Self {
        Self {
            size: lot.size,
            time_of_day: lot.time_of_day,
            rotation: camera.rotation & 3,
            zoom: camera.zoom,
            heights: lot.heights.clone(),
            grass: lot.grass.clone(),
            floors0: lot.levels[0].floors.clone(),
            walls0: lot.levels[0].walls.clone(),
        }
    }

    fn matches(&self, lot: &Lot, camera: &Camera) -> bool {
        self.size == lot.size
            && self.time_of_day == lot.time_of_day
            && self.rotation == (camera.rotation & 3)
            && self.zoom == camera.zoom
            && self.heights == lot.heights
            && self.grass == lot.grass
            && self.floors0 == lot.levels[0].floors
            && self.walls0 == lot.levels[0].walls
    }
}

/// Render-relevant snapshot of one object read by `Pass::Lot`.
#[derive(Clone, PartialEq, Eq)]
struct ObjectVisual {
    id: u16,
    guid: u32,
    is_person: bool,
    position: [i32; 3],
    /// Data words `[0, 1, 2, 3, 4, 8, 13, 34, 51]` (word 3 defaults to `-1`,
    /// all others default to `0`).
    words: [i16; 9],
    dynamic_sprites: Vec<i16>,
    multipart: Option<(i16, [i32; 3])>,
}

impl ObjectVisual {
    #[inline]
    fn words_of(o: &Object) -> [i16; 9] {
        let w = |i: usize| o.data.get(i).copied().unwrap_or(0);
        [
            w(0),
            w(1),
            w(2),
            o.data.get(3).copied().unwrap_or(-1),
            w(4),
            w(8),
            w(13),
            w(34),
            w(51),
        ]
    }

    #[inline]
    fn multipart_of(o: &Object) -> Option<(i16, [i32; 3])> {
        o.multipart.as_ref().map(|m| (m.parent, m.offset))
    }

    fn from_object(id: u16, o: &Object) -> Self {
        Self {
            id,
            guid: o.guid,
            is_person: o.person.is_some(),
            position: o.position,
            words: Self::words_of(o),
            dynamic_sprites: o.dynamic_sprites.clone(),
            multipart: Self::multipart_of(o),
        }
    }

    #[inline]
    fn matches(&self, id: u16, o: &Object) -> bool {
        self.id == id
            && self.guid == o.guid
            && self.is_person == o.person.is_some()
            && self.position == o.position
            && self.words == Self::words_of(o)
            && self.dynamic_sprites == o.dynamic_sprites
            && self.multipart == Self::multipart_of(o)
    }
}

/// Complete visual key covering every input read by level-1 static environment
/// (`clear` + `floors(1)` + `corners(1)` + `walls(1)` + `trims(1)`).
struct EnvKey {
    assets_id: u64,
    width: usize,
    height: usize,
    clip_bottom: i32,
    zoom: Zoom,
    rotation: u8,
    origin: [i32; 2],
    /// Effective wall-cut mode for level 1 (`camera.walls` when `top == 1`,
    /// `Walls::Up` when `top == 2` since lower floors never cut away).
    env_walls: Walls,
    /// Focus that produced `cut_mask` when `env_walls == Walls::Cutaway`.
    focus: Focus,
    cut_mask: Option<Mask>,
    options: RenderOptions,
    time_of_day: i16,
    house: i16,
    show_grid: bool,
    grid_flags: Option<Vec<u8>>,
    lot: Lot,
    /// Resolved level-1 floor shadows (`(cell, code, strength)`).
    shadows0: Vec<(u16, u8, i16)>,
    /// Resolved level-1 wall-shaping object apertures (`Edge -> (resource, shape)`).
    wall_openings0: BTreeMap<Edge, (usize, u16)>,
}

/// Complete visual key covering every input read by `paint(..., Pass::Lot)`.
struct LotKey {
    assets_id: u64,
    width: usize,
    height: usize,
    clip_bottom: i32,
    camera: Camera,
    cut_mask: Option<Mask>,
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    time_of_day: i16,
    house: i16,
    show_grid: bool,
    grid_flags: Option<Vec<u8>>,
    lot: Lot,
    held: BTreeSet<u16>,
    /// Resolved `(cell, code, strength)` shadows per active level, so walking
    /// Sims (which modify raw array `3` but cast no floor shadow) never cause
    /// spurious cache misses while any actual shadow-caster change invalidates.
    shadows: [Vec<(u16, u8, i16)>; 2],
    objects: Vec<ObjectVisual>,
}

/// Whether `o` must be tracked in `LotKey::objects`.
///
/// All non-person objects are always tracked. Person objects are skipped
/// (their animation/movement is drawn in the people pass) unless they shape a
/// wall or are referenced by an active Buy Mode `preview`.
fn object_included(id: u16, o: &Object, assets: &mut Assets, preview: Option<(u16, u16)>) -> bool {
    if o.person.is_none() {
        return true;
    }
    if preview.is_some_and(|(c, h)| {
        id == c
            || id == h
            || o.multipart
                .as_ref()
                .is_some_and(|m| m.parent == c as i16 || m.parent == h as i16)
    }) {
        return true;
    }
    assets
        .definition_ref(o.guid)
        .is_some_and(|d| d.shapes_wall())
}

fn collect_level_shadows(
    world: &World,
    assets: &mut Assets,
    rotation: u8,
    lot_size: usize,
    level_index: usize,
) -> Vec<(u16, u8, i16)> {
    let mut out = Vec::new();
    let Some(grid) = world.arrays.get(&[3, 103][level_index]) else {
        return out;
    };
    let has_zero = world.objects.contains_key(&0);
    for x in 0..lot_size {
        for y in 0..lot_size {
            let cell = Lot::cell(x, y);
            let Some(word) = grid.data.get(cell * 2..cell * 2 + 2) else {
                continue;
            };
            let head = i16::from_le_bytes([word[0], word[1]]);
            if head < 0 || (head == 0 && !has_zero) {
                continue;
            }
            if let Some((code, strength)) = shadow_of_chain(world, assets, rotation, head) {
                out.push((cell as u16, code, strength));
            }
        }
    }
    out
}

fn level_shadows_match(
    cached_shadows: &[(u16, u8, i16)],
    world: &World,
    assets: &mut Assets,
    rotation: u8,
    lot_size: usize,
    level_index: usize,
) -> bool {
    let Some(grid) = world.arrays.get(&[3, 103][level_index]) else {
        return cached_shadows.is_empty();
    };
    let has_zero = world.objects.contains_key(&0);
    let mut cursor = 0usize;
    for x in 0..lot_size {
        for y in 0..lot_size {
            let cell = Lot::cell(x, y);
            let Some(word) = grid.data.get(cell * 2..cell * 2 + 2) else {
                continue;
            };
            let head = i16::from_le_bytes([word[0], word[1]]);
            if head < 0 || (head == 0 && !has_zero) {
                continue;
            }
            if let Some((code, strength)) = shadow_of_chain(world, assets, rotation, head) {
                if cached_shadows.get(cursor) != Some(&(cell as u16, code, strength)) {
                    return false;
                }
                cursor += 1;
            }
        }
    }
    cursor == cached_shadows.len()
}

fn objects_match(
    cached: &[ObjectVisual],
    world: &World,
    assets: &mut Assets,
    preview: Option<(u16, u16)>,
) -> bool {
    let mut cursor = 0usize;
    for (&id, o) in &world.objects {
        if !object_included(id, o, assets, preview) {
            continue;
        }
        let Some(prev) = cached.get(cursor) else {
            return false;
        };
        if !prev.matches(id, o) {
            return false;
        }
        cursor += 1;
    }
    cursor == cached.len()
}

fn collect_objects(
    world: &World,
    assets: &mut Assets,
    preview: Option<(u16, u16)>,
) -> Vec<ObjectVisual> {
    let mut out = Vec::with_capacity(world.objects.len());
    for (&id, o) in &world.objects {
        if object_included(id, o, assets, preview) {
            out.push(ObjectVisual::from_object(id, o));
        }
    }
    out
}

impl LotCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops all cached surface and grass data.
    pub fn clear(&mut self) {
        self.entry = None;
        self.env = None;
        self.grass = None;
        self.last_hit = CacheHit::Miss;
    }

    /// Which cache tier satisfied the most recent [`LotCache::draw_lot_with_preview`] call.
    pub fn last_hit(&self) -> CacheHit {
        self.last_hit
    }

    /// Draws `Pass::Lot` into `canvas`, returning the exact [`Stats`] of
    /// [`crate::draw_lot_with_preview`]. On a Tier-2 hit, restores the full
    /// cached surface via fast slice copies; on a Tier-1 hit (only object state
    /// changed), restores the cached level-1 environment and repaints only the
    /// objects pass (and level 2 if active) on top.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lot_with_preview(
        &mut self,
        world: &World,
        lot: &Lot,
        assets: &mut Assets,
        camera: &Camera,
        viewport: (u32, u32),
        preview: Option<(u16, u16)>,
        canvas: &mut Canvas,
    ) -> Stats {
        self.draw_lot_with_preview_and_options(
            world,
            lot,
            assets,
            camera,
            viewport,
            preview,
            RenderOptions::default(),
            canvas,
        )
    }

    /// Like [`LotCache::draw_lot_with_preview`], applying [`RenderOptions`]
    /// (`shadows` and `lighting`) and invalidating the cache when they change.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_lot_with_preview_and_options(
        &mut self,
        world: &World,
        lot: &Lot,
        assets: &mut Assets,
        camera: &Camera,
        viewport: (u32, u32),
        preview: Option<(u16, u16)>,
        options: RenderOptions,
        canvas: &mut Canvas,
    ) -> Stats {
        let assets_id = assets.instance_id();
        let width = canvas.width;
        let height = canvas.height;
        let clip_bottom = (viewport.1 as usize).min(height) as i32;
        let time_of_day = world.globals.get(4).copied().unwrap_or(lot.time_of_day);
        let house = world.globals.get(10).copied().unwrap_or(0);
        let show_grid = matches!(world.globals.get(19).copied().unwrap_or(0), 1 | 2);
        let grid_slice = show_grid
            .then(|| world.arrays.get(&8).map(|a| a.data.as_slice()))
            .flatten();
        let top = camera.level.clamp(1, 2) as usize;
        let env_walls = if top == 1 { camera.walls } else { Walls::Up };
        // Zero-area canvases can differ in rectangle recording between empty
        // and padded `color` buffers; bypass caching for zero-area or
        // undersized backing stores.
        let can_cache = width > 0 && height > 0 && canvas.has_pixel_storage();

        // Compute `holds_person` once (cheap: only climbs from the few Sims) so
        // both the cache check and any miss reuse it.
        let held = holds_person(world);

        // Track an already-computed cutaway mask so a miss never builds it twice.
        let mut computed_mask: Option<Mask> = None;

        // --- Tier 2 check: complete `Pass::Lot` surface ---
        if can_cache && let Some(entry) = &mut self.entry {
            let k = &entry.key;
            let base_match = k.assets_id == assets_id
                && k.width == width
                && k.height == height
                && k.clip_bottom == clip_bottom
                && k.camera.zoom == camera.zoom
                && k.camera.rotation == camera.rotation
                && k.camera.level == camera.level
                && k.camera.origin == camera.origin
                && k.camera.walls == camera.walls
                && k.preview == preview
                && k.options == options
                && k.time_of_day == time_of_day
                && k.house == house
                && k.show_grid == show_grid
                && (!show_grid || k.grid_flags.as_deref() == grid_slice)
                && k.held == held
                && k.lot.exact_eq(lot);

            if base_match {
                let mask_match = if camera.walls != Walls::Cutaway || k.camera.focus == camera.focus
                {
                    true
                } else {
                    let mask = Mask::new(lot, camera);
                    let same = k.cut_mask.as_ref() == Some(&mask);
                    computed_mask = Some(mask);
                    same
                };

                if mask_match && objects_match(&k.objects, world, assets, preview) {
                    let mut shadows_ok = true;
                    if options.shadows {
                        for level_index in 0..top {
                            if !level_shadows_match(
                                &k.shadows[level_index],
                                world,
                                assets,
                                camera.rotation,
                                lot.size,
                                level_index,
                            ) {
                                shadows_ok = false;
                                break;
                            }
                        }
                    }
                    if shadows_ok {
                        // Refresh cached focus when a focus change produced the
                        // identical cutaway mask so subsequent frames skip
                        // `Mask::new` too.
                        entry.key.camera.focus = camera.focus;
                        canvas.restore_pixels_from(&entry.canvas, background(time_of_day, house));
                        self.last_hit = CacheHit::Full;
                        return entry.stats;
                    }
                }
            }
        }

        // Compute level-1 wall-shaping apertures once so both Tier-1 validation
        // and any Tier-1 miss repaint share it without scanning objects twice.
        let wall_openings0 = can_cache.then(|| wall_objects_on_level(world, assets, 1));

        // --- Tier 1 check: level-1 static environment (before `objects(1)`) ---
        if can_cache
            && let Some(env) = &mut self.env
            && let Some(openings) = wall_openings0.as_ref()
        {
            let k = &env.key;
            let env_base_match = k.assets_id == assets_id
                && k.width == width
                && k.height == height
                && k.clip_bottom == clip_bottom
                && k.zoom == camera.zoom
                && k.rotation == camera.rotation
                && k.origin == camera.origin
                && k.env_walls == env_walls
                && k.options == options
                && k.time_of_day == time_of_day
                && k.house == house
                && k.show_grid == show_grid
                && (!show_grid || k.grid_flags.as_deref() == grid_slice)
                && k.lot.exact_eq(lot)
                && &k.wall_openings0 == openings;

            if env_base_match {
                let env_mask_match = if env_walls != Walls::Cutaway || k.focus == camera.focus {
                    true
                } else {
                    let mask = computed_mask.get_or_insert_with(|| Mask::new(lot, camera));
                    k.cut_mask.as_ref() == Some(&*mask)
                };

                if env_mask_match
                    && (!options.shadows
                        || level_shadows_match(
                            &k.shadows0,
                            world,
                            assets,
                            camera.rotation,
                            lot.size,
                            0,
                        ))
                {
                    if env_walls == Walls::Cutaway {
                        env.key.focus = camera.focus;
                    }
                    let cut_mask = if camera.walls == Walls::Cutaway {
                        if top == 1 {
                            env.key.cut_mask.clone()
                        } else {
                            Some(computed_mask.unwrap_or_else(|| Mask::new(lot, camera)))
                        }
                    } else {
                        None
                    };
                    canvas.restore_pixels_from(&env.canvas, background(time_of_day, house));
                    let stats = paint_lot_from_env(
                        world,
                        lot,
                        assets,
                        camera,
                        viewport,
                        preview,
                        options,
                        canvas,
                        held.clone(),
                        cut_mask.as_ref(),
                        env.stats,
                    );
                    let shadows = if options.shadows {
                        [
                            env.key.shadows0.clone(),
                            if top >= 2 {
                                collect_level_shadows(world, assets, camera.rotation, lot.size, 1)
                            } else {
                                Vec::new()
                            },
                        ]
                    } else {
                        [Vec::new(), Vec::new()]
                    };
                    let key = LotKey {
                        assets_id,
                        width,
                        height,
                        clip_bottom,
                        camera: *camera,
                        cut_mask,
                        preview,
                        options,
                        time_of_day,
                        house,
                        show_grid,
                        grid_flags: grid_slice.map(<[u8]>::to_vec),
                        lot: lot.clone(),
                        held,
                        shadows,
                        objects: collect_objects(world, assets, preview),
                    };
                    store_entry(&mut self.entry, key, canvas, stats);
                    self.last_hit = CacheHit::Environment;
                    return stats;
                }
            }
        }

        // --- Full cache miss: refresh grass tables only when grass inputs changed ---
        let grass_matches = self
            .grass
            .as_ref()
            .is_some_and(|g| g.key.matches(lot, camera));
        if !grass_matches {
            self.grass = Some(GrassEntry {
                key: GrassKey::from(lot, camera),
                grass: Grass::new(lot, camera),
            });
        }
        let grass = &self.grass.as_ref().expect("grass populated").grass;

        let cut_mask = if camera.walls == Walls::Cutaway {
            Some(computed_mask.unwrap_or_else(|| Mask::new(lot, camera)))
        } else {
            None
        };

        let mut env_canvas_slot = self.env.take().map(|e| e.canvas);
        let mut env_stats = Stats::default();
        let stats = paint_lot_with_env_hook(
            world,
            lot,
            assets,
            camera,
            viewport,
            preview,
            options,
            canvas,
            grass,
            held.clone(),
            cut_mask.as_ref(),
            wall_openings0.as_ref(),
            |env_src, stats_so_far| {
                if can_cache {
                    let target =
                        env_canvas_slot.get_or_insert_with(|| Canvas::new(0, 0, [0, 0, 0]));
                    target.snapshot_pixels_from(env_src);
                    env_stats = stats_so_far;
                }
            },
        );

        self.last_hit = CacheHit::Miss;

        if !can_cache {
            return stats;
        }

        let (shadows0, shadows1) = if options.shadows {
            (
                collect_level_shadows(world, assets, camera.rotation, lot.size, 0),
                if top >= 2 {
                    collect_level_shadows(world, assets, camera.rotation, lot.size, 1)
                } else {
                    Vec::new()
                },
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let grid_flags = grid_slice.map(<[u8]>::to_vec);

        if let Some(env_canvas) = env_canvas_slot {
            let env_key = EnvKey {
                assets_id,
                width,
                height,
                clip_bottom,
                zoom: camera.zoom,
                rotation: camera.rotation,
                origin: camera.origin,
                env_walls,
                focus: if env_walls == Walls::Cutaway {
                    camera.focus
                } else {
                    Focus::default()
                },
                cut_mask: if env_walls == Walls::Cutaway {
                    cut_mask.clone()
                } else {
                    None
                },
                options,
                time_of_day,
                house,
                show_grid,
                grid_flags: grid_flags.clone(),
                lot: lot.clone(),
                shadows0: shadows0.clone(),
                wall_openings0: wall_openings0.unwrap_or_default(),
            };
            self.env = Some(EnvEntry {
                key: env_key,
                canvas: env_canvas,
                stats: env_stats,
            });
        }

        let key = LotKey {
            assets_id,
            width,
            height,
            clip_bottom,
            camera: *camera,
            cut_mask,
            preview,
            options,
            time_of_day,
            house,
            show_grid,
            grid_flags,
            lot: lot.clone(),
            held,
            shadows: [shadows0, shadows1],
            objects: collect_objects(world, assets, preview),
        };
        store_entry(&mut self.entry, key, canvas, stats);
        stats
    }
}

fn store_entry(slot: &mut Option<Entry>, key: LotKey, canvas: &Canvas, stats: Stats) {
    match slot {
        Some(entry) => {
            entry.key = key;
            entry.canvas.snapshot_pixels_from(canvas);
            entry.stats = stats;
        }
        None => {
            let mut snapshot = Canvas::new(0, 0, [0, 0, 0]);
            snapshot.snapshot_pixels_from(canvas);
            *slot = Some(Entry {
                key,
                canvas: snapshot,
                stats,
            });
        }
    }
}
