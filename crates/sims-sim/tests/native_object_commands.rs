use sims_sim::edit_history::{
    List, Store, Vector,
    command::{self, Method},
    dependencies::{Operation as D, Side},
    manager,
    object::{
        self, Class, Object, Objects, Request, SnapshotMethod, Which, budget, construction, driver,
    },
    snapshot::{self, CaptureView, References, Registry, Snapshot, lifecycle, placement},
};
use std::collections::BTreeMap;

fn cold<T: serde::Serialize + serde::de::DeserializeOwned>(x: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(x).unwrap()).unwrap()
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Host {
    captures: usize,
}
impl Registry for Host {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Ok(match id {
            11 => Some(101),
            12 => Some(102),
            21 => Some(201),
            22 => Some(202),
            _ => None,
        })
    }
    fn on_lot(&mut self, _: u32) -> Result<bool, String> {
        Ok(true)
    }
}
impl CaptureView for Host {
    fn normalized_master(&self, o: u32) -> Result<Option<u32>, String> {
        Ok((o == 102).then_some(101))
    }
    fn script_id(&self, o: u32) -> Result<i16, String> {
        match o {
            101 => Ok(11),
            102 => Ok(12),
            201 => Ok(21),
            202 => Ok(22),
            _ => Err("object absent".into()),
        }
    }
    fn capture(&mut self, o: u32) -> Result<snapshot::Position, String> {
        self.captures += 1;
        Ok(snapshot::Position {
            position: [o as i32, 32],
            level: 1,
            container: None,
            slot: -1,
        })
    }
    fn definition(&self, _: u32) -> Result<u32, String> {
        Ok(70)
    }
    fn facing(&self, _: u32) -> Result<i16, String> {
        Ok(0)
    }
}
impl placement::View for Host {
    fn facing(&self, _: u32) -> Result<i16, String> {
        Ok(0)
    }
}
impl lifecycle::View for Host {
    fn manager(&self) -> Result<u32, String> {
        Ok(900)
    }
}
impl construction::View for Host {
    fn special(&self, o: u32) -> Result<bool, String> {
        Ok(o == 101)
    }
    fn slots(&self, o: u32) -> Result<usize, String> {
        Ok(match o {
            101 => 2,
            102 => 1,
            _ => 0,
        })
    }
    fn child(&self, o: u32, n: usize) -> Result<Option<u32>, String> {
        Ok(match (o, n) {
            (101, 0) => Some(201),
            (102, 0) => Some(202),
            _ => None,
        })
    }
    fn next_part(&self, o: u32) -> Result<Option<u32>, String> {
        Ok((o == 101).then_some(102))
    }
}
fn constructed(nullable: bool) -> (Store, Objects, References, Host) {
    let (mut store, mut objects, mut refs, mut h) = (
        Store::default(),
        Objects::default(),
        References::new(1),
        Host::default(),
    );
    let mut next = 1;
    construction::construct(
        construction::Input {
            command: 1,
            script_id: 12,
            kind: 2,
            cost: 400,
            budget: 900,
        },
        &mut store,
        &mut objects,
        &mut refs,
        &mut h,
        &mut || {
            next += 1;
            Ok((!nullable).then_some(next))
        },
    )
    .unwrap();
    (store, objects, refs, h)
}
fn fixture(kind: i32) -> (Store, Objects, References) {
    let (mut store, mut objects, mut refs, h) = (
        Store::default(),
        Objects::default(),
        References::new(1),
        Host::default(),
    );
    let key = refs.acquire(11, &h).unwrap();
    for id in [1, 2, 3, 4] {
        store.construct_command(id).unwrap();
        store.command_mut(id).unwrap().state = 1;
        store.command_mut(id).unwrap().references = 1;
    }
    store.lists.insert(List::Objects(1), Vector::default());
    for id in [2, 3, 4] {
        store.append(List::Objects(1), id).unwrap();
    }
    store.pop_end(List::Objects(1)).unwrap();
    let saved = Snapshot {
        key,
        position: [16, 32],
        level: Some(1),
        facing: Some(0),
        container: Some(0),
        slot: Some(-1),
        definition: Some(70),
        active: 1,
    };
    objects.0.insert(
        1,
        Object {
            cost: Some(400),
            budget: Some(900),
            kind,
            special: Some(0),
            snapshots: [saved.clone(), saved],
            class: Class::Object,
        },
    );
    (store, objects, refs)
}
fn call(method: Method) -> object::Call {
    object::Call::new(command::Request { command: 1, method }).unwrap()
}

