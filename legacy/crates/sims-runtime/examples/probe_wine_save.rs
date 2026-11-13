//! Read-only investigation of an actual Wine-produced original save.
//! Use a coherent private installation tree with its saved UserData in place.
//! Report unsupported import state without clearing it or ticking the session.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::{Index, saves::House};
use sims_runtime::original_session::{Options, Session};
use std::{fs::File, path::PathBuf};

fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("save investigation panicked"))?
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "usage: probe_wine_save CONTENT_ROOT SOURCE NEW_REPORT"
    );
    let content_root = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[2]);
    ensure!(!output.exists(), "preserve previous report");
    let private = PathBuf::from("cache").canonicalize()?;
    ensure!(
        output
            .parent()
            .context("report parent")?
            .canonicalize()?
            .starts_with(private),
        "report must be inside the private cache"
    );
    // Avatar loading also resolves physical installation paths. Combining two
    // resource lists with different roots does not provide a coherent index.
    let index = Index::scan(&content_root)?;
    let mut report = json!({
        "scope": "Actual original-save import probe; no simulation parity is assumed",
        "source": args[1],
        "content_root": content_root,
        "tick_parity_established": false,
        "crt_seed_is_reference_capture": false,
    });
    match House::load(&index, &args[1]) {
        Err(e) => report["house_import_error"] = json!(format!("{e:#}")),
        Ok(house) => {
            report["saved_simulator"] = serde_json::to_value(&house.simulator)?;
            report["source_hashes"] = json!(house.sources);
            report["objects"] = json!(house.objects.instances.len());
            report["objects_with_saved_stacks"] = json!(
                house
                    .objects
                    .instances
                    .values()
                    .filter(|o| !o.stack.is_empty())
                    .count()
            );
            report["saved_person_execution"] = json!(
                house
                    .objects
                    .instances
                    .iter()
                    .filter_map(|(id, object)| object.person.as_ref().map(|person| json!({
                        "object": id,
                        "stack_depth": object.stack.len(),
                        "object_uses": person.object_uses,
                        "active": person.active,
                        "queue": person.queue,
                    })))
                    .collect::<Vec<_>>()
            );
            let program = house.link_program(&index)?;
            match house.prepare_execution(&program) {
                Err(e) => report["deserialization_error"] = json!(format!("{e:#}")),
                Ok(execution) => {
                    report["restored_actions"] = json!(execution.vm.world.actions);
                    report["restored_person_threads"] = json!(
                        house
                            .objects
                            .instances
                            .iter()
                            .filter(|(_, o)| o.person.is_some())
                            .map(|(&id, _)| {
                                let thread = execution.vm.object_threads[&(id as i16)];
                                json!({"object":id,"thread":execution.vm.threads[&thread]})
                            })
                            .collect::<Vec<_>>()
                    );
                    report["deserialization_succeeded"] = json!(true);
                }
            }
            // Keep native post-load ownership. Deserialization success alone
            // would not establish a matching original startup/tick boundary.
            match Session::load(
                &index,
                &args[1],
                Options {
                    native_postload: true,
                    native_load_completion: true,
                    ..Default::default()
                },
            ) {
                Err(e) => report["session_import_error"] = json!(format!("{e:#}")),
                Ok(session) => {
                    report["loaded_tick"] = json!(session.simulation.vm.tick);
                    report["startup_pending"] = json!(true);
                    report["session_loaded"] = json!(true);
                    let path = output.with_extension("session.json");
                    serde_json::to_writer(File::create_new(&path)?, &session.save())?;
                    report["session"] = json!(path);
                }
            }
        }
    }
    serde_json::to_writer_pretty(File::create_new(&output)?, &report)
        .context("write private import report")?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
