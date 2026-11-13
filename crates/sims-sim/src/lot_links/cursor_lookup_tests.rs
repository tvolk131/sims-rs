//! Independent frozen old Cursor::advance, plus malformed/continuation tests.
//! The reference still calls unchanged Links::slot; it never uses the borrowed
//! lookup path introduced by this change. All fixture state is synthetic.
use super::*;
use sims_vm::{Object, person::WorldState};

fn reference_advance(cursor: &mut Cursor, links: &Links, world: &World) -> Result<(), String> {
    let Some(mut node) = cursor.current else {
        return Ok(());
    };
    let mut slot = 0;
    let mut seen = Seen::default();
    loop {
        if !seen.insert(node) {
            return Err("cyclic physical iterator ancestry".into());
        }
        let count = world
            .objects
            .get(&node)
            .ok_or("physical iterator object absent")?
            .slots
            .len()
            + 1;
        while slot < count as i32 {
            let child = links.slot(world, node, slot)?;
            slot += 1;
            if child.is_some() {
                cursor.current = child;
                return Ok(());
            }
        }
        cursor.current = None;
        if node == cursor.root {
            return Ok(());
        }
        let parent = Links::registered(world, Links::word(world, node, 26)?)
            .ok_or("physical iterator lost its predecessor")?;
        slot = i32::from(Links::word(world, node, 14)?) + 1;
        if slot < 0 {
            return Err("physical iterator has a negative slot".into());
        }
        node = parent;
    }
}

