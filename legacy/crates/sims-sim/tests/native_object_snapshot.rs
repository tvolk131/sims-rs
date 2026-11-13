use sims_sim::edit_history::snapshot::{
    self, CaptureView, Position, References, Registry, Release, Snapshot,
    placement::{Call, Place, Reply, Request, View},
};
use std::collections::BTreeMap;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct Host {
    objects: BTreeMap<i16, u32>,
    on_lot: bool,
    facing: i16,
    normalized: Option<u32>,
    capture_count: usize,
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
impl View for Host {
    fn facing(&self, _: u32) -> Result<i16, String> {
        Ok(self.facing)
    }
}
impl CaptureView for Host {
    fn normalized_master(&self, _: u32) -> Result<Option<u32>, String> {
        Ok(self.normalized)
    }
    fn script_id(&self, object: u32) -> Result<i16, String> {
        self.objects
            .iter()
            .find_map(|(&id, &o)| (o == object).then_some(id))
            .ok_or("object absent".into())
    }
    fn capture(&mut self, object: u32) -> Result<Position, String> {
        self.events
            .borrow_mut()
            .push(serde_json::json!(["Capture", object]));
        self.capture_count += 1;
        self.facing = -3;
        Ok(Position {
            position: [41, 57],
            level: 2,
            container: Some(300),
            slot: -2,
        })
    }
    fn definition(&self, _: u32) -> Result<u32, String> {
        Ok(71)
    }
    fn facing(&self, _: u32) -> Result<i16, String> {
        Ok(self.facing)
    }
}
fn host() -> Host {
    Host {
        objects: BTreeMap::from([(1, 100), (2, 200), (3, 300)]),
        on_lot: true,
        ..Host::default()
    }
}
fn snapshot(refs: &mut References, h: &Host) -> Snapshot {
    Snapshot {
        key: refs.acquire(1, h).unwrap(),
        position: [16, 32],
        level: Some(1),
        facing: Some(0),
        container: Some(0),
        slot: Some(-1),
        definition: Some(70),
        active: 1,
    }
}
fn next(c: &mut Call, s: &Snapshot, r: &References, h: &Host) -> Option<Request> {
    let request = c.advance(s, r, h).unwrap();
    *c = serde_json::from_slice(&serde_json::to_vec(c).unwrap()).unwrap();
    assert_eq!(c.advance(s, r, h).unwrap(), request);
    request
}

#[test]
fn references_reuse_first_script_record_wrap_keys_and_preserve_missing_objects() {
    let mut h = host();
    let mut refs = References::new(u32::MAX);
    assert_eq!(refs.acquire(0, &h).unwrap(), 0);
    assert!(refs.records.is_empty());
    let key = refs.acquire(1, &h).unwrap();
    assert_eq!(key, u32::MAX);
    h.objects.insert(1, 999);
    assert_eq!(refs.acquire(1, &h).unwrap(), key);
    let id = refs.by_key(key).unwrap().unwrap();
    assert_eq!(refs.records[&id].object, Some(100));
    assert_eq!(refs.records[&id].count, 2);
    refs.records.get_mut(&id).unwrap().count = u32::MAX;
    assert_eq!(refs.acquire(1, &h).unwrap(), key);
    assert_eq!(refs.records[&id].count, 0);
    assert_eq!(refs.acquire(2, &h).unwrap(), 0);
    assert_eq!(refs.next_key, 1);
    assert_eq!(refs.by_key(0).unwrap(), None);
    assert!(refs.by_script_id(2).unwrap().is_some());
    let key = refs.acquire(-7, &h).unwrap();
    let s = Snapshot {
        key,
        active: 1,
        ..Snapshot::default()
    };
    assert_eq!(refs.script_id(&s).unwrap(), -7);
    assert_eq!(refs.object(&s).unwrap(), None);
    assert!(
        refs.object(&Snapshot {
            key: 999,
            active: 1,
            ..Snapshot::default()
        })
        .is_err()
    );
    refs = serde_json::from_slice(&serde_json::to_vec(&refs).unwrap()).unwrap();
    refs.validate().unwrap();
}

#[test]
fn last_reference_unlinks_before_delete_and_retains_allocation_across_callback() {
    for on_lot in [true, false] {
        let mut h = host();
        h.on_lot = on_lot;
        let mut refs = References::new(1);
        let key = refs.acquire(1, &h).unwrap();
        let old = refs.by_key(key).unwrap().unwrap();
        let mut release = Release::new(key);
        let request = release.advance(&mut refs, &mut h).unwrap();
        assert!(refs.order.is_empty());
        if on_lot {
            assert!(request.is_none() && refs.records.is_empty() && release.complete());
            continue;
        }
        assert_eq!(request, Some(snapshot::Delete { script_id: 1 }));
        (release, refs) =
            serde_json::from_slice(&serde_json::to_vec(&(&release, &refs)).unwrap()).unwrap();
        let before = refs.clone();
        assert_eq!(release.advance(&mut refs, &mut h).unwrap(), request);
        assert_eq!(refs, before);
        let before = release.clone();
        assert!(
            release
                .acknowledge(snapshot::Delete { script_id: 2 })
                .is_err()
        );
        assert_eq!(release, before);
        // A callback can acquire the same script ID after the old record was
        // unlinked. Cleanup must free its captured allocation, not this one.
        let new = refs.acquire(1, &h).unwrap();
        assert_ne!(new, key);
        refs.records.get_mut(&old).unwrap().object = Some(300);
        release.acknowledge(request.unwrap()).unwrap();
        assert_eq!(release.advance(&mut refs, &mut h).unwrap(), None);
        assert!(!refs.records.contains_key(&old));
        assert_eq!(refs.records.len(), 1);
        assert_eq!(
            refs.records[&refs.by_key(new).unwrap().unwrap()].object,
            Some(100)
        );
        refs.validate().unwrap();
    }
}

#[test]
fn capture_preserves_partial_initialization_and_normalizes_before_acquiring() {
    let mut h = host();
    let mut refs = References::new(1);
    let mut s = Snapshot::default();
    assert!(snapshot::capture(&mut s, &mut refs, 0, &mut h).unwrap());
    assert_eq!(s, Snapshot::default());
    assert!(!snapshot::capture(&mut s, &mut refs, 19, &mut h).unwrap());
    assert_eq!(s, Snapshot::default());
    h.normalized = Some(200);
    assert!(snapshot::capture(&mut s, &mut refs, 1, &mut h).unwrap());
    assert_eq!(refs.script_id(&s).unwrap(), 2);
    assert_eq!(refs.object(&s).unwrap(), Some(200));
    assert_eq!(s.position, [41, 57]);
    assert_eq!(s.level, Some(2));
    assert_eq!(s.facing, Some(-3));
    assert_eq!(s.container, Some(3));
    assert_eq!(s.slot, Some(-2));
    assert_eq!(s.definition, Some(71));
    let before = (s.clone(), refs.clone());
    assert!(!snapshot::capture(&mut s, &mut refs, 0, &mut h).unwrap());
    assert_eq!((s, refs), before);
    assert_eq!(h.capture_count, 1);
}

#[test]
fn live_queries_preserve_native_slot_normalization_and_reject_reused_script_ids() {
    use sims_sim::{edit_history::snapshot::live, lot_links::Links};
    use sims_vm::{Object, Program, World, autonomy::Placement, scheduler::storage::Storage};
    let mut world = World::default();
    world.scheduler_storage = Some(Storage::default());
    let mut object = Object::default();
    object.data.extend([(1, 0), (14, 0), (26, 0)]);
    world.register_scheduler_object(1, &mut object).unwrap();
    world.objects.insert(1, object);
    world.positions.insert(
        1,
        Placement {
            x: 16,
            y: 16,
            level: 1,
        },
    );
    let links = Links {
        size: 4,
        heads: vec![vec![0; 16]],
        next: BTreeMap::from([(1, 0)]),
    };
    let program = Program::default();
    let definition = |_: &sims_vm::Definition| Ok(1);
    let mut refs = References::new(1);
    let old;
    {
        let mut view = live::View {
            world: &mut world,
            program: &program,
            links: &links,
            definition: &definition,
        };
        old = view.lookup(1).unwrap().unwrap();
        refs.acquire(1, &view).unwrap();
        assert!(view.on_lot(old).unwrap());
        assert_eq!(view.world.objects[&1].data[&14], -1);
    }
    world.unregister_scheduler_object(1).unwrap();
    world.objects.remove(&1);
    let mut object = Object::default();
    object.data.extend([(1, 7), (14, -1), (26, 0)]);
    world.register_scheduler_object(1, &mut object).unwrap();
    world.objects.insert(1, object);
    let mut view = live::View {
        world: &mut world,
        program: &program,
        links: &links,
        definition: &definition,
    };
    let new = view.lookup(1).unwrap().unwrap();
    assert_ne!(old, new);
    assert!(view.on_lot(old).is_err());
    assert!(CaptureView::facing(&view, old).is_err());
    assert_eq!(CaptureView::facing(&view, new).unwrap(), 7);
    assert_eq!(
        refs.object(&Snapshot {
            key: 1,
            active: 1,
            ..Snapshot::default()
        })
        .unwrap(),
        Some(old)
    );
}

#[test]
fn placement_keeps_captured_receivers_and_check_position_but_rereads_level_and_slot() {
    let mut h = host();
    let mut refs = References::new(1);
    let mut s = snapshot(&mut refs, &h);
    s.container = Some(3);
    let mut c = Call::new();
    let remove = Request::RemoveFromLot(100);
    assert_eq!(next(&mut c, &s, &refs, &h), Some(remove));
    // Repointing the reference must not change the already captured receiver.
    refs.records.get_mut(&1).unwrap().object = Some(200);
    c.acknowledge(remove, Reply::Done).unwrap();
    let check = Request::Check(Place {
        object: 100,
        position: [16, 32],
        level: 1,
        container: Some(300),
        slot: -1,
    });
    assert_eq!(next(&mut c, &s, &refs, &h), Some(check));
    let before = c.clone();
    assert!(c.acknowledge(check, Reply::Done).is_err());
    assert_eq!(c, before);
    s.position = [99, 101];
    s.level = Some(2);
    s.slot = Some(4);
    s.container = Some(2);
    h.objects.insert(3, 777);
    c.acknowledge(
        check,
        Reply::Check {
            value: 0x80000001,
            position: [18, 35],
        },
    )
    .unwrap();
    let place = Request::Place(Place {
        object: 100,
        position: [18, 35],
        level: 2,
        container: Some(300),
        slot: 4,
    });
    assert_eq!(next(&mut c, &s, &refs, &h), Some(place));
    c.acknowledge(place, Reply::Done).unwrap();
    assert_eq!(next(&mut c, &s, &refs, &h), None);
    assert_eq!(c.result(), Some(true));
}

#[test]
fn placement_obeys_eight_rotation_limit_and_low_byte_acceptance() {
    for desired in [-2, 0, 7, 8, 9] {
        for answer in [0, 0x100, 1, u32::MAX] {
            let mut h = host();
            let mut refs = References::new(1);
            let mut s = snapshot(&mut refs, &h);
            s.facing = Some(desired);
            let mut c = Call::new();
            let mut rotations = 0;
            let mut placed = false;
            while let Some(r) = next(&mut c, &s, &refs, &h) {
                let reply = match r {
                    Request::RemoveFromLot(_) => Reply::Done,
                    Request::Rotate(_) => {
                        rotations += 1;
                        h.facing += 1;
                        Reply::Done
                    }
                    Request::Check(p) => Reply::Check {
                        value: answer,
                        position: p.position,
                    },
                    Request::Place(_) => {
                        placed = true;
                        Reply::Done
                    }
                };
                c.acknowledge(r, reply).unwrap();
            }
            assert_eq!(
                rotations,
                if (0..=8).contains(&desired) {
                    desired
                } else {
                    8
                }
            );
            assert_eq!(placed, (0..=8).contains(&desired) && answer as u8 != 0);
            assert_eq!(c.result(), Some(placed));
        }
    }
}

fn corpus_case(c: &serde_json::Value, restore: bool) -> serde_json::Value {
    use serde_json::json;
    let op = c["op"].as_str().unwrap();
    let mut h = host();
    let mut refs = References::new(c["next_key"].as_u64().unwrap_or(1) as u32);
    let mut s = Snapshot::default();
    s.active = c["active"].as_u64().unwrap_or(0) as u8;
    if (op == "Acquire" && c["existing"] == true) || matches!(op, "Release" | "Place") {
        refs.acquire(1, &h).unwrap();
        let r = refs.records.get_mut(&1).unwrap();
        r.key = if op == "Acquire" { 17 } else { 1 };
        r.count = c["count"].as_u64().unwrap_or(1) as u32;
        if op != "Acquire" {
            r.object = c["object"].as_u64().map(|v| v as u32);
        }
        refs.next_key = c["next_key"]
            .as_u64()
            .unwrap_or(if op == "Place" { 2 } else { 1 }) as u32;
    }
    if op == "Place" {
        s = Snapshot {
            key: c["key"].as_u64().unwrap() as u32,
            position: [16, 32],
            level: Some(1),
            facing: Some(c["desired"].as_i64().unwrap() as i32),
            container: Some(c["container"].as_i64().unwrap() as i16),
            slot: Some(-1),
            definition: Some(70),
            active: s.active,
        };
    }
    h.on_lot = c["on_lot"].as_bool().unwrap_or(true);
    h.normalized = (c["kind"] == "multipart").then_some(200);
    h.events.borrow_mut().clear();
    let mut result = None;
    match op {
        "Acquire" => {
            result = Some(
                refs.acquire(c["script"].as_i64().unwrap() as i16, &h)
                    .unwrap(),
            )
        }
        "Capture" => {
            result = Some(u32::from(
                snapshot::capture(
                    &mut s,
                    &mut refs,
                    c["script"].as_i64().unwrap() as i16,
                    &mut h,
                )
                .unwrap(),
            ))
        }
        "Release" => {
            let mut call = Release::new(c["key"].as_u64().unwrap() as u32);
            while let Some(r) = call.advance(&mut refs, &mut h).unwrap() {
                if restore {
                    (call, refs, h) =
                        serde_json::from_slice(&serde_json::to_vec(&(&call, &refs, &h)).unwrap())
                            .unwrap();
                    let before = refs.clone();
                    assert_eq!(call.advance(&mut refs, &mut h).unwrap(), Some(r));
                    assert_eq!(refs, before);
                }
                h.events
                    .borrow_mut()
                    .push(json!(["Delete", r.script_id as u16, refs.order.len()]));
                call.acknowledge(r).unwrap();
                if restore {
                    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                }
            }
            assert!(call.complete());
        }
        "Place" => {
            let mut call = Call::new();
            while let Some(r) = call.advance(&s, &refs, &h).unwrap() {
                if restore {
                    (call, s, refs, h) = serde_json::from_slice(
                        &serde_json::to_vec(&(&call, &s, &refs, &h)).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(call.advance(&s, &refs, &h).unwrap(), Some(r));
                }
                let reply = match r {
                    Request::RemoveFromLot(o) => {
                        h.events.borrow_mut().push(json!(["Remove", o, 0]));
                        Reply::Done
                    }
                    Request::Rotate(o) => {
                        h.events.borrow_mut().push(json!(["Rotate", o, 1]));
                        h.facing += 1;
                        Reply::Done
                    }
                    Request::Check(p) | Request::Place(p) => {
                        let check = matches!(r, Request::Check(_));
                        h.events.borrow_mut().push(json!([
                            if check { "Check" } else { "Place" },
                            p.object,
                            p.position,
                            p.level,
                            p.container,
                            p.slot
                        ]));
                        if check {
                            let mut position = p.position;
                            if c["mutate"] == true {
                                position[0] += 2;
                                position[1] += 3;
                                s.position = [99, 101];
                                s.level = Some(2);
                                s.slot = Some(4);
                                s.container = Some(2);
                                refs.records.get_mut(&1).unwrap().object = Some(200);
                            }
                            Reply::Check {
                                value: c["answer"].as_u64().unwrap() as u32,
                                position,
                            }
                        } else {
                            Reply::Done
                        }
                    }
                };
                call.acknowledge(r, reply).unwrap();
                if restore {
                    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                }
            }
            result = Some(u32::from(call.result().unwrap()));
        }
        _ => panic!("unknown snapshot operation"),
    }
    refs.validate().unwrap();
    if restore {
        (s, refs, h) =
            serde_json::from_slice(&serde_json::to_vec(&(&s, &refs, &h)).unwrap()).unwrap();
    }
    let mut out = json!({"events":h.events.into_inner(),"records":refs.order.iter().map(|id|&refs.records[id]).collect::<Vec<_>>(),"next_key":refs.next_key});
    if let Some(v) = result {
        out["result"] = json!(v);
    }
    if matches!(op, "Capture" | "Place") {
        out["snapshot"] = json!(s);
    }
    out
}

#[test]
#[ignore = "private prepared snapshot cases; not an original-instruction comparison"]
fn prepared_snapshot_cases_restore_pending_reference_and_placement_calls() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-object-snapshot-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(v["native_execution"], false);
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 158);
    for (n, c) in cases.iter().enumerate() {
        assert_eq!(corpus_case(c, false), corpus_case(c, true), "case {n}: {c}");
    }
}

#[test]
#[ignore = "private original snapshot corpus; generation uses the shared opt-in host-load guard"]
fn object_snapshots_match_original_callers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let v: serde_json::Value =
        serde_json::from_slice(
            &std::fs::read(root.join(
                std::env::var("SIMS_NATIVE_OBJECT_SNAPSHOT").unwrap_or_else(|_| {
                    "cache/research/sims-exe/oracle/object-snapshot.json".into()
                }),
            ))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(v["native_execution"], true);
    assert_eq!(
        v["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        v["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 158);
    for (n, c) in cases.iter().enumerate() {
        for restore in [false, true] {
            assert_eq!(
                corpus_case(&c["input"], restore),
                c["original"],
                "case {n}, restore={restore}: {}",
                c["input"]
            );
        }
    }
}
