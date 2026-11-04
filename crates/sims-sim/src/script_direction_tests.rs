//! Independent frozen map-index reference for script direction synchronization.
//! Synthetic malformed worlds deliberately bypass unrelated geometry validation.
use super::*;
use sims_vm::{Object, World};
use std::{
    any::Any,
    panic::{AssertUnwindSafe, catch_unwind},
};

fn reference(s: &mut Simulation) -> Result<(), String> {
    let mut changes = vec![];
    for (&id, body) in &s.bodies {
        let object = &s.vm.world.objects[&id];
        let direction = *object.data.get(&1).ok_or("missing object direction")?;
        // BHAVs temporarily add to the raw direction word before wrapping.
        // Preserve that word in the VM; the eight-way spatial cache projects
        // its compass bits even at an instruction-budget boundary.
        let compass = (direction as u16 & 7) as u8;
        if compass == body.facing.trailing_zeros() as u8 {
            continue;
        }
        if object.multitile.is_some()
            || s.routes.values().any(|r| r.caller == id)
            || s.turns.contains_key(&id)
        {
            return Err(
                "script direction write during group placement or active movement is unsupported"
                    .into(),
            );
        }
        changes.push((id, 1u8 << compass));
    }
    for (id, facing) in changes {
        s.bodies.get_mut(&id).unwrap().facing = facing;
    }
    Ok(())
}

