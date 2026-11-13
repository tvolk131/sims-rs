use super::*;
use crate::saved_matrix::Backing;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn matrix(size: usize, stride: usize, token: u64, fill: u8) -> Matrix {
    // Reversed columns, independent gaps, nonzero leading/trailing padding.
    let pitch = size * stride + 5;
    Matrix {
        width: size as i32,
        height: size as i32,
        stride: stride as i32,
        allocation: Allocation::Known(Backing {
            token,
            column_offsets: (0..size)
                .map(|x| (3 + (size - x - 1) * pitch) as u32)
                .collect(),
            data: vec![fill; 3 + size * pitch + 7],
        }),
    }
}
fn vertices(size: usize) -> [Matrix; 2] {
    [matrix(size, 1, 101, 0xa7), matrix(size, 1, 102, 0xb8)]
}
fn walls(size: usize) -> [Matrix; 2] {
    [matrix(size, 14, 201, 0x3f), matrix(size, 14, 202, 0x2f)]
}
fn cold<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn set(m: &mut Matrix, x: usize, y: usize, value: u8) {
    let stride = m.stride as usize;
    saved_world::cell_mut(m, x, y, stride).unwrap()[0] = value;
}
fn byte(m: &Matrix, x: usize, y: usize) -> u8 {
    saved_world::cell(m, x, y, m.stride as usize).unwrap()[0]
}
fn views(m: &[Matrix; 2]) -> [&Matrix; 2] {
    [&m[0], &m[1]]
}
fn expected_vertex(m: &Matrix, active: usize, x: usize, y: usize) -> u8 {
    // Independent literal contribution table from the bound479c58 controls.
    let mut out = 0;
    for (xx, yy, contributions) in [
        (x as i32 - 1, y as i32 - 1, [(8, 2), (32, 16)]),
        (x as i32, y as i32 - 1, [(1, 1), (16, 128)]),
        (x as i32 - 1, y as i32, [(4, 4), (16, 32)]),
        (x as i32, y as i32, [(2, 8), (32, 64)]),
    ] {
        if xx >= 1 && yy >= 1 && xx < active as i32 - 1 && yy < active as i32 - 1 {
            for (segment, vertex) in contributions {
                if byte(m, xx as usize, yy as usize) & segment != 0 {
                    out |= vertex;
                }
            }
        }
    }
    out
}
fn run_borders(
    mut cursor: ShrinkBorders,
    mut maps: [Matrix; 2],
) -> (ShrinkBorders, [Matrix; 2], Vec<Write>) {
    let mut writes = Vec::new();
    while let Some(w) = cursor.step(&mut maps).unwrap() {
        writes.push(w);
    }
    (cursor, maps, writes)
}
fn run_rebuild(
    mut cursor: Rebuild,
    walls: &[Matrix; 2],
    mut maps: [Matrix; 2],
) -> (Rebuild, [Matrix; 2], Vec<Write>) {
    let mut writes = Vec::new();
    while let Some(w) = cursor.step(views(walls), &mut maps).unwrap() {
        writes.push(w);
    }
    (cursor, maps, writes)
}

#[test]
fn shrink_exact_source_store_order_retains_all_other_bytes_and_layouts() {
    let input = vertices(6);
    let mut expected = input.clone();
    let mut tape = Vec::new();
    for coordinate in 0..4 {
        for level in 1..=2 {
            set(&mut expected[level - 1], 3, coordinate, 0);
            tape.push(Write {
                at: Coord {
                    x: 3,
                    y: coordinate as u8,
                    level: level as u8,
                },
                value: 0,
                instruction: 0x477a4b,
            });
        }
    }
    for coordinate in 0..4 {
        for level in 1..=2 {
            set(&mut expected[level - 1], coordinate, 3, 0);
            tape.push(Write {
                at: Coord {
                    x: coordinate as u8,
                    y: 3,
                    level: level as u8,
                },
                value: 0,
                instruction: 0x477bb6,
            });
        }
    }
    let (cursor, actual, writes) = run_borders(ShrinkBorders::new(6, 4).unwrap(), input);
    assert!(cursor.completed());
    assert_eq!(actual, expected);
    assert_eq!(writes, tape);
    assert_eq!(writes.len(), 16); // Corner is intentionally written twice per level.
}

#[test]
fn rebuild_nonzero_diagonals_active_extent_store_order_and_inactive_preservation() {
    let mut raw = walls(6);
    // Poison border/inactive walls. Only the two interior coordinates per axis
    // are eligible sources; allocation dimensions must not broaden that band.
    for (level, m) in raw.iter_mut().enumerate() {
        for x in 1..3 {
            for y in 1..3 {
                set(m, x, y, ((x * 13 + y * 9 + level * 7) & 63) as u8);
            }
        }
    }
    let unchanged_raw = raw.clone();
    let input = vertices(6);
    let mut expected = input.clone();
    let mut tape = Vec::new();
    for level in 1..=2 {
        for x in 0..4 {
            for y in 0..4 {
                let value = expected_vertex(&raw[level - 1], 4, x, y);
                set(&mut expected[level - 1], x, y, value);
                tape.push(Write {
                    at: Coord {
                        x: x as u8,
                        y: y as u8,
                        level: level as u8,
                    },
                    value,
                    instruction: 0x479f72,
                });
            }
        }
    }
    let (cursor, actual, writes) = run_rebuild(Rebuild::new(4).unwrap(), &raw, input);
    assert!(cursor.completed());
    assert!(writes.iter().any(|w| w.value != 0));
    assert_eq!(actual, expected);
    assert_eq!(writes, tape);
    assert_eq!(raw, unchanged_raw);
}

