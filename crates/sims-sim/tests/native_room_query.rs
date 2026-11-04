use sims_formats::lot::WallTile;
use sims_sim::{Point, lot_rooms::object_room};

#[test]
fn diagonal_membership_uses_ordered_neighbors_and_excludes_border() {
    let mut walls = vec![WallTile::default(); 25];
    walls[12].segments = 16;
    let mut rooms = vec![4; 25];
    rooms[12] = -5;
    rooms[7] = 7;
    rooms[11] = 11;
    rooms[17] = -5;
    rooms[13] = 13;
    assert_eq!(
        object_room(5, &walls, &rooms, Point::new(33, 33)).unwrap(),
        7
    );
    assert_eq!(
        object_room(5, &walls, &rooms, Point::new(40, 40)).unwrap(),
        13
    );
    assert_eq!(
        object_room(5, &walls, &rooms, Point::new(8, 40)).unwrap(),
        -5
    );
    rooms[7] = -5;
    assert_eq!(
        object_room(5, &walls, &rooms, Point::new(33, 33)).unwrap(),
        11
    );
}

#[test]
#[ignore = "requires private original-code room lookup oracle"]
fn original_object_room_queries() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/room-query.json");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for case in report["cases"].as_array().unwrap() {
        let input = &case["input"];
        let masks: Vec<u8> = serde_json::from_value(input["walls"].clone()).unwrap();
        let walls: Vec<_> = masks
            .into_iter()
            .map(|segments| WallTile {
                segments,
                ..Default::default()
            })
            .collect();
        let primary: Vec<i16> = serde_json::from_value(input["primary"].clone()).unwrap();
        for q in case["queries"].as_array().unwrap() {
            let p: [i32; 2] = serde_json::from_value(q["point"].clone()).unwrap();
            let result = object_room(
                input["size"].as_u64().unwrap() as usize,
                &walls,
                &primary,
                Point::new(p[0], p[1]),
            )
            .unwrap();
            assert_eq!(
                result,
                q["room"].as_i64().unwrap() as i16,
                "{} floor {} at {p:?}",
                case["source"],
                input["level"]
            );
        }
    }
}
