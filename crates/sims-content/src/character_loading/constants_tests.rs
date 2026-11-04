//! Synchronous saved-person child boundary tests, not whole-loader save/resume.
use super::drive_saved_person_constants;
use sims_vm::{autonomy_constants::*, *};

fn incoming() -> Owner {
    serde_json::from_str(include_str!(
        "../../../sims-vm/tests/fixtures/actual-goth-process-incoming.json"
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
fn fixture(owner: Owner) -> Vm {
    let mut vm = Vm::new(
        World {
            autonomy_constants: Some(owner),
            ..Default::default()
        },
        0x12345678,
    );
    vm.tick = 123;
    vm.lot_rng_state = Some(0x12345678);
    vm.crt_rng_state = Some(0x76543210);
    vm
}
fn cold(vm: &mut Vm, call: &mut Call) {
    let program = Program::default();
    let bytes = serde_json::to_vec(&(Snapshot::capture(&program, vm), &*call)).unwrap();
    let (snapshot, continuation): (Snapshot, Call) = serde_json::from_slice(&bytes).unwrap();
    let restored = snapshot.restore(&program).unwrap();
    assert_eq!(restored, *vm);
    assert_eq!(continuation, *call);
    continuation.validate().unwrap();
    *vm = restored;
    *call = continuation;
}

#[cfg(feature = "private-fixtures")]
#[test]
fn saved_child_stops_on_real_unavailable_load_then_flush_and_preserves_partial_process() {
    let mut vm = fixture(configured());
    let before = vm.clone();
    let mut call = vm.world.autonomy_constants.as_ref().unwrap().begin();
    let error = drive_saved_person_constants(&mut vm.world, &mut call)
        .unwrap_err()
        .to_string();
    assert!(error.contains("source-bound autonomy resource service Load"));
    assert_eq!(vm, before);
    cold(&mut vm, &mut call);
    let Request::Load(source) = call.pending().unwrap().clone() else {
        panic!()
    };
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sims-vm/tests/fixtures/native-autonomy-constants.json"
    ))
    .unwrap();
    let table = Table {
        rows: serde_json::from_value(data["cases"][0]["source_table"].clone()).unwrap(),
    };
    call.reply(Reply::Loaded {
        binding: source.clone(),
        table,
    })
    .unwrap();
    let error = drive_saved_person_constants(&mut vm.world, &mut call)
        .unwrap_err()
        .to_string();
    assert!(error.contains("source-bound autonomy resource service Flush"));
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(0)
    );
    assert_eq!(
        vm.world
            .autonomy_constants
            .as_ref()
            .unwrap()
            .friendship_threshold(),
        Ok(50)
    );
    cold(&mut vm, &mut call);
    let pending = vm.clone();
    assert_eq!(
        drive_saved_person_constants(&mut vm.world, &mut call)
            .unwrap_err()
            .to_string(),
        error
    );
    assert_eq!(vm, pending);
    // Explicit supplied provider normal return; never a production no-op host.
    call.reply(Reply::Flushed {
        binding: source,
        return_value: 0,
    })
    .unwrap();
    drive_saved_person_constants(&mut vm.world, &mut call).unwrap();
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(1)
    );
    let mut expected = before;
    expected.world.autonomy_constants = vm.world.autonomy_constants.clone();
    assert_eq!(vm, expected);
    cold(&mut vm, &mut call);
}

#[test]
fn saved_loaded_nonzero_is_lazy_but_unloaded_unknown_provider_fails_without_defaulting() {
    for loaded in [1, 2, 255] {
        let mut owner = Owner::default();
        owner.process.loaded = Some(loaded);
        let mut vm = fixture(owner);
        let before = vm.clone();
        let mut call = vm.world.autonomy_constants.as_ref().unwrap().begin();
        drive_saved_person_constants(&mut vm.world, &mut call).unwrap();
        assert_eq!(vm, before);
        cold(&mut vm, &mut call);
    }
    let mut vm = fixture(incoming());
    let before = vm.clone();
    let mut call = vm.world.autonomy_constants.as_ref().unwrap().begin();
    let error = drive_saved_person_constants(&mut vm.world, &mut call)
        .unwrap_err()
        .to_string();
    assert!(error.contains("profile"));
    assert_eq!(vm, before);
    cold(&mut vm, &mut call);
}

