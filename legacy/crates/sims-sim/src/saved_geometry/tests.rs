use super::*;
use crate::saved_matrix::{
    self, Allocated, Allocation, AllocationRequest, Backing, Lookup, Matrix,
};
use crate::surface_remap::{ExpandedWall, Groups, Kind};
use serde_json::{Value, json};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Services {
    statuses: [i32; 3],
    block: Option<usize>,
    calls: Vec<String>,
    leases: Vec<u64>,
    next: u64,
    tables: Vec<u64>,
    temporaries: Vec<u64>,
    dimension: usize,
    sample: bool,
}
impl Services {
    fn enter(&mut self, index: usize) -> Result<(), Stop> {
        let name = ["arrays", "floors", "walls"][index];
        if self.calls.last().map(String::as_str) != Some(name) {
            self.calls.push(name.into());
        }
        if self.block == Some(index) {
            return Err(Stop::Unavailable(format!("pending {name}")));
        }
        Ok(())
    }
}
impl saved_matrix::Host for Services {
    fn lookup(&mut self, _tag: u32, id: u32, _callback: u32) -> Result<Lookup, Stop> {
        if id == 0 {
            self.enter(0)?;
        }
        if self.statuses[0] != 0 {
            return Ok(Lookup::Missing {
                native_status: self.statuses[0],
            });
        }
        let lease = u64::from(id) + 1;
        self.leases.push(lease);
        Ok(Lookup::Found { lease })
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        let id = (lease - 1) as u32;
        let (stride, dimension) = match id % 100 {
            0 => (4, self.dimension),
            1 => (1, 64),
            2 => (8, 64),
            3 | 11 => (2, self.dimension),
            12 => (14, self.dimension),
            _ => (1, self.dimension),
        };
        let mut bytes = Vec::new();
        for n in [stride, dimension, dimension, 0] {
            bytes.extend_from_slice(&(n as i16).to_le_bytes());
        }
        bytes.resize(8 + dimension * 4 + dimension * dimension * stride, 0);
        if self.sample {
            let base = 8 + dimension * 4;
            if id == 1 {
                bytes[base] = 255;
            }
            if id == 2 {
                bytes[base..base + 8].copy_from_slice(&[16, 0, 30, 1, 31, 0, 0, 0]);
                let other = base + dimension * stride;
                bytes[other..other + 8].copy_from_slice(&[1, 0, 1, 0, 42, 0, 0, 0]);
            }
        }
        Ok(bytes)
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        let at = self.leases.iter().position(|l| *l == lease).unwrap();
        self.leases.remove(at);
        Ok(())
    }
    fn free(&mut self, _token: u64) -> Result<(), Stop> {
        panic!("same-size source must not free")
    }
    fn allocate(&mut self, _request: AllocationRequest) -> Result<Allocated, Stop> {
        panic!("same-size source must not allocate")
    }
}
impl saved_world::Host for Services {
    fn construct_temporary(
        &mut self,
        kind: saved_world::Legacy,
    ) -> Result<saved_world::Temporary, Stop> {
        self.next += 1;
        let owner = self.next;
        self.temporaries.push(owner);
        self.next += 1;
        let stride = if kind == saved_world::Legacy::Floor {
            1
        } else {
            8
        };
        Ok(saved_world::Temporary {
            owner,
            matrix: Matrix {
                width: 64,
                height: 64,
                stride,
                allocation: Allocation::Known(Backing {
                    token: self.next,
                    column_offsets: (0..64).map(|x| x * 64 * stride as u32).collect(),
                    data: vec![0xa5; 64 * 64 * stride as usize],
                }),
            },
        })
    }
    fn destroy_temporary(&mut self, temporary: &saved_world::Temporary) -> Result<(), Stop> {
        let at = self
            .temporaries
            .iter()
            .position(|o| *o == temporary.owner)
            .unwrap();
        self.temporaries.remove(at);
        Ok(())
    }
    fn name_arrays(&mut self) -> Result<(), Stop> {
        Ok(())
    }
}
impl surface_remap::Host for Services {
    fn free_vector(&mut self, _owner: u64, _bytes: usize) -> Result<(), Stop> {
        panic!("no authored old mapping storage")
    }
    fn read_map(&mut self, kind: Kind, _id: u32, _target: &mut Groups) -> Result<i32, Stop> {
        let index = if kind == Kind::Floor { 1 } else { 2 };
        self.enter(index)?;
        Ok(self.statuses[index])
    }
    fn normalize_legacy(&mut self, source: &Groups, target: &mut Groups) -> Result<(), Stop> {
        assert!(source.values.is_empty() && target.values.is_empty());
        Ok(())
    }
    fn allocate_table(&mut self, _bytes: usize) -> Result<u64, Stop> {
        self.next += 1;
        self.tables.push(self.next);
        Ok(self.next)
    }
    fn free_table(&mut self, owner: u64) -> Result<(), Stop> {
        let at = self.tables.iter().position(|o| *o == owner).unwrap();
        self.tables.remove(at);
        Ok(())
    }
    fn match_file(&mut self, _name: &[u8], _file: u64) -> Result<bool, Stop> {
        panic!("empty groups do not consume catalog")
    }
    fn resolve_direct(&mut self, _kind: Kind, _index: usize, _local: u16) -> Result<u16, Stop> {
        panic!("empty groups do not consume catalog")
    }
    fn resolve_nearest(&mut self, _kind: Kind, _key: u32) -> Result<u16, Stop> {
        panic!("empty groups do not consume catalog")
    }
    fn get_wall(&mut self, _coord: surface_remap::Coord) -> Result<ExpandedWall, Stop> {
        panic!("active0 does not consume wall service")
    }
    fn put_wall(&mut self, _coord: surface_remap::Coord, _wall: &ExpandedWall) -> Result<(), Stop> {
        panic!("active0 does not consume wall service")
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Case {
    state: State,
    exec: Execution,
    host: Services,
}
fn setup(statuses: [i32; 3], block: Option<usize>) -> Case {
    let mut next = 0;
    let mut matrix = |stride| {
        next += 1;
        Matrix {
            width: 2,
            height: 2,
            stride,
            allocation: Allocation::Known(Backing {
                token: next,
                column_offsets: vec![0, 2 * stride as u32],
                data: vec![0xa5; 4 * stride as usize],
            }),
        }
    };
    let mut floor = || saved_world::Floor {
        covering: matrix(2),
        wall: matrix(14),
        head: matrix(2),
        flags: matrix(1),
        primary: matrix(2),
        lights: saved_world::Lights {
            used: 0,
            capacity: 0,
            values: vec![],
        },
    };
    let floors = [floor(), floor()];
    let arrays = saved_world::State {
        active_size: 0,
        altitude: matrix(4),
        ground: matrix(1),
        grass: matrix(1),
        target_grass: matrix(1),
        pool: matrix(1),
        water: matrix(1),
        floors,
    };
    let vertices = [matrix(1), matrix(1)];
    let mapping = || surface_remap::State {
        groups: Groups::default(),
        runtime_files: vec![],
    };
    Case {
        state: State {
            arrays,
            vertices,
            floors: mapping(),
            walls: mapping(),
            scratch: Scratch {
                requested_converter: 0,
            },
        },
        exec: Execution::new(62).unwrap(),
        host: Services {
            statuses,
            block,
            calls: vec![],
            leases: vec![],
            next: 100,
            tables: vec![],
            temporaries: vec![],
            dimension: 2,
            sample: false,
        },
    }
}
fn run(case: &mut Case) -> Result<[i32; 3], Stop> {
    for _ in 0..1000 {
        if let Some(statuses) = case.exec.step(&mut case.state, &mut case.host)? {
            return Ok(statuses);
        }
    }
    panic!("geometry caller did not complete")
}
#[cfg(feature = "private-fixtures")]
#[test]
fn actual_caller_known_statuses_advance_and_unreturned_services_remain_pending() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/saved-geometry-caller-native.json"
    ))
    .unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let statuses: [i32; 3] = serde_json::from_value(row["statuses"].clone()).unwrap();
        let block = row["unreturned_service"].as_u64().map(|n| n as usize);
        let mut case = setup(statuses, block);
        let result = run(&mut case);
        let native: Vec<_> = row["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| {
                let s = c["phase"].as_str().unwrap();
                (s != "types").then_some(s)
            })
            .collect();
        assert_eq!(case.host.calls, native);
        assert_eq!(
            case.exec.ready_for_types(),
            row["reached_types"].as_bool().unwrap()
        );
        if let Some(block) = block {
            assert!(matches!(result, Err(Stop::Unavailable(_))));
            assert_eq!(case.exec.statuses()[block], None);
            let raw = serde_json::to_vec(&case).unwrap();
            let mut cold: Case = serde_json::from_slice(&raw).unwrap();
            assert_eq!(case, cold);
            cold.host.block = None;
            assert_eq!(run(&mut cold).unwrap(), statuses);
            let mut warm = setup(statuses, None);
            run(&mut warm).unwrap();
            assert_eq!(cold, warm);
        } else {
            assert_eq!(result.unwrap(), statuses);
            assert_eq!(case.exec.statuses(), statuses.map(Some));
            assert!(
                case.host.leases.is_empty()
                    && case.host.tables.is_empty()
                    && case.host.temporaries.is_empty()
            );
            let before = case.clone();
            assert_eq!(
                case.exec.step(&mut case.state, &mut case.host).unwrap(),
                Some(statuses)
            );
            assert_eq!(case, before);
        }
    }
}
#[test]
fn completed_stages_cold_restore_without_resetting_prior_array_or_mapping_state() {
    for stage in [Phase::Floors, Phase::Walls, Phase::Types] {
        let mut case = setup([0, 0, 0], None);
        for _ in 0..1000 {
            if case.exec.phase == stage {
                break;
            }
            case.exec.step(&mut case.state, &mut case.host).unwrap();
        }
        assert_eq!(case.exec.phase, stage);
        let raw = serde_json::to_vec(&case).unwrap();
        let mut cold: Case = serde_json::from_slice(&raw).unwrap();
        assert_eq!(case, cold);
        run(&mut case).unwrap();
        run(&mut cold).unwrap();
        assert_eq!(case, cold);
        assert_eq!(json!(case.host.calls), json!(["arrays", "floors", "walls"]));
    }
}

