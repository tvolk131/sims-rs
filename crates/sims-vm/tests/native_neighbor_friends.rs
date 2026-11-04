use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    person::{Neighbor, WorldState},
    *,
};
use std::collections::BTreeMap;

fn fixture(case: &Value) -> (Program, Vm, ThreadId) {
    let write = case["write"] == true;
    let mut module = Module {
        source: "friends".into(),
        ..Default::default()
    };
    module.routines.insert(
        4096,
        Routine {
            name: "friend count resolver".into(),
            body: Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 255,
                    operands: if write {
                        [5, 0, 0, 0, 0, 5, 24, 7]
                    } else {
                        [0, 0, 5, 0, 0, 5, 8, 24]
                    },
                }],
            },
        },
    );
    let resources = if case["resource"] == true {
        [1, 2, 5]
            .map(|id| (id, case["flags"].as_u64().unwrap() as u32))
            .into()
    } else {
        BTreeMap::new()
    };
    let program = Program {
        modules: [("friends".into(), module)].into(),
        neighbor_resource_flags: Some(resources),
        ..Default::default()
    };
    let mut people = WorldState {
        neighbor_slots: Some(serde_json::from_value(case["slots"].clone()).unwrap()),
        ..Default::default()
    };
    for id in [1, 2, 5] {
        let values: Vec<i32> =
            serde_json::from_value(case[if id == 1 { "forward" } else { "reverse" }].clone())
                .unwrap();
        people.neighbors.insert(
            id,
            Neighbor {
                guid: id as u32,
                person: [(61, case["family"].as_i64().unwrap() as i16)].into(),
                dirty: case["dirty"] == true,
                friend_count: Some(case["cached"].as_i64().unwrap() as i32),
                relationships: if id == 5 {
                    BTreeMap::new()
                } else {
                    [(3 - id, values)].into()
                },
                ..Default::default()
            },
        );
    }
    let mut constants = autonomy_constants::Owner::default();
    constants.process.words[10] = Some(25); // supplied native vector input, loaded remains unknown
    let mut vm = Vm::new(
        World {
            autonomy_constants: Some(constants),
            person: Some(people),
            objects: [(1, Object::default())].into(),
            ..Default::default()
        },
        7,
    );
    let id = vm
        .spawn(
            &program,
            Start {
                owner: "friends",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: case["id"].as_i64().unwrap() as i16,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (program, vm, id)
}

fn base() -> Value {
    json!({"id":1,"dirty":true,"cached":-123,"flags":4,"family":1,
        "forward":[25],"reverse":[25],"write":false,"resource":true,"slots":[1,2,5]})
}

#[test]
fn friend_cache_updates_during_resolution_but_inspection_is_read_only() {
    for write in [false, true] {
        let mut c = base();
        c["write"] = json!(write);
        let (p, mut vm, id) = fixture(&c);
        let before = vm.clone();
        let read = vm.inspect_memory(&p, id, 24, 5, write).unwrap();
        assert_eq!(read.value, 1);
        assert!(!read.writable);
        assert_eq!(vm, before);
        let mut restored = Snapshot::capture(&p, &vm).restore(&p).unwrap();
        assert_eq!(
            vm.run_slice(&p, id, 10).unwrap(),
            restored.run_slice(&p, id, 10).unwrap()
        );
        assert_eq!(vm, restored);
        let n = &vm.world.person.as_ref().unwrap().neighbors[&1];
        assert_eq!(n.friend_count, Some(1));
        assert!(n.dirty);
        if write {
            let Status::Faulted(f) = &vm.threads[&id].status else {
                panic!()
            };
            assert!(f.message.contains("0x13"));
        } else {
            assert_eq!(vm.world.objects[&1].temps[0], 1);
        }
        assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    }
}

#[test]
fn friend_query_requires_real_category_and_preserves_clean_or_unknown_cache() {
    for (flags, family, forward, reverse, expected) in [
        (4, 1, 25, 25, 1),
        (4, -1, 25, 25, 1),
        (0x104, 1, 25, 25, 1),
        (0, 1, 25, 25, 0),
        (8, 1, 25, 25, 0),
        (12, 1, 25, 25, 0),
        (4, 0, 25, 25, 0),
        (4, 1, 24, 25, 0),
        (4, 1, 25, 24, 0),
    ] {
        let mut c = base();
        c["flags"] = json!(flags);
        c["family"] = json!(family);
        c["forward"] = json!([forward]);
        c["reverse"] = json!([reverse]);
        let (p, mut vm, id) = fixture(&c);
        vm.run_slice(&p, id, 10).unwrap();
        assert_eq!(vm.world.objects[&1].temps[0], expected, "{c}");
    }
    let (mut p, mut vm, id) = fixture(&base());
    p.neighbor_resource_flags = None;
    assert!(
        vm.inspect_memory(&p, id, 24, 5, false)
            .unwrap_err()
            .contains("catalog unavailable")
    );
    let n = vm
        .world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&1)
        .unwrap();
    n.dirty = false;
    n.friend_count = Some(40000);
    assert_eq!(
        vm.inspect_memory(&p, id, 24, 5, false).unwrap().value,
        40000u16 as i16
    );
    vm.world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&1)
        .unwrap()
        .friend_count = None;
    assert!(
        vm.inspect_memory(&p, id, 24, 5, false)
            .unwrap_err()
            .contains("cache uninitialized")
    );
    vm.threads
        .get_mut(&id)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .stack_object = 0;
    assert_eq!(vm.inspect_memory(&p, id, 24, 5, false).unwrap().value, 0);
}

