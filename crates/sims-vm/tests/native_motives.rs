#[allow(dead_code)]
#[path = "../examples/compare_native_motives.rs"]
mod comparison;

#[cfg(feature = "private-fixtures")]
#[test]
fn native_motive_vectors() {
    comparison::compare(serde_json::from_str(include_str!("fixtures/motives.json")).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_gameplay_single_precision_motive_updates() {
    comparison::compare(
        serde_json::from_str(include_str!("fixtures/motives-single.json")).unwrap(),
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_complete_advertisement_modifier_table() {
    use sims_vm::motives::{self, Advertisement, Curve};
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/advertisement-modifiers.json")).unwrap();
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let values: [f32; 16] = serde_json::from_value(c["values"].clone()).unwrap();
        let person: Vec<i16> = serde_json::from_value(c["person"].clone()).unwrap();
        let curves: Vec<Curve> = serde_json::from_value(c["curves"].clone()).unwrap();
        let ads: [Advertisement; 16] = serde_json::from_value(c["advertisements"].clone()).unwrap();
        let actual = motives::score(
            &values.map(f32::to_bits),
            &person.try_into().unwrap(),
            &curves,
            Some(&ads),
        )
        .unwrap();
        assert_eq!(
            serde_json::json!(actual.to_bits()),
            row["original"]["bits"],
            "modifier score {i}"
        );
        assert!(row["original"]["error"].is_null());
    }
}