#[test]
fn rebuild_borrows_existing_floor_fields_without_moving_their_matrix_owners() {
    let absent = || Matrix {
        width: 0,
        height: 0,
        stride: 0,
        allocation: Allocation::Unavailable,
    };
    let floor = |wall| saved_world::Floor {
        covering: absent(),
        wall,
        head: absent(),
        flags: absent(),
        primary: absent(),
        lights: saved_world::Lights {
            used: 0,
            capacity: 0,
            values: Vec::new(),
        },
    };
    let [first, second] = walls(6);
    let floors = [floor(first), floor(second)];
    let unchanged = floors.clone();
    let mut maps = vertices(6);
    let mut cursor = Rebuild::new(4).unwrap();
    let mut writes = 0;
    while cursor
        .step([&floors[0].wall, &floors[1].wall], &mut maps)
        .unwrap()
        .is_some()
    {
        writes += 1;
    }
    assert_eq!(writes, 32);
    assert_eq!(floors, unchanged);
}

#[test]
fn all_authored_store_cold_cuts_preserve_cursor_inputs_outputs_and_order() {
    let input = vertices(6);
    let start = ShrinkBorders::new(6, 4).unwrap();
    let reference = run_borders(start.clone(), input.clone());
    for cut in 0..=reference.2.len() {
        let (mut cursor, mut maps) = (start.clone(), input.clone());
        for _ in 0..cut {
            cursor.step(&mut maps).unwrap().unwrap();
        }
        let (cursor, maps) = cold(&(cursor, maps));
        let after = run_borders(cursor, maps);
        assert_eq!(after.0, reference.0, "border cursor cut{cut}");
        assert_eq!(after.1, reference.1, "border owners cut{cut}");
        assert_eq!(after.2, reference.2[cut..], "border tape cut{cut}");
    }
    // The real construction barrier remains outside either cursor. This
    // authored source change between them is visible to the new rebuild.
    let mut raw = walls(6);
    set(&mut raw[0], 1, 1, 0x32);
    let start = Rebuild::new(4).unwrap();
    let input = reference.1;
    let reference = run_rebuild(start.clone(), &raw, input.clone());
    for cut in 0..=reference.2.len() {
        let (mut cursor, mut maps) = (start.clone(), input.clone());
        for _ in 0..cut {
            cursor.step(views(&raw), &mut maps).unwrap().unwrap();
        }
        let (cursor, raw, maps) = cold(&(cursor, raw.clone(), maps));
        let after = run_rebuild(cursor, &raw, maps);
        assert_eq!(after.0, reference.0, "rebuild cursor cut{cut}");
        assert_eq!(after.1, reference.1, "rebuild owners cut{cut}");
        assert_eq!(after.2, reference.2[cut..], "rebuild tape cut{cut}");
    }
}

