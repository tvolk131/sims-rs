use sims_sim::edit_history::snapshot::{
    References, Registry, Snapshot,
    lifecycle::{Destroy, Remove, RemoveObject, Reply, Request, Restore, View},
    placement,
};
use std::collections::BTreeMap;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Host {
    objects: BTreeMap<i16, u32>,
    manager: u32,
    facing: i16,
    on_lot: bool,
    events: std::cell::RefCell<Vec<serde_json::Value>>,
}
impl Registry for Host {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        self.events
            .borrow_mut()
            .push(serde_json::json!(["Lookup", id]));
        Ok(self.objects.get(&id).copied())
    }
    fn on_lot(&mut self, object: u32) -> Result<bool, String> {
        self.events
            .borrow_mut()
            .push(serde_json::json!(["OnLot", object]));
        Ok(self.on_lot)
    }
}
impl placement::View for Host {
    fn facing(&self, _: u32) -> Result<i16, String> {
        Ok(self.facing)
    }
}
impl View for Host {
    fn manager(&self) -> Result<u32, String> {
        Ok(self.manager)
    }
}
fn fixture() -> (Snapshot, References, Host) {
    let h = Host {
        objects: BTreeMap::from([(1, 100), (2, 200), (3, 300)]),
        manager: 900,
        facing: 0,
        on_lot: true,
        events: Default::default(),
    };
    let mut refs = References::new(1);
    let key = refs.acquire(1, &h).unwrap();
    refs.acquire(2, &h).unwrap();
    for r in refs.records.values_mut() {
        r.removed = 7;
    }
    h.events.borrow_mut().clear();
    (
        Snapshot {
            key,
            position: [16, 32],
            level: Some(1),
            facing: Some(0),
            container: Some(0),
            slot: Some(-1),
            definition: Some(70),
            active: 1,
        },
        refs,
        h,
    )
}
fn restored<T: serde::Serialize + serde::de::DeserializeOwned>(v: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(v).unwrap()).unwrap()
}

#[test]
fn removal_marks_the_fresh_snapshot_key_without_deleting_the_object() {
    for key in [0, 1, 2, 99] {
        let (mut s, mut refs, _) = fixture();
        let mut call = Remove::default();
        let r = call.advance(&s, &mut refs).unwrap().unwrap();
        assert_eq!(r, RemoveObject(100));
        (call, refs, s) = restored(&(call, refs, s));
        let before = (call.clone(), refs.clone());
        assert_eq!(call.advance(&s, &mut refs).unwrap(), Some(r));
        assert_eq!((call.clone(), refs.clone()), before);
        assert!(call.acknowledge(RemoveObject(200)).is_err());
        assert_eq!(call, before.0);
        s.key = key;
        call.acknowledge(r).unwrap();
        let result = call.advance(&s, &mut refs);
        if key == 99 {
            assert!(result.is_err());
            assert_eq!(call.result(), None);
        } else {
            assert_eq!(result.unwrap(), None);
            assert_eq!(call.result(), Some(true));
            for (&allocation, record) in &refs.records {
                assert_eq!(record.removed, if allocation == key { 1 } else { 7 });
                assert!(record.object.is_some());
            }
        }
    }
    let (mut s, mut refs, _) = fixture();
    refs.records.get_mut(&1).unwrap().object = None;
    let mut c = Remove::default();
    assert_eq!(c.advance(&s, &mut refs).unwrap(), None);
    assert_eq!(c.result(), Some(true));
    assert_eq!(refs.records[&1].removed, 7);
    s.active = 0;
    let mut c = Remove::default();
    assert_eq!(c.advance(&s, &mut refs).unwrap(), None);
    assert_eq!(c.result(), Some(false));
}

#[test]
fn restoration_keeps_factory_owner_and_compares_the_fresh_script_id() {
    let (mut s, mut refs, mut h) = fixture();
    refs.records.get_mut(&1).unwrap().object = None;
    let mut c = Restore::default();
    let master = c.advance(&s, &mut refs, &h).unwrap().unwrap();
    assert_eq!(
        master,
        Request::Master {
            manager: 900,
            definition: 70
        }
    );
    h.manager = 901;
    c.acknowledge(master, Reply::Master(None)).unwrap();
    let create = c.advance(&s, &mut refs, &h).unwrap().unwrap();
    assert_eq!(
        create,
        Request::Create {
            manager: 900,
            definition: None
        }
    );
    (c, s, refs, h) = restored(&(c, s, refs, h));
    s.key = 2;
    c.acknowledge(create, Reply::Created(2)).unwrap();
    let remove = c.advance(&s, &mut refs, &h).unwrap().unwrap();
    assert_eq!(
        remove,
        Request::Placement(placement::Request::RemoveFromLot(200))
    );
    assert_eq!(refs.records[&1].removed, 7);
    assert_eq!(refs.records[&2].removed, 0);
    // A rejected factory ID does not cause fabricated registry deletion.
    let (s, mut refs, h) = fixture();
    refs.records.get_mut(&1).unwrap().object = None;
    let mut c = Restore::default();
    let r = c.advance(&s, &mut refs, &h).unwrap().unwrap();
    c.acknowledge(r, Reply::Master(Some(70))).unwrap();
    let r = c.advance(&s, &mut refs, &h).unwrap().unwrap();
    c.acknowledge(r, Reply::Created(2)).unwrap();
    assert_eq!(c.advance(&s, &mut refs, &h).unwrap(), None);
    assert_eq!(c.result(), Some(false));
    assert_eq!(refs.records[&1].removed, 7);
}

