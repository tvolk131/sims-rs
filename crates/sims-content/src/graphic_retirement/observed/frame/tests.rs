use super::*;
use sims_sim::pose::graphic_frame::Request;
use sims_vm::{Object, World, refresh::RefreshState};

fn cold<T: Serialize + serde::de::DeserializeOwned>(x: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(x).unwrap()).unwrap()
}
fn environment() -> Environment {
    Environment {
        tick: 88444,
        physical: Physical {
            position: [-16; 2],
            facing: 0,
            level: 1,
        },
        camera_rotation: 1,
        floor_vector_bits: [0x3f000000, 0x4114cee5, 0x3f000000],
        terrain_extent: None,
        terrain_corners: None,
        fp_control: 0x003f,
        origin_pose_bits: IDENTITY_POSE,
    }
}
fn authored() -> Input {
    let mut r: Retained = serde_json::from_str(include_str!("../authored.json")).unwrap();
    r.reset.scale_bits = [-16., -16., 0.].map(f32::to_bits);
    r.reset.offset_bits = [0; 3];
    r.model.scale = raw(model::Matrix::IDENTITY);
    r.skin.bounds_bits = empty_bounds();
    r.skin.hierarchy_dirty = 1;
    r.skin.geometry_dirty = 1;
    r.skin.hierarchy_evaluated = u32::MAX;
    r.skin.hierarchy_generation = 1;
    r.skin.bounds_generation = u32::MAX;
    r.skin.generation = 1;
    r.skin.vertex_generation = 1;
    r.skin.pose_generation = 2;
    r.skin.last_clock = u32::MAX;
    let b = &mut r.skin.bones[0];
    b.flags = [1; 3];
    b.wiggle_amplitude_bits = 0;
    b.local_pose_bits = IDENTITY_POSE;
    b.absolute_pose_bits = [0xffff0000; 7];
    b.update = 1;
    b.geometry_stamp = 1;
    let extra = BoneFields {
        first_child: None,
        priority: 17,
        attachment: raw(model::Matrix::IDENTITY),
        rotation_bits: [0xffff0000; 9],
        bounds_generation: u32::MAX,
        bounds_bits: empty_bounds(),
        vertex_bounds_bits: empty_bounds(),
    };
    Input {
        fields: Fields {
            clock_mode: 0,
            now: 43,
            explicit: 0,
            visible: 1,
            bones: BTreeMap::from([(b.id, extra)]),
        },
        retained: r,
    }
}
/// Defaults outside the consumed draw projection are authored test scaffolding.
/// This does not import an actual native World, Thread, lifecycle or registry.
fn vm_for(input: &Input, id: ObjectId) -> (Vm, Binding) {
    let definition = Definition {
        source: "authored.iff".into(),
        object: 128,
        guid: 123,
        slots: 1,
    };
    let mut vm = Vm::new(
        World {
            scheduler_storage: Some(Default::default()),
            refresh: Some(RefreshState {
                visible_level: 1,
                render_clock: 268566,
                draw: BTreeMap::from([(
                    id,
                    DrawState {
                        level: 1,
                        dynamic: true,
                        expiry: i32::MAX,
                        refreshed_at: 46984,
                    },
                )]),
                ..Default::default()
            }),
            ..Default::default()
        },
        17,
    );
    vm.lot_rng_state = Some(111);
    vm.crt_rng_state = Some(222);
    let mut object = Object {
        definition: Some(definition.clone()),
        slots: vec![None],
        ..Default::default()
    };
    vm.world.register_scheduler_object(id, &mut object).unwrap();
    vm.world.objects.insert(id, object);
    let binding = Binding {
        object: id,
        definition,
        registry_lifetime: vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(id)
            .unwrap(),
        native_actor: input.retained.actor,
    };
    (vm, binding)
}
fn finish(owner: &mut Owner, vm: &mut Vm, env: &Environment) -> Vec<Request> {
    let mut requests = vec![];
    for _ in 0..16 {
        let Some(r) = owner.advance(env).unwrap() else {
            return requests;
        };
        if r == Request::Invalidate {
            owner.perform_invalidation(vm).unwrap();
        }
        requests.push(r);
    }
    panic!("Refresh did not complete");
}

