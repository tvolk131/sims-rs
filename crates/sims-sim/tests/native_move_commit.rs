use serde_json::{Value, json};
use sims_sim::object_catalog::move_tool::{cleanup, commit};

struct View {
    underlying: bool,
    facing: i16,
}
impl commit::View for View {
    fn underlying(&self, h: u32) -> Result<Option<u32>, String> {
        assert_eq!(h, 10);
        Ok(self.underlying.then_some(20))
    }
    fn facing(&self, h: u32) -> Result<i16, String> {
        assert_eq!(h, 20);
        Ok(self.facing)
    }
}
fn run(input: &Value, restored: bool) -> Value {
    let flag = |key: &str| input[key].as_bool().unwrap();
    let view = View {
        underlying: flag("underlying"),
        facing: input["facing"].as_i64().unwrap() as i16,
    };
    let mut state = cleanup::State::constructor_fields(40);
    state.held = flag("held").then_some(10);
    state.payload = flag("payload").then_some(30);
    state.set_byte(0x98, u8::from(flag("selection")));
    let mut direction = 123;
    let mut call = commit::Call::default();
    let mut events = vec![];
    let mut references = [2, 2];
    for _ in 0..20 {
        let Some(request) = call.advance(&mut state, &mut direction, &view).unwrap() else {
            break;
        };
        if restored {
            (call, state, direction) =
                serde_json::from_value(json!([call, state, direction])).unwrap();
            call.validate().unwrap();
        }
        let before = (call.clone(), state.clone(), direction);
        assert_eq!(
            call.advance(&mut state, &mut direction, &view).unwrap(),
            Some(request)
        );
        assert_eq!(
            (call.clone(), state.clone(), direction),
            before,
            "polling repeated effects"
        );
        let wrong = match request {
            commit::Request::ReleaseCursor(_) => commit::Reply::Done,
            _ => commit::Reply::Released(true),
        };
        assert!(call.acknowledge(request, wrong).is_err());
        assert_eq!(call, before.0);
        let reply = match request {
            commit::Request::ReleaseCursor(h) => {
                events.push(json!(["ReleaseCursor", h]));
                if flag("replace_at_release") {
                    state.payload = Some(31);
                }
                commit::Reply::Released(flag("released"))
            }
            commit::Request::Shared(r) => {
                match r {
                    cleanup::Request::Append { clipboard, payload } => {
                        events.push(json!(["Append", clipboard, payload]));
                        references[(payload - 30) as usize] += 1;
                        if flag("replace_at_append") {
                            state.payload = Some(31);
                        }
                    }
                    cleanup::Request::ReleasePayload(h) => {
                        events.push(json!(["ReleasePayload", h]));
                        references[(h - 30) as usize] -= 1;
                    }
                    cleanup::Request::ClearCatalogSelection => {
                        events.push(json!(["ClearCatalogSelection"]));
                        if state.byte(0x98) != 0 {
                            events.push(json!(["ClearSelection"]));
                            state.set_byte(0x98, 0);
                        }
                    }
                    cleanup::Request::Cursor => events.push(json!(["Cursor"])),
                    _ => panic!("unexpected child {r:?}"),
                }
                commit::Reply::Done
            }
        };
        call.acknowledge(request, reply).unwrap();
        if restored {
            call = serde_json::from_value(json!(call)).unwrap();
        }
    }
    json!({"events":events,"result":call.outcome().unwrap(),"direction":direction,"payload":state.payload,
           "selection":state.byte(0x98),"references":references})
}
#[test]
fn release_failure_and_success_keep_native_ownership_boundaries() {
    let base = json!({"held":true,"underlying":true,"payload":true,"selection":true,"facing":-1,
        "released":false,"replace_at_release":false,"replace_at_append":false});
    let failed = run(&base, true);
    assert_eq!(failed["events"], json!([["ReleaseCursor", 10], ["Cursor"]]));
    assert_eq!(failed["payload"], 30);
    assert_eq!(failed["selection"], 1);
    assert_eq!(failed["direction"], -1);
    let mut success = base;
    success["released"] = json!(true);
    success["replace_at_append"] = json!(true);
    let actual = run(&success, true);
    assert_eq!(
        actual["events"],
        json!([
            ["ReleaseCursor", 10],
            ["Append", 40, 30],
            ["ReleasePayload", 31],
            ["ClearCatalogSelection"],
            ["ClearSelection"],
            ["Cursor"]
        ])
    );
    assert_eq!(actual["references"], json!([3, 1]));
    assert_eq!(actual["payload"], Value::Null);
    assert_eq!(actual, run(&success, false));
}
#[test]
#[ignore = "private original-instruction corpus; generate with native_move_commit.py"]
fn move_commit_matches_original_instructions_with_restores_and_callback_changes() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/move-commit-1.json");
    let corpus: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(corpus["native_execution"], true);
    let cases = corpus["cases"].as_array().unwrap();
    for (n, row) in cases.iter().enumerate() {
        for restored in [false, true] {
            assert_eq!(
                run(&row["input"], restored),
                row["original"],
                "case {n}, restored {restored}"
            );
        }
    }
    assert_eq!(cases.len(), 104);
}
