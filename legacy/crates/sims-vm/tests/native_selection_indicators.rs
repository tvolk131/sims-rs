use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::{ObjectId, selection_indicators::*};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Host {
    input: Value,
    primary: Option<ObjectId>,
    secondary: Option<ObjectId>,
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
            primary: id(&c["primary"]),
            secondary: id(&c["secondary"]),
            flags: serde_json::from_value(c["flags"].clone()).unwrap(),
            target: c["target"].as_i64().unwrap() as i16,
            fame: c["fame"].as_i64().unwrap() as i16,
            pool: pool(&c["pool"]),
            active: c["active"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v.as_i64().unwrap() as i16)
                .collect(),
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
            script_flags: [c["script_flags"].as_u64().unwrap_or(0) as u16; 8],
        }
    }
    fn event(&mut self, event: Value) {
        self.events.push(event.clone());
        for (i, mutation) in self.input["mutations"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if self.applied.contains(&i) || mutation["after"] != json!([event[0], event[1]]) {
                continue;
            }
            self.applied.push(i);
            if !mutation["primary"].is_null() {
                self.primary = id(&mutation["primary"]);
            }
            if !mutation["secondary"].is_null() {
                self.secondary = id(&mutation["secondary"]);
            }
            if !mutation["pool"].is_null() {
                self.pool = pool(&mutation["pool"]);
            }
            if let Some(active) = mutation["active"].as_array() {
                self.active = active.iter().map(|v| v.as_i64().unwrap() as i16).collect();
            }
            let object = mutation["object"].as_i64().unwrap_or(1) as i16;
            if let Some(flags) = mutation["flags"].as_u64() {
                self.flags[object as usize - 1] = flags as u32;
            }
            if let Some(target) = mutation["target"].as_i64() {
                self.target = target as i16;
            }
            if let Some(fame) = mutation["fame"].as_i64() {
                self.fame = fame as i16;
            }
            if let Some(outside) = mutation["outside"].as_bool() {
                self.outside[object as usize - 1] = outside;
            }
            if !mutation["position"].is_null() {
                self.positions.insert(
                    object,
                    serde_json::from_value(mutation["position"].clone()).unwrap(),
                );
            }
            if let Some(graphic) = mutation["graphic"].as_u64() {
                self.graphics.insert(object, Graphic(graphic));
            }
        }
    }
    fn service(&mut self, request: Request) -> Reply {
        match request {
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
            Request::Graphic { graphic, call } => {
                let event = match call {
                    GraphicCall::BindTransform(source) => json!(["bind", graphic.0, source.0]),
                    GraphicCall::TransformControl => json!(["graphic", graphic.0, 0x64, 1]),
                    GraphicCall::ReloadModel => json!(["graphic", graphic.0, 0xd4, 1]),
                    GraphicCall::Refresh => json!(["graphic", graphic.0, 0x54, 0]),
                    GraphicCall::Update => json!(["graphic", graphic.0, 0x50, 0]),
                };
                self.event(event);
            }
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
}
impl View for Host {
    fn enabled(&self) -> bool {
        self.input["enabled"] != 0
    }
    fn primary(&self) -> Option<ObjectId> {
        self.primary
    }
    fn secondary(&self) -> Option<ObjectId> {
        self.secondary
    }
    fn selected(&self) -> Result<Option<ObjectId>, String> {
        Ok(self.input["people"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(id)
            .find(|&i| self.flags[i as usize - 1] & 2 != 0))
    }
    fn flags(&self, object: ObjectId) -> Result<u32, String> {
        Ok(self.flags[object as usize - 1])
    }
    fn is_person(&self, object: ObjectId) -> Result<bool, String> {
        Ok(self.input["people"]
            .as_array()
            .unwrap()
            .contains(&json!(object)))
    }
    fn person_data(&self, _object: ObjectId, index: i16) -> Result<i16, String> {
        Ok(match index {
            77 => self.target,
            81 => self.fame,
            _ => panic!("unexpected person field"),
        })
    }
    fn expansions(&self) -> u16 {
        self.input["expansions"].as_u64().unwrap() as u16
    }
    fn lookup(&self, script_id: i16) -> Option<ObjectId> {
        (1..=8).contains(&script_id).then_some(script_id)
    }
    fn guid(&self, object: ObjectId) -> Result<u32, String> {
        Ok(
            if let Some(guid) = self.input["guids"][object.to_string()].as_u64() {
                guid as u32
            } else if object == 6 {
                self.input["occupant_guid"].as_u64().unwrap() as u32
            } else {
                0
            },
        )
    }
    fn outside(&self, object: ObjectId) -> Result<bool, String> {
        Ok(self.outside[object as usize - 1])
    }
    fn slot(&self, object: ObjectId) -> Result<Option<Slot>, String> {
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
        Ok(self.positions[&object])
    }
    fn graphic(&self, object: ObjectId) -> Result<Graphic, String> {
        Ok(self.graphics[&object])
    }
    fn spare(&self) -> Option<Option<ObjectId>> {
        self.pool.last().copied()
    }
    fn pop_spare(&mut self) -> Result<(), String> {
        self.pool.pop().ok_or("empty native spare pool")?;
        Ok(())
    }
}

fn restore(update: &mut Update, host: &mut Host) {
    let saved = serde_json::to_vec(&(&*update, &*host)).unwrap();
    let (u, h): (Update, Host) = serde_json::from_slice(&saved).unwrap();
    u.validate().unwrap();
    assert_eq!((&u, &h), (&*update, &*host));
    *update = u;
    *host = h;
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_indicator_order_pool_timing_and_callback_rereads_survive_every_boundary() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/selection-indicators.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 208);
    check_indicators(&report, None);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_date_refresh_detaches_before_reading_target_and_restores_every_service() {
    let report: Value = serde_json::from_str(include_str!("fixtures/date-indicator.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 53);
    check_indicators(&report, Some(Entry::Date));
}

fn check_indicators(report: &Value, entry: Option<Entry>) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut host = Host::new(c);
        let mut update = Update::new(
            id(&c["object"]),
            entry.unwrap_or(if c["entry"] == "star" {
                Entry::Star
            } else {
                Entry::Selection
            }),
        );
        while let Some(request) = update.advance(&mut host).unwrap() {
            restore(&mut update, &mut host);
            let before = (update.clone(), host.clone());
            assert_eq!(update.advance(&mut host).unwrap(), Some(request));
            assert_eq!(
                (update.clone(), host.clone()),
                before,
                "poll repeated a native effect"
            );
            let wrong = if matches!(request, Request::Parent(_)) {
                Reply::Done
            } else {
                Reply::Parent(None)
            };
            assert!(update.acknowledge(request, wrong).is_err());
            assert_eq!((update.clone(), host.clone()), before);
            let reply = host.service(request);
            restore(&mut update, &mut host);
            assert_eq!(update.advance(&mut host).unwrap(), Some(request));
            update.acknowledge(request, reply).unwrap();
            let before = update.clone();
            assert!(update.acknowledge(request, reply).is_err());
            assert_eq!(update, before, "duplicate acknowledgment changed state");
            restore(&mut update, &mut host);
        }
        assert!(update.complete());
        let actual = json!({"events":host.events,"pool":host.pool.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(),
            "active":host.active,"flags":host.flags,"applied":host.applied});
        assert_eq!(actual, row["original"], "{c}");
    }
}

#[test]
fn inconsistent_indicator_service_snapshot_is_rejected() {
    let update = Update::new(Some(1), Entry::Selection);
    let mut saved = serde_json::to_value(update).unwrap();
    saved["stage"] = json!("Bind");
    assert!(
        serde_json::from_value::<Update>(saved.clone())
            .unwrap()
            .validate()
            .is_err()
    );
    saved["stage"] = json!("PrimaryParent");
    saved["pending"] = json!({"Detach":3});
    assert!(
        serde_json::from_value::<Update>(saved.clone())
            .unwrap()
            .validate()
            .is_err()
    );
    saved["pending"] = json!({"Parent":0});
    assert!(
        serde_json::from_value::<Update>(saved.clone())
            .unwrap()
            .validate()
            .is_err()
    );
    saved["pending"] = json!({"Parent":3});
    let mut pending: Update = serde_json::from_value(saved.clone()).unwrap();
    let before = pending.clone();
    assert!(
        pending
            .acknowledge(Request::Parent(3), Reply::Parent(Some(0)))
            .is_err()
    );
    assert_eq!(pending, before);
    saved["stage"] = json!("PrimaryRefresh");
    saved["pending"] = json!({"Graphic":{"graphic":0,"call":"Refresh"}});
    assert!(
        serde_json::from_value::<Update>(saved)
            .unwrap()
            .validate()
            .is_err()
    );
}

impl sims_vm::indicator_pools::View for Host {
    fn enabled(&self) -> bool {
        View::enabled(self)
    }
    fn expansions(&self) -> u16 {
        View::expansions(self)
    }
    fn guid(&self, object: ObjectId) -> Result<u32, String> {
        View::guid(self, object)
    }
    fn kind(&self, object: ObjectId) -> Result<i16, String> {
        Ok(if self.is_person(object)? { 2 } else { 4 })
    }
    fn slot(&self, object: ObjectId) -> Result<Option<Slot>, String> {
        View::slot(self, object)
    }
    fn lookup(&self, script_id: i16) -> Option<ObjectId> {
        View::lookup(self, script_id)
    }
    fn hide(&mut self, object: ObjectId) -> Result<(), String> {
        self.script_flags[object as usize - 1] |= 0x4000;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum PoolOperation {
    Refill(sims_vm::indicator_pools::Refill),
    Release(sims_vm::indicator_pools::Release),
}
impl PoolOperation {
    fn validate(&self) {
        match self {
            Self::Refill(v) => v.validate(),
            Self::Release(v) => v.validate(),
        }
        .unwrap();
    }
    fn advance(
        &mut self,
        pools: &mut sims_vm::indicator_pools::Pools,
        host: &mut Host,
    ) -> Option<sims_vm::indicator_pools::Request> {
        match self {
            Self::Refill(v) => v.advance(pools, host),
            Self::Release(v) => v.advance(pools, host),
        }
        .unwrap()
    }
    fn ack(
        &mut self,
        request: sims_vm::indicator_pools::Request,
        reply: sims_vm::indicator_pools::Reply,
    ) -> Result<(), String> {
        match self {
            Self::Refill(v) => v.acknowledge(request, reply),
            Self::Release(v) => v.acknowledge(request),
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_star_factories_and_release_preserve_pool_order_and_restore() {
    use sims_vm::indicator_pools::{
        Pools, Refill, Release, Reply as Answer, Request as Service, Resource,
    };
    let report: Value =
        serde_json::from_str(include_str!("fixtures/indicator-pools.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 196);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut host = Host::new(c);
        let mut pools = Pools {
            spare: host.pool.clone(),
            active: host.active.iter().map(|&i| (i != 0).then_some(i)).collect(),
        };
        let mut op = if c["entry"] == "refill" {
            PoolOperation::Refill(Refill::default())
        } else {
            PoolOperation::Release(Release::new(id(&c["object"])))
        };
        let mut calls = 0;
        while let Some(request) = op.advance(&mut pools, &mut host) {
            let before = (op.clone(), pools.clone(), host.clone());
            assert_eq!(op.advance(&mut pools, &mut host), Some(request));
            assert_eq!((op.clone(), pools.clone(), host.clone()), before);
            let saved = serde_json::to_vec(&(&op, &pools, &host)).unwrap();
            (op, pools, host) = serde_json::from_slice(&saved).unwrap();
            op.validate();
            host.pool = pools.spare.clone();
            host.active = pools.active.iter().map(|v| v.unwrap_or(0)).collect();
            let reply = match request {
                Service::LookupStar => {
                    host.event(json!(["lookup", STAR_GUID]));
                    Answer::Resource(c["catalog"].as_bool().unwrap().then_some(Resource(1)))
                }
                Service::CreateStar(resource) => {
                    assert_eq!(resource, Resource(1));
                    let id = c["created"][calls].as_i64().unwrap() as i16;
                    calls += 1;
                    host.event(json!(["create", id]));
                    Answer::Created(id)
                }
                Service::Detach(object) => {
                    host.event(json!([
                        "pools_at_detach",
                        host.pool.iter().map(|v| v.unwrap_or(0)).collect::<Vec<_>>(),
                        host.active
                    ]));
                    host.event(json!(["detach", object, 0]));
                    Answer::Done
                }
            };
            pools.spare = host.pool.clone();
            pools.active = host.active.iter().map(|&i| (i != 0).then_some(i)).collect();
            let saved = serde_json::to_vec(&(&op, &pools, &host)).unwrap();
            (op, pools, host) = serde_json::from_slice(&saved).unwrap();
            op.validate();
            let before = (op.clone(), pools.clone(), host.clone());
            assert_eq!(op.advance(&mut pools, &mut host), Some(request));
            assert_eq!((op.clone(), pools.clone(), host.clone()), before);
            op.ack(request, reply).unwrap();
            let before = op.clone();
            assert!(op.ack(request, reply).is_err());
            assert_eq!(op, before);
        }
        op.validate();
        let actual = json!({"events":host.events,"pool":pools.spare.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(),
            "active":pools.active.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(),"flags":host.flags,"applied":host.applied,"script_flags":host.script_flags});
        assert_eq!(actual, row["original"], "{c}");
    }
}
