//! Supplied animator/score/resource children; this is constructor composition
//! and shared-owner proof, not execution of the full original person factory.
use super::*;
use crate::{autonomy_constants::*, *};

fn incoming() -> Owner {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/actual-goth-process-incoming.json"
    ))
    .unwrap()
}
fn configured() -> Owner {
    let mut owner = incoming();
    owner.configuration = Configuration {
        profile: Some(Profile::CompleteCollection191),
        source: Some(Binding {
            module: "Global.iff".into(),
            module_sha256: "bdfd8456abceaa3e44f0a9754815248ea1e80c7b0df4fbaff53843fae3731478"
                .into(),
            resource_id: 2,
            resource_sha256: "fd3a44e2068aa43ba01d17d79bafb2cd8882beb44a2f53376d34f6172c637a2e"
                .into(),
        }),
        // Actual control observed at a fresh unloaded gate; provider remains supplied.
        fp_control: Some(0x3f),
    };
    owner
}
fn table() -> Table {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/native-autonomy-constants.json"
    ))
    .unwrap();
    Table {
        rows: serde_json::from_value(fixture["cases"][0]["source_table"].clone()).unwrap(),
    }
}
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
fn actual_incoming_owner_stays_single_unknowns_lazy_and_legacy_absence_roundtrips() {
    let owner = incoming();
    owner.validate().unwrap();
    assert_eq!(owner.process.loaded, Some(0));
    assert!(owner.process.words.iter().all(Option::is_some));
    assert_eq!(owner.friendship_threshold(), Ok(25));
    let p = Program::default();
    let mut vm = Vm::new(
        World {
            autonomy_constants: Some(owner.clone()),
            ..Default::default()
        },
        77,
    );
    let original = serde_json::to_vec(&vm).unwrap();
    assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
    assert_eq!(serde_json::to_vec(&vm).unwrap(), original);
    vm.world.person = Some(Default::default());
    vm.world.person = None;
    assert_eq!(vm.world.autonomy_constants, Some(owner));
    let old = Vm::new(World::default(), 77);
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &old)).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("autonomy_constants"));
    let restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert!(
        restored
            .restore(&p)
            .unwrap()
            .world
            .autonomy_constants
            .is_none()
    );
}

#[test]
fn dynamic_child_precedes_attachment_typed_replies_and_every_service_cold() {
    use registered::Request;
    let (p, mut vm, mut factory) = fixture(Some(configured()));
    scores(&p, &mut vm, &mut factory);
    let initial_person = vm.world.objects[&1].person.clone();
    let streams = (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state);
    assert_eq!(
        factory.next(&mut vm, &p).unwrap(),
        Some(Request::Post(PostConstructRequest::AutonomyConstants))
    );
    assert_eq!(
        vm.world
            .autonomy_constants
            .as_ref()
            .unwrap()
            .friendship_threshold(),
        Ok(25)
    );
    assert_eq!(vm.world.objects[&1].person, initial_person);
    cold(&p, &mut vm, &mut factory);
    let request = factory.autonomy_constants_request().unwrap().clone();
    let Request::Post(marker) = factory.pending_request().unwrap() else {
        unreachable!()
    };
    let before = factory.clone();
    assert!(
        factory
            .complete(&mut vm, &p, Request::Post(marker), None)
            .unwrap_err()
            .contains("typed resource reply")
    );
    assert_eq!(factory, before);
    let crate::autonomy_constants::Request::Load(binding) = request else {
        panic!("load required")
    };
    factory
        .reply_autonomy_constants(Reply::Loaded {
            binding,
            table: table(),
        })
        .unwrap();
    cold(&p, &mut vm, &mut factory);
    assert_eq!(
        factory.next(&mut vm, &p).unwrap(),
        Some(Request::Post(PostConstructRequest::AutonomyConstants))
    );
    assert_eq!(
        vm.world
            .autonomy_constants
            .as_ref()
            .unwrap()
            .friendship_threshold(),
        Ok(50)
    );
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(0)
    );
    assert_eq!(vm.world.objects[&1].person, initial_person);
    cold(&p, &mut vm, &mut factory);
    let crate::autonomy_constants::Request::Flush(binding) =
        factory.autonomy_constants_request().unwrap().clone()
    else {
        panic!("flush required")
    };
    factory
        .reply_autonomy_constants(Reply::Flushed {
            binding,
            return_value: 0,
        })
        .unwrap();
    cold(&p, &mut vm, &mut factory);
    assert_eq!(
        factory.next(&mut vm, &p).unwrap(),
        Some(Request::Post(PostConstructRequest::RefreshResourceProfile))
    );
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(1)
    );
    assert_ne!(
        vm.world.objects[&1].person, initial_person,
        "neighbor attachment must follow the completed producer"
    );
    assert_eq!(
        (vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state),
        streams
    );
}