#[test]
fn every_service_cold_cut_preserves_raw_caches_and_exact_vm_effect() {
    let input = authored();
    let (mut vm, binding) = vm_for(&input, 1);
    let before_vm = vm.clone();
    let mut restored_vm = cold(&vm);
    let mut warm = Owner::new(input.clone(), binding).unwrap();
    warm.begin_refresh(true).unwrap();
    let mut restored = cold(&warm);
    let env = environment();
    let root = input.retained.skin.root;
    let mut seen = vec![];
    for _ in 0..16 {
        let request = warm.advance(&env).unwrap();
        assert_eq!(request, restored.advance(&env).unwrap());
        if let Some(request) = &request {
            match request {
                Request::Invalidate => {
                    assert_eq!(
                        warm.input().retained.skin.bones[0].absolute_pose_bits,
                        input.retained.skin.bones[0].absolute_pose_bits
                    );
                    let pending = warm.clone();
                    assert_eq!(
                        warm.advance(&Environment {
                            tick: -1,
                            ..env.clone()
                        })
                        .unwrap(),
                        Some(Request::Invalidate)
                    );
                    assert_eq!(warm, pending);
                    warm.perform_invalidation(&mut vm).unwrap();
                    restored.perform_invalidation(&mut restored_vm).unwrap();
                }
                Request::Skin { .. } => {
                    assert_eq!(warm.input().retained.skin.pose_generation, 3);
                    assert_eq!(
                        warm.input().fields.bones[&root].rotation_bits,
                        [0xffff0000; 9]
                    );
                }
                Request::Placement => {
                    assert_eq!(warm.input().retained.skin.hierarchy_generation, 2);
                    assert_eq!(warm.input().retained.skin.pose_generation, 3);
                    assert_eq!(
                        warm.input().retained.skin.bones[0].absolute_pose_bits,
                        [0xffff0000; 7]
                    );
                }
                Request::Hierarchy { .. } => {
                    assert_eq!(warm.input().retained.skin.pose_generation, 4);
                    assert_eq!(warm.input().retained.skin.hierarchy_evaluated, 2);
                    assert_eq!(
                        warm.input().retained.skin.bones[0].absolute_pose_bits,
                        IDENTITY_POSE
                    );
                    assert_eq!(
                        warm.input().fields.bones[&root].rotation_bits,
                        IDENTITY_ROTATION
                    );
                }
                Request::Geometry { .. } => {
                    assert_eq!(warm.input().retained.skin.generation, u32::MAX);
                    assert_eq!(warm.input().retained.skin.vertex_generation, 2);
                    assert_eq!(warm.input().retained.skin.pose_generation, 2);
                }
                _ => panic!("unexpected Refresh service"),
            }
            seen.push(request.clone());
        }
        restored = cold(&restored);
        restored_vm = cold(&restored_vm);
        restored.validate_vm(&restored_vm).unwrap();
        assert_eq!(warm, restored);
        assert_eq!(vm, restored_vm);
        if request.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 5);
    assert!(warm.complete());
    let mut expected_vm = before_vm;
    expected_vm.refresh_object(1, 0, 0, 0).unwrap();
    assert_eq!(
        vm, expected_vm,
        "all VM fields/RNG outside real invalidation stay unchanged"
    );
    assert_eq!(
        warm.input().retained.reset.dirty,
        input.retained.reset.dirty
    );
    assert_eq!(
        warm.input().fields.bones[&root].attachment,
        input.fields.bones[&root].attachment
    );
    let mut retirement = warm.into_retirement().unwrap();
    let mut cold_retirement = cold(&retirement);
    while !retirement.destructor().complete() {
        assert_eq!(
            retirement.advance().unwrap(),
            cold_retirement.advance().unwrap()
        );
        cold_retirement = cold(&cold_retirement);
        cold_retirement.validate().unwrap();
        assert_eq!(retirement, cold_retirement);
    }
    retirement.release_animator().unwrap();
    cold_retirement.release_animator().unwrap();
    assert_eq!(retirement, cold_retirement);
    assert_eq!(
        retirement.retained().skin.bones[0].absolute_pose_bits,
        IDENTITY_POSE
    );
}

