use serde_json::Value;
use sims_formats::lot::WallTile;
use sims_sim::{
    Point, lot_flags,
    lot_geometry::{FloorSource, Lot},
    lot_rooms,
};

fn walls(c: &Value) -> Vec<WallTile> {
    let masks: Vec<u8> = serde_json::from_value(c["walls"].clone()).unwrap();
    let styles: Vec<[u16; 2]> = serde_json::from_value(c["styles"].clone()).unwrap();
    masks
        .into_iter()
        .zip(styles)
        .map(|(segments, styles)| WallTile {
            segments,
            styles,
            ..Default::default()
        })
        .collect()
}

fn compare(report: Value) {
    for (number, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let size = c["size"].as_u64().unwrap() as usize;
        let active = c["active_size"].as_u64().unwrap() as usize;
        let level = c["level"].as_i64().unwrap() as i32;
        let walls = walls(c);
        let floors: Vec<u16> = serde_json::from_value(c["floors"].clone()).unwrap();
        let flags: Vec<u8> = serde_json::from_value(c["flags"].clone()).unwrap();
        let heads: Vec<u16> = serde_json::from_value(c["object_heads"].clone()).unwrap();
        let heights: Vec<[u8; 4]> = serde_json::from_value(c["heights"].clone()).unwrap();
        let below: Vec<u8> = serde_json::from_value(c["below"].clone()).unwrap();
        let regions: Vec<u8> = serde_json::from_value(c["ground_regions"].clone()).unwrap();
        let actual = lot_flags::rebuild_in_active(
            lot_flags::Input {
                size,
                level,
                walls: &walls,
                floors: &floors,
                object_heads: &heads,
                heights: &heights,
                saved_flags: &flags,
                below: Some(&below),
            },
            active,
        )
        .unwrap();
        let expected: lot_flags::Rebuilt = serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual, expected, "flags case{number}");
        let rooms =
            lot_rooms::partition_in_active(size, active, level, &walls, &floors, Some(&regions))
                .unwrap();
        assert_eq!(
            serde_json::to_value(&rooms).unwrap(),
            row["original"]["rooms"],
            "rooms case{number}"
        );
        for x in 0..size {
            for y in 0..size {
                let i = x * size + y;
                if x >= active || y >= active {
                    assert_eq!(rooms.primary[i], -1);
                    assert_eq!(actual.flags[i], flags[i] & 0x28);
                }
            }
        }
        if level == 1 {
            let lot = Lot::rebuild_in_active(
                size,
                active,
                heights,
                regions,
                vec![255; size * size],
                vec![FloorSource {
                    coverings: floors,
                    walls,
                    object_heads: heads,
                    flags,
                }],
            )
            .unwrap();
            assert_eq!(lot.floors[0].rooms, rooms);
            assert_eq!(lot.floors[0].flags, actual);
            lot.validate().unwrap();
            let restored: Lot = serde_json::from_slice(&serde_json::to_vec(&lot).unwrap()).unwrap();
            assert_eq!(lot, restored);
            restored.validate().unwrap();
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn active_and_backing_extents_match_full_native_room_and_flag_arrays() {
    let report = serde_json::from_str(include_str!("fixtures/active-lot-geometry.json")).unwrap();
    compare(report);
}

#[test]
#[ignore = "requires private asset-derived active geometry oracle"]
fn all_three_original_house_geometries_match_native_active_and_full_extent() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/native-active-geometry-20261023-first/native-active-flags-first.json");
    let report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    compare(report);
}

#[test]
fn legacy_geometry_does_not_gain_an_active_extent_and_new_bounds_use_it() {
    let source = FloorSource {
        coverings: vec![0; 64],
        walls: vec![WallTile::default(); 64],
        object_heads: vec![0; 64],
        flags: vec![0; 64],
    };
    let historical = Lot::rebuild(
        8,
        vec![[0; 4]; 64],
        vec![255; 64],
        vec![255; 64],
        vec![source.clone()],
    )
    .unwrap();
    let encoded = serde_json::to_value(&historical).unwrap();
    assert!(encoded.get("active_size").is_none());
    let restored: Lot = serde_json::from_value(encoded.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), encoded);
    let active = Lot::rebuild_in_active(
        8,
        4,
        historical.heights.clone(),
        historical.pool.clone(),
        historical.water.clone(),
        vec![source],
    )
    .unwrap();
    let f = &active.floors[0];
    assert_eq!(
        lot_rooms::object_room_in_active(
            8,
            4,
            &f.source.walls,
            &f.rooms.primary,
            Point::new(40, 40)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        lot_rooms::object_room_in_active(
            8,
            4,
            &f.source.walls,
            &f.rooms.primary,
            Point::new(56, 40)
        )
        .unwrap(),
        -5
    );
    assert_eq!(historical.floors[0].flags.flags[6 * 8 + 6] & 1, 1);
    assert_eq!(f.flags.flags[6 * 8 + 6], 0);
    let mut invalid = active.clone();
    invalid.active_size = Some(8);
    assert!(invalid.validate().is_err());
    for bad in [0, 1, 9] {
        assert!(
            Lot::rebuild_in_active(
                8,
                bad,
                active.heights.clone(),
                active.pool.clone(),
                active.water.clone(),
                vec![active.floors[0].source.clone()]
            )
            .is_err()
        );
    }
}
