//! Independent clone-and-apply reference retained before the bounded overlay.
//! apply_inner/attach_raw/at are the unchanged old implementation, test-only;
//! none of them calls the new transaction planner.
use super::*;
use sims_vm::{Object, person::WorldState};

fn reference(l: &mut Links, w: &mut World, id: ObjectId, d: Descriptor) -> Result<(), String> {
    l.validate_arrays()?;
    let mut next = l.clone();
    let mut updated = w.clone();
    next.apply_inner(&mut updated, id, d)?;
    *l = next;
    *w = updated;
    Ok(())
}
fn fixture() -> (Links, World) {
    let mut w = World {
        globals: [(-1, 23), (0, 19), (34, 111)].into(),
        neighborhood: [(3, 77)].into(),
        room_half_tiles: [(1, 18)].into(),
        person: Some(WorldState {
            object_order: vec![6, 5, 4, 3, 2, 1],
            tile_chains: vec![vec![1, 2, 3, 4], vec![6]],
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut l = Links {
        size: 6,
        heads: vec![vec![0; 36]; 2],
        next: BTreeMap::new(),
    };
    for id in [ObjectId::MIN, -2, 0, 1, 2, 3, 4, 5, 6, ObjectId::MAX] {
        let (slot, parent) = match id {
            2 => (0, 1),
            3 => (1, 1),
            4 => (1, 3),
            _ => (-1, 0),
        };
        w.objects.insert(
            id,
            Object {
                data: [(2, 123), (3, -456), (14, slot), (26, parent), (55, 317)].into(),
                attributes: vec![111, -222],
                slots: match id {
                    1 => vec![Some(3), None],
                    3 => vec![Some(4), None],
                    _ => vec![None, None],
                },
                ..Default::default()
            },
        );
        let p = match id {
            1..=4 => Placement {
                x: 40,
                y: 40,
                level: 1,
            },
            6 => Placement {
                x: 56,
                y: 56,
                level: 2,
            },
            _ => Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        };
        w.positions.insert(id, p);
        l.next.insert(id, if id == 1 { 2 } else { 0 });
    }
    l.heads[0][14] = 1;
    l.heads[1][21] = 6;
    (l, w)
}
fn descriptor(
    on_lot: bool,
    container: Option<ObjectId>,
    slot: i32,
    level: i32,
    position: [i32; 2],
) -> Descriptor {
    Descriptor {
        on_lot,
        container,
        slot,
        level,
        position,
    }
}
fn operations() -> Vec<(ObjectId, Descriptor)> {
    let mut result = vec![];
    for id in [ObjectId::MIN, -2, 0, 1, 2, 3, 4, 5, 6, ObjectId::MAX, 99] {
        result.push((id, descriptor(false, None, -1, 1, [-16, -16])));
        result.push((id, descriptor(true, None, -1, 1, [40, 40])));
    }
    for (parent, slot) in [
        (1, 0),
        (1, 1),
        (1, -1),
        (1, -2),
        (1, i32::MIN),
        (1, i32::MAX),
        (2, 0),
        (5, 0),
        (99, 0),
        (99, -1),
        (0, 0),
        (-2, 0),
    ] {
        result.push((5, descriptor(true, Some(parent), slot, 2, [40, 40])));
    }
    for level in [i32::MIN, -1, 0, 1, 2, 3, i32::MAX] {
        result.push((5, descriptor(true, None, -1, level, [40, 40])));
    }
    for point in [
        [0, 0],
        [16, 16],
        [80, 80],
        [96, 96],
        [-1, 40],
        [4096, 4096],
        [i32::MAX, i32::MIN],
    ] {
        result.push((5, descriptor(true, None, -1, 1, point)));
    }
    result.push((5, descriptor(true, None, i32::MAX, 1, [40, 40])));
    result
}
fn compare(l: &Links, w: &World, id: ObjectId, d: Descriptor, label: &str) -> usize {
    let (mut old_l, mut old_w) = (l.clone(), w.clone());
    let (mut new_l, mut new_w) = (l.clone(), w.clone());
    let before = serde_json::to_vec(&(l, w)).unwrap();
    let a = reference(&mut old_l, &mut old_w, id, d);
    let b = new_l.apply(&mut new_w, id, d);
    let preview = if l.validate_arrays().is_ok() {
        Some(transaction::preview(l, w, id, d))
    } else {
        None
    };
    if a != b || new_l != old_l || new_w != old_w {
        if let Ok(path) = std::env::var("SIMS_LINK_TRANSACTION_FIRST_FAILURE") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                let value = serde_json::json!({"label":label,"id":id,"descriptor":d,
                    "before":{"links":l,"world":w},"old":{"result":a,"links":old_l,"world":old_w},
                    "new":{"result":b,"links":new_l,"world":new_w}});
                file.write_all(&serde_json::to_vec_pretty(&value).unwrap())
                    .unwrap();
            }
        }
    }
    assert_eq!(a, b, "Result: {label}, id={id}, d={d:?}");
    if let Some((result, _)) = &preview {
        assert_eq!(result, &b, "preview: {label}");
    }
    assert_eq!(new_l, old_l, "links: {label}");
    assert_eq!(new_w, old_w, "World: {label}");
    assert_eq!(
        serde_json::to_vec(&(&new_l, &new_w)).unwrap(),
        serde_json::to_vec(&(&old_l, &old_w)).unwrap(),
        "bytes: {label}"
    );
    if b.is_err() {
        assert_eq!(
            serde_json::to_vec(&(&new_l, &new_w)).unwrap(),
            before,
            "error mutated: {label}"
        );
    }
    let restored: (Links, World) =
        serde_json::from_slice(&serde_json::to_vec(&(&new_l, &new_w)).unwrap()).unwrap();
    assert_eq!(restored, (new_l, new_w), "restore: {label}");
    preview.map_or(0, |(_, n)| n)
}

#[test]
fn compact_link_transaction_preserves_aliases_error_order_and_atomicity() {
    type Mutation = (&'static str, fn(&mut Links, &mut World));
    let mutations: &[Mutation] = &[
        ("unchanged", |_, _| {}),
        ("invalid size", |l, _| l.size = 1),
        ("invalid floor length", |l, _| {
            l.heads[0].pop();
        }),
        ("extra floor", |l, _| l.heads.push(vec![0; 36])),
        ("target position absent", |_, w| {
            w.positions.remove(&5);
        }),
        ("head position absent", |_, w| {
            w.positions.remove(&1);
        }),
        ("successor position absent", |_, w| {
            w.positions.remove(&2);
        }),
        ("target object absent", |_, w| {
            w.objects.remove(&5);
        }),
        ("head object absent", |_, w| {
            w.objects.remove(&1);
        }),
        ("head next absent", |l, _| {
            l.next.remove(&1);
        }),
        ("target next absent", |l, _| {
            l.next.remove(&5);
        }),
        ("head missing slot word", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.remove(&14);
            }
        }),
        ("head missing parent word", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.remove(&26);
            }
        }),
        ("target words absent", |_, w| {
            if let Some(o) = w.objects.get_mut(&5) {
                o.data.remove(&14);
                o.data.remove(&26);
            }
        }),
        ("parent slots absent", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.slots.clear();
            }
        }),
        ("self zero parent", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.extend([(14, 0), (26, 1)]);
            }
        }),
        ("self logical parent", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.extend([(14, 1), (26, 1)]);
            }
        }),
        ("head self next", |l, _| {
            l.next.insert(1, 1);
        }),
        ("next cycle", |l, _| {
            l.next.insert(2, 1);
        }),
        ("invalid old slot", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.extend([(14, -2), (26, 3)]);
            }
        }),
        ("unknown old parent", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.insert(26, 99);
            }
        }),
        ("negative head", |l, _| l.heads[0][14] = -2),
        ("unknown head", |l, _| l.heads[0][14] = 99),
        ("head off lot", |_, w| {
            if let Some(p) = w.positions.get_mut(&1) {
                p.x = -16;
                p.y = -16;
            }
        }),
        ("head invalid level", |_, w| {
            if let Some(p) = w.positions.get_mut(&1) {
                p.level = 3;
            }
        }),
        ("head physical slot maximum", |_, w| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.data.extend([(14, i16::MAX), (26, 3)]);
            }
        }),
    ];
    let (l, w) = fixture();
    let ops = operations();
    let mut cases = 0;
    let mut max_edits = 0;
    for (i, &(a_name, a)) in mutations.iter().enumerate() {
        for (j, &(b_name, b)) in mutations.iter().enumerate() {
            let (mut x, mut y) = (l.clone(), w.clone());
            a(&mut x, &mut y);
            b(&mut x, &mut y);
            // Twelve deterministic distinct branches per ordered mutation pair;
            // the additional pristine loop below covers every descriptor.
            for k in 0..12 {
                let (id, d) = ops[(i * 17 + j * 7 + k * 11) % ops.len()];
                max_edits =
                    max_edits.max(compare(&x, &y, id, d, &format!("{a_name}/{b_name}/{k}")));
                cases += 1;
            }
        }
    }
    for (id, d) in ops {
        max_edits = max_edits.max(compare(&l, &w, id, d, "pristine operation"));
        cases += 1;
    }
    assert!(max_edits >= 7 && max_edits <= 13);
    eprintln!("{cases} ordered malformed/alias cases; maximum {max_edits} staged edits");
}

