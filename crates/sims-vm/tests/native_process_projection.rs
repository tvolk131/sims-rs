#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{
    family::Metadata,
    person::{Neighbor, WorldState},
    *,
};
use std::collections::BTreeMap;

fn report() -> Value {
    serde_json::from_str(include_str!("fixtures/family-friend-cache.json")).unwrap()
}
fn number(c: &Value, key: &str, fallback: i64) -> i64 {
    c[key].as_i64().unwrap_or(fallback)
}
fn identity(c: &Value, n: i16) -> i16 {
    c["ids"][n.to_string()].as_i64().unwrap_or(i64::from(n)) as i16
}
fn guid(c: &Value, n: i16) -> u32 {
    if n == 0 {
        0
    } else {
        c["guids"][n.to_string()]
            .as_u64()
            .unwrap_or(0x1000 + n as u64) as u32
    }
}
fn values(c: &Value, key: &str, fallback: &[i32]) -> Vec<i32> {
    c.get(key).map_or_else(
        || fallback.to_vec(),
        |v| serde_json::from_value(v.clone()).unwrap(),
    )
}
fn fixture(c: &Value) -> Result<WorldState, String> {
    let mut w = WorldState {
        controlled_family: 1,
        ..Default::default()
    };
    let slots: Vec<Option<i16>> = serde_json::from_value(c["slots"].clone()).unwrap();
    w.neighbor_slots = Some(
        slots
            .into_iter()
            .map(|n| n.map(|n| identity(c, n)))
            .collect(),
    );
    let v = [number(c, "value", 50) as i32];
    let forward = values(c, "forward", &v);
    let reverse = values(c, "reverse", &v);
    for n in [1, 2, 3, 5] {
        let rows = match n {
            2 => {
                let a: Vec<Vec<i32>> = c.get("duplicate_forward").map_or_else(
                    || vec![forward.clone()],
                    |v| serde_json::from_value(v.clone()).unwrap(),
                );
                a.into_iter()
                    .map(|v| (identity(c, 1), v))
                    .chain([(identity(c, 3), forward.clone())])
                    .collect::<Vec<_>>()
            }
            1 => vec![(identity(c, 2), values(c, "reverse_first", &reverse))],
            3 => vec![(identity(c, 2), values(c, "reverse_second", &reverse))],
            _ => vec![],
        };
        let mut relationships = BTreeMap::new();
        for (id, values) in rows {
            if relationships.insert(id, values).is_some() {
                return Err("unsupported duplicate native relationship target".into());
            }
        }
        w.neighbors.insert(
            identity(c, n),
            Neighbor {
                guid: guid(c, n),
                person: [
                    (
                        61,
                        if n == 1 || n == 3 {
                            1
                        } else {
                            number(c, "family2", 2) as i16
                        },
                    ),
                    (
                        65,
                        if n == 2 {
                            number(c, "species2", 0) as i16
                        } else {
                            0
                        },
                    ),
                ]
                .into(),
                relationships,
                ..Default::default()
            },
        );
    }
    let members: Vec<i16> = serde_json::from_value(c["members"].clone()).unwrap();
    let guids: Vec<u32> = members.iter().map(|&n| guid(c, n)).collect();
    let adapter = guids
        .iter()
        .filter_map(|g| {
            w.neighbor_slots
                .as_ref()
                .unwrap()
                .iter()
                .flatten()
                .find(|id| w.neighbors[id].guid == *g)
                .copied()
        })
        .collect();
    w.families.insert(1, adapter);
    w.families.insert(2, vec![]);
    w.family_order = Some(vec![1, 2]);
    w.family_metadata.insert(
        1,
        Metadata {
            flags: 0,
            dirty: number(c, "dirty", 1) != 0,
            friend_count: Some(number(c, "cached", -123) as i32),
            member_guids: Some(guids),
        },
    );
    w.family_metadata.insert(
        2,
        Metadata {
            flags: 0,
            dirty: false,
            friend_count: Some(234),
            member_guids: Some(vec![]),
        },
    );
    if c["entry"] == "invalidate" {
        let order: Vec<i16> = serde_json::from_value(c["family_order"].clone()).unwrap();
        let first = w.family_metadata[&1].clone();
        let second = w.family_metadata[&2].clone();
        w.families.clear();
        w.family_metadata.clear();
        for (i, &id) in order.iter().enumerate() {
            w.families.insert(id, vec![]);
            w.family_metadata.insert(
                id,
                if i == 0 {
                    first.clone()
                } else {
                    second.clone()
                },
            );
        }
        w.family_order = Some(order);
    }
    Ok(w)
}
fn constants(c: &Value) -> autonomy_constants::Owner {
    let mut owner = autonomy_constants::Owner::default();
    owner.process.words[autonomy_constants::Field::FriendshipThreshold as usize] =
        Some(number(c, "threshold", 50) as i32 as u32);
    owner
}
fn base() -> Value {
    json!({"slots":[1,null,2,3,5],"members":[1,3],"dirty":1,"cached":-123,"threshold":50,"family2":2,"species2":0,"entry":"getter"})
}

