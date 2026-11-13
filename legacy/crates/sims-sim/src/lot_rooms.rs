//! Room partitioning from original wall/floor cells (0x479d9a, first pass).
//! Exterior flags and structural support are separate native passes.
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Split {
    pub tile: [usize; 2],
    /// Native triangle types: 1 southwest, 2 southeast, 3 northeast, 4 northwest.
    pub rooms: BTreeMap<u8, i16>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// Column-major. Native -5 marks a tile whose diagonal halves differ.
    pub primary: Vec<i16>,
    pub halves: Vec<Split>,
    /// Native room registration vectors, including insertion order and repeated
    /// tiles. They feed room-area and exterior queries; do not deduplicate them.
    pub room_tiles: BTreeMap<i16, Vec<[usize; 3]>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    pub outdoors: bool,
    pub half_tiles: i32,
}

/// Geometry-only fields of native room rebuilding (0x466c3c/0x467bb1).
/// Object contributions, lighting and the environment score are separate.
pub fn metrics(
    partition: &Partition,
    walls: &[WallTile],
    flags: &[u8],
    size: usize,
) -> Result<BTreeMap<i16, Metrics>, String> {
    super::lot_walls::Grid::new(size, walls)?;
    if flags.len() != walls.len() || partition.primary.len() != walls.len() {
        return Err("invalid room metrics grids".into());
    }
    let mut result = BTreeMap::new();
    for (&room, tiles) in &partition.room_tiles {
        let mut cells = Vec::with_capacity(tiles.len());
        for &[x, y, _] in tiles {
            if x >= size || y >= size {
                return Err("room tile outside lot".into());
            }
            cells.push(x * size + y);
        }
        let outdoors = crate::spatial::room_outdoors(
            room,
            &cells
                .iter()
                .map(|&i| (partition.primary[i], flags[i]))
                .collect::<Vec<_>>(),
        );
        let half_tiles = cells
            .iter()
            .map(|&i| {
                if outdoors || walls[i].segments & 0x30 == 0 {
                    2
                } else {
                    1
                }
            })
            .sum();
        result.insert(
            room,
            Metrics {
                outdoors,
                half_tiles,
            },
        );
    }
    Ok(result)
}

/// Object membership, original 0x433bc5. This differs from route candidate
/// classification, which reads the primary tile directly. On a split tile the
/// original samples a horizontal neighbor, then a vertical neighbor if needed;
/// it does not directly look up the stored triangle's room ID.
pub fn object_room(
    size: usize,
    walls: &[WallTile],
    primary: &[i16],
    p: crate::Point,
) -> Result<i16, String> {
    object_room_in_active(size, size, walls, primary, p)
}

///433bc5 gates coarse coordinates with world+10, independently of array stride.
pub fn object_room_in_active(
    size: usize,
    active: usize,
    walls: &[WallTile],
    primary: &[i16],
    p: crate::Point,
) -> Result<i16, String> {
    super::lot_walls::Grid::new(size, walls)?;
    if primary.len() != walls.len() || !(2..=size).contains(&active) {
        return Err("invalid original room query grid".into());
    }
    // The native coarse coordinate record consists of signed bytes.
    let x = i32::from((p.x >> 4) as i8);
    let y = i32::from((p.y >> 4) as i8);
    let inside = |n: i32| n > 0 && n < active as i32 - 1;
    if !inside(x) || !inside(y) {
        return Ok(-5);
    }
    let cell = (x as usize) * size + y as usize;
    let mut room = primary[cell];
    if room != -5 {
        return Ok(room);
    }
    let (dx, dy) = if walls[cell].segments & 16 != 0 {
        if (p.x & 15) + (p.y & 15) < 16 {
            (-1, -1)
        } else {
            (1, 1)
        }
    } else if walls[cell].segments & 32 != 0 {
        if (p.x & 15) < (p.y & 15) {
            (-1, 1)
        } else {
            (1, -1)
        }
    } else {
        (0, 0)
    };
    // Failure of the first neighbor's bounds check does not try the second.
    if inside(x + dx) {
        room = primary[(x + dx) as usize * size + y as usize];
        if room == -5 && inside(y + dy) {
            room = primary[x as usize * size + (y + dy) as usize];
        }
    }
    Ok(room)
}