#[test]
fn dynamic_loaded_nonzero_skips_unknown_provider_and_unloaded_unknown_control_preserves_partial_words()
 {
    use registered::Request;
    for loaded in [1, 2, 255] {
        let mut owner = Owner::default();
        owner.process.loaded = Some(loaded);
        owner.process.words[Field::FriendshipThreshold.index()] = Some((-37i32) as u32);
        let (p, mut vm, mut f) = fixture(Some(owner.clone()));
        scores(&p, &mut vm, &mut f);
        assert_eq!(
            f.next(&mut vm, &p).unwrap(),
            Some(Request::Post(PostConstructRequest::RefreshResourceProfile))
        );
        assert_eq!(vm.world.autonomy_constants, Some(owner));
        assert!(f.autonomy_constants_request().is_none());
    }
    let mut owner = configured();
    owner.configuration.fp_control = None;
    let (p, mut vm, mut f) = fixture(Some(owner));
    scores(&p, &mut vm, &mut f);
    f.next(&mut vm, &p).unwrap();
    let crate::autonomy_constants::Request::Load(binding) =
        f.autonomy_constants_request().unwrap().clone()
    else {
        unreachable!()
    };
    f.reply_autonomy_constants(Reply::Loaded {
        binding,
        table: table(),
    })
    .unwrap();
    assert_eq!(
        f.next(&mut vm, &p).unwrap_err(),
        "autonomy constants x87 control is unknown"
    );
    let owner = vm.world.autonomy_constants.as_ref().unwrap();
    assert_eq!(owner.process.loaded, Some(0));
    assert_eq!(owner.friendship_threshold(), Ok(25));
    let state = (vm.clone(), f.clone());
    cold(&p, &mut vm, &mut f);
    assert_eq!(
        f.next(&mut vm, &p).unwrap_err(),
        "autonomy constants x87 control is unknown"
    );
    assert_eq!((vm, f), state);
    let mut owner = configured();
    owner.configuration.fp_control = Some(0x037e);
    let (p, mut vm, mut f) = fixture(Some(owner));
    scores(&p, &mut vm, &mut f);
    f.next(&mut vm, &p).unwrap();
    let crate::autonomy_constants::Request::Load(binding) =
        f.autonomy_constants_request().unwrap().clone()
    else {
        unreachable!()
    };
    f.reply_autonomy_constants(Reply::Loaded {
        binding,
        table: table(),
    })
    .unwrap();
    assert_eq!(
        f.next(&mut vm, &p).unwrap_err(),
        "autonomy constants x87 control is unverified"
    );
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(0)
    );
    cold(&p, &mut vm, &mut f);
}

#[test]
fn forged_constructor_child_owner_pending_and_wrong_source_are_rejected() {
    let (p, mut vm, mut f) = fixture(Some(configured()));
    scores(&p, &mut vm, &mut f);
    f.next(&mut vm, &p).unwrap();
    let before = f.clone();
    let mut binding = configured().configuration.source.unwrap();
    binding.resource_sha256 = "0".repeat(64);
    assert!(
        f.reply_autonomy_constants(Reply::Loaded {
            binding,
            table: table()
        })
        .is_err()
    );
    assert_eq!(f, before);
    let bytes = serde_json::to_value(&f).unwrap();
    for (label, mut forged) in [("stage", bytes.clone()), ("pending", bytes.clone())] {
        let post = &mut forged["stage"]["Post"];
        if label == "stage" {
            post["stage"] = serde_json::json!(5);
        } else {
            post["constants"] = serde_json::Value::Null;
        }
        let forged: registered::Factory = serde_json::from_value(forged).unwrap();
        assert!(forged.validate(&vm, &p).is_err());
    }
    // Both nested and outer markers can be forged consistently. Stage9 may
    // still own only its typed resource marker, never a generic Base reply.
    let mut unrelated = bytes.clone();
    unrelated["pending"] = serde_json::json!({"Post":"Base"});
    unrelated["stage"]["Post"]["pending"] = serde_json::json!("Base");
    unrelated["stage"]["Post"]["constants"] = serde_json::Value::Null;
    let unrelated: registered::Factory = serde_json::from_value(unrelated).unwrap();
    assert!(unrelated.validate(&vm, &p).is_err());
    vm.world.autonomy_constants = None;
    assert!(
        f.validate(&vm, &p)
            .unwrap_err()
            .contains("lost process owner")
    );
}
