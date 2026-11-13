use super::*;
use crate::saved_matrix::{Allocated, AllocationRequest, Backing, Lookup};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn observed(v: &Value, token: u64) -> Matrix {
    Matrix {
        width: v["width"].as_i64().unwrap() as i32,
        height: v["height"].as_i64().unwrap() as i32,
        stride: v["stride"].as_i64().unwrap() as i32,
        allocation: Allocation::Known(Backing {
            token,
            column_offsets: serde_json::from_value(v["columns_relative_to_data"].clone()).unwrap(),
            data: unhex(v["data_hex"].as_str().unwrap()),
        }),
    }
}
fn view(m: &Matrix) -> Value {
    let (columns, data) = match &m.allocation {
        Allocation::Known(b) => (json!(b.column_offsets), json!(hex(&b.data))),
        Allocation::Null => (Value::Null, Value::Null),
        other => panic!("unexpected {other:?}"),
    };
    json!({"width":m.width,"height":m.height,"stride":m.stride,"columns_relative_to_data":columns,"data_hex":data})
}
fn names(state: &State) -> BTreeMap<&'static str, &Matrix> {
    [
        ("altitude", &state.altitude),
        ("ground", &state.ground),
        ("grass", &state.grass),
        ("target_grass", &state.target_grass),
        ("pool", &state.pool),
        ("water", &state.water),
        ("floor1", &state.floors[0].covering),
        ("floor2", &state.floors[1].covering),
        ("walls1", &state.floors[0].wall),
        ("walls2", &state.floors[1].wall),
        ("heads1", &state.floors[0].head),
        ("heads2", &state.floors[1].head),
        ("flags1", &state.floors[0].flags),
        ("flags2", &state.floors[1].flags),
        ("primary1", &state.floors[0].primary),
        ("primary2", &state.floors[1].primary),
    ]
    .into()
}
fn setup(row: &Value) -> State {
    let before = &row["before"];
    let mut token = 0;
    let mut matrix = |name: &str| {
        token += 1;
        observed(&before[name], token)
    };
    let mut floors = vec![];
    for level in 1..=2 {
        let l = &row["lights"][level - 1];
        let words: Vec<u32> = serde_json::from_value(l["input"].clone()).unwrap();
        floors.push(Floor {
            covering: matrix(&format!("floor{level}")),
            wall: matrix(&format!("walls{level}")),
            head: matrix(&format!("heads{level}")),
            flags: matrix(&format!("flags{level}")),
            primary: matrix(&format!("primary{level}")),
            lights: Lights {
                used: l["used"].as_u64().unwrap() as usize,
                capacity: l["capacity"].as_u64().unwrap() as usize,
                values: words.into_iter().map(u32::to_le_bytes).collect(),
            },
        });
    }
    State {
        active_size: row["input"]["active"].as_i64().unwrap_or(2) as i32,
        altitude: matrix("altitude"),
        ground: matrix("ground"),
        grass: matrix("grass"),
        target_grass: matrix("target_grass"),
        pool: matrix("pool"),
        water: matrix("water"),
        floors: floors.try_into().unwrap(),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Block {
    Lookup,
    Source,
    Construct,
    Destroy,
    Names,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    statuses: BTreeMap<u32, i32>,
    parsed_failure: Option<u32>,
    ordinal: u64,
    current_temp: Option<String>,
    events: Vec<Value>,
    leases: BTreeSet<u64>,
    live: BTreeMap<u64, String>,
    block: Option<Block>,
}
fn destination(id: u32, current: &Option<String>) -> String {
    match id {
        1 | 2 | 101 | 102 => current.clone().unwrap(),
        0 => "altitude".into(),
        4 => "ground".into(),
        6 => "grass".into(),
        7 => "target_grass".into(),
        9 => "pool".into(),
        10 => "water".into(),
        11 => "floor1".into(),
        111 => "floor2".into(),
        12 => "walls1".into(),
        112 => "walls2".into(),
        3 => "heads1".into(),
        103 => "heads2".into(),
        8 => "flags1".into(),
        108 => "flags2".into(),
        _ => panic!("unexpected {id}"),
    }
}
fn stride(id: u32) -> usize {
    match id {
        0 => 4,
        1 | 101 | 4 | 6 | 7 | 8 | 108 | 9 | 10 => 1,
        2 | 102 => 8,
        11 | 111 | 3 | 103 => 2,
        12 | 112 => 14,
        _ => unreachable!(),
    }
}
fn byte(id: u32, x: usize, y: usize, c: usize) -> u8 {
    ((id as usize * 13 + x * 17 + y * 29 + c * 31) & 255) as u8 | 128
}
impl Services {
    fn new(row: &Value) -> Self {
        Self {
            statuses: serde_json::from_value(row["input"]["statuses"].clone()).unwrap_or_default(),
            parsed_failure: row["input"]["parsed_allocation_failure"]
                .as_u64()
                .map(|x| x as u32),
            ordinal: 0,
            current_temp: None,
            events: vec![],
            leases: BTreeSet::new(),
            live: BTreeMap::new(),
            block: None,
        }
    }
    fn ready(&self, b: Block) -> Result<(), Stop> {
        if self.block == Some(b) {
            Err(Stop::Unavailable(format!("{b:?} pending")))
        } else {
            Ok(())
        }
    }
}
impl saved_matrix::Host for Services {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        self.ready(Block::Lookup)?;
        assert_eq!(tag, ARRY);
        assert_eq!(callback, saved_matrix::NOOP_RESOURCE_CALLBACK);
        let status = self.statuses.get(&id).copied().unwrap_or(0);
        self.events.push(json!({"event":"read","id":id,"destination":destination(id,&self.current_temp),"converter":if matches!(id,3|103){saved_matrix::HEAD_CONVERTER}else{0},"status":status,"host_wrote":status==0}));
        if status != 0 && Some(id) != self.parsed_failure {
            Ok(Lookup::Missing {
                native_status: status,
            })
        } else {
            let lease = id as u64 + 1;
            assert!(self.leases.insert(lease));
            Ok(Lookup::Found { lease })
        }
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        self.ready(Block::Source)?;
        assert!(self.leases.contains(&lease));
        let id = lease as u32 - 1;
        let stride = stride(id);
        let temp = matches!(id, 1 | 2 | 101 | 102);
        let width = if Some(id) == self.parsed_failure {
            65
        } else if temp {
            64
        } else {
            4
        };
        let height = if temp { 64 } else { 4 };
        let mut out = [stride as i16, width as i16, height as i16, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        out.resize(8 + width * 4, 0xcc);
        if Some(id) != self.parsed_failure {
            for x in 0..width {
                for y in 0..height {
                    for c in 0..stride {
                        out.push(byte(id, x, y, c));
                    }
                }
            }
        }
        Ok(out)
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        assert!(self.leases.remove(&lease));
        Ok(())
    }
    fn free(&mut self, _: u64) -> Result<(), Stop> {
        Ok(())
    }
    fn allocate(&mut self, _: AllocationRequest) -> Result<Allocated, Stop> {
        assert!(self.parsed_failure.is_some());
        Ok(Allocated::Null)
    }
}
impl Host for Services {
    fn construct_temporary(&mut self, kind: Legacy) -> Result<Temporary, Stop> {
        self.ready(Block::Construct)?;
        let ordinal = self.ordinal;
        self.ordinal += 1;
        let kind_name = if kind == Legacy::Floor {
            "floor"
        } else {
            "wall"
        };
        let name = format!("temp_{kind_name}{ordinal}");
        let stride = if kind == Legacy::Floor { 1 } else { 8 };
        let owner = 1000 + ordinal;
        assert!(self.live.insert(owner, name.clone()).is_none());
        self.current_temp = Some(name);
        self.events
            .push(json!({"event":"allocate_temp_object","bytes":32,"ordinal":ordinal}));
        self.events.push(json!({"event":"construct_temp","kind":kind_name,"size":64,"ordinal":ordinal,"stride":stride}));
        Ok(Temporary {
            owner,
            matrix: Matrix {
                width: 64,
                height: 64,
                stride,
                allocation: Allocation::Known(Backing {
                    token: 2000 + ordinal,
                    column_offsets: (0..64).map(|x| x * 64 * stride as u32).collect(),
                    data: vec![0xa5; 64 * 64 * stride as usize],
                }),
            },
        })
    }
    fn destroy_temporary(&mut self, t: &Temporary) -> Result<(), Stop> {
        self.ready(Block::Destroy)?;
        let name = self.live.remove(&t.owner).unwrap();
        self.events
            .push(json!({"event":"destroy_temp_backing","destination":name}));
        self.events
            .push(json!({"event":"free_temp_object","destination":name}));
        self.current_temp = None;
        Ok(())
    }
    fn name_arrays(&mut self) -> Result<(), Stop> {
        self.ready(Block::Names)?;
        self.events.push(json!({"event":"debug_names_requested"}));
        Ok(())
    }
}
fn step(
    exec: &mut Execution,
    state: &mut State,
    scratch: &mut Scratch,
    host: &mut Services,
    writes: &mut Vec<Value>,
) -> Result<Option<i32>, Stop> {
    let before = exec.operations[exec.next];
    let (x, y) = (exec.copy_x, exec.copy_y);
    let result = exec.step(state, scratch, host);
    if result.is_ok() {
        match before {
            Operation::Lights(level) => host
                .events
                .push(json!({"event":"reset_light","level":level+1})),
            Operation::Copy { kind, level }
                if exec.copy_x == x + 1 && y < state.active_size.max(0) as usize =>
            {
                let (name, matrix, width) = match kind {
                    Legacy::Floor => (
                        format!("floor{}", level + 1),
                        &state.floors[level as usize].covering,
                        2,
                    ),
                    Legacy::Wall => (
                        format!("walls{}", level + 1),
                        &state.floors[level as usize].wall,
                        14,
                    ),
                };
                let value = cell(matrix, x, y, width).unwrap();
                let fields = if kind == Legacy::Floor {
                    vec![(0, 2)]
                } else {
                    vec![
                        (0, 1),
                        (1, 1),
                        (2, 2),
                        (4, 2),
                        (6, 2),
                        (8, 2),
                        (10, 2),
                        (12, 2),
                    ]
                };
                for (offset, size) in fields {
                    let number = if size == 1 {
                        u32::from(value[offset])
                    } else {
                        u32::from(u16::from_le_bytes(
                            value[offset..offset + 2].try_into().unwrap(),
                        ))
                    };
                    writes.push(json!({"name":name,"x":x,"y":y,"cell_offset":offset,"size":size,"value":number}));
                }
            }
            _ => {}
        }
    }
    result
}
fn finish(
    exec: &mut Execution,
    state: &mut State,
    scratch: &mut Scratch,
    host: &mut Services,
    writes: &mut Vec<Value>,
) -> i32 {
    for _ in 0..10000 {
        if let Some(status) = step(exec, state, scratch, host, writes).unwrap() {
            return status;
        }
    }
    panic!("driver did not complete");
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_478599_cases_match_with_real_matrix_reader_composition() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/saved-world-native.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 24);
    let mut count = 0;
    for row in fixture["cases"].as_array().unwrap() {
        let case = row["input"]["name"].as_str().unwrap();
        let mut state = setup(row);
        let original = state.clone();
        let mut exec = Execution::new(row["input"]["version"].as_i64().unwrap() as i32).unwrap();
        let mut scratch = Scratch {
            requested_converter: 0x1234,
        };
        let mut host = Services::new(row);
        let mut writes = vec![];
        let status = finish(&mut exec, &mut state, &mut scratch, &mut host, &mut writes);
        assert_eq!(json!(status), row["status"], "{case}");
        for (name, matrix) in names(&state) {
            assert_eq!(view(matrix), row["after"][name], "{case}:{name}");
        }
        for (level, floor) in state.floors.iter().enumerate() {
            let words: Vec<_> = floor
                .lights
                .values
                .iter()
                .copied()
                .map(u32::from_le_bytes)
                .collect();
            assert_eq!(
                json!(words),
                row["lights"][level]["output"],
                "{case}:lights"
            );
            assert_eq!(
                floor.primary, original.floors[level].primary,
                "{case}:primary changed"
            );
        }
        assert_eq!(json!(host.events), row["events"], "{case}:events");
        assert_eq!(
            json!(scratch.requested_converter),
            row["last_requested_converter"],
            "{case}:scratch"
        );
        let expected: Vec<_> = row["original_widening_writes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                let mut w = w.clone();
                w.as_object_mut().unwrap().remove("eip");
                let size = w["size"].as_u64().unwrap();
                w["value"] = json!(w["value"].as_u64().unwrap() & ((1u64 << (size * 8)) - 1));
                w
            })
            .collect();
        assert_eq!(writes, expected, "{case}:widening order");
        count += writes.len();
        assert_eq!(
            json!(host.live.values().collect::<Vec<_>>()),
            row["live_temps"],
            "{case}:live temps"
        );
        assert_eq!(
            json!(host.leases.iter().map(|x| x - 1).collect::<Vec<_>>()),
            row["held_source_leases"],
            "{case}:leases"
        );
        if let Some(t) = &exec.temporary
            && row["input"]["parsed_allocation_failure"].is_number()
        {
            assert_eq!(t.matrix.width, 65);
            assert!(matches!(t.matrix.allocation, Allocation::Null));
        }
        let frozen = (exec.clone(), state.clone(), scratch, host.clone());
        assert_eq!(
            exec.step(&mut state, &mut scratch, &mut host).unwrap(),
            Some(status)
        );
        assert_eq!(
            (exec, state, scratch, host),
            frozen,
            "{case}:repeat completion changed state"
        );
    }
    assert_eq!(count, 732);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn pending_host_services_and_mid_copy_cuts_restore_without_replaying_work() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/saved-world-native.json")).unwrap();
    let row = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["name"] == "v62_success")
        .unwrap_or(&fixture["cases"][0]);
    for block in [
        Some(Block::Lookup),
        Some(Block::Source),
        Some(Block::Construct),
        Some(Block::Destroy),
        Some(Block::Names),
        None,
    ] {
        let mut state = setup(row);
        let mut exec = Execution::new(62).unwrap();
        let mut scratch = Scratch {
            requested_converter: 19,
        };
        let mut host = Services::new(row);
        host.block = block;
        let mut writes = vec![];
        let mut stopped = false;
        for _ in 0..1000 {
            match step(&mut exec, &mut state, &mut scratch, &mut host, &mut writes) {
                Err(Stop::Unavailable(_)) => {
                    stopped = true;
                    break;
                }
                Ok(None) if block.is_none() && exec.copy_x == 1 => {
                    stopped = true;
                    break;
                }
                Ok(None) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(stopped, "{block:?}");
        let bytes = serde_json::to_vec(&(
            state.clone(),
            exec.clone(),
            scratch,
            host.clone(),
            writes.clone(),
        ))
        .unwrap();
        let (mut cold_state, mut cold_exec, mut cold_scratch, mut cold_host, mut cold_writes): (
            State,
            Execution,
            Scratch,
            Services,
            Vec<Value>,
        ) = serde_json::from_slice(&bytes).unwrap();
        host.block = None;
        cold_host.block = None;
        assert_eq!(
            finish(&mut exec, &mut state, &mut scratch, &mut host, &mut writes),
            0
        );
        assert_eq!(
            finish(
                &mut cold_exec,
                &mut cold_state,
                &mut cold_scratch,
                &mut cold_host,
                &mut cold_writes
            ),
            0
        );
        assert_eq!(
            (state, exec, scratch, host, writes),
            (cold_state, cold_exec, cold_scratch, cold_host, cold_writes),
            "{block:?}"
        );
    }
}

/// Host effects may complete before their reply reaches the driver. This
/// wrapper retains that reply instead of executing constructor/free/name twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PartialServices {
    inner: Services,
    block: Block,
    pending_construct: Option<Temporary>,
    pending_destroy: Option<u64>,
    pending_names: bool,
    stopped: bool,
}
impl saved_matrix::Host for PartialServices {
    fn lookup(&mut self, t: u32, i: u32, c: u32) -> Result<Lookup, Stop> {
        self.inner.lookup(t, i, c)
    }
    fn source(&mut self, l: u64) -> Result<Vec<u8>, Stop> {
        self.inner.source(l)
    }
    fn release(&mut self, l: u64) -> Result<(), Stop> {
        self.inner.release(l)
    }
    fn free(&mut self, t: u64) -> Result<(), Stop> {
        self.inner.free(t)
    }
    fn allocate(&mut self, r: AllocationRequest) -> Result<Allocated, Stop> {
        self.inner.allocate(r)
    }
}
impl Host for PartialServices {
    fn construct_temporary(&mut self, k: Legacy) -> Result<Temporary, Stop> {
        if let Some(reply) = self.pending_construct.take() {
            return Ok(reply);
        }
        let reply = self.inner.construct_temporary(k)?;
        if self.block == Block::Construct && !self.stopped {
            self.stopped = true;
            self.pending_construct = Some(reply);
            return Err(Stop::Unavailable("constructor reply pending".into()));
        }
        Ok(reply)
    }
    fn destroy_temporary(&mut self, t: &Temporary) -> Result<(), Stop> {
        if self.pending_destroy == Some(t.owner) {
            self.pending_destroy = None;
            return Ok(());
        }
        self.inner.destroy_temporary(t)?;
        if self.block == Block::Destroy && !self.stopped {
            self.stopped = true;
            self.pending_destroy = Some(t.owner);
            return Err(Stop::Unavailable("destructor reply pending".into()));
        }
        Ok(())
    }
    fn name_arrays(&mut self) -> Result<(), Stop> {
        if self.pending_names {
            self.pending_names = false;
            return Ok(());
        }
        self.inner.name_arrays()?;
        if self.block == Block::Names && !self.stopped {
            self.stopped = true;
            self.pending_names = true;
            return Err(Stop::Unavailable("names reply pending".into()));
        }
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn host_partial_effects_and_pending_replies_survive_cold_restore() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/saved-world-native.json")).unwrap();
    let row = &fixture["cases"][0];
    for block in [Block::Construct, Block::Destroy, Block::Names] {
        let mut state = setup(row);
        let mut exec = Execution::new(62).unwrap();
        let mut scratch = Scratch {
            requested_converter: 0,
        };
        let mut host = PartialServices {
            inner: Services::new(row),
            block,
            pending_construct: None,
            pending_destroy: None,
            pending_names: false,
            stopped: false,
        };
        let mut stopped = false;
        for _ in 0..1000 {
            match exec.step(&mut state, &mut scratch, &mut host) {
                Err(Stop::Unavailable(_)) => {
                    stopped = true;
                    break;
                }
                Ok(None) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(stopped);
        let raw =
            serde_json::to_vec(&(state.clone(), exec.clone(), scratch, host.clone())).unwrap();
        let (mut cold_state, mut cold_exec, mut cold_scratch, mut cold_host): (
            State,
            Execution,
            Scratch,
            PartialServices,
        ) = serde_json::from_slice(&raw).unwrap();
        for _ in 0..1000 {
            if exec.step(&mut state, &mut scratch, &mut host).unwrap() == Some(0) {
                break;
            }
        }
        for _ in 0..1000 {
            if cold_exec
                .step(&mut cold_state, &mut cold_scratch, &mut cold_host)
                .unwrap()
                == Some(0)
            {
                break;
            }
        }
        assert_eq!(exec.status(), Some(0));
        assert_eq!(
            (state, exec, scratch, host),
            (cold_state, cold_exec, cold_scratch, cold_host)
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn light_reset_requires_used_prefix_without_demanding_unconsumed_capacity_tail() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/saved-world-native.json")).unwrap();
    let row = &fixture["cases"][0];
    let mut state = setup(row);
    for f in &mut state.floors {
        f.lights.values.truncate(f.lights.used);
    }
    let mut exec = Execution::new(62).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = Services::new(row);
    let mut writes = vec![];
    assert_eq!(
        finish(&mut exec, &mut state, &mut scratch, &mut host, &mut writes),
        0
    );
    for f in &state.floors {
        assert_eq!(f.lights.values, vec![[0x7f; 4]; f.lights.used]);
        assert!(f.lights.capacity > f.lights.values.len());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn ignored_native_allocation_failure_keeps_the_lease_and_continues_to_water() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/saved-world-native.json")).unwrap();
    let row = &fixture["cases"][0];
    let mut state = setup(row);
    let mut exec = Execution::new(62).unwrap();
    let mut scratch = Scratch {
        requested_converter: 0,
    };
    let mut host = Services::new(row);
    host.statuses.insert(9, -1);
    host.parsed_failure = Some(9);
    let mut writes = vec![];
    assert_eq!(
        finish(&mut exec, &mut state, &mut scratch, &mut host, &mut writes),
        0
    );
    assert_eq!(exec.abandoned_reads.len(), 1);
    assert_eq!(exec.abandoned_reads[0].held_lease, Some(10));
    assert_eq!(host.leases, [10].into());
    assert!(matches!(state.pool.allocation, Allocation::Null));
    assert_eq!(state.pool.width, 65);
    assert!(
        host.events
            .iter()
            .any(|e| e["event"] == "read" && e["id"] == 10)
    );
    assert!(exec.temporary.is_none());
    assert!(host.live.is_empty());
    let cut = (state, exec, scratch, host);
    let raw = serde_json::to_vec(&cut).unwrap();
    let cold: (State, Execution, Scratch, Services) = serde_json::from_slice(&raw).unwrap();
    assert_eq!(cut, cold);
}
#[test]
fn older_unimplemented_profile_is_not_a_fabricated_native_status() {
    assert!(matches!(Execution::new(52), Err(Stop::Unsupported(_))));
}
