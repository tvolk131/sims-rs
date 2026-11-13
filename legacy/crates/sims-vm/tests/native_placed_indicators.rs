#![cfg(feature = "private-fixtures")]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::{ObjectId, load_indicators, placed_indicators::*};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Host {
    input: Value,
    indicators: load_indicators::Indicators,
    flags: [u32; 2],
    target: i16,
    fame: i16,
    pool: Vec<Option<ObjectId>>,
    active: Vec<ObjectId>,
    parents: BTreeMap<ObjectId, Option<ObjectId>>,
    positions: BTreeMap<ObjectId, [i32; 3]>,
    graphics: BTreeMap<ObjectId, Graphic>,
    outside: [bool; 2],
    events: Vec<Value>,
    applied: Vec<usize>,
    script_flags: [u16; 8],
    reads: RefCell<Vec<String>>,
    fail: BTreeSet<String>,
}
fn id(v: &Value) -> Option<ObjectId> {
    let i = v.as_i64().unwrap() as i16;
    (i != 0).then_some(i)
}
fn pool(v: &Value) -> Vec<Option<ObjectId>> {
    v.as_array().unwrap().iter().map(id).collect()
}
impl Host {
    fn new(c: &Value) -> Self {
        Self {
            input: c.clone(),
            indicators: load_indicators::Indicators {
                primary: id(&c["primary"]),
                secondary: id(&c["secondary"]),
            },
            flags: serde_json::from_value(c["flags"].clone()).unwrap(),
            target: c["target"].as_i64().unwrap() as i16,
            fame: c["fame"].as_i64().unwrap() as i16,
            pool: pool(&c["pool"]),
            active: vec![],
            parents: [(3, id(&c["parent"]))].into(),
            positions: (1..=8)
                .map(|i| {
                    (
                        i,
                        [
                            40 + i32::from(i),
                            56 + i32::from(i),
                            if i <= 2 { i32::from(i) } else { 1 },
                        ],
                    )
                })
                .collect(),
            graphics: (1..=8).map(|i| (i, Graphic(i as u64))).collect(),
            outside: serde_json::from_value(c["outside"].clone()).unwrap(),
            events: vec![],
            applied: vec![],
            script_flags: [c["script_flags"].as_u64().unwrap() as u16; 8],
            reads: RefCell::default(),
            fail: BTreeSet::new(),
        }
    }
    fn read(&self, field: &str) -> Result<(), String> {
        self.reads.borrow_mut().push(field.into());
        if self.fail.contains(field) {
            Err(format!("absent retained {field}"))
        } else {
            Ok(())
        }
    }
    fn event(&mut self, event: Value) {
        self.events.push(event.clone());
        for (i, m) in self.input["mutations"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if self.applied.contains(&i) || m["after"] != json!([event[0], event[1]]) {
                continue;
            }
            self.applied.push(i);
            if !m["primary"].is_null() {
                self.indicators.primary = id(&m["primary"]);
            }
            if !m["secondary"].is_null() {
                self.indicators.secondary = id(&m["secondary"]);
            }
            if !m["pool"].is_null() {
                self.pool = pool(&m["pool"]);
            }
            let object = m["object"].as_i64().unwrap_or(1) as i16;
            if let Some(v) = m["flags"].as_u64() {
                self.flags[object as usize - 1] = v as u32;
            }
            if let Some(v) = m["target"].as_i64() {
                self.target = v as i16;
            }
            if let Some(v) = m["fame"].as_i64() {
                self.fame = v as i16;
            }
            if let Some(v) = m["outside"].as_bool() {
                self.outside[object as usize - 1] = v;
            }
            if !m["position"].is_null() {
                self.positions.insert(
                    object,
                    serde_json::from_value(m["position"].clone()).unwrap(),
                );
            }
            if let Some(v) = m["graphic"].as_u64() {
                self.graphics.insert(object, Graphic(v));
            }
        }
    }
    fn initialize(&mut self) {
        let mut child = load_indicators::Initialization::default();
        while let Some(request) = child.advance(&self.indicators).unwrap() {
            let reply = match request {
                load_indicators::Request::Lookup(i) => {
                    self.event(json!(["lookup", load_indicators::GUIDS[i]]));
                    load_indicators::Reply::Available(self.input["available"][i].as_bool().unwrap())
                }
                load_indicators::Request::Create(i) => {
                    let created = self.input["created"][i].as_i64().unwrap() as i16;
                    self.event(json!(["create", load_indicators::GUIDS[i], created]));
                    load_indicators::Reply::Created((1..=8).contains(&created).then_some(created))
                }
                load_indicators::Request::Hide(object) => {
                    self.script_flags[object as usize - 1] |= 0x4000;
                    load_indicators::Reply::Done
                }
            };
            child
                .acknowledge(&mut self.indicators, request, reply)
                .unwrap();
            // The actual child, owner cache and Placed caller can all be saved
            // at its resource/factory/hide boundaries; no synthetic result cache.
            let bytes = serde_json::to_vec(&(&child, &*self)).unwrap();
            let (cold, host): (load_indicators::Initialization, Host) =
                serde_json::from_slice(&bytes).unwrap();
            cold.validate().unwrap();
            assert_eq!((&child, &*self), (&cold, &host));
        }
    }
    fn service(&mut self, request: Request) -> Reply {
        match request {
            Request::Initialize => {
                self.event(json!(["initialize", 0]));
                self.initialize();
            }
            Request::Parent(indicator) => {
                let parent = self.parents.get(&indicator).copied().flatten();
                self.event(json!(["parent", indicator, parent.unwrap_or(0)]));
                return Reply::Parent(parent);
            }
            Request::Detach(object) => {
                self.event(json!(["detach", object, 1]));
                self.parents.insert(object, None);
            }
            Request::Release(object) => self.event(json!(["release", object])),
            Request::Attach {
                indicator,
                person,
                position: [x, y, level],
            } => {
                self.event(json!(["attach", indicator, person, x, y, level, 1, 1]));
                self.parents.insert(indicator, Some(person));
            }
            Request::Graphic { graphic, call } => self.event(match call {
                GraphicCall::BindTransform(source) => json!(["bind", graphic.0, source.0]),
                GraphicCall::TransformControl => json!(["graphic", graphic.0, 0x64, 1]),
                GraphicCall::ReloadModel => json!(["graphic", graphic.0, 0xd4, 1]),
                GraphicCall::Refresh => json!(["graphic", graphic.0, 0x54, 0]),
                GraphicCall::Update => json!(["graphic", graphic.0, 0x50, 0]),
            }),
            Request::Refill => {
                self.event(json!(["refill", 0]));
                self.pool = pool(&self.input["refill"]);
            }
            Request::RecordStar(star) => {
                self.event(json!([
                    "record",
                    star,
                    self.pool.iter().map(|v| v.unwrap_or(0)).collect::<Vec<_>>()
                ]));
                self.active.push(star);
            }
        }
        Reply::Done
    }
    fn actual(&self) -> Value {
        json!({"events":self.events,"pool":self.pool.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(),"active":self.active,"applied":self.applied,"flags":self.flags,
            "primary":self.indicators.primary.unwrap_or(0),"secondary":self.indicators.secondary.unwrap_or(0),"script_flags":self.script_flags})
    }
}
impl View for Host {
    fn enabled_byte(&self) -> Result<u8, String> {
        self.read("enabled")?;
        Ok(self.input["enabled"].as_u64().unwrap() as u8)
    }
    fn primary(&self) -> Result<Option<ObjectId>, String> {
        self.read("primary")?;
        Ok(self.indicators.primary)
    }
    fn secondary(&self) -> Result<Option<ObjectId>, String> {
        self.read("secondary")?;
        Ok(self.indicators.secondary)
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        self.read("flags")?;
        Ok(self.flags[object as usize - 1])
    }
    fn is_person(&self, object: ObjectId) -> Result<bool, String> {
        self.read("rtti")?;
        Ok(self.input["people"]
            .as_array()
            .unwrap()
            .contains(&json!(object)))
    }
    fn person_data(&self, _: ObjectId, index: i16) -> Result<i16, String> {
        self.read("person-data")?;
        Ok(match index {
            77 => self.target,
            81 => self.fame,
            _ => panic!("unexpected index"),
        })
    }
    fn installed_expansions(&self) -> Result<u16, String> {
        self.read("installed-mask")?;
        Ok(self.input["expansions"].as_u64().unwrap() as u16)
    }
    fn lookup(&self, script_id: i16) -> Result<Option<ObjectId>, String> {
        self.read("lookup")?;
        Ok((1..=8).contains(&script_id).then_some(script_id))
    }
    fn outside(&self, object: ObjectId) -> Result<bool, String> {
        self.read("outside")?;
        Ok(self.outside[object as usize - 1])
    }
    fn slot(&self, object: ObjectId) -> Result<Option<Slot>, String> {
        self.read("slot")?;
        if self.input["missing_slots"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|v| v == &json!(object))
        {
            return Ok(None);
        }
        Ok(Some(Slot {
            flags: self.input["slot_flags"][object as usize - 1]
                .as_u64()
                .unwrap() as u32,
            occupant: self.input["occupants"][object as usize - 1]
                .as_i64()
                .unwrap() as i16,
        }))
    }
    fn position(&self, object: ObjectId) -> Result<[i32; 3], String> {
        self.read("position")?;
        Ok(self.positions[&object])
    }
    fn graphic(&self, object: ObjectId) -> Result<Graphic, String> {
        self.read("graphic")?;
        Ok(self.graphics[&object])
    }
    fn transform(&self, object: ObjectId) -> Result<Graphic, String> {
        self.read("transform")?;
        Ok(self.graphics[&object])
    }
    fn spare(&self) -> Result<Option<Option<ObjectId>>, String> {
        self.read("spare")?;
        Ok(self.pool.last().copied())
    }
    fn pop_spare(&mut self) -> Result<(), String> {
        self.read("pop")?;
        self.pool.pop().ok_or("empty spare")?;
        Ok(())
    }
}
fn restore(call: &mut Placed, host: &mut Host) {
    let bytes = serde_json::to_vec(&(&*call, &*host)).unwrap();
    let (c, h): (Placed, Host) = serde_json::from_slice(&bytes).unwrap();
    c.validate().unwrap();
    assert_eq!((&c, &h), (&*call, &*host));
    *call = c;
    *host = h;
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/placed-indicators.json")).unwrap()
}
fn case(name: &str) -> Value {
    fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["input"]["name"] == name)
        .unwrap()["input"]
        .clone()
}

