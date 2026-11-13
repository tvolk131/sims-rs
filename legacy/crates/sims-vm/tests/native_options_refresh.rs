//! Unchanged native instruction expectations; explicit child-service scope is
//! retained in the fixture. No household, window transplant or tick claim.
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::interface::options::*;

fn number(c: &Value, key: &str) -> u32 {
    c[key].as_i64().unwrap() as u32
}
fn yes(c: &Value, key: &str) -> bool {
    c[key].as_bool().unwrap()
}
fn cold<T: Serialize + DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}
fn mutation(c: &Value, n: usize) -> &Value {
    &c["mutations"][n.to_string()]
}
fn show(c: &Value, restore: bool) -> Value {
    let mut fields = Fields {
        selected_tab: c["tab"].as_i64().unwrap() as i32,
        tutorial_source: yes(c, "source").then_some(2),
    };
    let mut g26 = c["g26"].as_i64().unwrap() as i16;
    let mut call = Show::new(1).unwrap();
    let mut events = vec![];
    for _ in 0..16 {
        let Some(request) = call.advance(&fields, g26).unwrap() else {
            break;
        };
        let held = call.clone();
        assert_eq!(call.advance(&fields, g26).unwrap(), Some(request));
        assert_eq!(held, call);
        if restore {
            (call, fields, g26) = cold(&(call, fields, g26));
        }
        let reply = match request {
            Request::SelectTab { value, .. } => {
                fields.selected_tab = value;
                Reply::Done
            }
            Request::TutorialReceiver(_) => Reply::Receiver(yes(c, "cast").then_some(2)),
            Request::FlashPermission => Reply::Number(number(c, "permission")),
            _ => Reply::Done,
        };
        events.push(json!({"request":request,"state":{
            "tab":fields.selected_tab,"source":fields.tutorial_source.is_some(),"g26":g26}}));
        let m = mutation(c, events.len());
        if let Some(n) = m["tab"].as_i64() {
            fields.selected_tab = n as i32;
        }
        if let Some(b) = m["source"].as_bool() {
            fields.tutorial_source = b.then_some(2);
        }
        if let Some(n) = m["g26"].as_i64() {
            g26 = n as i16;
        }
        if restore {
            (call, fields, g26) = cold(&(call, fields, g26));
        }
        call.acknowledge(request, reply).unwrap();
        if restore {
            call = cold(&call);
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":{"tab":fields.selected_tab,
        "source":fields.tutorial_source.is_some(),"g26":g26}})
}
fn flash(c: &Value, restore: bool) -> Value {
    let mut n = 0u32;
    let groups = [6, 3, 4, 4].map(|count| {
        (0..count)
            .map(|_| {
                n += 1;
                (number(c, "pattern") == 0 || (number(c, "pattern") == 2 && n % 2 == 0))
                    .then_some(n)
            })
            .collect()
    });
    let controls = (1..=18u32)
        .map(|n| {
            (
                n,
                FlashControl {
                    active: 0xdead0000 + n,
                    interval: 0x12340000 + n,
                    phase: (129 + n) as u8,
                },
            )
        })
        .collect();
    let mut fields = FlashFields {
        groups,
        special: number(c, "special"),
        controls,
    };
    let gated = |value: &Value, id| {
        value
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.as_bool() == Some(true))
            .then_some(id)
    };
    let mut input = FlashInputs {
        interval: number(c, "interval"),
        people: yes(c, "people").then_some(20),
        auxiliary: gated(&c["aux"], 30),
    };
    let mut call = Flash::new(number(c, "value"));
    let mut events = vec![];
    for _ in 0..8 {
        let Some(request) = call.advance(&mut fields, input).unwrap() else {
            break;
        };
        let held = (call.clone(), fields.clone());
        assert_eq!(call.advance(&mut fields, input).unwrap(), Some(request));
        assert_eq!(held, (call.clone(), fields.clone()));
        if restore {
            (call, fields, input) = cold(&(call, fields, input));
        }
        events.push(json!({"request":request,"state":fields.controls}));
        let m = mutation(c, events.len());
        if m["interval"].is_number() {
            input.interval = number(m, "interval");
        }
        if m["aux"].is_array() {
            input.auxiliary = gated(&m["aux"], m["aux_id"].as_u64().unwrap_or(30) as u32);
        }
        if restore {
            (call, fields, input) = cold(&(call, fields, input));
        }
        call.acknowledge(request).unwrap();
        if restore {
            call = cold(&call);
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":fields.controls})
}
fn enable(c: &Value, restore: bool) -> Value {
    let mut fields = TutorialControl {
        child: yes(c, "child").then_some(1),
        enabled: number(c, "old") as u8,
    };
    let mut call = EnableTutorial::new(number(c, "value") as u8);
    let mut events = vec![];
    for _ in 0..8 {
        let Some(child) = call.advance(&mut fields).unwrap() else {
            break;
        };
        let held = (call.clone(), fields.clone());
        assert_eq!(call.advance(&mut fields).unwrap(), Some(child));
        assert_eq!(held, (call.clone(), fields.clone()));
        if restore {
            (call, fields) = cold(&(call, fields));
        }
        events.push(json!({"request":child,"state":{"child":fields.child.is_some(),"enabled":fields.enabled}}));
        let m = mutation(c, events.len());
        if let Some(b) = m["child"].as_bool() {
            fields.child = b.then_some(1);
        }
        if m["enabled"].is_number() {
            fields.enabled = number(m, "enabled") as u8;
        }
        if restore {
            (call, fields) = cold(&(call, fields));
        }
        call.acknowledge(child).unwrap();
        if restore {
            call = cold(&call);
        }
    }
    assert!(call.complete());
    json!({"events":events,"state":{"child":fields.child.is_some(),"enabled":fields.enabled}})
}
#[cfg(feature = "private-fixtures")]
#[test]
fn options_refresh_matches_original_services_fields_and_cold_returns() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/native-options-refresh.json")).unwrap();
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 3083);
    for (i, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        for restore in [false, true] {
            let result = match c["kind"].as_str().unwrap() {
                "Show" => show(c, restore),
                "Flash" => flash(c, restore),
                "Enable" => enable(c, restore),
                "Permission" => {
                    json!({"value":flash_permission(yes(c,"lot"),c["g26"].as_i64().unwrap()as i16,number(c,"preference")as u8)})
                }
                _ => unreachable!(),
            };
            assert_eq!(result, row["original"], "case{i},cold{restore}");
        }
    }
}
#[test]
fn options_kernels_reject_foreign_returns_without_mutation() {
    let fields = Fields {
        selected_tab: 1,
        tutorial_source: Some(2),
    };
    let mut show = Show::new(1).unwrap();
    let request = show.advance(&fields, 0).unwrap().unwrap();
    let before = show.clone();
    assert!(show.acknowledge(Request::BaseShow(2), Reply::Done).is_err());
    assert!(show.acknowledge(request, Reply::Number(0)).is_err());
    assert_eq!(before, show);
    show.acknowledge(request, Reply::Done).unwrap();
    let before = show.clone();
    assert!(show.acknowledge(request, Reply::Done).is_err());
    assert_eq!(before, show);
    let mut malformed = serde_json::to_value(&show).unwrap();
    malformed["value"] = json!(7);
    assert!(
        serde_json::from_value::<Show>(malformed)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut enable = EnableTutorial::new(1);
    let mut field = TutorialControl {
        child: Some(2),
        enabled: 0,
    };
    assert_eq!(enable.advance(&mut field).unwrap(), Some(2));
    let before = enable.clone();
    assert!(enable.acknowledge(3).is_err());
    assert_eq!(before, enable);
    let mut flash = Flash::new(0);
    let before = flash.clone();
    assert!(
        flash
            .acknowledge(FlashRequest::People {
                owner: 20,
                value: 0
            })
            .is_err()
    );
    assert_eq!(before, flash);
}