fn reference_walk(l: &Links, w: &World, root: ObjectId) -> Result<Vec<ObjectId>, String> {
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    let mut cursor = Cursor::new(root);
    while let Some(id) = cursor.current {
        if !seen.insert(id) {
            return Err("cyclic or duplicate physical slot traversal".into());
        }
        result.push(id);
        reference_advance(&mut cursor, l, w)?;
    }
    Ok(result)
}
fn reference_collect(l: &Links, w: &World) -> Result<Vec<Vec<ObjectId>>, String> {
    l.validate_arrays()?;
    let active = w
        .map_size
        .map(|map| map.checked_size(l.size))
        .transpose()?
        .unwrap_or(l.size);
    let mut chains = vec![];
    let mut seen = BTreeSet::new();
    for head in l.heads.iter().flat_map(|floor| {
        floor
            .chunks_exact(l.size)
            .take(active)
            .flat_map(move |column| column[..active].iter())
    }) {
        if let Some(id) = Links::registered(w, *head) {
            let chain = reference_walk(l, w, id)?;
            if chain.iter().any(|id| !seen.insert(*id)) {
                return Err("object occurs in multiple native tile chains".into());
            }
            chains.push(chain);
        }
    }
    Ok(chains)
}
fn reference_validate(l: &Links, w: &World) -> Result<(), String> {
    let actual = reference_collect(l, w)?;
    let expected = &w
        .person
        .as_ref()
        .ok_or("tile registry unavailable")?
        .tile_chains;
    if &actual != expected {
        return Err("native runtime physical tile order mismatch".into());
    }
    Ok(())
}
fn fixture() -> (Links, World) {
    let mut w = World {
        person: Some(WorldState::default()),
        ..Default::default()
    };
    let mut l = Links {
        size: 6,
        heads: vec![vec![0; 36]; 2],
        next: BTreeMap::new(),
    };
    for id in [i16::MIN, -2, -1, 0, 1, 2, 3, 4, 5, 6, 17, i16::MAX] {
        let (parent, slot) = match id {
            2 => (1, 0),
            3 => (1, 1),
            4 => (3, 1),
            5 => (1, 3),
            _ => (0, -1),
        };
        w.objects.insert(
            id,
            Object {
                data: [(14, slot), (26, parent)].into(),
                slots: match id {
                    1 => vec![Some(3), None, Some(5)],
                    3 => vec![Some(4)],
                    _ => vec![],
                },
                ..Default::default()
            },
        );
        w.positions.insert(
            id,
            Placement {
                x: if id > 0 { 32 } else { -16 },
                y: 32,
                level: if id == 6 { 2 } else { 1 },
            },
        );
        l.next.insert(id, if id == 1 { 2 } else { 0 });
    }
    l.heads[0][14] = 1;
    l.heads[1][21] = 6;
    w.person.as_mut().unwrap().tile_chains = vec![vec![1, 2, 3, 4, 5], vec![6]];
    (l, w)
}
fn first_failure(
    context: &str,
    l: &Links,
    w: &World,
    before: &Cursor,
    old: &Cursor,
    new: &Cursor,
    old_result: &Result<(), String>,
    new_result: &Result<(), String>,
) {
    let Some(path) = std::env::var_os("SIMS_CURSOR_LOOKUP_FIRST_FAILURE") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        use std::io::Write;
        let bytes=serde_json::to_vec_pretty(&serde_json::json!({"context":context,"links":l,"world":w,
            "before":before,"reference":old,"actual":new,"reference_result":old_result,"actual_result":new_result})).unwrap();
        file.write_all(&bytes).unwrap();
    }
}
fn check(context: &str, l: &Links, w: &World, before: Cursor) -> (Result<(), String>, Cursor) {
    let links_before = l.clone();
    let world_before = w.clone();
    let bytes_before = serde_json::to_vec(&(l, w)).unwrap();
    let mut old = before.clone();
    let mut new = before.clone();
    let a = reference_advance(&mut old, l, w);
    let b = new.advance(l, w);
    if a != b || old != new {
        first_failure(context, l, w, &before, &old, &new, &a, &b);
    }
    assert_eq!(a, b, "{context}; input {before:?}");
    assert_eq!(old, new, "{context}; input {before:?}");
    assert_eq!(
        serde_json::to_vec(&old).unwrap(),
        serde_json::to_vec(&new).unwrap()
    );
    assert_eq!(*l, links_before);
    assert_eq!(*w, world_before);
    assert_eq!(serde_json::to_vec(&(l, w)).unwrap(), bytes_before);
    (b, new)
}
fn object(w: &mut World, id: ObjectId, f: impl FnOnce(&mut Object)) {
    if let Some(o) = w.objects.get_mut(&id) {
        f(o)
    }
}
fn word(w: &mut World, id: ObjectId, index: i16, value: i16) {
    object(w, id, |o| {
        o.data.insert(index, value);
    });
}
type Mutation = fn(&mut Links, &mut World);
fn mutations() -> Vec<Mutation> {
    vec![
        |_, _| {},
        |_, w| {
            w.objects.remove(&1);
        },
        |_, w| {
            w.objects.remove(&2);
        },
        |_, w| {
            w.objects.remove(&3);
        },
        |l, _| {
            l.next.remove(&1);
        },
        |l, _| {
            l.next.remove(&2);
        },
        |l, _| {
            l.next.insert(1, i16::MIN);
        },
        |l, _| {
            l.next.insert(1, 99);
        },
        |l, _| {
            l.next.insert(1, 1);
        },
        |_, w| object(w, 1, |o| o.slots.clear()),
        |_, w| object(w, 1, |o| o.slots = vec![None]),
        |_, w| object(w, 1, |o| o.slots = vec![Some(99)]),
        |_, w| object(w, 1, |o| o.slots = vec![Some(-1)]),
        |_, w| object(w, 3, |o| o.slots = vec![Some(3)]),
        |_, w| {
            object(w, 2, |o| {
                o.data.remove(&26);
            })
        },
        |_, w| {
            object(w, 2, |o| {
                o.data.remove(&14);
            })
        },
        |_, w| word(w, 2, 26, 0),
        |_, w| word(w, 2, 26, -1),
        |_, w| word(w, 2, 26, 99),
        |_, w| word(w, 2, 26, 2),
        |_, w| word(w, 2, 14, i16::MIN),
        |_, w| word(w, 2, 14, -2),
        |_, w| word(w, 2, 14, -1),
        |_, w| word(w, 2, 14, i16::MAX),
        |_, w| word(w, 2, 14, 200),
        |_, w| w.person = None,
        |l, _| l.size = 1,
        |l, _| {
            l.heads[0].pop();
        },
        |l, w| {
            w.map_size = Some(sims_vm::world_map::State { active_size: 4 });
            if let Some(head) = l.heads[0].get_mut(35) {
                *head = 1;
            }
        },
        |l, _| {
            l.next.insert(1, i16::MAX);
        },
    ]
}

