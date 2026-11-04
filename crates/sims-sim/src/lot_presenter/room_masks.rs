//! Native468203 indoor-room wall visibility masks. Camera fields are explicit;
//! automatic camera lifecycle and the visible-room list remain separate owners.
use crate::{lot_geometry::Lot, lot_presenter::walls::cutaway::Mask};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Retained native view/projection fields; no demo camera defaults are supplied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Camera {
    pub origin: [i32; 2],
    pub zoom: u8,
    /// View+58; signed division by two is used for each room tile rectangle.
    pub tile_width: i32,
    /// View+5c and+60, used by4ac25d for nearby wall rectangles.
    pub half_width: i32,
    pub bottom: i32,
    /// View+130,+12c,+128,+124 for zoom0..3. The original viewer
    /// initializes zooms1..3; callers must supply the current zoom's height.
    /// A vector also retains older explicit contexts containing only0..2.
    pub floor_heights: Vec<i32>,
}
impl Camera {
    /// Projection fields after all three native lookup zooms initialize.
    /// 4c9edd publishes the heights into View+b8; 4a7b92 writes the widths.
    /// Origin remains a caller input. This does not construct depth buffers,
    /// initialize the complete viewer, or execute a zoom-change lifecycle.
    pub fn initialized(origin: [i32; 2], zoom: u8) -> Result<Self, String> {
        if !(1..=3).contains(&zoom) {
            return Err("original viewer projection requires zoom one through three".into());
        }
        Ok(Self {
            origin,
            zoom,
            tile_width: 16 << zoom,
            half_width: 8 << zoom,
            bottom: 8 << zoom,
            // Retain the actual contiguous fields read by native indexing.
            // Index zero aliases a neighboring lookup parameter (23), not
            // a fourth original zoom. This constructor never selects it.
            floor_heights: vec![23, 58, 116, 232],
        })
    }
}
pub type SplitRooms = BTreeMap<[i8; 3], [(u8, Option<u16>); 2]>;
///4b3c88 shared by room masks and the original diagonal renderer scan.
/// Callers validate the zoom/height storage before selecting it.
pub(crate) fn project(c: &Camera, p: [i8; 3]) -> [i32; 2] {
    let [x, y, z] = p.map(i32::from);
    [
        (x - y).wrapping_mul(8i32.wrapping_shl(u32::from(c.zoom))).wrapping_add(c.origin[0]),
        (x + y).wrapping_mul(4i32.wrapping_shl(u32::from(c.zoom))).wrapping_add(c.origin[1])
            .wrapping_sub(c.floor_heights[c.zoom as usize].wrapping_mul(z - 1)),
    ]
}
pub struct View<'a> {
    pub size: usize,
    pub rotation: u8,
    /// World+70,+74,+78,+7c: inclusive outer column/row bounds.
    pub bounds: [i32; 4],
    pub camera: &'a Camera,
    pub heights: &'a [[u8; 4]],
    pub walls: &'a [sims_formats::lot::WallTile],
    pub splits: &'a SplitRooms,
}
pub(crate) fn rotate([x, y, z]: [i8; 3], r: u8) -> [i8; 3] {
    //407704 tile table uses63, unlike wall-corner rotation's64.
    match r {
        1 => [63 - y, x, z],
        2 => [63 - x, 63 - y, z],
        3 => [y, 63 - x, z],
        _ => [x, y, z],
    }
}
fn add(mut p: [i8; 3], x: i8, y: i8) -> [i8; 3] {
    p[0] = p[0].wrapping_add(x);
    p[1] = p[1].wrapping_add(y);
    p
}
///479567: terrain corner bytes in display order, without projecting a point.
pub fn terrain_corners(size:usize,heights:&[[u8;4]],p:[i8;3],rotation:u8)->Result<[u8;4],String>{
    if !(1..=64).contains(&size)||heights.len()!=size*size||p[..2].iter().any(|&v|!(0..64).contains(&v))||rotation>3 {
        return Err("native room-mask point outside rotation table".into());
    }
    let [x,y,_]=rotate(p,4u8.wrapping_sub(rotation)&3);
    if x<0||y<0||x as usize>=size||y as usize>=size{return Err("native room-mask terrain/wall input unavailable".into());}
    let h=heights[x as usize*size+y as usize];
    Ok(std::array::from_fn(|n|h[(n+4-rotation as usize)&3]))
}
fn overlap(a: [i32; 4], b: [i32; 4]) -> bool {
    // Native signed OR of four wrapping differences, with strict boundaries.
    (a[3].wrapping_sub(b[1]).wrapping_sub(1)
        | b[3].wrapping_sub(a[1]).wrapping_sub(1)
        | b[2].wrapping_sub(a[0]).wrapping_sub(1)
        | a[2].wrapping_sub(b[0]).wrapping_sub(1))
        >= 0
}
impl View<'_> {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=64).contains(&self.size)
            || self.rotation > 3
            || self.camera.zoom > 3
            || self.camera.floor_heights.len() <= self.camera.zoom as usize
            || self.camera.floor_heights.len() > 4
            || self.heights.len() != self.size * self.size
            || self.walls.len() != self.size * self.size
            || self.bounds.iter().any(|&n| !(0..64).contains(&n))
            || self.bounds[0] > self.bounds[1]
            || self.bounds[2] > self.bounds[3]
        {
            return Err("invalid native room-mask view inputs".into());
        }
        Ok(())
    }
    fn cell(&self, p: [i8; 3]) -> Result<usize, String> {
        let [x, y, _] = p;
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return Err("native room-mask terrain/wall input unavailable".into());
        }
        Ok(x as usize * self.size + y as usize)
    }
    fn interior(&self, p: [i8; 3]) -> bool {
        let [x, y, _] = p.map(i32::from);
        x > self.bounds[0] && x < self.bounds[1] && y > self.bounds[2] && y < self.bounds[3]
    }
    /// Original479567: inverse tile rotation and ordered terrain corners.
    pub fn terrain(&self, p: [i8; 3]) -> Result<[u8; 4], String> {
        self.validate()?;
        terrain_corners(self.size,self.heights,p,self.rotation)
    }
    /// Original4b3c88 at the current projection zoom.
    pub(super) fn project(&self, p: [i8; 3]) -> [i32; 2] {
        project(self.camera, p)
    }
    /// The same-zoom, no-height-override branch of4a9c50 used by468203.
    /// Its short pair is stored vertical first, horizontal second.
    pub fn point(&self, p: [i8; 3]) -> Result<[i16; 2], String> {
        self.validate()?;
        let h = if self.interior(p) {
            self.terrain(p)?[1]
        } else {
            0
        };
        let [x, y] = self.project(p);
        Ok([
            y.wrapping_sub(i32::from(h) << (self.camera.zoom + 1)) as i16,
            x as i16,
        ])
    }
    /// Original4ac25d, including full-width coordinates and terrain adjustment.
    pub fn rectangle(&self, p: [i8; 3]) -> Result<[i32; 4], String> {
        let elevation = i32::from(self.terrain(p)?[1]) << (self.camera.zoom + 1);
        let [x, y] = self.project(p);
        let c = self.camera;
        Ok([
            x.wrapping_sub(c.half_width).wrapping_sub(1),
            y.wrapping_sub(c.floor_heights[c.zoom as usize])
                .wrapping_sub(elevation),
            x.wrapping_add(c.half_width).wrapping_sub(1),
            y.wrapping_add(c.bottom).wrapping_sub(elevation),
        ])
    }
    pub fn room(&self, id: u16, outdoors: bool, tiles: &[[i8; 3]]) -> Result<Mask, String> {
        self.validate()?;
        let mut rows = vec![0u64; 64];
        if outdoors {
            return Ok(Mask { first_row: 0, rows });
        }
        for &tile in tiles {
            let index = self.cell(tile)?;
            let mut p = rotate(tile, self.rotation);
            let [y, x] = self.point(p)?.map(i32::from);
            let half = self.camera.tile_width / 2;
            let mut rect = [
                x.wrapping_sub(half).wrapping_add(1),
                y.wrapping_add(1),
                x.wrapping_add(half).wrapping_sub(2),
                y.wrapping_add(half).wrapping_sub(2),
            ];
            //4686f7 queries inverse-rotated bit32, so odd views use bit16.
            let diagonal = if self.rotation & 1 == 0 { 32 } else { 16 };
            if self.walls[index].segments & diagonal != 0 {
                let sides = self
                    .splits
                    .get(&tile)
                    .ok_or("native room-mask split lookup unavailable")?;
                if let Some(&(kind, _)) = sides.iter().find(|(_, room)| *room == Some(id)) {
                    let kind = if kind == 0 {
                        0
                    } else {
                        (kind.wrapping_sub(self.rotation).wrapping_sub(1) & 3) + 1
                    };
                    match kind {
                        1 => rect[2] = rect[2].wrapping_sub(half),
                        3 => rect[0] = rect[0].wrapping_add(half),
                        _ => {}
                    }
                }
            }
            rows[p[1] as usize] |= 1u64 << p[0];
            p = add(p, 0, 1);
            // Each row of the native walk checks three neighboring columns.
            // Stop only when all three are outside, regardless of overlap.
            loop {
                let mut outside = 0;
                for step in [[0, 0], [1, -1], [0, 1]] {
                    p = add(p, step[0], step[1]);
                    if !self.interior(p) {
                        outside += 1;
                    } else {
                        let bit = 1u64 << p[0];
                        if rows[p[1] as usize] & bit == 0 && overlap(rect, self.rectangle(p)?) {
                            rows[p[1] as usize] |= bit;
                        }
                    }
                }
                if outside == 3 {
                    break;
                }
                p = add(p, 0, 1);
            }
        }
        Ok(Mask { first_row: 0, rows })
    }
}

