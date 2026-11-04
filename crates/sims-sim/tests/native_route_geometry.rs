use serde_json::Value;
use sims_sim::{
    lot_route::{Prepared, geometry},
    regions::Region,
};

fn report(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/parity-investigation-20261020/results")
        .join(name);
    read_report(&path)
}

fn read_report(path: &std::path::Path) -> Value {
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_hashes"]["capture-000000ec-00010000.bin"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    assert_eq!(report["original_execution"], true);
    report
}

#[test]
#[ignore = "requires preserved playable-original geometry observations"]
fn playable_original_dynamic_transforms() {
    let report = report("gameplay-routing-dynamic-1.json");
    assert_eq!(report["supplied_instructions"], false);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 512);
    for (i, case) in cases.iter().enumerate() {
        let c = &case["input"];
        let [a, b, x, y]: [i32; 4] = serde_json::from_value(c["actor"].clone()).unwrap();
        let result = geometry::dynamic(
            serde_json::from_value(c["rectangle"].clone()).unwrap(),
            Region([a, b, x, y, 0]),
            c["kind"].as_i64().unwrap() as i16,
            c["wall_mask"].as_i64().unwrap() as i16,
            c["direction"].as_i64().unwrap() as i16,
            c["padding"].as_i64().unwrap() as i32,
        );
        assert_eq!(
            serde_json::to_value(result.rectangle).unwrap(),
            case["original"]["rectangle"],
            "case {i}"
        );
        assert_eq!(
            result.excluded,
            case["original"]["excluded"].as_bool().unwrap(),
            "case {i}"
        );
    }
}

#[test]
#[ignore = "requires preserved playable-original geometry observations"]
fn playable_original_hard_prefix() {
    let report = report("gameplay-routing-tail-1.json");
    assert_eq!(report["supplied_instructions"], false);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 256);
    for (i, case) in cases.iter().enumerate() {
        let result = geometry::finalize(Prepared {
            rectangles: serde_json::from_value(case["input"]["rectangles"].clone()).unwrap(),
            hard_count: case["input"]["hard_count"].as_u64().unwrap() as usize,
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            case["original"],
            "case {i}"
        );
    }
}

#[test]
#[ignore = "requires preserved playable-original search observations"]
fn playable_original_search_without_supplied_pointer() {
    compare_search(report("gameplay-routing-full-2.json"), None);
}

#[test]
#[ignore = "requires preserved playable-original nonzero free-cost observations"]
fn playable_original_search_with_free_cost_three() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/native-route-free-cost-20261022.json");
    compare_search(read_report(&path), Some(3));
}

fn compare_search(report: Value, free_cost: Option<i32>) {
    use sims_sim::{Point, region_path};
    assert_eq!(report["supplied_stack_pointer"], false);
    assert_eq!(report["unchanged_recovered_instructions"], true);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 620);
    for (i, case) in cases.iter().enumerate() {
        let c = &case["input"];
        let o = &case["original"];
        assert_eq!(
            c["free_cost"].as_i64().unwrap_or(0) as i32,
            free_cost.unwrap_or(0)
        );
        let goals: Vec<Region> = serde_json::from_value(c["goals"].clone()).unwrap();
        let fixed: Vec<Region> = serde_json::from_value(c["obstacles"].clone()).unwrap();
        let dynamic: Vec<Region> =
            serde_json::from_value(c.get("dynamic").cloned().unwrap_or(serde_json::json!([])))
                .unwrap();
        let result = region_path::search(region_path::Query {
            start: Point::new(
                c["start"][0].as_i64().unwrap() as i32,
                c["start"][1].as_i64().unwrap() as i32,
            ),
            goals: &goals,
            fixed: &fixed,
            dynamic: &dynamic,
            maximum: c["bounded"]
                .as_bool()
                .unwrap_or(true)
                .then_some(c["maximum"].as_i64().unwrap_or(64) as i32),
            aspect: c["aspect"].as_bool().unwrap_or(false).then_some(3),
            free_cost: c["free_cost"].as_i64().unwrap_or(0) as i32,
            smooth: c["smooth"].as_bool().unwrap(),
            steps: c["limit"].as_u64().map(|n| n as usize),
        })
        .unwrap();
        assert_eq!(
            result.goal.map_or(-1, |g| g as i64),
            o["goal"].as_i64().unwrap(),
            "case {i}"
        );
        assert_eq!(
            result.points.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
            serde_json::from_value::<Vec<[i32; 2]>>(o["path"].clone()).unwrap(),
            "case {i}"
        );
        assert_eq!(
            serde_json::to_value(result.regions).unwrap(),
            o["rectangles"],
            "case {i}"
        );
        assert_eq!(
            result.iterations,
            o["iterations"].as_u64().unwrap() as usize,
            "case {i}"
        );
    }
}
