//! Authored adapter/codec controls, not private continuation or restore parity.
use super::*;
use crate::original_session::{application_parent, clock, completed_application, transport};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_content::{Resource, character_loading::live};
use sims_sim::{Room, pose::animator::Settings};
use sims_vm::{Vm, World, scheduler::clock as native_clock};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

mod observer_tests;

fn bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(postcard::to_allocvec(value)?)
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn resource(locator: &str, value: &[u8]) -> Resource {
    Resource {
        locator: locator.into(),
        name: locator.rsplit('/').next().unwrap().into(),
        source: PathBuf::from("/service-capture-must-not-open-payloads").join(locator),
        offset: 0,
        size: value.len() as u64,
        unpacked_size: value.len() as u64,
        archived: false,
        owned: Some(Arc::from(value)),
    }
}

fn session() -> Result<Session> {
    let index = Arc::new(Index {
        root: PathBuf::from("/service-capture-must-not-open-installation"),
        resources: vec![
            resource("UserData/Neighborhood.iff", b"authored neighborhood"),
            resource("UserData/Houses/House07.iff", b"authored house"),
        ],
        archives: 0,
        diagnostics: vec![],
    });
    let mut house: House = serde_json::from_value(json!({
        "source": "UserData/Houses/House07.iff",
        "number": 7,
        "neighborhood_source": "UserData/Neighborhood.iff",
        "sources": {}, "width": 3, "height": 3,
        "floors": [], "floor_map": [], "wall_map": [], "arrays": {},
        "house": {"version": 38, "name": "authored", "pairs": ([[0, 0]; 6]), "tail": []},
        "view": {
            "version": 38, "fields": [0, 0, 0, 0], "camera_direction": 0,
            "unknown_shorts": [0, 0], "unknown_word": 0, "roof": "", "tail": []
        },
        "simulator": {
            "version": 62, "globals": [], "ticks": 0, "scheduler_cursor": 0,
            "funds": 0, "lot_rng": 7, "global_rng": 11, "pending_funds": 0,
            "lot_value": 0, "objects_value": 0, "architecture_value": 0,
            "budget_days": [], "tail": []
        },
        "types": {"version": 2, "entries": []},
        "objects": {"version": 0, "order": [], "types_by_id": {}, "instances": {}, "tail": []},
        "definitions": {}, "neighbors": {"version": 0, "entries": [], "tail": []},
        "neighborhood": {
            "version": 20, "data": ([0; 16]), "inventories": [],
            "options": ([false; 11]), "option_bytes": [], "tail": []
        },
        "resource_storage": null, "families": {}, "household": null, "missing_tile_heads": []
    }))?;
    house.sources = index
        .resources
        .iter()
        .map(|resource| {
            (
                resource.locator.clone(),
                digest(resource.owned.as_ref().unwrap()),
            )
        })
        .collect();
    let assets = live::Snapshot {
        floors: BTreeMap::new(),
        neighborhood_source: house.neighborhood_source.clone(),
        initializer_settings: Settings {
            fade_enabled: false,
            fade_duration: 0,
            sampling_config: 0,
        },
        fresh: BTreeMap::new(),
        reloads: BTreeMap::new(),
        dependent_jobs: BTreeMap::new(),
        accessories: BTreeMap::new(),
        retirements: BTreeMap::new(),
        notification_jobs: BTreeMap::new(),
        notifications: None,
        characters: BTreeMap::new(),
        mesh_pool: Default::default(),
        clothing: Default::default(),
        skeletons: None,
        ordinary: Default::default(),
        autonomy_resources: None,
    };
    Ok(Session {
        loaded_input_origin: None,
        host: Host::restore(&index, View::default(), assets)?,
        index,
        managed_policy: managed::Policy::HistoricalSchema1,
        house,
        program: Program::default(),
        simulation: Simulation::new(
            Vm::new(World::default(), 0x1234),
            Room {
                width: 48,
                height: 48,
                obstacles: vec![],
            },
            BTreeMap::new(),
        )
        .map_err(anyhow::Error::msg)?,
        options: Options::default(),
    })
}

fn state(session: &Session) -> Result<Vec<u8>> {
    let owned: Vec<_> = session
        .index
        .resources
        .iter()
        .map(|resource| (&resource.locator, resource.owned.as_deref()))
        .collect();
    bytes(&(
        session.save(),
        &session.house,
        &session.program,
        &session.index.resources,
        owned,
        session.managed_policy == managed::Policy::CompletedOwnedSchema2,
    ))
}

fn capture(session: &mut Session, format: Format) -> Result<Saved> {
    with_session(session, format, |_, simulation, observe| {
        observe(simulation)
    })
}

