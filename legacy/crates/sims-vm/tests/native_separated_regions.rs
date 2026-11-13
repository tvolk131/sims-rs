use serde_json::Value;
use sims_vm::scheduler::clock::draw::regions::List;
fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_separated_coalescer_preserves_native_allocation_metadata_and_area_decisions() {
    let f: Value =
        serde_json::from_str(include_str!("fixtures/native-separated-regions.json")).unwrap();
    let rows = f["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 512);
    for (i, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let mut l = List {
            header: hex(&input["header"]),
            backing: hex(&input["backing"]),
        };
        l = serde_json::from_slice(&serde_json::to_vec(&l).unwrap()).unwrap();
        assert!(
            l.coalesce_separated(
                input["enabled"].as_u64().unwrap() as u32,
                f32::from_bits(input["weight_bits"].as_u64().unwrap() as u32)
            )
            .unwrap(),
            "unsupported original case{i}"
        );
        assert_eq!(l.header, hex(&row["expected"]["header"]), "case{i} header");
        assert_eq!(
            l.backing,
            hex(&row["expected"]["backing"]),
            "case{i} backing"
        );
    }
}
