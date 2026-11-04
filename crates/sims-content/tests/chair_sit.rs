use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, chair_room, load_chair_animations, load_program},
};
use sims_sim::{Snapshot, StepStop};
use sims_vm::Status;
use std::path::PathBuf;

#[test]
#[ignore = "requires the user's private Complete Collection corpus"]
fn original_sit_finishes_with_real_animation_events_and_mid_animation_resume() {
    let root = std::env::var_os("SIMS_DATA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
        });
    let index = Index::scan(&root).unwrap();
    let mut program = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    load_chair_animations(&index, &mut program).unwrap();
    for asset in program.animations.values() {
        assert_eq!(
            asset.samples.translations.len(),
            asset.clip.translation_count as usize
        );
        assert_eq!(
            asset.samples.rotations.len(),
            asset.clip.rotation_count as usize
        );
        assert!(
            asset
                .samples
                .translations
                .iter()
                .flatten()
                .chain(asset.samples.rotations.iter().flatten())
                .all(|x| x.is_finite())
        );
        assert!(asset.source.ends_with(".cmx.bcf"));
        assert!(asset.samples_source.ends_with(".cfp"));
    }
    for facing in [1, 4, 16, 64] {
        let fixture =
            ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty).unwrap();
        let mut a = chair_room(&program, fixture, false, facing).unwrap();
        a.run_fixture_tick(&program, 100).unwrap();
        // Original Sit dispatches on RouteEntryFlags, not the chair's absolute
        // facing. Keep exact clip/event timings without fixing a host path tie.
        let (name, frames, event_tick, end_tick) = match a.routes[&2].goal.entry_flags {
            1 => ("a2o-sit-front", 122, 95, 102),
            64 => ("a2o-sit-left", 161, 112, 135),
            16 => ("a2o-sit-behind", 161, 109, 135),
            4 => ("a2o-sit-right", 165, 113, 138),
            entry => panic!("invalid chair entry {entry}"),
        };
        assert_eq!(program.animations[name].clip.frames, frames);
        let arrival = u64::from(a.routes[&2].locomotion.duration());
        let event_tick = arrival + event_tick;
        let end_tick = arrival + end_tick;
        let mut checkpoints = Vec::new();
        let mut trace = Vec::new();
        let mut events = Vec::new();
        loop {
            let result = a.run_fixture_tick(&program, 100).unwrap();
            trace.extend(result.trace);
            events.extend(result.events);
            if [arrival + 46, event_tick - 1, event_tick].contains(&a.vm.tick) {
                assert_eq!(a.animations[&1].clip, name);
                assert!(!a.animations[&1].looping);
                checkpoints.push(
                    Snapshot::from_json(
                        &serde_json::to_vec(&Snapshot::capture(&program, &a)).unwrap(),
                        &program,
                    )
                    .unwrap(),
                );
            }
            if result.stop == StepStop::Complete {
                break;
            }
            assert_eq!(result.stop, StepStop::Waiting);
            assert!(a.vm.tick < 512);
            a.advance_fixture_tick().unwrap();
        }
        assert_eq!(a.vm.tick, end_tick);
        assert_eq!(a.vm.threads[&2].status, Status::Returned(true));
        assert_eq!(a.vm.world.objects[&1].container, Some((2, 0)));
        assert_eq!(a.vm.world.objects[&2].slots, [Some(1)]);
        assert_eq!(a.bodies[&1].facing, facing);
        assert_eq!(a.vm.world.objects[&1].person.as_ref().unwrap()[&0], 1);
        assert_eq!(a.bodies[&1].elevation, 7);
        assert_eq!(a.animations[&1].clip, "a2o-sit-loop");
        assert!(a.animations[&1].looping);
        assert!(a.reservations.is_empty());
        assert_eq!(a.vm.sounds.len(), 1);
        assert_eq!(a.vm.sounds[0].tick, event_tick);
        assert_eq!(a.vm.sounds[0].name, "chair_sit_wood");
        assert_eq!(a.vm.sounds[0].object, 2);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.detail.contains("delivered xevt 100"))
                .count(),
            1
        );
        assert!(trace.iter().any(|t| t.opcode == 0x31));
        for mut resumed in checkpoints {
            let checkpoint = resumed.vm.tick;
            let mut continued_trace = Vec::new();
            let mut continued_events = Vec::new();
            loop {
                resumed.advance_fixture_tick().unwrap();
                let result = resumed.run_fixture_tick(&program, 100).unwrap();
                continued_trace.extend(result.trace);
                continued_events.extend(result.events);
                if result.stop == StepStop::Complete {
                    break;
                }
                assert!(resumed.vm.tick <= end_tick);
            }
            assert_eq!(resumed, a);
            assert_eq!(
                continued_trace,
                trace
                    .iter()
                    .filter(|t| t.tick > checkpoint)
                    .cloned()
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                continued_events,
                events
                    .iter()
                    .filter(|e| e.tick > checkpoint)
                    .cloned()
                    .collect::<Vec<_>>()
            );
        }
        Snapshot::capture(&program, &a).restore(&program).unwrap();
    }
}