/// Borrowed original house adapter. Room activity/refresh scheduling remains
/// with the caller; these masks do not alter simulation state or assets.
pub fn floor(
    lot: &Lot,
    level: i32,
    rotation: u8,
    bounds: [i32; 4],
    camera: &Camera,
) -> Result<BTreeMap<i16, Mask>, String> {
    let f = lot
        .floor(level)
        .ok_or("native room-mask floor unavailable")?;
    let splits = split_rooms(f, level)?;
    let view = View {
        size: lot.size,
        rotation,
        bounds,
        camera,
        heights: &lot.heights,
        walls: &f.source.walls,
        splits: &splits,
    };
    f.rooms
        .room_tiles
        .iter()
        .map(|(&id, tiles)| {
            let outdoors = f
                .metrics
                .get(&id)
                .ok_or("native room-mask outdoors cache unavailable")?
                .outdoors;
            let tiles: Vec<_> = tiles.iter().map(|p| p.map(|n| n as i8)).collect();
            Ok((id, view.room(id as u16, outdoors, &tiles)?))
        })
        .collect()
}

fn split_rooms(f: &crate::lot_geometry::Floor, level: i32) -> Result<SplitRooms, String> {
    let mut splits = BTreeMap::new();
    for s in &f.rooms.halves {
        let sides: Vec<_> = s
            .rooms
            .iter()
            .map(|(&k, &r)| (k, f.rooms.room_tiles.contains_key(&r).then_some(r as u16)))
            .collect();
        let sides: [(u8, Option<u16>); 2] = sides
            .try_into()
            .map_err(|_| "native room-mask split must have two sides")?;
        splits.insert([s.tile[0] as i8, s.tile[1] as i8, level as i8], sides);
    }
    Ok(splits)
}

