use serde_json::{Value, json};
use sims_vm::{
    Object, World,
    autonomy::Placement,
    loading::{coarse_position, repair_ground_object_heads, repair_ground_object_heads_in_active},
};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires private original-code grid repair oracle"]
fn loaded_grid_repair_matches_original_bytes_and_floor_scope() {
    let report: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../cache/research/sims-exe/oracle/load-grid.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["positions"].as_array().unwrap() {
        let [x, y, level]: [i32; 3] = serde_json::from_value(row["input"].clone()).unwrap();
        assert_eq!(
            json!(coarse_position(Placement { x, y, level })),
            row["original"]
        );
    }
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut world = World::default();
        for o in c["objects"].as_array().unwrap() {
            let id = o["id"].as_i64().unwrap() as i16;
            let [column, row, level]: [i32; 3] =
                serde_json::from_value(o["coarse"].clone()).unwrap();
            world.objects.insert(id, Object::default());
            world.positions.insert(
                id,
                Placement {
                    x: row * 16,
                    y: column * 16,
                    level,
                },
            );
        }
        let mut heads: Vec<Vec<u16>> = serde_json::from_value(c["heads"].clone()).unwrap();
        repair_ground_object_heads(c["size"].as_u64().unwrap() as usize, &mut heads[0], &world)
            .unwrap();
        assert_eq!(json!(heads), row["original"], "{c}");
    }
}

#[test]
fn a_recycled_offlot_id_is_not_a_valid_grid_head() {
    let mut world = World::default();
    world.objects.insert(1, Object::default());
    world.positions.insert(
        1,
        Placement {
            x: -16,
            y: -16,
            level: 1,
        },
    );
    let mut heads = [0, 1, 0, 2];
    let repairs = repair_ground_object_heads(2, &mut heads, &world).unwrap();
    assert_eq!(heads, [0; 4]);
    assert_eq!(repairs[0].location, Some([-1, -1, 1]));
    assert_eq!(repairs[1].location, None);
    assert_eq!(repairs[0].tile, [0, 1]);
}

fn active_cases(report: Value) {
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut world = World::default();
        for object in c["objects"].as_array().unwrap() {
            let id = object["id"].as_i64().unwrap() as i16;
            let [column, row, level]: [i32; 3] =
                serde_json::from_value(object["coarse"].clone()).unwrap();
            world.objects.insert(id, Object::default());
            world.positions.insert(
                id,
                Placement {
                    x: row * 16,
                    y: column * 16,
                    level,
                },
            );
        }
        let before = serde_json::to_value(&world).unwrap();
        let size = c["size"].as_u64().unwrap() as usize;
        let active = c["active_size"].as_u64().unwrap() as usize;
        let mut heads: Vec<Vec<u16>> = serde_json::from_value(c["heads"].clone()).unwrap();
        let repairs =
            repair_ground_object_heads_in_active(size, active, &mut heads[0], &world).unwrap();
        assert_eq!(json!(heads), row["original"], "size{size} active{active}");
        assert!(
            repairs
                .iter()
                .all(|r| r.tile[0] < active && r.tile[1] < active)
        );
        assert_eq!(serde_json::to_value(&world).unwrap(), before);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_active_extent_preserves_full_backing_head_payload() {
    active_cases(serde_json::from_str(include_str!("fixtures/active-load-grid.json")).unwrap());
}

#[test]
#[ignore = "requires private active grid-repair oracle and baseline witness"]
fn private_active_grid_cases_and_original_baseline_match() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/native-active-geometry-20261023-first/native-active-grid-first.json");
    active_cases(serde_json::from_slice(&fs::read(path).unwrap()).unwrap());
}

#[test]
fn active_repair_does_not_read_inactive_objects_and_keeps_failure_atomicity() {
    let mut world = World::default();
    world.objects.insert(1, Object::default());
    //Object1 deliberately has no position: inactive cells must not read it.
    let mut heads = vec![0; 64];
    heads[7 * 8 + 7] = 1;
    heads[1] = 65535;
    let repairs = repair_ground_object_heads_in_active(8, 4, &mut heads, &world).unwrap();
    assert_eq!(heads[63], 1);
    assert_eq!(heads[1], 0);
    assert_eq!(repairs.len(), 1);
    heads[1] = 65535;
    heads[2] = 1;
    let before = heads.clone();
    assert_eq!(
        repair_ground_object_heads_in_active(8, 4, &mut heads, &world),
        Err("registered object has no physical position".into())
    );
    assert_eq!(heads, before);
    for active in [0, 9] {
        assert!(repair_ground_object_heads_in_active(8, active, &mut heads, &world).is_err());
        assert_eq!(heads, before);
    }
}
