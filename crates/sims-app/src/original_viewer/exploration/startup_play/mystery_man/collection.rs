use super::*;
use sims_vm::person::InventoryEntry;
use std::collections::BTreeSet;

const ITEMS: [(u16, usize, u8, u32, i16); 5] = [
    (4104, 4, 7, 0x99e81bec, 35),
    (4112, 0, 7, 0x1fec6005, 1),
    (4124, 1, 8, 0x7ac1442e, 1),
    (4125, 2, 8, 0x7aa53f96, 1),
    (4126, 3, 8, 0x7bcb0f36, 1),
];
const EFFECT: u32 = 0xa79366fd;

#[test]
#[ignore = "private opened-package checkpoint; exclusive SIMS_MAGIC_PACKAGE_COLLECTION_OUTPUT"]
fn actual_package_collection_and_empty_lifecycle_through_player_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_collection)?
        .join()
        .map_err(|_| anyhow::anyhow!("package collection probe panicked"))?
}

fn inventory(app: &Application, guid: u32) -> Result<Vec<InventoryEntry>> {
    let people = app
        .session()?
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .context("inventory owner absent")?;
    let matches: Vec<_> = people
        .neighbors
        .values()
        .filter(|neighbor| neighbor.guid == guid)
        .collect();
    ensure!(matches.len() == 1, "resident inventory identity ambiguous");
    Ok(matches[0].inventory.clone())
}

fn save_checkpoint(app: &Application, output: &std::path::Path, name: &str) -> Result<String> {
    verify_retained_routing(app.session()?, app.viewer.args.inferred_routing)?;
    let bytes = app.checkpoint()?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join(name))?
        .write_all(&bytes)?;
    Ok(engine::digest(&bytes))
}

