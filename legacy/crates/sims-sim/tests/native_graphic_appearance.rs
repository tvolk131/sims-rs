use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::pose::{appearance_selection::descriptor, graphic_appearance::*};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Host {
    state: State,
    index: i16,
    model: u64,
    record: Value,
    events: Vec<Value>,
    applied: Vec<usize>,
}
impl Host {
    fn mutate(&mut self, c: &Value, after: &str) {
        for (i, m) in c["mutations"].as_array().unwrap().iter().enumerate() {
            if self.applied.contains(&i) || m["after"] != after {
                continue;
            }
            self.applied.push(i);
            if let Some(v) = m["cached"].as_i64() {
                self.state.cached = v as i32;
            }
            if let Some(v) = m["kind"].as_u64() {
                self.state.kind = v as u32;
            }
            if let Some(v) = m["dirty"].as_bool() {
                self.state.dirty = v;
            }
            if let Some(v) = m["marker"].as_bool() {
                self.state.marker = v;
            }
            if let Some(v) = m["ordinary_material"].as_bool() {
                self.state.ordinary_material = v;
            }
            if let Some(v) = m["index"].as_i64() {
                self.index = v as i16;
            }
            if let Some(v) = m["model"].as_u64() {
                self.model = v;
            }
            if !m["tint"].is_null() {
                self.state.tint = serde_json::from_value(m["tint"].clone()).unwrap();
            }
        }
    }
    fn model(&self) -> Option<Model> {
        (self.model != 0).then_some(Model {
            model: self.model,
            skin: self.model,
        })
    }
    fn service(&mut self, c: &Value, request: &Request) -> Reply {
        match request {
            Request::Descriptor => {
                self.events.push(json!({"profile":2}));
                let value = descriptor(c["profile"][1].as_str().unwrap(), false);
                self.mutate(c, "profile");
                Reply::Descriptor(value)
            }
            Request::Bind { skin, descriptor } => {
                self.events.push(json!({"skin":skin}));
                self.events.push(json!({"bind":{"record":0,"name":descriptor.name,"texture":descriptor.texture}}));
                if !descriptor.name.is_empty() {
                    self.record =
                        json!({"bound":true,"name":descriptor.name,"texture":descriptor.texture});
                }
                self.mutate(c, "bind");
                Reply::Done
            }
            Request::Tint { model, tint } => {
                self.events.push(json!({"model":model}));
                self.events.push(json!({"tint":tint}));
                self.mutate(c, "tint");
                Reply::Done
            }
        }
    }
}
fn restore(reload: &mut Reload, host: &mut Host) {
    let original = (reload.clone(), host.clone());
    let restored = serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(original, restored);
    (*reload, *host) = restored;
    reload.validate().unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_base_graphic_appearance_order_flags_and_live_model_rereads() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-appearance.json")).unwrap();
    for row in fixture["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut host = Host {
            state: serde_json::from_value(c.clone()).unwrap(),
            index: c["index"].as_i64().unwrap() as i16,
            model: c["model"].as_u64().unwrap(),
            record: c["records"][0].clone(),
            events: vec![],
            applied: vec![],
        };
        let mut reload = Reload::new(c["force"].as_bool().unwrap());
        loop {
            restore(&mut reload, &mut host);
            let model = host.model();
            let Some(request) = reload.advance(&mut host.state, host.index, model).unwrap() else {
                break;
            };
            restore(&mut reload, &mut host);
            let before = (reload.clone(), host.clone());
            assert_eq!(
                reload.advance(&mut host.state, host.index, model).unwrap(),
                Some(request.clone())
            );
            assert_eq!((reload.clone(), host.clone()), before);
            let wrong = if request == Request::Descriptor {
                Reply::Done
            } else {
                Reply::Descriptor(Default::default())
            };
            assert!(reload.acknowledge(&request, wrong).is_err());
            assert_eq!((reload.clone(), host.clone()), before);
            let reply = host.service(c, &request);
            restore(&mut reload, &mut host);
            reload.acknowledge(&request, reply.clone()).unwrap();
            let before = reload.clone();
            assert!(reload.acknowledge(&request, reply).is_err());
            assert_eq!(reload, before);
        }
        assert!(reload.complete());
        let mut actual = serde_json::to_value(&host.state).unwrap();
        actual["record"] = host.record;
        assert_eq!(actual, row["original"], "{c}");
        assert_eq!(json!(host.events), row["events"], "{c}");
        assert_eq!(json!(host.applied), row["applied"], "{c}");
    }
}

#[test]
fn invalid_graphic_appearance_service_identity_is_rejected() {
    let mut state = State::constructed();
    let mut reload = Reload::new(true);
    let request = reload.advance(&mut state, 0, None).unwrap().unwrap();
    reload
        .acknowledge(
            &request,
            Reply::Descriptor(descriptor("head-star,white", false)),
        )
        .unwrap();
    let before = reload.clone();
    assert!(
        reload
            .advance(&mut state, 0, Some(Model { model: 1, skin: 0 }))
            .is_err()
    );
    assert_eq!(reload, before);
    let mut saved = serde_json::to_value(reload).unwrap();
    saved["pending"] =
        json!({"Bind":{"skin":0,"descriptor":{"name":"head-star","texture":"white"}}});
    assert!(
        serde_json::from_value::<Reload>(saved)
            .unwrap()
            .validate()
            .is_err()
    );
}