/// 466f0c keeps the supplied room-directory order and skips inactive rooms.
pub fn refresh_ids(floor: i32, rooms: impl IntoIterator<Item = (u16, bool)>) -> Vec<u16> {
    rooms
        .into_iter()
        .filter_map(|(id, active)| {
            (active && (floor == 0 || i32::from((id >> 10) & 3) + 1 == floor)).then_some(id)
        })
        .collect()
}

/// Borrowed live-manager adapter for 466f0c/468203. Unvisited cached masks
/// remain intact, including inactive rooms and rooms on another floor.
pub fn refresh(
    lot: &Lot,
    manager: &crate::lot_environment::manager::Manager,
    floor: i32,
    rotation: u8,
    bounds: [i32; 4],
    camera: &Camera,
    cache: &mut BTreeMap<u16, super::visibility::Room>,
) -> Result<Vec<u16>, String> {
    let ids = refresh_ids(
        floor,
        manager.rooms.iter().map(|(&id, room)| (id, room.active)),
    );
    let mut splits = BTreeMap::new();
    for &id in &ids {
        let room = &manager.rooms[&id];
        let level = i32::from((id >> 10) & 3) + 1;
        let f = lot
            .floor(level)
            .ok_or("native room refresh floor unavailable")?;
        if let std::collections::btree_map::Entry::Vacant(e) = splits.entry(level) {
            e.insert(split_rooms(f, level)?);
        }
        let view = View {
            size: lot.size,
            rotation,
            bounds,
            camera,
            heights: &lot.heights,
            walls: &f.source.walls,
            splits: &splits[&level],
        };
        let tiles: Vec<_> = room.tiles.iter().map(|p| p.map(|n| n as i8)).collect();
        let mask = view.room(id, room.outdoors, &tiles)?;
        cache.insert(
            id,
            super::visibility::Room {
                outdoors: room.outdoors,
                mask,
            },
        );
    }
    Ok(ids)
}