#[test]
fn original_placed_order_callbacks_pool_pop_and_fault_effects_survive_every_child() {
    let report = fixture();
    assert_eq!(report["cases"].as_array().unwrap().len(), 91);
    assert_eq!(
        report["playable_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut host = Host::new(c);
        let mut call = Placed::new(id(&c["object"]));
        let mut error = None;
        let mut services = 0;
        loop {
            restore(&mut call, &mut host);
            let request = match call.advance(&mut host) {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    error = Some(e);
                    break;
                }
            };
            services += 1;
            assert!(services < 100, "unbounded caller");
            restore(&mut call, &mut host);
            let before = (call.clone(), host.clone());
            assert_eq!(call.advance(&mut host).unwrap(), Some(request));
            assert_eq!(
                (call.clone(), host.clone()),
                before,
                "pending poll repeated an effect: {c}"
            );
            let wrong = if matches!(request, Request::Parent(_)) {
                Reply::Done
            } else {
                Reply::Parent(None)
            };
            assert!(call.acknowledge(request, wrong).is_err());
            assert_eq!((call.clone(), host.clone()), before);
            let reply = host.service(request);
            restore(&mut call, &mut host);
            assert_eq!(call.advance(&mut host).unwrap(), Some(request));
            call.acknowledge(request, reply).unwrap();
            let before = call.clone();
            assert!(call.acknowledge(request, reply).is_err());
            assert_eq!(call, before);
            restore(&mut call, &mut host);
        }
        restore(&mut call, &mut host);
        assert_eq!(
            error.is_some(),
            !row["original"]["fault"].is_null(),
            "{c}: {error:?}"
        );
        assert_eq!(call.complete(), error.is_none());
        let mut expected = row["original"].clone();
        expected.as_object_mut().unwrap().remove("fault");
        expected.as_object_mut().unwrap().remove("visited");
        assert_eq!(host.actual(), expected, "{c}: {error:?}");
    }
}

