use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::interface::{buy::definition, tool};

fn number(v: &Value) -> u32 {
    v.as_u64().unwrap().try_into().unwrap()
}
fn restore<T: Serialize + DeserializeOwned>(v: &mut T, enabled: bool) {
    if enabled {
        *v = serde_json::from_slice(&serde_json::to_vec(v).unwrap()).unwrap();
    }
}
struct Driver<'a> {
    input: &'a Value,
    fields: tool::Fields,
    ordinal: u32,
    activations: u32,
    events: Vec<Value>,
    resumed: bool,
}
impl Driver<'_> {
    fn event(&mut self, kind: &str, tool: u32, argument: Option<u32>) {
        self.events.push(json!({
            "kind": kind, "tool": tool, "argument": argument, "fields": self.fields,
        }));
    }
    fn change(&mut self, mut change: tool::Change) {
        let mut slots = [None; 12];
        slots[6] = Some(1);
        slots[7] = Some(2);
        for _ in 0..20 {
            let Some(request) = change.advance(&mut self.fields).unwrap() else {
                assert!(change.complete());
                return;
            };
            restore(&mut change, self.resumed);
            restore(&mut self.fields, self.resumed);
            let before = self.fields.clone();
            assert_eq!(
                change.advance(&mut self.fields).unwrap(),
                Some(request.clone())
            );
            assert_eq!(self.fields, before);
            let reply = match request {
                tool::Request::Lookup(index) => {
                    tool::Reply::Tool(tool::lookup(&slots, index).unwrap())
                }
                tool::Request::Activate { tool, argument } => {
                    self.event("activate", tool, Some(argument));
                    if self.activations == 0 && !self.input["patch"].is_null() {
                        self.fields = serde_json::from_value(self.input["patch"].clone()).unwrap();
                    }
                    self.activations += 1;
                    tool::Reply::Done
                }
                tool::Request::Deactivate(tool) => {
                    self.event("deactivate", tool, None);
                    tool::Reply::Done
                }
            };
            change.acknowledge(&request, reply).unwrap();
            restore(&mut change, self.resumed);
        }
        panic!("tool caller failed to return");
    }
    fn definition(&mut self) {
        let mut selection = definition::Selection::new(number(&self.input["guid"]));
        for _ in 0..20 {
            let Some(request) = selection.advance(&mut self.fields).unwrap() else {
                assert!(selection.complete());
                return;
            };
            restore(&mut selection, self.resumed);
            restore(&mut self.fields, self.resumed);
            let before = self.fields.clone();
            assert_eq!(selection.advance(&mut self.fields).unwrap(), Some(request));
            assert_eq!(self.fields, before);
            let reply = match request {
                definition::Request::Find(_) => {
                    definition::Reply::Number(if self.input["found"].as_bool().unwrap() {
                        30
                    } else {
                        0
                    })
                }
                definition::Request::LookupTool => definition::Reply::Number(2),
                definition::Request::Ordinal(30) => definition::Reply::Number(self.ordinal),
                definition::Request::Reset(tool) => {
                    self.event("reset", tool, None);
                    self.ordinal = number(&self.input["patch_ordinal"]);
                    definition::Reply::Done
                }
                definition::Request::Replace { tool, argument } => {
                    self.change(tool::Change::replace(Some(tool), argument).unwrap());
                    definition::Reply::Done
                }
                definition::Request::SelectDefault => {
                    self.change(tool::Change::select(7, u32::MAX));
                    definition::Reply::Done
                }
                definition::Request::Classify { tool, definition } => {
                    self.event("classify", tool, Some(definition));
                    definition::Reply::Number(number(&self.input["classification"]))
                }
                other => panic!("unexpected definition request {other:?}"),
            };
            selection.acknowledge(request, reply).unwrap();
            restore(&mut selection, self.resumed);
        }
        panic!("definition caller failed to return");
    }
}

fn run(input: &Value, resumed: bool) -> Value {
    let kind = input["kind"].as_str().unwrap();
    if kind == "picker_activate" {
        let mut mode = Some(number(&input["mode"]) as i32);
        tool::activate_picker(&mut mode, number(&input["argument"]));
        return json!({ "mode": mode.unwrap() as u32 });
    }
    if kind == "picker_deactivate" {
        return json!({ "drawing": tool::PICKER_DEACTIVATED_DRAWING_MODE });
    }
    let mut driver = Driver {
        input,
        fields: tool::Fields {
            active: input["active"].as_u64().map(|n| n as u32),
            argument: 42,
        },
        ordinal: input["ordinal"].as_u64().unwrap_or(0) as u32,
        activations: 0,
        events: Vec::new(),
        resumed,
    };
    match kind {
        "select" => driver.change(tool::Change::select(
            if input["new"] == 1 { 6 } else { 7 },
            number(&input["argument"]),
        )),
        "replace" => driver.change(
            tool::Change::replace(
                input["new"].as_u64().map(|n| n as u32),
                number(&input["argument"]),
            )
            .unwrap(),
        ),
        "definition" => driver.definition(),
        _ => panic!("unknown case"),
    }
    json!({ "events": driver.events, "fields": driver.fields })
}

#[test]
#[ignore = "requires private original native corpus"]
fn tool_handoffs_match_original_callbacks_and_picker_writes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/tool-handoff.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], true);
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 135);
    for (n, case) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(&case["input"], false), case["original"], "case {n}");
        assert_eq!(
            run(&case["input"], true),
            case["original"],
            "restored case {n}"
        );
    }
}

#[test]
fn picker_undefined_mode_survives_nonwriting_arguments() {
    let mut mode = None;
    for argument in [2, 0xffff, u32::MAX] {
        tool::activate_picker(&mut mode, argument);
        assert_eq!(mode, None);
    }
    tool::activate_picker(&mut mode, 0x10001);
    assert_eq!(mode, Some(2));
    tool::activate_picker(&mut mode, 0x10000);
    assert_eq!(mode, Some(0));
}
