use super::*;
use crate::saved_matrix::Backing;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/native-wall-io.json")).unwrap()
}
fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn at(row: &Value) -> Coord {
    let a = row["input"]["at"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        })
        .unwrap_or(vec![2, 2, 1]);
    Coord {
        x: a[0],
        y: a[1],
        level: a[2],
    }
}
fn matrices(f: &Value, row: &Value) -> BTreeMap<String, Matrix> {
    row["maps"]
        .as_object()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, (name, m))| {
            let index = row["before"][name].as_u64().unwrap() as usize;
            let n = row["backing"].as_i64().unwrap() as i32;
            (
                name.clone(),
                Matrix {
                    width: n,
                    height: n,
                    stride: m["stride"].as_i64().unwrap() as i32,
                    allocation: Allocation::Known(Backing {
                        token: i as u64 + 1,
                        column_offsets: m["offsets"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_u64().unwrap() as u32)
                            .collect(),
                        data: bytes(f["images"][index].as_str().unwrap()),
                    }),
                },
            )
        })
        .collect()
}
fn getter_mask(row: &Value, m: &Matrix, coord: Coord) -> Vec<u8> {
    let mut known = vec![255; 44];
    known[40] = 31;
    known[41..44].fill(0);
    let active = row["active"].as_i64().unwrap() as i32;
    let x = usize::from(coord.x);
    let y = usize::from(coord.y);
    if i32::from(coord.x) >= active - 1 || raw(m, x + 1, y).unwrap()[0] & 1 == 0 {
        known[12..14].fill(0);
    }
    if i32::from(coord.y) >= active - 1 || raw(m, x, y + 1).unwrap()[0] & 2 == 0 {
        known[14..16].fill(0);
    }
    known
}
fn compare_get(row: &Value, m: &Matrix, wall: &ExpandedWall) {
    let expected = bytes(row["expanded_hex"].as_str().unwrap());
    let known = getter_mask(row, m, at(row));
    for (i, ((actual, value), mask)) in wall.0.iter().zip(expected).zip(known).enumerate() {
        assert_eq!(actual.known, mask, "{}/byte{i}", row["input"]["name"]);
        assert_eq!(
            actual.value & mask,
            value & mask,
            "{}/byte{i}",
            row["input"]["name"]
        );
        assert_eq!(actual.value & !mask, 0, "unknown bits must be canonical");
    }
}
fn completed(mut put: Put, mut walls: Matrix, mut vertices: Matrix) -> (Put, Matrix, Matrix) {
    while put.step(&mut walls, &mut vertices).unwrap().is_some() {}
    (put, walls, vertices)
}
fn event(w: &Write) -> Value {
    let map = format!(
        "{}{}",
        if w.target == Target::Walls {
            "walls"
        } else {
            "vertices"
        },
        w.at.level
    );
    let value = w
        .value
        .iter()
        .enumerate()
        .fold(0u64, |n, (i, b)| n | (u64::from(*b) << (8 * i)));
    serde_json::json!({"map":map,"x":w.at.x,"y":w.at.y,"offset":w.offset,"size":w.value.len(),"value":value,"eip":w.instruction})
}
#[test]
fn original_thirty_three_get_put_and_vertex_cases_match_bytes_bits_and_all_stores() {
    let f = fixture();
    assert_eq!(f["cases"].as_array().unwrap().len(), 33);
    let mut stores = 0;
    for row in f["cases"].as_array().unwrap() {
        let mut maps = matrices(&f, row);
        let coord = at(row);
        let active = row["active"].as_i64().unwrap() as i32;
        let wall_name = format!("walls{}", coord.level);
        let vertex_name = format!("vertices{}", coord.level);
        let wall = maps.remove(&wall_name).unwrap();
        let vertices = maps.remove(&vertex_name).unwrap();
        let op = row["input"]["op"].as_str().unwrap();
        let input = if op == "get" || op == "roundtrip" {
            let got = get(&wall, active, coord).unwrap();
            compare_get(row, &wall, &got);
            let size = wall.width as usize;
            assert_eq!(wall.width, wall.height);
            let mut cells = Vec::new();
            for x in 0..size {
                for y in 0..size {
                    let b = raw(&wall, x, y).unwrap();
                    cells.push(sims_formats::lot::WallTile {
                        segments: b[0],
                        flags: b[1],
                        styles: std::array::from_fn(|n| {
                            u16::from_le_bytes(b[2 + n * 2..4 + n * 2].try_into().unwrap())
                        }),
                        patterns: std::array::from_fn(|n| {
                            u16::from_le_bytes(b[6 + n * 2..8 + n * 2].try_into().unwrap())
                        }),
                    });
                }
            }
            let live = get_cells(size, active as usize, &cells, coord).unwrap();
            compare_get(row, &wall, &live);
            assert_eq!(
                live, got,
                "borrowed live-cell getter differs from the original matrix profile"
            );
            Some(got)
        } else if op == "put" {
            Some(ExpandedWall::known(bytes(
                row["expanded_hex"].as_str().unwrap(),
            )))
        } else {
            None
        };
        let (wall, vertices) = if op == "put" || op == "roundtrip" {
            let result = plan_put(&wall, &vertices, active, coord, input.as_ref().unwrap());
            if !row["completed"].as_bool().unwrap() {
                assert!(matches!(result, Err(Stop::Unsupported(_))));
                (wall, vertices)
            } else {
                let put = result.unwrap();
                let events: Vec<_> = put.writes().iter().map(event).collect();
                assert_eq!(
                    serde_json::to_value(events).unwrap(),
                    row["writes"],
                    "{}",
                    row["input"]["name"]
                );
                stores += put.writes().len();
                let (_, wall, vertices) = completed(put, wall, vertices);
                (wall, vertices)
            }
        } else if op == "vertex" {
            assert_eq!(
                vertex(&wall, active, coord).unwrap(),
                row["vertex"].as_u64().unwrap() as u8
            );
            (wall, vertices)
        } else {
            (wall, vertices)
        };
        maps.insert(wall_name, wall);
        maps.insert(vertex_name, vertices);
        for (name, matrix) in maps {
            let index = row["after"][&name].as_u64().unwrap() as usize;
            assert_eq!(
                hex(&backing(&matrix).unwrap().data),
                f["images"][index].as_str().unwrap(),
                "{}/{}",
                row["input"]["name"],
                name
            );
        }
    }
    assert_eq!(stores, 216);
}
#[test]
fn every_original_ordered_store_cold_resumes_with_exact_owner_and_backing_state() {
    let f = fixture();
    let mut cuts = 0;
    for row in f["cases"].as_array().unwrap() {
        let op = row["input"]["op"].as_str().unwrap();
        if !["put", "roundtrip"].contains(&op) || !row["completed"].as_bool().unwrap() {
            continue;
        }
        let maps = matrices(&f, row);
        let coord = at(row);
        let active = row["active"].as_i64().unwrap() as i32;
        let walls = maps[&format!("walls{}", coord.level)].clone();
        let vertices = maps[&format!("vertices{}", coord.level)].clone();
        let expanded = if op == "roundtrip" {
            get(&walls, active, coord).unwrap()
        } else {
            ExpandedWall::known(bytes(row["expanded_hex"].as_str().unwrap()))
        };
        let plan = plan_put(&walls, &vertices, active, coord, &expanded).unwrap();
        let baseline = completed(plan.clone(), walls.clone(), vertices.clone());
        for n in 0..=plan.writes().len() {
            let mut state = (plan.clone(), walls.clone(), vertices.clone());
            for _ in 0..n {
                assert!(state.0.step(&mut state.1, &mut state.2).unwrap().is_some());
            }
            let encoded = serde_json::to_vec(&state).unwrap();
            let cold: (Put, Matrix, Matrix) = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(state, cold);
            assert_eq!(
                completed(cold.0, cold.1, cold.2),
                baseline,
                "{}/cut{n}",
                row["input"]["name"]
            );
            cuts += 1;
        }
    }
    assert!(cuts > 216);
}
fn sample() -> (Matrix, Matrix, Coord) {
    let f = fixture();
    let row = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["name"] == "insert_cardinals")
        .unwrap();
    let mut maps = matrices(&f, row);
    (
        maps.remove("walls1").unwrap(),
        maps.remove("vertices1").unwrap(),
        at(row),
    )
}
#[test]
fn only_consumed_expanded_bits_are_required_and_preflight_never_publishes_partial_work() {
    let (walls, vertices, coord) = sample();
    let before = (walls.clone(), vertices.clone());
    let mut partial = ExpandedWall(vec![ObservedByte::unknown(); 44]);
    partial.0[0] = ObservedByte {
        value: 1,
        known: 63,
    };
    for i in 36..40 {
        partial.0[i] = 0.into();
    }
    partial.0[8] = 1.into();
    partial.0[9] = 2.into();
    partial.0[20] = 3.into();
    partial.0[21] = 4.into();
    partial.0[4] = ObservedByte { value: 2, known: 3 };
    let plan = plan_put(&walls, &vertices, 5, coord, &partial).unwrap();
    let mut full = ExpandedWall::known(vec![0; 44]);
    for i in [0, 4, 8, 9, 20, 21] {
        full.0[i] = ObservedByte::from(partial.0[i].value);
    }
    assert_eq!(plan, plan_put(&walls, &vertices, 5, coord, &full).unwrap());
    partial.0[8].known = 0;
    assert!(matches!(
        plan_put(&walls, &vertices, 5, coord, &partial),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!((walls, vertices), before);
}
#[test]
fn changed_allocation_columns_or_read_only_source_cells_cannot_publish_a_stale_plan() {
    let (walls, vertices, coord) = sample();
    let mut input = ExpandedWall::known(vec![0; 44]);
    input.0[0] = 15.into();
    let plan = plan_put(&walls, &vertices, 5, coord, &input).unwrap();
    let mut reused = walls.clone();
    let Allocation::Known(b) = &mut reused.allocation else {
        unreachable!()
    };
    b.token += 100;
    let mut v = vertices.clone();
    assert!(matches!(
        plan.clone().step(&mut reused, &mut v),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!(v, vertices);
    let mut moved = walls.clone();
    let Allocation::Known(b) = &mut moved.allocation else {
        unreachable!()
    };
    b.column_offsets[usize::from(coord.x)] += 1;
    assert!(matches!(
        plan.clone().step(&mut moved, &mut v),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!(v, vertices);
    let witness = plan
        .witnesses
        .iter()
        .find(|w| w.target == Target::Walls)
        .unwrap();
    let mut changed = walls.clone();
    saved_world::cell_mut(
        &mut changed,
        usize::from(witness.at.x),
        usize::from(witness.at.y),
        14,
    )
    .unwrap()[13] ^= 1;
    assert!(matches!(
        plan.clone().step(&mut changed, &mut v),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!(v, vertices);
}

#[test]
fn aliased_views_stop_before_the_call_instead_of_using_independent_overlay_semantics() {
    let (mut walls, mut vertices, coord) = sample();
    let mut input = ExpandedWall::known(vec![0; 44]);
    input.0[0] = 15.into();
    let original = (walls.clone(), vertices.clone());
    let owner = backing(&walls).unwrap().token;
    let Allocation::Known(b) = &mut vertices.allocation else {
        unreachable!()
    };
    b.token = owner;
    assert!(matches!(
        plan_put(&walls, &vertices, 5, coord, &input),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!(walls, original.0);
    vertices = original.1;
    let Allocation::Known(b) = &mut walls.allocation else {
        unreachable!()
    };
    b.column_offsets[3] = b.column_offsets[2];
    let before = (walls.clone(), vertices.clone());
    assert!(matches!(
        plan_put(&walls, &vertices, 5, coord, &input),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!((walls, vertices), before);
}

#[test]
fn rotation_gate_does_not_demand_unconsumed_grid_owners() {
    let (walls, _, coord) = sample();
    let mut input = ExpandedWall::known(vec![0; 44]);
    input.0[36] = 1.into();
    let unavailable = Matrix {
        width: 0,
        height: 0,
        stride: 0,
        allocation: Allocation::Unavailable,
    };
    assert!(
        matches!(plan_put(&walls, &unavailable, 5, coord, &input), Err(Stop::Unsupported(message)) if message.contains("rotation"))
    );
}
