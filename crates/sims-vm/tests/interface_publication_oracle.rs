use serde_json::{Value, json};
use sims_vm::interface::{panel_dispatch as panel, state::State, world_publication as world};

fn root(path: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache")
        .join(path)
}
fn merge(to: &mut Value, from: &Value) {
    if let (Some(dst), Some(src)) = (to.as_object_mut(), from.as_object()) {
        for (k, v) in src {
            merge(
                dst.get_mut(k)
                    .unwrap_or_else(|| panic!("unknown mutation {k}")),
                v,
            );
        }
    } else {
        *to = from.clone();
    }
}
fn world_snapshot(s: &State, input: world::Inputs, lot: i16) -> Value {
    json!({"state":{"mode":s.mode,"paused":s.paused,"speed":s.speed,"dirty":s.buy.dirty},"inputs":input,"lot_mode":lot})
}
fn world_restore(v: &Value, s: &mut State, input: &mut world::Inputs, lot: &mut i16) {
    s.mode = v["state"]["mode"].as_i64().unwrap() as i32;
    s.paused = v["state"]["paused"].as_u64().unwrap() as u8;
    s.speed = v["state"]["speed"].as_i64().unwrap() as i32;
    s.buy.dirty = v["state"]["dirty"].as_u64().unwrap() as u32;
    *input = serde_json::from_value(v["inputs"].clone()).unwrap();
    *lot = v["lot_mode"].as_i64().unwrap() as i16;
}
fn run(c: &Value, restore: bool) -> Value {
    let mut events = vec![];
    if c["kind"] == "world" {
        let mut s = State::constructed(760);
        let mut input: world::Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
        let mut lot = 0;
        world_restore(c, &mut s, &mut input, &mut lot);
        let mut f = world::Update::new(c["mask"].as_u64().unwrap() as u32);
        for _ in 0..40 {
            let Some(r) = f.advance(&s, &mut lot, input).unwrap() else {
                break;
            };
            let snapshot = world_snapshot(&s, input, lot);
            events.push(json!({"request":r,"state":snapshot}));
            if restore {
                let data = serde_json::to_vec(&(s.clone(), input, lot, f.clone())).unwrap();
                (s, input, lot, f) = serde_json::from_slice(&data).unwrap();
                assert_eq!(f.advance(&s, &mut lot, input).unwrap(), Some(r));
                assert_eq!(world_snapshot(&s, input, lot), snapshot);
            }
            let rv = serde_json::to_value(r).unwrap();
            let name = rv
                .as_str()
                .unwrap_or_else(|| rv.as_object().unwrap().keys().next().unwrap());
            if let Some(patch) = c["mutations"].get(name) {
                let mut v = world_snapshot(&s, input, lot);
                merge(&mut v, patch);
                world_restore(&v, &mut s, &mut input, &mut lot);
            }
            f.acknowledge(r).unwrap();
        }
        assert!(f.complete());
        json!({"events":events,"state":world_snapshot(&s,input,lot)})
    } else {
        assert_eq!(c["kind"], "panel");
        let mut input: panel::Inputs = serde_json::from_value(c["inputs"].clone()).unwrap();
        let mut f = panel::Update::new(c["argument"].as_u64().unwrap() as u32);
        for _ in 0..12 {
            let Some(r) = f.advance(input).unwrap() else {
                break;
            };
            events.push(json!({"request":r,"state":input}));
            if restore {
                let data = serde_json::to_vec(&(input, f.clone())).unwrap();
                (input, f) = serde_json::from_slice(&data).unwrap();
                assert_eq!(f.advance(input).unwrap(), Some(r));
            }
            let name = serde_json::to_value(r.operation).unwrap();
            if let Some(patch) = c["mutations"].get(name.as_str().unwrap()) {
                let mut v = serde_json::to_value(input).unwrap();
                merge(&mut v, patch);
                input = serde_json::from_value(v).unwrap();
            }
            f.acknowledge(r).unwrap();
        }
        assert!(f.complete());
        json!({"events":events,"state":input})
    }
}

#[test]
#[ignore = "private prepared inputs; Rust continuation test, no original execution"]
fn prepared_publication_cases_restore_at_every_child() {
    let report: Value = serde_json::from_slice(
        &std::fs::read(root("native-world-publication-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], false);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1755);
    for (n, c) in cases.iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case {n}: {c}");
    }
}

#[test]
#[ignore = "private guarded original4ce44f/50a8ec caller corpus; child bodies separate"]
fn world_and_panel_publication_match_original_callers() {
    let report: Value = serde_json::from_slice(
        &std::fs::read(root("research/sims-exe/oracle/world-publication.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1755);
    for (n, row) in cases.iter().enumerate() {
        assert_eq!(
            run(&row["input"], true),
            row["original"],
            "case {n}: {}",
            row["input"]
        );
    }
}
