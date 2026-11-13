#[cfg(feature = "private-fixtures")]
#[test]
fn notification_route_references_match_native_and_preserve_other_contexts() {
    use super::*;
    let report: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/reset-route-references.json"
    ))
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 40);
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let (p, mut s) = crate::lot_runtime::tests::fixture();
        if !s.vm.object_threads.contains_key(&4) {
            s.vm.start_object_lifecycle(&p, 4).unwrap();
        }
        let thread = s.vm.object_threads[&4];
        s.vm.world.objects.get_mut(&4).unwrap().person = Some((0..256).map(|i| (i, 0)).collect());
        s.vm.world.objects.get_mut(&4).unwrap().native_flags =
            Some(c["flags"].as_u64().unwrap() as u32);
        if c["kind"] == 2 {
            s.vm.world.objects.get_mut(&2).unwrap().person = Some(Default::default());
        }
        s.vm.world.objects.get_mut(&2).unwrap().native_flags = Some(0x40);
        s.vm.world.person.as_mut().unwrap().person_order = vec![4];
        s.vm.world.actions = Some(sims_vm::action::WorldState {
            actors: [(4, Default::default())].into(),
            ..Default::default()
        });
        let context = |caller, target, success, serial| crate::contention::State {
            caller,
            thread,
            controller: crate::contention::Controller::new(crate::Point::new(23, 45)),
            link: (target != 0).then_some(crate::contention::Link {
                blocker: target,
                object: 1,
                action: serial,
            }),
            blocker_success: success,
            callback: None,
            deciding: false,
            shoo_result: None,
            last_portal: Some(1),
            original: None,
        };
        for (i, r) in c["routes"].as_array().unwrap().iter().enumerate() {
            s.route_control.insert(
                i as u64,
                context(
                    4,
                    r[0].as_i64().unwrap() as i16,
                    r[1] == 1,
                    r[2].as_u64().unwrap() as u32,
                ),
            );
        }
        s.route_control.insert(100, context(3, 2, false, 17));
        let before = s.clone();
        let mut e = Execution::broadcast(2);
        let mut step = Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        assert!(
            s.drive_person_reset(&p, &mut e, &mut 100, &mut step)
                .unwrap()
        );
        e.validate(&s, &p).unwrap();
        for (i, expected) in row["original"]["routes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let state = &s.route_control[&(i as u64)];
            assert_eq!(
                state.link.as_ref().map_or(0, |l| l.blocker),
                expected[0].as_i64().unwrap() as i16
            );
            assert_eq!(state.blocker_success, expected[1] == 1);
            if let Some(link) = &state.link {
                assert_eq!(link.action, expected[2].as_u64().unwrap() as u32);
            }
        }
        let mut expected = before;
        if c["kind"] == 2 && c["flags"].as_u64().unwrap() & 0x800 == 0 {
            for state in expected
                .route_control
                .values_mut()
                .filter(|s| s.caller == 4)
            {
                if state.link.as_ref().is_some_and(|l| l.blocker == 2) {
                    state.link = None;
                    state.blocker_success = true;
                }
            }
        }
        assert_eq!(s, expected, "{c}");
        assert!(step.trace.is_empty() && step.events.is_empty());
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn route_context_clear_matches_native_scope_and_preserves_paths_caches_and_other_people() {
    let report: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/reset-route-clear.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let (_, mut s) = crate::lot_runtime::tests::fixture();
        let count = row["capacities"].as_array().unwrap().len();
        let thread = s.vm.object_threads[&4];
        let context = |caller| crate::contention::State {
            caller,
            thread,
            controller: crate::contention::Controller::new(crate::Point::new(23, 45)),
            link: Some(crate::contention::Link {
                blocker: 3,
                object: 2,
                action: 99,
            }),
            blocker_success: true,
            callback: None,
            deciding: false,
            shoo_result: None,
            last_portal: Some(2),
            original: None,
        };
        for i in 0..count {
            s.route_control.insert(i as u64, context(4));
        }
        // A different person keeps its context, including any shoo linkage.
        s.route_control.insert(100, context(3));
        s.native_lot
            .as_mut()
            .unwrap()
            .route_paths
            .insert(4, vec![[40, 56], [24, 56]]);
        s.portal_cache.entry(4).or_default().costs.insert(2, 1001.);
        let before = s.clone();
        s.clear_reset_route_contexts(4).unwrap();
        assert_eq!(row["original"]["remaining"], 0);
        assert_eq!(row["original"]["retained_person_storage"], true);
        assert_eq!(s.route_control.len(), 1);
        let mut expected = before;
        expected.route_control.retain(|_, c| c.caller != 4);
        assert_eq!(s, expected);
    }
}
