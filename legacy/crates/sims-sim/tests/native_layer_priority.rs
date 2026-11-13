use serde_json::Value;
use sims_sim::pose::layer::{self, Binding, Layer, Start};
#[cfg(feature = "private-fixtures")]
#[test]
fn native_priority_preparation_retains_equal_order_and_honors_override_bindings() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-layer-priority.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 512);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut priorities: Vec<i32> = serde_json::from_value(c["bones"].clone()).unwrap();
        let mut layers = c["layers"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let mut layer = Layer::new(
                    i.to_string(),
                    1000.,
                    &[],
                    &[],
                    Start {
                        priority: l["priority"].as_i64().unwrap() as i32,
                        speed: 1.,
                        weight: 0.,
                        now: 0,
                    },
                )
                .unwrap();
                layer.priority_override = l["priority_override"].as_bool().unwrap();
                layer.bindings = l["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| Binding {
                        bone: b.as_u64().unwrap() as usize,
                        motion: 0,
                        event_cursor: None,
                    })
                    .collect();
                layer
            })
            .collect::<Vec<_>>();
        if c["dirty"].as_bool().unwrap() {
            layer::prepare(&mut layers, &mut priorities).unwrap();
        }
        let order = layers
            .iter()
            .map(|l| l.name.parse::<usize>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(order).unwrap(),
            row["original"]["order"],
            "case {n}"
        );
        assert_eq!(
            serde_json::to_value(priorities).unwrap(),
            row["original"]["bones"],
            "case {n}"
        );
        assert_eq!(row["original"]["dirty"], false);
    }
}