#[test]
fn constructor_uses_normalized_master_and_occupied_slots_across_all_parts() {
    let (s, o, r, h) = constructed(false);
    assert_eq!(h.captures, 3);
    assert_eq!(o.0.len(), 3);
    let list = List::Objects(1);
    let first = s.begin(list).unwrap();
    assert_eq!(s.read(first).unwrap(), 2);
    assert_eq!(s.read(first.next().unwrap()).unwrap(), 3);
    for (id, script, cost, special) in [(1, 11, 400, 1), (2, 21, 0, 0), (3, 22, 0, 0)] {
        let c = o.get(id).unwrap();
        assert_eq!(r.script_id(&c.snapshots[0]).unwrap(), script);
        assert_eq!(
            (c.cost, c.budget, c.kind, c.special),
            (Some(cost), Some(900), 2, Some(special))
        );
        assert_eq!(c.snapshots[1], Snapshot::default());
        assert_eq!(s.command(id).unwrap().references, 0);
    }
    s.validate().unwrap();
    r.validate().unwrap();
}

#[test]
fn nullable_child_allocation_is_retained_and_destruction_skips_only_null_children() {
    let (mut s, mut o, mut r, mut h) = constructed(true);
    assert_eq!(h.captures, 1);
    assert_eq!(o.0.len(), 1);
    assert_eq!(s.read(s.begin(List::Objects(1)).unwrap()).unwrap(), 0);
    let mut c = driver::Execution::new(command::Request {
        command: 1,
        method: Method::Destroy,
    })
    .unwrap();
    let mut b = budget::Budgets::default();
    assert_eq!(
        c.advance(&mut s, &mut o, &mut r, &mut b, &mut h, 20)
            .unwrap(),
        driver::Step::Complete(1)
    );
    assert!(s.commands.is_empty() && o.0.is_empty() && r.records.is_empty());
    s.validate().unwrap();
}