#[test]
fn native_wall_services_mutate_the_same_read_owner_and_distinct_vertices_across_stage_restore() {
    let mut case = setup([0, 0, 0], None);
    while case.exec.phase == Phase::Arrays {
        case.exec.step(&mut case.state, &mut case.host).unwrap();
    }
    fn backing(stride: i32, token: u64, fill: u8) -> Matrix {
        let offsets = (0..4)
            .map(|x| x * (4 * stride as u32 + 3))
            .collect::<Vec<_>>();
        let length = offsets[3] as usize + 4 * stride as usize;
        Matrix {
            width: 4,
            height: 4,
            stride,
            allocation: Allocation::Known(Backing {
                token,
                column_offsets: offsets,
                data: vec![fill; length],
            }),
        }
    }
    case.state.arrays.active_size = 2;
    for i in 0..2 {
        case.state.arrays.floors[i].covering = backing(2, 1000 + i as u64, 0);
        case.state.arrays.floors[i].wall = backing(14, 2000 + i as u64, 0);
        case.state.vertices[i] = backing(1, 3000 + i as u64, 0xa5);
    }
    saved_world::cell_mut(&mut case.state.arrays.floors[0].covering, 0, 0, 2)
        .unwrap()
        .copy_from_slice(&255u16.to_le_bytes());
    let wall = saved_world::cell_mut(&mut case.state.arrays.floors[0].wall, 0, 0, 14).unwrap();
    wall[0] = 0x10;
    wall[2..4].copy_from_slice(&30u16.to_le_bytes());
    wall[4..6].copy_from_slice(&1u16.to_le_bytes());
    wall[6..8].copy_from_slice(&31u16.to_le_bytes());
    let wall = saved_world::cell_mut(&mut case.state.arrays.floors[0].wall, 1, 0, 14).unwrap();
    wall[0] = 1;
    wall[2..4].copy_from_slice(&1u16.to_le_bytes());
    wall[6..8].copy_from_slice(&42u16.to_le_bytes());
    let primaries = case
        .state
        .arrays
        .floors
        .iter()
        .map(|f| f.primary.clone())
        .collect::<Vec<_>>();
    let saved_flags = case
        .state
        .arrays
        .floors
        .iter()
        .map(|f| f.flags.clone())
        .collect::<Vec<_>>();
    while case.exec.phase == Phase::Floors {
        case.exec.step(&mut case.state, &mut case.host).unwrap();
    }
    let wall = saved_world::cell(&case.state.arrays.floors[0].wall, 0, 0, 14).unwrap();
    assert_eq!(&wall[2..4], &65535u16.to_le_bytes());
    assert_eq!(&wall[6..8], &65535u16.to_le_bytes());
    let raw = serde_json::to_vec(&case).unwrap();
    let mut cold: Case = serde_json::from_slice(&raw).unwrap();
    assert_eq!(case, cold);
    for branch in [&mut case, &mut cold] {
        for _ in 0..1000 {
            if branch
                .exec
                .step(&mut branch.state, &mut branch.host)
                .unwrap()
                .is_some()
            {
                break;
            }
        }
        assert!(branch.exec.ready_for_types());
    }
    assert_eq!(case, cold);
    let wall = saved_world::cell(&case.state.arrays.floors[0].wall, 1, 0, 14).unwrap();
    assert_eq!(&wall[6..8], &65535u16.to_le_bytes());
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1), (2, 0), (2, 1)] {
        assert_eq!(
            saved_world::cell(&case.state.vertices[0], x, y, 1).unwrap(),
            [0]
        );
    }
    assert_eq!(
        saved_world::cell(&case.state.vertices[0], 3, 3, 1).unwrap(),
        [0xa5]
    );
    assert_eq!(
        saved_world::cell(&case.state.vertices[1], 0, 0, 1).unwrap(),
        [0xa5]
    );
    assert_eq!(
        case.state
            .arrays
            .floors
            .iter()
            .map(|f| f.primary.clone())
            .collect::<Vec<_>>(),
        primaries
    );
    assert_eq!(
        case.state
            .arrays
            .floors
            .iter()
            .map(|f| f.flags.clone())
            .collect::<Vec<_>>(),
        saved_flags
    );
}

