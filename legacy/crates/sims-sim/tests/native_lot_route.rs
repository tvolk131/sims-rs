use sims_formats::lot::WallTile;
use sims_sim::{
    lot_route::{self, Input, Prepared},
    regions::Region,
};

#[test]
fn water_access_and_room_access_are_independent() {
    let portal = lot_route::TileOccupant {
        kind: 8,
        flags: 0,
        function15: true,
        portal_class: true,
        partner_room: Some(7),
    };
    assert!(lot_route::tile_available(1, 8, 7, 255, &[portal.clone()]).unwrap());
    assert!(!lot_route::tile_available(1, 8, 7, 0, &[portal.clone()]).unwrap());
    let walkable = lot_route::TileOccupant {
        kind: 2,
        flags: 0x1000,
        ..portal.clone()
    };
    assert!(!lot_route::tile_available(1, 8, 7, 0, &[walkable.clone()]).unwrap());
    assert!(lot_route::tile_available(1, 8, 7, 0, &[portal.clone(), walkable]).unwrap());
    assert!(lot_route::tile_available(2, 8, 7, 0, &[portal]).unwrap());
    assert!(lot_route::tile_available(1, -5, 7, 255, &[]).unwrap());
}

#[test]
#[ignore = "requires private native tile eligibility oracle"]
fn original_tile_eligibility() {
    #[derive(serde::Deserialize)]
    struct Case {
        input: Tile,
        original: bool,
    }
    #[derive(serde::Deserialize)]
    struct Tile {
        level: i32,
        primary: i16,
        room: i16,
        water: u8,
        occupants: Vec<lot_route::TileOccupant>,
    }
    #[derive(serde::Deserialize)]
    struct Report {
        capture_sha256: String,
        cases: Vec<Case>,
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/lot-route-tile.json");
    let report: Report = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, Case { input: c, original }) in report.cases.into_iter().enumerate() {
        assert_eq!(
            lot_route::tile_available(c.level, c.primary, c.room, c.water, &c.occupants).unwrap(),
            original,
            "case {i}"
        );
    }
}

#[test]
fn diagonal_seeds_do_not_block_a_whole_tile_and_terrain_keeps_seed_cost() {
    let size = 5;
    let mut walls = vec![WallTile::default(); size * size];
    walls[12].segments = 16;
    walls[12].styles = [1, 3]; // The diagonal branch does not filter wall styles.
    let mut flags = vec![0; size * size];
    flags[6] = 32;
    let r = lot_route::prepare(Input {
        size,
        level: 1,
        room: 0,
        primary: &vec![0; 25],
        walls: &walls,
        coverings: &vec![1; 25],
        flags: &flags,
        available: &vec![true; 25],
        outside: true,
        terrain: true,
        ignore_marker: false,
    })
    .unwrap();
    assert_eq!(r.hard_count, 6);
    assert_eq!(
        &r.rectangles[4..6],
        &[
            Region([36, 44, 36, 44, i32::MAX]),
            Region([44, 36, 44, 36, i32::MAX])
        ]
    );
    assert_eq!(r.rectangles[6], Region([16, 16, 64, 64, 10]));
}

#[test]
fn unavailable_tiles_merge_and_split_only_room_has_explicit_boundary() {
    let walls = vec![WallTile::default(); 25];
    let c = |primary| Input {
        size: 5,
        level: 1,
        room: 0,
        primary,
        walls: &walls,
        coverings: &[0; 25],
        flags: &[0; 25],
        available: &[false; 25],
        outside: true,
        terrain: false,
        ignore_marker: false,
    };
    let r = lot_route::prepare(c(&[0; 25])).unwrap();
    assert_eq!(r.rectangles.len(), 5);
    assert_eq!(r.rectangles[4], Region([16, 16, 64, 64, i32::MAX]));
    assert_eq!(
        lot_route::prepare(c(&[-5; 25])).unwrap_err(),
        "room has no primary routing cell"
    );
}

#[test]
#[ignore = "requires private native pre-tail obstacle oracle"]
fn original_room_obstacle_generation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/lot-route-obstacles.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let masks: Vec<u8> = serde_json::from_value(c["walls"].clone()).unwrap();
        let styles: Vec<[u16; 2]> = serde_json::from_value(c["styles"].clone()).unwrap();
        let walls: Vec<_> = masks
            .into_iter()
            .zip(styles)
            .map(|(segments, styles)| WallTile {
                segments,
                styles,
                ..Default::default()
            })
            .collect();
        let actual = lot_route::prepare(Input {
            size: c["size"].as_u64().unwrap() as usize,
            level: c["level"].as_i64().unwrap() as i32,
            room: c["room"].as_i64().unwrap() as i16,
            primary: &serde_json::from_value::<Vec<i16>>(c["primary"].clone()).unwrap(),
            walls: &walls,
            coverings: &serde_json::from_value::<Vec<u16>>(c["floors"].clone()).unwrap(),
            flags: &serde_json::from_value::<Vec<u8>>(c["flags"].clone()).unwrap(),
            available: &serde_json::from_value::<Vec<bool>>(c["available"].clone()).unwrap(),
            outside: c["outside"].as_bool().unwrap(),
            terrain: c["terrain"].as_bool().unwrap(),
            ignore_marker: c["ignore_marker"].as_bool().unwrap(),
        })
        .unwrap();
        let expected: Prepared = serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual, expected, "case {i}");
    }
}
