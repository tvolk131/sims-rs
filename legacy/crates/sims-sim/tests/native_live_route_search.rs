//! Actual live-wrapper vectors, not synthetic geometry or a native tick replay.
use serde_json::{Value, json};
use sims_sim::{
    Point,
    lot_route::{geometry, search},
    region_path,
    regions::Region,
    routing::{Candidate, CandidateSet},
};
use std::io::BufRead;

#[test]
#[ignore = "private original wrapper trace; explicit fresh SIMS_NATIVE_ROUTE_COMPARE_OUTPUT"]
fn live_wrapper_search_passes_match_original() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let trace = root.join(
        std::env::var("SIMS_NATIVE_ROUTE_WRAPPER_TRACE")
            .unwrap_or_else(|_| "cache/native-route-wrapper-20261022-first/trace.jsonl".into()),
    );
    let output = root.join(std::env::var("SIMS_NATIVE_ROUTE_COMPARE_OUTPUT").unwrap());
    assert!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output).unwrap();
    let mut pending = None;
    let mut results = vec![];
    let mut complete = false;
    let mut wrapper_output = None;
    let mut wrapper_entry = None;
    let mut search_inputs = vec![];
    let mut geometry_input: Option<Value> = None;
    let mut geometry_insert = None;
    let mut geometry_results = vec![];
    let mut legacy_object_kind_rows = 0;
    for line in std::io::BufReader::new(std::fs::File::open(&trace).unwrap()).lines() {
        let row: Value = serde_json::from_str(&line.unwrap()).unwrap();
        match row["phase"].as_str() {
            Some("wrapper_entry") => wrapper_entry = Some(row.clone()),
            Some("dynamic_before") => {
                assert!(geometry_input.replace(row.clone()).is_none());
                assert!(geometry_insert.is_none());
            }
            Some("dynamic_insert") => {
                assert!(geometry_input.is_some());
                assert!(geometry_insert.replace(row.clone()).is_none());
            }
            Some("dynamic_after") => {
                let input = geometry_input.take().expect("geometry return has no entry");
                let object_kind = input.get("object_kind").unwrap_or_else(|| {
                    // The first immutable capture predates the discriminator fix:
                    // dynamic_before's object kind overwrote the record kind.
                    legacy_object_kind_rows += 1;
                    &input["kind"]
                });
                let [a, b, c, d]: [i32; 4] =
                    serde_json::from_value(input["actor_bounds"].clone()).unwrap();
                let result = geometry::dynamic(
                    serde_json::from_value(input["rectangle"].clone()).unwrap(),
                    Region([a, b, c, d, 0]),
                    object_kind.as_i64().unwrap() as i16,
                    input["mask"].as_i64().unwrap() as i16,
                    input["direction"].as_i64().unwrap() as i16,
                    input["padding"].as_i64().unwrap() as i32,
                );
                let mut expected_vector: Vec<Region> =
                    serde_json::from_value(input["dynamic"]["values"].clone()).unwrap();
                if !result.excluded {
                    expected_vector.push(result.rectangle);
                }
                let inserted = geometry_insert.take();
                let expected_rectangle = (!result.excluded).then(|| json!(result.rectangle));
                let original_rectangle = inserted.as_ref().map(|r| r["rectangle"].clone());
                let matches = input["object"] == row["object"]
                    && inserted
                        .as_ref()
                        .is_none_or(|r| r["object"] == row["object"])
                    && row["inserted"] == !result.excluded
                    && expected_rectangle == original_rectangle
                    && json!(expected_vector) == row["dynamic"]["values"];
                geometry_results.push(json!({"object":input["object"],"object_kind":object_kind,
                    "excluded":result.excluded,"rectangle":result.rectangle,
                    "original_insert_rectangle":original_rectangle,"matches":matches}));
            }
            Some("search_entry") => {
                assert!(
                    pending.replace(row.clone()).is_none(),
                    "nested/unreturned search"
                );
            }
            Some("search_result") => {
                let input = pending.take().expect("search result has no entry");
                search_inputs.push(input.clone());
                let c = &input["decoded_config"];
                let regions = |key: &str| {
                    serde_json::from_value::<Vec<Region>>(input[key]["values"].clone()).unwrap()
                };
                let (goals, dynamic, fixed) =
                    (regions("goals"), regions("dynamic"), regions("fixed"));
                let result = region_path::search(region_path::Query {
                    start: Point::new(
                        c["start"][0].as_i64().unwrap() as i32,
                        c["start"][1].as_i64().unwrap() as i32,
                    ),
                    goals: &goals,
                    dynamic: &dynamic,
                    fixed: &fixed,
                    maximum: (c["bounded"].as_u64().unwrap() != 0)
                        .then_some(c["maximum"].as_i64().unwrap() as i32),
                    aspect: (c["aspect_enabled"].as_u64().unwrap() != 0)
                        .then_some(c["aspect_ratio"].as_i64().unwrap() as i32),
                    free_cost: c["free_cost"].as_i64().unwrap() as i32,
                    smooth: c["smooth"].as_u64().unwrap() != 0,
                    steps: Some(input["limit"].as_u64().unwrap() as usize + 1),
                })
                .unwrap();
                let actual = json!({"goal":result.goal.map_or(-1, |n|n as i64),
                    "path":result.points.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>(),
                    "rectangles":result.regions,"iterations":result.iterations});
                let original = json!({"goal":row["goal"],"path":row["path"]["values"],
                    "rectangles":row["rectangles"]["values"],"iterations":row["iterations"]});
                results.push(
                    json!({"decoded_config":c,"actual":actual,"original":original,
                    "matches":actual==original}),
                );
            }
            Some("wrapper_return") => wrapper_output = Some(row["output"]["values"].clone()),
            _ => (),
        }
        if row["kind"] == "complete" {
            assert_eq!(row["reason"], "requested_wrapper_returns");
            assert_eq!(row["routes"], 1);
            complete = true;
        }
    }
    assert!(
        complete
            && pending.is_none()
            && results.len() == 2
            && geometry_input.is_none()
            && geometry_insert.is_none()
            && geometry_results.len() == 71
    );
    //4823fa transposes the rectangle-search coordinates for its caller.
    let final_points: Vec<[i32; 2]> =
        serde_json::from_value(results.last().unwrap()["original"]["path"].clone()).unwrap();
    assert_eq!(
        wrapper_output.unwrap(),
        json!(
            final_points
                .iter()
                .map(|p| [p[1], p[0]])
                .collect::<Vec<_>>()
        )
    );
    // Build the original's goals and policy from observed caller inputs,
    // rather than supplying its already-decoded search configuration again.
    let entry = wrapper_entry.unwrap();
    let decode = |s: &str| {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    let context = decode(entry["context"]["bytes"].as_str().unwrap());
    let maximum = i32::from_le_bytes(context[0x54..0x58].try_into().unwrap());
    let set = CandidateSet {
        candidates: entry["context"]["candidates"]["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| Candidate {
                position: [c[0].as_i64().unwrap() as i32, c[1].as_i64().unwrap() as i32],
                score: c[2].as_i64().unwrap() as i32,
                seat: 0,
                flags: 0,
            })
            .collect(),
        maximum,
        result: 0,
        actor_facing: 0,
    };
    let (goals, indices) = search::goals(&set);
    let input = &search_inputs[0];
    let room = decode(input["room_bytes"].as_str().unwrap());
    let half_tiles = i32::from_le_bytes(room[0x38..0x3c].try_into().unwrap());
    let objects = i32::from_le_bytes(room[0x40..0x44].try_into().unwrap());
    let actor = entry["state"]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == entry["context"]["actor"])
        .unwrap();
    let special = [68, 84]
        .iter()
        .any(|&n| actor["person"]["data"][n].as_i64().unwrap() & 1 != 0);
    let config = search::Config::prepare(search::Input {
        half_tiles,
        objects,
        special,
        ground: actor["data"][17] == 0,
        level: i32::from((u16::from_le_bytes(room[0..2].try_into().unwrap()) >> 10) & 3) + 1,
        outdoors: room[0x72] != 0,
        positive_goals: goals.len().try_into().unwrap(),
    });
    let vector =
        |key: &str| serde_json::from_value::<Vec<Region>>(input[key]["values"].clone()).unwrap();
    let c = &input["decoded_config"];
    let assembled = config
        .run(
            Point::new(
                c["start"][0].as_i64().unwrap() as i32,
                c["start"][1].as_i64().unwrap() as i32,
            ),
            &goals,
            &vector("dynamic"),
            &vector("fixed"),
        )
        .unwrap();
    let assembled_passes: Vec<_> = assembled.preliminary.iter().chain(assembled.search.iter()).map(|r|json!({
        "goal":r.goal.map_or(-1,|n|n as i64),"path":r.points.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>(),
        "rectangles":r.regions,"iterations":r.iterations,
    })).collect();
    let original_passes: Vec<_> = results.iter().map(|r| r["original"].clone()).collect();
    let assembly_matches = json!(goals) == input["goals"]["values"]
        && assembled_passes == original_passes
        && assembled.failure_result.is_none();
    std::fs::write(output.join("report.json"),serde_json::to_vec_pretty(&json!({
        "scope":"Direct Rust search comparison against live original wrapper vectors and results; not full routing-wrapper integration or household tick parity.",
        "trace":trace,"passes":results,"dynamic_cases":geometry_results,
        "assembled_policy":config,"assembled_candidate_indices":indices,"assembled_passes":assembled_passes,
        "assembly_matches":assembly_matches,
        "legacy_object_kind_rows":legacy_object_kind_rows,"strict_original_parity_ticks_added":0
    })).unwrap()).unwrap();
    assert!(
        assembly_matches
            && results
                .iter()
                .chain(&geometry_results)
                .all(|r| r["matches"] == true),
        "live original geometry/search differs; expected/actual report preserved"
    );
}
