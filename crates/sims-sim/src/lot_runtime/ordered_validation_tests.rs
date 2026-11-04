//! Freeze the complete lookup-based validator before ordered body traversal.
//! This reference intentionally keeps its independent BTreeMap::get path and
//! all checks before/after it, including cursor construction and queue checks.
use super::{Point, Simulation, tests::fixture};
use sims_vm::{ObjectId, autonomy::Placement};

fn lookup_reference(s: &Simulation) -> Result<(), String> {
    let Some(native) = &s.native_lot else {
        return Ok(());
    };
    let lot = s
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .ok_or("native runtime requires original architecture")?;
    let links = &native.placement.links;
    links.validate_arrays()?;
    if let Some(map) = s.vm.world.map_size {
        map.checked_size(lot.size)?;
    }
    if links.size != lot.size
        || links.heads.len() != lot.floors.len()
        || native.placement.flags.len() != links.heads.len()
        || native
            .placement
            .flags
            .iter()
            .any(|f| f.len() != lot.size * lot.size)
        || links.next.keys().ne(s.vm.world.objects.keys())
        || native.placement.bounds.keys().ne(s.vm.world.objects.keys())
        || s.vm.world.positions.keys().ne(s.vm.world.objects.keys())
    {
        return Err("native runtime spatial registry mismatch".into());
    }
    let world = &s.vm.world;
    links.validate_tile_chains(world)?;
    let people = world
        .objects
        .iter()
        .filter_map(|(&id, o)| o.person.as_ref().map(|_| id));
    if native.membership.0.keys().copied().ne(people) {
        return Err("native runtime room membership registry mismatch".into());
    }
    for (&id, p) in &world.positions {
        if let Some(b) = s.bodies.get(&id) {
            if b.position != Point::new(p.y, p.x) || b.level != p.level {
                return Err(format!(
                    "native runtime body #{id} bypassed physical placement"
                ));
            }
        } else if s.cursor_constructor_pending(id) {
            s.validate_cursor_constructor(id)?;
        } else if !s.unplaced.contains(&id) || (p.x >= 0 && p.y >= 0) {
            return Err(format!(
                "native runtime object #{id} has no placement cache"
            ));
        }
    }
    if native.selected_person.is_some_and(|id| {
        s.vm.world
            .objects
            .get(&id)
            .is_none_or(|o| o.person.is_none())
    }) {
        return Err("native selected person is absent".into());
    }
    for id in [native.indicators.primary, native.indicators.secondary]
        .into_iter()
        .flatten()
    {
        if !world.objects.contains_key(&id) {
            return Err("native runtime indicator is not registered".into());
        }
    }
    world.validate_indicator_queues()?;
    Ok(())
}

fn signed_fixture() -> Simulation {
    let (_, mut s) = fixture();
    let object = s.vm.world.objects[&4].clone();
    let template = s.bodies[&1].clone();
    // Include both signed extremes, holes in either map, and ID zero. The
    // validator historically ignores bodies with no corresponding position.
    for id in [ObjectId::MIN, -2, 0, 6, ObjectId::MAX] {
        s.vm.world.objects.insert(id, object.clone());
        s.vm.world.positions.insert(
            id,
            Placement {
                x: -16,
                y: -32,
                level: 2,
            },
        );
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .links
            .next
            .insert(id, 0);
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(id, [0; 4]);
        if id == -2 || id == 6 {
            s.unplaced.insert(id);
        } else {
            let mut body = template.clone();
            body.position = Point::new(-32, -16);
            body.level = 2;
            s.bodies.insert(id, body);
        }
    }
    for id in [ObjectId::MIN + 1, -1, 5, ObjectId::MAX - 1] {
        s.bodies.insert(id, template.clone());
    }
    assert_eq!(lookup_reference(&s), Ok(()));
    s
}

fn cursor_fixture() -> Simulation {
    let (_, mut s) = fixture();
    s.vm.world.scheduler_storage = Some(Default::default());
    let mut object = s.vm.world.objects.remove(&4).unwrap();
    object.cursor = Some(sims_vm::cursor::State::constructed(None, None));
    object.container = None;
    object.slots.clear();
    object.data.extend([(14, 0), (26, 0)]);
    let definition = object.definition.clone().unwrap();
    s.vm.world
        .register_scheduler_object(4, &mut object)
        .unwrap();
    s.vm.world.objects.insert(4, object);
    let handle =
        s.vm.world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(4)
            .unwrap();
    s.vm.world.positions.insert(
        4,
        Placement {
            x: 0,
            y: 0,
            level: 0,
        },
    );
    s.unplaced.remove(&4);
    s.native_lot
        .as_mut()
        .unwrap()
        .placement
        .bounds
        .insert(4, [0; 4]);
    s.cursor_factory = Some(
        serde_json::from_value(serde_json::json!({
            "tick": s.vm.tick, "definition": definition,
            "date": {"year": 0, "month": 0, "day": 0},
            "selected": 1, "furniture": null, "current": null,
            "grouped": false, "position": [0, 0], "level": 0,
            "created": [handle], "selected_cursor": null,
            "phase": "Postconstruct", "reset": null
        }))
        .unwrap(),
    );
    assert!(s.cursor_constructor_pending(4));
    assert_eq!(s.validate_cursor_constructor(4), Ok(()));
    assert_eq!(lookup_reference(&s), Ok(()));
    s
}

