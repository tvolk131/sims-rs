#[path = "support/movement.rs"]
mod support;
use serde_json::Value;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_path_following_projects_accepts_brakes_and_restores() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-movement-paths.json")).unwrap();
    for (index, row) in report["projections"].as_array().unwrap().iter().enumerate() {
        let points: Vec<[i32; 2]> = serde_json::from_value(row["points"].clone()).unwrap();
        let point: [i32; 2] = serde_json::from_value(row["point"].clone()).unwrap();
        let expected: [i32; 2] = serde_json::from_value(row["original"].clone()).unwrap();
        let actual = sims_sim::pose::movement::path::project(
            &points,
            point,
            row["index"].as_i64().unwrap() as i32,
        )
        .unwrap();
        assert_eq!(actual, expected, "projection {index}");
    }
    let projections = report["projections"].as_array().unwrap().len();
    let (cases, services, paths) = support::run(report);
    assert!(cases >= 540 && services >= 884 && projections >= 1204);
    assert_eq!(paths, 0, "supplied paths must execute to the shared tail");
    eprintln!("{cases} native path cases, {services} restored services, {projections} projections");
}
