//! Original540619/53cfa7/53f6c1 construction in the unrotated house frame.
//! Heap addresses become indices; room objects become their original IDs.
use super::{Corner, Edge, Floor, Segment, State, grid};
use crate::{lot_geometry::Lot, lot_rooms::Partition, lot_walls::Grid};
use serde::{Deserialize, Serialize};
use sims_formats::lot::WallTile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub positions: Vec<[i8; 3]>,
    /// Native edge +28/+2c retain the last resolved room on each side.
    pub rooms: Vec<[Option<u16>; 2]>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    #[serde(default)]
    pub rotation: u8,
    pub geometry: Geometry,
    pub state: State,
}
fn key([x, y, _]: [i8; 3]) -> (i32, i32) {
    (i32::from(x) + i32::from(y), i32::from(x) - i32::from(y))
}
fn blank() -> Graph {
    Graph {
        rotation: 0,
        geometry: Geometry {
            positions: vec![],
            rooms: vec![],
        },
        state: State {
            corners: vec![],
            edges: vec![],
            floors: std::array::from_fn(|_| Floor {
                corners: vec![],
                edges: vec![],
            }),
        },
    }
}
impl Graph {
    /// Read-only adapter over imported geometry. This does not schedule view
    /// callbacks, mutate the simulation, or choose a cutaway mode.
    pub fn from_lot(lot: &Lot) -> Result<Self, String> {
        if lot.floors.len() > 2 {
            return Err("wall graph has more than two original floors".into());
        }
        let mut graph = blank();
        for (i, floor) in lot.floors.iter().enumerate() {
            let mut next = Self::floor(lot.size, i as i32 + 1, &floor.source.walls, &floor.rooms)?;
            let cs = graph.state.corners.len();
            let es = graph.state.edges.len();
            for c in &mut next.state.corners {
                for e in c.edges.iter_mut().flatten() {
                    *e += es;
                }
            }
            for e in &mut next.state.edges {
                for c in &mut e.ends {
                    *c += cs;
                }
            }
            graph.state.floors[i] = Floor {
                corners: (cs..cs + next.state.corners.len()).collect(),
                edges: (es..es + next.state.edges.len()).collect(),
            };
            graph.geometry.positions.extend(next.geometry.positions);
            graph.geometry.rooms.extend(next.geometry.rooms);
            graph.state.corners.extend(next.state.corners);
            graph.state.edges.extend(next.state.edges);
        }
        graph.state.validate()?;
        Ok(graph)
    }

