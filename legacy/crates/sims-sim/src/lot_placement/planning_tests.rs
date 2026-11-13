//! Differential reference frozen before borrowed placement planning.
use super::*;
use crate::{
    Point,
    lot_loading::{Effect, PlacementUpdate, SpatialUpdate},
    spatial,
};
use std::hint::black_box;

fn legacy_restore_position(
    world: &mut World,
    architecture: &Architecture,
    root: ObjectId,
    position: Placement,
) -> Result<PlacementUpdate, String> {
    if architecture.original.is_none() || !(1..=2).contains(&position.level) {
        return Err("loaded placement needs original architecture and a supported floor".into());
    }
    let room = architecture.room_at(position.level, Point::new(position.y, position.x));
    let mut pending = vec![root];
    let mut seen = BTreeSet::new();
    let mut update = PlacementUpdate::default();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err("cyclic or duplicate loaded containment".into());
        }
        let object = world
            .objects
            .get(&id)
            .ok_or("loaded placement root is absent")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("loaded placement definition absent")?;
        let shape = architecture
            .footprints
            .get(&definition.guid)
            .ok_or("loaded placement footprint absent")?;
        let facing = *object.data.get(&1).ok_or("loaded direction absent")?;
        if !(0..=7).contains(&facing) {
            return Err("loaded direction outside native footprint contract".into());
        }
        let person_field = |index| -> Result<u16, String> {
            if shape.kind != 2 {
                return Ok(0);
            }
            Ok(*object
                .person
                .as_ref()
                .and_then(|p| p.get(&index))
                .ok_or("loaded person footprint field absent")? as u16)
        };
        let bounds = spatial::footprint(
            [position.x, position.y],
            shape.kind == 2,
            (person_field(68)? | person_field(84)?) & 1 != 0,
            shape.size,
            shape.insets,
            person_field(73)?,
            facing as u8,
        );
        update.effects.push(Effect::Invalidate(id));
        let old_room = *object.data.get(&29).ok_or("loaded room field absent")?;
        if old_room != room && *object.data.get(&7).ok_or("loaded lighting field absent")? != 0 {
            update
                .effects
                .extend([Effect::Room(old_room), Effect::Room(room)]);
        }
        update.objects.push(SpatialUpdate {
            object: id,
            position: [position.x, position.y, position.level],
            coarse: sims_vm::loading::coarse_position(position),
            bounds,
            room,
        });
        // Native registry lookup skips absent child IDs. A root must exist.
        pending.extend(
            object
                .slots
                .iter()
                .rev()
                .flatten()
                .copied()
                .filter(|id| world.objects.contains_key(id)),
        );
    }
    // Validate before mutation; a malformed import cannot leave half a subtree
    // in a new room. Successful updates retain the original visitation order.
    for o in &update.objects {
        world.positions.insert(o.object, position);
        world
            .objects
            .get_mut(&o.object)
            .unwrap()
            .data
            .insert(29, o.room);
    }
    Ok(update)
}

fn fixture() -> (Program, World, Architecture, State) {
    let (p, s) = crate::lot_runtime::tests::fixture();
    let mut w = s.vm.world;
    w.objects.get_mut(&1).unwrap().slots = vec![Some(2), None, Some(3), Some(99)];
    w.objects.get_mut(&2).unwrap().slots = vec![Some(4)];
    for (&id, o) in &mut w.objects {
        o.data.extend([(7, 1), (29, 10 + id)]);
    }
    (
        p,
        w,
        s.architecture.unwrap(),
        s.native_lot.unwrap().placement,
    )
}

#[test]
fn borrowed_plan_preserves_updates_errors_and_atomic_restore() {
    type Mutation = fn(&mut World, &mut Architecture);
    let mutations: &[Mutation] = &[
        |_, _| {},
        |_, a| a.original = None,
        |w, _| {
            w.objects.remove(&1);
        },
        |w, _| {
            w.objects.get_mut(&2).unwrap().definition = None;
        },
        |_, a| a.footprints.clear(),
        |w, _| {
            w.objects.get_mut(&2).unwrap().data.remove(&1);
        },
        |w, _| {
            w.objects.get_mut(&2).unwrap().data.insert(1, 8);
        },
        |w, _| {
            w.objects.get_mut(&2).unwrap().data.insert(1, -1);
        },
        |w, _| {
            w.objects.get_mut(&2).unwrap().data.remove(&29);
        },
        |w, _| {
            w.objects.get_mut(&2).unwrap().data.remove(&7);
        },
        |w, _| {
            w.objects.get_mut(&4).unwrap().slots = vec![Some(1)];
        },
        |w, _| {
            if let Some(o) = w.objects.get_mut(&1) {
                o.slots.push(Some(2));
            }
        },
        |w, _| {
            w.positions.clear();
        },
        |w, _| {
            w.objects.remove(&3);
        },
        |w, a| {
            if let Some(shape) = a.footprints.get_mut(&1) {
                shape.kind = 2;
            }
            for o in w.objects.values_mut() {
                o.person = Some([(68, 1), (84, 0), (73, 3)].into());
            }
        },
        |w, _| {
            w.objects.get_mut(&4).unwrap().person = None;
        },
    ];
    let (_, initial, architecture, _) = fixture();
    for (first, f) in mutations.iter().enumerate() {
        for (second, g) in mutations.iter().enumerate() {
            for destination in [[56, 40, 1], [-16, -32, 2], [40, 40, 0], [40, 40, 3]] {
                let mut w = initial.clone();
                let mut a = architecture.clone();
                f(&mut w, &mut a);
                g(&mut w, &mut a);
                let to = Placement {
                    x: destination[0],
                    y: destination[1],
                    level: destination[2],
                };
                let before = w.clone();
                let mut legacy = w.clone();
                let expected = legacy_restore_position(&mut legacy, &a, 1, to);
                assert_eq!(
                    lot_loading::plan_position(&w, &a, 1, to),
                    expected,
                    "plan {first}/{second}/{destination:?}"
                );
                assert_eq!(w, before, "planning mutated world");
                assert_eq!(
                    lot_loading::restore_position(&mut w, &a, 1, to),
                    expected,
                    "restore {first}/{second}/{destination:?}"
                );
                assert_eq!(w, legacy, "restore state {first}/{second}/{destination:?}");
                if expected.is_err() {
                    assert_eq!(w, before);
                }
            }
        }
    }
}

