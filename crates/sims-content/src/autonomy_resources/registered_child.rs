//! Supplied animator/scores boundaries with the real installed FCNS provider.
//! This is not execution of full original animation/graphics construction.
use super::*;
use sims_vm::{autonomy_constants::*, person_factory::*, *};
fn fixture(owner: Option<Owner>) -> (Program, Vm, registered::Factory) {
    let guid = 0x44a86a6c;
    let mut words = vec![0; 108];
    words[9] = 2;
    words[14] = guid as u16 as i16;
    words[15] = (guid >> 16) as i16;
    let p = Program {
        globals: "person".into(),
        modules: [(
            "person".into(),
            Module {
                source: "person".into(),
                object_definition_words: [(128, words)].into(),
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    let mut vm = Vm::new(
        World {
            autonomy_constants: owner,
            person: Some(person::WorldState {
                neighbor_slots: Some(vec![None, None, None]),
                ..Default::default()
            }),
            actions: Some(Default::default()),
            refresh: Some(Default::default()),
            ..Default::default()
        },
        0x10203040,
    );
    vm.lot_rng_state = Some(0x11223344);
    vm.crt_rng_state = Some(0x55667788);
    let factory = registered::Factory::begin(
        &mut vm,
        &p,
        Definition {
            source: "person".into(),
            object: 128,
            guid,
            slots: 0,
        },
        construction::ObjectDate {
            year: 1997,
            month: 1,
            day: 1,
        },
        [0, 0],
    )
    .unwrap()
    .unwrap();
    (p, vm, factory)
}
fn scores(p: &Program, vm: &mut Vm, factory: &mut registered::Factory) {
    use registered::Request;
    for expected in [
        Request::Registered(1),
        Request::Post(PostConstructRequest::Base),
        Request::Post(PostConstructRequest::BindAnimator),
        Request::Post(PostConstructRequest::InitialScores),
    ] {
        assert_eq!(factory.next(vm, p).unwrap(), Some(expected));
        if expected == Request::Post(PostConstructRequest::Base) {
            vm.prepare_person_base_postconstruction(p, 1).unwrap();
        }
        // Animator and scores are explicit supplied children in this fixture.
        factory.complete(vm, p, expected, None).unwrap();
    }
}
fn cold(p: &Program, vm: &mut Vm, factory: &mut registered::Factory) {
    let bytes = serde_json::to_vec(&(Snapshot::capture(p, vm), &*factory)).unwrap();
    let (snapshot, restored): (Snapshot, registered::Factory) =
        serde_json::from_slice(&bytes).unwrap();
    let world = snapshot
        .restore_with_direct_person_factories(p, &[], &[&restored])
        .unwrap();
    restored.validate(&world, p).unwrap();
    assert_eq!(world, *vm);
    assert_eq!(restored, *factory);
    *vm = world;
    *factory = restored;
}

#[test]
#[ignore = "private immutable Global.far; registered constructor/actual provider cold composition"]
fn actual_provider_is_retained_with_each_pending_registered_child_service() {
    let index = Index::scan(&std::path::PathBuf::from(
        std::env::var_os("SIMS_FCNS_ASSETS").unwrap(),
    ))
    .unwrap();
    let input = InitialProfile::CompleteCollection191ObservedInitial20261024
        .prepare(&index)
        .unwrap();
    let mut provider = input.provider;
    let (p, mut vm, mut factory) = fixture(Some(input.owner));
    scores(&p, &mut vm, &mut factory);
    let streams = (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state);
    for flush in [false, true] {
        assert_eq!(
            factory.next(&mut vm, &p).unwrap(),
            Some(registered::Request::Post(
                PostConstructRequest::AutonomyConstants
            ))
        );
        cold(&p, &mut vm, &mut factory);
        provider = serde_json::from_slice(&serde_json::to_vec(&provider).unwrap()).unwrap();
        provider.validate(&index).unwrap();
        let request = factory.autonomy_constants_request().unwrap().clone();
        assert_eq!(matches!(request, Request::Flush(_)), flush);
        let reply = provider
            .service(&request, factory.autonomy_constants_table())
            .unwrap();
        if flush {
            assert!(matches!(
                reply,
                Reply::Flushed {
                    return_value: -99,
                    ..
                }
            ));
        }
        factory.reply_autonomy_constants(reply).unwrap();
        cold(&p, &mut vm, &mut factory);
        provider = serde_json::from_slice(&serde_json::to_vec(&provider).unwrap()).unwrap();
        provider.validate(&index).unwrap();
        assert_eq!(
            streams,
            (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state)
        );
    }
    assert_eq!(
        factory.next(&mut vm, &p).unwrap(),
        Some(registered::Request::Post(
            PostConstructRequest::RefreshResourceProfile
        ))
    );
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(1)
    );
    assert_eq!(
        vm.world
            .autonomy_constants
            .as_ref()
            .unwrap()
            .friendship_threshold(),
        Ok(50)
    );
    assert_eq!(provider.events.len(), 4);
    assert_eq!(provider.events.last().unwrap().return_value, -99);
}
