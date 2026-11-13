use serde_json::Value;
use sims_sim::person_notification::currency::Locale;

#[cfg(feature = "private-fixtures")]
#[test]
fn original_currency_bytes_all_locale_branches() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-currency.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 528);
    for (case, row) in rows.iter().enumerate() {
        let input = &row["input"];
        let locale = Locale {
            language: input["language"].as_u64().unwrap() as u32,
            real_currency: input["real_currency"].as_bool().unwrap(),
        };
        let expected: Vec<u8> = serde_json::from_value(row["original"]["text"].clone()).unwrap();
        assert_eq!(
            locale.format(input["amount"].as_i64().unwrap()),
            expected,
            "case {case}: {input}"
        );
        assert_eq!(row["original"]["result"], 1);
    }
}
