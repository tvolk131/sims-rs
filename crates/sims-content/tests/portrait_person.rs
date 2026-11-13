use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Saved, Session, View};
use sims_sim::{Snapshot, Step, StepStop, lot_route::path::Policy};

#[test]
#[ignore = "requires original Mashuga house and character assets"]
fn original_portrait_person_uses_direct_factory_and_restores_real_assets() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let session = Session::load(
        &index,
        "UserData2/Houses/House09.iff",
        Options {
            routing: Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    construct(&index, session, "initial")
}

#[test]
#[ignore = "requires explicit post-death diagnostic capture and original assets"]
fn deceased_neighbor_constructs_without_a_live_person_and_restores_real_assets() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let capture: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
        "cache/neighborhood-input-experiments/mashuga-live-reset-removal-v81/original-live-character-final.json",
    ))?)?;
    let source = capture["source"]
        .as_str()
        .context("capture source absent")?;
    let house = sims_content::saves::House::load(&index, source)?;
    let session = Session::restore(
        &index,
        Saved {
            version: 1,
            managed: None,
            source: source.into(),
            source_hashes: house.sources,
            options: Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
            modules: serde_json::from_value(capture["program_modules"].clone())?,
            animations: serde_json::from_value(capture["program_animations"].clone())?,
            native_animation_absences: Default::default(),
            headline_catalog: false,
            normalized_object_definitions: Default::default(),
            simulation: serde_json::from_value(capture["simulation"].clone())?,
            assets: serde_json::from_value(capture["assets"].clone())?,
            view: View::default(),
        },
    )?;
    ensure!(session.simulation.vm.tick == 215742);
    ensure!(
        session
            .simulation
            .vm
            .world
            .objects
            .values()
            .all(|o| o.definition.as_ref().is_none_or(|d| d.guid != 1480716884))
    );
    ensure!(
        session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .unwrap()
            .neighbors[&48]
            .person[&61]
            == 0
    );
    construct(&index, session, "deceased-diagnostic")
}

fn construct(index: &Index, mut session: Session, label: &str) -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let original_tick = session.simulation.vm.tick;
    let original_threads = session.simulation.vm.threads.clone();
    let definition = session
        .program
        .modules
        .values()
        .flat_map(|m| {
            m.object_definition_words
                .iter()
                .filter_map(|(&number, words)| {
                    let guid = u32::from(*words.get(14)? as u16)
                        | (u32::from(*words.get(15)? as u16) << 16);
                    (guid == 1480716884).then(|| sims_vm::Definition {
                        source: m.source.clone(),
                        object: number,
                        guid,
                        slots: words[20] as u16,
                    })
                })
        })
        .next()
        .context("Frankie definition absent")?;
    let id = session
        .simulation
        .begin_direct_person_factory(&session.program, definition)
        .map_err(anyhow::Error::msg)?
        .context("object IDs exhausted")?;
    let before = session.simulation.clone();
    ensure!(
        session
            .simulation
            .run_current_tick(&session.program, 100)
            .unwrap_err()
            .contains("synchronous owner")
    );
    ensure!(
        session.simulation == before,
        "ordinary scheduling changed direct construction"
    );
    let factory = &session.simulation.person_factories[&id].factory;
    let vm_snapshot = sims_vm::Snapshot::capture(&session.program, &session.simulation.vm);
    ensure!(
        vm_snapshot
            .clone()
            .restore_with_person_factories(&session.program, &[factory])
            .unwrap_err()
            .contains("unbound saved person factory")
    );
    ensure!(
        vm_snapshot
            .restore_with_direct_person_factories(&session.program, &[factory], &[factory])
            .unwrap_err()
            .contains("duplicate saved person factory owner")
    );
    let mut invalid_owner = Snapshot::capture(&session.program, &session.simulation);
    invalid_owner
        .state
        .person_factories
        .get_mut(&id)
        .unwrap()
        .request = 1;
    ensure!(
        invalid_owner
            .restore(&session.program)
            .unwrap_err()
            .contains("direct person factory ownership")
    );
    let mut restores = 0;
    let mut services = 0;
    let mut complete = false;
    let mut instructions = 0;
    let mut boundaries = std::collections::BTreeSet::new();
    for slice in 0..20000 {
        session.host.update_contexts(&mut session.simulation)?;
        let mut remaining = 1;
        let mut step = Step {
            stop: StepStop::Waiting,
            trace: vec![],
            events: vec![],
        };
        complete = session
            .simulation
            .drive_direct_person_factory(&session.program, id, &mut remaining, &mut step)
            .map_err(anyhow::Error::msg)?;
        instructions += step.trace.len();
        let callback = session
            .simulation
            .vm
            .object_threads
            .get(&id)
            .and_then(|t| session.simulation.vm.threads.get(t))
            .is_some_and(|t| t.callback.is_some());
        let boundary = format!(
            "{:?}/callback={callback}",
            session.simulation.person_factories[&id]
                .factory
                .pending_request()
        );
        let first_boundary = boundaries.insert(boundary);
        let count = session
            .host
            .service(&mut session.simulation, &mut session.program)?;
        services += count;
        ensure!(session.simulation.vm.tick == original_tick);
        for (thread, old) in &original_threads {
            ensure!(
                session.simulation.vm.threads.get(thread) == Some(old),
                "unrelated thread advanced during portrait construction"
            );
        }
        if count > 0 && restores < 12 || first_boundary || complete || slice == 0 {
            let simulation = Snapshot::capture(&session.program, &session.simulation);
            let simulation = serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&simulation)?)?
                .restore(&session.program)
                .map_err(anyhow::Error::msg)?;
            ensure!(simulation == session.simulation);
            let saved = session.host.snapshot();
            let host = sims_content::character_loading::live::Host::restore(
                &index,
                session.host.presentation.clone(),
                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
            )?;
            ensure!(host.snapshot() == saved);
            session.simulation = simulation;
            session.host = host;
            restores += 1;
        }
        if complete {
            break;
        }
    }
    ensure!(complete, "direct factory did not complete");
    ensure!(session.simulation.person_factories[&id].parent.is_none());
    ensure!(session.simulation.person_factories[&id].finalized);
    ensure!(
        session.simulation.characters.contains_key(&id)
            && session.host.characters.contains_key(&id)
    );
    ensure!(session.simulation.unplaced.contains(&id));
    let position = session.simulation.vm.world.positions[&id];
    ensure!([position.x, position.y, position.level] == [-16, -16, 1]);
    let restored = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
    )?;
    ensure!(
        restored.simulation == session.simulation
            && restored.host.snapshot() == session.host.snapshot()
    );
    let report = serde_json::json!({"source":session.house.source,"label":label,"guid":1480716884u32,"object":id,"tick":original_tick,
        "services":services,"instructions":instructions,"restores":restores,"boundaries":boundaries,"complete":true,
        "position":[position.x,position.y,position.level],"existing_threads_unchanged":true,
        "fresh_program_restore_identical":true,
        "scope":"Direct440b2d branch with actual original character/animator assets; placement, portrait pixels and live dialog not connected"});
    std::fs::write(
        root.join(format!(
            "cache/original-direct-portrait-person-{label}.json"
        )),
        serde_json::to_vec_pretty(&report)?,
    )?;
    // Explicit diagnostic input for read-only headless model checks. The
    // off-lot person is not a completed portrait or a playable save milestone.
    std::fs::write(
        root.join(format!(
            "cache/original-direct-portrait-person-{label}-session.json"
        )),
        serde_json::to_vec(&session.save())?,
    )?;
    println!("{report}");
    Ok(())
}
