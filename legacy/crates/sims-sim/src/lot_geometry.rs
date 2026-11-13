//! Saved native lot inputs and the shared geometry rebuild used by import and
//! simulation snapshot validation. All arrays here are column-major.
use crate::{lot_flags, lot_rooms, lot_walls};
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorSource {
    /// Zero is no covering, which is a hole upstairs but terrain downstairs.
    pub coverings: Vec<u16>,
    pub walls: Vec<WallTile>,
    pub object_heads: Vec<u16>,
    pub flags: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Floor {
    pub source: FloorSource,
    pub rooms: lot_rooms::Partition,
    pub flags: lot_flags::Rebuilt,
    pub metrics: BTreeMap<i16, lot_rooms::Metrics>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lot {
    pub size: usize,
    ///Explicit world+10 extent for fresh retained imports. Historical saves
    ///without this field retain their original full-stride derived geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_size: Option<usize>,
    pub heights: Vec<[u8; 4]>,
    pub pool: Vec<u8>,
    /// Arry 10/world+0x64. 255 is dry; all other values are water.
    pub water: Vec<u8>,
    pub floors: Vec<Floor>,
}
impl Lot {
    pub fn rebuild(
        size: usize,
        heights: Vec<[u8; 4]>,
        pool: Vec<u8>,
        water: Vec<u8>,
        sources: Vec<FloorSource>,
    ) -> Result<Self, String> {
        let mut lot = Self::rebuild_in_active(size, size, heights, pool, water, sources)?;
        lot.active_size = None;
        Ok(lot)
    }
    pub fn rebuild_in_active(
        size: usize,
        active: usize,
        heights: Vec<[u8; 4]>,
        pool: Vec<u8>,
        water: Vec<u8>,
        sources: Vec<FloorSource>,
    ) -> Result<Self, String> {
        if !(2..=64).contains(&size)
            || !(2..=size).contains(&active)
            || !(1..=2).contains(&sources.len())
            || heights.len() != size * size
            || pool.len() != size * size
            || water.len() != size * size
        {
            return Err("invalid imported lot dimensions or arrays".into());
        }
        let mut floors: Vec<Floor> = vec![];
        for (i, source) in sources.into_iter().enumerate() {
            let rooms = lot_rooms::partition_in_active(
                size,
                active,
                i as i32 + 1,
                &source.walls,
                &source.coverings,
                Some(&pool),
            )?;
            let flags = lot_flags::rebuild_in_active(
                lot_flags::Input {
                    size,
                    level: i as i32 + 1,
                    walls: &source.walls,
                    floors: &source.coverings,
                    object_heads: &source.object_heads,
                    heights: &heights,
                    saved_flags: &source.flags,
                    below: floors.last().map(|f| f.flags.flags.as_slice()),
                },
                active,
            )?;
            let metrics = lot_rooms::metrics(&rooms, &source.walls, &flags.flags, size)?;
            floors.push(Floor {
                source,
                rooms,
                flags,
                metrics,
            });
        }
        Ok(Self {
            size,
            active_size: Some(active),
            heights,
            pool,
            water,
            floors,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut rebuilt = Self::rebuild_in_active(
            self.size,
            self.active_extent(),
            self.heights.clone(),
            self.pool.clone(),
            self.water.clone(),
            self.floors.iter().map(|f| f.source.clone()).collect(),
        )?;
        rebuilt.active_size = self.active_size;
        if *self != rebuilt {
            return Err("imported lot derived geometry mismatch".into());
        }
        Ok(())
    }
    pub fn active_extent(&self) -> usize {
        self.active_size.unwrap_or(self.size)
    }
    pub fn floor(&self, level: i32) -> Option<&Floor> {
        level
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|i| self.floors.get(i))
    }
    pub fn wall_grid(&self, level: i32) -> Result<lot_walls::Grid<'_>, String> {
        lot_walls::Grid::new(
            self.size,
            &self
                .floor(level)
                .ok_or("imported floor not present")?
                .source
                .walls,
        )
    }
    pub fn index(&self, p: crate::Point) -> Option<usize> {
        (p.x >= 0 && p.y >= 0 && p.x < self.size as i32 * 16 && p.y < self.size as i32 * 16)
            .then(|| p.x as usize / 16 * self.size + p.y as usize / 16)
    }
    /// Prepare original routing geometry with an explicit world boundary.
    /// The host supplies actual tile iteration and resolved portal partners;
    /// no missing objects or room transitions are synthesized from wall gaps.
    pub fn route_obstacles(
        &self,
        room: i16,
        occupants: &[Vec<crate::lot_route::TileOccupant>],
        terrain: bool,
        ignore_marker: bool,
    ) -> Result<crate::lot_route::Prepared, String> {
        self.route_obstacles_in(room, occupants, terrain, ignore_marker, self.size)
    }

    ///4818af scans the retained active world while arrays keep their allocated
    ///stride. Room classification is retained from the complete backing lot.
    pub fn route_obstacles_in(
        &self,
        room: i16,
        occupants: &[Vec<crate::lot_route::TileOccupant>],
        terrain: bool,
        ignore_marker: bool,
        active_size: usize,
    ) -> Result<crate::lot_route::Prepared, String> {
        if !(3..=self.size).contains(&active_size) {
            return Err("route active extent exceeds backing arrays".into());
        }
        if occupants.len() != self.size * self.size {
            return Err("incomplete original tile occupants".into());
        }
        let level = i32::from((room as u16 & 0xc00) >> 10) + 1;
        let f = self.floor(level).ok_or("route room floor is absent")?;
        let metric = f.metrics.get(&room).ok_or("route room is absent")?;
        let available = (0..active_size * active_size)
            .map(|cell| {
                let i = cell / active_size * self.size + cell % active_size;
                crate::lot_route::tile_available(
                    level,
                    f.rooms.primary[i],
                    room,
                    self.water[i],
                    &occupants[i],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        fn select<T: Clone>(
            values: &[T],
            stride: usize,
            active: usize,
        ) -> std::borrow::Cow<'_, [T]> {
            if stride == active {
                std::borrow::Cow::Borrowed(values)
            } else {
                std::borrow::Cow::Owned(
                    (0..active * active)
                        .map(|i| values[i / active * stride + i % active].clone())
                        .collect(),
                )
            }
        }
        let primary = select(&f.rooms.primary, self.size, active_size);
        let walls = select(&f.source.walls, self.size, active_size);
        let coverings = select(&f.source.coverings, self.size, active_size);
        let flags = select(&f.flags.flags, self.size, active_size);
        crate::lot_route::prepare(crate::lot_route::Input {
            size: active_size,
            level,
            room,
            primary: &primary,
            walls: &walls,
            coverings: &coverings,
            flags: &flags,
            available: &available,
            outside: metric.outdoors,
            terrain,
            ignore_marker,
        })
    }
}