#[test]
fn failed_service_and_stale_vm_bindings_are_atomic() {
    let input = authored();
    let (mut vm, binding) = vm_for(&input, 1);
    let mut owner = Owner::new(input, binding).unwrap();
    owner.begin_refresh(true).unwrap();
    assert_eq!(
        owner.advance(&environment()).unwrap(),
        Some(Request::Invalidate)
    );
    for change in 0..6 {
        let mut bad_vm = vm.clone();
        match change {
            0 => bad_vm.world.refresh = None,
            1 => {
                bad_vm.world.refresh.as_mut().unwrap().draw.clear();
            }
            2 => bad_vm.world.objects.get_mut(&1).unwrap().definition = None,
            3 => bad_vm.world.objects.get_mut(&1).unwrap().container = Some((1, 0)),
            4 => {
                bad_vm
                    .world
                    .refresh
                    .as_mut()
                    .unwrap()
                    .draw
                    .get_mut(&1)
                    .unwrap()
                    .dynamic = false
            }
            _ => {
                bad_vm.world.unregister_scheduler_object(1).unwrap();
                let mut object = bad_vm.world.objects.remove(&1).unwrap();
                bad_vm
                    .world
                    .register_scheduler_object(1, &mut object)
                    .unwrap();
                bad_vm.world.objects.insert(1, object);
            }
        }
        let before_vm = bad_vm.clone();
        let before = owner.clone();
        assert!(owner.perform_invalidation(&mut bad_vm).is_err());
        assert_eq!(owner, before);
        assert_eq!(bad_vm, before_vm);
    }
    owner.perform_invalidation(&mut vm).unwrap();
    owner.advance(&environment()).unwrap(); // Skin
    let before = owner.clone();
    assert!(
        owner
            .advance(&Environment {
                fp_control: 0x037f,
                ..environment()
            })
            .is_err()
    );
    assert_eq!(owner, before);
    let mut inbounds = environment();
    // Cached position must be explicitly invalidated by its owner. Here use an
    // authored initial cache, rather than mutate an already validated history.
    let mut input = authored();
    input.retained.reset.scale_bits = [0; 3];
    let (mut vm, binding) = vm_for(&input, 1);
    let mut owner = Owner::new(input, binding).unwrap();
    owner.begin_refresh(true).unwrap();
    owner.advance(&inbounds).unwrap();
    owner.perform_invalidation(&mut vm).unwrap();
    owner.advance(&inbounds).unwrap();
    let before = owner.clone();
    inbounds.terrain_extent = Some(2);
    inbounds.terrain_corners = Some(vec![[0; 4]; 4]);
    assert!(
        owner
            .advance(&inbounds)
            .unwrap_err()
            .to_string()
            .contains("scale-cache")
    );
    assert_eq!(owner, before);
}

#[test]
fn visible_gate_history_forgery_and_unfinished_handoff_reject() {
    let mut input = authored();
    input.fields.visible = 0;
    let (mut vm, binding) = vm_for(&input, 1);
    let mut owner = Owner::new(input.clone(), binding.clone()).unwrap();
    owner.begin_refresh(true).unwrap();
    assert!(owner.clone().into_retirement().is_err());
    assert!(owner.advance(&environment()).unwrap().is_none());
    assert_eq!(owner.input(), &input);
    let mut bad = owner.clone();
    bad.core.input.retained.skin.bones[0].absolute_pose_bits[0] ^= 1;
    assert!(bad.validate().is_err());
    let mut bad = owner.clone();
    bad.history.clear();
    assert!(bad.validate().is_err());
    // Gate false still performs the real Refresh, without changing visible0.
    owner.begin_refresh(false).unwrap();
    assert_eq!(finish(&mut owner, &mut vm, &environment()).len(), 5);
    assert_eq!(owner.input().fields.visible, 0);
    let mut retirement = owner.into_retirement().unwrap();
    retirement.owner.initial.reset.dirty[0] ^= 1;
    assert!(retirement.validate().is_err());
}

