#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_vm::dialog::presentation::{self, Presentation};

fn report() -> Value {
    let r: Value = serde_json::from_str(include_str!("fixtures/dialog-layout.json")).unwrap();
    assert_eq!(
        r["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        r["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    r
}

#[test]
fn presentation_prefix_and_button_sources_match_original() {
    let r = report();
    assert_eq!(r["prefix"].as_array().unwrap().len(), 144);
    for row in r["prefix"].as_array().unwrap() {
        let c = &row["input"];
        let caller = c["caller"].as_bool().unwrap().then_some(1);
        let owner = match c["owner"].as_u64().unwrap() {
            0 => None,
            1 => Some(1),
            _ => Some(2),
        };
        let mode = c["mode"].as_u64().unwrap() as u8;
        let flags = c["flags"].as_u64().unwrap() as u8;
        let p = Presentation::capture(
            caller,
            owner,
            Some(c["guid"].as_u64().unwrap() as u32),
            mode,
        );
        assert_eq!(
            json!({"mode":mode&127,"immediate":flags&1,"modal":u8::from(flags&128==0),
            "anchored":u8::from(p.anchored),"hide_title":u8::from(p.hide_title),"large":u8::from(p.large)}),
            row["original"]
        );
        let restored: Presentation =
            serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(restored, p);
    }
    assert_eq!(r["buttons"].as_array().unwrap().len(), 21);
    for row in r["buttons"].as_array().unwrap() {
        let c = &row["input"];
        let actual = presentation::buttons(
            c["mode"].as_u64().unwrap() as u8,
            serde_json::from_value(c["indices"].clone()).unwrap(),
        );
        let events: Vec<_> = actual
            .iter()
            .map(|b| json!([0x14 + (b.result - 2) * 12, b.index, b.fallback, 1]))
            .collect();
        assert_eq!(json!(events), row["original"]);
        assert_eq!(
            actual.len(),
            presentation::button_count(c["mode"].as_u64().unwrap() as u8)
        );
    }
}

#[test]
fn widget_keys_settings_and_tutorial_control_count_match_original() {
    let r = report();
    assert_eq!(r["layouts"].as_array().unwrap().len(), 160);
    for row in r["layouts"].as_array().unwrap() {
        let c = &row["input"];
        let mode = c["mode"].as_u64().unwrap() as u8;
        let p = Presentation {
            anchored: c["anchored"].as_bool().unwrap(),
            hide_title: c["hide_title"].as_bool().unwrap(),
            large: c["large"].as_bool().unwrap(),
        };
        let events = row["original"]["events"].as_array().unwrap();
        let keys: Vec<_> = presentation::keys(mode)
            .into_iter()
            .map(|(key, result)| json!(["key", result, key, 0]))
            .collect();
        assert_eq!(
            keys,
            events
                .iter()
                .filter(|e| e[0] == "key")
                .cloned()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            json!(p.widget_setting(mode).unwrap_or((456, 123))),
            row["original"]["setting"]
        );
        assert_eq!(events.iter().any(|e| e[0] == "title"), !p.hide_title);
        assert_eq!(
            events
                .iter()
                .filter(|e| e[0] == "button" || e[0] == "dismiss")
                .count(),
            presentation::button_count(mode)
        );
        if mode == 4 {
            assert!(presentation::supported(mode));
            assert!(!events.iter().any(|e| e[0] == "setting" || e[0] == "input"));
        }
    }
    assert!(!presentation::supported(3)); // Text input still requires its own host.
}
