//! Original adult need reactions must be available beyond the first household day.
use sims_content::{Index, household};
use sims_sim::{Snapshot, StepStop};
use std::collections::BTreeSet;

fn setup() -> (sims_vm::Program, sims_sim::Simulation) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    household::autonomous_household(&index).unwrap()
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_need_reaction_animation_references_are_loaded() {
    let (p, _) = setup();
    let person = p
        .modules
        .values()
        .find(|m| m.source.ends_with("PersonGlobals.iff"))
        .unwrap();
    let globals = &p.modules[&p.globals].strings[&156];
    let mut required = BTreeSet::new();
    for id in [8218, 8219, 8220, 8221, 8222, 8224, 8225, 8226] {
        let routine = &person.routines[&id];
        assert!(routine.name.starts_with("check "));
        for i in &routine.body.instructions {
            if i.opcode == 0x2c && i.operands[4] == 3 {
                let number = u16::from_le_bytes([i.operands[0], i.operands[1]]) as usize;
                if number != 0 {
                    required.insert(globals[number].to_ascii_lowercase());
                }
            }
        }
    }
    assert!(required.contains("a2o-bored1"));
    let missing: BTreeSet<_> = required
        .iter()
        .filter(|name| !p.animations.contains_key(*name))
        .collect();
    assert!(missing.is_empty(), "unloaded need reactions: {missing:?}");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn two_household_days_reach_boredom_and_resume_mid_reaction() {
    let (p, mut s) = setup();
    // This regression deliberately removes fun advertisements from the fixture
    // so its original low-Fun reaction remains reachable with a working bookcase.
    s.vm.world
        .person
        .as_mut()
        .unwrap()
        .autonomy
        .as_mut()
        .unwrap()
        .advertised
        .as_mut()
        .unwrap()
        .remove(&10);
    let mut reactions = BTreeSet::new();
    let mut continuation = None;
    for tick in 1..=86400 {
        s.advance_fixture_tick().unwrap();
        let step = s
            .run_fixture_tick(&p, 2000)
            .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
        assert!(
            !matches!(
                step.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "tick {tick}: {:?}",
            step.stop
        );
        if let Some(other) = &mut continuation {
            let other: &mut sims_sim::Simulation = other;
            other.advance_fixture_tick().unwrap();
            let replay = other.run_fixture_tick(&p, 2000).unwrap();
            assert_eq!(
                step.trace, replay.trace,
                "restored reaction trace at {tick}"
            );
            assert_eq!(&s, other, "restored reaction state at {tick}");
            other.vm.sounds.clear();
            other.household.as_mut().unwrap().clock_events.clear();
        }
        if let Some(a) = s.animations.get(&1) {
            if a.clip.starts_with("a2o-bored") {
                reactions.insert(a.clip.clone());
                if continuation.is_none() && a.frame_fifths > 0 && !a.completed {
                    eprintln!("Mid-boredom save at tick {tick}: {}", a.clip);
                    continuation = Some(Snapshot::capture(&p, &s).restore(&p).unwrap());
                    // Match the end-of-tick consumption below.
                    let other = continuation.as_mut().unwrap();
                    other.vm.sounds.clear();
                    other.household.as_mut().unwrap().clock_events.clear();
                }
            }
        }
        s.vm.sounds.clear();
        s.household.as_mut().unwrap().clock_events.clear();
    }
    assert!(reactions.contains("a2o-bored1"), "{reactions:?}");
    assert!(
        continuation.is_some(),
        "never saved an active boredom reaction"
    );
    assert_eq!(s.household.as_ref().unwrap().clock.elapsed_days, 2);
}