type Mutation = (&'static str, fn(&mut Simulation));
fn check(s: &Simulation, label: &str) {
    let before = s.clone();
    let expected = lookup_reference(s);
    assert_eq!(s, &before, "reference mutated {label}");
    assert_eq!(s.validate_native_lot(), expected, "result differs: {label}");
    assert_eq!(s, &before, "validator mutated {label}");
}

#[test]
fn ordered_body_validation_preserves_signed_registry_and_first_errors() {
    let mutations: &[Mutation] = &[
        ("unchanged", |_| {}),
        ("host absent", |s| s.native_lot = None),
        ("architecture absent", |s| s.architecture = None),
        ("array length", |s| {
            if let Some(n) = &mut s.native_lot {
                n.placement.links.heads[0].pop();
            }
        }),
        ("negative retained size", |s| {
            s.vm.world.map_size = Some(sims_vm::world_map::State { active_size: -1 })
        }),
        ("retained size exceeds stride", |s| {
            s.vm.world.map_size = Some(sims_vm::world_map::State { active_size: 7 })
        }),
        ("flags length", |s| {
            if let Some(n) = &mut s.native_lot {
                n.placement.flags.clear();
            }
        }),
        ("next registry", |s| {
            if let Some(n) = &mut s.native_lot {
                n.placement.links.next.remove(&4);
            }
        }),
        ("bounds registry", |s| {
            if let Some(n) = &mut s.native_lot {
                n.placement.bounds.remove(&4);
            }
        }),
        ("position registry", |s| {
            s.vm.world.positions.remove(&4);
        }),
        ("object registry", |s| {
            s.vm.world.objects.remove(&4);
        }),
        ("tile registry absent", |s| s.vm.world.person = None),
        ("tile chain order", |s| {
            if let Some(p) = &mut s.vm.world.person {
                p.tile_chains.clear();
            }
        }),
        ("room registry", |s| {
            if let Some(n) = &mut s.native_lot {
                n.membership.0.insert(4, 0);
            }
        }),
        ("first body position", |s| {
            if let Some(b) = s.bodies.get_mut(&1) {
                b.position.x += 1;
            }
        }),
        ("later body level", |s| {
            if let Some(b) = s.bodies.get_mut(&3) {
                b.level += 1;
            }
        }),
        ("first body missing", |s| {
            s.bodies.remove(&1);
        }),
        ("all bodies missing", |s| s.bodies.clear()),
        ("unplaced absent", |s| {
            s.unplaced.remove(&4);
        }),
        ("unplaced nonnegative", |s| {
            if let Some(p) = s.vm.world.positions.get_mut(&4) {
                p.x = 0;
                p.y = 0;
            }
        }),
        ("unplaced one negative axis", |s| {
            if let Some(p) = s.vm.world.positions.get_mut(&4) {
                p.x = 0;
            }
        }),
        ("unplaced with matching body", |s| {
            if let Some(p) = s.vm.world.positions.get(&4) {
                let mut b = s
                    .bodies
                    .get(&1)
                    .cloned()
                    .unwrap_or_else(|| fixture().1.bodies[&1].clone());
                b.position = Point::new(p.y, p.x);
                b.level = p.level;
                s.bodies.insert(4, b);
            }
        }),
        ("selected ordinary object", |s| {
            if let Some(n) = &mut s.native_lot {
                n.selected_person = Some(1);
            }
        }),
        ("selected missing", |s| {
            if let Some(n) = &mut s.native_lot {
                n.selected_person = Some(99);
            }
        }),
        ("primary indicator absent", |s| {
            if let Some(n) = &mut s.native_lot {
                n.indicators.primary = Some(99);
            }
        }),
        ("secondary indicator absent", |s| {
            if let Some(n) = &mut s.native_lot {
                n.indicators.secondary = Some(98);
            }
        }),
        ("date queue absent", |s| {
            s.vm.world.indicator_queues.dates.push(Some(99))
        }),
        ("star queue zero", |s| {
            s.vm.world.indicator_queues.stars.push(Some(0))
        }),
        ("null queue allowed", |s| {
            s.vm.world.indicator_queues.stars.push(None)
        }),
    ];
    let (_, ordinary) = fixture();
    let fixtures = [
        ("ordinary", ordinary),
        ("signed", signed_fixture()),
        ("pending cursor", cursor_fixture()),
    ];
    let mut cases = 0;
    for (name, initial) in fixtures {
        for &(first_name, first) in mutations {
            for &(second_name, second) in mutations {
                let mut s = initial.clone();
                first(&mut s);
                second(&mut s);
                check(&s, &format!("{name}/{first_name}/{second_name}"));
                cases += 1;
            }
        }
    }
    eprintln!("{cases} complete-validator ordered mutation pairs matched");
    // Two errors at signed extremes must report the lower signed ID, not
    // insertion order, unsigned order, or the extra body-only map entries.
    let mut s = signed_fixture();
    s.bodies.get_mut(&ObjectId::MAX).unwrap().level += 1;
    s.bodies.get_mut(&ObjectId::MIN).unwrap().level += 1;
    assert_eq!(
        s.validate_native_lot(),
        Err(format!(
            "native runtime body #{} bypassed physical placement",
            ObjectId::MIN
        ))
    );
    check(&s, "signed extremes first error");
    s.bodies.remove(&ObjectId::MIN);
    assert_eq!(
        s.validate_native_lot(),
        Err(format!(
            "native runtime object #{} has no placement cache",
            ObjectId::MIN
        ))
    );
    check(&s, "signed missing body first error");
}

#[test]
fn ordered_body_validation_preserves_pending_cursor_and_body_precedence() {
    let mutations: &[Mutation] = &[
        ("valid constructor", |_| {}),
        ("cursor owner absent", |s| s.cursor_factory = None),
        ("postconstructed", |s| {
            if let Some(c) = s.vm.world.objects.get_mut(&4).unwrap().cursor.as_mut() {
                c.postconstructed = true;
            }
        }),
        ("cursor absent", |s| {
            s.vm.world.objects.get_mut(&4).unwrap().cursor = None
        }),
        ("constructor position", |s| {
            s.vm.world.positions.get_mut(&4).unwrap().x = 1
        }),
        ("constructor level", |s| {
            s.vm.world.positions.get_mut(&4).unwrap().level = 1
        }),
        ("constructor container", |s| {
            s.vm.world.objects.get_mut(&4).unwrap().container = Some((1, 0))
        }),
        ("constructor slots", |s| {
            s.vm.world.objects.get_mut(&4).unwrap().slots.push(None)
        }),
        ("constructor word14", |s| {
            s.vm.world.objects.get_mut(&4).unwrap().data.insert(14, -1);
        }),
        ("constructor word26", |s| {
            s.vm.world.objects.get_mut(&4).unwrap().data.insert(26, 1);
        }),
        ("constructor next", |s| {
            s.native_lot
                .as_mut()
                .unwrap()
                .placement
                .links
                .next
                .insert(4, 1);
        }),
        ("constructor bounds", |s| {
            s.native_lot
                .as_mut()
                .unwrap()
                .placement
                .bounds
                .insert(4, [1; 4]);
        }),
        ("constructor unplaced", |s| {
            s.unplaced.insert(4);
        }),
        ("matching body wins", |s| {
            let p = s.vm.world.positions[&4];
            let mut b = s.bodies[&1].clone();
            b.position = Point::new(p.y, p.x);
            b.level = p.level;
            s.bodies.insert(4, b);
        }),
        ("mismatching body", |s| {
            let b = s.bodies[&1].clone();
            s.bodies.insert(4, b);
        }),
        ("earlier body mismatch", |s| {
            s.bodies.get_mut(&1).unwrap().level += 1
        }),
        ("later queue error", |s| {
            s.vm.world.indicator_queues.stars.push(Some(99))
        }),
        ("pickup owner phase", |s| {
            if let Some(factory) = &s.cursor_factory {
                let mut value = serde_json::to_value(factory).unwrap();
                value["phase"] = "Pickup".into();
                s.cursor_factory = Some(serde_json::from_value(value).unwrap());
            }
        }),
        ("invalid owner phase", |s| {
            if let Some(factory) = &s.cursor_factory {
                let mut value = serde_json::to_value(factory).unwrap();
                value["phase"] = "Reset".into();
                s.cursor_factory = Some(serde_json::from_value(value).unwrap());
            }
        }),
        ("invalid owner handle", |s| {
            if let Some(factory) = &s.cursor_factory {
                let mut value = serde_json::to_value(factory).unwrap();
                value["created"] = serde_json::json!([1234]);
                s.cursor_factory = Some(serde_json::from_value(value).unwrap());
            }
        }),
    ];
    let initial = cursor_fixture();
    for &(first_name, first) in mutations {
        for &(second_name, second) in mutations {
            let mut s = initial.clone();
            first(&mut s);
            second(&mut s);
            check(&s, &format!("cursor/{first_name}/{second_name}"));
        }
    }
    eprintln!(
        "{} pending-cursor mutation pairs matched",
        mutations.len() * mutations.len()
    );
    let mut s = cursor_fixture();
    let mut b = s.bodies[&1].clone();
    b.position = Point::new(0, 0);
    b.level = 0;
    s.bodies.insert(4, b);
    s.vm.world.objects.get_mut(&4).unwrap().data.insert(14, -1);
    assert!(s.cursor_constructor_pending(4));
    assert_eq!(
        s.validate_cursor_constructor(4),
        Err("cursor constructor transient storage differs".into())
    );
    assert_eq!(
        s.validate_native_lot(),
        Ok(()),
        "matching body bypasses constructor-only checks"
    );
    s.bodies.get_mut(&4).unwrap().level = 1;
    assert_eq!(
        s.validate_native_lot(),
        Err("native runtime body #4 bypassed physical placement".into())
    );
}
