use serde_json::Value;
use sims_sim::lot_presenter::walls::construction::Graph;

fn compare(report: Value, histories: usize) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), histories);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let mut graph: Graph = serde_json::from_value(row["input"].clone()).unwrap();
        graph.validate().unwrap();
        assert_eq!(row["steps"].as_array().unwrap().len(), 16);
        for (i, step) in row["steps"].as_array().unwrap().iter().enumerate() {
            graph
                .rotate(step["rotation"].as_u64().unwrap() as u8)
                .unwrap();
            let expected: Graph = serde_json::from_value(step["original"].clone()).unwrap();
            assert_eq!(graph, expected, "history {n}, step {i}");
            graph.validate().unwrap();
            let restored: Graph =
                serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
            assert_eq!(graph, restored);
            graph = restored;
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_shared_edge_corner_rotation_histories() {
    compare(
        serde_json::from_str(include_str!("fixtures/wall-rotation.json")).unwrap(),
        48,
    );
}
#[test]
#[ignore = "requires private original-house native wall-rotation evidence"]
fn original_house_wall_rotation_histories() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-wall-rotation.json");
    compare(
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        12,
    );
}
