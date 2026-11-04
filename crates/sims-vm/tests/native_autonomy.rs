#[allow(dead_code)]
#[path = "../examples/compare_native_autonomy.rs"]
mod comparison;
#[cfg(feature = "private-fixtures")]
#[test]
fn native_autonomy_selection_vectors() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/autonomy-selection.json")).unwrap(),
    );
}