#[test]
fn family_cache_matches_79_supported_original_vectors_and_rejects_two_normalization_cases() {
    let r = report();
    assert_eq!(r["case_count"], 81);
    let mut supported = 0;
    let mut unsupported = 0;
    for row in r["cases"].as_array().unwrap() {
        let c = &row["input"];
        let o = &row["original"];
        let mut w = match fixture(c) {
            Ok(w) => w,
            Err(e) => {
                assert!(
                    c["label"]
                        .as_str()
                        .unwrap()
                        .starts_with("duplicate-relation-first-")
                );
                assert_eq!(e, "unsupported duplicate native relationship target");
                unsupported += 1;
                continue;
            }
        };
        let constants = constants(c);
        if c["entry"] == "invalidate" {
            w.mark_relationship_dirty(identity(c, 2)).unwrap();
            let order: Vec<i16> = serde_json::from_value(c["family_order"].clone()).unwrap();
            if let Some(id) = order.first() {
                assert_eq!(
                    w.family_metadata[id].friend_count,
                    Some(o["cached"].as_i64().unwrap() as i32)
                );
                assert_eq!(w.family_metadata[id].dirty, o["dirty"] != 0);
            }
            if let Some(id) = order.get(1) {
                assert_eq!(
                    w.family_metadata[id].friend_count,
                    Some(o["second_cached"].as_i64().unwrap() as i32)
                );
                assert_eq!(w.family_metadata[id].dirty, o["second_dirty"] != 0);
            }
            assert_eq!(
                w.neighbors[&identity(c, 2)].dirty,
                o["neighbor2_dirty"] != 0
            );
        } else {
            let before = w.clone();
            let inspection = w.family_friend_count(1, Some(&constants)).unwrap();
            assert_eq!(w, before);
            let count = if c["entry"] == "recompute" {
                assert_eq!(inspection, c["cached"].as_i64().unwrap() as i32);
                w.recompute_family_friend_count(1, Some(&constants))
                    .unwrap()
            } else {
                assert_eq!(inspection, o["eax"].as_i64().unwrap() as i32);
                w.resolve_family_friend_count(1, Some(&constants)).unwrap()
            };
            assert_eq!(count, o["cached"].as_i64().unwrap() as i32, "{c}");
            assert_eq!(w.family_metadata[&1].friend_count, Some(count));
            assert_eq!(w.family_metadata[&1].dirty, o["dirty"] != 0);
            if c["entry"] != "recompute" {
                assert_eq!(count as i16, o["low_i16"].as_i64().unwrap() as i16);
            }
            let mut expected = before;
            if number(c, "dirty", 1) != 0 || c["entry"] == "recompute" {
                expected.family_metadata.get_mut(&1).unwrap().dirty = false;
                expected.family_metadata.get_mut(&1).unwrap().friend_count = Some(count);
            }
            assert_eq!(w, expected, "{c}");
        }
        supported += 1;
    }
    assert_eq!((supported, unsupported), (79, 2));
}

#[test]
fn unknown_legacy_inputs_are_lazy_and_never_become_defaults() {
    let mut w = fixture(&base()).unwrap();
    let m = w.family_metadata.get_mut(&1).unwrap();
    m.dirty = false;
    m.friend_count = Some(40000);
    m.member_guids = None;
    w.neighbor_slots = None;
    let before = w.clone();
    assert_eq!(w.family_friend_count(1, None), Ok(40000));
    assert_eq!(w.resolve_family_friend_count(1, None), Ok(40000));
    assert_eq!(w, before);
    w.family_metadata.get_mut(&1).unwrap().friend_count = None;
    let before = w.clone();
    assert_eq!(
        w.resolve_family_friend_count(1, None),
        Err("native family friend cache uninitialized".into())
    );
    assert_eq!(w, before);
    w.family_metadata.get_mut(&1).unwrap().dirty = true;
    assert_eq!(
        w.family_friend_count(1, None),
        Err("native friend scan slots unavailable".into())
    );
    w.neighbor_slots = Some(vec![]);
    assert_eq!(w.resolve_family_friend_count(1, None), Ok(0));
    assert_eq!(w.family_metadata[&1].friend_count, Some(0));
    assert!(!w.family_metadata[&1].dirty);
    assert_eq!(WorldState::default().family_friends(-1, None), Ok(0));
    assert_eq!(WorldState::default().family_friends(0, None), Ok(0));
    let mut empty = base();
    empty["forward"] = json!([]);
    let mut w = fixture(&empty).unwrap();
    w.neighbors.get_mut(&2).unwrap().person.remove(&65);
    assert_eq!(w.resolve_family_friend_count(1, None), Ok(0));
}