impl View<'_> {
    /// 4a7518 computes a tile's dirty rectangle, then clamps both ends to the
    /// current viewport. The caller owns5cdf18 and its mutable region list.
    pub fn invalidation_rectangle(
        &self,
        p: [i8; 3],
        visible_level: i32,
        viewport: [i32; 4],
    ) -> Result<Option<[i32; 4]>, String> {
        if !self.interior(p) || i32::from(p[2]) > visible_level {
            return Ok(None);
        }
        self.validate()?;
        let h = self.terrain(p)?;
        let [x, y] = self.project(p);
        let c = self.camera;
        let elevation = i32::from(h[1]).wrapping_shl(u32::from(c.zoom) + 1);
        let y = y.wrapping_sub(elevation);
        let padding = 3i32.wrapping_shl(u32::from(c.zoom).wrapping_sub(1));
        let mut rect = [
            x.wrapping_sub(c.tile_width)
                .wrapping_sub(padding)
                .wrapping_sub(1),
            y.wrapping_sub(c.bottom)
                .wrapping_sub(c.floor_heights[c.zoom as usize]),
            x.wrapping_add(c.tile_width)
                .wrapping_add(padding)
                .wrapping_add(1),
            y.wrapping_add(c.bottom).wrapping_add(1),
        ];
        if h.iter().any(|v| *v != h[0]) {
            let next = [p[0].wrapping_add(1), p[1].wrapping_add(1), p[2]];
            let [x, y, _] = next.map(i32::from);
            if x >= self.bounds[0]
                && x <= self.bounds[1]
                && y >= self.bounds[2]
                && y <= self.bounds[3]
            {
                rect[3] = self.project(next)[1]
                    .wrapping_sub(i32::from(h[3]).wrapping_shl(u32::from(c.zoom) + 1));
            }
        }
        rect = clip_rectangle(rect, viewport);
        Ok((rect[0] < rect[2] && rect[1] < rect[3]).then_some(rect))
    }
}

/// Native4a76ed signed, ordered endpoint clamping, including inverted bounds.
pub fn clip_rectangle(mut rect:[i32;4],viewport:[i32;4])->[i32;4]{
        rect[0] = rect[0].max(viewport[0]).min(viewport[2]);
        rect[1] = rect[1].max(viewport[1]).min(viewport[3]);
        rect[2] = rect[2].min(viewport[2]).max(viewport[0]);
        rect[3] = rect[3].min(viewport[3]).max(viewport[1]);
        rect
}
