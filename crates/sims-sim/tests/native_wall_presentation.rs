use serde_json::{Value, json};
use sims_sim::lot_presenter::walls::{self, Event, State};

fn check_grids(report: &Value) {
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let masks: Vec<u8> = serde_json::from_value(c["walls"].clone()).unwrap();
        let cells: Vec<_> = masks
            .into_iter()
            .map(|segments| sims_formats::lot::WallTile {
                segments,
                ..Default::default()
            })
            .collect();
        let result = walls::grid::corners(c["size"].as_u64().unwrap() as usize, &cells).unwrap();
        assert_eq!(json!(result), row["original"], "grid {n}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_corner_grid_and_all_initialized_tables() {
    let report: Value = serde_json::from_str(include_str!("fixtures/wall-grid.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 288);
    check_grids(&report);
    for mask in 0..=255 {
        let (junction, endpoint) = walls::grid::classification(mask);
        assert_eq!(
            json!(u8::from(junction)),
            report["tables"]["junction"][mask as usize]
        );
        assert_eq!(
            json!(u8::from(endpoint)),
            report["tables"]["endpoint"][mask as usize]
        );
        assert_eq!(walls::grid::rotate(mask, 0).unwrap(), mask);
        for rotation in 1..=3 {
            assert_eq!(
                json!(walls::grid::rotate(mask, rotation).unwrap()),
                report["tables"]["rotations"][rotation as usize - 1][mask as usize]
            );
        }
    }
}

#[test]
#[ignore = "requires private original-house native corner-grid evidence"]
fn native_corner_grids_from_original_house_rebuilds() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-wall-grid.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 6);
    check_grids(&report);
}

fn initialized_caps(state: &mut State) {
    for corner in &mut state.corners {
        for (valid, cap) in corner.valid.iter().zip(&mut corner.caps) {
            if *valid == 0 {
                *cap = [0; 20];
            } else {
                cap[3] = 0;
                cap[17..].fill(0);
            }
        }
    }
}

fn check_construction(report: &Value) {
    use sims_sim::{lot_presenter::walls::construction::Graph, lot_rooms::Partition};
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let cells =
            serde_json::from_value::<Vec<sims_formats::lot::WallTile>>(c["walls"].clone()).unwrap();
        let rooms = Partition {
            primary: serde_json::from_value(c["rooms"]["primary"].clone()).unwrap(),
            halves: serde_json::from_value(c["rooms"]["halves"].clone()).unwrap(),
            room_tiles: serde_json::from_value(c["rooms"]["room_tiles"].clone()).unwrap(),
        };
        let mut graph = Graph::floor(
            c["size"].as_u64().unwrap() as usize,
            c["level"].as_i64().unwrap() as i32,
            &cells,
            &rooms,
        )
        .unwrap_or_else(|e| panic!("graph {n}: {e}"));
        assert_eq!(
            json!(graph.geometry),
            row["original"]["geometry"],
            "geometry {n}"
        );
        let mut expected: State =
            serde_json::from_value(row["original"]["before"].clone()).unwrap();
        initialized_caps(&mut expected);
        assert_eq!(graph.state, expected, "constructed graph {n}");
        graph
            .state
            .update(
                c["enabled"].as_u64().unwrap() as u8,
                c["level"].as_i64().unwrap() as i32,
                walls::BASIS,
                |_| {},
            )
            .unwrap();
        let mut expected: State = serde_json::from_value(row["original"]["after"].clone()).unwrap();
        initialized_caps(&mut expected);
        assert_eq!(graph.state, expected, "updated graph {n}");
        let restored: Graph = serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
        assert_eq!(graph, restored);
        restored.state.validate().unwrap();
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_wall_construction_segment_walk_order_and_patterns() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/wall-construction.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 58);
    check_construction(&report);
}

#[test]
#[ignore = "requires private original-house native wall-construction evidence"]
fn native_original_house_wall_construction() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-wall-construction.json");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 12);
    check_construction(&report);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_floor_passes_corner_flags_caps_and_padding() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/wall-presentation.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["caps"].as_array().unwrap().len(), 1024);
    assert_eq!(
        json!(walls::BASIS),
        json!([report["directions"][0], report["directions"][2]])
    );
    for row in report["caps"].as_array().unwrap() {
        let c = &row["input"];
        let mut record: [u8; 20] = serde_json::from_value(c["before"].clone()).unwrap();
        let basis = serde_json::from_value(c["basis"].clone()).unwrap();
        let valid = walls::cap(
            c["mask"].as_u64().unwrap() as u8,
            c["excluded"].as_u64().unwrap() as u8,
            c["included"].as_u64().unwrap() as u8,
            basis,
            &mut record,
        );
        assert_eq!(
            json!({"valid":u8::from(valid),"record":record}),
            row["original"],
            "cap {c}"
        );
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 194);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let basis = serde_json::from_value(c["basis"].clone()).unwrap();
        let mut events = vec![];
        state
            .update(
                c["enabled"].as_u64().unwrap() as u8,
                c["level"].as_i64().unwrap() as i32,
                basis,
                |event| {
                    events.push(match event {
                        Event::Edge(id) => json!(["0x53d76a", id]),
                        Event::Corner(id) => json!(["0x53ca67", id]),
                        Event::Caps(id) => json!(["0x53cc00", id]),
                    })
                },
            )
            .unwrap();
        assert_eq!(
            json!({"state":state,"events":events}),
            row["original"],
            "graph {n}"
        );
        let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored, state);
    }
}