///479d9a first fills the complete allocated primary map with-1 (47c5c0),
///then partitions only world+10's active square, preserving native scan order.
pub fn partition_in_active(
    size: usize,
    active: usize,
    level: i32,
    walls: &[WallTile],
    floors: &[u16],
    ground_regions: Option<&[u8]>,
) -> Result<Partition, String> {
    super::lot_walls::Grid::new(size, walls)?;
    if !(2..=size).contains(&active)
        || floors.len() != walls.len()
        || ground_regions.is_some_and(|g| g.len() != walls.len())
    {
        return Err("invalid active room partition inputs".into());
    }
    if active == size {
        return partition(size, level, walls, floors, ground_regions);
    }
    let select = |i: usize| i / active * size + i % active;
    let cells = active * active;
    let compact_walls: Vec<_> = (0..cells).map(|i| walls[select(i)].clone()).collect();
    let compact_floors: Vec<_> = (0..cells).map(|i| floors[select(i)]).collect();
    let compact_regions: Option<Vec<_>> =
        ground_regions.map(|g| (0..cells).map(|i| g[select(i)]).collect());
    let mut result = partition(
        active,
        level,
        &compact_walls,
        &compact_floors,
        compact_regions.as_deref(),
    )?;
    let mut primary = vec![-1; walls.len()];
    for (i, value) in result.primary.into_iter().enumerate() {
        primary[select(i)] = value;
    }
    result.primary = primary;
    Ok(result)
}

