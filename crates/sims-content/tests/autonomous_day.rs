//! Private end-to-end acceptance of original household scripts and shared services.
use sims_content::{Index, household};
use sims_sim::{Snapshot, StepStop};
use std::collections::BTreeSet;
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_household_day_with_player_intervention_is_budget_and_save_invariant() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    let (p, mut full) = household::autonomous_household(&index).unwrap();
    let mut split = full.clone();
    let mut increases = [0f32; 4];
    let indices = [5, 7, 8, 9];
    let mut autonomous_targets = BTreeSet::new();
    let mut saved_services = BTreeSet::new();
    let mut player_first = None;
    let mut player_second = None;
    let mut stood = false;
    let mut cancelled = false;
    let mut frames = 0;
    for tick in 1..=43200 {
        if tick == 5000 {
            let active = full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .unwrap()
                .id;
            for s in [&mut full, &mut split] {
                s.cancel_interaction(&p, 1, u64::from(active)).unwrap();
            }
            cancelled = true;
            let a = full.enqueue(&p, 1, 2, 0).unwrap();
            assert_eq!(split.enqueue(&p, 1, 2, 0).unwrap(), a);
            player_first = Some(a);
            let b = full.enqueue(&p, 1, 3, 0).unwrap();
            assert_eq!(split.enqueue(&p, 1, 3, 0).unwrap(), b);
            for s in [&mut full, &mut split] {
                s.cancel_interaction(&p, 1, b).unwrap();
            }
        }
        if player_first.is_some()
            && player_second.is_none()
            && full.vm.world.objects[&1].container == Some((2, 0))
        {
            let id = full.enqueue(&p, 1, 3, 0).unwrap();
            assert_eq!(split.enqueue(&p, 1, 3, 0).unwrap(), id);
            player_second = Some(id);
        }
        if !stood && player_second.is_some() && full.vm.world.objects[&1].container == Some((3, 0))
        {
            for s in [&mut full, &mut split] {
                s.cancel_interaction(&p, 1, player_second.unwrap()).unwrap();
            }
            stood = true;
        }
        let before = full.vm.world.objects[&1]
            .motives
            .unwrap()
            .map(f32::from_bits);
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let whole = full.run_fixture_tick(&p, 2000).unwrap();
        assert!(
            !matches!(
                whole.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "whole stop {:?} tick {tick}",
            whole.stop
        );
        let mut trace = vec![];
        loop {
            let part = split.run_fixture_tick(&p, 1).unwrap();
            trace.extend(part.trace);
            assert!(
                !matches!(part.stop, StepStop::Faulted | StepStop::Animation),
                "split stop {:?} tick {tick}",
                part.stop
            );
            let mut services = vec![];
            if !split.routes.is_empty() {
                services.push("routing");
            }
            if !split.turns.is_empty() {
                services.push("turning");
            }
            if !split.animations.is_empty() {
                services.push("animation");
            }
            if !split.vm.world.objects[&1].accessories.is_empty() {
                services.push("accessory");
            }
            if split.vm.world.objects[&1].container.is_some() {
                services.push("seated");
            }
            if !split.removals.is_empty() {
                services.push("deletion");
            }
            let checkpoint = services.into_iter().any(|kind| saved_services.insert(kind));
            if checkpoint {
                split = Snapshot::capture(&p, &split).restore(&p).unwrap();
                frames += 1;
            }
            if part.stop != StepStop::Budget {
                break;
            }
        }
        assert!(whole.trace == trace, "trace divergence at {tick}");
        assert!(full == split, "state divergence at {tick}");
        let after = full.vm.world.objects[&1]
            .motives
            .unwrap()
            .map(f32::from_bits);
        if let Some(a) = &full.vm.world.actions.as_ref().unwrap().actors[&1].active {
            if a.priority < 100 {
                autonomous_targets.insert(a.target);
                for (i, motive) in indices.iter().enumerate() {
                    increases[i] += (after[*motive] - before[*motive]).max(0.);
                }
            }
        }
        if tick % 250 == 0 {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
            frames += 1;
        }
        for s in [&mut full, &mut split] {
            s.vm.sounds.clear();
            s.household.as_mut().unwrap().clock_events.clear();
        }
    }
    assert!(
        cancelled && player_first.is_some() && player_second.is_some() && stood,
        "player sequence did not complete"
    );
    for (index, value) in indices.into_iter().zip(increases) {
        assert!(value > 5., "motive {index} never recovered: {value}");
    }
    for id in [4, 6] {
        assert!(autonomous_targets.contains(&id), "object {id} unused");
    }
    assert!(autonomous_targets.contains(&5) || autonomous_targets.contains(&9));
    assert!(autonomous_targets.contains(&7) || autonomous_targets.contains(&8));
    for service in ["routing", "animation", "accessory", "seated", "deletion"] {
        assert!(saved_services.contains(service), "no {service} save");
    }
    assert_eq!(full.household.as_ref().unwrap().clock.elapsed_days, 1);
    assert_eq!(full.household.as_ref().unwrap().clock.hour, 8);
    Snapshot::capture(&p, &full).restore(&p).unwrap();
    eprintln!(
        "43200 deterministic ticks, {frames} saves, autonomous motive gains {increases:?}; player queue/cancel/transfer/stand completed"
    );
}