fn legacy_move(
    op: &mut Operation,
    w: &World,
    a: &Architecture,
    to: Placement,
) -> Result<(), String> {
    let update = legacy_restore_position(&mut w.clone(), a, op.object, to)?;
    let mut positions = update.objects.into_iter();
    for effect in update.effects {
        match effect {
            Effect::Invalidate(object) => {
                op.pending.push_back(Pending::Request(Request::Invalidate {
                    object,
                    children_only: object != op.object,
                }));
                let position = positions.next().ok_or("spatial update order incomplete")?;
                if position.object != object {
                    return Err("spatial update order mismatch".into());
                }
                op.pending.push_back(Pending::Position(position));
            }
            Effect::Room(value) => op
                .pending
                .push_back(Pending::Request(Request::Signal { event: 0xf3, value })),
        }
    }
    Ok(())
}

#[test]
fn borrowed_plan_preserves_every_observer_and_saved_pending_write() {
    for mutation in 0..3 {
        let (p, mut old_world, a, mut old_state) = fixture();
        let mut new_world = old_world.clone();
        let mut new_state = old_state.clone();
        let mut old = Operation::remove(1, true);
        // Isolate the actual pending-write consumer from later removal phases.
        old.phase = 20;
        let mut new = old.clone();
        let to = Placement {
            x: 56,
            y: 40,
            level: 2,
        };
        legacy_move(&mut old, &old_world, &a, to).unwrap();
        new.move_position(&new_world, &a, to).unwrap();
        assert_eq!(new, old);
        let mut boundary = 0;
        loop {
            let expected = old.next(&mut old_world, &p, &a, &mut old_state);
            let actual = new.next(&mut new_world, &p, &a, &mut new_state);
            assert_eq!(actual, expected);
            assert_eq!(new_world, old_world);
            assert_eq!(new_state, old_state);
            assert_eq!(new, old);
            if boundary == 0 {
                assert!(matches!(
                    actual,
                    Ok(Some(Request::Invalidate { object: 1, .. }))
                ));
                assert_eq!(
                    new_world.positions[&1].x, 40,
                    "invalidation precedes publication"
                );
                for w in [&mut old_world, &mut new_world] {
                    if mutation == 1 {
                        w.objects
                            .get_mut(&3)
                            .unwrap()
                            .data
                            .extend([(1, 2), (29, 55)]);
                    } else if mutation == 2 {
                        w.objects.remove(&2);
                    }
                }
            }
            // Restore both queued writes and all world state at every boundary.
            new = serde_json::from_slice(&serde_json::to_vec(&new).unwrap()).unwrap();
            new_world = serde_json::from_slice(&serde_json::to_vec(&new_world).unwrap()).unwrap();
            new_state = serde_json::from_slice(&serde_json::to_vec(&new_state).unwrap()).unwrap();
            if !matches!(actual, Ok(Some(_))) {
                break;
            }
            boundary += 1;
            assert!(boundary < 30);
        }
        assert_eq!(old_world.positions[&1], to);
    }
}

#[test]
#[ignore = "uses retained private Mashuga snapshots and prints release planning timings"]
fn borrowed_plan_matches_real_objects_and_measures_cost() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in [
        "cache/original-live-character-final.json",
        "cache/neighborhood-input-experiments/mashuga-empty-frame-floor-v72/original-live-character-final.json",
    ] {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join(relative)).unwrap()).unwrap();
        let snapshot: crate::Snapshot =
            serde_json::from_value(value["simulation"].clone()).unwrap();
        let s = snapshot.state;
        let w = &s.vm.world;
        let a = s.architecture.as_ref().unwrap();
        let before = w.clone();
        for (&id, &to) in &w.positions {
            assert_eq!(
                lot_loading::plan_position(w, a, id, to),
                legacy_restore_position(&mut w.clone(), a, id, to),
                "{relative} #{id}"
            );
        }
        let id = 196;
        let to = w.positions[&id];
        let measure = |legacy| {
            let start = std::time::Instant::now();
            for _ in 0..200 {
                let result = if legacy {
                    legacy_restore_position(&mut black_box(w).clone(), black_box(a), id, to)
                } else {
                    lot_loading::plan_position(black_box(w), black_box(a), id, to)
                };
                black_box(result).unwrap();
            }
            start.elapsed().as_nanos() as f64 / 200.0 / 1000.0
        };
        let (mut old, mut new) = (vec![], vec![]);
        for round in 0..5 {
            if round % 2 == 0 {
                old.push(measure(true));
                new.push(measure(false));
            } else {
                new.push(measure(false));
                old.push(measure(true));
            }
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        assert_eq!(*w, before);
        println!(
            "{relative}: {} objects compared, median planning {:.2} -> {:.2} us ({:.1}x); five alternating 200-call batches, unchanged world",
            w.objects.len(),
            old[2],
            new[2],
            old[2] / new[2]
        );
    }
}
