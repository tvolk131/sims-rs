use sims_sim::lot_presenter::floor_light;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_classifier_outputs_and_store_order() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-floor-light.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 1024);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        let i = &c["input"];
        let (count, output, stores) = floor_light::classify(
            serde_json::from_value(i["descriptor"].clone()).unwrap(),
            i["radius"].as_i64().unwrap() as i32,
            serde_json::from_value(i["outputs"].clone()).unwrap(),
        );
        assert_eq!(
            serde_json::to_value(count).unwrap(),
            c["count"],
            "count case{n}"
        );
        assert_eq!(
            serde_json::to_value(output).unwrap(),
            c["outputs"],
            "outputs case{n}"
        );
        assert_eq!(
            serde_json::to_value(stores).unwrap(),
            c["events"],
            "stores case{n}"
        );
    }
}