#[test]
fn unavailable_owner_is_a_cold_retryable_host_stop_not_a_native_status() {
    let input = vertices(6);
    let mut maps = input.clone();
    let restored = maps[1].allocation.clone();
    maps[1].allocation = Allocation::Unavailable;
    let mut cursor = ShrinkBorders::new(6, 4).unwrap();
    let before = maps.clone();
    assert!(matches!(cursor.step(&mut maps), Err(Stop::Unavailable(_))));
    assert_eq!(maps, before);
    let (mut cursor, mut maps) = cold(&(cursor, maps));
    // An actual host reply supplies the authored owner, never a default/filler.
    maps[1].allocation = restored;
    let after = run_borders(cursor.clone(), maps.clone());
    let expected = run_borders(ShrinkBorders::new(6, 4).unwrap(), input);
    assert_eq!(after, expected);

    let mut raw = walls(6);
    let restored = raw[1].allocation.clone();
    raw[1].allocation = Allocation::Unavailable;
    let mut rebuild = Rebuild::new(4).unwrap();
    let before = maps.clone();
    assert!(matches!(
        rebuild.step(views(&raw), &mut maps),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(maps, before);
    let (mut rebuild, mut raw, mut maps) = cold(&(rebuild, raw, maps));
    raw[1].allocation = restored;
    assert!(rebuild.step(views(&raw), &mut maps).unwrap().is_some());
    // Keep the border cursor independently resumable too.
    assert!(cursor.step(&mut before.clone()).unwrap().is_some());
}

#[test]
fn changed_lifetime_columns_or_header_cannot_retarget_a_cold_cursor() {
    for mutation in 0..4 {
        let mut maps = vertices(6);
        let mut cursor = ShrinkBorders::new(6, 4).unwrap();
        cursor.step(&mut maps).unwrap().unwrap();
        let (mut cursor, mut maps) = cold(&(cursor, maps));
        match mutation {
            0 => {
                if let Allocation::Known(b) = &mut maps[1].allocation {
                    b.token += 1000;
                }
            }
            1 => {
                if let Allocation::Known(b) = &mut maps[0].allocation {
                    b.column_offsets.swap(0, 1);
                }
            }
            2 => maps[0].width += 1,
            _ => {
                if let Allocation::Known(b) = &mut maps[1].allocation {
                    b.data.push(99);
                }
            }
        }
        let before = maps.clone();
        assert!(matches!(cursor.step(&mut maps), Err(Stop::Unsupported(_))));
        assert_eq!(maps, before);
    }
    let mut raw = walls(6);
    let mut maps = vertices(6);
    let mut cursor = Rebuild::new(4).unwrap();
    cursor.step(views(&raw), &mut maps).unwrap().unwrap();
    if let Allocation::Known(b) = &mut raw[1].allocation {
        b.token += 1000;
    }
    let before = maps.clone();
    assert!(matches!(
        cursor.step(views(&raw), &mut maps),
        Err(Stop::Unsupported(_))
    ));
    assert_eq!(maps, before);
}

#[test]
fn empty_and_equal_operations_do_not_demand_unused_owners_and_invalid_profiles_fail() {
    let absent = Matrix {
        width: 0,
        height: 0,
        stride: 1,
        allocation: Allocation::Unavailable,
    };
    let mut maps = [absent.clone(), absent.clone()];
    assert_eq!(
        ShrinkBorders::new(52, 52).unwrap().step(&mut maps).unwrap(),
        None
    );
    assert_eq!(
        Rebuild::new(0)
            .unwrap()
            .step([&absent, &absent], &mut maps)
            .unwrap(),
        None
    );
    for active in [1, 2] {
        let (cursor, result, writes) = run_rebuild(
            Rebuild::new(active).unwrap(),
            &[absent.clone(), absent.clone()],
            vertices(4),
        );
        assert!(cursor.completed());
        assert_eq!(writes.len(), active as usize * active as usize * 2);
        for m in &result {
            assert_eq!(byte(m, 0, 0), 0);
        }
    }
    assert!(ShrinkBorders::new(52, 64).is_err());
    assert!(ShrinkBorders::new(64, 3).is_err());
    assert!(Rebuild::new(128).is_err());
    let mut maps = vertices(4);
    if let Allocation::Known(b) = &mut maps[1].allocation {
        b.token = 101;
    }
    assert!(matches!(
        ShrinkBorders::new(6, 4).unwrap().step(&mut maps),
        Err(Stop::Unsupported(_))
    ));
}

fn bytes(s: &str) -> Vec<u8> {
    let (chunks, remainder) = s.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    chunks
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn same_return_wall_segments_agree_with_only_observed_active_vertex_bytes() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/native-wall-vertices-partial.json"
    ))
    .unwrap();
    assert!(fixture["level2_vertex_values"].is_null());
    let active = fixture["active"].as_u64().unwrap() as usize;
    let allocated = fixture["allocated"].as_u64().unwrap() as usize;
    let mut raw = walls(allocated);
    for (m, runs) in raw
        .iter_mut()
        .zip(fixture["wall_segment_runs"].as_array().unwrap())
    {
        let mut index = 0;
        for run in runs.as_array().unwrap() {
            for _ in 0..run["count"].as_u64().unwrap() {
                set(
                    m,
                    index / active,
                    index % active,
                    run["value"].as_u64().unwrap() as u8,
                );
                index += 1;
            }
        }
        assert_eq!(index, active * active);
    }
    // Destinations, padding and unread wall fields are deliberately authored
    // poison, not claimed original input. No missing second-level byte is
    // filled from another epoch. This checks the helper at one observed cut.
    let input = vertices(allocated);
    let (_, result, writes) =
        run_rebuild(Rebuild::new(active as i32).unwrap(), &raw, input.clone());
    assert_eq!(writes.len(), active * active * 2);
    let mut compared = 0;
    for run in fixture["level1_vertex_runs"].as_array().unwrap() {
        let x = run["x"].as_u64().unwrap() as usize;
        let y = run["y"].as_u64().unwrap() as usize;
        for (dy, expected) in bytes(run["raw"].as_str().unwrap()).into_iter().enumerate() {
            if x < active && y + dy < active {
                assert_eq!(
                    byte(&result[0], x, y + dy),
                    expected,
                    "native+4c/{x}/{}",
                    y + dy
                );
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 1176);
    for (before, after) in input.iter().zip(&result) {
        for x in 0..allocated {
            for y in 0..allocated {
                if x >= active || y >= active {
                    assert_eq!(byte(before, x, y), byte(after, x, y));
                }
            }
        }
    }
}