    pub fn floor(
        size: usize,
        level: i32,
        walls: &[WallTile],
        rooms: &Partition,
    ) -> Result<Self, String> {
        if !(1..=2).contains(&level) || rooms.primary.len() != walls.len() {
            return Err("invalid wall graph floor or room grid".into());
        }
        let wall_grid = Grid::new(size, walls)?;
        let corners = grid::corners(size, walls)?;
        // 466fcc returns null for a room absent from the manager. The
        // partition's registration map supplies that identity boundary.
        let room_pointer = |id: i16| rooms.room_tiles.contains_key(&id).then_some(id as u16);
        let mut graph = blank();
        for y in 0..size {
            for x in 0..size {
                let mask = corners[x * size + y];
                let (junction, endpoint) = grid::classification(mask);
                if mask != 0 && (junction || endpoint) {
                    graph
                        .geometry
                        .positions
                        .push([x as i8, y as i8, level as i8]);
                }
            }
        }
        // Each corner position is unique, so comparator ties cannot occur.
        graph.geometry.positions.sort_by_key(|&p| key(p));
        for &[x, y, _] in &graph.geometry.positions {
            graph.state.corners.push(Corner {
                base: corners[x as usize * size + y as usize],
                masks: [0; 4],
                edges: [None; 4],
                valid: [0; 2],
                caps: [[0; 20]; 2],
            });
        }
        // Native leaves inactive descriptor storage uninitialized. Rust uses
        // zeroed storage; valid bytes remain the authority for reading caps.
        let index = |[x, y]: [i32; 2]| -> Result<usize, String> {
            if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
                return Err("native wall edge leaves the loaded grid".into());
            }
            Ok(x as usize * size + y as usize)
        };
        let mut sorted_edges: Vec<usize> = vec![];
        let mut starts = vec![];
        for id in 0..graph.state.corners.len() {
            let base = graph.state.corners[id].base;
            for (bits, slot, cardinal, diagonal) in [(0x44, 2, 1, 5), (0x88, 3, 3, 6)] {
                if base & bits == 0 {
                    continue;
                }
                let kind = if base & (bits & 15) != 0 {
                    cardinal
                } else {
                    diagonal
                };
                let (direction, bit, side, endpoint_bits, patterns) = match kind {
                    1 => ([0, 1], 1u8, 1, [4, 1], [0, 2]),
                    3 => ([1, 0], 2, 0, [8, 2], [1, 3]),
                    5 => ([1, 1], 32, 0, [64, 16], [6, 7]),
                    6 => ([1, -1], 16, 0, [128, 32], [5, 4]),
                    _ => unreachable!(),
                };
                let p = graph.geometry.positions[id];
                let mut corner = [i32::from(p[0]), i32::from(p[1])];
                let mut tile = corner;
                if kind == 6 {
                    tile[1] -= 1;
                }
                let neighbor = match kind {
                    1 => [-1, 0],
                    3 => [0, -1],
                    _ => [0, 0],
                };
                let mut segments = vec![];
                let mut sides = [None; 2];
                loop {
                    let first = index(tile)?;
                    let other = [tile[0] + neighbor[0], tile[1] + neighbor[1]];
                    let second = index(other)?;
                    let a = wall_grid.expanded(tile[0] as usize, tile[1] as usize)?;
                    let b = wall_grid.expanded(other[0] as usize, other[1] as usize)?;
                    let style = a.styles[bit.trailing_zeros() as usize]
                        .ok_or("wall edge owner style absent")?;
                    let mut wallpaper = [0; 2];
                    wallpaper[side] = a.patterns[patterns[0]];
                    wallpaper[1 - side] = b.patterns[patterns[1]];
                    segments.push(Segment {
                        flags: 0,
                        style: style as u8,
                        patterns: wallpaper,
                    });
                    let room = rooms.primary[first] as u16;
                    if room < 0xfffb {
                        sides[side] = room_pointer(room as i16);
                    } else if let Some(split) = rooms
                        .halves
                        .iter()
                        .find(|s| s.tile == [tile[0] as usize, tile[1] as usize])
                    {
                        if split.rooms.len() != 2 {
                            return Err("wall edge split room is incomplete".into());
                        }
                        let mut pair = split.rooms.iter();
                        let (&first_kind, &r1) = pair.next().unwrap();
                        let (_, &r2) = pair.next().unwrap();
                        let primary_first = if bit == 16 {
                            matches!(first_kind, 2 | 3)
                        } else {
                            matches!(first_kind, 1 | 2)
                        };
                        let (r1, r2) = if primary_first { (r1, r2) } else { (r2, r1) };
                        sides[side] = room_pointer(r1);
                        sides[1 - side] = room_pointer(r2);
                    }
                    let room = rooms.primary[second] as u16;
                    if room < 0xfffb {
                        sides[1 - side] = room_pointer(room as i16);
                    }
                    for n in 0..2 {
                        tile[n] += direction[n];
                        corner[n] += direction[n];
                    }
                    let mask = corners[index(corner)?];
                    let (junction, endpoint) = grid::classification(mask);
                    if mask == 0 || junction || endpoint {
                        break;
                    }
                    if segments.len() > size {
                        return Err("wall edge did not reach a corner".into());
                    }
                }
                // 53f351 converts the final tile back to the corner coordinate.
                // This is corner above, including the kind6 row adjustment.
                let end = [corner[0] as i8, corner[1] as i8, level as i8];
                let end_id = graph
                    .geometry
                    .positions
                    .partition_point(|&p| key(p) < key(end));
                if graph.geometry.positions.get(end_id) != Some(&end) {
                    return Err("native wall edge endpoint has no corner".into());
                }
                let edge = graph.state.edges.len();
                graph.state.corners[id].edges[slot] = Some(edge);
                graph.state.corners[end_id].edges[usize::from(matches!(kind, 3 | 6))] = Some(edge);
                graph.state.edges.push(Edge {
                    kind,
                    ends: [id, end_id],
                    bits: endpoint_bits,
                    segments,
                });
                graph.geometry.rooms.push(sides);
                starts.push(p);
                // 5409ab inserts at lower_bound, before equal start positions.
                let at = sorted_edges.partition_point(|&e| key(starts[e]) < key(p));
                sorted_edges.insert(at, edge);
            }
        }
        // 5404b5 sorts the vectors again even at rotation zero.
        sims_vm::presenter::sort_wall_records(&mut sorted_edges, &starts)?;
        let mut remap = vec![0; sorted_edges.len()];
        for (new, &old) in sorted_edges.iter().enumerate() {
            remap[old] = new;
        }
        let mut edges: Vec<_> = graph.state.edges.into_iter().map(Some).collect();
        let mut sides: Vec<_> = graph.geometry.rooms.into_iter().map(Some).collect();
        graph.state.edges = sorted_edges
            .iter()
            .map(|&e| edges[e].take().unwrap())
            .collect();
        graph.geometry.rooms = sorted_edges
            .iter()
            .map(|&e| sides[e].take().unwrap())
            .collect();
        for corner in &mut graph.state.corners {
            for e in corner.edges.iter_mut().flatten() {
                *e = remap[*e];
            }
        }
        graph.state.floors[level as usize - 1] = Floor {
            corners: (0..graph.state.corners.len()).collect(),
            edges: (0..graph.state.edges.len()).collect(),
        };
        graph.state.validate()?;
        Ok(graph)
    }
}
