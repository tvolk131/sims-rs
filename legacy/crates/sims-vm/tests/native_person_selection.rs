use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_vm::{
    ObjectId,
    selection::{self, FlagScope, FlagUpdate, Flags, Request, Selection, View},
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Mutation {
    after: (String, ObjectId),
    object: ObjectId,
    flags: Option<u32>,
    id: Option<i16>,
    next: Option<ObjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Input {
    flags: [u32; 4],
    ids: [i16; 4],
    order: Vec<ObjectId>,
    target: ObjectId,
    scope: String,
    requested: Option<u32>,
    #[serde(default)]
    mutations: Vec<Mutation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Supplied {
    input: Input,
    next: [ObjectId; 4],
    applied: BTreeSet<usize>,
    events: Vec<Value>,
}

impl Supplied {
    fn new(input: Input) -> Self {
        Self {
            input,
            next: [2, 3, 0, 0],
            applied: BTreeSet::new(),
            events: vec![],
        }
    }

    fn callback(&mut self, event: &str, object: ObjectId) {
        self.events.push(json!([event, object, self.input.flags]));
        for (index, mutation) in self.input.mutations.iter().enumerate() {
            if mutation.after != (event.into(), object) || self.applied.contains(&index) {
                continue;
            }
            self.applied.insert(index);
            let slot = mutation.object as usize - 1;
            if let Some(flags) = mutation.flags {
                self.input.flags[slot] = flags;
            }
            if let Some(id) = mutation.id {
                self.input.ids[slot] = id;
            }
            if let Some(next) = mutation.next {
                self.next[slot] = next;
            }
        }
    }

    fn result(&self, initial: Option<ObjectId>) -> Value {
        json!({"events": self.events, "flags": self.input.flags, "ids": self.input.ids,
            "next": self.next, "selected_before": initial.unwrap_or(0),
            "selected_after": selection::selected(self).unwrap().unwrap_or(0),
            "applied": self.applied, "unchanged_other_bytes": true})
    }
}

impl View for Supplied {
    fn person_order(&self) -> &[ObjectId] {
        &self.input.order
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.input
            .flags
            .get(object as usize - 1)
            .copied()
            .ok_or("missing flags".into())
    }
    fn script_id(&self, object: ObjectId) -> Result<i16, String> {
        self.input
            .ids
            .get(object as usize - 1)
            .copied()
            .ok_or("missing ID".into())
    }
}

impl Flags for Supplied {
    fn scope(&self, object: ObjectId) -> Result<FlagScope, String> {
        Ok(if self.input.scope == "group" {
            FlagScope::Group((object == 2 || object == 3).then_some(1))
        } else {
            FlagScope::Single
        })
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        View::flags(self, object)
    }
    fn write_flags(&mut self, object: ObjectId, value: u32) -> Result<(), String> {
        *self
            .input
            .flags
            .get_mut(object as usize - 1)
            .ok_or("missing flags")? = value;
        Ok(())
    }
    fn next(&self, object: ObjectId) -> Result<Option<ObjectId>, String> {
        let value = *self
            .next
            .get(object as usize - 1)
            .ok_or("missing next link")?;
        Ok((value != 0).then_some(value))
    }
}

fn restore<T: Serialize + DeserializeOwned + std::fmt::Debug + PartialEq>(value: &mut T) {
    let copy = serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap();
    assert_eq!(&copy, value);
    *value = copy;
}

fn set_flags(host: &mut Supplied, object: ObjectId, low: u32, resumed: bool) {
    host.events.push(json!(["flags", object, low]));
    let mut setter = FlagUpdate::new(object, low);
    while let Some(object) = setter.advance(host).unwrap() {
        setter.validate().unwrap();
        if resumed {
            restore(&mut setter);
            restore(host);
            let unchanged = (setter.clone(), host.clone());
            assert_eq!(setter.advance(host).unwrap(), Some(object));
            assert_eq!((setter.clone(), host.clone()), unchanged);
        }
        host.callback("invalidate", object);
        if resumed {
            // Also exercise the interval after effects and before acknowledgment.
            restore(&mut setter);
            restore(host);
        }
        setter.acknowledge(object).unwrap();
    }
    assert!(setter.complete());
    setter.validate().unwrap();
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_selection_order_flags_and_notification_match_with_resume() {
    let data: Value = serde_json::from_str(include_str!("fixtures/person-selection.json")).unwrap();
    let cases = data["selection_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 125);
    for case in cases {
        for resumed in [false, true] {
            let input: Input = serde_json::from_value(case["input"].clone()).unwrap();
            let mut selection = Selection::new((input.target != 0).then_some(input.target));
            let mut host = Supplied::new(input);
            let initial = selection::selected(&host).unwrap();
            while let Some(request) = selection.advance(&host).unwrap() {
                if resumed {
                    restore(&mut selection);
                    restore(&mut host);
                    let unchanged = selection.clone();
                    assert_eq!(selection.advance(&host).unwrap(), Some(request));
                    assert_eq!(selection, unchanged);
                }
                match request {
                    Request::Flags { object, low } => set_flags(&mut host, object, low, resumed),
                    Request::Invalidate(object) => host.callback("invalidate", object),
                    Request::Indicators(object) => host.callback("indicators", object),
                    Request::Notify(payload) => {
                        host.events
                            .push(json!(["notify", selection::SIGNAL, payload]))
                    }
                }
                if resumed {
                    restore(&mut selection);
                    restore(&mut host);
                }
                selection.acknowledge(request).unwrap();
            }
            assert!(selection.complete());
            selection.validate().unwrap();
            assert_eq!(
                host.result(initial),
                case["original"],
                "{}; resumed={resumed}",
                case["input"]
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_low_flags_match_for_ordinary_split_and_live_multipart_chains() {
    let data: Value = serde_json::from_str(include_str!("fixtures/person-selection.json")).unwrap();
    let cases = data["flag_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 1024);
    for case in cases {
        for resumed in [false, true] {
            let input: Input = serde_json::from_value(case["input"].clone()).unwrap();
            let (target, requested) = (input.target, input.requested.unwrap());
            let mut host = Supplied::new(input);
            let initial = selection::selected(&host).unwrap();
            set_flags(&mut host, target, requested, resumed);
            assert_eq!(
                host.result(initial),
                case["original"],
                "{}; resumed={resumed}",
                case["input"]
            );
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_selection_enters_actual_house_broadcast_before_returning() {
    use sims_vm::commander::{
        Registry,
        house::{
            Fields, Receiver,
            dispatch::{Dispatch, Progress, Trace},
        },
    };
    use std::collections::BTreeMap;
    fn state(f: &Fields, g: &BTreeMap<i16, i16>) -> Value {
        let mut fields = serde_json::to_value(f).unwrap();
        fields["ui"]["word44"] = json!(g[&26]);
        json!({"fields":fields,"globals":[g[&2],g[&3]]})
    }
    fn patch(target: &mut Value, input: &Value) {
        if let Some(map) = input.as_object() {
            for (key, value) in map {
                patch(&mut target[key], value);
            }
        } else {
            *target = input.clone();
        }
    }
    let data: Value =
        serde_json::from_str(include_str!("fixtures/selection-notifications.json")).unwrap();
    assert_eq!(
        data["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 192);
    let handlers = [
        Receiver::World,
        Receiver::Module,
        Receiver::Objects,
        Receiver::Globals,
        Receiver::Lot,
        Receiver::Viewer,
        Receiver::Renderer,
        Receiver::Interface,
    ];
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        for budget in [1, 10000] {
            let target = c["target"].as_i64().unwrap() as ObjectId;
            let input = Input {
                target,
                flags: serde_json::from_value(c["flags"].clone()).unwrap(),
                ids: serde_json::from_value(c["ids"].clone()).unwrap(),
                order: serde_json::from_value(c["person_order"].clone()).unwrap(),
                scope: "ordinary".into(),
                requested: None,
                mutations: c["mutations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| Mutation {
                        after: ("indicators".into(), target),
                        object: m["object"].as_i64().unwrap() as ObjectId,
                        flags: m["flags"].as_u64().map(|n| n as u32),
                        id: m["id"].as_i64().map(|n| n as i16),
                        next: None,
                    })
                    .collect(),
            };
            let mut host = Supplied::new(input);
            let mut s = Selection::new((target != 0).then_some(target));
            let mut fields: Fields = serde_json::from_value(c["fields"].clone()).unwrap();
            let mut globals = BTreeMap::from([
                (2, c["globals"][0].as_i64().unwrap() as i16),
                (3, c["globals"][1].as_i64().unwrap() as i16),
                (26, c["fields"]["ui"]["word44"].as_i64().unwrap() as i16),
            ]);
            while let Some(request) = s.advance(&host).unwrap() {
                restore(&mut s);
                restore(&mut host);
                match request {
                    Request::Flags { object, low } => set_flags(&mut host, object, low, true),
                    Request::Invalidate(object) => host.callback("invalidate", object),
                    Request::Indicators(object) => host.callback("indicators", object),
                    Request::Notify(payload) => {
                        let mut registry = Registry::default();
                        registry.generation = c["generation"].as_u64().unwrap() as u32;
                        for id in c["order"].as_array().unwrap() {
                            let id = id.as_u64().unwrap() as u32;
                            registry.construct(id).unwrap();
                            registry.bind(id, handlers[id as usize - 1]).unwrap();
                        }
                        let mut d = Dispatch::begin(
                            registry,
                            fields.clone(),
                            selection::SIGNAL,
                            payload as u32,
                        )
                        .unwrap();
                        let parent = s.clone();
                        let mut services = 0;
                        for _ in 0..5000 {
                            restore(&mut d);
                            restore(&mut s);
                            d.validate().unwrap();
                            assert_eq!(s.advance(&host).unwrap(), Some(request));
                            assert_eq!(s, parent);
                            let selected = selection::selected(&host).unwrap().is_some();
                            let progress = d
                                .advance(&mut globals, selected, budget, |depth, trace, f, g| {
                                    let mut e = state(f, g);
                                    e["depth"] = json!(depth);
                                    let (k, v) = match trace {
                                        Trace::Enter { event, value } => {
                                            ("enter", json!({"event":event,"value":value}))
                                        }
                                        Trace::Invoke(i) => {
                                            ("invoke", serde_json::to_value(i).unwrap())
                                        }
                                        Trace::Returned(value) => ("returned", json!(value)),
                                    };
                                    e[k] = v;
                                    host.events.push(e);
                                })
                                .unwrap();
                            match progress {
                                Progress::Pending => (),
                                Progress::Complete(_) => break,
                                Progress::NestedComplete(_) => panic!("unexpected nested E4"),
                                Progress::Service(call) => {
                                    let mut e = state(&d.fields, &globals);
                                    e["depth"] = json!(d.depth());
                                    e["service"] = serde_json::to_value(&call.request).unwrap();
                                    host.events.push(e);
                                    for change in c["changes"].as_array().unwrap() {
                                        if change["service"].as_u64().unwrap() as usize == services
                                        {
                                            let mut f =
                                                state(&d.fields, &globals)["fields"].clone();
                                            patch(&mut f, &change["fields"]);
                                            globals.insert(
                                                26,
                                                f["ui"]["word44"].as_i64().unwrap() as i16,
                                            );
                                            d.fields = serde_json::from_value(f).unwrap();
                                        }
                                    }
                                    services += 1;
                                    d.acknowledge(&call).unwrap();
                                }
                            }
                        }
                        assert!(d.returned().is_some());
                        fields = d.fields;
                    }
                }
                s.acknowledge(request).unwrap();
            }
            assert!(s.complete());
            let mut result = state(&fields, &globals);
            result["events"] = json!(host.events);
            result["flags"] = json!(host.input.flags);
            result["ids"] = json!(host.input.ids);
            assert_eq!(
                result, row["original"],
                "selection/broadcast case {n}, budget {budget}"
            );
        }
    }
}

fn host() -> Supplied {
    Supplied::new(Input {
        flags: [2, 0, 0, 0],
        ids: [195, 196, 197, 198],
        order: vec![1, 2, 3],
        target: 2,
        scope: "ordinary".into(),
        requested: None,
        mutations: vec![],
    })
}

#[test]
fn selection_rejects_wrong_duplicate_and_tampered_service_ownership() {
    let mut selection = Selection::new(Some(2));
    let host = host();
    assert!(selection.acknowledge(Request::Invalidate(1)).is_err());
    let request = selection.advance(&host).unwrap().unwrap();
    let before = selection.clone();
    assert!(
        selection
            .acknowledge(Request::Flags { object: 2, low: 0 })
            .is_err()
    );
    assert_eq!(selection, before);
    let mut bad = serde_json::to_value(&selection).unwrap();
    bad["pending"]["Flags"]["object"] = json!(2);
    assert!(
        serde_json::from_value::<Selection>(bad)
            .unwrap()
            .validate()
            .is_err()
    );
    selection.acknowledge(request).unwrap();
    let before = selection.clone();
    assert!(selection.acknowledge(request).is_err());
    assert_eq!(selection, before);
    assert!(Selection::new(Some(0)).validate().is_err());
}

#[test]
fn low_flags_reject_wrong_acks_and_cycles_without_extra_member_writes() {
    let mut host = host();
    host.input.scope = "group".into();
    host.next[1] = 1;
    let mut setter = FlagUpdate::new(1, 3);
    assert_eq!(setter.advance(&mut host).unwrap(), Some(1));
    let before = setter.clone();
    assert!(setter.acknowledge(2).is_err());
    assert_eq!(setter, before);
    setter.acknowledge(1).unwrap();
    assert!(setter.acknowledge(1).is_err());
    assert_eq!(setter.advance(&mut host).unwrap(), Some(2));
    setter.acknowledge(2).unwrap();
    let before = host.clone();
    assert!(setter.advance(&mut host).unwrap_err().contains("cyclic"));
    assert_eq!(host, before);
}

#[test]
fn missing_source_state_remains_an_error_instead_of_a_selection_default() {
    let mut host = host();
    host.input.order = vec![5];
    let mut selection = Selection::new(Some(2));
    let before = selection.clone();
    assert!(selection.advance(&host).is_err());
    assert_eq!(selection, before);
    let mut setter = FlagUpdate::new(5, 2);
    assert!(setter.advance(&mut host).is_err());
    setter.validate().unwrap();
    assert!(setter.advance(&mut host).is_err());
}
