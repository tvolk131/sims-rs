use serde_json::Value;
use sims_sim::lot_presenter::walls::{
    construction::Graph,
    cutaway::{Cursor, History, Mask},
};

fn change(graph: &mut Graph, v: &Value) {
    let s = &mut graph.state.edges[v["edge"].as_u64().unwrap() as usize].segments
        [v["segment"].as_u64().unwrap() as usize];
    if let Some(n) = v["flags"].as_u64() {
        s.flags = n as u8;
    }
    if let Some(n) = v["style"].as_u64() {
        s.style = n as u8;
    }
    if !v["patterns"].is_null() {
        s.patterns = serde_json::from_value(v["patterns"].clone()).unwrap();
    }
}
fn compare(report: Value, cases: usize) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), cases);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut graph: Graph = serde_json::from_value(c["graph"].clone()).unwrap();
        let mut history: History = serde_json::from_value(c["history"].clone()).unwrap();
        assert_eq!(
            History::capture(&graph).unwrap(),
            history,
            "history copy {n}"
        );
        for v in c["changes"].as_array().unwrap() {
            change(&mut graph, v);
        }
        let mask: Mask = serde_json::from_value(c["mask"].clone()).unwrap();
        let mut cursor = Cursor::begin(
            &mut graph,
            &history,
            &mask,
            c["level"].as_i64().unwrap() as i32,
        )
        .unwrap();
        let mut events = vec![];
        while let Some(at) = cursor.advance(&graph, &history).unwrap() {
            assert_eq!(cursor.advance(&graph, &history).unwrap(), Some(at));
            let bytes = serde_json::to_vec(&(&graph, &history, &cursor)).unwrap();
            (graph, history, cursor) = serde_json::from_slice(&bytes).unwrap();
            cursor.validate(&graph, &history).unwrap();
            assert_eq!(cursor.advance(&graph, &history).unwrap(), Some(at));
            let mut wrong = at;
            wrong[2] = 0;
            assert!(cursor.acknowledge(&graph, &history, wrong).is_err());
            let mut broken = serde_json::to_value(&cursor).unwrap();
            broken["at"] = Value::Null;
            assert!(
                serde_json::from_value::<Cursor>(broken)
                    .unwrap()
                    .validate(&graph, &history)
                    .is_err()
            );
            for v in c["mutations"].as_array().unwrap() {
                if v["after_event"].as_u64().unwrap() as usize == events.len() {
                    change(&mut graph, v);
                }
            }
            events.push(at);
            cursor.acknowledge(&graph, &history, at).unwrap();
            assert!(cursor.acknowledge(&graph, &history, at).is_err());
        }
        assert!(cursor.complete());
        cursor.validate(&graph, &history).unwrap();
        let expected: Graph = serde_json::from_value(row["original"]["graph"].clone()).unwrap();
        assert_eq!(graph, expected, "cutaway graph {n}");
        assert_eq!(
            serde_json::to_value(history).unwrap(),
            row["original"]["history"],
            "retained history {n}"
        );
        assert_eq!(
            serde_json::to_value(events).unwrap(),
            row["original"]["events"],
            "callbacks {n}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_cutaway_flags_history_callbacks_and_resume() {
    compare(
        serde_json::from_str(include_str!("fixtures/wall-cutaway.json")).unwrap(),
        96,
    );
}
#[test]
#[ignore = "requires private original-house native wall-cutaway evidence"]
fn native_house_cutaway_flags_history_callbacks_and_resume() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/house-wall-cutaway.json");
    compare(
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap(),
        48,
    );
}
