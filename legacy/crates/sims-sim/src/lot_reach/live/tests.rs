use super::*;

fn fixture(transfer: bool) -> (Simulation, Program, ThreadId, Request) {
    let (mut s, mut p, thread, asset) = crate::character_animation::tests::fixture(0);
    let (_, lot) = crate::lot_runtime::tests::fixture();
    s.native_lot = lot.native_lot;
    let m = p.modules.get_mut("person").unwrap();
    m.strings.insert(154, vec!["test".into(); 11]);
    m.strings.insert(155, vec!["child-test".into(); 11]);
    let ins = &mut m.routines.get_mut(&4096).unwrap().body.instructions[0];
    ins.opcode = 47;
    ins.operands = [2, 0, u8::from(transfer), 0, 0, 0, 0, 0];
    let r = Request {
        id: 1,
        origin: s.vm.threads[&thread].stack.last().unwrap().location.clone(),
        caller: 1,
        target: 1,
        operands: ins.operands,
    };
    s.vm.threads.get_mut(&thread).unwrap().status = Status::AwaitingReach(r.clone());
    p.animations.insert("test".into(), asset);
    (s, p, thread, r)
}
fn step() -> Step {
    Step {
        stop: crate::StepStop::Waiting,
        trace: vec![],
        events: vec![],
    }
}
fn restore(s: Simulation, p: &Program) -> Simulation {
    let restored: Simulation = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
    assert_eq!(restored, s);
    restored.validate_reaches(p).unwrap();
    for (&key, e) in &restored.native_reaches {
        e.validate(&restored, p, key).unwrap();
    }
    restored
}

#[test]
fn native_reach_missing_asset_and_profile_are_saved_synchronous_services() {
    let (mut s, mut p, thread, r) = fixture(false);
    let asset = p.animations.remove("test").unwrap();
    s.characters.get_mut(&1).unwrap().profile_dirty = true;
    let before = s.characters[&1].character.skin.animator.clone();
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Budget
    );
    assert!(!s.characters[&1].profile_dirty);
    assert_eq!(s.pending_reach_profiles(), [(1, 1, false)]);
    assert_eq!(s.characters[&1].character.skin.animator, before);
    assert!(s.advance_fixture_tick().is_err());
    s = restore(s, &p);
    s.begin_reach_profile(1, 1).unwrap();
    assert!(s.begin_reach_profile(1, 1).is_err());
    s = restore(s, &p);
    s.complete_reach_profile(1, 1).unwrap();
    assert!(s.complete_reach_profile(1, 1).is_err());
    assert_eq!(s.native_reach_asset_names(&p), ["test".to_owned()].into());
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Budget
    );
    assert_eq!(s.characters[&1].character.skin.animator, before);
    s = restore(s, &p);
    p.animations.insert("test".into(), asset);
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Waiting
    );
    assert_eq!(s.characters[&1].controls.ordinary.pickup, 1);
    assert_eq!(s.vm.threads[&thread].stack.last().unwrap().continuation, 1);
    let before = s.clone();
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Waiting
    );
    assert_eq!(
        s, before,
        "same-tick polls cannot replace the foreground or consume events"
    );
    restore(s, &p);
}

#[test]
fn native_reach_height_only_waits_an_extra_tick_before_finishing_and_preserves_counters() {
    let (mut s, p, thread, r) = fixture(false);
    s.characters.get_mut(&1).unwrap().controls.engaged = 5;
    s.characters.get_mut(&1).unwrap().controls.animation_events = 9;
    s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
        .unwrap();
    // Missing foreground is an ended animator result in 58a000. This isolates
    // primitive phase ordering; real layer clocks are tested in the native corpus.
    let c = s.characters.get_mut(&1).unwrap();
    c.character
        .skin
        .animator
        .clear_foreground(0, c.controls.settings)
        .unwrap();
    s.vm.tick = 1;
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Waiting
    );
    assert_eq!(s.vm.threads[&thread].stack.last().unwrap().continuation, 2);
    s = restore(s, &p);
    s.vm.tick = 2;
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Ready
    );
    assert!(s.native_reaches.is_empty());
    assert_eq!(s.characters[&1].controls.ordinary.pickup, 0);
    assert_eq!(s.characters[&1].controls.engaged, 5);
    assert_eq!(s.characters[&1].controls.animation_events, 9);
    assert!(!matches!(
        s.vm.threads[&thread].status,
        Status::AwaitingReach(_)
    ));
}

#[test]
fn native_reach_rejects_a_lost_in_flight_continuation_without_restarting_animation() {
    let (mut s, p, thread, r) = fixture(false);
    s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
        .unwrap();
    s.native_reaches.clear();
    let before = s.clone();
    assert!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap_err()
            .contains("original in-flight reconstruction")
    );
    assert_eq!(s, before);
    assert!(s.validate_reaches(&p).is_err());
}

#[test]
fn native_reach_invalid_transfer_mode_retains_phase_one_even_when_ended() {
    let (mut s, p, thread, r) = fixture(true);
    s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
        .unwrap();
    let c = s.characters.get_mut(&1).unwrap();
    c.character
        .skin
        .animator
        .clear_foreground(0, c.controls.settings)
        .unwrap();
    for tick in 1..4 {
        s.vm.tick = tick;
        assert_eq!(
            s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
                .unwrap(),
            Outcome::Waiting
        );
        assert_eq!(s.vm.threads[&thread].stack.last().unwrap().continuation, 1);
        s = restore(s, &p);
    }
}

#[test]
fn native_reach_child_table_includes_zero_and_negative_age() {
    for age in [-1, 0, 17, 18] {
        let (mut s, mut p, thread, r) = fixture(false);
        p.animations.clear();
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(58, age);
        assert_eq!(
            s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
                .unwrap(),
            Outcome::Budget
        );
        assert_eq!(
            s.native_reach_asset_names(&p),
            [if age <= 17 {
                "child-test".to_owned()
            } else {
                "test".to_owned()
            }]
            .into()
        );
    }
}

#[test]
fn native_reach_rereads_profile_age_after_reload_but_retains_prepared_height_index() {
    let (mut s, p, thread, r) = fixture(false);
    s.characters.get_mut(&1).unwrap().profile_dirty = true;
    assert_eq!(
        s.service_native_reach(&p, thread, &r, &mut 1, &mut step())
            .unwrap(),
        Outcome::Budget
    );
    s.begin_reach_profile(1, r.id).unwrap();
    // 5888d3 writes person[58] from the profile before 59dcd4 selects its table.
    s.vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(58, 10);
    s.complete_reach_profile(1, r.id).unwrap();
    assert_eq!(
        s.native_reach_asset_names(&p),
        ["child-test".to_owned()].into()
    );
    assert!(matches!(
        s.native_reaches[&r.id].stage,
        Stage::Asset(pose::reach::Selection {
            index: 9,
            strings: 155,
            ..
        })
    ));
    restore(s, &p);
}
