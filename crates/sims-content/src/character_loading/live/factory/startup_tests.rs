use super::*;
use crate::{
    character_loading::startup_tests::{SETTINGS, fixture, person},
    skeleton_resources::Cache,
    startup_memo::{Memo, tests::retained},
};
use sims_sim::{Room, character_update, pose::movement, pose::ordinary};
use sims_vm::person_factory::registered::Factory;
use std::sync::Arc;

#[derive(Clone)]
struct UnusedPresentation;

impl Presentation for UnusedPresentation {
    fn retirement_references(&mut self, _object: ObjectId) -> Result<RetirementReferences> {
        anyhow::bail!("unexpected retirement reference query")
    }

    fn retire(&mut self, _object: ObjectId, _event: RetirementEvent) -> Result<()> {
        anyhow::bail!("unexpected retirement")
    }

    fn context(
        &mut self,
        _object: ObjectId,
        _placement: &transform::Placement,
    ) -> Result<properties::Context> {
        anyhow::bail!("unexpected property context query")
    }

    fn viewport(&mut self) -> Result<[i32; 4]> {
        anyhow::bail!("unexpected viewport query")
    }

    fn visibility(&mut self, _object: ObjectId, _viewport: [i32; 4]) -> Result<bool> {
        anyhow::bail!("unexpected visibility query")
    }

    fn appearance(
        &mut self,
        _object: ObjectId,
        _request: &character_appearance::Request,
    ) -> Result<()> {
        anyhow::bail!("unexpected appearance callback")
    }

    fn saved_texture(&mut self, _object: ObjectId) -> Result<()> {
        anyhow::bail!("unexpected texture callback")
    }

    fn invalidate(&mut self, _object: ObjectId) -> Result<()> {
        anyhow::bail!("unexpected invalidation")
    }

    fn property(&mut self, _object: ObjectId, _request: &properties::Request) -> Result<Response> {
        anyhow::bail!("unexpected property callback")
    }
}

fn host(index: Arc<Index>) -> Host<UnusedPresentation> {
    let neighborhood_source = index
        .resources
        .iter()
        .find(|resource| resource.name.eq_ignore_ascii_case("Global.iff"))
        .unwrap()
        .locator
        .clone();
    Host {
        floors: BTreeMap::new(),
        startup: Memo::new(index.clone()),
        index,
        neighborhood_source,
        initializer_settings: SETTINGS,
        fresh: BTreeMap::new(),
        reloads: BTreeMap::new(),
        dependent_jobs: BTreeMap::new(),
        accessories: BTreeMap::new(),
        retirements: BTreeMap::new(),
        notification_jobs: BTreeMap::new(),
        notifications: None,
        presentation: UnusedPresentation,
        mesh_pool: MeshPool::default(),
        characters: BTreeMap::new(),
        clothing: crate::clothing_catalog::Cache::default(),
        skeletons: Some(Cache::default()),
        ordinary: crate::graphic_loading::Registry::default(),
        autonomy_resources: None,
    }
}

fn settings() -> character_update::Settings {
    character_update::Settings {
        terrain_scale: None,
        movement: movement::Settings {
            short_distance: 3.1,
            turn_rate: 1.,
            small_turn: 0.5,
            medium_turn: 1.5,
            large_turn: 3.,
            minimum_weight: 0.5,
            snap_distance: 0.5,
            rate: ordinary::RateSettings {
                base: 1.2,
                positive: 0.2,
                negative: 0.4,
                change: 0.2,
            },
        },
        root_motion: 0,
        person: sims_sim::person_update::Context {
            only_visible: true,
            display_animation_names: false,
            debug_recording: false,
        },
        floor_vector: [0.; 3],
    }
}

fn pending_factory(program: &Program, mut vm: Vm) -> (Simulation, ObjectId) {
    vm.world.person = Some(Default::default());
    vm.world.actions = Some(Default::default());
    vm.world.refresh = Some(Default::default());
    let definition = vm.world.objects[&11].definition.clone().unwrap();
    let mut factory = Factory::begin(&mut vm, program, definition, Default::default(), [0; 2])
        .unwrap()
        .unwrap();
    let id = factory.object();
    assert_eq!(
        factory.next(&mut vm, program).unwrap(),
        Some(FactoryRequest::Registered(id))
    );
    factory
        .complete(&mut vm, program, FactoryRequest::Registered(id), None)
        .unwrap();
    assert_eq!(
        factory.next(&mut vm, program).unwrap(),
        Some(FactoryRequest::Post(Post::Base))
    );
    vm.prepare_person_base_postconstruction(program, id)
        .unwrap();
    factory
        .complete(&mut vm, program, FactoryRequest::Post(Post::Base), None)
        .unwrap();
    assert_eq!(
        factory.next(&mut vm, program).unwrap(),
        Some(FactoryRequest::Post(Post::BindAnimator))
    );
    factory.validate(&vm, program).unwrap();
    let unplaced = vm.world.objects.keys().copied().collect();
    let mut simulation = Simulation::from_placements(
        vm,
        Room {
            width: 16,
            height: 16,
            obstacles: vec![],
        },
        BTreeMap::new(),
        unplaced,
        None,
    )
    .unwrap();
    simulation.native_character_settings = Some(settings());
    simulation.person_factories.insert(
        id,
        serde_json::from_value(serde_json::json!({
            "factory": factory,
            "parent": null,
            "request": 1,
            "wrapper_finished": false,
            "finalized": false,
            "off_lot": true,
            "visitor": null,
            "active": false,
            "native_flags": 0,
            "scores": [],
            "presentation": [],
            "placement": null,
            "acceptance": null,
            "asset_service_started": false,
        }))
        .unwrap(),
    );
    (simulation, id)
}

