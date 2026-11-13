//! Acceptance of the bounded original-script household, requiring private assets.
use sims_content::{Index, household};
use sims_sim::{Simulation, Snapshot, StepStop};
use sims_vm::Program;
use std::collections::BTreeSet;
fn step(p: &Program, s: &mut Simulation, budget: usize) -> Vec<sims_vm::Trace> {
    s.advance_fixture_tick().unwrap();
    let mut trace = vec![];
    loop {
        let r = s
            .run_fixture_tick(p, budget)
            .unwrap_or_else(|e| panic!("tick {}: {e}", s.vm.tick));
        trace.extend(r.trace);
        assert!(
            !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
            "tick {}: {:?}",
            s.vm.tick,
            s.vm.threads
        );
        if r.stop != StepStop::Budget {
            break;
        }
    }
    s.vm.sounds.clear();
    s.household.as_mut().unwrap().clock_events.clear();
    trace
}
fn setup() -> (Program, Simulation) {
    let i = Index::scan(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted"),
    )
    .unwrap();
    household::autonomous_household(&i).unwrap()
}
fn menu(p: &Program, s: &Simulation, id: i16, bhav: u16) -> usize {
    let d = s.vm.world.objects[&id].definition.as_ref().unwrap();
    p.modules[&d.source].object_interactions[&d.object]
        .iter()
        .position(|e| e.action == bhav)
        .unwrap()
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn three_days_match_single_instruction_slices_and_saved_continuations() {
    let (p, mut full) = setup();
    let mut split = full.clone();
    let mut clips = BTreeSet::new();
    let mut cleaned = false;
    let mut active_saved = BTreeSet::new();
    let mut max_objects = 0;
    for tick in 1..=129600 {
        let trace = step(&p, &mut full, 2000);
        assert_eq!(trace, step(&p, &mut split, 1), "trace at {tick}");
        assert_eq!(full, split, "state at {tick}");
        max_objects = max_objects.max(full.vm.world.objects.len());
        cleaned |= trace
            .iter()
            .any(|t| t.location.source.ends_with("trashpile.iff") && t.opcode == 18);
        if let Some(a) = full.animations.get(&1) {
            clips.insert(a.clip.clone());
        }
        if tick % 997 == 0
            || full.carry.get(&1).is_some_and(|c| c.selector.mode == 6) && tick % 211 == 0
        {
            if let Some(a) = full.animations.get(&1) {
                active_saved.insert(a.clip.clone());
            }
            split = Snapshot::from_json(
                &serde_json::to_vec(&Snapshot::capture(&p, &split)).unwrap(),
                &p,
            )
            .unwrap();
        }
    }
    assert!(cleaned, "never disposed of autonomous trash");
    for part in ["shower1-scrub", "book", "trash"] {
        assert!(
            clips.iter().any(|c| c.contains(part)),
            "missing {part}: {clips:?}"
        );
    }
    assert!(active_saved.iter().any(|c| c.contains("book")));
    assert!(active_saved.iter().any(|c| c.contains("shower")));
    assert_eq!(full.household.as_ref().unwrap().clock.elapsed_days, 3);
    eprintln!(
        "3 days: {} clips, {} saved animation states, peak {} live objects, final {}",
        clips.len(),
        active_saved.len(),
        max_objects,
        full.vm.world.objects.len()
    );
    // Player interventions after three autonomous days, while the person is busy.
    let shower = menu(&p, &full, 9, 4098);
    let reading = menu(&p, &full, 10, 4110);
    for s in [&mut full, &mut split] {
        s.vm.world.person.as_mut().unwrap().free_will = false;
        s.enqueue(&p, 1, 9, shower).unwrap();
        let id = s.enqueue(&p, 1, 10, reading).unwrap();
        s.cancel_interaction(&p, 1, id).unwrap();
    }
    split = Snapshot::capture(&p, &split).restore(&p).unwrap();
    let mut cancelled = false;
    for tick in 0..12000 {
        assert_eq!(step(&p, &mut full, 2000), step(&p, &mut split, 1));
        assert_eq!(full, split);
        if !cancelled
            && full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .is_some_and(|a| a.target == 9)
            && full
                .animations
                .get(&1)
                .is_some_and(|a| a.clip.contains("shower1-scrub"))
        {
            let id = full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .unwrap()
                .id as u64;
            for s in [&mut full, &mut split] {
                s.cancel_interaction(&p, 1, id).unwrap();
            }
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
            cancelled = true;
        }
        if tick % 257 == 0 {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        }
        if cancelled
            && full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .is_none()
            && full.pending_interactions(1).unwrap().is_empty()
        {
            assert_eq!(
                full.vm.world.objects[&1].person.as_ref().unwrap()[&8],
                0,
                "cancelled shower did not redress"
            );
            eprintln!(
                "Player queue/cancel and shower exit completed at {}",
                full.vm.tick
            );
            return;
        }
    }
    panic!("player-directed shower cancellation did not complete");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn player_cleans_the_shower_puddle_with_original_behavior() {
    let (p, mut s) = setup();
    s.vm.world.person.as_mut().unwrap().free_will = false;
    let shower = menu(&p, &s, 9, 4098);
    s.enqueue(&p, 1, 9, shower).unwrap();
    let mut puddle = None;
    let mut began = false;
    for _ in 0..12000 {
        step(&p, &mut s, 2000);
        if s.vm.tick % 137 == 0 {
            s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        }
        let active = &s.vm.world.actions.as_ref().unwrap().actors[&1].active;
        began |= active.as_ref().is_some_and(|a| a.target == 9);
        if began
            && puddle.is_none()
            && active.is_none()
            && s.pending_interactions(1).unwrap().is_empty()
        {
            let id = *s
                .vm
                .world
                .objects
                .iter()
                .find(|(_, o)| o.definition.as_ref().is_some_and(|d| d.guid == 0x3e7470f6))
                .expect("shower created no puddle")
                .0;
            let clean = menu(&p, &s, id, 4097);
            s.enqueue(&p, 1, id, clean).unwrap();
            puddle = Some(id);
        }
        if puddle.is_some_and(|id| !s.vm.world.objects.contains_key(&id))
            && s.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .is_none()
        {
            eprintln!(
                "Original shower and puddle cleanup completed at {}",
                s.vm.tick
            );
            return;
        }
    }
    panic!("puddle cleanup failed to complete");
}