#[test]
fn hierarchy_requires_its_explicit_cpu_and_origin_owners_without_early_cache_writes() {
    let input = authored();
    let (mut vm, binding) = vm_for(&input, 1);
    let mut owner = Owner::new(input, binding).unwrap();
    owner.begin_refresh(true).unwrap();
    owner.advance(&environment()).unwrap();
    owner.perform_invalidation(&mut vm).unwrap();
    owner.advance(&environment()).unwrap(); // Skin
    assert_eq!(
        owner.advance(&environment()).unwrap(),
        Some(Request::Placement)
    );
    for change in 0..3 {
        let mut env = environment();
        match change {
            0 => env.fp_control = 0x037f,
            1 => env.origin_pose_bits[0] = 0x3f800000,
            _ => env.origin_pose_bits[0] = 0x80000000,
        }
        let before = owner.clone();
        let before_vm = vm.clone();
        assert!(owner.advance(&env).is_err());
        assert_eq!(owner, before);
        assert_eq!(vm, before_vm);
        assert_eq!(
            owner.input().retained.skin.bones[0].absolute_pose_bits,
            [0xffff0000; 7]
        );
    }
    assert!(matches!(
        owner.advance(&environment()).unwrap(),
        Some(Request::Hierarchy { .. })
    ));
    owner.validate_vm(&vm).unwrap();
}

#[test]
#[ignore = "actual same-cut original58598c component cases; not a full native World/load/tick import"]
fn actual_fifth_refresh_components_match_every_retained_field_and_cold_cut() -> Result<()> {
    use sha2::{Digest, Sha256};
    let path = std::env::var_os("SIMS_OBSERVED_REFRESH_CASES").context("cases path required")?;
    let bytes = std::fs::read(path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes))
            == "41661036fb7e6a287a6fc9654ff59c99d088d57615c929c09a449b173302f36e",
        "actual retained Refresh cases changed"
    );
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
    ensure!(rows.len() == 2, "expected two captured owners");
    for row in rows {
        let input: Input = serde_json::from_value(row["input"].clone())?;
        let expected: Input = serde_json::from_value(row["expected_input"].clone())?;
        let env: Environment = serde_json::from_value(row["environment"].clone())?;
        let id = row["vm_object_id"].as_i64().context("object id missing")? as ObjectId;
        // This VM is an explicitly authored minimal projection. Only the draw
        // fields/containment inputs were read from the actual same-cut capture.
        let (mut vm, binding) = vm_for(&input, id);
        let before_vm = vm.clone();
        let mut restored_vm = cold(&vm);
        let mut owner = Owner::new(input, binding)?;
        owner.begin_refresh(row["component_argument"] == 1)?;
        let mut restored = cold(&owner);
        let mut cuts = 0;
        loop {
            let request = owner.advance(&env)?;
            ensure!(request == restored.advance(&env)?, "cold request mismatch");
            if request == Some(Request::Invalidate) {
                owner.perform_invalidation(&mut vm)?;
                restored.perform_invalidation(&mut restored_vm)?;
            }
            restored = cold(&restored);
            restored_vm = cold(&restored_vm);
            restored.validate_vm(&restored_vm)?;
            ensure!(
                owner == restored && vm == restored_vm,
                "cold retained Refresh mismatch"
            );
            if request.is_none() {
                break;
            }
            cuts += 1;
            ensure!(cuts <= 8, "Refresh did not finish");
        }
        ensure!(
            owner.input() == &expected,
            "actual original component output differs for{id}"
        );
        let mut expected_vm = before_vm;
        expected_vm
            .refresh_object(id, 0, 0, 0)
            .map_err(anyhow::Error::msg)?;
        ensure!(vm == expected_vm, "unexpected authored VM/RNG mutation");
        owner.into_retirement()?.validate()?;
        println!(
            "{}",
            serde_json::json!({"object":id,"actor":row["actor"],
            "component_services":cuts,"all_raw_fields_equal":true,
            "vm_scope":"authored minimal draw projection; no native Thread/World import",
            "strict_native_parity_ticks_added":0})
        );
    }
    Ok(())
}