fn run_collection() -> Result<()> {
    let assets = PathBuf::from(std::env::var("SIMS_PRIVATE_ASSET_ROOT")?).canonicalize()?;
    let prior_directory =
        PathBuf::from(std::env::var("SIMS_MAGIC_PACKAGE_COLLECT_FROM")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("SIMS_MAGIC_PACKAGE_COLLECTION_OUTPUT")?);
    ensure!(output.is_absolute(), "private output must be absolute");
    ensure!(
        !output.starts_with(&assets),
        "output is inside the installation"
    );
    std::fs::create_dir(&output).context("refusing to overwrite collection evidence")?;
    let source = std::env::var("SIMS_MAGIC_PACKAGE_SOURCE")?;
    let actor_name = std::env::var("SIMS_MAGIC_PACKAGE_ACTOR")?;
    let inferred =
        std::env::var("SIMS_MAGIC_PACKAGE_INFERRED_ROUTING").is_ok_and(|value| value == "1");
    let save = output.join("unused-player-save.json");
    let mut arguments = vec![
        "sims-room",
        "--original",
        &source,
        "--root",
        assets.to_str().context("asset path is not UTF-8")?,
        "--paused",
        "--floor",
        "1",
        "--save",
        save.to_str().context("output path is not UTF-8")?,
    ];
    if inferred {
        arguments.push("--inferred-routing");
    }
    let args = Args::parse_from(arguments);
    let limits = Limits {
        commands: 10_000,
        completed_ticks: 3_000,
        service_polls: 5_000,
        wall_seconds: 90.0,
    };
    let mut app = None;
    let mut stage = "restore_opened_package";
    let mut dialogs = Vec::new();
    let mut package_dialog = false;
    let mut effects_seen = BTreeSet::new();
    let mut completed = Vec::new();
    let mut report = json!({"native_qualification":false,"source":source,"actor_name":actor_name,
        "routing":if inferred {"inferred_diagnostic"} else {"strict"},
        "scope":"five actual package inventory offers and empty-object lifecycle; not travel or native presentation"});
    let started = Instant::now();
    let result = (|| -> Result<()> {
        let receipt_bytes = std::fs::read(prior_directory.join("receipt.json"))?;
        let prior: Value = serde_json::from_slice(&receipt_bytes)?;
        ensure!(
            prior["success"] == true && prior["stage"] == "opened_action_completed",
            "prior run did not finish the actual Open journey"
        );
        ensure!(
            prior["source"] == source
                && prior["actor_name"] == actor_name
                && prior["routing"] == report["routing"],
            "prior journey identity or routing differs"
        );
        ensure!(
            prior["delivery"]["salesman_observed"] == true
                && prior["delivery"]["injected"] == false,
            "natural delivery evidence absent"
        );
        let mut checkpoint = std::fs::read(prior_directory.join("after-open.private.json"))?;
        verify_resume_checkpoint(
            &json!({"checkpoint_sha256":prior["after_open_checkpoint_sha256"]}),
            &checkpoint,
        )?;
        report["restored_open"] = json!({"directory":prior_directory,
            "receipt_sha256":engine::digest(&receipt_bytes),"checkpoint_sha256":engine::digest(&checkpoint)});
        let resume = if let Ok(directory) = std::env::var("SIMS_MAGIC_PACKAGE_COLLECTION_RESUME") {
            let directory = PathBuf::from(directory).canonicalize()?;
            let bytes = std::fs::read(directory.join("receipt.json"))?;
            let prior_collection: Value = serde_json::from_slice(&bytes)?;
            ensure!(
                prior_collection["success"] == true
                    && prior_collection["stage"] == "five_transfers_and_empty_lifecycle_completed"
                    && prior_collection["completed_items"]
                        .as_array()
                        .is_some_and(|items| items.len() == 5),
                "resume requires a completed five-item journey"
            );
            ensure!(
                prior_collection["source"] == source
                    && prior_collection["actor_name"] == actor_name
                    && prior_collection["routing"] == report["routing"]
                    && prior_collection["restored_open"] == report["restored_open"],
                "collection resume lineage differs"
            );
            checkpoint = std::fs::read(directory.join("before-last-item.private.json"))?;
            verify_resume_checkpoint(
                &json!({"checkpoint_sha256":prior_collection["before_last_item_sha256"]}),
                &checkpoint,
            )?;
            report["resumed_last_item"] = json!({"directory":directory,
                "receipt_sha256":engine::digest(&bytes),"checkpoint_sha256":engine::digest(&checkpoint)});
            report["scope"] = json!(
                "restored final inventory transfer and empty-object lifecycle; not five new transfers"
            );
            Some(prior_collection)
        } else {
            None
        };
        let first_item = if resume.is_some() { 4 } else { 0 };
        let index = Index::scan(&assets)?;
        let icons = Arc::new(dialog_view::Icons::load(&index)?);
        app = Some(Application::restore_checkpoint(
            &args,
            &index,
            &icons,
            Instant::now(),
            &checkpoint,
        )?);
        let app = app.as_mut().unwrap();
        report["retained_routing_before_service"] =
            verify_retained_routing(app.session()?, inferred)?;
        app.drain(&limits)?;
        let initial_tick = app.session()?.simulation.vm.tick;
        let runtime = Instant::now();
        let resident = app
            .session()?
            .house
            .household
            .as_ref()
            .context("household absent")?
            .residents
            .iter()
            .find(|resident| resident.name == actor_name)
            .context("resident absent")?
            .clone();
        ensure!(
            prior["actor"]["guid"] == json!(resident.guid)
                && prior["actor"]["neighbor"] == json!(resident.neighbor),
            "resident lineage differs"
        );
        let actor = Identity::get(app.session()?, resident.object as i16)?;
        ensure!(
            app.session()?.simulation.vm.world.objects[&actor.object]
                .person
                .is_some(),
            "actor is not a person"
        );
        if !app.viewer.people_panel_visible() {
            app.execute(&Command::ExpandPanel, &limits)?;
        }
        app.execute(
            &Command::Resident {
                identity: actor.clone(),
            },
            &limits,
        )?;
        ensure!(
            app.session()?
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(actor.object)
        );
        let packages = instances(app, PACKAGE)?;
        ensure!(packages.len() == 1, "one opened package required");
        let package = Identity::get(app.session()?, packages[0])?;
        let attributes = &app.session()?.simulation.vm.world.objects[&package.object].attributes;
        let mut remaining_flags = [1; 5];
        for (_, attribute, _, _, _) in &ITEMS[..first_item] {
            remaining_flags[*attribute] = 0;
        }
        ensure!(
            attributes.get(7) == Some(&1)
                && attributes.get(..5) == Some(remaining_flags.as_slice()),
            "opened package stock does not match the continuation"
        );
        ensure!(
            instances(app, EFFECT)?.is_empty(),
            "preexisting package effect"
        );
        let initial_inventory = match &resume {
            Some(prior) => serde_json::from_value(prior["initial_inventory"].clone())?,
            None => inventory(app, resident.guid)?,
        };
        let mut expected: Vec<InventoryEntry> = initial_inventory;
        ensure!(
            ITEMS.iter().all(|(_, _, kind, guid, _)| expected
                .iter()
                .all(|entry| entry.kind != *kind || entry.guid != *guid)),
            "diagnostic requires initially absent package tokens"
        );
        report["actor"] =
            json!({"identity":actor,"guid":resident.guid,"neighbor":resident.neighbor});
        report["package"] = json!(package);
        report["initial_inventory"] = json!(expected);
        for (_, _, kind, guid, quantity) in &ITEMS[..first_item] {
            expected.push(InventoryEntry {
                kind: *kind,
                guid: *guid,
                count: *quantity,
            });
        }
        ensure!(
            inventory(app, resident.guid)? == expected,
            "inventory does not match the continuation"
        );
        report["before_collection_sha256"] = json!(save_checkpoint(
            app,
            &output,
            "before-collection.private.json"
        )?);
        let mut frames = 0usize;
        for (item, (routine, attribute, kind, guid, quantity)) in
            ITEMS.into_iter().enumerate().skip(first_item)
        {
            stage = "offer_collection";
            package.check(app.session()?)?;
            app.execute(&Command::Playing { value: false }, &limits)?;
            if item == 4 {
                report["before_last_item_sha256"] = json!(save_checkpoint(
                    app,
                    &output,
                    "before-last-item.private.json"
                )?);
            }
            app.execute(
                &Command::Object {
                    identity: package.clone(),
                },
                &limits,
            )?;
            let menu = app.menu().context("collection menu absent")?;
            ensure!(menu.actor() == actor.object && menu.target() == package.object);
            let offers: Vec<_> = menu
                .actions()
                .iter()
                .enumerate()
                .filter(|(_, action)| {
                    action.target == package.object
                        && action.routine == routine
                        && action.flags & 16 == 0
                })
                .map(|(index, action)| (index, action.clone()))
                .collect();
            ensure!(
                offers.len() == 1,
                "one enabled collection offer required for routine {routine}"
            );
            let (choice, action) = offers.into_iter().next().unwrap();
            let label = app.viewer.action_label(&action);
            app.execute(
                &Command::Choice {
                    index: choice,
                    expected: action,
                },
                &limits,
            )?;
            let actions = app
                .session()?
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .context("action owner absent")?;
            let serial = actions.serial;
            let selected = actions
                .actors
                .get(&actor.object)
                .context("actor action owner absent")?;
            ensure!(
                selected
                    .active
                    .iter()
                    .chain(selected.queue.pending.iter())
                    .any(|action| action.id == serial
                        && action.routine == routine
                        && action.target == package.object),
                "collection was not queued"
            );
            report["current_submission"] =
                json!({"item":item,"routine":routine,"serial":serial,"label":label});
            expected.push(InventoryEntry {
                kind,
                guid,
                count: quantity,
            });
            remaining_flags[attribute] = 0;
            stage = if item == 4 {
                "last_collection_and_removal"
            } else {
                "collecting"
            };
            loop {
                ensure!(
                    frames < 6_000 && app.session()?.simulation.vm.tick - initial_tick <= 3_000,
                    "collection frame/tick limit"
                );
                ensure!(
                    runtime.elapsed() < Duration::from_secs(180),
                    "collection wall limit"
                );
                actor.check(app.session()?)?;
                app.check_errors()?;
                ensure!(!package_dialog, "collection replayed an Open dialog");
                effects_seen.extend(instances(app, EFFECT)?);
                let world = &app.session()?.simulation.vm.world;
                let selected = world
                    .actions
                    .as_ref()
                    .and_then(|actions| actions.actors.get(&actor.object))
                    .context("actor actions absent")?;
                let pending = selected
                    .active
                    .iter()
                    .chain(selected.queue.pending.iter())
                    .any(|action| action.id == serial);
                let successful_return = selected.previous.as_ref().is_some_and(|action| {
                    action.id == serial
                        && action.target == package.object
                        && action.routine == routine
                        && action.flags & 32 != 0
                });
                let transferred = inventory(app, resident.guid)? == expected;
                let package_present = world.objects.contains_key(&package.object);
                if item < 4 {
                    ensure!(
                        package_present,
                        "package removed before all items transferred"
                    );
                    package.check(app.session()?)?;
                    if transferred && successful_return && !pending {
                        let attributes = &world.objects[&package.object].attributes;
                        ensure!(
                            attributes.get(..5) == Some(remaining_flags.as_slice()),
                            "remaining-content flags differ"
                        );
                        break;
                    }
                } else if transferred
                    && !package_present
                    && !pending
                    && !effects_seen.is_empty()
                    && instances(app, EFFECT)?.is_empty()
                {
                    ensure!(
                        !world.positions.contains_key(&package.object)
                            && !app
                                .session()?
                                .simulation
                                .vm
                                .object_threads
                                .contains_key(&package.object),
                        "package teardown incomplete"
                    );
                    ensure!(
                        selected
                            .active
                            .iter()
                            .chain(selected.queue.pending.iter())
                            .all(|action| action.target != package.object),
                        "stale package action"
                    );
                    break;
                }
                if frames % 128 == 0 {
                    println!(
                        "collection item={item}, frame={frames}, tick={}, transferred={transferred}, package={package_present}",
                        app.session()?.simulation.vm.tick
                    );
                }
                advance(
                    app,
                    &limits,
                    &mut dialogs,
                    &mut package_dialog,
                    actor.object,
                )?;
                frames += 1;
            }
            completed.push(json!({"item":item,"attribute":attribute,"routine":routine,"serial":serial,"kind":kind,"guid":guid,
                "quantity":quantity,"inventory":inventory(app,resident.guid)?,"tick":app.session()?.simulation.vm.tick}));
        }
        report["retained_routing_before_final_checkpoint"] =
            verify_retained_routing(app.session()?, inferred)?;
        report["after_collection_sha256"] = json!(save_checkpoint(
            app,
            &output,
            "after-collection.private.json"
        )?);
        stage = if resume.is_some() {
            "last_transfer_and_empty_lifecycle_resumed"
        } else {
            "five_transfers_and_empty_lifecycle_completed"
        };
        Ok(())
    })();
    report["stage"] = json!(stage);
    report["success"] = json!(result.is_ok());
    report["completed_items"] = json!(completed);
    report["effects_observed"] = json!(effects_seen);
    report["dialogs"] = json!(dialogs);
    report["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    if let Err(error) = &result {
        report["error"] = json!(format!("{error:#}"));
    }
    if let Some(app) = &app {
        if let Ok(session) = app.session() {
            report["remaining_packages"] = json!(instances(app, PACKAGE).ok());
            report["package_attributes"] = json!(
                session
                    .simulation
                    .vm
                    .world
                    .objects
                    .values()
                    .filter(|object| object
                        .definition
                        .as_ref()
                        .is_some_and(|definition| definition.guid == PACKAGE))
                    .map(|object| &object.attributes)
                    .collect::<Vec<_>>()
            );
            if let Ok(Some(actor)) = session.simulation.native_selected_person() {
                report["selected_actions"] = json!(
                    session
                        .simulation
                        .vm
                        .world
                        .actions
                        .as_ref()
                        .and_then(|actions| actions.actors.get(&actor))
                );
            }
            report["remaining_effects"] = json!(instances(app, EFFECT).ok());
            report["final_tick"] = json!(session.simulation.vm.tick);
        }
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("receipt.json"))?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!("collection stage={stage}; success={}", result.is_ok());
    result
}

