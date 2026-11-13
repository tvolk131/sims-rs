use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::{
    lot_acceptance::person::{Collision, Diagnostics, Reply, Request},
    lot_links::Links,
};
use sims_vm::{Object, World, autonomy::Placement};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Item {
    id: i16,
    position: [i32; 3],
    bounds: [i32; 4],
    data: BTreeMap<i16, i16>,
    slots: Vec<i16>,
    #[serde(default)]
    insets: u16,
    #[serde(default)]
    size: i16,
    #[serde(default)]
    marker: i16,
    person: Option<BTreeMap<i16, i16>>,
}
#[derive(Deserialize)]
struct Input {
    size: usize,
    heads: Vec<Vec<i16>>,
    objects: Vec<Item>,
    position: [i32; 2],
    level: i32,
    person: BTreeMap<i16, i16>,
    #[serde(default = "yes")]
    root_person: bool,
    replies: Vec<bool>,
    mutate_at: i32,
    wall_blocked: bool,
    initial_error: i32,
    initial_blocker: i16,
    #[serde(default)]
    unlink_after_callback: bool,
    #[serde(default)]
    ignore_walls_after_callback: bool,
    #[serde(default)]
    group: Vec<i16>,
}
fn yes() -> bool {
    true
}
fn compare(report: Value) {
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let mut w = World::default();
        let mut bounds: BTreeMap<_, _> = c.objects.iter().map(|o| (o.id, o.bounds)).collect();
        let mut links = Links {
            size: c.size,
            heads: c.heads,
            next: c.objects.iter().map(|o| (o.id, o.slots[0])).collect(),
        };
        for o in &c.objects {
            w.objects.insert(
                o.id,
                Object {
                    data: o.data.clone(),
                    person: if o.id == 1 && c.root_person {
                        Some(c.person.clone())
                    } else {
                        o.person.clone()
                    },
                    slots: o
                        .slots
                        .iter()
                        .skip(1)
                        .map(|&id| (id != 0).then_some(id))
                        .collect(),
                    ..Default::default()
                },
            );
            w.positions.insert(
                o.id,
                Placement {
                    x: o.position[0],
                    y: o.position[1],
                    level: o.position[2],
                },
            );
        }
        for (i, &id) in c.group.iter().enumerate() {
            w.objects.get_mut(&id).unwrap().multitile = Some(sims_vm::multitile::Part {
                head: (i != 0).then_some(c.group[0]),
                next: c.group.get(i + 1).copied(),
                origin: [0; 3],
                offset: [0; 3],
            });
        }
        let initial = Diagnostics {
            error: c.initial_error,
            blocker: Some(c.initial_blocker),
        };
        let mut op = Collision::begin_object(
            &mut w,
            &links,
            1,
            c.position,
            c.level,
            c.objects[0].size,
            c.objects[0].insets,
            initial,
        )
        .unwrap();
        let mut events = vec![];
        let mut calls = 0;
        while let Some(request) = op.next(&w, &links, &bounds).unwrap() {
            let saved = serde_json::to_vec(&op).unwrap();
            op = serde_json::from_slice(&saved).unwrap();
            assert_eq!(op.next(&w, &links, &bounds).unwrap(), Some(request));
            let old = op.clone();
            let wrong = match request {
                Request::Function { .. } | Request::WallObjects { .. } => Reply::Walls(false),
                Request::Walls { .. } => Reply::Function {
                    allowed: false,
                    diagnostics: initial,
                },
            };
            assert!(op.resume(request, wrong).is_err());
            assert_eq!(old, op);
            events.push(request);
            let reply = match request {
                Request::Function { .. } => {
                    if calls == c.mutate_at {
                        bounds.insert(2, [100, 100, 90, 90]);
                    }
                    if calls == 0 && c.unlink_after_callback {
                        links.next.insert(2, 0);
                    }
                    if calls == 0 && c.ignore_walls_after_callback {
                        w.objects.get_mut(&1).unwrap().data.insert(42, 0x503);
                    }
                    let allowed = c.replies.get(calls as usize).copied().unwrap_or(false);
                    calls += 1;
                    Reply::Function {
                        allowed,
                        diagnostics: initial,
                    }
                }
                Request::Walls { .. } => Reply::Walls(c.wall_blocked),
                Request::WallObjects { object, other } => Reply::WallObjects {
                    both_windows_or_neither: (c
                        .objects
                        .iter()
                        .find(|o| o.id == object)
                        .unwrap()
                        .marker
                        == 2)
                        == (c.objects.iter().find(|o| o.id == other).unwrap().marker == 2),
                },
            };
            op.resume(request, reply).unwrap();
        }
        let (accepted, d) = op.outcome().unwrap();
        assert_eq!(
            json!({"accepted":accepted,"error":d.error,"blocker":d.blocker,"events":events}),
            row["original"],
            "native collision case {i}: {}",
            row["input"]
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_object_floor_collision_callbacks_and_wall_objects() {
    compare(serde_json::from_str(include_str!("fixtures/object-floor-collision.json")).unwrap());
}

#[test]
#[ignore = "requires fresh private original object floor collision report"]
fn all_original_object_floor_collisions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/object-floor-collision.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_person_floor_collision_callbacks_and_resume() {
    compare(serde_json::from_str(include_str!("fixtures/person-floor-collision.json")).unwrap());
}
#[test]
#[ignore = "requires fresh private original person floor collision report"]
fn all_original_person_floor_collisions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cache/research/sims-exe/oracle/person-floor-collision.json");
    compare(serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn multipart_floor_collision_excludes_live_siblings() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/group-floor-collision.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 36);
    compare(report);
}
