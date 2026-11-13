use serde_json::{Value, json};
use sims_sim::house_statistics::State;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_house_statistics_load_sample_rollover_and_zoning() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-house-statistics.json")).unwrap();
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state =
            State::from_saved_pairs(serde_json::from_value(c["saved"].clone()).unwrap());
        assert_eq!(json!(state.pairs), row["original"]["loaded"], "load {n}");
        let mut days = vec![];
        for (step, action) in c["actions"].as_array().unwrap().iter().enumerate() {
            if action == "day" {
                state.new_day();
                days.push(state.pairs);
            } else {
                state.sample(action.as_bool().unwrap());
            }
            assert_eq!(
                json!(state.pairs),
                row["original"]["states"][step],
                "state {n}/{step}"
            );
            let restored: State =
                serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
            assert_eq!(state, restored);
            state = restored;
        }
        assert_eq!(
            json!(days),
            row["original"]["valuations"],
            "valuation order {n}"
        );
        let rows: Vec<[i32; 2]> = serde_json::from_value(c["zoning"].clone()).unwrap();
        let table = sims_vm::zoning::query_table(&rows);
        let queries: Vec<_> = c["lots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lot| sims_vm::zoning::is_residential(&table, lot.as_i64().unwrap() as i16))
            .collect();
        assert_eq!(json!(queries), row["original"]["residential"], "zoning {n}");
    }
}
