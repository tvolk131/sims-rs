//! Frozen pre-optimization validator and publisher, used only as a differential
//! reference. Keep their old cloning/error ordering independent of production.
use super::{Point, Simulation, tests::fixture};
use crate::{Step, StepStop, lot_links::Links};
use sims_vm::{ObjectId, Program, World};
use std::collections::BTreeSet;

fn legacy_publish(links: &Links, world: &mut World) -> Result<(), String> {
    links.validate_arrays()?;
    let mut chains = vec![];
    let mut seen = BTreeSet::new();
    for head in links.heads.iter().flatten() {
        if let Some(id) = (*head > 0 && world.objects.contains_key(head)).then_some(*head) {
            let chain = links.walk(world, id)?;
            if chain.iter().any(|&id| !seen.insert(id)) {
                return Err("object occurs in multiple native tile chains".into());
            }
            chains.push(chain);
        }
    }
    world
        .person
        .as_mut()
        .ok_or("tile registry unavailable")?
        .tile_chains = chains;
    Ok(())
}

fn legacy_validate(s: &Simulation) -> Result<(), String> {
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
    let mut world = s.vm.world.clone();
    legacy_publish(links, &mut world)?;
    if world.person.as_ref().map(|p| &p.tile_chains)
        != s.vm.world.person.as_ref().map(|p| &p.tile_chains)
    {
        return Err("native runtime physical tile order mismatch".into());
    }
    let people: Vec<ObjectId> = world
        .objects
        .iter()
        .filter_map(|(&id, o)| o.person.as_ref().map(|_| id))
        .collect();
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
    Ok(())
}