#[test]
fn compact_link_transaction_rolls_back_errors_after_earlier_writes() {
    let (l, w) = fixture();
    let mut cases = vec![];
    let mut a = l.clone();
    a.next.remove(&1);
    cases.push((
        "unlink then missing sentinel",
        a,
        w.clone(),
        1,
        descriptor(false, None, -1, 1, [-16, -16]),
        "native sentinel link absent",
    ));
    let mut b = w.clone();
    b.positions.remove(&5);
    cases.push((
        "head and words then missing position",
        l.clone(),
        b,
        5,
        descriptor(true, None, -1, 1, [40, 40]),
        "link position absent",
    ));
    let mut b = w.clone();
    b.objects.remove(&5);
    cases.push((
        "unlink old head then missing target",
        l.clone(),
        b,
        5,
        descriptor(true, None, -1, 1, [40, 40]),
        "link object absent",
    ));
    let mut a = l.clone();
    a.next.remove(&5);
    cases.push((
        "new head then successor attachment failure",
        a,
        w.clone(),
        5,
        descriptor(true, None, -1, 1, [40, 40]),
        "native sentinel link absent",
    ));
    for (name, l, w, id, d, error) in cases {
        let (result, len) = transaction::preview(&l, &w, id, d);
        assert_eq!(result, Err(error.into()), "{name}");
        assert!(len >= 3, "must fail after earlier staged writes: {name}");
        compare(&l, &w, id, d, name);
    }
    // Existing==id intentionally skips invalid target words/position entirely.
    let (mut l, mut w) = fixture();
    w.positions.remove(&1);
    w.objects.get_mut(&1).unwrap().data.clear();
    assert_eq!(
        l.apply(&mut w, 1, descriptor(true, None, -1, 1, [40, 40])),
        Ok(())
    );
    // Unlinking a self-parent writes its own sentinel before the successor read.
    let (mut l, mut w) = fixture();
    w.objects
        .get_mut(&1)
        .unwrap()
        .data
        .extend([(14, 0), (26, 1)]);
    l.next.insert(1, 2);
    compare(
        &l,
        &w,
        1,
        descriptor(false, None, -1, 1, [-16, -16]),
        "live self-parent sentinel read",
    );
    l.apply(&mut w, 1, descriptor(false, None, -1, 1, [-16, -16]))
        .unwrap();
    assert_eq!(l.next[&1], 0);
    assert_eq!(
        w.objects[&2].data[&26], 1,
        "successor must remain untouched after aliased clear"
    );
}

