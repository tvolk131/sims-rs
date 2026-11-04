use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_vm::scheduler::clock::draw::actions::{Frame, Record, Reply, Request, State, Vector};
use std::collections::BTreeMap;
const APP: u32 = 0x10000000;
const TABLE: u32 = 0x10003000;
const HEADER: u32 = 0x10008000;
const NODES: u32 = 0x10007000;
fn bytes(v: &str) -> Vec<u8> {
    (0..v.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&v[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn word(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn set(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
fn record(v: &Value) -> Record {
    bytes(v.as_str().unwrap()).try_into().unwrap()
}
fn number(v: &Value) -> u32 {
    v.as_u64().unwrap() as u32
}
fn state(c: &Value) -> State {
    let headers: BTreeMap<_, _> = c["windows"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, h)| (HEADER + i as u32 * 0x200, bytes(h.as_str().unwrap())))
        .collect();
    State {
        initialized: number(&c["initialized"]) as u8,
        windows: headers.keys().copied().collect(),
        records: vec![],
        scratch: vec![],
        headers,
        hint_pending: number(&c["hint_pending"]) as u8,
        hint_token: number(&c["hint_token"]),
        hint_point: [number(&c["hint_point"][0]), number(&c["hint_point"][1])],
    }
}
fn event(r: &Request) -> Value {
    let mut v = serde_json::to_value(r).unwrap();
    if let Some(o) = v.as_object_mut() {
        for inner in o.values_mut() {
            if let Some(record) = inner.get_mut("record") {
                let b: Vec<u8> = record
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| number(v) as u8)
                    .collect();
                *record = Value::String(hex(&b));
            }
        }
    }
    v
}
fn service(c: &Value, s: &mut State, r: &Request, created: &mut u32) -> Reply {
    let token = |r: &Record| number(&c["tokens"][((word(r, 12) - NODES) / 0x80) as usize]);
    match r {
        Request::Mode => Reply::Value(number(&c["mode"])),
        Request::Parent { window } => Reply::Value(word(&s.headers[window], 0x40)),
        Request::Show { window, visible } => {
            set(
                s.headers.get_mut(window).unwrap(),
                0x18,
                if *visible { 8 } else { 0 },
            );
            Reply::Done
        }
        Request::Attach { window, attach } => {
            set(
                s.headers.get_mut(window).unwrap(),
                0x40,
                if *attach { APP } else { 0 },
            );
            Reply::Done
        }
        Request::Initialize { .. } | Request::ClearRecords => Reply::Done,
        Request::SelectedPerson => Reply::Value(326),
        Request::Collect { selected } => {
            assert_eq!(*selected, 326);
            Reply::Records(
                c["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(record)
                    .collect(),
            )
        }
        Request::Token { record } => Reply::Value(token(record)),
        Request::Create { record } => {
            let window = HEADER + (8 + *created) * 0x200;
            *created += 1;
            let mut h = vec![0; 0x124];
            set(&mut h, 0, TABLE);
            set(&mut h, 0x40, APP);
            set(&mut h, 0xdc, word(record, 4));
            set(&mut h, 0x108, token(record));
            h[0x120] = record[8];
            s.headers.insert(window, h);
            s.windows.push(window);
            Reply::Value(window)
        }
        Request::Image { window, image } => {
            set(s.headers.get_mut(window).unwrap(), 0xdc, *image);
            Reply::Done
        }
        Request::CaptionDirty { record } => Reply::Value(u32::from(
            c["dirty"][((word(record, 12) - NODES) / 0x80) as usize]
                .as_bool()
                .unwrap(),
        )),
        Request::Caption { .. } => Reply::Done,
        Request::Move { window, point } | Request::Tween { window, point, .. } => {
            let tween = matches!(r, Request::Tween { .. });
            let h = s.headers.get_mut(window).unwrap();
            h[0xf0] = u8::from(tween);
            set(h, 0xfc, point[0]);
            set(h, 0x100, point[1]);
            if !tween {
                set(h, 0xf4, point[0]);
                set(h, 0xf8, point[1]);
                set(h, 0x7c, point[0]);
                set(h, 0x80, point[1]);
                h[0x105] = 1;
            }
            Reply::Done
        }
        Request::Push { vector, window } => {
            match vector {
                Vector::Windows => s.windows.push(*window),
                Vector::Scratch => s.scratch.push(*window),
                Vector::Records => panic!("unexpected record pointer push"),
            };
            Reply::Done
        }
        Request::Remove { window } => {
            let i = s.windows.iter().position(|w| w == window).unwrap();
            s.windows.remove(i);
            let h = s.headers.get_mut(window).unwrap();
            set(h, 0x40, 0);
            set(h, 0xdc, 0);
            Reply::Done
        }
        Request::Advance { .. } => Reply::Done,
    }
}
fn result(s: &State, events: Vec<Value>) -> Value {
    json!({"events":events,"state":{"initialized":s.initialized,"windows":s.windows,"scratch":s.scratch,
 "records":s.records.iter().map(|r|hex(r)).collect::<Vec<_>>(),"headers":s.headers.iter().map(|(k,v)|(k.to_string(),hex(v))).collect::<BTreeMap<_,_>>(),
 "hint_pending":s.hint_pending,"hint_token":s.hint_token,"hint_point":s.hint_point}})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_action_window_caller_and_every_child_cut() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-action-refresh.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 161);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut s = state(c);
        let mut f = Frame::at_entry();
        let mut created = 0;
        let mut events = vec![];
        let mut cuts = vec![];
        for _ in 0..2000 {
            let Some(request) = f.advance(&mut s).unwrap() else {
                break;
            };
            assert_eq!(f.advance(&mut s).unwrap(), Some(request.clone()));
            let captured = result(&s, vec![])["state"].clone();
            cuts.push(hex(&Sha256::digest(serde_json::to_vec(&captured).unwrap())));
            // JSON is available in the VM crate; the source-bound runtime also tests
            // its actual binary codec. This checks every externally suspended caller.
            let encoded = serde_json::to_vec(&(&f, &s)).unwrap();
            let (mut restored, mut copy): (Frame, State) =
                serde_json::from_slice(&encoded).unwrap();
            let wrong = if matches!(request, Request::Collect { .. }) {
                Reply::Done
            } else {
                Reply::Records(vec![])
            };
            assert!(f.acknowledge(&mut s, request.clone(), wrong).is_err());
            assert_eq!(serde_json::to_vec(&(&f, &s)).unwrap(), encoded);
            let reply = service(c, &mut s, &request, &mut created);
            let mut created_copy = created - u32::from(matches!(request, Request::Create { .. }));
            let reply_copy = service(c, &mut copy, &request, &mut created_copy);
            assert_eq!(reply, reply_copy);
            f.acknowledge(&mut s, request.clone(), reply).unwrap();
            restored
                .acknowledge(&mut copy, request.clone(), reply_copy)
                .unwrap();
            assert_eq!(f, restored);
            assert_eq!(s, copy);
            events.push(event(&request));
        }
        assert!(f.complete(), "case {n}");
        assert_eq!(
            json!(cuts),
            row["expected"]["cut_sha256"],
            "native child-boundary state case {n}"
        );
        let mut expected = row["expected"].clone();
        expected.as_object_mut().unwrap().remove("cut_sha256");
        assert_eq!(result(&s, events), expected, "case {n}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn restored_child_arguments_cannot_redirect_the_native_caller() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-action-refresh.json")).unwrap();
    let case = data["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["mode"] == 2 && !r["input"]["windows"].as_array().unwrap().is_empty())
        .unwrap();
    let mut s = state(&case["input"]);
    let mut f = Frame::at_entry();
    assert_eq!(f.advance(&mut s).unwrap(), Some(Request::Mode));
    f.acknowledge(&mut s, Request::Mode, Reply::Value(2))
        .unwrap();
    let request = f.advance(&mut s).unwrap().unwrap();
    assert!(matches!(request, Request::Parent { .. }));
    let mut wire = serde_json::to_value(&f).unwrap();
    wire["pending"]["Parent"]["window"] = json!(0xdeadbeefu32);
    let mut bad: Frame = serde_json::from_value(wire).unwrap();
    assert!(bad.validate_state(&s).is_err());
    assert!(bad.advance(&mut s).is_err());
}