#[test]
fn manager_mask_is_required_after_nonperson_initialization_and_rtti() {
    let c = case("nonperson-cold");
    let mut host = Host::new(&c);
    host.fail.insert("installed-mask".into());
    let mut call = Placed::new(Some(1));
    assert_eq!(call.advance(&mut host).unwrap(), Some(Request::Initialize));
    let reply = host.service(Request::Initialize);
    call.acknowledge(Request::Initialize, reply).unwrap();
    let expected = host.indicators.clone();
    assert_eq!(expected.primary, Some(3));
    assert!(
        call.advance(&mut host)
            .unwrap_err()
            .contains("installed-mask")
    );
    assert_eq!(host.indicators, expected);
    assert_eq!(host.script_flags[2], 0x5234);
    assert_eq!(host.script_flags[3], 0x5234);
    assert!(host.reads.borrow().ends_with(&[
        "flags".into(),
        "rtti".into(),
        "installed-mask".into()
    ]));
    restore(&mut call, &mut host);
    let events = host.events.clone();
    assert!(call.advance(&mut host).is_err());
    assert_eq!(host.events, events);
    host.fail.clear();
    assert_eq!(call.advance(&mut host).unwrap(), None);
    assert_eq!(host.indicators, expected);
    assert!(call.complete());
}

