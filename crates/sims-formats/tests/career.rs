use sims_formats::career::Career;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_carr_raw_and_packed_numeric_fields_and_strings() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/career-codec.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let hex = row["bytes"].as_str().unwrap();
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(
            serde_json::to_value(Career::parse(&bytes).unwrap()).unwrap(),
            row["original"]
        );
        for n in 0..bytes.len() {
            assert!(
                Career::parse(&bytes[..n]).is_err(),
                "accepted truncated record at {n}"
            );
        }
    }
}