#[test]
fn borrowed_validation_preserves_errors_and_publisher_results() {
    type Mutation = (&'static str, fn(&mut Simulation));
    let mutations: &[Mutation] = &[
        ("valid", |_| {}),
        ("no host", |s| s.native_lot = None),
        ("no architecture", |s| s.architecture = None),
        ("bad size", |s| {
            s.native_lot.as_mut().unwrap().placement.links.size = 1
        }),
        ("bad head array", |s| {
            s.native_lot.as_mut().unwrap().placement.links.heads[0]
                .pop()
                .map(|_| ())
                .unwrap()
        }),
        ("bad flags", |s| {
            s.native_lot.as_mut().unwrap().placement.flags.clear()
        }),
        ("missing next", |s| {
            s.native_lot
                .as_mut()
                .unwrap()
                .placement
                .links
                .next
                .remove(&2);
        }),
        ("missing bounds", |s| {
            s.native_lot.as_mut().unwrap().placement.bounds.remove(&2);
        }),
        ("missing position", |s| {
            s.vm.world.positions.remove(&2);
        }),
        ("no tile registry", |s| s.vm.world.person = None),
        ("stale published order", |s| {
            if let Some(person) = &mut s.vm.world.person {
                person.tile_chains.clear();
            }
        }),
        ("duplicate head", |s| {
            s.native_lot.as_mut().unwrap().placement.links.heads[1][14] = 1
        }),
        ("unregistered head", |s| {
            s.native_lot.as_mut().unwrap().placement.links.heads[1][14] = 99
        }),
        ("cycle", |s| {
            s.native_lot
                .as_mut()
                .unwrap()
                .placement
                .links
                .next
                .insert(2, 1);
        }),
        ("negative slot", |s| {
            s.vm.world.objects.get_mut(&2).unwrap().data.insert(14, -2);
        }),
        ("lost predecessor", |s| {
            s.vm.world.objects.get_mut(&2).unwrap().data.insert(26, 0);
        }),
        ("duplicate child", |s| {
            s.vm.world.objects.get_mut(&1).unwrap().slots = vec![Some(3)]
        }),
        ("room membership", |s| {
            s.native_lot.as_mut().unwrap().membership.0.insert(1, 0);
        }),
        ("body position", |s| {
            if let Some(body) = s.bodies.get_mut(&1) {
                body.position.x += 1;
            }
        }),
        ("lost body", |s| {
            s.bodies.remove(&1);
        }),
        ("lost unplaced", |s| {
            s.unplaced.remove(&4);
        }),
        ("selected ordinary object", |s| {
            s.native_lot.as_mut().unwrap().selected_person = Some(1)
        }),
        ("missing indicator", |s| {
            s.native_lot.as_mut().unwrap().indicators.primary = Some(99)
        }),
    ];
    let (_, initial) = fixture();
    for &(first_name, first) in mutations {
        for &(second_name, second) in mutations {
            let mut s = initial.clone();
            first(&mut s);
            // A removed host short-circuits every check; avoid mutating it again.
            if s.native_lot.is_some() {
                second(&mut s);
            }
            let before = s.clone();
            assert_eq!(
                s.validate_native_lot(),
                legacy_validate(&s),
                "{first_name}/{second_name}"
            );
            assert_eq!(s, before, "validation mutated {first_name}/{second_name}");
            if let Some(native) = &s.native_lot {
                let mut old = s.vm.world.clone();
                let mut new = old.clone();
                assert_eq!(
                    native.placement.links.publish_tile_chains(&mut new),
                    legacy_publish(&native.placement.links, &mut old),
                    "publisher {first_name}/{second_name}"
                );
                assert_eq!(old, new, "publisher state {first_name}/{second_name}");
            }
        }
    }
}

fn legacy_slice(s: &mut Simulation, p: &Program, budget: usize) -> Result<Step, String> {
    legacy_validate(s)?;
    let result = s.run_current_tick_inner(p, budget, Simulation::next_fixture_thread);
    s.sync_spatial_world();
    legacy_validate(s)?;
    result
}

#[test]
fn borrowed_validation_preserves_every_slice_and_saved_continuation() {
    use sims_formats::behavior::Instruction;
    let (mut p, mut initial) = fixture();
    p.modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .instructions = vec![
        Instruction {
            opcode: 2,
            on_true: 1,
            on_false: 255,
            operands: [34, 0, 1, 0, 0, 3, 6, 7],
        },
        Instruction {
            opcode: 2,
            on_true: 2,
            on_false: 255,
            operands: [0, 0, 1, 0, 0, 5, 9, 7],
        },
        Instruction {
            opcode: 0,
            on_true: 0,
            on_false: 255,
            operands: [0; 8],
        },
    ];
    p.modules
        .get_mut("objects")
        .unwrap()
        .routines
        .get_mut(&4096)
        .unwrap()
        .body
        .args = 1;
    let thread = initial.vm.object_threads[&4];
    let t = initial.vm.threads.get_mut(&thread).unwrap();
    t.stack.last_mut().unwrap().args = vec![0];
    t.frame_memory.resize(12, Some(0));
    initial.vm.world.globals.insert(34, 0);
    for budget in [1, 2, 1000] {
        let mut old = initial.clone();
        let mut new = initial.clone();
        let mut slices = 0;
        for _ in 0..20 {
            loop {
                let a = legacy_slice(&mut old, &p, budget);
                let b = new.run_fixture_tick(&p, budget);
                assert_eq!(a, b, "budget {budget}, slice {slices}");
                assert_eq!(old, new, "state at budget {budget}, slice {slices}");
                slices += 1;
                let step = b.unwrap();
                assert_ne!(step.stop, StepStop::Faulted);
                let saved = crate::Snapshot::capture(&p, &new);
                new =
                    serde_json::from_slice::<crate::Snapshot>(&serde_json::to_vec(&saved).unwrap())
                        .unwrap()
                        .restore(&p)
                        .unwrap();
                assert_eq!(old, new);
                if step.stop != StepStop::Budget {
                    break;
                }
                assert!(slices < 500);
            }
            old.advance_fixture_tick().unwrap();
            new.advance_fixture_tick().unwrap();
            assert_eq!(old, new);
        }
        assert!(new.vm.world.globals[&34] > 1);
        if budget == 1 {
            assert!(slices > 20);
        }
    }
    // Preserve the same prevalidation short circuit and postvalidation ordering
    // even when the inner execution reports an error or a VM fault.
    for budget in [0, 1] {
        let mut old = initial.clone();
        let mut new = initial.clone();
        old.vm.world.globals.clear();
        new.vm.world.globals.clear();
        assert_eq!(
            legacy_slice(&mut old, &p, budget),
            new.run_fixture_tick(&p, budget)
        );
        assert_eq!(old, new);
    }
}

#[test]
#[ignore = "requires private retained Mashuga snapshots; prints comparative release timings"]
fn borrowed_validation_matches_real_mashuga_and_measures_cost() {
    use std::{hint::black_box, time::Instant};
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
        let before = s.clone();
        assert_eq!(legacy_validate(&s), s.validate_native_lot());
        s.validate_native_lot().unwrap();
        let measure = |check: fn(&Simulation) -> Result<(), String>| {
            let start = Instant::now();
            for _ in 0..200 {
                black_box(check(black_box(&s))).unwrap();
            }
            start.elapsed().as_nanos() as f64 / 200.0 / 1000.0
        };
        let mut old = vec![];
        let mut new = vec![];
        for round in 0..5 {
            if round % 2 == 0 {
                old.push(measure(legacy_validate));
                new.push(measure(Simulation::validate_native_lot));
            } else {
                new.push(measure(Simulation::validate_native_lot));
                old.push(measure(legacy_validate));
            }
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        assert_eq!(s, before);
        println!(
            "{relative}: {} objects, median validation {:.1} -> {:.1} us ({:.2}x); five alternating 200-call batches, unchanged full state",
            s.vm.world.objects.len(),
            old[2],
            new[2],
            old[2] / new[2]
        );
    }
}
