use serde_json::Value;
use sims_vm::dialog::{format_locals, format_time, format_times};

#[cfg(feature = "private-fixtures")]
#[test]
fn time_formats_and_local_tokens_match_original_language_manager() {
    let report: Value = serde_json::from_str(include_str!("fixtures/dialog-time.json")).unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let formats = report["formats"].as_array().unwrap();
    assert_eq!(formats.len(), 2376);
    for row in formats {
        let c = &row["input"];
        let text = format_time(
            c["language"].as_u64().unwrap() as u32,
            c["hour"].as_i64().unwrap() as i32,
            c["minute"].as_u64().unwrap() as u32,
            c["uppercase"].as_bool().unwrap(),
            c["prefix"].as_bool().unwrap(),
        );
        assert_eq!(text, row["original"]["text"], "{c}");
        assert_eq!(row["original"]["returned"], 1);
    }
    let dialogs = report["dialogs"].as_array().unwrap();
    assert_eq!(dialogs.len(), 384);
    for row in dialogs {
        let c = &row["input"];
        let locals: Vec<i16> = serde_json::from_value(c["locals"].clone()).unwrap();
        let locals = c["frame"].as_bool().unwrap().then_some(locals.as_slice());
        let text = format_locals(c["text"].as_str().unwrap(), locals).unwrap();
        let text = format_times(&text, c["language"].as_u64().unwrap() as u32, locals).unwrap();
        assert_eq!(text, row["original"]["text"], "{c}");
    }
}