#[test]
fn flags_are_captured_once_before_a_suspended_fallible_rtti_read() {
    let c = case("primary-secondary-2-0-0");
    let mut host = Host::new(&c);
    host.fail.insert("rtti".into());
    let mut call = Placed::new(Some(1));
    assert!(call.advance(&mut host).is_err());
    restore(&mut call, &mut host);
    host.flags[0] = 0;
    host.fail.clear();
    assert_eq!(call.advance(&mut host).unwrap(), Some(Request::Parent(3)));
    assert_eq!(
        host.reads
            .borrow()
            .iter()
            .filter(|v| v.as_str() == "flags")
            .count(),
        1
    );
}

#[test]
fn early_null_and_disabled_guards_do_not_read_later_missing_owners() {
    let mut host = Host::new(&case("disabled"));
    host.fail
        .extend(["primary", "flags", "rtti", "installed-mask"].map(str::to_owned));
    let mut call = Placed::new(None);
    assert_eq!(call.advance(&mut host).unwrap(), None);
    assert!(host.reads.borrow().is_empty());
    let mut call = Placed::new(Some(1));
    assert_eq!(call.advance(&mut host).unwrap(), None);
    assert_eq!(&*host.reads.borrow(), &["enabled"]);
}

#[test]
fn star_pop_is_retained_before_slot_failure_and_not_repeated_on_cold_resume() {
    let mut c = case("star-pop-before-slot-fault");
    c["pool"] = json!([5, 7]);
    let mut host = Host::new(&c);
    let mut call = Placed::new(Some(1));
    assert!(call.advance(&mut host).unwrap_err().contains("slot absent"));
    assert_eq!(host.pool, vec![Some(5)]);
    restore(&mut call, &mut host);
    assert!(call.advance(&mut host).is_err());
    assert_eq!(host.pool, vec![Some(5)]);
    assert_eq!(
        host.reads
            .borrow()
            .iter()
            .filter(|v| v.as_str() == "pop")
            .count(),
        1
    );
}

#[test]
fn captured_null_member_is_distinct_from_uncaptured_after_json_restore() {
    let mut host = Host::new(&case("star-[0]-False-0-10"));
    let mut call = Placed::new(Some(1));
    assert_eq!(serde_json::to_value(&call).unwrap()["star"], Value::Null);
    assert_eq!(call.advance(&mut host).unwrap(), None);
    assert_eq!(serde_json::to_value(&call).unwrap()["star"], "Null");
    assert!(host.pool.is_empty());
    restore(&mut call, &mut host);
    assert_eq!(call.advance(&mut host).unwrap(), None);
}

#[test]
fn forged_stages_requests_and_nonpositive_identities_are_rejected() {
    let initial = serde_json::to_value(Placed::new(Some(1))).unwrap();
    for (field, value) in [
        ("object", json!(0)),
        ("object", json!(-1)),
        ("stage", json!("Bind")),
        ("stage", json!("PrimaryParent")),
        ("stage", json!("SecondaryLookup")),
        ("stage", json!("StarPop")),
        ("stage", json!("Record")),
        ("pending", json!("Initialize")),
        ("pending", json!({"Parent":3})),
        ("graphic", json!(0)),
    ] {
        let mut saved = initial.clone();
        saved[field] = value;
        let call: Placed = serde_json::from_value(saved).unwrap();
        assert!(call.validate().is_err(), "forged {field}");
    }
    let mut host = Host::new(&case("primary-secondary-2-0-0"));
    let mut call = Placed::new(Some(1));
    let request = call.advance(&mut host).unwrap().unwrap();
    let before = call.clone();
    assert!(call.acknowledge(request, Reply::Parent(Some(-1))).is_err());
    assert_eq!(call, before);
    let mut saved = serde_json::to_value(&call).unwrap();
    saved["pending"] = json!({"Parent":0});
    assert!(
        serde_json::from_value::<Placed>(saved)
            .unwrap()
            .validate()
            .is_err()
    );
}