const IDS: [ObjectId; 8] = [ObjectId::MIN, -300, -2, 0, 2, 9, 300, ObjectId::MAX];
const MOVEMENT: &str =
    "script direction write during group placement or active movement is unsupported";

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Returned(Result<(), String>),
    Panicked(String),
}
fn payload(p: Box<dyn Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = p.downcast_ref::<&str>() {
        (*s).into()
    } else {
        panic!("unexpected non-string panic payload")
    }
}
fn invoke(s: &mut Simulation, old: bool) -> Outcome {
    match catch_unwind(AssertUnwindSafe(|| {
        if old {
            reference(s)
        } else {
            s.sync_script_directions()
        }
    })) {
        Ok(result) => Outcome::Returned(result),
        Err(p) => Outcome::Panicked(payload(p)),
    }
}
fn body() -> Body {
    Body {
        level: 1,
        position: Point::new(64, 64),
        facing: 1,
        radius: 0,
        elevation: 0,
    }
}
fn empty() -> Simulation {
    Simulation::new(
        Vm::new(World::default(), 123),
        Room {
            width: 128,
            height: 128,
            obstacles: vec![],
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn fixture() -> Simulation {
    let mut s = empty();
    for id in IDS {
        s.bodies.insert(id, body());
        s.vm.world.objects.insert(
            id,
            Object {
                data: BTreeMap::from([(1, 0), (9, -123)]),
                ..Default::default()
            },
        );
    }
    // Objects without bodies are never inspected, even with no direction word.
    for id in [ObjectId::MIN + 1, -301, -1, 1, 10, 301, ObjectId::MAX - 1] {
        s.vm.world.objects.insert(id, Object::default());
    }
    s
}
fn add_route(s: &mut Simulation, id: ObjectId, key: ThreadId) {
    s.routes.insert(
        key,
        Route {
            request: key,
            caller: id,
            target: 9,
            target_body: body(),
            goal: Destination {
                position: Point::new(64, 64),
                facing: 1,
                entry_flags: 0,
            },
            path: vec![],
            original_path: None,
            next: 0,
            started_tick: 0,
            last_tick: 0,
            locomotion: locomotion::Locomotion {
                initial_facing: 1,
                distances: vec![],
                phases: vec![],
                elapsed: 0,
            },
        },
    );
}
fn add_turn(s: &mut Simulation, id: ObjectId) {
    s.turns.insert(
        id,
        turn::Turn {
            request: 1,
            thread: 7,
            path: [[0; 2]; 3],
            started_tick: 0,
            last_tick: None,
            phase: turn::Phase::Starting,
            from: 0.,
            target: 0.,
            base: 0.,
            heading: 0.,
            clip: None,
            progress: 0.,
            next_event: 0,
        },
    );
}
fn add_group(s: &mut Simulation, id: ObjectId) {
    s.vm.world.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
        head: Some(9),
        next: Some(id),
        origin: [0; 3],
        offset: [0; 3],
    });
}
fn check(s: &Simulation, label: &str) -> Outcome {
    let before = serde_json::to_vec(s).unwrap();
    let mut old = s.clone();
    let mut new = s.clone();
    let expected = invoke(&mut old, true);
    let actual = invoke(&mut new, false);
    let old_bytes = serde_json::to_vec(&old).unwrap();
    let new_bytes = serde_json::to_vec(&new).unwrap();
    if expected != actual || old_bytes != new_bytes {
        let evidence = serde_json::json!({"label":label,"expected":format!("{expected:?}"),"actual":format!("{actual:?}"),"before":s,"old":old,"new":new});
        if let Some(path) = std::env::var_os("SCRIPT_DIRECTION_FIRST_FAILURE") {
            let path = std::path::PathBuf::from(path);
            if !path.exists() {
                std::fs::write(path, serde_json::to_vec(&evidence).unwrap()).unwrap();
            }
        }
        panic!("script direction differential: {label}: {expected:?} != {actual:?}");
    }
    if actual != Outcome::Returned(Ok(())) {
        assert_eq!(new_bytes, before, "failure modified staged state: {label}");
    }
    assert_eq!(
        serde_json::to_vec(&new.vm).unwrap(),
        serde_json::to_vec(&s.vm).unwrap(),
        "raw VM mutation: {label}"
    );
    actual
}
fn mutate(s: &mut Simulation, id: ObjectId, kind: usize) {
    match kind {
        0 => {}
        1 => {
            s.vm.world.objects.remove(&id);
        }
        2 => {
            if let Some(o) = s.vm.world.objects.get_mut(&id) {
                o.data.remove(&1);
            }
        }
        3 => {
            if let Some(o) = s.vm.world.objects.get_mut(&id) {
                o.data.insert(1, 3);
            }
        }
        4 => {
            if s.vm.world.objects.contains_key(&id) {
                add_group(s, id);
            }
        }
        5 => add_route(s, id, (id as u16 as u64) + 100),
        6 => add_turn(s, id),
        7 => {
            if let Some(b) = s.bodies.get_mut(&id) {
                b.facing = 0;
            }
        }
        8 => {
            s.bodies.remove(&id);
        }
        _ => unreachable!(),
    }
}
#[test]
fn directions_signed_raw_word_and_every_body_facing_match() {
    let raw = [i16::MIN, -32761, -9, -8, -1, 0, 1, 7, 8, 15, 256, i16::MAX];
    let mut cases = 0;
    for id in IDS {
        for facing in 0..=u8::MAX {
            for direction in raw {
                let mut s = fixture();
                s.bodies.get_mut(&id).unwrap().facing = facing;
                s.vm.world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .data
                    .insert(1, direction);
                assert_eq!(
                    check(
                        &s,
                        &format!("id={id} facing={facing} direction={direction}")
                    ),
                    Outcome::Returned(Ok(()))
                );
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 24576);
    assert_eq!(check(&empty(), "empty maps"), Outcome::Returned(Ok(())));
}
#[test]
fn directions_ordered_malformed_pairs_preserve_error_priority_and_atomicity() {
    let mut cases = 0;
    for first in IDS {
        for a in 0..9 {
            for second in IDS {
                for b in 0..9 {
                    let mut s = fixture();
                    mutate(&mut s, first, a);
                    mutate(&mut s, second, b);
                    let _ = check(&s, &format!("({first},{a}) then ({second},{b})"));
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 5184);
}
#[test]
fn directions_known_first_failures_and_unchanged_movement_are_explicit() {
    let mut s = fixture();
    s.vm.world
        .objects
        .get_mut(&ObjectId::MIN)
        .unwrap()
        .data
        .insert(1, 3);
    s.vm.world.objects.get_mut(&9).unwrap().data.remove(&1);
    assert_eq!(
        check(&s, "later missing direction discards earlier staged facing"),
        Outcome::Returned(Err("missing object direction".into()))
    );
    add_group(&mut s, ObjectId::MIN);
    assert_eq!(
        check(&s, "earlier changed group beats later direction"),
        Outcome::Returned(Err(MOVEMENT.into()))
    );
    s.vm.world
        .objects
        .get_mut(&ObjectId::MIN)
        .unwrap()
        .data
        .remove(&1);
    assert_eq!(
        check(&s, "direction checked before same-body movement"),
        Outcome::Returned(Err("missing object direction".into()))
    );
    s.vm.world.objects.remove(&ObjectId::MIN);
    assert_eq!(
        check(&s, "earliest missing object retains index panic"),
        Outcome::Panicked("no entry found for key".into())
    );
    for kind in [4, 5, 6] {
        let mut s = fixture();
        mutate(&mut s, 0, kind);
        s.vm.world.objects.get_mut(&0).unwrap().data.insert(1, -8);
        assert_eq!(
            check(&s, "unchanged compass bypasses movement rejection"),
            Outcome::Returned(Ok(()))
        );
        s.vm.world.objects.get_mut(&0).unwrap().data.insert(1, -7);
        assert_eq!(
            check(&s, "changed compass rejects group/route/turn"),
            Outcome::Returned(Err(MOVEMENT.into()))
        );
    }
    let mut s = fixture();
    s.bodies.get_mut(&0).unwrap().facing = 3;
    assert_eq!(
        check(&s, "existing invalid facing shares matching trailing zero"),
        Outcome::Returned(Ok(()))
    );
}
fn random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state
}
#[test]
fn directions_seeded_sparse_registries_match_through_serialization() {
    for seed in 0..4096u64 {
        let mut rng = seed + 1;
        let mut s = empty();
        for n in 0..24 {
            let id = if n < 8 {
                IDS[n]
            } else {
                random(&mut rng) as u16 as i16
            };
            let flags = random(&mut rng);
            if flags & 3 != 0 {
                let mut b = body();
                b.facing = (flags >> 9) as u8;
                s.bodies.insert(id, b);
            }
            if flags & 12 != 0 {
                s.vm.world.objects.insert(
                    id,
                    Object {
                        data: if flags & 16 != 0 {
                            BTreeMap::from([(1, (flags >> 17) as i16)])
                        } else {
                            BTreeMap::new()
                        },
                        ..Default::default()
                    },
                );
            }
            if flags & 32 != 0 {
                add_route(&mut s, id, random(&mut rng));
            }
            if flags & 64 != 0 {
                add_turn(&mut s, id);
            }
            if flags & 128 != 0 && s.vm.world.objects.contains_key(&id) {
                add_group(&mut s, id);
            }
        }
        let restored: Simulation =
            serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        let _ = check(&restored, &format!("seed={seed}"));
    }
}
#[test]
fn directions_observe_between_call_registry_and_raw_word_changes() {
    let mut s = fixture();
    for (step, (id, kind)) in [
        (0, 3),
        (9, 8),
        (ObjectId::MIN, 3),
        (-2, 7),
        (2, 4),
        (2, 3),
        (2, 8),
        (ObjectId::MAX, 1),
    ]
    .into_iter()
    .enumerate()
    {
        mutate(&mut s, id, kind);
        let restored: Simulation =
            serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        let _ = check(&restored, &format!("external call {step}"));
        let _ = invoke(&mut s, false);
    }
}
