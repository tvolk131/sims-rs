//! All World/Program/physical owners here are authored. Optional actual cases
//! compare only the captured raw Animator component inputs/outputs.
use super::*;
use crate::graphic_retirement::observed::{Matrix, Retained};
use sims_sim::{
    Step, StepStop,
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
};
use sims_vm::{
    Definition, Module, Object, Vm, World,
    autonomy::Placement,
    refresh::{DrawState, RefreshState},
};

fn cold<T: Serialize + serde::de::DeserializeOwned>(x: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(x).unwrap()).unwrap()
}
fn index() -> crate::Index {
    crate::Index {
        root: ".".into(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    }
}
fn restored(r: &crate::graphic_loading::Registry) -> crate::graphic_loading::Registry {
    crate::graphic_loading::Registry::restore(&index(), cold(&r.snapshot())).unwrap()
}
fn identity() -> Matrix {
    Matrix {
        bits: sims_sim::pose::model::Matrix::IDENTITY
            .values
            .map(f32::to_bits),
        flags: 1,
    }
}
fn bounds() -> [u32; 6] {
    let b = sims_sim::pose::model::Bounds::EMPTY;
    [
        b.minimum[0],
        b.minimum[1],
        b.minimum[2],
        b.maximum[0],
        b.maximum[1],
        b.maximum[2],
    ]
    .map(f32::to_bits)
}
fn input() -> frame::Input {
    let mut r: Retained = serde_json::from_str(include_str!(
        "../../../graphic_retirement/observed/authored.json"
    ))
    .unwrap();
    r.reset.scale_bits = [-16., -16., 0.].map(f32::to_bits);
    r.reset.offset_bits = [0; 3];
    r.model.scale = identity();
    r.skin.bounds_bits = bounds();
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
    b.local_pose_bits = [0, 0, 0, 0, 0, 0, 0x3f800000];
    b.absolute_pose_bits = [0xffff0000; 7];
    b.update = 1;
    b.geometry_stamp = 1;
    frame::Input {
        fields: frame::Fields {
            clock_mode: 0,
            now: 43,
            explicit: 0,
            visible: 1,
            bones: BTreeMap::from([(
                b.id,
                frame::BoneFields {
                    first_child: None,
                    priority: 17,
                    attachment: identity(),
                    rotation_bits: [0xffff0000; 9],
                    bounds_generation: u32::MAX,
                    bounds_bits: bounds(),
                    vertex_bounds_bits: bounds(),
                },
            )]),
        },
        retained: r,
    }
}
fn context() -> RefreshContext {
    RefreshContext {
        floor_vector_bits: [0x3f000000, 0x4114cee5, 0x3f000000],
        fp_control: 0x003f,
        origin_pose_bits: [0, 0, 0, 0, 0, 0, 0x3f800000],
    }
}
fn incoming(
    input: frame::Input,
    context: RefreshContext,
    id: ObjectId,
) -> (Program, Simulation, crate::graphic_loading::Registry) {
    let definition = Definition {
        source: "authored".into(),
        object: 128,
        guid: 55,
        slots: 0,
    };
    let mut words = vec![0; 128];
    words[9] = 4;
    words[14] = 55;
    words[67] = 8;
    words[102] = 1;
    let mut module = Module {
        source: "authored".into(),
        ..Default::default()
    };
    module.object_definition_words.insert(128, words);
    module.object_functions.insert(
        128,
        vec![
            sims_formats::execution::ObjectFunction {
                action: 0,
                condition: 0
            };
            27
        ],
    );
    // A registered off-lot object needs a genuine owned Main even though this
    // test never schedules it. This is authored lifecycle state, not an import.
    module.object_functions.get_mut(&128).unwrap()[1].action = 4096;
    module.routines.insert(
        4096,
        sims_vm::Routine {
            name: "Authored Main".into(),
            body: sims_formats::behavior::Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                instructions: vec![sims_formats::behavior::Instruction {
                    opcode: 2,
                    on_true: 254,
                    on_false: 253,
                    operands: [0, 0, 0, 0, 0, 5, 8, 8],
                }],
            },
        },
    );
    let program = Program {
        globals: "authored".into(),
        modules: BTreeMap::from([("authored".into(), module)]),
        ..Default::default()
    };
    let source = FloorSource {
        coverings: vec![0; 36],
        walls: vec![Default::default(); 36],
        object_heads: vec![0; 36],
        flags: vec![0; 36],
    };
    let lot = Lot::rebuild(
        6,
        vec![[0; 4]; 36],
        vec![0; 36],
        vec![255; 36],
        vec![source.clone(), source],
    )
    .unwrap();
    let architecture = Architecture::from_original(lot, &program).unwrap();
    let mut world = World {
        person: Some(sims_vm::person::WorldState {
            object_order: vec![id],
            ..Default::default()
        }),
        camera_rotation: 1,
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
    };
    let mut data = (0..72).map(|i| (i, 0)).collect::<BTreeMap<_, _>>();
    data.extend([(3, -1), (14, -1), (26, 0), (29, -5)]);
    let mut object = Object {
        definition: Some(definition.clone()),
        data,
        native_flags: Some(0),
        ..Default::default()
    };
    world.register_scheduler_object(id, &mut object).unwrap();
    world.objects.insert(id, object);
    world.positions.insert(
        id,
        Placement {
            x: -16,
            y: -16,
            level: 1,
        },
    );
    architecture.publish_room_context(&mut world);
    let links = Links {
        size: 6,
        heads: vec![vec![0; 36]; 2],
        next: BTreeMap::from([(id, 0)]),
    };
    links.publish_tile_chains(&mut world).unwrap();
    let placement =
        sims_sim::lot_placement::State::new_for_world(links, &architecture, &world).unwrap();
    let native = sims_sim::lot_runtime::State {
        placement,
        membership: sims_vm::person_loading::RoomMembership::constructed(&world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        movement: None,
        completion: None,
        route_paths: Default::default(),
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: Default::default(),
        selected_person: None,
        room_effects: vec![],
        routing_policy: Default::default(),
    };
    let binding = frame::Binding {
        object: id,
        definition,
        registry_lifetime: world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(id)
            .unwrap(),
        native_actor: input.retained.actor,
    };
    let mut vm = Vm::new(world, 17);
    vm.tick = 88444;
    vm.lot_rng_state = Some(111);
    vm.crt_rng_state = Some(222);
    vm.start_object_lifecycle(&program, id).unwrap();
    let mut simulation = Simulation::from_loaded_lot(vm, architecture, native).unwrap();
    simulation.retained_ordinary_graphics.insert(id);
    let mut registry = crate::graphic_loading::Registry::default();
    registry
        .install_retained_frame(binding, 1, input, context)
        .unwrap();
    (program, simulation, registry)
}
fn fixture(
    input: frame::Input,
    context: RefreshContext,
    id: ObjectId,
) -> (Program, Simulation, crate::graphic_loading::Registry) {
    let (program, mut simulation, registry) = incoming(input, context, id);
    simulation.begin_synchronous_removal(&program, id).unwrap();
    let mut budget = 1000;
    let mut step = Step {
        stop: StepStop::Budget,
        trace: vec![],
        events: vec![],
    };
    for _ in 0..16 {
        simulation
            .drive_synchronous_removal(&program, &mut budget, &mut step)
            .unwrap();
        if simulation.pending_removal_graphic().is_some() {
            break;
        }
    }
    assert_eq!(simulation.pending_removal_graphic(), Some((id, false)));
    assert_eq!(
        sims_sim::Snapshot::capture(&program, &simulation)
            .restore(&program)
            .unwrap(),
        simulation
    );
    (program, simulation, registry)
}

#[test]
fn public_removal_owner_refreshes_once_across_every_cold_cut() {
    let (program, mut sim, mut warm) = fixture(input(), context(), 1);
    let initial = sim.clone();
    let mut cold_sim = cold(&sim);
    let mut cold_registry = restored(&warm);
    let mut calls = 0;
    while sim.pending_removal_graphic().is_some() {
        calls += 1;
        assert!(calls <= 12);
        for (r, s) in [(&mut warm, &mut sim), (&mut cold_registry, &mut cold_sim)] {
            service(r, s, &program, 1).unwrap();
            r.validate(&s.vm).unwrap();
        }
        cold_registry = restored(&cold_registry);
        cold_sim = sims_sim::Snapshot::capture(&program, &cold_sim)
            .restore(&program)
            .unwrap();
        assert_eq!(sim, cold_sim);
        assert_eq!(warm.snapshot(), cold_registry.snapshot());
    }
    assert!(!warm.updates_pending());
    assert!(warm.retained.owner(1).unwrap().frame().unwrap().complete());
    let mut expected_vm = initial.vm.clone();
    expected_vm.refresh_object(1, 0, 0, 0).unwrap();
    assert_eq!(sim.vm, expected_vm);
    let before = sim.clone();
    let before_assets = warm.snapshot();
    assert!(service(&mut warm, &mut sim, &program, 1).is_err());
    assert_eq!(sim, before);
    assert_eq!(warm.snapshot(), before_assets);
}

#[test]
fn stale_public_removal_state_fails_before_any_asset_or_vm_write() {
    let (program, mut sim, mut registry) = fixture(input(), context(), 1);
    for _ in 0..12 {
        for attack in 0..7 {
            let mut bad = sim.clone();
            let mut assets = registry.clone();
            match attack {
                0 => bad.synchronous_removal = None,
                1 => {
                    bad.retained_ordinary_graphics.clear();
                }
                2 => {
                    bad.vm.tick += 1;
                }
                3 => {
                    bad.vm
                        .world
                        .objects
                        .get_mut(&1)
                        .unwrap()
                        .definition
                        .as_mut()
                        .unwrap()
                        .guid += 1;
                }
                4 => {
                    bad.vm.world.unregister_scheduler_object(1).unwrap();
                    let mut object = bad.vm.world.objects.remove(&1).unwrap();
                    bad.vm
                        .world
                        .register_scheduler_object(1, &mut object)
                        .unwrap();
                    bad.vm.world.objects.insert(1, object);
                }
                5 => {
                    let (_, started) = bad.pending_removal_graphic().unwrap();
                    if started {
                        bad.complete_removal_graphic(1).unwrap();
                    } else {
                        bad.start_removal_graphic(1).unwrap();
                    }
                }
                _ => {
                    bad.native_lot
                        .as_mut()
                        .unwrap()
                        .placement
                        .links
                        .next
                        .clear();
                }
            }
            let before = bad.clone();
            let before_assets = assets.snapshot();
            assert!(
                service(&mut assets, &mut bad, &program, 1).is_err(),
                "attack{attack}"
            );
            assert_eq!(bad, before);
            assert_eq!(assets.snapshot(), before_assets);
        }
        service(&mut registry, &mut sim, &program, 1).unwrap();
        if sim.pending_removal_graphic().is_none() {
            return;
        }
    }
    panic!("frame did not complete");
}

#[test]
fn legacy_wire_shape_and_strict_phase_discrimination() {
    let legacy: Retained = serde_json::from_str(include_str!(
        "../../../graphic_retirement/observed/authored.json"
    ))
    .unwrap();
    let owner = crate::graphic_retirement::observed::Owner::from_retained(legacy.clone()).unwrap();
    let definition = Definition {
        source: "authored".into(),
        object: 128,
        guid: 55,
        slots: 0,
    };
    let mut registry = crate::graphic_loading::Registry::default();
    registry
        .install_retained(1, definition.clone(), 1, 1, legacy)
        .unwrap();
    let expected = serde_json::json!({"graphics":{"1":{"definition":definition,"registry_lifetime":1,"ready":1,"owner":owner,"started":false}},"survivors":{}});
    let bytes = serde_json::to_vec(&registry.snapshot()).unwrap();
    assert_eq!(serde_json::to_value(&registry.retained).unwrap(), expected);
    assert_eq!(
        serde_json::to_vec(&restored(&registry).snapshot()).unwrap(),
        bytes
    );
    let mut hybrid = serde_json::to_value(&owner).unwrap();
    hybrid["live_frame"] = serde_json::json!({});
    assert!(serde_json::from_value::<super::super::Owner>(hybrid).is_err());
    // Raw JSON is essential: Value would already have overwritten duplicates.
    let legacy_json = serde_json::to_string(&owner).unwrap();
    let duplicate = format!(
        "{{\"initial\":{},{}",
        serde_json::to_string(&serde_json::to_value(&owner).unwrap()["initial"]).unwrap(),
        &legacy_json[1..]
    );
    assert!(serde_json::from_str::<super::super::Owner>(&duplicate).is_err());
    let (binding, raw) = {
        let raw = input();
        (
            frame::Binding {
                object: 1,
                definition: Definition {
                    source: "authored".into(),
                    object: 128,
                    guid: 55,
                    slots: 0,
                },
                registry_lifetime: 1,
                native_actor: raw.retained.actor,
            },
            raw,
        )
    };
    let live = frame::Owner::new(raw, binding).unwrap();
    let live_json = serde_json::to_string(&live).unwrap();
    for malformed in [
        format!("{{\"live_frame\":{live_json},\"live_frame\":{live_json}}}"),
        format!("{{\"live_frame\":{live_json},{}", &legacy_json[1..]),
        format!("{{\"live_frame\":{live_json},\"after_frame\":null}}"),
    ] {
        assert!(serde_json::from_str::<super::super::Owner>(&malformed).is_err());
    }
    assert!(
        !serde_json::to_value(crate::graphic_loading::Saved::default())
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("retained")
    );
}

#[test]
fn context_only_storage_is_not_silently_omitted_on_cold_save() {
    let mut saved = crate::graphic_loading::Saved::default();
    saved.retained.refresh_context = Some(context());
    assert!(!saved.retained.is_empty());
    let bytes = serde_json::to_vec(&saved).unwrap();
    let r = crate::graphic_loading::Registry::restore(
        &index(),
        serde_json::from_slice(&bytes).unwrap(),
    )
    .unwrap();
    assert_eq!(r.snapshot(), saved);
}

#[derive(Debug, Clone, Default, PartialEq)]
struct RetirementOnly(Vec<crate::character_loading::RetirementEvent>);
impl crate::character_loading::Presentation for RetirementOnly {
    fn retirement_references(
        &mut self,
        _: ObjectId,
    ) -> Result<crate::character_loading::RetirementReferences> {
        Ok(crate::character_loading::RetirementReferences {
            selected: false,
            focus: false,
            base_focus: false,
        })
    }
    fn retire(
        &mut self,
        _: ObjectId,
        event: crate::character_loading::RetirementEvent,
    ) -> Result<()> {
        self.0.push(event);
        Ok(())
    }
    fn context(
        &mut self,
        _: ObjectId,
        _: &sims_sim::pose::transform::Placement,
    ) -> Result<sims_sim::pose::properties::Context> {
        anyhow::bail!("unexpected person context")
    }
    fn viewport(&mut self) -> Result<[i32; 4]> {
        anyhow::bail!("unexpected viewport")
    }
    fn visibility(&mut self, _: ObjectId, _: [i32; 4]) -> Result<bool> {
        anyhow::bail!("unexpected visibility")
    }
    fn appearance(&mut self, _: ObjectId, _: &crate::character_appearance::Request) -> Result<()> {
        anyhow::bail!("unexpected appearance")
    }
    fn saved_texture(&mut self, _: ObjectId) -> Result<()> {
        anyhow::bail!("unexpected texture")
    }
    fn invalidate(&mut self, _: ObjectId) -> Result<()> {
        anyhow::bail!("unexpected presentation invalidation")
    }
    fn property(
        &mut self,
        _: ObjectId,
        _: &sims_sim::pose::properties::Request,
    ) -> Result<sims_sim::pose::refresh::Response> {
        anyhow::bail!("unexpected property")
    }
}

#[test]
fn public_removal_keeps_refresh_preludes_through_unlink_destruction_and_survival() {
    let (p, mut sim, mut registry) = fixture(input(), context(), 1);
    let mut restored_sim = cold(&sim);
    let mut restored_registry = restored(&registry);
    let mut presentation = RetirementOnly::default();
    let mut cold_presentation = presentation.clone();
    let mut pool = crate::character_geometry::MeshPool::default();
    let mut cold_pool = pool.clone();
    let settings = sims_sim::pose::animator::Settings {
        fade_enabled: false,
        fade_duration: -1,
        sampling_config: u32::MAX,
    };
    let streams = (
        sim.vm.tick,
        sim.vm.rng_state,
        sim.vm.lot_rng_state,
        sim.vm.crt_rng_state,
    );
    let mut observed_retiring = false;
    for _ in 0..128 {
        for (r, s, pool, view) in [
            (&mut registry, &mut sim, &mut pool, &mut presentation),
            (
                &mut restored_registry,
                &mut restored_sim,
                &mut cold_pool,
                &mut cold_presentation,
            ),
        ] {
            if s.pending_removal_graphic().is_some() {
                service(r, s, &p, 1).unwrap();
            } else if s.retired_graphics.contains_key(&1) {
                crate::graphic_retirement::service(&index(), r, pool, settings, view, s, 1)
                    .unwrap();
            } else if !s.synchronous_removal.as_ref().unwrap().complete() {
                let mut budget = 1000;
                let mut step = Step {
                    stop: StepStop::Budget,
                    trace: vec![],
                    events: vec![],
                };
                s.drive_synchronous_removal(&p, &mut budget, &mut step)
                    .unwrap();
            }
            r.validate_with_retired(&s.vm, &s.retired_graphics).unwrap();
        }
        if registry.retained.owner(1).is_some_and(|o| o.retiring()) {
            observed_retiring = true;
            let mut forged = serde_json::to_value(registry.snapshot()).unwrap();
            forged["retained"]["graphics"]["1"]["started"] = false.into();
            assert!(
                crate::graphic_loading::Registry::restore(
                    &index(),
                    serde_json::from_value(forged).unwrap()
                )
                .is_err()
            );
        }
        restored_registry = restored(&restored_registry);
        restored_sim = sims_sim::Snapshot::capture(&p, &restored_sim)
            .restore(&p)
            .unwrap();
        assert_eq!(sim, restored_sim);
        assert_eq!(registry.snapshot(), restored_registry.snapshot());
        assert_eq!(pool, cold_pool);
        assert_eq!(presentation, cold_presentation);
        assert_eq!(
            (
                sim.vm.tick,
                sim.vm.rng_state,
                sim.vm.lot_rng_state,
                sim.vm.crt_rng_state
            ),
            streams
        );
        if sim.synchronous_removal.as_ref().unwrap().complete() {
            break;
        }
    }
    assert!(observed_retiring);
    assert!(sim.synchronous_removal.as_ref().unwrap().complete());
    assert!(registry.retained.owner(1).is_none());
    let survivor = registry.retained.survivors().values().next().unwrap();
    assert!(survivor.outer_released());
    assert!(survivor.prelude().unwrap().complete());
    assert_eq!(
        presentation.0,
        [crate::character_loading::RetirementEvent::ModelRemoved]
    );
}

#[test]
fn frozen_house_clear_exposes_its_actual_refresh_child_without_restarting_removal() {
    let (program, mut simulation, mut registry) = incoming(input(), context(), 1);
    simulation.native_lot.as_mut().unwrap().indicators.primary = Some(1);
    let streams = (
        simulation.vm.tick,
        simulation.vm.rng_state,
        simulation.vm.lot_rng_state,
        simulation.vm.crt_rng_state,
    );
    let mut pool = crate::character_geometry::MeshPool::default();
    let mut presentation = RetirementOnly::default();
    let settings = sims_sim::pose::animator::Settings {
        fade_enabled: false,
        fade_duration: -1,
        sampling_config: u32::MAX,
    };
    let mut refresh_services = 0;
    let mut destructor_services = 0;
    let removed = simulation
        .clear_for_house_read(&program, &mut |s, p| {
            (|| -> Result<()> {
                assert!(s.vm.world.registry_ui_disabled);
                // The clear helper must publish the exact child that produced the
                // request. Reconstructing Removal::new would replay unregister work.
                let owned = s
                    .synchronous_removal
                    .as_ref()
                    .context("frozen clear lost its actual child")?;
                ensure!(
                    owned.target() == 1 && !owned.complete(),
                    "wrong frozen child"
                );
                if let Some((id, _)) = s.pending_removal_graphic() {
                    ensure!(id == 1, "wrong pending frozen graphic");
                    *s = sims_sim::Snapshot::capture(p, s)
                        .restore(p)
                        .map_err(anyhow::Error::msg)?;
                    registry = restored(&registry);
                    service(&mut registry, s, p, id)?;
                    *s = sims_sim::Snapshot::capture(p, s)
                        .restore(p)
                        .map_err(anyhow::Error::msg)?;
                    refresh_services += 1;
                } else if s.retired_graphics.contains_key(&1) {
                    // Native445f84 keeps its pointer until after destruction returns.
                    // This is an incomplete loader, not an arbitrary player-save cut.
                    ensure!(
                        s.native_lot.as_ref().unwrap().indicators.primary == Some(1),
                        "indicator nulled before destruction"
                    );
                    registry = restored(&registry);
                    crate::graphic_retirement::service(
                        &index(),
                        &mut registry,
                        &mut pool,
                        settings,
                        &mut presentation,
                        s,
                        1,
                    )?;
                    destructor_services += 1;
                } else {
                    anyhow::bail!("frozen clear asset request is invisible");
                }
                registry.validate_with_retired(&s.vm, &s.retired_graphics)?;
                ensure!(
                    (
                        s.vm.tick,
                        s.vm.rng_state,
                        s.vm.lot_rng_state,
                        s.vm.crt_rng_state
                    ) == streams,
                    "frozen clear advanced clock or RNG"
                );
                Ok(())
            })()
            .map_err(|e| format!("{e:#}"))
        })
        .unwrap();
    assert_eq!(removed, [1]);
    assert!(refresh_services >= 6 && destructor_services > 0);
    assert!(simulation.synchronous_removal.is_none());
    assert!(simulation.vm.world.objects.is_empty());
    assert_eq!(
        simulation.native_lot.as_ref().unwrap().indicators.primary,
        None
    );
    assert!(!simulation.vm.world.registry_ui_disabled);
    assert_eq!(
        sims_sim::Snapshot::capture(&program, &simulation)
            .restore(&program)
            .unwrap(),
        simulation
    );
    let survivor = registry.retained.survivors().values().next().unwrap();
    assert!(survivor.outer_released() && survivor.prelude().unwrap().complete());
    assert_eq!(
        presentation.0,
        [crate::character_loading::RetirementEvent::ModelRemoved]
    );
}

#[test]
fn frozen_house_clear_host_failures_retain_refresh_and_destructor_progress() {
    for cut in 0..3 {
        let (program, mut simulation, mut registry) = incoming(input(), context(), 1);
        simulation
            .vm
            .world
            .globals
            .extend([(10, 5), (17, 0), (19, 0)]);
        simulation.enable_registry_scheduler(0).unwrap();
        simulation.native_lot.as_mut().unwrap().indicators.primary = Some(1);
        let streams = (
            simulation.vm.tick,
            simulation.vm.rng_state,
            simulation.vm.lot_rng_state,
            simulation.vm.crt_rng_state,
        );
        let mut pool = crate::character_geometry::MeshPool::default();
        let mut presentation = RetirementOnly::default();
        let settings = sims_sim::pose::animator::Settings {
            fade_enabled: false,
            fade_duration: -1,
            sampling_config: u32::MAX,
        };
        let mut stopped = None;
        let result = simulation.clear_for_house_read(&program, &mut |s, p| {
            (|| -> Result<()> {
                let fail = if let Some((id, started)) = s.pending_removal_graphic() {
                    if cut == 0 {
                        ensure!(!started, "first Refresh already started");
                        true
                    } else {
                        service(&mut registry, s, p, id)?;
                        cut == 1
                    }
                } else if s.retired_graphics.contains_key(&1) {
                    crate::graphic_retirement::service(
                        &index(),
                        &mut registry,
                        &mut pool,
                        settings,
                        &mut presentation,
                        s,
                        1,
                    )?;
                    cut == 2
                } else {
                    anyhow::bail!("frozen request lost its child");
                };
                if fail {
                    stopped = Some((s.clone(), registry.snapshot()));
                    anyhow::bail!("injected host failure at cut{cut}");
                }
                Ok(())
            })()
            .map_err(|e| format!("{e:#}"))
        });
        assert_eq!(
            result.unwrap_err(),
            format!("injected host failure at cut{cut}")
        );
        let (at_failure, assets_at_failure) = stopped.unwrap();
        assert_eq!(
            simulation, at_failure,
            "child progress discarded at cut{cut}"
        );
        assert_eq!(registry.snapshot(), assets_at_failure);
        assert!(simulation.vm.world.registry_ui_disabled);
        assert_eq!(
            simulation.native_lot.as_ref().unwrap().indicators.primary,
            Some(1)
        );
        assert_eq!(
            (
                simulation.vm.tick,
                simulation.vm.rng_state,
                simulation.vm.lot_rng_state,
                simulation.vm.crt_rng_state
            ),
            streams
        );
        let child = simulation.synchronous_removal.as_ref().unwrap();
        assert_eq!(child.target(), 1);
        assert!(!child.complete());
        if cut < 2 {
            assert_eq!(simulation.pending_removal_graphic(), Some((1, cut == 1)));
            assert_eq!(
                sims_sim::Snapshot::capture(&program, &simulation)
                    .restore(&program)
                    .unwrap(),
                simulation
            );
        } else {
            assert!(simulation.retired_graphics[&1].asset_service_started);
            // The original pointer still lives through the destructor. The
            // outer clear cursor is not an arbitrary-instruction save owner.
            assert!(
                sims_sim::Snapshot::capture(&program, &simulation)
                    .restore(&program)
                    .is_err()
            );
        }
        assert_eq!(
            simulation.clear_for_house_read(&program, &mut |_, _| panic!("clear reentered")),
            Err("house clear requires a frozen, available registry".into())
        );
        assert_eq!(simulation, at_failure);
        assert_eq!(
            simulation.run_current_tick(&program, 100).unwrap_err(),
            "synchronous removal requires its synchronous owner"
        );
        assert_eq!(simulation, at_failure, "failed loader leaked into gameplay");
    }
}

#[test]
fn incomplete_frame_cannot_retire_and_shared_context_cannot_diverge() {
    let (_p, _sim, mut registry) = fixture(input(), context(), 1);
    let before = registry.snapshot();
    assert!(registry.begin_retirement(1, false).is_err());
    assert_eq!(registry.snapshot(), before);
    let mut binding = registry
        .retained
        .owner(1)
        .unwrap()
        .frame()
        .unwrap()
        .binding()
        .clone();
    binding.object = 2;
    binding.registry_lifetime = 2;
    let mut changed = context();
    changed.fp_control = 0x037f;
    assert!(
        registry
            .install_retained_frame(binding, 1, input(), changed)
            .is_err()
    );
    assert_eq!(registry.snapshot(), before);
}

#[test]
#[ignore = "actual raw component output through an authored physical/removal owner; no native World or gameplay parity credit"]
fn actual_two_refresh_outputs_survive_shared_removal_glue() -> Result<()> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(
        std::env::var_os("SIMS_OBSERVED_REFRESH_CASES").context("cases path required")?,
    )?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes))
            == "41661036fb7e6a287a6fc9654ff59c99d088d57615c929c09a449b173302f36e",
        "actual cases changed"
    );
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes)?;
    ensure!(rows.len() == 2, "two owners required");
    for row in rows {
        let input: frame::Input = serde_json::from_value(row["input"].clone())?;
        let expected: frame::Input = serde_json::from_value(row["expected_input"].clone())?;
        let env: frame::Environment = serde_json::from_value(row["environment"].clone())?;
        let id = row["vm_object_id"].as_i64().context("object missing")? as ObjectId;
        let context = RefreshContext {
            floor_vector_bits: env.floor_vector_bits,
            fp_control: env.fp_control,
            origin_pose_bits: env.origin_pose_bits,
        };
        let (p, mut s, mut r) = fixture(input, context, id);
        let mut cold_s = cold(&s);
        let mut cold_r = restored(&r);
        for calls in 0..12 {
            service(&mut r, &mut s, &p, id)?;
            service(&mut cold_r, &mut cold_s, &p, id)?;
            cold_r = restored(&cold_r);
            cold_s = sims_sim::Snapshot::capture(&p, &cold_s)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                s == cold_s && r.snapshot() == cold_r.snapshot(),
                "cold shared glue differs"
            );
            if s.pending_removal_graphic().is_none() {
                break;
            }
            ensure!(calls < 11, "Refresh did not complete");
        }
        ensure!(
            r.retained.owner(id).unwrap().frame().unwrap().input() == &expected,
            "actual original raw output differs"
        );
        println!(
            "{}",
            serde_json::json!({"object":id,"actor":row["actor"],"all_raw_fields_equal":true,"shared_removal_owner":"authored public API fixture","strict_native_parity_ticks_added":0})
        );
    }
    Ok(())
}