#[test]
fn invalid_caller_cannot_publish_a_friend_cache() {
    for write in [false, true] {
        let mut c = base();
        c["write"] = json!(write);
        let (p, mut vm, id) = fixture(&c);
        vm.threads
            .get_mut(&id)
            .unwrap()
            .stack
            .last_mut()
            .unwrap()
            .caller = 99;
        let before = vm.world.clone();
        let error = vm.inspect_memory(&p, id, 24, 5, write).unwrap_err();
        assert!(error.contains("missing object"));
        vm.run_slice(&p, id, 10).unwrap();
        let Status::Faulted(f) = &vm.threads[&id].status else {
            panic!()
        };
        assert_eq!(f.message, error);
        assert_eq!(vm.world, before);
    }
}

#[test]
fn relationship_notification_makes_unknown_cache_recomputable_after_resume() {
    let mut c = base();
    c["reverse"] = json!([24]);
    let (mut p, mut vm, id) = fixture(&c);
    vm.world.objects.get_mut(&1).unwrap().person = Some([(31, 2)].into());
    for neighbor in vm.world.person.as_mut().unwrap().neighbors.values_mut() {
        neighbor.dirty = false;
        neighbor.friend_count = None;
    }
    p.modules
        .get_mut("friends")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions
        .insert(
            0,
            Instruction {
                opcode: 26,
                on_true: 1,
                on_false: 255,
                operands: [0, 0, 6, 0, 7, 0, 25, 0],
            },
        );
    vm.run_slice(&p, id, 1).unwrap();
    for neighbor in [1, 2] {
        let n = &vm.world.person.as_ref().unwrap().neighbors[&neighbor];
        assert!(n.dirty);
        assert_eq!(n.friend_count, None);
    }
    let mut resumed = Snapshot::capture(&p, &vm).restore(&p).unwrap();
    assert_eq!(
        vm.run_slice(&p, id, 10).unwrap(),
        resumed.run_slice(&p, id, 10).unwrap()
    );
    assert_eq!(vm, resumed);
    assert_eq!(vm.world.objects[&1].temps[0], 1);
    let n = &vm.world.person.as_ref().unwrap().neighbors[&1];
    assert_eq!(n.friend_count, Some(1));
    assert!(n.dirty);
}

#[test]
#[ignore = "requires SIMS_NEIGHBOR_FRIEND_ORACLE generated from original engine"]
fn neighbor_friend_count_matches_original_resolver_and_cache() {
    let path = std::env::var("SIMS_NEIGHBOR_FRIEND_ORACLE").expect("explicit native evidence path");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 551);
    for row in rows {
        let c = &row["input"];
        let (p, mut vm, id) = fixture(c);
        let read = vm
            .inspect_memory(&p, id, 24, 5, c["write"] == true)
            .unwrap();
        vm.run_slice(&p, id, 10).unwrap();
        let n = &vm.world.person.as_ref().unwrap().neighbors;
        let actual = json!({"value":read.value,"writable":read.writable,
            "cached":[n[&1].friend_count,n[&2].friend_count,n[&5].friend_count],
            "dirty":[u8::from(n[&1].dirty),u8::from(n[&2].dirty),u8::from(n[&5].dirty)]});
        assert_eq!(actual, row["original"], "{c}");
    }
    eprintln!(
        "{} original neighbor friend-count cases matched",
        rows.len()
    );
}
