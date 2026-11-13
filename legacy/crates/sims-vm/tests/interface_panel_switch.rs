use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::{
    interface::{
        panel_switch::{Fields as Panels, Request as PanelRequest, Selection},
        view_control::expansion::{self, Expansion, Fields, Reply, Request},
    },
    ui_effect::widgets::visibility::{Call, Visibility},
};
use std::collections::BTreeMap;

fn saved<T: Serialize + DeserializeOwned>(value: T, restore: bool) -> T {
    if restore {
        serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap()
    } else {
        value
    }
}
fn merge(to: &mut Value, patch: &Value) {
    if let Some(p) = patch.as_object() {
        for (k, v) in p {
            if v.is_object() {
                merge(&mut to[k], v);
            } else {
                to[k] = v.clone();
            }
        }
    }
}
fn emit(c: &Value, events: &mut Vec<Value>, request: Value, mut state: Value) -> Value {
    let n = events.len();
    events.push(json!({"request":request,"state":state}));
    merge(&mut state, &c["patches"][n.to_string()]);
    state
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Environment {
    app_panel: Option<u32>,
    policy: u8,
    camera: u32,
    screen: u32,
    windows: BTreeMap<u32, [i32; 4]>,
    surfaces: BTreeMap<u32, [i32; 4]>,
}
impl Environment {
    fn inputs(&self) -> expansion::Inputs<'_> {
        expansion::Inputs {
            app_panel: self.app_panel,
            policy: self.policy,
            camera: self.camera,
            screen: self.screen,
            windows: &self.windows,
            surfaces: &self.surfaces,
        }
    }
}
fn run(c: &Value, restore: bool) -> Value {
    let mut events = vec![];
    match c["kind"].as_str().unwrap() {
        "select" => {
            let mut fields: Panels = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut policy = c["policy"].as_u64().unwrap() as u8;
            let mut call = Selection::new(c["selector"].as_i64().unwrap() as i32);
            while let Some(r) = call.advance(&mut fields).unwrap() {
                (call, fields, policy) = saved((call, fields, policy), restore);
                match r {
                    PanelRequest::Visibility { panel, show } => {
                        let mut child = Visibility::new(panel, show);
                        while let Some(w) = child.advance(policy).unwrap() {
                            (call, child, fields, policy) =
                                saved((call, child, fields, policy), restore);
                            let x = emit(
                                c,
                                &mut events,
                                json!({"Window":w}),
                                json!({"fields":fields,"policy":policy}),
                            );
                            fields = serde_json::from_value(x["fields"].clone()).unwrap();
                            policy = x["policy"].as_u64().unwrap() as u8;
                            (call, child, fields, policy) =
                                saved((call, child, fields, policy), restore);
                            child.acknowledge(&w).unwrap();
                        }
                        assert!(child.complete());
                    }
                    PanelRequest::Invalidate(w) => {
                        let x = emit(
                            c,
                            &mut events,
                            json!({"Invalidate":w}),
                            json!({"fields":fields,"policy":policy}),
                        );
                        fields = serde_json::from_value(x["fields"].clone()).unwrap();
                        policy = x["policy"].as_u64().unwrap() as u8;
                    }
                }
                call.acknowledge(r).unwrap();
                (call, fields, policy) = saved((call, fields, policy), restore);
            }
            assert!(call.complete());
            json!({"events":events,"state":{"fields":fields,"policy":policy}})
        }
        "visibility" => {
            let mut policy = c["policy"].as_u64().unwrap() as u8;
            let mut call = Visibility::new(
                c["window"].as_u64().unwrap() as u32,
                c["show"].as_u64().unwrap() as u32,
            );
            while let Some(w) = call.advance(policy).unwrap() {
                call = saved(call, restore);
                let x = emit(
                    c,
                    &mut events,
                    json!({"Window":w}),
                    json!({"policy":policy}),
                );
                policy = x["policy"].as_u64().unwrap() as u8;
                call = saved(call, restore);
                call.acknowledge(&w).unwrap();
            }
            assert!(call.complete());
            json!({"events":events,"state":{"policy":policy}})
        }
        "expansion" => {
            let mut fields: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut env: Environment = serde_json::from_value(c["env"].clone()).unwrap();
            let mut call = Expansion::new(
                c["view"].as_u64().unwrap() as u32,
                c["value"].as_i64().unwrap() as i32,
            );
            let mut parents = 0;
            while let Some(r) = call.advance(&mut fields, env.inputs()).unwrap() {
                (call, fields, env) = saved((call, fields, env), restore);
                let reply = if matches!(r, Request::Parent(_)) {
                    let p = c["parents"][parents].as_u64().unwrap() as u32;
                    parents += 1;
                    Reply::Parent(p)
                } else {
                    Reply::Done
                };
                let x = emit(c, &mut events, json!(r), json!({"fields":fields,"env":env}));
                fields = serde_json::from_value(x["fields"].clone()).unwrap();
                env = serde_json::from_value(x["env"].clone()).unwrap();
                (call, fields, env) = saved((call, fields, env), restore);
                call.acknowledge(&r, reply).unwrap();
                call = saved(call, restore);
            }
            assert!(call.complete());
            json!({"events":events,"state":{"fields":fields,"env":env}})
        }
        _ => panic!("unknown case"),
    }
}
fn checked(c: &Value) -> Value {
    let a = run(c, false);
    assert_eq!(a, run(c, true));
    a
}
fn requests(v: &Value) -> Vec<Value> {
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["request"].clone())
        .collect()
}
fn select(selector: i32) -> Value {
    json!({"kind":"select","selector":selector,"fields":{"selected":selector,"panels":[11,12,13,14,15,16]},"policy":0,"patches":{}})
}
fn expand(value: i32) -> Value {
    json!({"kind":"expansion","view":7,"value":value,"fields":{"value":9,"source":21},
        "env":{"app_panel":11,"policy":0,"camera":31,"screen":22,
            "windows":{"11":[0,50,100,150],"12":[0,20,200,400],"13":[0,30,200,500]},
            "surfaces":{"21":[0,10,100,60],"22":[0,20,800,620],"23":[0,30,1000,830]}},
        "parents":[12,13],"patches":{}})
}
#[test]
fn panel_selection_always_visits_ordered_children_and_invalidates_only_selected_capture() {
    for selector in -1..=6 {
        let v = checked(&select(selector));
        let r = requests(&v);
        assert_eq!(v["state"]["fields"]["selected"], selector);
        for n in 0..6 {
            assert_eq!(
                r[n],
                json!({"Window":Call::new(11+n as u32,if n as i32==selector {0x94}else{0x98},&[])})
            );
        }
        if (0..6).contains(&selector) {
            assert_eq!(r[6], json!({"Invalidate":11+selector}));
            assert_eq!(r.len(), 7);
        } else {
            assert_eq!(r.len(), 6);
        }
    }
    let mut c = select(0);
    c["fields"]["panels"] = json!([0, 0, 0, 0, 0, 0]);
    assert!(requests(&checked(&c)).is_empty());
}
#[test]
fn panel_selection_captures_target_but_rereads_later_identities_and_policy() {
    let mut c = select(0);
    c["patches"] = json!({"0":{"fields":{"selected":5,"panels":[99,88,13,14,15,16]},"policy":128}});
    let v = checked(&c);
    let r = requests(&v);
    assert_eq!(r[0], json!({"Window":Call::new(11,0x94,&[])}));
    assert_eq!(r[1], json!({"Window":Call::new(88,0xa8,&[0,0,0])}));
    assert_eq!(r[6], json!({"Invalidate":11}));
    assert_eq!(v["state"]["fields"]["selected"], 5);
}
#[test]
fn visibility_preserves_full_policy_argument_but_tests_low_byte_without_policy() {
    for show in [0u32, 1, 255, 256, 257, u32::MAX] {
        for policy in [0u8, 1, 128] {
            let c = json!({"kind":"visibility","window":11,"show":show,"policy":policy,"patches":{"0":{"policy":0}}});
            let r = requests(&checked(&c));
            assert_eq!(
                r,
                vec![
                    json!({"Window":if policy!=0 {Call::new(11,0xa8,&[show,0,0])} else {Call::new(11,if show as u8!=0 {0x94}else{0x98},&[])}})
                ]
            );
        }
    }
    let mut v = Visibility::new(11, 256);
    let r = v.advance(1).unwrap().unwrap();
    let before = v.clone();
    assert_eq!(v.advance(0).unwrap(), Some(r.clone()));
    assert!(v.acknowledge(&Call::new(12, 0xa8, &[256, 0, 0])).is_err());
    assert_eq!(v, before);
    v.acknowledge(&r).unwrap();
    assert!(v.complete());
    assert_eq!(v.advance(1).unwrap(), None);
}
#[test]
fn expansion_keeps_distinct_equality_null_policy_and_layout_paths() {
    for value in [0, 1] {
        for policy in [0, 128] {
            let mut c = expand(value);
            c["env"]["policy"] = json!(policy);
            let v = checked(&c);
            let r = requests(&v);
            assert_eq!(v["events"][0]["state"]["fields"]["value"], value);
            assert_eq!(
                r[0],
                json!({"Window":if policy!=0 {Call::new(11,0xa8,&[u32::from(value==0),0,0])} else {Call::new(11,if value==0 {0x94}else{0x98},&[])}})
            );
            let mut expected = vec![r[0].clone()];
            if policy != 0 && value != 0 {
                expected.push(json!({"Flag4000":{"window":11,"value":1}}));
            }
            if policy == 0 || value != 0 {
                expected
                    .push(json!({"Viewport":{"camera":31,"height":if value==0 {550}else{600}}}));
            }
            expected.extend([
                json!({"Parent":7}),
                json!({"Parent":7}),
                json!({"Move":{"window":7,"x":0,"y":420}}),
                json!({"Invalidate":7}),
            ]);
            assert_eq!(r, expected);
        }
    }
    let mut c = expand(0);
    c["fields"]["value"] = json!(0);
    c["env"]["app_panel"] = Value::Null;
    assert!(requests(&checked(&c)).is_empty());
    c["fields"]["value"] = json!(1);
    c["env"]["app_panel"] = json!(0);
    let v = checked(&c);
    assert!(requests(&v).is_empty());
    assert_eq!(v["state"]["fields"]["value"], 0);
    c = expand(0);
    c["parents"] = json!([0, 0]);
    assert_eq!(requests(&checked(&c)).len(), 4);
    c = expand(0);
    c["fields"]["source"] = json!(0);
    assert_eq!(requests(&checked(&c)).len(), 4);
}
#[test]
fn expansion_captures_panel_height_and_choice_but_rereads_camera_parent_and_surface() {
    let mut c = expand(0);
    c["patches"] = json!({"0":{"fields":{"value":1},"env":{"app_panel":13,"camera":32,"screen":23,"windows":{"11":[0,100,100,300]}}},
        "2":{"fields":{"source":23}},"3":{"fields":{"source":22},"env":{"windows":{"13":[0,-100,200,100]}}}});
    let v = checked(&c);
    let r = requests(&v);
    assert_eq!(r[1], json!({"Viewport":{"camera":32,"height":450}}));
    assert_eq!(r[4], json!({"Move":{"window":7,"x":0,"y":-400}}));
    assert_eq!(v["state"]["fields"]["value"], 1);
    c = expand(1);
    c["patches"] = json!({"0":{"env":{"screen":23,"camera":32}}});
    assert_eq!(
        requests(&checked(&c))[1],
        json!({"Viewport":{"camera":32,"height":600}})
    );
}
#[test]
fn expansion_policy_height_is_fresh_after_recursive_flag_and_geometry_wraps() {
    let mut c = expand(1);
    c["env"]["policy"] = json!(128);
    c["patches"] = json!({"0":{"fields":{"value":0},"env":{"app_panel":13,"camera":32}},"1":{"env":{"screen":23}}});
    let r = requests(&checked(&c));
    assert_eq!(r[1], json!({"Flag4000":{"window":11,"value":1}}));
    assert_eq!(r[2], json!({"Viewport":{"camera":32,"height":800}}));
    c = expand(0);
    c["env"]["surfaces"]["22"] = json!([0, i32::MAX, 1, i32::MIN]);
    c["env"]["windows"]["11"] = json!([0, i32::MIN, 1, i32::MAX]);
    c["env"]["surfaces"]["21"] = json!([0, i32::MIN, 1, i32::MAX]);
    c["env"]["windows"]["13"] = json!([0, i32::MIN, 1, i32::MAX]);
    let r = requests(&checked(&c));
    assert_eq!(r[1]["Viewport"]["height"], 52);
    assert_eq!(r[4]["Move"]["y"], 0);
}
#[test]
fn panel_and_expansion_reject_corrupt_continuations_before_shared_writes() {
    let mut f = Panels {
        selected: 8,
        panels: [11, 12, 13, 14, 15, 16],
    };
    let mut p = Selection::new(1);
    let r = p.advance(&mut f).unwrap().unwrap();
    let before = (p.clone(), f.clone());
    assert!(p.acknowledge(PanelRequest::Invalidate(12)).is_err());
    assert_eq!((p.clone(), f.clone()), before);
    let mut corrupt = json!(p);
    corrupt["next"] = json!(7);
    let mut bad: Selection = serde_json::from_value(corrupt).unwrap();
    assert!(bad.advance(&mut f).is_err());
    assert_eq!(f, before.1);
    p.acknowledge(r).unwrap();
    let c = expand(0);
    let mut f: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
    let env: Environment = serde_json::from_value(c["env"].clone()).unwrap();
    let mut call = Expansion::new(7, 0);
    let r = call.advance(&mut f, env.inputs()).unwrap().unwrap();
    let before = call.clone();
    assert!(call.acknowledge(&r, Reply::Parent(12)).is_err());
    assert_eq!(call, before);
    let mut corrupt = json!(call);
    corrupt["height"] = Value::Null;
    let mut bad: Expansion = serde_json::from_value(corrupt).unwrap();
    let before = f.clone();
    assert!(bad.advance(&mut f, env.inputs()).is_err());
    assert_eq!(f, before);
}
fn corpus(path: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../cache")
                .join(path),
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
#[ignore = "private prepared input matrix; Rust continuations, no original execution"]
fn prepared_panel_switch_cases_restore_each_child() {
    let v = corpus("native-panel-switch-inputs.json");
    assert_eq!(v["native_execution"], false);
    assert_eq!(v["cases"].as_array().unwrap().len(), 853);
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(run(c, false), run(c, true), "case{n}: {c}");
    }
}
#[test]
#[ignore = "guarded original panel visibility/switch/expansion corpus; concrete children supplied"]
fn panel_switch_and_expansion_match_original_callers() {
    let v = corpus("research/sims-exe/oracle/panel-switch.json");
    assert_eq!(v["cases"].as_array().unwrap().len(), 853);
    assert_eq!(
        v["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        v["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (n, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            run(&c["input"], true),
            c["original"],
            "case{n}: {}",
            c["input"]
        );
    }
}
