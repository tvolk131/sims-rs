use sims_formats::lot::WallTile;
use sims_sim::lot_rooms::{Partition, partition};

#[test]
fn upper_floor_coverage_splits_rooms_without_walls() {
    let walls = vec![WallTile::default(); 16];
    let mut floors = vec![0; 16];
    floors[5] = 1;
    let ground = partition(4, 1, &walls, &floors, None).unwrap();
    assert_eq!(ground.primary, vec![0; 16]);
    let upper = partition(4, 2, &walls, &floors, None).unwrap();
    assert_eq!(upper.primary[5], 1025);
    assert_eq!(upper.primary[0], 1024);
}

#[test]
fn native_single_cell_scan_retains_an_unvisited_diagonal_half() {
    let mut walls = vec![WallTile::default(); 16];
    walls[5].segments = 16;
    let mut floors = vec![0; 16];
    floors[5] = 1;
    let result = partition(4, 2, &walls, &floors, None).unwrap();
    assert_eq!(result.primary[5], -5);
    assert_eq!(result.halves[0].rooms[&4], 1025);
    assert_eq!(result.halves[0].rooms[&2], -1);
    walls[4].segments = 1;
    assert!(
        partition(4, 2, &walls, &floors, None)
            .unwrap_err()
            .contains("asymmetric")
    );
}

#[test]
#[ignore = "requires private original-code room partition oracle"]
fn original_room_partition_membership_and_ids() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/room-partition.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let masks: Vec<u8> = serde_json::from_value(c["walls"].clone()).unwrap();
        let walls: Vec<_> = masks
            .into_iter()
            .map(|segments| WallTile {
                segments,
                ..Default::default()
            })
            .collect();
        let floors: Vec<u16> = serde_json::from_value(c["floors"].clone()).unwrap();
        let ground: Option<Vec<u8>> = c
            .get("ground_regions")
            .map(|v| serde_json::from_value(v.clone()).unwrap());
        let actual = partition(
            c["size"].as_u64().unwrap() as usize,
            c["level"].as_i64().unwrap() as i32,
            &walls,
            &floors,
            ground.as_deref(),
        )
        .unwrap();
        let expected: Partition = serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual, expected, "case {i}");
    }
}