/// Topology uses wall bits, independently of their collision/exterior styles.
/// `ground_regions` is the native pool map at world+0x60 (saved Arry 9): on
/// level one, crossing its <16 boundary separates regions.
pub fn partition(
    size: usize,
    level: i32,
    walls: &[WallTile],
    floors: &[u16],
    ground_regions: Option<&[u8]>,
) -> Result<Partition, String> {
    super::lot_walls::Grid::new(size, walls)?;
    if !(1..=2).contains(&level)
        || floors.len() != walls.len()
        || ground_regions.is_some_and(|g| g.len() != walls.len())
    {
        return Err("invalid original room partition inputs".into());
    }
    // Native transient states permit these two triangles. Crossing diagonal
    // segments in one cell needs additional investigation before activation.
    if walls.iter().any(|w| w.segments & 0x30 == 0x30) {
        return Err("crossing diagonal room segments are unsupported".into());
    }
    for x in 0..size {
        for y in 0..size {
            let bits = walls[x * size + y].segments;
            if (x > 0 && (bits & 1 != 0) != (walls[(x - 1) * size + y].segments & 4 != 0))
                || (y > 0 && (bits & 2 != 0) != (walls[x * size + y - 1].segments & 8 != 0))
            {
                return Err("asymmetric original wall topology is unsupported".into());
            }
        }
    }
    let kinds = |cell: usize| match walls[cell].segments & 0x30 {
        16 => [4, 2],
        32 => [3, 1],
        _ => [0, 0],
    };
    // Direction order in DAT_006e486c: north, south, west, east.
    let allows = |kind, d| match kind {
        1 => matches!(d, 1 | 2),
        2 => matches!(d, 1 | 3),
        3 => matches!(d, 0 | 3),
        4 => matches!(d, 0 | 2),
        _ => true,
    };
    let mut primary = vec![-1i16; walls.len()];
    // A split record retains first/second insertion order until both halves
    // merge. Negative primary values are native flood work states: -1 unseen,
    // -2/-3 one queued half, -4 queued whole tile, -5 registered split.
    let mut split: BTreeMap<usize, [(u8, i16); 2]> = BTreeMap::new();
    let mut room_tiles = BTreeMap::new();
    let tile = |cell: usize| [cell / size, cell % size, level as usize];
    let mut next = 0u16;
    for cell in 0..walls.len() {
        // Native scans each cell once. A second triangle is seeded only if
        // the first was visited by an earlier cell's flood; an isolated other
        // half of the current seed can remain -1 in the native split record.
        if !(primary[cell] >= -3 && primary[cell] < 0
            || primary[cell] == -5 && split[&cell][1].1 == -1)
        {
            continue;
        }
        if next >= 1024 {
            return Err("more than 1024 rooms on one floor is unsupported".into());
        }
        let room = (((level as u16 - 1) << 10) | next) as i16;
        next += 1;
        let mut stack = vec![cell];
        let mut tiles = vec![];
        let coverage = floors[cell] != 0;
        let region = ground_regions.is_some_and(|g| level == 1 && g[cell] < 16);
        while let Some(cell) = stack.pop() {
            if primary[cell] == room {
                continue;
            }
            let kind = match primary[cell] {
                -1 => {
                    let kind = kinds(cell)[0];
                    primary[cell] = match kind {
                        4 => -2,
                        3 => -3,
                        _ => -4,
                    };
                    stack.push(cell);
                    tiles.push(tile(cell));
                    kind
                }
                -4 => {
                    primary[cell] = room;
                    0
                }
                state @ (-2 | -3 | -5) => {
                    let kind = if state == -5 {
                        let second = split[&cell][1];
                        if second.1 == room {
                            continue;
                        }
                        if second.1 == -1 {
                            tiles.push(tile(cell));
                        }
                        second.0
                    } else if state == -2 {
                        if walls[cell].segments & 16 != 0 { 4 } else { 1 }
                    } else if walls[cell].segments & 16 != 0 {
                        2
                    } else {
                        3
                    };
                    if let Some(record) = split.get_mut(&cell) {
                        if record[0].1 == room {
                            split.remove(&cell);
                            primary[cell] = room;
                        } else {
                            record[1] = (kind, room);
                            primary[cell] = -5;
                        }
                    } else {
                        let opposite = match kind {
                            1 => 3,
                            2 => 4,
                            3 => 1,
                            4 => 2,
                            _ => unreachable!(),
                        };
                        split.insert(cell, [(kind, room), (opposite, -1)]);
                        primary[cell] = -5;
                    }
                    kind
                }
                _ => 0,
            };
            let x = cell / size;
            let y = cell % size;
            for (d, (dx, dy, opposite_wall)) in [(0, -1, 8), (0, 1, 2), (-1, 0, 4), (1, 0, 1)]
                .into_iter()
                .enumerate()
            {
                if !allows(kind, d) {
                    continue;
                }
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= size as i32 || ny >= size as i32 {
                    continue;
                }
                let target = nx as usize * size + ny as usize;
                if !(-5..=-1).contains(&primary[target])
                    || walls[target].segments & opposite_wall != 0
                    || (level != 1 && (floors[target] != 0) != coverage)
                    || ground_regions.is_some_and(|g| level == 1 && (g[target] < 16) != region)
                {
                    continue;
                }
                let target_kind = kinds(target)[usize::from(!allows(kinds(target)[0], d ^ 1))];
                let queued = match target_kind {
                    1 | 4 => -2,
                    2 | 3 => -3,
                    _ => -4,
                };
                let state = primary[target];
                let updated = if target_kind == 0 {
                    -4
                } else if state == -1 {
                    queued
                } else if matches!((state, queued), (-2, -3) | (-3, -2)) {
                    -4
                } else if state == -5
                    && split[&target]
                        .iter()
                        .any(|&(k, r)| k == target_kind && r == -1)
                {
                    -5
                } else {
                    continue;
                };
                primary[target] = updated;
                stack.push(target);
                tiles.push(tile(target));
            }
        }
        if tiles.is_empty() {
            next -= 1;
        } else {
            room_tiles.insert(room, tiles);
        }
    }
    let halves = split
        .into_iter()
        .map(|(cell, records)| Split {
            tile: [cell / size, cell % size],
            rooms: BTreeMap::from(records),
        })
        .collect();
    Ok(Partition {
        primary,
        halves,
        room_tiles,
    })
}