#[test]
fn ordered_corruption_pairs_preserve_cursor_result_state_and_tile_checks() {
    let changes = mutations();
    let mut cursors = 0;
    let mut walks = 0;
    for (a, first) in changes.iter().enumerate() {
        for (b, second) in changes.iter().enumerate() {
            let (mut l, mut w) = fixture();
            first(&mut l, &mut w);
            second(&mut l, &mut w);
            let context = format!("mutation pair {a}/{b}");
            for (root, current) in [
                (1, Some(1)),
                (1, Some(2)),
                (1, Some(3)),
                (1, Some(4)),
                (6, Some(2)),
                (17, Some(2)),
                (i16::MIN, Some(i16::MIN)),
                (0, Some(0)),
                (-1, Some(1)),
                (i16::MAX, Some(i16::MAX)),
                (99, Some(99)),
                (1, None),
            ] {
                let _ = check(&context, &l, &w, Cursor { root, current });
                cursors += 1;
            }
            for root in [i16::MIN, -2, -1, 0, 1, 2, 3, 4, 5, 6, 17, i16::MAX, 99] {
                assert_eq!(
                    l.walk(&w, root),
                    reference_walk(&l, &w, root),
                    "{context}, root {root}"
                );
                walks += 1;
            }
            assert_eq!(
                l.collect_tile_chains(&w),
                reference_collect(&l, &w),
                "{context}"
            );
            assert_eq!(
                l.validate_tile_chains(&w),
                reference_validate(&l, &w),
                "{context}"
            );
        }
    }
    println!(
        "cursor differentials={cursors}; complete walks={walks}; tile collect/validate pairs={}",
        changes.len() * changes.len()
    );
}

fn random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
#[test]
fn seeded_signed_graphs_and_serialized_cursors_match_the_frozen_path() {
    let ids = [i16::MIN, -2, -1, 0, 1, 2, 3, 4, 5, 6, 17, i16::MAX, 99];
    let mut seed = 0x43f3794528u64;
    for case in 0..4096 {
        let (mut l, mut w) = fixture();
        for id in ids {
            if random(&mut seed) % 5 == 0 {
                w.objects.remove(&id);
            }
            if random(&mut seed) % 5 == 0 {
                l.next.remove(&id);
            } else {
                l.next.insert(id, ids[(random(&mut seed) % 13) as usize]);
            }
            object(&mut w, id, |o| {
                o.slots = (0..random(&mut seed) % 5)
                    .map(|_| {
                        if random(&mut seed) & 1 == 0 {
                            None
                        } else {
                            Some(ids[(random(&mut seed) % 13) as usize])
                        }
                    })
                    .collect();
                for key in [14, 26] {
                    if random(&mut seed) % 5 == 0 {
                        o.data.remove(&key);
                    } else {
                        let value = if key == 14 {
                            (random(&mut seed) as i16) % 6
                        } else {
                            ids[(random(&mut seed) % 13) as usize]
                        };
                        o.data.insert(key, value);
                    }
                }
            });
        }
        let before = Cursor {
            root: ids[(random(&mut seed) % 13) as usize],
            current: Some(ids[(random(&mut seed) % 13) as usize]),
        };
        let before: Cursor = serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
        let _ = check(&format!("seeded {case}"), &l, &w, before);
    }
    println!("seeded signed graph/serialized cursor cases=4096");
}

