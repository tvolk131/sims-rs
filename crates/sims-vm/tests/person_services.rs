//! Synthetic inputs/results only: no executable bytes or original resources.
#[allow(dead_code)]
#[path = "../examples/compare_native_person_main.rs"]
mod comparison;
#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_person_service_vectors() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/person_main_services.json")).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_non_person_enumeration() {
    let mut report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/home-extra.json")).unwrap();
    report["cases"] = report["iterators"].take();
    comparison::compare(report).unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_category_enumeration() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/category-iteration.json")).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_neighbor_guid_enumeration() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/neighbor-enumeration.json")).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_neighbor_metadata() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/neighbor-metadata.json")).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn recovered_nearby_house_enumeration() {
    let report = serde_json::from_str(include_str!("fixtures/next-house.json")).unwrap();
    comparison::compare(report).unwrap();
}
