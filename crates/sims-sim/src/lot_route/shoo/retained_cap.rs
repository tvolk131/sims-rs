//! Authored continuation ownership guards, not imported native threads.
use super::super::live::{Execution as LiveQuery, Potential};
use super::*;
use sims_vm::RouteRequest;
fn route() -> sims_vm::RouteRequest {
    let descriptor = sims_vm::routing::Descriptor([
        0,
        0,
        0,
        1,
        1,
        0,
        0,
        0x801,
        u32::MAX,
        16,
        16,
        16,
        0.1875_f32.to_bits(),
        (-3_i32) as u32,
        16,
    ]);
    sims_vm::RouteRequest {
        id: 1,
        origin: sims_vm::Location {
            source: "authored-cap-owner".into(),
            bhav: 0,
            instruction: 0,
        },
        caller: 2,
        target: 3,
        destination: sims_vm::routing::Destination::Slot {
            source: "authored-cap-owner".into(),
            chunk: 0,
            index: 0,
            version: 10,
            slot: descriptor.slot(),
        },
        no_failure_trees: false,
    }
}
fn owner(limit: Option<usize>) -> Execution {
    Execution {
        thread: 0,
        route: route(),
        selection: Selection::new(vec![3]),
        ignored_word: 0,
        blocked_object: 0,
        retained_result: 0,
        candidate_limit: limit,
        query: None,
        attempt: None,
    }
}
#[test]
fn absent_or_invalid_historical_cap_is_not_a_fresh_constructor() {
    for limit in [None, Some(0), Some(7), Some(9), Some(47), Some(49)] {
        let (p, mut s) = crate::lot_runtime::tests::fixture();
        let mut old = serde_json::to_value(owner(limit)).unwrap();
        if limit.is_none() {
            old.as_object_mut().unwrap().remove("candidate_limit");
        }
        let restored: Execution = serde_json::from_value(old).unwrap();
        assert_eq!(restored.candidate_limit, limit);
        let error = if limit.is_none() {
            "shoo retained candidate limit absent"
        } else {
            "invalid shoo retained candidate limit"
        };
        assert_eq!(restored.validate(&s, &p).unwrap_err(), error);
        s.native_lot.as_mut().unwrap().shoo = Some(restored);
        let before = s.clone();
        let mut remaining = 1;
        let mut step = crate::Step {
            stop: crate::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        assert_eq!(
            s.drive_native_shoo(&p, &mut remaining, &mut step)
                .unwrap_err(),
            error
        );
        assert_eq!(s, before);
        assert_eq!(remaining, 1);
        assert!(step.trace.is_empty() && step.events.is_empty());
    }
}
#[test]
fn retained_query_cap_cold_binding_rejects_mismatched_owners() {
    let (p, mut s) = crate::lot_runtime::tests::fixture();
    s.vm.world.objects.get_mut(&2).unwrap().person = Some((0..100).map(|i| (i, 0)).collect());
    let mut e = owner(Some(8));
    let q = super::super::live::Execution::begin_retained_potential(
        &mut s,
        &p,
        0,
        &e.route,
        super::super::live::Potential {
            ignored_person: None,
            ignore_people: false,
            ignored_word: 0,
            blocked_object: 0,
            retained_result: 0,
        },
        8,
    )
    .unwrap();
    e.query = Some(q);
    assert_eq!(e.retained_candidate_limit().unwrap(), 8);
    let restored: Execution = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
    assert_eq!(restored, e);
    assert_eq!(restored.retained_candidate_limit().unwrap(), 8);
    let mut forged = restored;
    forged.candidate_limit = Some(48);
    assert_eq!(
        forged.retained_candidate_limit().unwrap_err(),
        "shoo temporary candidate limit mismatch"
    );
    for limit in [8, 48] {
        let e = owner(Some(limit));
        let e: Execution = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
        assert_eq!(e.retained_candidate_limit().unwrap(), limit);
    }
}

#[test]
fn retained_cap_large_then_small_room_preparation() {
    //Authored room-getter transition. The current shoo caller constructs a new
    //query on each Path; it currently loses the earlier native context cap.
    let (p, mut s) = crate::lot_runtime::tests::fixture();
    s.vm.world.objects.get_mut(&2).unwrap().person = Some((0..100).map(|i| (i, 0)).collect());
    let descriptor = sims_vm::routing::Descriptor([
        0,
        0,
        0,
        1,
        1,
        0,
        0,
        0x801,
        u32::MAX,
        16,
        16,
        16,
        0.1875_f32.to_bits(),
        (-3_i32) as u32,
        16,
    ]);
    let r = RouteRequest {
        id: 1,
        origin: sims_vm::Location {
            source: "authored-cap-transition".into(),
            bhav: 0,
            instruction: 0,
        },
        caller: 2,
        target: 3,
        destination: sims_vm::routing::Destination::Slot {
            source: "authored-cap-transition".into(),
            chunk: 0,
            index: 0,
            version: 10,
            slot: descriptor.slot(),
        },
        no_failure_trees: false,
    };
    let settings = Potential {
        ignored_person: None,
        ignore_people: false,
        ignored_word: 0,
        blocked_object: 0,
        retained_result: 0,
    };
    s.vm.world.room_half_tiles.insert(0, 1000);
    let first = LiveQuery::begin_potential(&mut s, &p, 0, &r, settings.clone()).unwrap();
    assert_eq!(first.potential_candidate_limit(), 8);
    s.vm.world.room_half_tiles.insert(0, 100);
    //This is the current drive_native_shoo reentry behavior, before the private
    //retained-owner correction. The expected8 comes from native unwritten+98.
    let second = LiveQuery::begin_retained_potential(
        &mut s,
        &p,
        0,
        &r,
        settings.clone(),
        first.potential_candidate_limit(),
    )
    .unwrap();
    let fresh_small = LiveQuery::begin_potential(&mut s, &p, 0, &r, settings.clone()).unwrap();
    assert_eq!(fresh_small.potential_candidate_limit(), 48);
    for invalid in [0, 7, 9, 47, 49, usize::MAX] {
        let before = s.clone();
        assert_eq!(
            LiveQuery::begin_retained_potential(&mut s, &p, 0, &r, settings.clone(), invalid)
                .unwrap_err(),
            "invalid retained potential candidate limit"
        );
        assert_eq!(s, before);
    }
    assert_eq!(
        second.potential_candidate_limit(),
        8,
        "same native context must retain8 rather than reset48"
    );
}

#[test]
fn empty_eligibility_does_not_construct_a_temporary_cap_owner() {
    let (_, mut s) = crate::lot_runtime::tests::fixture();
    s.vm.world.objects.get_mut(&2).unwrap().person = Some((0..100).map(|i| (i, 0)).collect());
    s.vm.world.person.as_mut().unwrap().person_order = vec![2];
    let before = s.clone();
    assert!(!s.start_original_shoo(&route()).unwrap());
    assert!(s.native_lot.as_ref().unwrap().shoo.is_none());
    assert_eq!(s, before);
}