#[test]
fn compact_link_transaction_matches_seeded_corrupt_states_and_sequences() {
    let mut seed = 0x4329c124_u32;
    let mut random = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed
    };
    let ids = [i16::MIN, -2, 0, 1, 2, 3, 4, 5, 6, i16::MAX, 99];
    let points = [i32::MIN, -32, -16, 0, 16, 40, 56, 80, 96, 4096, i32::MAX];
    let levels = [i32::MIN, -2, -1, 0, 1, 2, 3, i32::MAX];
    let slots = [i32::MIN, -32768, -3, -2, -1, 0, 1, 2, 3, 32767, i32::MAX];
    let mut max_edits = 0;
    for n in 0..3000 {
        let (mut l, mut w) = fixture();
        for id in [-2, 0, 1, 2, 3, 4, 5, 6, i16::MAX] {
            let o = w.objects.get_mut(&id).unwrap();
            for field in [14, 26] {
                if random() % 9 == 0 {
                    o.data.remove(&field);
                } else {
                    o.data.insert(
                        field,
                        if field == 14 {
                            slots[random() as usize % slots.len()] as i16
                        } else {
                            ids[random() as usize % ids.len()]
                        },
                    );
                }
            }
            o.slots = (0..random() % 4)
                .map(|_| (random() % 4 != 0).then(|| ids[random() as usize % ids.len()]))
                .collect();
            l.next.insert(id, ids[random() as usize % ids.len()]);
            if random() % 11 == 0 {
                l.next.remove(&id);
            }
            if random() % 11 == 0 {
                w.positions.remove(&id);
            } else {
                w.positions.insert(
                    id,
                    Placement {
                        x: points[random() as usize % points.len()],
                        y: points[random() as usize % points.len()],
                        level: levels[random() as usize % levels.len()],
                    },
                );
            }
        }
        for head in l.heads.iter_mut().flatten() {
            *head = ids[random() as usize % ids.len()];
        }
        let id = ids[random() as usize % ids.len()];
        let d = descriptor(
            random() % 2 == 0,
            (random() % 3 != 0).then(|| ids[random() as usize % ids.len()]),
            slots[random() as usize % slots.len()],
            levels[random() as usize % levels.len()],
            [
                points[random() as usize % points.len()],
                points[random() as usize % points.len()],
            ],
        );
        max_edits = max_edits.max(compare(&l, &w, id, d, &format!("seed case {n}")));
    }
    let (mut old_l, mut old_w) = fixture();
    let (mut new_l, mut new_w) = (old_l.clone(), old_w.clone());
    let ops = operations();
    for n in 0..500 {
        let (id, d) = ops[random() as usize % ops.len()];
        assert_eq!(
            new_l.apply(&mut new_w, id, d),
            reference(&mut old_l, &mut old_w, id, d),
            "sequence {n}"
        );
        assert_eq!((&new_l, &new_w), (&old_l, &old_w), "sequence state {n}");
        let bytes = serde_json::to_vec(&(&new_l, &new_w)).unwrap();
        let restored: (Links, World) = serde_json::from_slice(&bytes).unwrap();
        new_l = restored.0;
        new_w = restored.1;
        assert_eq!((&new_l, &new_w), (&old_l, &old_w));
    }
    assert!(max_edits <= 13);
    eprintln!(
        "3000 seeded corrupt states and 500 saved/restored sequential operations matched; max {max_edits} edits"
    );
}
