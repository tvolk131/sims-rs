//! Runtime BodyStrings normalization from 587b4b. The native loader appends
//! empty entries through 35 and writes the resource back. We normalize the
//! in-memory resource view only; original files remain read-only.
pub(crate) fn normalize(strings: &mut Vec<String>) {
    strings.resize(strings.len().max(35), String::new());
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_loader_preserves_existing_entries_and_appends_only_through_35() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/body-profile-load.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let length = row["length"].as_u64().unwrap() as usize;
            let original: Vec<_> = (1..=length).map(|i| format!("entry-{i}")).collect();
            let mut strings = original.clone();
            super::normalize(&mut strings);
            assert_eq!(
                strings.len(),
                row["original"]["length"].as_u64().unwrap() as usize
            );
            assert_eq!(strings[..length], original);
            assert!(strings[length..].iter().all(String::is_empty));
            assert_eq!(
                serde_json::json!((length + 1..=strings.len()).collect::<Vec<_>>()),
                row["original"]["appends"]
            );
            let before = strings.clone();
            super::normalize(&mut strings);
            assert_eq!(strings, before);
        }
    }
}