#[test]
fn sentinel_ancestry_and_partial_error_precedence_are_explicit() {
    let (mut l, mut w) = fixture();
    l.next.remove(&1);
    w.objects.remove(&1);
    let (r, c) = check("parent before missing sentinel", &l, &w, Cursor::new(1));
    assert_eq!(r, Err("physical iterator object absent".into()));
    assert_eq!(c.current, Some(1));
    let (mut l, w) = fixture();
    l.next.remove(&1);
    let (r, c) = check(
        "sentinel before valid positive child",
        &l,
        &w,
        Cursor::new(1),
    );
    assert_eq!(r, Err("native sentinel link absent".into()));
    assert_eq!(c.current, Some(1));
    let (l, mut w) = fixture();
    object(&mut w, 2, |o| {
        o.data.clear();
    });
    let (r, c) = check(
        "predecessor word before slot word",
        &l,
        &w,
        Cursor {
            root: 1,
            current: Some(2),
        },
    );
    assert_eq!(r, Err("native physical link word absent".into()));
    assert_eq!(c.current, None);
    word(&mut w, 2, 26, 99);
    let (r, c) = check(
        "missing predecessor before missing slot word",
        &l,
        &w,
        Cursor {
            root: 1,
            current: Some(2),
        },
    );
    assert_eq!(r, Err("physical iterator lost its predecessor".into()));
    assert_eq!(c.current, None);
    word(&mut w, 2, 26, 1);
    word(&mut w, 2, 14, -2);
    let (r, _) = check(
        "negative ancestry slot",
        &l,
        &w,
        Cursor {
            root: 1,
            current: Some(2),
        },
    );
    assert_eq!(r, Err("physical iterator has a negative slot".into()));
    word(&mut w, 2, 14, i16::MAX);
    word(&mut w, 1, 26, 2);
    word(&mut w, 1, 14, i16::MAX);
    let (r, c) = check(
        "cycle after exhausted ancestry",
        &l,
        &w,
        Cursor {
            root: 6,
            current: Some(2),
        },
    );
    assert_eq!(r, Err("cyclic physical iterator ancestry".into()));
    assert_eq!(c.current, None);
}

#[test]
fn between_call_changes_are_observed_after_cursor_save_restore() {
    for change in 0..5 {
        let (mut l, mut w) = fixture();
        let mut cursor = Cursor::new(1);
        let (r, next) = check("first external call", &l, &w, cursor);
        assert_eq!(r, Ok(()));
        cursor = next;
        assert_eq!(cursor.current, Some(2));
        cursor = serde_json::from_slice(&serde_json::to_vec(&cursor).unwrap()).unwrap();
        let expected = match change {
            0 => {
                object(&mut w, 1, |o| o.slots[0] = Some(i16::MAX));
                Some(i16::MAX)
            }
            1 => {
                word(&mut w, 2, 26, 6);
                object(&mut w, 6, |o| o.slots = vec![Some(17)]);
                Some(17)
            }
            2 => {
                l.next.insert(2, 5);
                Some(5)
            }
            3 => {
                w.objects.remove(&2);
                Some(2)
            }
            _ => {
                l.next.remove(&2);
                Some(2)
            }
        };
        let (r, next) = check(&format!("external mutation {change}"), &l, &w, cursor);
        assert_eq!(next.current, expected);
        if change < 3 {
            assert_eq!(r, Ok(()));
        } else {
            assert!(r.is_err());
        }
    }
}

