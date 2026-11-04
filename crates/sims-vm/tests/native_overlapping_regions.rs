use sims_vm::scheduler::clock::draw::regions::List;
#[test]
fn unsupported_cost_or_growth_leaves_the_pending_component_unchanged() {
    let rectangles = [[0i32, 0, 20, 20], [10, 10, 30, 30]];
    let mut header = vec![0u8; 24];
    for (i, v) in [0x1000u32, 0x1020, 0x1020].into_iter().enumerate() {
        header[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    let original = List {
        header,
        backing: rectangles
            .into_iter()
            .flatten()
            .flat_map(i32::to_le_bytes)
            .collect(),
    };
    for weight in [0., 0.5, f32::NAN] {
        let mut list = original.clone();
        assert!(!list.coalesce(1, weight).unwrap());
        assert_eq!(list, original);
        let mut clipping = original.clone();
        assert!(!clipping.clip([5, 5, 40, 40], 1, weight).unwrap());
        assert_eq!(clipping, original);
    }
}
fn hex(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_clipping_and_overlapping_coalescing_preserve_allocation_bytes_and_restore() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-overlapping-regions.json")).unwrap();
    let rows = f["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 1535);
    for (i, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let mut list = List {
            header: hex(&input["header"]),
            backing: hex(&input["backing"]),
        };
        let mut cold: List = serde_json::from_slice(&serde_json::to_vec(&list).unwrap()).unwrap();
        let enabled = input["enabled"].as_u64().unwrap() as u32;
        let weight = f32::from_bits(input["weight_bits"].as_u64().unwrap() as u32);
        let run = |l: &mut List| {
            if input["clip"].is_null() {
                l.coalesce(enabled, weight)
            } else {
                l.clip(
                    serde_json::from_value(input["clip"].clone()).unwrap(),
                    enabled,
                    weight,
                )
            }
        };
        assert!(run(&mut list).unwrap(), "unsupported case{i}");
        assert!(run(&mut cold).unwrap());
        assert_eq!(list, cold, "case{i} cold resume");
        assert_eq!(
            list.header,
            hex(&row["expected"]["header"]),
            "case{i} header"
        );
        let expected = hex(&row["expected"]["backing"]);
        let different: Vec<_> = list
            .backing
            .iter()
            .zip(&expected)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(offset, (a, b))| (offset, *a, *b))
            .collect();
        assert!(
            different.is_empty(),
            "case{i} backing differences: {different:?}"
        );
        list.validate().unwrap();
        let end: List = serde_json::from_slice(&serde_json::to_vec(&list).unwrap()).unwrap();
        assert_eq!(end, list);
    }
}
