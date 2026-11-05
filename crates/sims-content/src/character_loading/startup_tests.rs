use super::*;
use crate::{skeleton_resources::Cache, startup_memo::Memo, startup_memo::tests::Fixture};
use sims_sim::pose::appearance::{Execution, Input};
use std::{fs, sync::Arc};

const SKELETON: &str = "GameData_ALL_Recursive/Skins/adult.cmx.bcf";
pub(super) const SETTINGS: Settings = Settings {
    fade_enabled: true,
    fade_duration: 100,
    sampling_config: 1,
};

fn skeleton(amplitude: f32) -> Vec<u8> {
    let mut bytes = 1i32.to_le_bytes().to_vec();
    bytes.push(5);
    bytes.extend(b"adult");
    bytes.extend(1u32.to_le_bytes());
    for name in [b"ROOT".as_slice(), b"NULL"] {
        bytes.push(name.len() as u8);
        bytes.extend(name);
    }
    bytes.extend(0u32.to_le_bytes());
    for value in [0f32, 0., 0., 0., 0., 0., 1.] {
        bytes.extend(value.to_le_bytes());
    }
    for flag in [0u32; 3] {
        bytes.extend(flag.to_le_bytes());
    }
    bytes.extend(2i32.to_le_bytes());
    bytes.extend(amplitude.to_le_bytes());
    bytes.extend(0i32.to_le_bytes());
    bytes.extend(0i32.to_le_bytes());
    bytes
}

pub(super) fn fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.write(SKELETON, &skeleton(0.125));
    for (relative, names) in [
        (
            "GameData_ALL_Recursive/Skins/startup.cmx.bcf",
            vec!["eager"],
        ),
        (
            "GameData_ALL_Recursive/Animation/lazy.cmx.bcf",
            vec!["first", "neighbor", "extra"],
        ),
    ] {
        let mut bytes = 0i32.to_le_bytes().to_vec();
        bytes.extend((names.len() as i32).to_le_bytes());
        for name in names {
            bytes.push(name.len() as u8);
            bytes.extend(name.as_bytes());
            bytes.extend([0; 12]);
        }
        bytes.extend(0i32.to_le_bytes());
        fixture.write(relative, &bytes);
    }
    fixture.index = Arc::new(Index::scan(&fixture.root).unwrap());
    fixture
}

pub(super) fn person() -> (Program, Vm) {
    let mut words = vec![0; 104];
    words[9] = 2;
    words[14] = 17;
    words[19] = 200;
    let mut profile = vec![String::new(); 35];
    for (entry, value) in [(0, "adult"), (12, "male"), (13, "21"), (14, "lgt")] {
        profile[entry] = value.into();
    }
    let mut program = Program::default();
    program.modules.insert(
        "person".into(),
        sims_vm::Module {
            source: "person".into(),
            semiglobal: Some("semi".into()),
            object_definition_words: [(1, words)].into(),
            strings: [(200, profile)].into(),
            ..Default::default()
        },
    );
    program.modules.insert(
        "semi".into(),
        sims_vm::Module {
            source: "semi".into(),
            strings: [(8192, vec!["".into(), "lgt".into()])].into(),
            ..Default::default()
        },
    );
    let definition = sims_vm::Definition {
        source: "person".into(),
        object: 1,
        guid: 17,
        slots: 0,
    };
    let object = Object::construct_person(definition, &program, 0).unwrap();
    let mut vm = Vm::new(
        World {
            objects: [(11, object)].into(),
            ..Default::default()
        },
        0x12345678,
    );
    vm.tick = 123;
    vm.crt_rng_state = Some(0x76543210);
    vm.lot_rng_state = Some(0x31415926);
    (program, vm)
}

fn begin(
    index: &Index,
    program: &Program,
    vm: &mut Vm,
    cache: &mut Cache,
    startup: Option<&mut Memo>,
) -> Result<Fresh> {
    let mut crt = vm.crt_rng_state.unwrap();
    let result = Fresh::begin(index, vm, program, 11, SETTINGS, &mut crt, cache, startup);
    vm.crt_rng_state = Some(crt);
    result
}