#[test]
fn long_ancestry_and_inactive_backing_heads_remain_separate() {
    let (mut l, mut w) = fixture();
    for id in 1..=128 {
        w.objects.insert(
            id,
            Object {
                data: [(14, i16::MAX), (26, id - 1)].into(),
                ..Default::default()
            },
        );
        l.next.insert(id, 0);
    }
    let (r, c) = check(
        "128 parent climb",
        &l,
        &w,
        Cursor {
            root: 1,
            current: Some(128),
        },
    );
    assert_eq!(r, Ok(()));
    assert_eq!(c.current, None);
    word(&mut w, 65, 26, 128);
    let (r, _) = check(
        "long ancestry cycle",
        &l,
        &w,
        Cursor {
            root: 1,
            current: Some(128),
        },
    );
    assert_eq!(r, Err("cyclic physical iterator ancestry".into()));
    let (mut l, mut w) = fixture();
    w.map_size = Some(sims_vm::world_map::State { active_size: 4 });
    //The duplicate/out-of-domain reference stays in backing storage. It must
    //not become active merely because a traversal helper has been optimized.
    if let Some(head) = l.heads[0].get_mut(35) {
        *head = 1;
    }
    let before = l.clone();
    assert_eq!(l.collect_tile_chains(&w), reference_collect(&l, &w));
    assert_eq!(l, before);
    w.map_size = None;
    assert_eq!(
        l.collect_tile_chains(&w),
        Err("object occurs in multiple native tile chains".into())
    );
}

#[test]
fn borrowed_ancestry_words_preserve_predecessor_before_slot_failures() {
    for (predecessor, expected) in [
        (None, "native physical link word absent"),
        (Some(0), "physical iterator lost its predecessor"),
        (Some(-2), "physical iterator lost its predecessor"),
        (Some(99), "physical iterator lost its predecessor"),
        (Some(1), "native physical link word absent"),
    ] {
        let (links, mut world) = fixture();
        let child = world.objects.get_mut(&2).unwrap();
        child.data.remove(&14);
        child.data.remove(&26);
        if let Some(predecessor) = predecessor {
            child.data.insert(26, predecessor);
        }
        let (result, cursor) = check(
            &format!("predecessor {predecessor:?}, missing slot"),
            &links,
            &world,
            Cursor {
                root: 1,
                current: Some(2),
            },
        );
        assert_eq!(result, Err(expected.into()));
        assert_eq!(cursor.current, None);
    }
}

#[test]
fn borrowed_ancestry_words_preserve_resumed_slot_sentinel_order() {
    for (physical_slot, expected, next) in [
        (-2, Err("physical iterator has a negative slot".into()), None),
        (-1, Err("native sentinel link absent".into()), None),
        (0, Ok(()), Some(3)),
        (1, Ok(()), Some(5)),
        (i16::MAX, Ok(()), None),
    ] {
        let (mut links, mut world) = fixture();
        links.next.remove(&1);
        world
            .objects
            .get_mut(&2)
            .unwrap()
            .data
            .insert(14, physical_slot);
        let (result, cursor) = check(
            &format!("resumed physical slot {physical_slot}"),
            &links,
            &world,
            Cursor {
                root: 1,
                current: Some(2),
            },
        );
        assert_eq!(result, expected);
        assert_eq!(cursor.current, next);
    }
}

#[test]
fn borrowed_ancestry_failure_precedes_duplicate_and_registry_checks_without_publication() {
    let (mut links, mut world) = fixture();
    links.heads[0][15] = 1;
    world.objects.get_mut(&2).unwrap().data.remove(&26);
    for missing_registry in [false, true] {
        let mut candidate = world.clone();
        if missing_registry {
            candidate.person = None;
        } else {
            candidate.person.as_mut().unwrap().tile_chains = vec![vec![17]];
        }
        let expected = Err("native physical link word absent".into());
        assert_eq!(links.validate_tile_chains(&candidate), expected);
        assert_eq!(
            links.validate_tile_chains(&candidate),
            reference_validate(&links, &candidate)
        );
        let before = candidate.clone();
        assert_eq!(links.publish_tile_chains(&mut candidate), expected);
        assert_eq!(candidate, before);
    }
}