#[test]
fn identity_tagged_raw_scale_words_are_ignored_and_preserved_at_every_cold_cut() {
    let mut input = authored();
    let scale_words = [
        0x7fc01234, 0x7f800001, 0xff800000, 0x80000000, 0xdeadbeef, 0x7f800000, 1, 0xffffffff,
        0x01234567, 0xffc01234, 0x7fa54321, 0x80000001, 0x76543210, 0x7fffffff, 0xff800001, 0,
    ];
    input.retained.model.scale.bits = scale_words;
    input.retained.model.scale.flags = 1;
    input.retained.notification.packed &= !2;
    let original_scale = input.retained.model.scale.clone();
    let original_packed = input.retained.notification.packed;
    let expected_matrix = Matrix {
        //Exact source-bound negative-position003f ordinary matrix from the
        //two retained Goth cases, not recalculated with the implementation.
        bits: [
            0x35033379, 0, 0x3f800000, 0, 0, 0x3f800000, 0, 0, 0xbf800000, 0, 0x35033379, 0,
            0xc0400000, 0, 0xc0400000, 0x3f800000,
        ],
        flags: 0,
    };
    let (mut vm, binding) = vm_for(&input, 1);
    let before_vm = vm.clone();
    let mut restored_vm = cold(&vm);
    let mut warm = Owner::new(input.clone(), binding).unwrap();
    warm.begin_refresh(true).unwrap();
    let mut restored = cold(&warm);
    let mut placement_seen = false;
    for _ in 0..8 {
        let before_generation = warm.input().retained.skin.hierarchy_generation;
        let request = warm.advance(&environment()).unwrap();
        assert_eq!(request, restored.advance(&environment()).unwrap());
        if request == Some(Request::Invalidate) {
            warm.perform_invalidation(&mut vm).unwrap();
            restored.perform_invalidation(&mut restored_vm).unwrap();
        }
        if request == Some(Request::Placement) {
            let r = &warm.input().retained;
            assert_eq!(r.reset.embedded_matrix, expected_matrix);
            assert_eq!(r.model.placement, expected_matrix);
            assert_eq!(r.skin.pose, expected_matrix);
            assert_eq!(r.notification.transform, expected_matrix);
            assert_eq!(r.notification.packed, original_packed | 2);
            assert_eq!(
                r.skin.hierarchy_generation,
                before_generation.wrapping_add(1)
            );
            placement_seen = true;
        }
        assert_eq!(warm.input().retained.model.scale, original_scale);
        restored = cold(&restored);
        restored_vm = cold(&restored_vm);
        restored.validate_vm(&restored_vm).unwrap();
        assert_eq!(warm, restored);
        assert_eq!(vm, restored_vm);
        if request.is_none() {
            break;
        }
    }
    assert!(placement_seen && warm.complete());
    let mut expected_vm = before_vm;
    expected_vm.refresh_object(1, 0, 0, 0).unwrap();
    assert_eq!(
        vm, expected_vm,
        "all VM fields and RNG except the real draw service stay unchanged"
    );
    //The finite identity reference differs only in the ignored raw scale words.
    input.retained.model.scale = raw(model::Matrix::IDENTITY);
    let (mut reference_vm, binding) = vm_for(&input, 1);
    let mut reference = Owner::new(input, binding).unwrap();
    reference.begin_refresh(true).unwrap();
    finish(&mut reference, &mut reference_vm, &environment());
    let mut expected = reference.input().clone();
    expected.retained.model.scale = original_scale;
    assert_eq!(warm.input(), &expected);
    assert_eq!(vm, reference_vm);
}