#[test]
fn explicit_initial_input_is_once_only_and_never_repairs_retained_unknowns() {
    let owner = configured();
    let mut world = World::default();
    super::apply_initial_process(&mut world, &mut None).unwrap();
    assert!(world.autonomy_constants.is_none());
    let mut input = Some(owner.clone());
    super::apply_initial_process(&mut world, &mut input).unwrap();
    assert!(input.is_none());
    assert_eq!(world.autonomy_constants, Some(owner.clone()));
    world.autonomy_constants.as_mut().unwrap().process.loaded = Some(2);
    let retained = world.clone();
    super::apply_initial_process(&mut world, &mut input).unwrap();
    assert_eq!(world, retained);
    let mut conflicting = Some(owner);
    let original = conflicting.clone();
    assert!(
        super::apply_initial_process(&mut world, &mut conflicting)
            .unwrap_err()
            .to_string()
            .contains("cannot overwrite")
    );
    assert_eq!(world, retained);
    assert_eq!(conflicting, original);
    world.autonomy_constants = Some(Default::default());
    let old = world.clone();
    assert!(super::apply_initial_process(&mut world, &mut conflicting).is_err());
    assert_eq!(world, old);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn shared_service_driver_preserves_process_only_mutations_and_suspend_order() {
    let mut vm = fixture(configured());
    let before = vm.clone();
    let mut call = vm.world.autonomy_constants.as_ref().unwrap().begin();
    let data: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sims-vm/tests/fixtures/native-autonomy-constants.json"
    ))
    .unwrap();
    let mut requests = vec![];
    let result = super::drive_saved_person_constants_with_service(
        &mut vm.world,
        &mut call,
        |request, table| {
            requests.push(request.clone());
            match request {
                Request::Load(binding) => {
                    assert!(table.is_none());
                    Ok(Reply::Loaded {
                        binding: binding.clone(),
                        table: Table {
                            rows: serde_json::from_value(data["cases"][0]["source_table"].clone())
                                .unwrap(),
                        },
                    })
                }
                Request::Flush(_) => {
                    assert!(table.is_some());
                    anyhow::bail!("explicit service boundary")
                }
            }
        },
    );
    assert_eq!(result.unwrap_err().to_string(), "explicit service boundary");
    assert!(matches!(
        &requests[..],
        [Request::Load(_), Request::Flush(_)]
    ));
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(0)
    );
    assert_eq!(
        vm.world
            .autonomy_constants
            .as_ref()
            .unwrap()
            .friendship_threshold(),
        Ok(50)
    );
    let mut expected = before;
    expected.world.autonomy_constants = vm.world.autonomy_constants.clone();
    assert_eq!(vm, expected); // clock, both RNGs, scheduler and every other owner
    cold(&mut vm, &mut call);
    let pending = call.pending().unwrap().clone();
    let mut resumed = 0;
    super::drive_saved_person_constants_with_service(&mut vm.world, &mut call, |request, _| {
        assert_eq!(request, &pending);
        resumed += 1;
        let Request::Flush(binding) = request else {
            panic!()
        };
        Ok(Reply::Flushed {
            binding: binding.clone(),
            return_value: -99,
        })
    })
    .unwrap();
    assert_eq!(resumed, 1);
    assert_eq!(
        vm.world.autonomy_constants.as_ref().unwrap().process.loaded,
        Some(1)
    );
    expected.world.autonomy_constants = vm.world.autonomy_constants.clone();
    assert_eq!(vm, expected);
    cold(&mut vm, &mut call);
    super::drive_saved_person_constants_with_service(&mut vm.world, &mut call, |_, _| {
        panic!("loaded gate serviced a resource")
    })
    .unwrap();
    assert_eq!(vm, expected);
}