#[test]
fn retained_routing_requires_both_actual_policies_and_the_native_lot_owner() {
    for inferred in [false, true] {
        let expected = if inferred {
            Policy::InferredRectangles
        } else {
            Policy::Strict
        };
        for session in [Policy::Strict, Policy::InferredRectangles] {
            for native_lot in [None, Some(Policy::Strict), Some(Policy::InferredRectangles)] {
                let result = validate_retained_routing(inferred, session, native_lot);
                assert_eq!(
                    result.is_ok(),
                    session == expected && native_lot == Some(expected),
                    "inferred={inferred}, session={session:?}, native_lot={native_lot:?}"
                );
                if let Ok(report) = result {
                    assert_eq!(report["requested"], json!(expected));
                    assert_eq!(report["session"], json!(session));
                    assert_eq!(report["native_lot"], json!(native_lot.unwrap()));
                }
            }
        }
    }
}

#[test]
fn retained_routing_failures_identify_the_unmatched_owner() {
    let wrong_session =
        validate_retained_routing(false, Policy::InferredRectangles, Some(Policy::Strict))
            .unwrap_err();
    assert!(
        wrong_session
            .to_string()
            .contains("Session routing differs")
    );
    let missing_lot = validate_retained_routing(false, Policy::Strict, None).unwrap_err();
    assert!(
        missing_lot
            .to_string()
            .contains("native lot routing owner absent")
    );
    let wrong_lot =
        validate_retained_routing(false, Policy::Strict, Some(Policy::InferredRectangles))
            .unwrap_err();
    assert!(wrong_lot.to_string().contains("native lot routing differs"));
}