#[test]
fn first_error_order_and_failed_recompute_preserve_old_cache() {
    let mut w = fixture(&base()).unwrap();
    w.neighbors.get_mut(&2).unwrap().person.remove(&65);
    let before = w.clone();
    assert_eq!(
        w.resolve_family_friend_count(1, None),
        Err("native friendship threshold unavailable".into())
    );
    assert_eq!(w, before);
    let constants = constants(&base());
    let before = w.clone();
    assert_eq!(
        w.resolve_family_friend_count(1, Some(&constants)),
        Err("family friend scan species word absent".into())
    );
    assert_eq!(w, before);
    let mut w = fixture(&base()).unwrap();
    w.neighbor_slots = Some(vec![Some(1)]);
    w.neighbors.get_mut(&1).unwrap().person.clear();
    assert_eq!(w.resolve_family_friend_count(1, None), Ok(0));
    let mut w = fixture(&base()).unwrap();
    w.family_metadata.get_mut(&1).unwrap().member_guids = Some(vec![0x1001]);
    w.neighbor_slots = Some(vec![Some(1), Some(2), Some(999)]);
    let before = w.clone();
    assert_eq!(
        w.resolve_family_friend_count(1, None),
        Err("friend GUID slot record absent".into())
    );
    assert_eq!(w, before);
}