#[test]
fn restoration_runs_master_lookup_for_existing_objects_and_cleans_failed_placement() {
    for answer in [0, 0x100, 1, 0xffffffff] {
        let (s, mut refs, h) = fixture();
        let mut c = Restore::default();
        let r = c.advance(&s, &mut refs, &h).unwrap().unwrap();
        assert_eq!(
            r,
            Request::Master {
                manager: 900,
                definition: 70
            }
        );
        let before = c.clone();
        assert!(c.acknowledge(r, Reply::Created(1)).is_err());
        assert_eq!(c, before);
        c.acknowledge(r, Reply::Master(Some(70))).unwrap();
        let mut requests = vec![];
        while let Some(r) = c.advance(&s, &mut refs, &h).unwrap() {
            c = restored(&c);
            assert_eq!(c.advance(&s, &mut refs, &h).unwrap(), Some(r));
            requests.push(r);
            let reply = match r {
                Request::Placement(placement::Request::Check(p)) => {
                    Reply::Placement(placement::Reply::Check {
                        value: answer,
                        position: p.position,
                    })
                }
                Request::Placement(_) => Reply::Placement(placement::Reply::Done),
                Request::Remove(_) => Reply::Removed,
                _ => panic!("unexpected factory service"),
            };
            c.acknowledge(r, reply).unwrap();
        }
        let success = answer as u8 != 0;
        assert_eq!(c.result(), Some(success));
        assert_eq!(refs.records[&1].removed, if success { 0 } else { 1 });
        assert_eq!(
            requests
                .iter()
                .filter(|r| matches!(r, Request::Remove(_)))
                .count(),
            usize::from(!success)
        );
        assert!(refs.records[&1].object.is_some());
    }
}

#[test]
fn embedded_snapshot_destruction_gates_release_and_retains_its_captured_key() {
    for active in [0, 1] {
        let (mut s, mut refs, mut h) = fixture();
        s.active = active;
        h.on_lot = false;
        let mut c = Destroy::default();
        let r = c.advance(&s, &mut refs, &mut h).unwrap();
        if active == 0 {
            assert!(c.complete());
            assert!(r.is_none());
            assert_eq!(refs.records[&1].count, 1);
        } else {
            let r = r.unwrap();
            assert_eq!(r.script_id, 1);
            assert_eq!(refs.order, vec![2]);
            (c, s, refs, h) = restored(&(c, s, refs, h));
            s.key = 2;
            s.active = 0;
            assert_eq!(c.advance(&s, &mut refs, &mut h).unwrap(), Some(r));
            c.acknowledge(r).unwrap();
            assert_eq!(c.advance(&s, &mut refs, &mut h).unwrap(), None);
            assert!(c.complete());
            assert!(!refs.records.contains_key(&1));
            assert_eq!(refs.records[&2].count, 1);
        }
    }
}

