use sims_content::{Index, household};
use sims_sim::{Snapshot, StepStop};

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_multitile_recliner_nap_recovers_energy_and_resumes() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    let (p, mut full) = household::needs_room(&index, "Recliners.iff", 16807, 5).unwrap();
    assert_eq!(
        full.vm.world.objects[&4]
            .definition
            .as_ref()
            .unwrap()
            .object,
        16809
    );
    assert_eq!(
        full.vm.world.objects[&5]
            .definition
            .as_ref()
            .unwrap()
            .object,
        16808
    );
    assert_eq!(full.vm.world.objects[&4].data[&41], 250);
    assert_eq!(
        sims_vm::multitile::chain(&full.vm.world, 5).unwrap(),
        [4, 5]
    );
    for id in [4, 5] {
        let d = full.vm.world.objects[&id].definition.as_ref().unwrap();
        assert_eq!(p.modules[&d.source].object_interactions[&d.object].len(), 2);
        assert!(full.vm.lifecycles[&id].run_init);
    }
    let mut split = full.clone();
    let mut clips = std::collections::BTreeSet::new();
    let (mut seated, mut positive, mut began) = (false, false, false);
    for _ in 0..20000 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let whole = full.run_fixture_tick(&p, 1000).unwrap();
        assert!(
            !matches!(
                whole.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "{:?}",
            full.vm.threads
        );
        let mut trace = vec![];
        loop {
            let part = split.run_fixture_tick(&p, 1).unwrap();
            trace.extend(part.trace);
            assert!(!matches!(
                part.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            if part.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(whole.trace, trace, "tick {}", full.vm.tick);
        assert_eq!(full, split, "tick {}", full.vm.tick);
        if let Some(a) = full.animations.get(&1) {
            clips.insert(a.clip.clone());
        }
        seated |= full.vm.world.objects[&1].container == Some((5, 0));
        positive |= f32::from_bits(full.vm.world.objects[&1].motives.unwrap()[5]) > 0.;
        began |= full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref()
            .is_some_and(|a| a.menu_id == 1 && [4, 5].contains(&a.target));
        if full.vm.tick % 250 == 0 {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        }
        if positive
            && full.vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .is_none()
        {
            break;
        }
    }
    assert!(seated && positive && began);
    for clip in [
        "a2o-recliner-nap-start",
        "a2o-recliner-nap-loop",
        "a2o-recliner-nap-stop",
    ] {
        assert!(clips.contains(clip), "{clips:?}");
    }
    assert!(
        full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .is_none()
    );
    // Original person Main can finish its post-action stand before the tick
    // returns. Both postures are valid here; require consistent containment
    // and then prove the next player action reaches its own chair.
    match full.vm.world.objects[&1].container {
        Some((5, 0)) => assert_eq!(full.vm.world.objects[&1].person.as_ref().unwrap()[&0], 1),
        None => assert_eq!(full.vm.world.objects[&1].person.as_ref().unwrap()[&0], 0),
        other => panic!("unexpected nap containment {other:?}"),
    }
    Snapshot::capture(&p, &full).restore(&p).unwrap();
    full.enqueue(&p, 1, 2, 0).unwrap();
    split = Snapshot::capture(&p, &full).restore(&p).unwrap();
    let mut departed = false;
    for _ in 0..2000 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let whole = full.run_fixture_tick(&p, 1000).unwrap();
        assert!(
            !matches!(
                whole.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "{:?}",
            full.vm.threads
        );
        let mut trace = vec![];
        loop {
            let step = split.run_fixture_tick(&p, 1).unwrap();
            trace.extend(step.trace);
            assert!(!matches!(
                step.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            if step.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(whole.trace, trace);
        assert_eq!(full, split);
        departed |= full.vm.world.objects[&1].container.is_none();
        if full.vm.tick % 50 == 0 {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        }
        if full.vm.world.objects[&1].container == Some((2, 0)) {
            assert!(departed);
            return;
        }
    }
    panic!("original recliner departure did not reach the player-selected chair");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_autonomous_toilet_use_completes_and_resumes_across_native_services() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let (p, mut full) = household::needs_room(&index, "Toilets.iff", 16807, 9).unwrap();
    let mut split = full.clone();
    let (mut began, mut seated, mut relative, mut numbered, mut cooldown) =
        (false, false, false, false, false);
    let mut held_pose = false;
    for _ in 0..1600 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let step = full.run_fixture_tick(&p, 1000).unwrap();
        assert!(
            !matches!(
                step.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "{:?}",
            full.vm.threads
        );
        let mut trace = vec![];
        loop {
            let step = split.run_fixture_tick(&p, 1).unwrap();
            trace.extend(step.trace);
            assert!(!matches!(
                step.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            if step.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(step.trace, trace);
        assert_eq!(full, split);
        let active = full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref();
        began |= active.is_some_and(|a| a.target == 4);
        seated |= full.vm.world.objects[&1].container == Some((4, 0));
        relative |= step.trace.iter().any(|t| t.opcode == 0x1b);
        numbered |= full
            .animations
            .get(&1)
            .is_some_and(|a| a.numbered_events > 0);
        cooldown |= full.vm.world.objects[&4].data[&25] > 0;
        held_pose |= full.animations.get(&1).is_some_and(|p| {
            p.completed
                && p.clip == "a2o-sit-front"
                && full.vm.world.objects[&1].container == Some((4, 0))
        });
        if full.vm.tick % 50 == 0
            || full
                .animations
                .get(&1)
                .is_some_and(|a| a.numbered_events > 0 && a.frame_fifths < 100)
        {
            let bytes = serde_json::to_vec(&Snapshot::capture(&p, &split)).unwrap();
            split = Snapshot::from_json(&bytes, &p).unwrap();
        }
        if began && active.is_none_or(|a| a.target != 4) {
            assert!(seated && relative && numbered && cooldown && held_pose);
            assert!(f32::from_bits(full.vm.world.objects[&1].motives.unwrap()[9]) > 90.);
            assert_eq!(full.vm.world.objects[&1].container, None);
            assert_eq!(full.bodies[&1].position, sims_sim::Point::new(88, 8));
            Snapshot::capture(&p, &full).restore(&p).unwrap();
            return;
        }
    }
    panic!("original toilet Use did not complete");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_autonomous_sink_washing_recovers_hygiene_and_resumes() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let (p, mut full) = household::needs_room(&index, "Sinks.iff", 16807, 8).unwrap();
    let mut split = full.clone();
    let (mut washed, mut footprint, mut stopped) = (false, false, false);
    for _ in 0..2000 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let step = full.run_fixture_tick(&p, 1000).unwrap();
        assert!(
            !matches!(
                step.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "{:?}",
            full.vm.threads
        );
        let mut trace = vec![];
        loop {
            let part = split.run_fixture_tick(&p, 1).unwrap();
            trace.extend(part.trace);
            assert!(!matches!(
                part.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            if part.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(step.trace, trace);
        assert_eq!(full, split);
        washed |= full
            .animations
            .get(&1)
            .is_some_and(|p| p.clip == "a2o-sink-washhands");
        footprint |= full.vm.world.objects[&1].person.as_ref().unwrap()[&72] != 0;
        stopped |= full.vm.sounds.iter().any(|s| s.stop && s.object == 4);
        if full.vm.tick % 50 == 0 {
            split = Snapshot::capture(&p, &split).restore(&p).unwrap();
        }
        let active = full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .as_ref();
        if f32::from_bits(full.vm.world.objects[&1].motives.unwrap()[8]) > 0.
            && active.is_none_or(|a| a.target != 4)
        {
            assert!(washed && footprint && stopped);
            Snapshot::capture(&p, &full).restore(&p).unwrap();
            return;
        }
    }
    panic!("original sink did not recover hygiene and complete");
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_fridge_eating_and_cleanup_resume_inside_native_services() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    let (p, mut full) = household::needs_room(&index, "Fridges.iff", 16807, 7).unwrap();
    let mut split = full.clone();
    let (mut init, mut main, mut named) = (false, false, false);
    let (mut pending_removal, mut removal_request, mut found_location) = (false, false, false);
    for _ in 0..1919 {
        full.advance_fixture_tick().unwrap();
        split.advance_fixture_tick().unwrap();
        let whole = full.run_fixture_tick(&p, 1000).unwrap();
        assert!(
            !matches!(
                whole.stop,
                StepStop::Faulted | StepStop::Budget | StepStop::Animation
            ),
            "{:?}",
            full.vm.threads
        );
        let mut trace = vec![];
        loop {
            let part = split.run_fixture_tick(&p, 1).unwrap();
            removal_request |= part.trace.iter().any(|t| t.opcode == 0x12);
            found_location |= part.trace.iter().any(|t| t.opcode == 0x10);
            trace.extend(part.trace);
            assert!(!matches!(
                part.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            let factory = split.vm.threads.values().find_map(|t| {
                if let sims_vm::Status::AwaitingCreation(r) = &t.status {
                    r.created
                } else {
                    None
                }
            });
            let in_named = split.vm.threads.values().any(|t| {
                t.stack
                    .iter()
                    .any(|f| f.code_owner.ends_with("::Food.iff") && f.location.bhav == 4169)
            });
            if let Some(id) = factory {
                assert!(split.unplaced.contains(&id));
                assert!(!split.bodies.contains_key(&id));
                assert_eq!(split.vm.world.objects[&id].container, None);
                match split.vm.lifecycles[&id].phase {
                    sims_vm::lifecycle::Phase::Init => init = true,
                    sims_vm::lifecycle::Phase::Main => main = true,
                    _ => {}
                }
                assert!(split.clone().advance_fixture_tick().is_err());
            }
            named |= in_named;
            let pending = !split.removals.is_empty();
            let removal = split
                .vm
                .threads
                .values()
                .any(|t| matches!(t.status, sims_vm::Status::AwaitingRemoval(_)));
            let placement = split
                .vm
                .threads
                .values()
                .any(|t| matches!(t.status, sims_vm::Status::AwaitingPlacement(_)));
            pending_removal |= pending;
            removal_request |= removal;
            found_location |= placement;
            if pending {
                assert!(split.clone().advance_fixture_tick().is_err());
            }
            if factory.is_some() || in_named || pending || removal || placement {
                split = Snapshot::capture(&p, &split).restore(&p).unwrap();
            }
            if part.stop != StepStop::Budget {
                break;
            }
        }
        assert_eq!(whole.trace, trace);
        assert_eq!(full, split, "tick {}", full.vm.tick);
        if full.vm.tick == 1000 {
            let food = &full.vm.world.objects[&5];
            assert_eq!(food.definition.as_ref().unwrap().guid, 0xf301e375);
            assert_eq!(food.container, Some((1, 0)));
            assert_eq!(full.vm.world.objects[&4].slots[0], None);
            assert_eq!(full.vm.world.objects[&1].slots[0], Some(5));
            assert_eq!(full.bodies[&5].position, full.bodies[&1].position);
            // Original Food Init sets placement flags to 3; the enclosing
            // native factory assigns its §15 price to the preceding word.
            assert_eq!(food.data[&41], 15);
            assert_eq!(food.data[&42], 3);
            assert_eq!(food.data[&62], 1);
            assert!(!full.animations[&1].pickup);
            assert_eq!(
                full.carry[&1].layer.as_ref().unwrap().clip,
                "a2o-rarm-carry-loop"
            );
            assert_eq!(
                full.vm.world.actions.as_ref().unwrap().actors[&1]
                    .active
                    .as_ref()
                    .unwrap()
                    .label
                    .as_deref(),
                Some("Eat")
            );
            assert!(f32::from_bits(full.vm.world.objects[&1].motives.unwrap()[7]) > -30.);
            assert!(full.unplaced.is_empty());
        }
    }
    assert!(init && main && named);
    assert!(pending_removal && removal_request && found_location);
    assert!(full.removals.is_empty() && full.unplaced.is_empty());
    assert!(
        full.vm
            .world
            .objects
            .values()
            .all(|o| o.definition.as_ref().is_none_or(|d| d.guid != 0xf301e375))
    );
    assert_eq!(full.vm.world.objects[&1].slots[0], None);
    assert!(
        full.vm.world.actions.as_ref().unwrap().actors[&1]
            .active
            .is_none()
    );
    assert!(f32::from_bits(full.vm.world.objects[&1].motives.unwrap()[7]) > 0.);
    Snapshot::capture(&p, &full).restore(&p).unwrap();
}

#[test]
#[ignore = "requires private Complete Collection assets"]
fn failed_food_placement_deletes_only_new_object_after_init_main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../game-data/cabinet-extracted");
    let index = Index::scan(&root).unwrap();
    let (mut p, mut s) = household::needs_room(&index, "Fridges.iff", 16807, 7).unwrap();
    let (thread, request) = 'outer: loop {
        s.advance_fixture_tick().unwrap();
        loop {
            let part = s.run_fixture_tick(&p, 1).unwrap();
            assert!(!matches!(
                part.stop,
                StepStop::Faulted | StepStop::Animation
            ));
            if let Some((&id,t))=s.vm.threads.iter().find(|(_,t)| matches!(&t.status,sims_vm::Status::AwaitingCreation(r) if r.created == Some(5))) {
                if let sims_vm::Status::AwaitingCreation(r)=&t.status {break 'outer (id,r.clone())}
            }
            if part.stop != StepStop::Budget {
                break;
            }
        }
        assert!(s.vm.tick < 600);
    };
    // Supply an occupied slot after the script has decided to create, using a
    // different GUID so the native duplicate-resource guard still passes.
    let body = s.bodies[&4].clone();
    household::add_object(&index, &mut p, &mut s, "DiningChairs.iff", 16807, 6, body).unwrap();
    let child = s.vm.world.objects.get_mut(&6).unwrap();
    child.container = Some((4, 0));
    child.data.extend([(2, 4), (3, 0), (14, 1), (26, 4)]);
    s.vm.world.objects.get_mut(&4).unwrap().slots[0] = Some(6);
    let mut initialized = false;
    let mut ran_main = false;
    let mut events = vec![];
    loop {
        let part = s.run_fixture_tick(&p, 1).unwrap();
        events.extend(part.events);
        assert!(!matches!(
            part.stop,
            StepStop::Faulted | StepStop::Animation
        ));
        if let Some(l) = s.vm.lifecycles.get(&5) {
            initialized |= l.phase == sims_vm::lifecycle::Phase::Init;
            ran_main |= l.phase == sims_vm::lifecycle::Phase::Main;
            s = Snapshot::capture(&p, &s).restore(&p).unwrap();
        }
        if !matches!(
            s.vm.threads[&thread].status,
            sims_vm::Status::AwaitingCreation(_)
        ) {
            break;
        }
    }
    assert!(initialized && ran_main);
    assert_eq!(
        s.vm.threads[&thread].stack.last().unwrap().stack_object,
        request.target
    );
    assert_eq!(s.vm.world.objects[&4].slots[0], Some(6));
    assert!(!s.vm.world.objects.contains_key(&5));
    assert!(!s.vm.object_threads.contains_key(&5));
    assert!(!s.vm.lifecycles.contains_key(&5));
    assert!(s.unplaced.is_empty());
    assert!(
        !s.vm
            .world
            .person
            .as_ref()
            .unwrap()
            .object_order
            .contains(&5)
    );
    let signal = events
        .iter()
        .position(|e| e.detail.contains("creation signal 0xe2"))
        .unwrap();
    let delete = events
        .iter()
        .position(|e| e.detail.contains("deleted newly created"))
        .unwrap();
    assert!(signal < delete);
    Snapshot::capture(&p, &s).restore(&p).unwrap();
}
