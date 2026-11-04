use serde_json::Value;
use sims_vm::{Object, Program, Snapshot, Vm, World, rng};

#[cfg(feature = "private-fixtures")]
#[test]
fn native_simi_stream_binding_reseeding_and_first_draw() {
    let report: Value = serde_json::from_str(include_str!("fixtures/saved-rng.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 162);
    for row in rows {
        let c = &row["input"];
        let n = &row["original"];
        let [lot, mut global] = rng::load_saved(
            c["lot"].as_u64().unwrap() as u32,
            c["global"].as_u64().unwrap() as u32,
            c["residential"].as_bool().unwrap(),
            Some([
                c["timers"][0].as_u64().unwrap() as u32,
                c["timers"][1].as_u64().unwrap() as u32,
            ]),
        )
        .unwrap();
        assert_eq!(u64::from(lot), n["lot"].as_u64().unwrap());
        assert_eq!(u64::from(global), n["global_"].as_u64().unwrap());
        assert_eq!(
            u64::from(rng::next(&mut global)),
            n["next"].as_u64().unwrap()
        );
        assert_eq!(u64::from(global), n["after"].as_u64().unwrap());
    }
    assert_eq!(rng::load_saved(1, 2, true, None).unwrap(), [1, 2]);
    assert!(rng::load_saved(1, 2, false, None).is_err());
}

#[test]
fn three_streams_remain_independent_across_snapshot_and_factory_draws() {
    let mut world = World::default();
    world.objects.insert(
        1,
        Object {
            person: Some((0..256).map(|i| (i, 0)).collect()),
            ..Default::default()
        },
    );
    let p = Program::default();
    let mut vm = Vm::new(world, 0x87654321);
    vm.lot_rng_state = Some(0x12345678);
    let before = vm.clone();
    assert!(vm.randomize_person_traits(1).is_err());
    assert_eq!(vm, before, "missing CRT seed changed the VM");
    vm.crt_rng_state = Some(0x51a5cafe);
    vm.randomize_person_traits(1).unwrap();
    let bytes = serde_json::to_vec(&Snapshot::capture(&p, &vm)).unwrap();
    let saved: Snapshot = serde_json::from_slice(&bytes).unwrap();
    let mut restored = saved.restore(&p).unwrap();
    vm.randomize_person_traits(1).unwrap();
    restored.randomize_person_traits(1).unwrap();
    assert_eq!(vm, restored);
    assert_eq!(vm.rng_state, 0x87654321);
    assert_eq!(vm.lot_rng_state, Some(0x12345678));
    assert_ne!(vm.crt_rng_state, Some(0x51a5cafe));
}