#[test]
fn startup_host_failed_construction_preserves_published_state_and_retry_installs_once() {
    let fixture = fixture();
    let (mut program, vm) = person();
    let (mut simulation, id) = pending_factory(&program, vm);
    let mut host = host(fixture.index.clone());
    host.validate_factory_bindings(&simulation).unwrap();
    let published_vm = simulation.vm.clone();
    let published_factories = simulation.person_factories.clone();
    let published_assets = host.snapshot();
    let tones = program
        .modules
        .get_mut("semi")
        .unwrap()
        .strings
        .remove(&8192)
        .unwrap();
    let error = host
        .service_factories(&mut simulation, &mut program)
        .unwrap_err();
    assert_eq!(error.to_string(), "appearance skin-tone strings absent");
    assert!(retained(&host.startup));
    assert_eq!(simulation.vm, published_vm);
    assert_eq!(simulation.person_factories, published_factories);
    assert_eq!(host.snapshot(), published_assets);
    assert!(!simulation.person_factories[&id].asset_service_started);
    assert!(host.fresh.is_empty());
    host.validate_factory_bindings(&simulation).unwrap();
    program
        .modules
        .get_mut("semi")
        .unwrap()
        .strings
        .insert(8192, tones);
    let error = host
        .service_factories(&mut simulation, &mut program)
        .unwrap_err();
    assert_eq!(error.to_string(), "fresh animator original terrain absent");
    assert!(simulation.person_factories[&id].asset_service_started);
    assert_eq!(host.fresh.len(), 1);
    assert!(host.fresh.contains_key(&id));
    assert_ne!(simulation.vm.crt_rng_state, published_vm.crt_rng_state);
    assert_ne!(host.skeletons, published_assets.skeletons);
    host.validate_factory_bindings(&simulation).unwrap();
    let retry_vm = simulation.vm.clone();
    let retry_factories = simulation.person_factories.clone();
    let retry_assets = host.snapshot();
    let error = host
        .service_factories(&mut simulation, &mut program)
        .unwrap_err();
    assert_eq!(error.to_string(), "fresh animator original terrain absent");
    assert_eq!(simulation.vm, retry_vm);
    assert_eq!(simulation.person_factories, retry_factories);
    assert_eq!(host.snapshot(), retry_assets);
}

#[test]
fn startup_host_restore_and_fork_keep_snapshot_bytes_and_memo_ownership_separate() {
    let fixture = fixture();
    let (program, mut vm) = person();
    let mut host = host(fixture.index.clone());
    assert!(!retained(&host.startup));
    let before = serde_json::to_vec(&host.snapshot()).unwrap();
    host.startup.installed_catalog(&host.index).unwrap();
    assert!(retained(&host.startup));
    assert_eq!(serde_json::to_vec(&host.snapshot()).unwrap(), before);
    let mut crt = vm.crt_rng_state.unwrap();
    let fresh = Fresh::begin(
        &host.index,
        &mut vm,
        &program,
        11,
        SETTINGS,
        &mut crt,
        host.skeletons.as_mut().unwrap(),
        Some(&mut host.startup),
    )
    .unwrap();
    host.fresh.insert(11, fresh);
    let saved = host.snapshot();
    let bytes = serde_json::to_vec(&saved).unwrap();
    let restored = Host::restore(&fixture.index, UnusedPresentation, saved.clone()).unwrap();
    assert!(retained(&restored.startup));
    assert_eq!(restored.snapshot(), saved);
    assert_eq!(serde_json::to_vec(&restored.snapshot()).unwrap(), bytes);
    fixture.write("GameData_ALL_Recursive/Skins/startup.cmx.bcf", &[0; 2]);
    let mut fork = restored.fork_for_exploration().unwrap();
    assert!(!retained(&fork.startup));
    assert_eq!(fork.snapshot(), saved);
    assert_eq!(serde_json::to_vec(&fork.snapshot()).unwrap(), bytes);
    fork.fresh.get_mut(&11).unwrap().avatar.records.entries[0].name = "fork-only".into();
    assert_ne!(fork.snapshot(), saved);
    assert_eq!(restored.snapshot(), saved);
    assert!(fork.startup.installed_catalog(&fork.index).is_err());
    assert!(!retained(&fork.startup));
    assert!(retained(&restored.startup));
    assert_eq!(restored.snapshot(), saved);
}
