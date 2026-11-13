use super::*;
use crate::saved_matrix::{Allocation, Backing};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
fn hex(b: &[u8]) -> String {
    b.iter().map(|n| format!("{n:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn groups(rows: &Value, owner: u64, entries: u64, capacity: usize) -> Groups {
    let values = rows
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let values = row["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Entry {
                    saved: v[0].as_u64().unwrap() as u16,
                    local: v[1].as_u64().unwrap() as u16,
                    key: v[2].as_u64().unwrap() as u32,
                })
                .collect::<Vec<_>>();
            Group {
                name: Some(row["name"].as_str().unwrap().as_bytes().to_vec()),
                entries: Entries {
                    allocation: Some(entries + i as u64),
                    used: values.len(),
                    capacity: values.len(),
                    values: Some(values),
                },
            }
        })
        .collect::<Vec<_>>();
    Groups {
        allocation: (!values.is_empty()).then_some(owner),
        capacity,
        values,
    }
}
fn group_view(g: &Groups) -> Value {
    json!(g.values.iter().map(|g|json!({"name":String::from_utf8(g.name.clone().unwrap()).unwrap(),"entries":g.entries.values.as_ref().unwrap().iter().map(|e|json!([e.saved,e.local,e.key])).collect::<Vec<_>>()})).collect::<Vec<_>>())
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Services {
    case: Value,
    events: Vec<Value>,
    backing: usize,
    walls: Vec<Vec<ExpandedWall>>,
    allocations: BTreeMap<u64, String>,
    table_image: Option<Vec<u16>>,
    block: Option<String>,
    pending: Option<(String, Value)>,
}
impl Services {
    fn start(&mut self, key: &str) -> Option<Value> {
        if self.pending.as_ref().is_some_and(|(k, _)| k == key) {
            Some(self.pending.take().unwrap().1)
        } else {
            None
        }
    }
    fn reply(&mut self, key: &str, value: Value) -> Result<Value, Stop> {
        if self.block.as_deref() == Some(key) {
            self.block = None;
            self.pending = Some((key.into(), value));
            return Err(Stop::Unavailable(format!("pending {key} reply")));
        }
        Ok(value)
    }
    fn install(&mut self, source: &Value, retained: bool, target: &mut Groups) {
        let count = source.as_array().unwrap().len();
        if count == 0 {
            return;
        }
        let entries = if retained { 200 } else { 100 };
        *target = groups(
            source,
            if retained { 1 } else { 3 },
            entries,
            if retained { 4 } else { count },
        );
        for (i, g) in target.values.iter().enumerate() {
            self.allocations.insert(
                entries + i as u64,
                format!(
                    "entry:{}:{i}",
                    String::from_utf8(g.name.clone().unwrap()).unwrap()
                ),
            );
        }
        self.allocations.insert(
            if retained { 1 } else { 3 },
            if retained {
                "groups:retained"
            } else {
                "groups:temporary"
            }
            .into(),
        );
    }
}
impl Host for Services {
    fn free_vector(&mut self, owner: u64, bytes: usize) -> Result<(), Stop> {
        let key = format!("free:{owner}");
        if self.start(&key).is_some() {
            return Ok(());
        }
        self.events
            .push(json!({"event":"free_vector","owner":self.allocations[&owner],"bytes":bytes}));
        self.reply(&key, Value::Null)?;
        Ok(())
    }
    fn read_map(&mut self, kind: Kind, id: u32, target: &mut Groups) -> Result<i32, Stop> {
        if let Some(v) = self.start("read") {
            return Ok(v.as_i64().unwrap() as i32);
        }
        self.events.push(json!({"event":"read_map","tag":if kind==Kind::Floor{0x464c526du32}else{0x57414c6du32},"id":id,"retained":id==1}));
        if self.case["source_stop"] == true {
            return Err(Stop::Unavailable("source stop".into()));
        }
        let rows = self.case.get("groups").cloned().unwrap_or(json!([]));
        self.install(&rows, id == 1, target);
        let status = self.case["status"].as_i64().unwrap_or(0) as i32;
        Ok(self.reply("read", json!(status))?.as_i64().unwrap() as i32)
    }
    fn normalize_legacy(&mut self, source: &Groups, target: &mut Groups) -> Result<(), Stop> {
        if self.start("normalize").is_some() {
            return Ok(());
        }
        let rows = group_view(source);
        self.events
            .push(json!({"event":"legacy_normalize","groups":rows}));
        self.install(&rows, true, target);
        self.reply("normalize", Value::Null)?;
        Ok(())
    }
    fn allocate_table(&mut self, bytes: usize) -> Result<u64, Stop> {
        if let Some(v) = self.start("allocate") {
            return Ok(v.as_u64().unwrap());
        }
        self.events
            .push(json!({"event":"allocate_table","bytes":bytes}));
        Ok(self.reply("allocate", json!(4))?.as_u64().unwrap())
    }
    fn free_table(&mut self, _owner: u64) -> Result<(), Stop> {
        if self.start("free_table").is_some() {
            return Ok(());
        }
        self.events.push(json!({"event":"free_table"}));
        self.reply("free_table", Value::Null)?;
        Ok(())
    }
    fn match_file(&mut self, name: &[u8], file: u64) -> Result<bool, Stop> {
        if let Some(v) = self.start("match") {
            return Ok(v.as_bool().unwrap());
        }
        let index = file as usize;
        let name = String::from_utf8(name.to_vec()).unwrap();
        let found = self.case["file_matches"][&name].as_u64() == Some(index as u64);
        self.events
            .push(json!({"event":"match_file","name":name,"index":index,"match":found}));
        Ok(self.reply("match", json!(found))?.as_bool().unwrap())
    }
    fn resolve_direct(&mut self, _kind: Kind, index: usize, local: u16) -> Result<u16, Stop> {
        if let Some(v) = self.start("direct") {
            return Ok(v.as_u64().unwrap() as u16);
        }
        let value = self.case["direct"][format!("{index}:{local}")]
            .as_u64()
            .unwrap_or(65535) as u16;
        self.events
            .push(json!({"event":"resolve_direct","index":index,"local":local,"value":value}));
        Ok(self.reply("direct", json!(value))?.as_u64().unwrap() as u16)
    }
    fn resolve_nearest(&mut self, _kind: Kind, key: u32) -> Result<u16, Stop> {
        if let Some(v) = self.start("nearest") {
            return Ok(v.as_u64().unwrap() as u16);
        }
        let value = self.case["nearest"][key.to_string()]
            .as_u64()
            .unwrap_or(65535) as u16;
        self.events
            .push(json!({"event":"resolve_nearest","key":key,"value":value}));
        Ok(self.reply("nearest", json!(value))?.as_u64().unwrap() as u16)
    }
    fn get_wall(&mut self, at: Coord) -> Result<ExpandedWall, Stop> {
        if let Some(v) = self.start("get") {
            return Ok(ExpandedWall(serde_json::from_value(v).unwrap()));
        }
        self.events
            .push(json!({"event":"get_wall","at":[at.x,at.y,at.level]}));
        let wall = self.walls[usize::from(at.level - 1)]
            [usize::from(at.y) * self.backing + usize::from(at.x)]
        .clone();
        Ok(ExpandedWall(
            serde_json::from_value(self.reply("get", json!(wall.0))?).unwrap(),
        ))
    }
    fn put_wall(&mut self, at: Coord, wall: &ExpandedWall) -> Result<(), Stop> {
        if self.start("put").is_some() {
            return Ok(());
        }
        self.events
            .push(json!({"event":"put_wall","at":[at.x,at.y,at.level],"value_hex":hex(&wall.bytes(0, 44).unwrap())}));
        self.walls[usize::from(at.level - 1)]
            [usize::from(at.y) * self.backing + usize::from(at.x)] = wall.clone();
        self.reply("put", Value::Null)?;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Case {
    state: State,
    exec: Execution,
    floors: [Matrix; 2],
    host: Services,
    floor_writes: Vec<Value>,
    wall_writes: Vec<Value>,
    active: i32,
}
fn setup(row: &Value) -> Case {
    let n = row["backing"].as_i64().unwrap() as i32;
    let cols: Vec<u32> = serde_json::from_value(row["column_offsets"].clone()).unwrap();
    let floors = std::array::from_fn(|i| Matrix {
        width: n,
        height: n,
        stride: 2,
        allocation: Allocation::Known(Backing {
            token: 10 + i as u64,
            column_offsets: cols.clone(),
            data: unhex(row["before"]["floors"][i].as_str().unwrap()),
        }),
    });
    let initial = groups(&row["before"]["groups"], 1, 2, 4);
    let kind = if row["input"]["kind"] == "floor" {
        Kind::Floor
    } else {
        Kind::Wall
    };
    Case {
        state: State {
            groups: initial,
            runtime_files: vec![0, 1],
        },
        exec: Execution::new(kind, row["input"]["version"].as_i64().unwrap() as i32),
        floors,
        active: row["active"].as_i64().unwrap() as i32,
        floor_writes: vec![],
        wall_writes: vec![],
        host: Services {
            case: row["input"].clone(),
            events: vec![],
            backing: n as usize,
            walls: row["before"]["walls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    r.as_array()
                        .unwrap()
                        .iter()
                        .map(|r| ExpandedWall::known(unhex(r.as_str().unwrap())))
                        .collect()
                })
                .collect(),
            allocations: [(1, "groups:retained".into()), (2, "entry:old:0".into())].into(),
            table_image: None,
            block: None,
            pending: None,
        },
    }
}
fn data(m: &Matrix) -> &[u8] {
    let Allocation::Known(b) = &m.allocation else {
        panic!()
    };
    &b.data
}
fn step(case: &mut Case) -> Result<Option<i32>, Stop> {
    let old = case.exec.clone();
    let before = case.floors.clone();
    // Local, non-service native function boundaries remain visible to the oracle.
    if old.phase == Phase::BeginErase {
        case.host
            .events
            .push(json!({"event":"erase_groups","count":case.state.groups.values.len()}));
    }
    if old.phase == Phase::LegacyClear {
        case.host
            .events
            .push(json!({"event":"clear_temporary_groups","count":0}));
    }
    if old.phase == Phase::BeginDestroy {
        case.host
            .events
            .push(json!({"event":"destroy_temporary_groups","groups":group_view(&old.temporary)}));
    }
    let pattern = if old.phase == Phase::Pattern
        && old.side < (if matches!(old.bit, 0x10 | 0x20) { 2 } else { 1 })
    {
        let side = old.pattern_side();
        let offset = pattern_offset(old.bit, side)?;
        case.host.events.push(json!({"event":"get_pattern","at":[old.coord.x,old.coord.y,old.coord.level],"args":[old.bit,side]}));
        Some((offset, side))
    } else {
        None
    };
    let [floor0, floor1] = &mut case.floors;
    let result = case.exec.step(
        &mut case.state,
        case.active,
        [floor0, floor1],
        &mut case.host,
    );
    if result.is_ok() {
        for (level, previous) in before.iter().enumerate() {
            if previous != &case.floors[level] {
                let x = usize::from(old.coord.x);
                let y = usize::from(old.coord.y);
                let value = u16::from_le_bytes(
                    saved_world::cell(&case.floors[level], x, y, 2)?
                        .try_into()
                        .unwrap(),
                );
                case.floor_writes.push(
                    json!({"level":level+1,"x":x,"y":y,"size":2,"value":value,"eip":0x4a1074}),
                );
            }
        }
        if let Some((offset, side)) = pattern {
            let old_word = old.wall.as_ref().unwrap().word(offset)?;
            let new_word = case.exec.wall.as_ref().unwrap().word(offset)?;
            if old_word != new_word {
                case.host.events.push(json!({"event":"set_pattern","at":[old.coord.x,old.coord.y,old.coord.level],"args":[new_word,old.bit,side]}));
            }
        }
        let offset = pattern
            .map(|v| v.0)
            .or_else(|| (old.phase == Phase::FloorHalf && old.half < 2).then(|| [8, 20][old.half]));
        if let Some(offset) = offset {
            let old_wall = old.wall.as_ref().unwrap();
            let new_wall = case.exec.wall.as_ref().unwrap();
            if old_wall.word(offset)? != new_wall.word(offset)? {
                let at = json!([old.coord.x, old.coord.y, old.coord.level]);
                case.wall_writes.push(json!({"at":at,"offset":offset,"size":2,"value":new_wall.word(offset)?,"eip":if pattern.is_some(){0x49399d}else if offset==8{0x4a114c}else{0x4a1146}}));
                case.wall_writes.push(json!({"at":at,"offset":40,"size":4,"value":u32::from_le_bytes(new_wall.bytes(40, 4)?.try_into().unwrap()),"eip":if pattern.is_some(){0x4939a8}else{0x4a1150}}));
            }
        }
    }
    if case.exec.phase == Phase::Free {
        case.host.table_image = case.exec.table.clone();
    }
    result
}
fn finish(case: &mut Case) -> Result<i32, Stop> {
    for _ in 0..10000 {
        if let Some(status) = step(case)? {
            return Ok(status);
        }
    }
    panic!("surface run not complete")
}
fn view(case: &Case) -> Value {
    json!({"floors":case.floors.iter().map(|f|hex(data(f))).collect::<Vec<_>>(),"walls":case.host.walls.iter().map(|r|r.iter().map(|w|hex(&w.bytes(0, 44).unwrap())).collect::<Vec<_>>()).collect::<Vec<_>>(),"groups":group_view(&case.state.groups)})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_surface_cases_match_all_bytes_ordered_services_and_stores() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    let mut floors = 0;
    let mut walls = 0;
    for row in fixture["cases"].as_array().unwrap() {
        let mut case = setup(row);
        let result = finish(&mut case);
        let name = row["input"]["name"].as_str().unwrap();
        if row["completed"] == false {
            assert!(
                matches!(result, Err(Stop::Unavailable(_))),
                "{name}: {result:?}"
            );
            assert_eq!(case.exec.status(), None);
        } else {
            assert_eq!(
                result.unwrap(),
                row["status"].as_i64().unwrap() as i32,
                "{name}"
            );
        }
        assert_eq!(view(&case), row["after"], "{name}.after");
        assert_eq!(json!(case.host.events), row["events"], "{name}.events");
        assert_eq!(
            json!(case.floor_writes),
            row["original_floor_writes"],
            "{name}.floor_writes"
        );
        assert_eq!(
            json!(case.wall_writes),
            row["original_expanded_wall_writes"],
            "{name}.wall_writes"
        );
        let hash = case.host.table_image.as_ref().map(|t| {
            let bytes: Vec<u8> = t.iter().flat_map(|n| n.to_le_bytes()).collect();
            hex(&Sha256::digest(bytes))
        });
        assert_eq!(json!(hash), row["table_sha256"], "{name}.table");
        assert_eq!(
            case.state.groups.capacity * 284,
            row["retained_group_extents"][2].as_u64().unwrap() as usize
        );
        floors += case.floor_writes.len();
        walls += case.wall_writes.len();
        let raw = serde_json::to_vec(&case).unwrap();
        let cold: Case = serde_json::from_slice(&raw).unwrap();
        assert_eq!(case, cold);
    }
    assert_eq!(floors, 138);
    assert_eq!(walls, 374);
}
#[cfg(feature = "private-fixtures")]
#[test]
fn old_group_teardown_does_not_require_unconsumed_names_or_entry_values() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    let row = &fixture["cases"][0];
    let mut case = setup(row);
    case.state.groups.values[0].name = None;
    case.state.groups.values[0].entries.values = None;
    assert_eq!(finish(&mut case).unwrap(), 0);
    assert_eq!(view(&case), row["after"]);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn pending_host_replies_preserve_side_effects_and_do_not_repeat_services() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    for name in [
        "floor_file1",
        "wall_file1",
        "floor_declared_unresolved",
        "wall_declared_unresolved",
    ] {
        let row = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["input"]["name"] == name)
            .unwrap();
        let mut baseline = setup(row);
        assert_eq!(finish(&mut baseline).unwrap(), 0);
        let mut keys = vec![
            "free:2",
            "read",
            "allocate",
            "match",
            "nearest",
            "get",
            "put",
            "free_table",
        ];
        if row["input"]["version"] == 62 {
            keys.extend(["normalize", "free:100", "free:3"]);
        } else {
            keys.push("direct");
        }
        for key in keys {
            let mut case = setup(row);
            case.host.block = Some(key.into());
            let mut cut = None;
            for _ in 0..10000 {
                match step(&mut case) {
                    Err(Stop::Unavailable(_)) => {
                        assert!(case.host.pending.is_some());
                        let raw = serde_json::to_vec(&case).unwrap();
                        let cold: Case = serde_json::from_slice(&raw).unwrap();
                        assert_eq!(case, cold);
                        cut = Some(cold);
                        break;
                    }
                    Ok(Some(_)) => break,
                    Ok(None) => {}
                    other => panic!("{name}/{key}: {other:?}"),
                }
            }
            let mut cold = cut.unwrap_or_else(|| panic!("service not exercised: {name}/{key}"));
            assert_eq!(finish(&mut cold).unwrap(), 0);
            assert_eq!(cold, baseline, "{name}/{key}");
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn local_mapping_and_half_wall_cursors_cold_restore_without_publication_changes() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    for (name, cut) in [
        ("floor_file1", 0),
        ("floor_empty_62", 1),
        ("wall_empty_62", 2),
    ] {
        let row = fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["input"]["name"] == name)
            .unwrap();
        let mut baseline = setup(row);
        finish(&mut baseline).unwrap();
        let mut case = setup(row);
        let mut found = false;
        for _ in 0..10000 {
            let ready = match cut {
                0 => case.exec.phase == Phase::Entry && case.exec.entry == 1,
                1 => case.exec.phase == Phase::FloorHalf && case.exec.half == 1,
                2 => case.exec.phase == Phase::Pattern && case.exec.side == 1,
                _ => unreachable!(),
            };
            if ready {
                let raw = serde_json::to_vec(&case).unwrap();
                let mut cold: Case = serde_json::from_slice(&raw).unwrap();
                assert_eq!(case, cold);
                finish(&mut cold).unwrap();
                assert_eq!(cold, baseline, "{name}");
                found = true;
                break;
            }
            if step(&mut case).unwrap().is_some() {
                break;
            }
        }
        assert!(found, "local cut not exercised: {name}");
    }
}

#[test]
fn untouched_expanded_bytes_are_not_an_eager_source_requirement() {
    let mut bytes = vec![ObservedByte::unknown(); 44];
    for at in [0, 1, 2, 3, 8, 9, 20, 21, 40, 41, 42, 43] {
        bytes[at] = 0.into();
    }
    bytes[0] = 0x10.into();
    bytes[8] = 30.into();
    bytes[20] = 31.into();
    let mut exec = Execution::new(Kind::Floor, 62);
    exec.table = Some(vec![u16::MAX; 65535]);
    exec.wall = Some(ExpandedWall(bytes));
    exec.remap_wall_field(8).unwrap();
    exec.remap_wall_field(20).unwrap();
    let wall = exec.wall.as_ref().unwrap();
    assert_eq!(wall.word(8).unwrap(), 65535);
    assert_eq!(wall.word(20).unwrap(), 65535);
    assert_eq!(wall.0[4], ObservedByte::unknown());
    assert_eq!(wall.0[39], ObservedByte::unknown());
    assert_eq!(wall.bytes(40, 4).unwrap(), [16, 0, 0, 0]);
    let before = exec.clone();
    exec.wall.as_mut().unwrap().0[20] = ObservedByte::unknown();
    let missing = exec.clone();
    assert!(matches!(
        exec.remap_wall_field(20),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(exec, missing);
    assert_eq!(before.wall.unwrap().0[4], ObservedByte::unknown());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn zero_capacity_skips_free_but_unused_reserved_storage_is_freed() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    let row = &fixture["cases"][0];
    let mut case = setup(row);
    let entries = &mut case.state.groups.values[0].entries;
    entries.used = 0;
    entries.capacity = 0;
    entries.values = None;
    finish(&mut case).unwrap();
    assert!(
        !case
            .host
            .events
            .iter()
            .any(|e| e["event"] == "free_vector" && e["owner"] == "entry:old:0")
    );
    let mut case = setup(row);
    let entries = &mut case.state.groups.values[0].entries;
    entries.used = 0;
    entries.values = None;
    finish(&mut case).unwrap();
    assert!(
        case.host
            .events
            .iter()
            .any(|e| e["event"] == "free_vector" && e["owner"] == "entry:old:0" && e["bytes"] == 8)
    );
    for capacity in [0, 1] {
        let mut case = setup(row);
        case.host.events.clear();
        case.host.allocations.insert(99, "groups:temporary".into());
        case.exec.phase = Phase::DestroyStorage;
        case.exec.temporary = Groups {
            allocation: Some(99),
            capacity,
            values: vec![],
        };
        case.exec.read_status = Some(-42);
        assert_eq!(finish(&mut case).unwrap(), -42);
        assert_eq!(case.host.events.len(), capacity);
        if capacity == 1 {
            assert_eq!(case.host.events[0]["bytes"], 284);
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn accepted_wall_reply_with_unknown_mask_is_retained_across_cold_resume() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    let row = &fixture["cases"][0];
    let mut baseline = setup(row);
    finish(&mut baseline).unwrap();
    let mut case = setup(row);
    // The first diagonal get is floor255 at x0/y1/storey1, mask10.
    case.host.walls[0][case.host.backing].0[0] = ObservedByte::unknown();
    let mut cut = None;
    for _ in 0..10000 {
        match step(&mut case) {
            Err(Stop::Unavailable(_)) => {
                assert_eq!(case.exec.phase, Phase::WallReady);
                assert!(case.exec.wall.is_some());
                cut = Some(serde_json::to_vec(&case).unwrap());
                break;
            }
            Ok(None) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    let mut cold: Case = serde_json::from_slice(&cut.unwrap()).unwrap();
    assert_eq!(cold, case);
    let before = cold.host.events.clone();
    assert!(matches!(step(&mut cold), Err(Stop::Unavailable(_))));
    assert_eq!(cold.host.events, before);
    cold.exec.wall.as_mut().unwrap().0[0] = 0x10.into();
    finish(&mut cold).unwrap();
    assert_eq!(cold, baseline);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn unqualified_initial_high_wall_segment_does_not_silently_use_a_different_setter() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/surface-remap-native.json"
    ))
    .unwrap();
    let row = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["name"] == "wall_empty_62")
        .unwrap();
    let mut case = setup(row);
    case.host.walls[0][0].0[0] = 0x40.into();
    let initial = case.host.walls.clone();
    let mut stopped = false;
    for _ in 0..10000 {
        let [floor0, floor1] = &mut case.floors;
        match case.exec.step(
            &mut case.state,
            case.active,
            [floor0, floor1],
            &mut case.host,
        ) {
            Err(Stop::Unsupported(_)) => {
                stopped = true;
                break;
            }
            Ok(None) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(stopped);
    assert_eq!(case.exec.status(), None);
    assert_eq!(case.host.walls, initial);
    assert!(!case.host.events.iter().any(|e| e["event"] == "put_wall"));
    assert_eq!(case.exec.phase, Phase::Pattern);
}

#[test]
fn dirty_or_preserves_unknown_upper_bits_instead_of_requiring_unconsumed_values() {
    let mut wall = ExpandedWall(vec![ObservedByte::unknown(); 44]);
    wall.0[8] = 30.into();
    wall.0[9] = 0.into();
    wall.0[40] = ObservedByte {
        value: 0x05,
        known: 0x0f,
    };
    wall.set(8, 500).unwrap();
    assert_eq!(wall.word(8).unwrap(), 500);
    assert_eq!(
        wall.0[40],
        ObservedByte {
            value: 0x15,
            known: 0x1f
        }
    );
    for at in [41, 42, 43] {
        assert_eq!(wall.0[at], ObservedByte::unknown());
    }
    assert!(matches!(wall.bytes(40, 4), Err(Stop::Unavailable(_))));
    let mut known = ExpandedWall::known([0u8; 44]);
    known.0[40] = 0xa5.into();
    known.0[41] = 0x81.into();
    known.set(8, 500).unwrap();
    assert_eq!(known.bytes(40, 4).unwrap(), [0xb5, 0x81, 0, 0]);
}
