use sims_content::{
    Index,
    vm::{lifecycle_fixture, load_program},
};
use sims_vm::{Snapshot, Status, Stop, Vm, lifecycle::Phase};
use std::path::PathBuf;
fn index() -> Index {
    Index::scan(
        &std::env::var_os("SIMS_DATA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted")
            }),
    )
    .unwrap()
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_cursor_init_and_main_resume_through_initialization_and_sleep() {
    let index = index();
    let p = load_program(&index, "Cursors.iff", "Global.iff").unwrap();
    let (_, world) = lifecycle_fixture(&index, "Cursors.iff", 128).unwrap();
    let mut s = Vm::new(world, 1);
    let id = s
        .reset_object_lifecycle(&p, 1, sims_vm::construction::ObjectDate::default())
        .unwrap();
    s.run_slice(&p, id, 5).unwrap();
    assert_eq!(s.lifecycles[&1].phase, Phase::Init);
    let mut resumed = Snapshot::capture(&p, &s).restore(&p).unwrap();
    for state in [&mut s, &mut resumed] {
        while state.run_slice(&p, id, 100).unwrap().stop == Stop::Budget {}
        assert_eq!(state.lifecycles[&1].phase, Phase::Main);
        assert!(matches!(state.threads[&id].status, Status::Sleeping { .. }));
        assert_eq!(state.world.objects[&1].data[&23], 2);
    }
    assert_eq!(s, resumed);
    resumed =
        Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p).unwrap();
    for state in [&mut s, &mut resumed] {
        for _ in 0..1010 {
            state.advance_tick().unwrap();
            state.run_slice(&p, id, 100).unwrap();
        }
    }
    assert_eq!(s, resumed);
    assert!(matches!(s.threads[&id].status, Status::Sleeping { .. }));
}
#[test]
#[ignore = "requires private Complete Collection assets"]
fn original_chair_init_refreshes_and_main_idles_with_snapshot_resume() {
    let index = index();
    let p = load_program(&index, "DiningChairs.iff", "Global.iff").unwrap();
    let (owner, world) = lifecycle_fixture(&index, "DiningChairs.iff", 16807).unwrap();
    let m = &p.modules[&owner];
    assert_eq!(m.object_functions[&16807][0].action, 4108);
    assert_eq!(m.object_functions[&16807][1].action, 4096);
    assert_eq!(m.object_function_sources[&16807], "OBJD fallback");
    assert_eq!(m.object_function_sources[&16811], "OBJf");
    let mut s = Vm::new(world, 1);
    let id = s
        .reset_object_lifecycle(&p, 1, sims_vm::construction::ObjectDate::default())
        .unwrap();
    s.run_slice(&p, id, 10).unwrap();
    let mut resumed =
        Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &s)).unwrap(), &p).unwrap();
    for state in [&mut s, &mut resumed] {
        while state.run_slice(&p, id, 200).unwrap().stop == Stop::Budget {}
        assert_eq!(state.lifecycles[&1].phase, Phase::Main);
        assert!(matches!(
            state.threads[&id].status,
            Status::Sleeping { wake_tick: 20000 }
        ));
        assert_eq!(state.world.objects[&1].data[&23], 2);
        assert_eq!(state.world.objects[&1].attributes[1], 1);
        let refresh = state.world.refresh.as_ref().unwrap();
        assert!(refresh.draw[&1].dynamic);
        assert!(!refresh.invalidated.contains(&1)); // Constructor already marks it dynamic.
    }
    assert_eq!(s, resumed);
    assert_eq!(s, Snapshot::capture(&p, &s).restore(&p).unwrap());
}