#[test]
fn ordinary_capture_matches_entire_saved_representation_and_is_pure() -> Result<()> {
    let mut session = session()?;
    session.host.presentation.viewport = [5, 7, 640, 480];
    session.options.crt_seed = 19;
    let expected = bytes(&session.save())?;
    let before = state(&session)?;
    let saved = with_session(
        &mut session,
        Format::Ordinary,
        |program, simulation, observe| {
            assert!(program.modules.is_empty());
            let before = bytes(simulation)?;
            let first = observe(simulation)?;
            assert_eq!(bytes(&first)?, bytes(&observe(simulation)?)?);
            assert_eq!(bytes(simulation)?, before);
            Ok(first)
        },
    )?;
    assert_eq!(bytes(&saved)?, expected);
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn managed_capture_matches_existing_codec_and_retains_both_origin_policies() -> Result<()> {
    for policy in [
        managed::Policy::HistoricalSchema1,
        managed::Policy::CompletedOwnedSchema2,
    ] {
        let mut session = session()?;
        session.managed_policy = policy;
        let expected = bytes(&session.save_managed()?)?;
        let before = state(&session)?;
        let saved = capture(&mut session, Format::Managed)?;
        assert_eq!(bytes(&saved)?, expected);
        assert_eq!(state(&session)?, before);
        assert_eq!(saved.version, 2);
        assert_eq!(
            saved.managed.unwrap().schema,
            if policy == managed::Policy::HistoricalSchema1 {
                1
            } else {
                2
            }
        );
    }
    Ok(())
}

#[test]
fn capture_observes_the_callback_instant_not_the_end_of_the_call() -> Result<()> {
    let mut session = session()?;
    let mut expected = session.fork_for_exploration()?;
    expected.simulation.vm.rng_state = 31;
    let saved = with_session(&mut session, Format::Ordinary, |_, simulation, observe| {
        simulation.vm.rng_state = 31;
        let before = bytes(simulation)?;
        let saved = observe(simulation)?;
        assert_eq!(bytes(simulation)?, before);
        simulation.vm.rng_state = 37;
        Ok(saved)
    })?;
    assert_eq!(bytes(&saved)?, bytes(&expected.save())?);
    assert_ne!(bytes(&saved)?, bytes(&session.save())?);
    assert_eq!(session.simulation.vm.rng_state, 37);
    Ok(())
}

#[test]
fn each_call_borrows_current_context_and_rejects_a_foreign_simulation() -> Result<()> {
    let mut session = session()?;
    let first = bytes(&capture(&mut session, Format::Ordinary)?)?;
    session.house.source = "UserData/Houses/House08.iff".into();
    session.options.crt_seed = 23;
    session.host.presentation.viewport = [9, 11, 320, 240];
    session.program.modules.insert(
        "authored.iff".into(),
        sims_vm::Module {
            source: "authored.iff".into(),
            normalized_object_definitions: true,
            ..Default::default()
        },
    );
    let expected = bytes(&session.save())?;
    let before = state(&session)?;
    let saved = with_session(&mut session, Format::Ordinary, |_, simulation, observe| {
        let foreign = simulation.clone();
        assert!(
            observe(&foreign)
                .unwrap_err()
                .to_string()
                .contains("does not belong")
        );
        observe(simulation)
    })?;
    assert_ne!(expected, first);
    assert_eq!(bytes(&saved)?, expected);
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn managed_changed_source_and_missing_owned_neighborhood_fail_without_repair() -> Result<()> {
    let mut changed = session()?;
    changed.house.sources.insert(
        changed.house.source.clone(),
        digest(b"different historical import"),
    );
    let before = state(&changed)?;
    assert!(
        changed
            .save_managed()
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    assert!(
        capture(&mut changed, Format::Managed)
            .unwrap_err()
            .to_string()
            .contains("source changed")
    );
    assert_eq!(state(&changed)?, before);

    let mut missing = session()?;
    Arc::make_mut(&mut missing.index)
        .resources
        .retain(|resource| !resource.locator.ends_with("Neighborhood.iff"));
    let before = state(&missing)?;
    assert!(
        missing
            .save_managed()
            .unwrap_err()
            .to_string()
            .contains("owned neighborhood absent")
    );
    assert!(
        capture(&mut missing, Format::Managed)
            .unwrap_err()
            .to_string()
            .contains("owned neighborhood absent")
    );
    assert_eq!(state(&missing)?, before);
    Ok(())
}

#[test]
fn completed_owned_module_context_mismatch_uses_existing_codec_rejection() -> Result<()> {
    let mut session = session()?;
    session.managed_policy = managed::Policy::CompletedOwnedSchema2;
    Arc::make_mut(&mut session.index)
        .resources
        .push(resource("Global.iff", b"authored current global module"));
    session.program.globals = "Global.iff".into();
    session.program.modules.insert(
        "Global.iff".into(),
        sims_vm::Module {
            source: "Global.iff".into(),
            content_hash: digest(b"different resident module version"),
            ..Default::default()
        },
    );
    let before = state(&session)?;
    assert!(
        session
            .save_managed()
            .unwrap_err()
            .to_string()
            .contains("current source version differs")
    );
    assert!(
        capture(&mut session, Format::Managed)
            .unwrap_err()
            .to_string()
            .contains("current source version differs")
    );
    assert_eq!(state(&session)?, before);
    Ok(())
}

struct ExternalSource(PathBuf);

impl ExternalSource {
    fn new(value: &[u8]) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sims-runtime-service-capture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path)?;
        let source = Self(path);
        std::fs::write(source.path(), value)?;
        Ok(source)
    }

    fn path(&self) -> PathBuf {
        self.0.join("authored.iff")
    }
}

impl Drop for ExternalSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn schema1_external_neighborhood_uses_existing_import_and_hash_checks() -> Result<()> {
    let mut session = session()?;
    let source = ExternalSource::new(b"authored neighborhood")?;
    let neighborhood = &mut Arc::make_mut(&mut session.index).resources[0];
    neighborhood.owned = None;
    neighborhood.source = source.path();
    let before = state(&session)?;
    assert_eq!(
        bytes(&capture(&mut session, Format::Managed)?)?,
        bytes(&session.save_managed()?)?
    );
    std::fs::write(source.path(), b"modified neighborhood")?;
    let expected = session.save_managed().unwrap_err().to_string();
    assert!(expected.contains("original import source changed"));
    assert_eq!(
        capture(&mut session, Format::Managed)
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn schema2_external_immutable_sources_use_existing_hash_checks_without_claiming_ownership()
-> Result<()> {
    let mut session = session()?;
    session.managed_policy = managed::Policy::CompletedOwnedSchema2;
    let source = ExternalSource::new(b"authored immutable global")?;
    let mut external = resource("Global.iff", b"authored immutable global");
    external.owned = None;
    external.source = source.path();
    Arc::make_mut(&mut session.index).resources.push(external);
    session.program.globals = "Global.iff".into();
    session.program.modules.insert(
        "Global.iff".into(),
        sims_vm::Module {
            source: "Global.iff".into(),
            content_hash: digest(b"authored immutable global"),
            ..Default::default()
        },
    );
    let before = state(&session)?;
    assert_eq!(
        bytes(&capture(&mut session, Format::Managed)?)?,
        bytes(&session.save_managed()?)?
    );
    assert!(session.index.resources.last().unwrap().owned.is_none());
    std::fs::write(source.path(), b"modified immutable global")?;
    let expected = session.save_managed().unwrap_err().to_string();
    assert!(expected.contains("current source version differs"));
    assert_eq!(
        capture(&mut session, Format::Managed)
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(state(&session)?, before);
    Ok(())
}

#[test]
fn schema2_still_rejects_unowned_mutable_sources() -> Result<()> {
    let mut session = session()?;
    session.managed_policy = managed::Policy::CompletedOwnedSchema2;
    Arc::make_mut(&mut session.index).resources[0].owned = None;
    let before = state(&session)?;
    let expected = session.save_managed().unwrap_err().to_string();
    assert!(expected.contains("mutable source is unowned"));
    assert_eq!(
        capture(&mut session, Format::Managed)
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(state(&session)?, before);
    Ok(())
}

fn application_parent(session: &mut Session) -> Result<()> {
    session.simulation.vm.tick = 88448;
    let child = clock::State::from_completed_update(
        native_clock::Fields {
            timing: Default::default(),
            period: 30,
            timer: Default::default(),
            budget_d8: 0,
            budget_dc: 0,
            poll_messages: 0,
            publish: 0,
        },
        100,
        0,
        session.simulation.vm.tick,
    );
    session.host.presentation.application_parent =
        Some(application_parent::State::from_completed_inputs(
            completed_application::Fields {
                in_update: 1,
                suppressed: 0,
                post_clock_flag: 1,
            },
            &child,
            session.simulation.vm.tick,
        )?);
    session.host.presentation.clock = Some(child);
    session.simulation.native_scheduler =
        Some(sims_sim::scheduler::State::from_completed_cursor(0).map_err(anyhow::Error::msg)?);
    let mut transport = transport::State::from_completed_inputs(
        sims_vm::interface::state::State::constructed(500),
        Default::default(),
    );
    transport.timing = None;
    session.host.presentation.transport = Some(transport);
    Ok(())
}

#[test]
fn application_and_clock_validation_reject_missing_or_crossed_owners() -> Result<()> {
    for format in [Format::Ordinary, Format::Managed] {
        let mut missing = session()?;
        application_parent(&mut missing)?;
        missing.host.presentation.clock = None;
        let before = state(&missing)?;
        assert!(
            missing
                .save_managed()
                .unwrap_err()
                .to_string()
                .contains("lost its elapsed clock")
        );
        assert!(
            capture(&mut missing, format)
                .unwrap_err()
                .to_string()
                .contains("lost its elapsed clock")
        );
        assert_eq!(state(&missing)?, before);

        let mut crossed = session()?;
        application_parent(&mut crossed)?;
        crossed.simulation.vm.tick += 1;
        let before = state(&crossed)?;
        assert!(
            crossed
                .host
                .presentation
                .clock
                .as_ref()
                .unwrap()
                .validate(&crossed)
                .unwrap_err()
                .to_string()
                .contains("unacknowledged lot tick")
        );
        assert!(
            capture(&mut crossed, format)
                .unwrap_err()
                .to_string()
                .contains("unacknowledged lot tick")
        );
        assert_eq!(state(&crossed)?, before);
    }
    Ok(())
}