#[test]
fn missing_consumed_vertex_owner_stays_pending_without_falling_back_to_saved_flags() {
    let mut case = setup([0, 0, 0], None);
    while case.exec.phase == Phase::Arrays {
        case.exec.step(&mut case.state, &mut case.host).unwrap();
    }
    case.state.arrays.active_size = 1;
    saved_world::cell_mut(&mut case.state.arrays.floors[0].covering, 0, 0, 2)
        .unwrap()
        .copy_from_slice(&255u16.to_le_bytes());
    let wall = saved_world::cell_mut(&mut case.state.arrays.floors[0].wall, 0, 0, 14).unwrap();
    wall.fill(0);
    wall[0] = 16;
    wall[2] = 30;
    wall[4] = 1;
    wall[6] = 31;
    // Clear the authored retained active cell: active0 legacy reads above
    // deliberately did not replace its old A5 bytes.
    saved_world::cell_mut(&mut case.state.arrays.floors[1].wall, 0, 0, 14)
        .unwrap()
        .fill(0);
    // Level2 needs no put, so its unavailable derived backing must remain unused.
    let vertices = case.state.vertices[0].clone();
    case.state.vertices[0].allocation = Allocation::Unavailable;
    case.state.vertices[1].allocation = Allocation::Unavailable;
    let raw_wall = case.state.arrays.floors[0].wall.clone();
    let flags = case.state.arrays.floors[0].flags.clone();
    let mut stopped = false;
    for _ in 0..1000 {
        match case.exec.step(&mut case.state, &mut case.host) {
            Err(Stop::Unavailable(_)) => {
                stopped = true;
                break;
            }
            Ok(None) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(stopped);
    assert_eq!(case.exec.phase, Phase::Floors);
    assert_eq!(case.exec.statuses(), [Some(0), None, None]);
    assert_eq!(case.state.arrays.floors[0].wall, raw_wall);
    assert_eq!(case.state.arrays.floors[0].flags, flags);
    let raw = serde_json::to_vec(&case).unwrap();
    let mut cold: Case = serde_json::from_slice(&raw).unwrap();
    assert_eq!(case, cold);
    cold.state.vertices[0] = vertices;
    for _ in 0..1000 {
        if cold
            .exec
            .step(&mut cold.state, &mut cold.host)
            .unwrap()
            .is_some()
        {
            break;
        }
    }
    assert!(cold.exec.ready_for_types());
    assert_eq!(cold.state.arrays.floors[0].flags, flags);
    assert!(matches!(
        cold.state.vertices[1].allocation,
        Allocation::Unavailable
    ));
    assert_eq!(
        &saved_world::cell(&cold.state.arrays.floors[0].wall, 0, 0, 14).unwrap()[2..4],
        &65535u16.to_le_bytes()
    );
}

#[test]
fn serialized_array_sources_flow_through_remaps_and_native_wall_owners_without_replacement() {
    let mut case = setup([0, 0, 0], None);
    let mut token = 5000;
    let mut matrix = |stride| {
        token += 1;
        Matrix {
            width: 4,
            height: 4,
            stride,
            allocation: Allocation::Known(Backing {
                token,
                column_offsets: (0..4).map(|x| x * 4 * stride as u32).collect(),
                data: vec![0xa5; 16 * stride as usize],
            }),
        }
    };
    case.state.arrays.active_size = 2;
    for floor in &mut case.state.arrays.floors {
        floor.covering = matrix(2);
        floor.wall = matrix(14);
        floor.head = matrix(2);
        floor.flags = matrix(1);
        floor.primary = matrix(2);
    }
    case.state.arrays.altitude = matrix(4);
    case.state.arrays.ground = matrix(1);
    case.state.arrays.grass = matrix(1);
    case.state.arrays.target_grass = matrix(1);
    case.state.arrays.pool = matrix(1);
    case.state.arrays.water = matrix(1);
    case.state.vertices = [matrix(1), matrix(1)];
    case.host.dimension = 4;
    case.host.sample = true;
    let original_owners = case
        .state
        .arrays
        .floors
        .iter()
        .map(|f| {
            let Allocation::Known(b) = &f.wall.allocation else {
                panic!()
            };
            b.token
        })
        .collect::<Vec<_>>();
    let primary = case.state.arrays.floors[0].primary.clone();
    while case.exec.phase == Phase::Arrays {
        case.exec.step(&mut case.state, &mut case.host).unwrap();
    }
    assert_eq!(
        saved_world::cell(&case.state.arrays.floors[0].covering, 0, 0, 2).unwrap(),
        255u16.to_le_bytes()
    );
    let wall = saved_world::cell(&case.state.arrays.floors[0].wall, 0, 0, 14).unwrap();
    assert_eq!(wall[0], 16);
    assert_eq!(&wall[2..4], 30u16.to_le_bytes());
    assert_eq!(&wall[6..8], 31u16.to_le_bytes());
    assert_eq!(
        saved_world::cell(&case.state.arrays.floors[0].wall, 3, 3, 14).unwrap(),
        [0xa5; 14]
    );
    let raw = serde_json::to_vec(&case).unwrap();
    let mut cold: Case = serde_json::from_slice(&raw).unwrap();
    assert_eq!(case, cold);
    for branch in [&mut case, &mut cold] {
        for _ in 0..1000 {
            if branch
                .exec
                .step(&mut branch.state, &mut branch.host)
                .unwrap()
                .is_some()
            {
                break;
            }
        }
        assert!(branch.exec.ready_for_types());
    }
    assert_eq!(case, cold);
    for (x, y) in [(0, 0), (1, 0)] {
        assert_eq!(
            &saved_world::cell(&case.state.arrays.floors[0].wall, x, y, 14).unwrap()[6..8],
            65535u16.to_le_bytes()
        );
    }
    assert_eq!(case.state.arrays.floors[0].primary, primary);
    assert_eq!(
        case.state
            .arrays
            .floors
            .iter()
            .map(|f| {
                let Allocation::Known(b) = &f.wall.allocation else {
                    panic!()
                };
                b.token
            })
            .collect::<Vec<_>>(),
        original_owners
    );
    assert_eq!(
        saved_world::cell(&case.state.vertices[0], 2, 1, 1).unwrap(),
        [0]
    );
    assert_eq!(
        saved_world::cell(&case.state.vertices[1], 0, 0, 1).unwrap(),
        [0xa5]
    );
    assert!(
        case.host.leases.is_empty()
            && case.host.tables.is_empty()
            && case.host.temporaries.is_empty()
    );
}
