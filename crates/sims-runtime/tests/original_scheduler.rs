use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Progress, Saved, Session};
use sims_sim::lot_route::path::Policy;

#[path = "support/registry_startup.rs"]
mod registry_startup;
use registry_startup::step_startup_inputs;

#[test]
#[ignore = "requires the user's original residential houses and assets; no native execution"]
fn original_homes_use_saved_registry_cursor_and_pause_without_running_people() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let start = session.simulation.vm.tick;
        ensure!(
            session.simulation.native_scheduler.as_ref().unwrap().cursor
                == i32::from(session.house.simulator.scheduler_cursor)
        );
        println!(
            "{source}: loaded tick{start}, original cursor{}",
            session.house.simulator.scheduler_cursor
        );
        let globals = &session.simulation.vm.world.globals;
        let initial_partial = globals[&17] != 0 || (globals[&19] != 0 && globals[&10] != 0);
        println!(
            "{source}: loaded pause{}, mode{}, lot{}",
            globals[&17], globals[&19], globals[&10]
        );
        if initial_partial {
            ensure!(step_startup_inputs(&mut session)? == Progress::PartialPass);
            ensure!(session.simulation.vm.tick == start);
        }
        // Explicit controlled Live/Play inputs. The application controller
        // still needs to publish these values; loading must preserve them.
        session.simulation.vm.world.globals.insert(17, 0);
        session.simulation.vm.world.globals.insert(19, 0);
        for n in 0..10 {
            let progress = step_startup_inputs(&mut session).with_context(|| {
                format!("{source}: live pass{n} at{}", session.simulation.vm.tick)
            })?;
            ensure!(
                progress == Progress::Tick,
                "{source}: unexpected live boundary {progress:?}"
            );
        }
        ensure!(session.simulation.vm.tick == start + 10);
        // Controlled pause input, not a change to the private original files or
        // a new authoritative gameplay frontier. Ordinary people must not run.
        session.simulation.vm.world.globals.insert(17, 1);
        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&session.save())?)?;
        let output = root
            .join(
                std::env::var("SIMS_SCHEDULER_EVIDENCE")
                    .unwrap_or_else(|_| "cache/native-simulation-scheduler-homes".into()),
            )
            .join(name);
        std::fs::create_dir_all(&output)?;
        std::fs::write(
            output.join("paused-before.json"),
            serde_json::to_vec(&saved)?,
        )?;
        let mut resumed = Session::restore(&index, saved)?;
        let people = session.simulation.characters.clone();
        let people_vm: Vec<_> = people
            .keys()
            .map(|&id| {
                (
                    id,
                    session.simulation.vm.world.objects[&id].clone(),
                    session.simulation.vm.threads[&session.simulation.vm.object_threads[&id]]
                        .clone(),
                )
            })
            .collect();
        let clock = session.simulation.household.as_ref().unwrap().clock.clone();
        for n in 0..3 {
            ensure!(
                step_startup_inputs(&mut session)
                    .with_context(|| format!("{source}: partial pass{n}"))?
                    == Progress::PartialPass
            );
            ensure!(step_startup_inputs(&mut resumed)? == Progress::PartialPass);
            ensure!(
                session.simulation == resumed.simulation
                    && session.host.snapshot() == resumed.host.snapshot()
            );
            ensure!(
                session.simulation.characters == people,
                "{source}: paused character changed"
            );
            for (id, object, thread) in &people_vm {
                ensure!(&session.simulation.vm.world.objects[id] == object);
                ensure!(
                    &session.simulation.vm.threads[&session.simulation.vm.object_threads[id]]
                        == thread
                );
            }
            ensure!(session.simulation.household.as_ref().unwrap().clock == clock);
            ensure!(session.simulation.vm.tick == start + 10);
        }
        std::fs::write(
            output.join("paused-after.json"),
            serde_json::to_vec(&session.save())?,
        )?;
        for s in [&mut session, &mut resumed] {
            s.simulation.vm.world.globals.insert(17, 0);
        }
        ensure!(
            step_startup_inputs(&mut session)? == Progress::Tick
                && step_startup_inputs(&mut resumed)? == Progress::Tick
        );
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot()
        );
        ensure!(session.simulation.vm.tick == start + 11);
        std::fs::write(
            output.join("live.json"),
            serde_json::to_vec(&session.save())?,
        )?;
        println!(
            "{source}: +11 full ticks,3 partial passes, original people and calendar frozen while paused; cold restoration identical"
        );
    }
    Ok(())
}

#[test]
fn session_options_have_one_scheduler_and_reject_legacy_wire_markers() -> Result<()> {
    let options = Options::default();
    let value = serde_json::to_value(&options)?;
    ensure!(value["native_scheduler"] == true);
    let restored: Options = serde_json::from_value(value.clone())?;
    ensure!(options == restored);
    let mut old = value.clone();
    old["native_scheduler"] = false.into();
    let error = serde_json::from_value::<Options>(old)
        .unwrap_err()
        .to_string();
    ensure!(error.contains("legacy scheduler snapshots are unsupported"));
    let mut missing = value;
    missing.as_object_mut().unwrap().remove("native_scheduler");
    ensure!(serde_json::from_value::<Options>(missing).is_err());
    Ok(())
}

#[test]
#[ignore = "requires private frozen native performance checkpoints and Complete Collection assets"]
fn existing_native_checkpoints_restore_exactly_and_reject_missing_registry_owner() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let base = root.join("cache/household-performance-20261021/after-verify-native");
    for name in ["initial", "checkpoint", "final"] {
        let bytes = std::fs::read(base.join(format!("{name}.json")))?;
        let saved: Saved = serde_json::from_slice(&bytes)?;
        let mut invalid = saved.clone();
        invalid.simulation.state.native_scheduler = None;
        let error = Session::restore(&index, invalid)
            .err()
            .context("missing registry owner accepted")?;
        ensure!(error.to_string().contains("legacy household snapshot"));
        let mut original = Session::restore(&index, saved.clone())?;
        ensure!(
            serde_json::to_vec(&original.save())? == bytes,
            "old {name} checkpoint changed on restore"
        );
        let mut restored = Session::restore(&index, original.save())?;
        for _ in 0..4 {
            let a = original.step()?;
            let b = restored.step()?;
            ensure!(
                a == b
                    && serde_json::to_vec(&original.save())?
                        == serde_json::to_vec(&restored.save())?
            );
            if a == Progress::Dialog {
                break;
            }
        }
    }
    Ok(())
}
