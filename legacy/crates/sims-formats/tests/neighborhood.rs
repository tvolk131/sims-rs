use serde::Deserialize;
use sims_formats::neighborhood::Neighborhood;

#[derive(Deserialize)]
struct Row {
    payload: Vec<u8>,
    original: Original,
}
#[derive(Deserialize)]
struct Original {
    data: [i16; 16],
    options: [bool; 11],
    inventories: serde_json::Value,
    consumed: usize,
}
#[derive(Deserialize)]
struct Report {
    capture_sha256: String,
    cases: Vec<Row>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn neighborhood_storage_matches_native_versioned_reader() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/neighborhood-read.json")).unwrap();
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report.cases.len(), 68);
    for (i, row) in report.cases.into_iter().enumerate() {
        let loaded = Neighborhood::parse(&row.payload).unwrap();
        assert_eq!(loaded.data, row.original.data, "case {i}");
        assert_eq!(loaded.options, row.original.options, "case {i}");
        assert_eq!(
            serde_json::to_value(&loaded.inventories).unwrap(),
            row.original.inventories,
            "case {i}"
        );
        assert_eq!(
            loaded.tail,
            row.payload[row.original.consumed..],
            "case {i}"
        );
        assert_eq!(
            loaded.option_bytes,
            row.payload[row.original.consumed - loaded.option_bytes.len()..row.original.consumed],
            "retain noncanonical option encoding, case {i}"
        );
        // The native fixtures are well formed. Our untrusted-file boundary
        // must reject every truncation inside the bytes those routines read.
        for end in 0..row.original.consumed {
            assert!(
                Neighborhood::parse(&row.payload[..end]).is_err(),
                "case {i} accepted incomplete input of {end} bytes"
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn family_magic_words_match_original_constructor_and_binary_reader() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/family-magic.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 18);
    for row in report["cases"].as_array().unwrap() {
        let payload: Vec<u8> = serde_json::from_value(row["payload"].clone()).unwrap();
        let family = sims_formats::neighborhood::Family::parse(&payload).unwrap();
        let history = family.magic_history().unwrap();
        assert_eq!(history.is_some(), row["original"]["enabled"]);
        assert_eq!(
            serde_json::json!(history.unwrap_or([0; 6])),
            row["original"]["values"]
        );
        assert_eq!(row["original"]["consumed"], payload.len() - 12);
        if family.version == 9 {
            for end in 0..16 {
                let mut truncated = family.clone();
                truncated.tail.truncate(end);
                assert!(
                    truncated.magic_history().is_err(),
                    "accepted incomplete magic words: {end}"
                );
            }
        }
    }
}