fn assert_fresh(expected: &Fresh, actual: &Fresh) {
    assert_eq!(expected.snapshot(), actual.snapshot());
    assert_eq!(
        format!("{:?}", expected.avatar.avatar),
        format!("{:?}", actual.avatar.avatar),
    );
    assert_eq!(expected.avatar.part_ranges, actual.avatar.part_ranges);
}

fn assert_outcome(expected: Result<Fresh>, actual: Result<Fresh>) {
    match (expected, actual) {
        (Ok(expected), Ok(actual)) => assert_fresh(&expected, &actual),
        (Err(expected), Err(actual)) => {
            assert_eq!(format!("{expected:#}"), format!("{actual:#}"));
        }
        _ => panic!("ordinary and memoized fresh service outcomes differ"),
    }
}

#[test]
fn startup_live_cold_warm_and_restored_resources_preserve_vm_crt_and_fresh_state() {
    let fixture = fixture();
    let (program, original) = person();
    let fingerprint = program.fingerprint();
    let mut memo = Memo::new(fixture.index.clone());
    let mut expected_cache = Cache::default();
    let mut actual_cache = Cache::default();
    let mut checkpoint = original.crt_rng_state;
    for iteration in 0..4 {
        let mut expected_vm = original.clone();
        expected_vm.crt_rng_state = checkpoint;
        let mut actual_vm = expected_vm.clone();
        let expected = begin(
            &fixture.index,
            &program,
            &mut expected_vm,
            &mut expected_cache,
            None,
        );
        let actual = begin(
            &fixture.index,
            &program,
            &mut actual_vm,
            &mut actual_cache,
            Some(&mut memo),
        );
        let expected = expected.unwrap();
        let actual = actual.unwrap();
        assert_fresh(&expected, &actual);
        assert_eq!(expected_vm, actual_vm);
        assert_eq!(expected_cache, actual_cache);
        assert_ne!(expected_cache, Cache::default());
        expected_cache.validate(&fixture.index).unwrap();
        assert_eq!(program.fingerprint(), fingerprint);
        checkpoint = expected_vm.crt_rng_state;
        if iteration == 1 {
            actual_cache =
                serde_json::from_slice(&serde_json::to_vec(&actual_cache).unwrap()).unwrap();
            actual_cache.validate(&fixture.index).unwrap();
        }
    }
}

#[test]
fn startup_live_warm_selection_preserves_source_changes_and_error_side_effects() {
    for mutation in 0..5 {
        let fixture = fixture();
        let (program, original) = person();
        let mut memo = Memo::new(fixture.index.clone());
        let mut cache = Cache::default();
        begin(
            &fixture.index,
            &program,
            &mut original.clone(),
            &mut cache,
            Some(&mut memo),
        )
        .unwrap();
        match mutation {
            0 => fixture.write(SKELETON, &skeleton(0.25)),
            1 => fixture.write(SKELETON, &[0; 2]),
            2 => fs::remove_file(fixture.root.join(SKELETON)).unwrap(),
            3 => fixture.write("GameData_ALL_Recursive/Skins/startup.cmx.bcf", &[0; 2]),
            4 => {
                let relative = "GameData_ALL_Recursive/Skins/startup.cmx.bcf";
                let mut bytes = fs::read(fixture.root.join(relative)).unwrap();
                bytes[9..14].copy_from_slice(b"fresh");
                fixture.write(relative, &bytes);
            }
            _ => unreachable!(),
        }
        let mut expected_vm = original.clone();
        let mut actual_vm = original.clone();
        let mut expected_cache = cache.clone();
        let mut actual_cache = cache.clone();
        let expected = begin(
            &fixture.index,
            &program,
            &mut expected_vm,
            &mut expected_cache,
            None,
        );
        let actual = begin(
            &fixture.index,
            &program,
            &mut actual_vm,
            &mut actual_cache,
            Some(&mut memo),
        );
        assert_eq!(expected.is_err(), mutation < 4, "mutation {mutation}");
        assert_outcome(expected, actual);
        assert_eq!(expected_vm, actual_vm);
        assert_eq!(expected_cache, actual_cache);
    }
}

