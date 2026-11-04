use super::*;
use crate::{Index, graphic_loading};
use sims_sim::pose::animator::Settings;

fn input() -> Retained {
    serde_json::from_str(include_str!(
        "../../graphic_retirement/observed/authored.json"
    ))
    .unwrap()
}
fn definition() -> Definition {
    Definition {
        source: "authored".into(),
        object: 128,
        guid: 55,
        slots: 0,
    }
}
fn index() -> Index {
    Index {
        root: ".".into(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    }
}
fn cold(registry: &graphic_loading::Registry) -> graphic_loading::Registry {
    let bytes = serde_json::to_vec(&registry.snapshot()).unwrap();
    graphic_loading::Registry::restore(&index(), serde_json::from_slice(&bytes).unwrap()).unwrap()
}
fn registry() -> graphic_loading::Registry {
    let mut r = graphic_loading::Registry::default();
    r.install_retained(1, definition(), 1, 1, input()).unwrap();
    r
}

#[test]
fn transaction_cold_cuts_and_vm_id_reuse_preserve_surviving_allocations() {
    let mut warm = registry();
    let mut restored = cold(&warm);
    let original = input();
    let mut pool = crate::character_geometry::MeshPool::default();
    let before_pool = pool.clone();
    for r in [&mut warm, &mut restored] {
        r.begin_retirement(1, false).unwrap();
    }
    let mut injected = false;
    for _ in 0..32 {
        let before = warm.snapshot();
        let result = warm.advance_retirement(
            &index(),
            1,
            &mut pool,
            i32::MIN,
            Settings {
                fade_enabled: false,
                fade_duration: -1,
                sampling_config: u32::MAX,
            },
            false,
            |r| {
                if matches!(r, Request::DestroyModel(_)) && !injected {
                    injected = true;
                    anyhow::bail!("injected model notification failure");
                }
                Ok(())
            },
        );
        if result.is_err() {
            assert!(injected);
            assert_eq!(warm.snapshot(), before);
            assert_eq!(pool, before_pool);
            continue;
        }
        let done = result.unwrap();
        let restored_done = restored
            .advance_retirement(
                &index(),
                1,
                &mut pool,
                i32::MAX,
                Settings {
                    fade_enabled: true,
                    fade_duration: 17,
                    sampling_config: 1,
                },
                false,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(done, restored_done);
        restored = cold(&restored);
        assert_eq!(warm.snapshot(), restored.snapshot());
        assert_eq!(pool, before_pool);
        if done {
            break;
        }
    }
    assert!(injected);
    assert!(warm.owned_objects().next().is_none());
    let survivor = &warm.retained.survivors()[&original.skin.id];
    assert!(survivor.outer_released());
    assert_eq!(survivor.retained().skin.references, u32::MAX);
    assert_eq!(survivor.retained().skin.bones, original.skin.bones);
    assert_eq!(
        survivor.retained().skin.model_backpointer,
        original.model.id
    );
    assert!(survivor.retained().allocations[&original.skin.id].live);
    assert!(!survivor.retained().allocations[&original.animator].live);
    // A reused script ID must not acquire the former graphics or destructor.
    let mut vm = sims_vm::Vm::new(Default::default(), 123);
    vm.world.objects.insert(
        1,
        sims_vm::Object {
            definition: Some(Definition {
                guid: 99,
                ..definition()
            }),
            ..Default::default()
        },
    );
    warm.validate(&vm).unwrap();
    assert!(!warm.retirement_started(1));
    assert!(warm.retained.owner(1).is_none());
}

#[test]
fn missing_aliases_wrong_definition_and_forged_retirement_fail() {
    let mut r = registry();
    let before = r.snapshot();
    assert!(r.install_retained(2, definition(), 2, 1, input()).is_err());
    assert_eq!(r.snapshot(), before);
    assert!(r.begin_retirement(1, true).is_err());
    assert_eq!(r.snapshot(), before);
    let mut vm = sims_vm::Vm::new(
        sims_vm::World {
            scheduler_storage: Some(Default::default()),
            ..Default::default()
        },
        5,
    );
    assert!(r.validate(&vm).is_err());
    let mut object = sims_vm::Object {
        definition: Some(Definition {
            guid: 99,
            ..definition()
        }),
        ..Default::default()
    };
    vm.world.register_scheduler_object(1, &mut object).unwrap();
    vm.world.objects.insert(1, object);
    assert!(r.validate(&vm).is_err());
    vm.world.objects.get_mut(&1).unwrap().definition = Some(definition());
    r.validate(&vm).unwrap();
    vm.world.unregister_scheduler_object(1).unwrap();
    let mut object = vm.world.objects.remove(&1).unwrap();
    vm.world.register_scheduler_object(1, &mut object).unwrap();
    vm.world.objects.insert(1, object);
    assert!(
        r.validate(&vm).is_err(),
        "same ID/definition reuse must reject the old asset owner"
    );
    let mut bad = serde_json::to_value(r.snapshot()).unwrap();
    bad["retained"]["graphics"]["1"]["owner"]["retained"]["skin"]["references"] = 8.into();
    assert!(
        graphic_loading::Registry::restore(&index(), serde_json::from_value(bad).unwrap()).is_err()
    );
}

/// Authored post-unlink boundary. No native Thread/World is reconstructed.
#[test]
fn shared_host_teardown_preserves_vm_rng_and_cold_resumption() {
    use crate::character_loading::{Presentation, RetirementEvent, RetirementReferences};
    use sims_sim::pose::{properties, refresh::Response, transform};
    #[derive(Clone, Default)]
    struct RetirementOnly(Vec<RetirementEvent>);
    impl Presentation for RetirementOnly {
        fn retirement_references(&mut self, _: ObjectId) -> anyhow::Result<RetirementReferences> {
            Ok(RetirementReferences {
                selected: false,
                focus: false,
                base_focus: false,
            })
        }
        fn retire(&mut self, id: ObjectId, event: RetirementEvent) -> anyhow::Result<()> {
            assert_eq!(id, 1);
            self.0.push(event);
            Ok(())
        }
        fn context(
            &mut self,
            _: ObjectId,
            _: &transform::Placement,
        ) -> anyhow::Result<properties::Context> {
            anyhow::bail!("unexpected context")
        }
        fn viewport(&mut self) -> anyhow::Result<[i32; 4]> {
            anyhow::bail!("unexpected viewport")
        }
        fn visibility(&mut self, _: ObjectId, _: [i32; 4]) -> anyhow::Result<bool> {
            anyhow::bail!("unexpected visibility")
        }
        fn appearance(
            &mut self,
            _: ObjectId,
            _: &crate::character_appearance::Request,
        ) -> anyhow::Result<()> {
            anyhow::bail!("unexpected appearance")
        }
        fn saved_texture(&mut self, _: ObjectId) -> anyhow::Result<()> {
            anyhow::bail!("unexpected texture")
        }
        fn invalidate(&mut self, _: ObjectId) -> anyhow::Result<()> {
            anyhow::bail!("unexpected invalidation")
        }
        fn property(&mut self, _: ObjectId, _: &properties::Request) -> anyhow::Result<Response> {
            anyhow::bail!("unexpected property")
        }
    }
    let mut vm = sims_vm::Vm::new(
        sims_vm::World {
            scheduler_storage: Some(Default::default()),
            ..Default::default()
        },
        99,
    );
    vm.lot_rng_state = Some(1234);
    vm.crt_rng_state = Some(5678);
    let mut object = sims_vm::Object {
        definition: Some(definition()),
        ..Default::default()
    };
    vm.world.register_scheduler_object(1, &mut object).unwrap();
    vm.world.objects.insert(1, object);
    vm.world.unregister_scheduler_object(1).unwrap();
    vm.world.objects.remove(&1);
    let before_vm = vm.clone();
    let mut simulation = sims_sim::Simulation::new(
        vm,
        sims_sim::Room {
            width: 16,
            height: 16,
            obstacles: vec![],
        },
        Default::default(),
    )
    .unwrap();
    simulation.retained_ordinary_graphics.insert(1);
    simulation.retired_graphics.insert(
        1,
        sims_sim::removal::graphic::Retired {
            definition: definition(),
            registry_lifetime: Some(1),
            tick: 0,
            asset_service_started: false,
        },
    );
    simulation.deletion.push(sims_sim::removal::Deletion {
        target: 1,
        notified: true,
        entry: None,
        next_slot: 0,
        slot_limit: Some(0),
        after_delete_room: None,
        person_reset: None,
        base_reset: None,
        cursor: None,
        entry_definition: Some(definition()),
        force_part: false,
        group: None,
        storage_unlinked: true,
        assets_released: false,
    });
    let settings = Settings {
        fade_enabled: false,
        fade_duration: -1,
        sampling_config: u32::MAX,
    };
    let mut warm = registry();
    let mut restored = cold(&warm);
    let mut restored_simulation = simulation.clone();
    let mut warm_pool = crate::character_geometry::MeshPool::default();
    let mut restored_pool = warm_pool.clone();
    let mut warm_presentation = RetirementOnly::default();
    let mut restored_presentation = warm_presentation.clone();
    for _ in 0..32 {
        // Reject an impossible before-state before touching assets, including
        // the final service that would otherwise commit the survivor handoff.
        for change in 0..7 {
            let mut bad = simulation.clone();
            match change {
                0 => bad.deletion.clear(),
                1 => bad.deletion.last_mut().unwrap().target = 2,
                2 => bad.deletion.last_mut().unwrap().storage_unlinked = false,
                3 => bad.deletion.last_mut().unwrap().assets_released = true,
                4 => {
                    bad.vm.world.objects.insert(1, sims_vm::Object::default());
                }
                5 => {
                    let r = bad.retired_graphics.get_mut(&1).unwrap();
                    r.asset_service_started = !r.asset_service_started;
                }
                _ => bad.retired_graphics.get_mut(&1).unwrap().registry_lifetime = Some(2),
            }
            let before_simulation = bad.clone();
            let mut assets = warm.clone();
            let before_assets = assets.snapshot();
            let mut pool = warm_pool.clone();
            let mut presentation = RetirementOnly::default();
            assert!(
                crate::graphic_retirement::service(
                    &index(),
                    &mut assets,
                    &mut pool,
                    settings,
                    &mut presentation,
                    &mut bad,
                    1
                )
                .is_err()
            );
            assert_eq!(bad, before_simulation);
            assert_eq!(assets.snapshot(), before_assets);
            assert_eq!(pool, warm_pool);
            assert!(presentation.0.is_empty());
        }
        for (registry, simulation, pool, presentation) in [
            (
                &mut warm,
                &mut simulation,
                &mut warm_pool,
                &mut warm_presentation,
            ),
            (
                &mut restored,
                &mut restored_simulation,
                &mut restored_pool,
                &mut restored_presentation,
            ),
        ] {
            registry
                .validate_with_retired(&simulation.vm, &simulation.retired_graphics)
                .unwrap();
            crate::graphic_retirement::service(
                &index(),
                registry,
                pool,
                settings,
                presentation,
                simulation,
                1,
            )
            .unwrap();
            registry
                .validate_with_retired(&simulation.vm, &simulation.retired_graphics)
                .unwrap();
            assert_eq!(simulation.vm, before_vm);
        }
        restored = cold(&restored);
        restored_simulation =
            serde_json::from_slice(&serde_json::to_vec(&restored_simulation).unwrap()).unwrap();
        assert_eq!(simulation, restored_simulation);
        assert_eq!(warm.snapshot(), restored.snapshot());
        assert_eq!(warm_pool, restored_pool);
        assert_eq!(warm_presentation.0, restored_presentation.0);
        if simulation.deletion.last().unwrap().assets_released {
            break;
        }
    }
    assert!(simulation.deletion.last().unwrap().assets_released);
    assert!(simulation.retained_ordinary_graphics.is_empty());
    assert!(simulation.retired_graphics.is_empty());
    assert_eq!(warm_presentation.0, [RetirementEvent::ModelRemoved]);
    assert_eq!(warm.retained.survivors().len(), 1);
    assert!(simulation.native_character_settings.is_none());
}
