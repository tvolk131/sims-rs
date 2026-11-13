use sims_formats::lot::WallTile;
use sims_sim::lot_flags::{Input, Rebuilt, rebuild};

#[test]
fn enclosed_room_and_fence_have_different_exterior_flags() {
    let mut walls = vec![WallTile::default(); 16];
    for (i, bits) in [(10, 15), (6, 4), (9, 8), (14, 1), (11, 2)] {
        walls[i].segments = bits;
        walls[i].styles = [1, 1];
    }
    let flags = vec![0; 16];
    let heights = vec![[0; 4]; 16];
    let floors = vec![0; 16];
    let input = |walls| Input {
        size: 4,
        level: 1,
        walls,
        floors: &floors,
        object_heads: &floors,
        heights: &heights,
        saved_flags: &flags,
        below: None,
    };
    let solid = rebuild(input(&walls)).unwrap();
    assert_eq!(solid.flags[10], 1);
    assert_eq!(solid.flags[0], 6);
    let mut fence_walls = walls.clone();
    for w in &mut fence_walls {
        w.styles = [2, 2];
    }
    let fence = rebuild(input(&fence_walls)).unwrap();
    assert_eq!(fence.flags[10], 7);
}

#[test]
fn upper_build_support_does_not_require_a_floor_to_exist() {
    let walls = vec![WallTile::default(); 16];
    let floors = vec![0; 16];
    let mut flags = vec![0; 16];
    flags[0] = 255;
    let heights = vec![[0; 4]; 16];
    let below = vec![0; 16];
    let actual = rebuild(Input {
        size: 4,
        level: 2,
        walls: &walls,
        floors: &floors,
        object_heads: &floors,
        heights: &heights,
        saved_flags: &flags,
        below: Some(&below),
    })
    .unwrap();
    assert_eq!(
        actual.flags[0], 46,
        "native keeps marker bits 0x28 and clears border buildability"
    );
    assert!(actual.upper_support);
    assert!(!actual.upper_contents);
    assert_eq!(actual.flags[5] & 1, 1);
}

#[test]
#[ignore = "requires private original-code lot flag oracle"]
fn original_exterior_and_build_support_flags() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/lot-flags.json");
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
        let floors: Vec<u16> = serde_json::from_value(c["floors"].clone()).unwrap();
        let heights: Vec<[u8; 4]> = serde_json::from_value(c["heights"].clone()).unwrap();
        let flags: Vec<u8> = serde_json::from_value(c["flags"].clone()).unwrap();
        let below: Vec<u8> = serde_json::from_value(c["below"].clone()).unwrap();
        let actual = rebuild(Input {
            size: c["size"].as_u64().unwrap() as usize,
            level: c["level"].as_i64().unwrap() as i32,
            walls: &walls,
            floors: &floors,
            object_heads: &vec![0; floors.len()],
            heights: &heights,
            saved_flags: &flags,
            below: Some(&below),
        })
        .unwrap();
        let expected: Rebuilt = serde_json::from_value(row["original"].clone()).unwrap();
        assert_eq!(actual, expected, "case {i}");
    }
}