#[test]
fn undo_failure_and_redo_failure_preserve_the_original_asymmetry() {
    for method in [Method::Undo, Method::Redo] {
        for kind in [0, 1, 2, 3] {
            let (mut s, mut o, r) = fixture(kind);
            let mut c = call(method);
            let mut events = vec![];
            while let Some(req) = c.advance(&mut s, &mut o, &r).unwrap() {
                c = cold(&c);
                let before = (s.clone(), o.clone());
                assert_eq!(c.advance(&mut s, &mut o, &r).unwrap(), Some(req));
                assert_eq!((s.clone(), o.clone()), before);
                events.push(req);
                c.acknowledge(req, 0).unwrap();
            }
            assert_eq!(c.result(), Some(u32::from(method == Method::Undo)));
            assert_eq!(
                events
                    .iter()
                    .filter(|r| matches!(r, Request::Virtual(_)))
                    .count(),
                2
            );
            let deps: Vec<_> = events
                .iter()
                .filter_map(|r| {
                    if let Request::Dependencies { operation, .. } = r {
                        Some(*operation)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                deps,
                if method == Method::Undo {
                    vec![D::Undo(Side::Before)]
                } else {
                    vec![D::Redo(Side::After), D::Redo(Side::Before)]
                }
            );
            assert!(!events.iter().any(|r| matches!(r, Request::Budget { .. })));
            assert_eq!(s.command(2).unwrap().state, 1);
        }
    }
}

#[test]
fn redo_rechecks_kind_after_children_and_budget_reads_post_snapshot_fields() {
    let (mut s, mut o, r) = fixture(3);
    let mut c = call(Method::Redo);
    let mut children = vec![];
    let mut budget_seen = false;
    while let Some(req) = c.advance(&mut s, &mut o, &r).unwrap() {
        let answer = match req {
            Request::Virtual(v) => {
                children.push(v.command);
                o.get_mut(1).unwrap().kind = 0;
                0
            }
            Request::Snapshot { which, method, .. } => {
                assert_eq!((which, method), (Which::Initial, SnapshotMethod::Remove));
                let parent = o.get_mut(1).unwrap();
                parent.cost = Some(i32::MIN);
                parent.budget = Some(901);
                parent.special = Some(255);
                1
            }
            Request::Budget {
                owner,
                amount,
                special,
            } => {
                assert_eq!((owner, amount, special), (901, i32::MIN, 255));
                budget_seen = true;
                0
            }
            _ => 0,
        };
        c.acknowledge(req, answer).unwrap();
    }
    assert!(budget_seen);
    assert_eq!(children, vec![2, 3, 2, 3]);
    assert_eq!(c.result(), Some(0));
}

#[test]
fn destruction_orders_snapshots_before_base_clear_and_frees_without_releasing_new_children() {
    let (mut s, mut o, r) = fixture(2);
    let mut c = call(Method::Destroy);
    let mut events = vec![];
    while let Some(req) = c.advance(&mut s, &mut o, &r).unwrap() {
        events.push(req);
        match req {
            Request::Virtual(v) => {
                assert_eq!(v.method, Method::Destroy);
                s.deallocate_command(v.command).unwrap();
            }
            Request::Snapshot {
                which: Which::Final,
                ..
            } => {
                s.append(List::Objects(1), 4).unwrap();
            }
            Request::Dependencies {
                operation: D::Clear,
                ..
            } => {
                assert!(!s.lists.contains_key(&List::Objects(1)));
                assert_eq!(o.get(1).unwrap().class, Class::Base);
                //Native base destruction frees a Before list even if a later
                //After callback appended to it after Clear visited it.
                s.append(List::Before(1), 4).unwrap();
            }
            _ => (),
        }
        c = cold(&c);
        c.acknowledge(req, 0).unwrap();
    }
    assert_eq!(events.len(), 5);
    assert_eq!(
        events[2],
        Request::Snapshot {
            owner: 1,
            which: Which::Final,
            method: SnapshotMethod::Destroy
        }
    );
    assert_eq!(
        events[3],
        Request::Snapshot {
            owner: 1,
            which: Which::Initial,
            method: SnapshotMethod::Destroy
        }
    );
    assert_eq!(c.result(), Some(1));
    assert!(o.0.is_empty());
    assert_eq!(s.commands.len(), 1);
    assert_eq!(s.command(4).unwrap().references, 1);
    s.validate().unwrap();
}

#[test]
fn budget_keeps_wrapping_amount_and_restores_category_after_callback_writes() {
    for amount in [i32::MIN, -17, 0, 17, i32::MAX] {
        for special in [0, 1, 255] {
            let mut b = budget::Budgets(BTreeMap::from([(
                900,
                budget::State {
                    pending: i32::MAX,
                    category: 123,
                },
            )]));
            let mut c = budget::Call::new(900, amount, special);
            let r = c.advance(&mut b).unwrap().unwrap();
            assert_eq!(
                r,
                budget::Apply {
                    category: 6 + i32::from(special != 0),
                    amount: i32::MAX.wrapping_add(amount)
                }
            );
            (c, b) = cold(&(c, b));
            let before = (c.clone(), b.clone());
            assert!(c.acknowledge(budget::Apply { amount: 1, ..r }).is_err());
            assert_eq!((c.clone(), b.clone()), before);
            assert_eq!(c.advance(&mut b).unwrap(), Some(r));
            assert_eq!(b, before.1);
            b.0.get_mut(&900).unwrap().pending = 999;
            b.0.get_mut(&900).unwrap().category = 456;
            c.acknowledge(r).unwrap();
            assert_eq!(c.advance(&mut b).unwrap(), None);
            assert!(c.complete());
            assert_eq!(
                b.0[&900],
                budget::State {
                    pending: 0,
                    category: 123
                }
            );
        }
    }
}

#[test]
fn saved_object_command_stack_rejects_a_different_virtual_child() {
    let (mut s, mut o, mut r, mut h) = constructed(false);
    let mut budgets = budget::Budgets::default();
    let request = command::Request {
        command: 1,
        method: Method::Execute,
    };
    let mut c = driver::Execution::new(request).unwrap();
    assert_eq!(c.root(), Some(request));
    c.validate().unwrap();
    assert_eq!(
        c.advance(&mut s, &mut o, &mut r, &mut budgets, &mut h, 1)
            .unwrap(),
        driver::Step::Budget
    );
    c = cold(&c);
    c.validate().unwrap();
    let before = (s.clone(), o.clone(), r.clone(), budgets.clone());
    let mut bad = serde_json::to_value(&c).unwrap();
    assert_eq!(bad["frames"].as_array().unwrap().len(), 2);
    bad["frames"][1]["call"]["request"]["command"] = 999.into();
    assert!(
        serde_json::from_value::<driver::Execution>(bad)
            .unwrap()
            .validate()
            .is_err()
    );
    assert_eq!((s, o, r, budgets), before);
}

#[test]
fn concrete_nested_commands_capture_and_destroy_through_the_history_bridge() {
    let (mut s, mut o, mut r, mut h) = constructed(false);
    let mut b = budget::Budgets::default();
    let mut execute = driver::Execution::new(command::Request {
        command: 1,
        method: Method::Execute,
    })
    .unwrap();
    loop {
        execute.validate().unwrap();
        match execute
            .advance(&mut s, &mut o, &mut r, &mut b, &mut h, 1)
            .unwrap()
        {
            driver::Step::Budget => (execute, s, o, r, b, h) = cold(&(execute, s, o, r, b, h)),
            driver::Step::Complete(result) => {
                assert_eq!(result, 0);
                break;
            }
            other => panic!("unexpected service: {other:?}"),
        }
    }
    assert_eq!(h.captures, 6);
    assert!(r.records.values().all(|r| r.count == 2));
    assert!(
        s.commands
            .values()
            .all(|c| c.state == 0 && c.references == 0)
    );
    s.command_mut(1).unwrap().references = 1;
    let mut history = sims_sim::edit_history::tool::Call::new(
        sims_sim::object_catalog::move_tool::cleanup::Request::ReleasePayload(1),
    )
    .unwrap()
    .unwrap();
    let virtual_call = command::Request {
        command: 1,
        method: Method::Destroy,
    };
    assert_eq!(
        history.advance(&mut s).unwrap(),
        Some(manager::Service::Virtual(virtual_call))
    );
    let mut destroy = driver::Execution::new(virtual_call).unwrap();
    loop {
        destroy.validate().unwrap();
        match destroy
            .advance(&mut s, &mut o, &mut r, &mut b, &mut h, 1)
            .unwrap()
        {
            driver::Step::Budget => (destroy, s, o, r, b, h) = cold(&(destroy, s, o, r, b, h)),
            driver::Step::Complete(result) => {
                history
                    .acknowledge(manager::Service::Virtual(virtual_call), result)
                    .unwrap();
                break;
            }
            other => panic!("unexpected destructor service: {other:?}"),
        }
    }
    assert_eq!(history.advance(&mut s).unwrap(), None);
    assert!(history.complete());
    assert!(s.commands.is_empty() && o.0.is_empty() && r.records.is_empty());
    s.validate().unwrap();
}

fn corpus_case(c: &serde_json::Value, restore: bool) -> serde_json::Value {
    use serde_json::json;
    let op = c["op"].as_str().unwrap();
    let mut events = vec![];
    if op == "Budget" {
        let mut b = budget::Budgets(BTreeMap::from([(
            900,
            budget::State {
                pending: c["pending"].as_i64().unwrap() as i32,
                category: 123,
            },
        )]));
        let mut call = budget::Call::new(
            900,
            c["amount"].as_i64().unwrap() as i32,
            c["special"].as_u64().unwrap() as u8,
        );
        while let Some(r) = call.advance(&mut b).unwrap() {
            if restore {
                (call, b) = cold(&(call, b));
                let before = b.clone();
                assert_eq!(call.advance(&mut b).unwrap(), Some(r));
                assert_eq!(b, before);
            }
            events.push(json!(["Apply", r.category, r.amount]));
            if c["mutate"].as_bool().unwrap() {
                b.0.get_mut(&900).unwrap().pending = 999;
                b.0.get_mut(&900).unwrap().category = 456;
            }
            call.acknowledge(r).unwrap();
        }
        return json!({"events":events,"pending":b.0[&900].pending,"category":b.0[&900].category});
    }
    let method = match op {
        "Execute" => Method::Execute,
        "Undo" => Method::Undo,
        "Redo" => Method::Redo,
        "Destroy" => Method::Destroy,
        _ => panic!("unknown op"),
    };
    let (mut store, mut objects, mut refs) = fixture(c["kind"].as_i64().unwrap() as i32);
    let list = List::Objects(1);
    store
        .erase(list, store.begin(list).unwrap(), store.end(list).unwrap())
        .unwrap();
    for child in c["children"].as_array().unwrap() {
        store.append(list, child.as_u64().unwrap() as u32).unwrap();
    }
    let mut call = call(method);
    let mut patched = false;
    let mutation = c["mutation"].as_str().unwrap();
    while let Some(r) = call.advance(&mut store, &mut objects, &refs).unwrap() {
        if restore {
            (call, store, objects, refs) = cold(&(call, store, objects, refs));
            let before = (call.clone(), store.clone(), objects.clone(), refs.clone());
            assert_eq!(
                call.advance(&mut store, &mut objects, &refs).unwrap(),
                Some(r)
            );
            assert_eq!(
                (call.clone(), store.clone(), objects.clone(), refs.clone()),
                before
            );
        }
        let value = match r {
            Request::Capture { script_id, .. } => {
                events.push(json!(["Capture", script_id]));
                c["answer"].as_u64().unwrap() as u32
            }
            Request::Dependencies { operation, .. } => {
                let name = match operation {
                    D::Undo(Side::Before) => "UndoBefore",
                    D::Undo(Side::After) => "UndoAfter",
                    D::Redo(Side::After) => "RedoAfter",
                    D::Redo(Side::Before) => "RedoBefore",
                    D::Clear => "Clear",
                    _ => panic!("unexpected dependency"),
                };
                events.push(json!(["Dependencies", name]));
                c[if matches!(name, "UndoBefore" | "RedoAfter") {
                    "first"
                } else {
                    "last"
                }]
                .as_u64()
                .unwrap() as u32
            }
            Request::Snapshot { which, method, .. } => {
                events.push(json!([
                    "Snapshot",
                    format!("{which:?}"),
                    format!("{method:?}")
                ]));
                if mutation == "snapshot_fields" && !patched {
                    let o = objects.get_mut(1).unwrap();
                    o.cost = Some(i32::MIN);
                    o.budget = Some(901);
                    o.special = Some(255);
                    o.kind = 3;
                    patched = true;
                }
                if mutation == "after_erase" && which == Which::Final {
                    store.append(list, 4).unwrap();
                }
                c["answer"].as_u64().unwrap() as u32
            }
            Request::Virtual(v) => {
                let command = store.command(v.command).unwrap();
                events.push(json!([
                    "Virtual",
                    format!("{:?}", v.method),
                    v.command,
                    command.state,
                    command.references
                ]));
                if v.method == Method::Destroy {
                    store.deallocate_command(v.command).unwrap();
                } else {
                    store.command_mut(v.command).unwrap().state = 99;
                }
                if !patched {
                    if mutation == "child_kind" {
                        let o = objects.get_mut(1).unwrap();
                        o.kind = if o.kind == 3 { 0 } else { 3 };
                        patched = true;
                    }
                    if mutation == "grow_children" {
                        store.append(list, 4).unwrap();
                        patched = true;
                    }
                }
                0xabcdef01
            }
            Request::Budget {
                owner,
                amount,
                special,
            } => {
                events.push(json!(["Budget", owner, amount, special]));
                0
            }
        };
        call.acknowledge(r, value).unwrap();
    }
    let owner = objects
        .0
        .get(&1)
        .map(|o| json!({"kind":o.kind,"cost":o.cost,"budget":o.budget,"special":o.special}));
    let children = objects.0.contains_key(&1).then(|| {
        let mut p = store.begin(list).unwrap();
        let mut children = vec![];
        while p != store.end(list).unwrap() {
            children.push(store.read(p).unwrap());
            p = p.next().unwrap();
        }
        children
    });
    let states: Vec<_> = (1..=4)
        .map(|id| store.commands.get(&id).map(|c| [c.state, c.references]))
        .collect();
    store.validate().unwrap();
    json!({"result":call.result().unwrap(),"events":events,"owner":owner,"children":children,"states":states})
}

#[test]
#[ignore = "private prepared caller cases; not original-instruction validation"]
fn prepared_object_command_cases_restore_every_service() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-object-commands-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], false);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 336);
    for c in cases {
        assert_eq!(corpus_case(c, false), corpus_case(c, true), "{c}");
    }
}

#[test]
#[ignore = "original caller corpus; generate only with all Cargo/rustc absent"]
fn object_command_callers_match_original_instructions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/object-commands.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 336);
    for row in cases {
        for cold in [false, true] {
            assert_eq!(
                corpus_case(&row["input"], cold),
                row["original"],
                "{}, restored={cold}",
                row["input"]
            );
        }
    }
}