fn corpus_case(input: &serde_json::Value, cold: bool) -> serde_json::Value {
    use serde_json::json;
    let (mut s, mut refs, mut h) = fixture();
    s.key = input["key"].as_u64().unwrap() as u32;
    s.active = input["active"].as_u64().unwrap() as u8;
    refs.records.get_mut(&1).unwrap().object = input["object"].as_u64().map(|v| v as u32);
    refs.records.get_mut(&1).unwrap().count = input["count"].as_u64().unwrap_or(1) as u32;
    h.on_lot = input["on_lot"].as_bool().unwrap_or(true);
    let result = match input["op"].as_str().unwrap() {
        "Remove" => {
            let mut call = Remove::default();
            while let Some(r) = call.advance(&s, &mut refs).unwrap() {
                if cold {
                    (call, s, refs, h) = restored(&(call, s, refs, h));
                    let bytes = serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap();
                    assert_eq!(call.advance(&s, &mut refs).unwrap(), Some(r));
                    assert_eq!(serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap(), bytes);
                }
                h.events.borrow_mut().push(json!(["Remove", r.0, 1]));
                s.key = input["remove_key"].as_u64().unwrap() as u32;
                call.acknowledge(r).unwrap();
            }
            Some(u32::from(call.result().unwrap()))
        }
        "Destroy" => {
            let mut call = Destroy::default();
            while let Some(r) = call.advance(&s, &mut refs, &mut h).unwrap() {
                if cold {
                    (call, s, refs, h) = restored(&(call, s, refs, h));
                    let bytes = serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap();
                    assert_eq!(call.advance(&s, &mut refs, &mut h).unwrap(), Some(r));
                    assert_eq!(serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap(), bytes);
                }
                h.events
                    .borrow_mut()
                    .push(json!(["Delete", r.script_id as u16, refs.order.len()]));
                call.acknowledge(r).unwrap();
            }
            assert!(call.complete());
            None
        }
        "Restore" => {
            let mut call = Restore::default();
            while let Some(r) = call.advance(&s, &mut refs, &h).unwrap() {
                if cold {
                    (call, s, refs, h) = restored(&(call, s, refs, h));
                    let bytes = serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap();
                    assert_eq!(call.advance(&s, &mut refs, &h).unwrap(), Some(r));
                    assert_eq!(serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap(), bytes);
                }
                let reply = match r {
                    Request::Master {
                        manager,
                        definition,
                    } => {
                        h.events
                            .borrow_mut()
                            .push(json!(["Master", manager, definition]));
                        h.manager = 901;
                        Reply::Master(input["master"].as_u64().map(|v| v as u32))
                    }
                    Request::Create {
                        manager,
                        definition,
                    } => {
                        h.events
                            .borrow_mut()
                            .push(json!(["Create", manager, definition]));
                        if input["publish"].as_bool().unwrap() {
                            refs.records.get_mut(&1).unwrap().object = Some(100);
                        }
                        if input["retarget"].as_bool().unwrap() {
                            s.key = 2;
                        }
                        Reply::Created(input["created"].as_i64().unwrap() as i16)
                    }
                    Request::Remove(RemoveObject(object)) => {
                        h.events.borrow_mut().push(json!(["Remove", object, 1]));
                        s.key = input["remove_key"].as_u64().unwrap() as u32;
                        Reply::Removed
                    }
                    Request::Placement(placement::Request::RemoveFromLot(object)) => {
                        h.events.borrow_mut().push(json!(["Remove", object, 0]));
                        Reply::Placement(placement::Reply::Done)
                    }
                    Request::Placement(placement::Request::Rotate(object)) => {
                        h.events.borrow_mut().push(json!(["Rotate", object, 1]));
                        h.facing = h.facing.wrapping_add(1);
                        Reply::Placement(placement::Reply::Done)
                    }
                    Request::Placement(placement::Request::Check(p)) => {
                        h.events.borrow_mut().push(json!([
                            "Check",
                            p.object,
                            p.position,
                            p.level,
                            p.container,
                            p.slot
                        ]));
                        Reply::Placement(placement::Reply::Check {
                            value: input["answer"].as_u64().unwrap() as u32,
                            position: p.position,
                        })
                    }
                    Request::Placement(placement::Request::Place(p)) => {
                        h.events.borrow_mut().push(json!([
                            "Place",
                            p.object,
                            p.position,
                            p.level,
                            p.container,
                            p.slot
                        ]));
                        Reply::Placement(placement::Reply::Done)
                    }
                };
                call.acknowledge(r, reply).unwrap();
            }
            Some(u32::from(call.result().unwrap()))
        }
        _ => panic!("unknown snapshot lifecycle input"),
    };
    refs.validate().unwrap();
    let records: Vec<_> = refs.order.iter().map(|id| &refs.records[id]).collect();
    let mut output = json!({"events":h.events.into_inner(),"records":records,"next_key":refs.next_key,"snapshot":s});
    if let Some(result) = result {
        output["result"] = json!(result);
    }
    output
}

#[test]
#[ignore = "private prepared lifecycle inputs; not original-instruction validation"]
fn prepared_snapshot_lifecycle_cases_restore_every_callback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let inputs: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-snapshot-lifecycle-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(inputs["native_execution"], false);
    let cases = inputs["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 252);
    for input in cases {
        assert_eq!(
            corpus_case(input, false),
            corpus_case(input, true),
            "{input}"
        );
    }
}

#[test]
#[ignore = "original lifecycle corpus; generation requires no Cargo/rustc processes"]
fn snapshot_lifecycle_matches_original_callers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/snapshot-lifecycle.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], true);
    assert_eq!(
        corpus["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 252);
    for row in cases {
        for cold in [false, true] {
            assert_eq!(
                corpus_case(&row["input"], cold),
                row["original"],
                "{}, cold={cold}",
                row["input"]
            );
        }
    }
}
