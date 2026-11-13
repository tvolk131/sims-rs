use sims_formats::lot::WallTile;
use sims_sim::{Point, Rect, lot_walls::Grid};

fn compare_fit(report: serde_json::Value) {
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let walls: Vec<WallTile> = serde_json::from_value(c["walls"].clone()).unwrap();
        let expanded = Grid::new(4, &walls).unwrap().expanded(1, 1).unwrap();
        let result = sims_sim::lot_walls::placement_fit(
            &expanded,
            c["facing"].as_i64().unwrap() as i16,
            c["mask"].as_u64().unwrap() as u16,
            c["error"].as_i64().unwrap() as i32,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            row["original"],
            "wall fit case {i}: {c}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_wall_fit_requirements_and_rotation() {
    compare_fit(serde_json::from_str(include_str!("fixtures/wall-fit.json")).unwrap());
}
#[test]
#[ignore = "requires private original wall-fit report"]
fn all_original_wall_fit_requirements() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/wall-fit.json");
    compare_fit(serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap());
}

#[test]
fn diagonal_contact_styles_and_mixed_mask_iteration() {
    let mut tiles = vec![WallTile::default(); 16];
    let rect = Rect {
        min: Point::new(20, 20),
        max: Point::new(28, 28),
    };
    for mask in [16, 32] {
        tiles[5].segments = mask;
        assert!(Grid::new(4, &tiles).unwrap().blocks_bounds(rect).unwrap());
        tiles[5].styles[1] = 3;
        assert!(!Grid::new(4, &tiles).unwrap().blocks_bounds(rect).unwrap());
        tiles[5].styles[1] = 0;
    }
    tiles[5].segments = 33;
    assert!(
        !Grid::new(4, &tiles).unwrap().blocks_bounds(rect).unwrap(),
        "native skips second diagonal when it is not the first set bit"
    );
    tiles[5].segments = 16;
    let touches = Rect {
        min: Point::new(16, 16),
        max: Point::new(24, 24),
    };
    assert!(
        !Grid::new(4, &tiles)
            .unwrap()
            .blocks_bounds(touches)
            .unwrap()
    );
    let segments: Vec<_> = Grid::new(4, &tiles).unwrap().owned_segments().collect();
    assert_eq!(segments, vec![(Point::new(32, 16), Point::new(16, 32), 0)]);
}

#[test]
#[ignore = "requires private original-code wall geometry oracle"]
fn original_expanded_walls_and_rectangles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/lot-walls.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["expansion"].as_array().unwrap() {
        let c = &row["input"];
        let tiles: Vec<WallTile> = serde_json::from_value(c["tiles"].clone()).unwrap();
        let g = Grid::new(c["size"].as_u64().unwrap() as usize, &tiles).unwrap();
        let p: [usize; 2] = serde_json::from_value(c["point"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(g.expanded(p[0], p[1]).unwrap()).unwrap(),
            row["original"]
        );
    }
    for (i, row) in report["collisions"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let tiles: Vec<WallTile> = serde_json::from_value(c["tiles"].clone()).unwrap();
        let g = Grid::new(c["size"].as_u64().unwrap() as usize, &tiles).unwrap();
        let b: [i32; 4] = serde_json::from_value(c["bounds"].clone()).unwrap();
        let result = g
            .blocks_bounds(Rect {
                min: Point::new(b[0], b[1]),
                max: Point::new(b[2], b[3]),
            })
            .unwrap();
        assert_eq!(
            result,
            row["original"].as_bool().unwrap(),
            "case {i}: {b:?}"
        );
    }
}
