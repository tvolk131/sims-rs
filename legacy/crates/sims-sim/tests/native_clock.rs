#[allow(dead_code)]
#[path = "../examples/compare_native_clock.rs"]
mod comparison;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_clock_vectors() {
    comparison::compare(serde_json::from_str(include_str!("fixtures/clock.json")).unwrap());
}
