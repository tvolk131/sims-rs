//! Static lot-surface cache for `Pass::Lot` ([`crate::draw_lot_with_preview`]).
//!
//! During normal gameplay most frames redraw the exact same static lot under
//! animating Sims, a spinning plumb bob, pointer hover, or an open pie menu.
//! [`LotCache`] caches the completed `Pass::Lot` [`Canvas`] (`color`, `depth`,
//! `ids`, `pen`, and `rects`) plus [`Stats`], and restores it via fast slice
//! copies whenever every visual input to `Pass::Lot` matches bit-for-bit.
//!
//! It also caches the computed [`Grass`] tables across camera pans (since
//! `Grass` depends on terrain/grass/level-1 walls & floors/time-of-day/rotation/
//! zoom, not `camera.origin`).

use crate::assets::Assets;
use crate::camera::{Camera, Walls, Zoom};
use crate::canvas::Canvas;
use crate::cutaway::Mask;
use crate::grass::Grass;
use crate::lot::{Lot, WallCell};
use crate::scene::{Stats, background, holds_person, paint_lot_precomputed, shadow_of_chain};
use sims_sim::{Object, World};
use std::collections::BTreeSet;

/// Caches the static `Pass::Lot` surface and per-zoom/rotation grass tables.
///
/// Surface entries are bound to the [`Assets`] instance so switching installs
/// or catalogs invalidates automatically; call [`LotCache::clear`] to drop
/// cached memory immediately.
#[derive(Default)]
pub struct LotCache {
    entry: Option<Entry>,
    grass: Option<GrassEntry>,
}

struct Entry {
    key: LotKey,
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

/// Complete visual key covering every input read by `paint(..., Pass::Lot)`.
struct LotKey {
    assets_id: u64,
    width: usize,
    height: usize,
    clip_bottom: i32,
    camera: Camera,
    cut_mask: Option<Mask>,
    preview: Option<(u16, u16)>,
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
        self.grass = None;
    }

    /// Draws `Pass::Lot` into `canvas`, returning the exact [`Stats`] of
    /// [`crate::draw_lot_with_preview`]. On a cache hit, restores the cached
    /// surface (`color`, `depth`, `ids`, `pen`, `rects`) via fast slice copies.
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
        let assets_id = assets.instance_id();
        let width = canvas.width;
        let height = canvas.height;
        let clip_bottom = (viewport.1 as usize).min(height) as i32;
        let time_of_day = world.globals.get(4).copied().unwrap_or(lot.time_of_day);
        let house = world.globals.get(10).copied().unwrap_or(0);
        let show_grid = matches!(world.globals.get(19).copied().unwrap_or(0), 1 | 2);
        let top = camera.level.clamp(1, 2) as usize;
        // Zero-area canvases can differ in rectangle recording between empty
        // and padded `color` buffers; bypass caching for zero-area or
        // undersized backing stores.
        let can_cache = width > 0 && height > 0 && canvas.has_pixel_storage();

        // Compute `holds_person` once (cheap: only climbs from the few Sims) so
        // both the cache check and any miss reuse it.
        let held = holds_person(world);

        // Track an already-computed cutaway mask so a miss never builds it twice.
        let mut computed_mask: Option<Mask> = None;

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
                && k.time_of_day == time_of_day
                && k.house == house
                && k.show_grid == show_grid
                && (!show_grid
                    || k.grid_flags.as_deref() == world.arrays.get(&8).map(|a| a.data.as_slice()))
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
                    if shadows_ok {
                        // Refresh cached focus when a focus change produced the
                        // identical cutaway mask so subsequent frames skip
                        // `Mask::new` too.
                        entry.key.camera.focus = camera.focus;
                        canvas.restore_pixels_from(&entry.canvas, background(time_of_day, house));
                        return entry.stats;
                    }
                }
            }
        }

        // Cache miss: refresh grass tables only when grass inputs changed.
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

        let stats = paint_lot_precomputed(
            world,
            lot,
            assets,
            camera,
            viewport,
            preview,
            canvas,
            grass,
            held.clone(),
            cut_mask.as_ref(),
        );

        if !can_cache {
            return stats;
        }

        let shadows = [
            collect_level_shadows(world, assets, camera.rotation, lot.size, 0),
            if top >= 2 {
                collect_level_shadows(world, assets, camera.rotation, lot.size, 1)
            } else {
                Vec::new()
            },
        ];
        let key = LotKey {
            assets_id,
            width,
            height,
            clip_bottom,
            camera: *camera,
            cut_mask,
            preview,
            time_of_day,
            house,
            show_grid,
            grid_flags: show_grid
                .then(|| world.arrays.get(&8).map(|a| a.data.clone()))
                .flatten(),
            lot: lot.clone(),
            held,
            shadows,
            objects: collect_objects(world, assets, preview),
        };

        match &mut self.entry {
            Some(entry) => {
                entry.key = key;
                entry.canvas.snapshot_pixels_from(canvas);
                entry.stats = stats;
            }
            None => {
                let mut snapshot = Canvas::new(0, 0, [0, 0, 0]);
                snapshot.snapshot_pixels_from(canvas);
                self.entry = Some(Entry {
                    key,
                    canvas: snapshot,
                    stats,
                });
            }
        }
        stats
    }
}
