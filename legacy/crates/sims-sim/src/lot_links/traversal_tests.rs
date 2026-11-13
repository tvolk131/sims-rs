//! Frozen BTreeSet traversal from df5dc87. Do not share the optimized walk:
//! compare its outputs, errors, and partially advanced cursor on malformed data.
use super::*;
use sims_vm::{Object, person::WorldState};

fn old_advance(cursor: &mut Cursor, links: &Links, world: &World) -> Result<(), String> {
    let Some(mut node) = cursor.current else {
        return Ok(());
    };
    let mut slot = 0;
    let mut seen = BTreeSet::new();
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

fn old_walk(links: &Links, world: &World, root: ObjectId) -> Result<Vec<ObjectId>, String> {
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    let mut cursor = Cursor::new(root);
    while let Some(id) = cursor.current {
        if !seen.insert(id) {
            return Err("cyclic or duplicate physical slot traversal".into());
        }
        result.push(id);
        old_advance(&mut cursor, links, world)?;
    }
    Ok(result)
}

fn old_collect(links: &Links, world: &World) -> Result<Vec<Vec<ObjectId>>, String> {
    links.validate_arrays()?;
    let mut chains = vec![];
    let mut seen = BTreeSet::new();
    for head in links.heads.iter().flatten() {
        if let Some(id) = Links::registered(world, *head) {
            let chain = old_walk(links, world, id)?;
            if chain.iter().any(|&id| !seen.insert(id)) {
                return Err("object occurs in multiple native tile chains".into());
            }
            chains.push(chain);
        }
    }
    Ok(chains)
}

fn old_validate(links: &Links, world: &World) -> Result<(), String> {
    let chains = old_collect(links, world)?;
    let expected = &world
        .person
        .as_ref()
        .ok_or("tile registry unavailable")?
        .tile_chains;
    if &chains != expected {
        return Err("native runtime physical tile order mismatch".into());
    }
    Ok(())
}

fn chain(count: i16) -> (Links, World) {
    let mut world = World {
        person: Some(WorldState::default()),
        ..Default::default()
    };
    let mut links = Links {
        size: 2,
        heads: vec![vec![0; 4]; 2],
        next: BTreeMap::new(),
    };
    for id in 1..=count {
        world.objects.insert(
            id,
            Object {
                data: [(14, 0), (26, id - 1)].into(),
                ..Default::default()
            },
        );
        links.next.insert(id, if id == count { 0 } else { id + 1 });
    }
    if count > 0 {
        links.heads[0][0] = 1;
    }
    links.publish_tile_chains(&mut world).unwrap();
    (links, world)
}

fn compare(links: &Links, world: &World) {
    let before = world.clone();
    let before_links = links.clone();
    assert_eq!(links.collect_tile_chains(world), old_collect(links, world));
    assert_eq!(
        links.validate_tile_chains(world),
        old_validate(links, world)
    );
    for root in world
        .objects
        .keys()
        .copied()
        .chain([0, -1, i16::MIN, i16::MAX])
    {
        assert_eq!(
            links.walk(world, root),
            old_walk(links, world, root),
            "root {root}"
        );
        // Test the cursor at arbitrary resumable positions as well as at root.
        for current in [Some(root), Some(1), Some(17), Some(i16::MIN), None] {
            let mut old = Cursor { root, current };
            let mut new = old.clone();
            assert_eq!(
                new.advance(links, world),
                old_advance(&mut old, links, world)
            );
            assert_eq!(new, old, "cursor root {root}, current {current:?}");
        }
    }
    assert_eq!(*world, before);
    assert_eq!(*links, before_links);
}

#[test]
fn retained_active_extent_preserves_inactive_heads_without_enumerating_them() {
    let (mut links, mut world) = chain(1);
    links.size = 8;
    links.heads = vec![vec![0; 64]; 2];
    links.heads[0][8 + 2] = 1;
    //The inactive duplicate would fail a full-stride enumeration. Native load
    //repair and gameplay tile scanning leave that backing cell unexamined.
    links.heads[0][7 * 8 + 7] = 1;
    world.map_size = Some(sims_vm::world_map::State { active_size: 4 });
    let before = links.clone();
    links.publish_tile_chains(&mut world).unwrap();
    assert_eq!(world.person.as_ref().unwrap().tile_chains, vec![vec![1]]);
    assert_eq!(links.validate_tile_chains(&world), Ok(()));
    assert_eq!(links, before);
    let restored: World = serde_json::from_slice(&serde_json::to_vec(&world).unwrap()).unwrap();
    assert_eq!(links.validate_tile_chains(&restored), Ok(()));
    world.map_size = None;
    assert_eq!(
        links.collect_tile_chains(&world),
        Err("object occurs in multiple native tile chains".into())
    );
}

#[test]
fn borrowed_traversal_preserves_valid_order_and_malformed_diagnostics() {
    type Mutation = fn(&mut Links, &mut World);
    let mutations: &[Mutation] = &[
        |_, _| {},
        |l, _| l.size = 1,
        |l, _| {
            l.heads[0].pop();
        },
        |_, w| w.person = None,
        |_, w| {
            if let Some(p) = &mut w.person {
                p.tile_chains.clear();
            }
        },
        |l, _| l.heads[1][0] = 1,
        |l, _| l.heads[0][1] = 2,
        |l, _| l.heads[0][0] = -1,
        |l, _| {
            l.next.remove(&2);
        },
        |l, _| {
            l.next.insert(32, 1);
        },
        |_, w| {
            w.objects.get_mut(&32).unwrap().data.insert(26, 32);
        },
        |_, w| {
            w.objects.get_mut(&32).unwrap().data.insert(26, 0);
        },
        |_, w| {
            w.objects.get_mut(&32).unwrap().data.insert(14, -2);
        },
        |_, w| {
            w.objects.get_mut(&32).unwrap().data.remove(&14);
        },
        |_, w| {
            w.objects.get_mut(&32).unwrap().data.remove(&26);
        },
        |_, w| {
            w.objects.get_mut(&1).unwrap().slots = vec![Some(2)];
        },
        |_, w| {
            w.objects.remove(&2);
        },
    ];
    for count in [0, 1, 16, 17, 32, 100] {
        let (l, w) = chain(count);
        compare(&l, &w);
    }
    let (links, world) = chain(32);
    for a in mutations {
        for b in mutations {
            let (mut l, mut w) = (links.clone(), world.clone());
            a(&mut l, &mut w);
            b(&mut l, &mut w);
            compare(&l, &w);
        }
    }
    // A logical child after a long slot-zero ancestry climb.
    let (mut l, mut w) = chain(32);
    l.next.insert(32, 0);
    w.objects.get_mut(&1).unwrap().slots = vec![Some(32)];
    l.next.insert(31, 0);
    w.objects
        .get_mut(&32)
        .unwrap()
        .data
        .extend([(26, 1), (14, 1)]);
    l.publish_tile_chains(&mut w).unwrap();
    assert_eq!(l.walk(&w, 1).unwrap(), (1..=32).collect::<Vec<_>>());
    compare(&l, &w);
}

#[test]
fn borrowed_traversal_matches_random_malformed_graphs() {
    let mut seed = 0x72b4c9e2_u32;
    let mut random = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        seed
    };
    for _ in 0..500 {
        let (mut l, mut w) = chain(24);
        for head in l.heads.iter_mut().flatten() {
            *head = (random() % 28) as i16 - 2;
        }
        for id in 1..=24 {
            let o = w.objects.get_mut(&id).unwrap();
            o.slots = (0..random() % 3)
                .map(|_| Some((random() % 28) as i16 - 2))
                .collect();
            o.data.insert(14, (random() % 6) as i16 - 3);
            o.data.insert(26, (random() % 28) as i16 - 2);
            l.next.insert(id, (random() % 28) as i16 - 2);
            if random() % 11 == 0 {
                l.next.remove(&id);
            }
        }
        compare(&l, &w);
    }
}

#[test]
fn local_seen_scratch_covers_signed_domain_and_reuses_storage() {
    let mut seen = Seen::default();
    let mut reference = BTreeSet::new();
    for _ in 0..2 {
        for id in i16::MIN..=i16::MAX {
            assert_eq!(seen.insert(id), reference.insert(id));
            assert_eq!(seen.insert(id), reference.insert(id));
        }
        seen.clear();
        reference.clear();
    }
}