fn vm(c: &Value, write: bool) -> (Program, Vm, u64) {
    let module = Module {
        source: "family-friends".into(),
        routines: [(
            4096,
            Routine {
                name: "family cache query".into(),
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
                            [9, 0, 0, 0, 0, 5, 24, 7]
                        } else {
                            [0, 0, 9, 0, 0, 5, 8, 24]
                        },
                    }],
                },
            },
        )]
        .into(),
        ..Default::default()
    };
    let p = Program {
        modules: [("family-friends".into(), module)].into(),
        ..Default::default()
    };
    let mut v = Vm::new(
        World {
            autonomy_constants: Some(constants(c)),
            person: Some(fixture(c).unwrap()),
            objects: [(1, Object::default())].into(),
            ..Default::default()
        },
        7,
    );
    let id = v
        .spawn(
            &p,
            Start {
                owner: "family-friends",
                bhav: 4096,
                caller: 1,
                callee: 1,
                stack_object: 1,
                args: vec![],
                check: false,
            },
        )
        .unwrap();
    (p, v, id)
}
#[test]
fn execution_publishes_before_nonwritable_fault_but_inspection_does_not() {
    for write in [false, true] {
        let (p, mut v, id) = vm(&base(), write);
        let before = v.clone();
        let m = v.inspect_memory(&p, id, 24, 9, write).unwrap();
        assert_eq!(m.value, 1);
        assert!(!m.writable);
        assert_eq!(v, before);
        let mut cold = Snapshot::capture(&p, &v).restore(&p).unwrap();
        let rng = (v.rng_state, v.lot_rng_state, v.crt_rng_state);
        assert_eq!(
            v.run_slice(&p, id, 10).unwrap(),
            cold.run_slice(&p, id, 10).unwrap()
        );
        assert_eq!(v, cold);
        assert_eq!((v.rng_state, v.lot_rng_state, v.crt_rng_state), rng);
        let state = v.world.person.as_ref().unwrap();
        assert_eq!(state.family_metadata[&1].friend_count, Some(1));
        assert!(!state.family_metadata[&1].dirty);
        if write {
            let Status::Faulted(f) = &v.threads[&id].status else {
                panic!()
            };
            assert!(f.message.contains("0x13"));
        } else {
            assert_eq!(v.world.objects[&1].temps[0], 1);
        }
        let bytes = serde_json::to_vec(&Snapshot::capture(&p, &v)).unwrap();
        let restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.restore(&p).unwrap(), v);
    }
}
#[test]
fn absent_caller_cannot_publish_cache_and_clean_wide_values_project_only_at_scope() {
    let (p, mut v, id) = vm(&base(), false);
    v.threads
        .get_mut(&id)
        .unwrap()
        .stack
        .last_mut()
        .unwrap()
        .caller = 99;
    let before = v.clone();
    let error = v.inspect_memory(&p, id, 24, 9, false).unwrap_err();
    assert_eq!(error, "missing object (native error 0x0a)");
    assert_eq!(v, before);
    v.run_slice(&p, id, 10).unwrap();
    let Status::Faulted(f) = &v.threads[&id].status else {
        panic!()
    };
    assert_eq!(f.message, error);
    assert_eq!(v.world, before.world);
    let mut c = base();
    c["dirty"] = json!(0);
    c["cached"] = json!(40000);
    let (p, v, id) = vm(&c, false);
    assert_eq!(
        v.world
            .person
            .as_ref()
            .unwrap()
            .family_friend_count(1, v.world.autonomy_constants.as_ref()),
        Ok(40000)
    );
    assert_eq!(
        v.inspect_memory(&p, id, 24, 9, false).unwrap().value,
        40000i32 as i16
    );
}
#[test]
fn notification_first_match_and_failure_publication_order_is_retained() {
    let mut w = fixture(&base()).unwrap();
    for m in w.family_metadata.values_mut() {
        m.dirty = false;
    }
    w.family_order = Some(vec![1, 999]);
    w.mark_relationship_dirty(1).unwrap();
    assert!(w.neighbors[&1].dirty);
    assert!(w.family_metadata[&1].dirty);
    assert!(!w.family_metadata[&2].dirty);
    let mut w = fixture(&base()).unwrap();
    w.family_order = Some(vec![999, 1]);
    let count = w.family_metadata[&1].friend_count;
    assert_eq!(
        w.mark_relationship_dirty(1),
        Err("relationship notification family header absent".into())
    );
    assert!(w.neighbors[&1].dirty);
    assert_eq!(w.family_metadata[&1].friend_count, count);
    let mut w = fixture(&base()).unwrap();
    w.family_order = Some(vec![]);
    w.neighbors.get_mut(&1).unwrap().person.clear();
    w.mark_relationship_dirty(1).unwrap();
    assert!(w.neighbors[&1].dirty);
}
#[test]
fn managed_json_roundtrip_keeps_unknowns_dirty_counts_and_authoritative_guids() {
    let (p, v, _) = vm(&base(), false);
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &v)).unwrap();
    let decoded: Snapshot = serde_json::from_slice(&bytes).unwrap();
    let mut cold = decoded.restore(&p).unwrap();
    assert_eq!(cold, v);
    cold.world
        .person
        .as_mut()
        .unwrap()
        .resolve_family_friend_count(1, cold.world.autonomy_constants.as_ref())
        .unwrap();
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &cold)).unwrap();
    let decoded: Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.restore(&p).unwrap(), cold);
    let mut legacy = serde_json::to_value(&v.world.person.as_ref().unwrap()).unwrap();
    for m in legacy["family_metadata"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        m.as_object_mut().unwrap().remove("friend_count");
        m.as_object_mut().unwrap().remove("member_guids");
    }
    let unknown: WorldState = serde_json::from_value(legacy.clone()).unwrap();
    assert!(
        unknown
            .family_metadata
            .values()
            .all(|m| m.friend_count.is_none() && m.member_guids.is_none())
    );
    assert_eq!(serde_json::to_value(&unknown).unwrap(), legacy);
    assert_eq!(
        unknown.family_friend_count(1, v.world.autonomy_constants.as_ref()),
        Err("native family friend GUID owner unavailable".into())
    );
}

#[test]
fn same_shared_word_changes_forced_projection_without_running_a_constructor() {
    let mut c = base();
    c["forward"] = json!([30]);
    c["reverse"] = json!([30]);
    let (p, mut vm, _) = vm(&c, false);
    let original = vm.clone();
    vm.world.autonomy_constants.as_mut().unwrap().process.words[10] = Some(25);
    assert_eq!(
        vm.world
            .person
            .as_ref()
            .unwrap()
            .family_friend_count(1, vm.world.autonomy_constants.as_ref()),
        Ok(1)
    );
    vm.world.autonomy_constants.as_mut().unwrap().process.words[10] = Some(50);
    assert_eq!(
        vm.world
            .person
            .as_ref()
            .unwrap()
            .family_friend_count(1, vm.world.autonomy_constants.as_ref()),
        Ok(0)
    );
    assert_eq!(vm.world.person, original.world.person);
    assert_eq!(
        (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state),
        (
            original.tick,
            original.rng_state,
            original.lot_rng_state,
            original.crt_rng_state
        )
    );
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    assert!(
        serde_json::to_value(vm.world.person.as_ref().unwrap())
            .unwrap()
            .get("friend_threshold")
            .is_none()
    );
}