#[test]
fn startup_live_restore_keeps_binding_holes_auxiliary_order_provenance_and_future_reads() {
    let fixture = fixture();
    let (program, mut vm) = person();
    let mut cache = Cache::default();
    let mut fresh = begin(&fixture.index, &program, &mut vm, &mut cache, None).unwrap();
    let record = fresh.avatar.records.body[0];
    for name in ["eager", "first"] {
        fresh
            .avatar
            .apply_record(
                &fixture.index,
                record,
                Execution::bind(Input::Replace {
                    name: name.into(),
                    texture: "".into(),
                }),
            )
            .unwrap();
    }
    fresh
        .avatar
        .bind_auxiliary_with_geometry(&fixture.index, "neighbor", 0x12345678, |_| Ok(()))
        .unwrap();
    let saved = fresh.snapshot();
    let mut memo = Memo::new(fixture.index.clone());
    for _ in 0..3 {
        let mut expected = Fresh::restore(&fixture.index, saved.clone(), None).unwrap();
        let mut actual = Fresh::restore(&fixture.index, saved.clone(), Some(&mut memo)).unwrap();
        assert_fresh(&expected, &actual);
        assert_eq!(expected.snapshot(), saved);
        let binding = expected.avatar.snapshot();
        assert!(binding.next_binding > binding.order.len() as u64 + 1);
        assert!(expected.avatar.avatar.sources.len() >= 2);
        let record = expected.avatar.records.head;
        let request = Execution::bind(Input::Replace {
            name: "eager".into(),
            texture: "".into(),
        });
        assert_eq!(
            expected
                .avatar
                .apply_record(&fixture.index, record, request.clone())
                .unwrap(),
            actual
                .avatar
                .apply_record(&fixture.index, record, request)
                .unwrap(),
        );
        assert_fresh(&expected, &actual);
    }
    fixture.write("GameData_ALL_Recursive/Animation/lazy.cmx.bcf", &[0; 2]);
    let expected = Fresh::restore(&fixture.index, saved.clone(), None);
    let actual = Fresh::restore(&fixture.index, saved.clone(), Some(&mut memo));
    assert!(expected.is_err());
    assert_outcome(expected, actual);
    assert_eq!(fresh.snapshot(), saved);
}

#[test]
fn startup_live_restore_retains_owner_binding_and_skeleton_rejection_guards() {
    let fixture = fixture();
    let (program, mut vm) = person();
    let fresh = begin(
        &fixture.index,
        &program,
        &mut vm,
        &mut Cache::default(),
        None,
    )
    .unwrap();
    let saved = fresh.snapshot();
    let mut memo = Memo::new(fixture.index.clone());
    Fresh::restore(&fixture.index, saved.clone(), Some(&mut memo)).unwrap();
    for mutation in 0..3 {
        let mut value = serde_json::to_value(&saved).unwrap();
        match mutation {
            0 => value["prepared"]["object"] = 0.into(),
            1 => value["binding"]["next_binding"] = 0.into(),
            2 => value["prepared"]["execution"]["source"]["bones"][0]["translation"][0] = 1.into(),
            _ => unreachable!(),
        }
        let changed: fresh::Saved = serde_json::from_value(value).unwrap();
        let expected = Fresh::restore(&fixture.index, changed.clone(), None);
        let actual = Fresh::restore(&fixture.index, changed, Some(&mut memo));
        assert!(expected.is_err());
        assert_outcome(expected, actual);
        assert_eq!(fresh.snapshot(), saved);
    }
}
