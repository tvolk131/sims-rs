use super::*;
use sims_sim::lot_route::path::Policy;
use std::io::Write;

#[path = "mystery_man/collection.rs"]
mod collection;

#[path = "mystery_man/opening_restore.rs"]
mod opening_restore;

const PACKAGE: u32 = 0x197cdb3c;
const SALESMAN: u32 = 0x44a86a6c;
const CONTENTS: [u32; 4] = [0x0a87e8cf, 0x552a8297, 0x8ec5750d, 0x2baa0029];

fn validate_retained_routing(
    inferred: bool,
    session: Policy,
    native_lot: Option<Policy>,
) -> Result<Value> {
    let expected = if inferred {
        Policy::InferredRectangles
    } else {
        Policy::Strict
    };
    ensure!(
        session == expected,
        "retained Session routing differs: expected {expected:?}, found {session:?}"
    );
    let native_lot = native_lot.context("retained native lot routing owner absent")?;
    ensure!(
        native_lot == expected,
        "retained native lot routing differs: expected {expected:?}, found {native_lot:?}"
    );
    Ok(json!({"requested":expected,"session":session,"native_lot":native_lot}))
}

fn verify_retained_routing(session: &Session, inferred: bool) -> Result<Value> {
    validate_retained_routing(
        inferred,
        session.options.routing,
        session
            .simulation
            .native_lot
            .as_ref()
            .map(|lot| lot.routing_policy),
    )
}

fn verify_resume_checkpoint(prior: &Value, bytes: &[u8]) -> Result<()> {
    let expected = prior["checkpoint_sha256"]
        .as_str()
        .context("resume checkpoint digest absent")?;
    ensure!(
        engine::digest(bytes) == expected,
        "resume checkpoint digest differs"
    );
    Ok(())
}

#[test]
fn resume_checkpoint_requires_the_recorded_exact_bytes() {
    let prior = json!({"checkpoint_sha256":"a8c1550796a691500237d2ff813ad0c269f842763eb4a969978debc0a44b2870"});
    verify_resume_checkpoint(&prior, b"authored checkpoint").unwrap();
    assert!(verify_resume_checkpoint(&prior, b"changed checkpoint").is_err());
    assert!(verify_resume_checkpoint(&json!({}), b"authored checkpoint").is_err());
}

#[test]
#[ignore = "private installed assets; exclusive SIMS_MAGIC_PACKAGE_OUTPUT; actual delivery and player menu"]
fn actual_mystery_man_package_open_through_player_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("package interaction probe panicked"))?
}

fn instances(app: &Application, guid: u32) -> Result<Vec<i16>> {
    Ok(app
        .session()?
        .simulation
        .vm
        .world
        .objects
        .iter()
        .filter_map(|(identifier, object)| {
            object
                .definition
                .as_ref()
                .is_some_and(|definition| definition.guid == guid)
                .then_some(*identifier)
        })
        .collect())
}

fn advance(
    app: &mut Application,
    limits: &Limits,
    dialogs: &mut Vec<Value>,
    package_dialog: &mut bool,
    actor: i16,
) -> Result<()> {
    advance_observed(
        app,
        limits,
        dialogs,
        package_dialog,
        actor,
        None,
        &mut |_, _| Ok(()),
    )
}

fn advance_observed(
    app: &mut Application,
    limits: &Limits,
    dialogs: &mut Vec<Value>,
    package_dialog: &mut bool,
    actor: i16,
    opening: Option<(
        &mut opening_restore::Probe,
        &Path,
        &Identity,
        &Identity,
        opening_restore::Continuation,
    )>,
    observer: &mut impl FnMut(&mut Application, bool) -> Result<()>,
) -> Result<()> {
    app.check_errors()?;
    let world = &app.session()?.simulation.vm.world;
    let command = if let Some(identifier) = world.dialogs.modal {
        let record = world
            .dialogs
            .records
            .iter()
            .find(|record| record.id == identifier)
            .context("modal record absent")?;
        let owned = record.origin.source.ends_with("::MagicStarterKit.iff")
            && record.origin.bhav == 4100
            && record.origin.instruction == 8;
        let car_pool = record.origin.source
            == "GameData_ALL_Recursive/Objects/Objects.far::CarPortal.iff"
            && record.source == record.origin.source
            && record.origin.bhav == 4125
            && record.origin.instruction == 11
            && record.stack_object == actor
            && record.title == "Car Pool"
            && engine::digest(record.body.as_bytes())
                == "c4f983d1377ed8931ec9c56fbe8d1c7dd975c29b8732edab106c7ab5eeaa8c9f"
            && record.buttons.len() == 1
            && record.buttons[0].label == "OK"
            && record.buttons[0].result == 2;
        let school_bus = record.origin.source
            == "GameData_ALL_Recursive/Objects/Objects.far::CarPortal.iff"
            && record.source == record.origin.source
            && record.origin.bhav == 4125
            && record.origin.instruction == 15
            && record.title == "School Bus"
            && engine::digest(record.body.as_bytes())
                == "ccdb5655d8be72b82dc54a592be42c38098d917904ba97c924560611884a3cbb"
            && app
                .session()?
                .house
                .household
                .as_ref()
                .is_some_and(|household| {
                    household.residents.iter().any(|resident| {
                        resident.name == "Cassandra"
                            && resident.object as i16 == record.stack_object
                            && world
                                .objects
                                .get(&record.stack_object)
                                .is_some_and(|object| {
                                    object.person.is_some()
                                        && object.definition.as_ref().is_some_and(|definition| {
                                            definition.guid == resident.guid
                                        })
                                })
                    })
                })
            && record.buttons.len() == 1
            && record.buttons[0].label == "OK"
            && record.buttons[0].result == 2;
        let result = if owned {
            ensure!(record.buttons.len() == 1, "unexpected package response set");
            *package_dialog = true;
            record.buttons[0].result
        } else if car_pool || school_bus {
            2
        } else {
            ensure!(!record.buttons.is_empty(), "modal has no offered response");
            startup_response(record)?
        };
        dialogs.push(json!({"id":identifier,"source":record.origin.source,
            "bhav":record.origin.bhav,"instruction":record.origin.instruction,"result":result,
            "package_owned":owned,"pinned_car_pool_notice":car_pool,"pinned_school_bus_notice":school_bus}));
        Command::Dialog {
            id: identifier,
            result,
            body_sha256: engine::digest(record.body.as_bytes()),
        }
    } else if app.viewer.presentation_pending() {
        Command::Poll
    } else if !app.viewer.playing {
        Command::Playing { value: true }
    } else {
        Command::Advance { millis: 40 }
    };
    if let Some((probe, output, actor, package, mut continuation)) = opening {
        continuation.package_dialog_seen = *package_dialog;
        if let Some(request) = probe.request(app, actor, package, &continuation)? {
            let outcome = app.execute_capturing(&command, limits, request, &mut |app| {
                observer(app, *package_dialog)
            })?;
            outcome.execution?;
            if let Some(captured) = outcome.captured {
                probe.consume(app, output, actor, package, captured)?;
            }
            return Ok(());
        }
    }
    app.execute_observed(&command, limits, &mut |app| observer(app, *package_dialog))
}

fn run() -> Result<()> {
    let assets = PathBuf::from(std::env::var("SIMS_PRIVATE_ASSET_ROOT")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("SIMS_MAGIC_PACKAGE_OUTPUT")?);
    std::fs::create_dir(&output).context("refusing to overwrite package evidence")?;
    let source = std::env::var("SIMS_MAGIC_PACKAGE_SOURCE")
        .unwrap_or_else(|_| "UserData/Houses/House07.iff".into());
    let actor_name = std::env::var("SIMS_MAGIC_PACKAGE_ACTOR").unwrap_or_else(|_| "Bob".into());
    let inferred =
        std::env::var("SIMS_MAGIC_PACKAGE_INFERRED_ROUTING").is_ok_and(|value| value == "1");
    let mut restore_probe = opening_restore::Probe::from_environment()?;
    let opening_resume = std::env::var_os("SIMS_MAGIC_PACKAGE_OPENING_RESUME");
    ensure!(
        opening_resume.is_none()
            || (std::env::var_os("SIMS_MAGIC_PACKAGE_RESUME").is_none()
                && std::env::var_os("SIMS_MAGIC_PACKAGE_OPENING_RESTORE_CUT").is_none()),
        "active-opening resume cannot also select a before-Open resume or another cut"
    );
    let started = Instant::now();
    let limits = Limits {
        commands: 10_000,
        completed_ticks: 3_000,
        service_polls: 5_000,
        wall_seconds: 90.0,
    };
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
    let mut app = None;
    let mut stage = "startup";
    let mut dialogs = Vec::new();
    let mut package_dialog = false;
    let mut observations = Vec::new();
    let mut offers = Vec::new();
    let mut report = json!({"native_qualification":false,"routing":if inferred {"inferred_diagnostic"} else {"strict"},"source":source,
        "actor_name":actor_name,
        "scope":"controller-delivered package and production player-input Open only; not collection"});
    let result = (|| -> Result<()> {
        let mut continuation = None;
        let prior = if let Some(directory) = opening_resume
            .clone()
            .or_else(|| std::env::var_os("SIMS_MAGIC_PACKAGE_RESUME"))
        {
            let directory = PathBuf::from(directory).canonicalize()?;
            let prior: Value =
                serde_json::from_slice(&std::fs::read(directory.join("receipt.json"))?)?;
            ensure!(
                prior["source"] == source && prior["actor_name"] == actor_name,
                "resume proof belongs to another household or resident"
            );
            ensure!(
                prior["delivery"]["salesman_observed"] == true
                    && prior["delivery"]["injected"] == false,
                "resume proof lacks observed natural delivery"
            );
            ensure!(
                prior["routing"] == report["routing"],
                "resume proof routing differs"
            );
            let checkpoint_name = if opening_resume.is_some() {
                continuation = Some(opening_restore::Continuation::from_receipt(&prior)?);
                "during-opening.private.json"
            } else {
                "before-open.private.json"
            };
            let proof = if continuation.is_some() {
                &prior["opening_restore"]["evidence"]
            } else {
                &prior
            };
            let bytes = std::fs::read(directory.join(checkpoint_name))?;
            verify_resume_checkpoint(proof, &bytes)?;
            let index = Index::scan(&assets)?;
            let icons = Arc::new(dialog_view::Icons::load(&index)?);
            app = Some(Application::restore_checkpoint(
                &args,
                &index,
                &icons,
                Instant::now(),
                &bytes,
            )?);
            report["retained_routing_before_service"] =
                verify_retained_routing(app.as_ref().unwrap().session()?, inferred)?;
            if continuation.is_some() {
                ensure!(
                    app.as_ref().unwrap().checkpoint()? == bytes,
                    "active-opening checkpoint changed during separate-process restore"
                );
                report["opening_resume"] = json!({
                    "cut":proof["cut"],"checkpoint_sha256":engine::digest(&bytes),
                    "capture_process_id":proof["process_id"],"resume_process_id":std::process::id(),
                    "exact_checkpoint_equal_before_service":true,
                    "separate_process_resume":true,"native_qualification":false,
                });
                report["offer"] = prior["offer"].clone();
                report["submission"] = prior["submission"].clone();
            } else {
                app.as_mut().unwrap().drain(&limits)?;
            }
            report["resume"] = json!({"directory":directory,"checkpoint_sha256":engine::digest(&bytes),
                "kind":if continuation.is_some() {"separate-process active-Open continuation; not a fresh delivery or submission"}
                    else {"restore of previously observed delivery; not a fresh delivery run"}});
            Some(prior)
        } else {
            let ready = Arc::new(Ready::load(&args)?);
            {
                let loaded = ready
                    .session
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Ready Session lock poisoned"))?;
                report["retained_routing_before_service"] = verify_retained_routing(
                    loaded.as_ref().context("Ready Session absent")?,
                    inferred,
                )?;
            }
            app = Some(Application::install_ready(
                &args,
                ready,
                Instant::now(),
                &limits,
            )?);
            None
        };
        report["startup_seconds"] = json!(started.elapsed().as_secs_f64());
        let app = app.as_mut().unwrap();
        let initial_tick = app.session()?.simulation.vm.tick;
        report["initial_calendar"] = json!(calendar(app)?);
        let resident = app
            .session()?
            .house
            .household
            .as_ref()
            .context("resident household absent")?
            .residents
            .iter()
            .find(|resident| resident.name == actor_name)
            .context("requested original resident identity absent")?
            .clone();
        let actor = Identity::get(app.session()?, resident.object as i16)?;
        let person = &app.session()?.simulation.vm.world.objects[&actor.object];
        ensure!(person.person.is_some(), "resident is not a person");
        ensure!(
            person.definition.as_ref().map(|definition| definition.guid) == Some(resident.guid)
        );
        report["actor"] = json!({"identity":actor,"guid":resident.guid,
            "neighbor":resident.neighbor,"source":resident.character_source});
        if !app.viewer.people_panel_visible() {
            app.execute(&Command::ExpandPanel, &limits)?;
        }
        ensure!(
            app.viewer.people_panel_visible(),
            "player controls are not visible"
        );
        if continuation.is_none() {
            app.execute(
                &Command::Resident {
                    identity: actor.clone(),
                },
                &limits,
            )?;
        }
        ensure!(
            app.session()?
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(actor.object),
            "original resident selection was not installed"
        );
        let mut selected_package = if let Some(prior) = &prior {
            let package: Identity = serde_json::from_value(prior["delivery"]["identity"].clone())?;
            package.check(app.session()?)?;
            ensure!(
                instances(app, PACKAGE)? == vec![package.object],
                "restored package identity differs"
            );
            ensure!(
                app.session()?.simulation.vm.world.objects[&package.object]
                    .attributes
                    .get(7)
                    == Some(&0),
                "restored package is already opened"
            );
            report["delivery"] = prior["delivery"].clone();
            Some(package)
        } else {
            ensure!(
                instances(app, PACKAGE)?.is_empty(),
                "package predates observed delivery"
            );
            None
        };
        let before_contents: Vec<_> = if let Some(continuation) = &continuation {
            package_dialog = continuation.package_dialog_seen;
            let package = selected_package.as_ref().context("active package absent")?;
            continuation.validate_active(app, actor.object, package)?;
            continuation.before_contents.clone()
        } else {
            CONTENTS
                .iter()
                .map(|guid| instances(app, *guid))
                .collect::<Result<_>>()?
        };
        let runtime = Instant::now();
        let mut observed_salesman = prior.is_some();
        let mut submitted = continuation.is_some();
        let mut submitted_serial = continuation.as_ref().map(|continuation| continuation.serial);
        let mut effects_seen = false;
        let mut opening_started = continuation.as_ref().map(|_| Instant::now());
        let mut checkpoint_saved = continuation.is_some();
        let mut next_offer_tick = initial_tick;
        stage = if continuation.is_some() { "opening" } else { "delivery" };
        for frame in 0..6_000 {
            app.check_errors()?;
            let phase_elapsed = opening_started.unwrap_or(runtime).elapsed().as_secs_f64();
            let phase_limit = if opening_started.is_some() {
                limits.wall_seconds
            } else {
                120.0
            };
            ensure!(
                phase_elapsed < phase_limit,
                "package phase runtime deadline"
            );
            ensure!(
                app.session()?
                    .simulation
                    .vm
                    .tick
                    .saturating_sub(initial_tick)
                    < limits.completed_ticks,
                "package tick budget"
            );
            observed_salesman |= !instances(app, SALESMAN)?.is_empty();
            let packages = instances(app, PACKAGE)?;
            ensure!(packages.len() <= 1, "multiple packages appeared");
            if selected_package.is_none() && !packages.is_empty() {
                ensure!(observed_salesman, "delivery actor was not observed");
                let identity = Identity::get(app.session()?, packages[0])?;
                ensure!(
                    app.session()?.simulation.vm.world.objects[&identity.object]
                        .person
                        .is_none()
                );
                report["delivery"] = json!({"identity":identity,"tick":app.session()?.simulation.vm.tick,
                    "salesman_observed":true,"injected":false});
                selected_package = Some(identity);
                stage = "offer";
            }
            if !submitted
                && !app.viewer.presentation_pending()
                && app.session()?.simulation.vm.world.dialogs.modal.is_none()
                && app.session()?.simulation.vm.tick >= next_offer_tick
            {
                if let Some(package) = &selected_package {
                    if !app
                        .session()?
                        .simulation
                        .vm
                        .world
                        .positions
                        .get(&package.object)
                        .is_some_and(|position| position.x >= 0 && position.y >= 0)
                    {
                        advance(
                            app,
                            &limits,
                            &mut dialogs,
                            &mut package_dialog,
                            actor.object,
                        )?;
                        continue;
                    }
                    app.execute(&Command::Playing { value: false }, &limits)?;
                    if !checkpoint_saved {
                        report["retained_routing_before_final_checkpoint"] =
                            verify_retained_routing(app.session()?, inferred)?;
                        let checkpoint = app.checkpoint()?;
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(output.join("before-open.private.json"))?
                            .write_all(&checkpoint)?;
                        report["checkpoint_sha256"] = json!(engine::digest(&checkpoint));
                        checkpoint_saved = true;
                    }
                    app.execute(
                        &Command::Object {
                            identity: package.clone(),
                        },
                        &limits,
                    )?;
                    let menu = app.menu().context("package menu not offered")?;
                    ensure!(menu.actor() == actor.object && menu.target() == package.object);
                    ensure!(offers.len() < 32, "package offer-attempt budget");
                    offers.push(json!({"tick":app.session()?.simulation.vm.tick,"actions":menu.actions(),
                        "displayed_labels":menu.actions().iter().map(|action| app.viewer.action_label(action)).collect::<Vec<_>>()}));
                    let choices: Vec<_> = menu
                        .actions()
                        .iter()
                        .enumerate()
                        .filter(|(_, action)| {
                            action.target == package.object
                                && action.routine == 4097
                                && app.viewer.action_label(action) == "Open"
                                && action.flags & 16 == 0
                        })
                        .map(|(index, action)| (index, action.clone()))
                        .collect();
                    if choices.is_empty() {
                        next_offer_tick = app.session()?.simulation.vm.tick + 30;
                        app.execute(&Command::CloseMenu, &limits)?;
                        advance(
                            app,
                            &limits,
                            &mut dialogs,
                            &mut package_dialog,
                            actor.object,
                        )?;
                        continue;
                    }
                    ensure!(
                        choices.len() == 1,
                        "one enabled original Open offer required"
                    );
                    let (index, expected) = choices.into_iter().next().unwrap();
                    report["offer"] = json!({"index":index,"action":expected});
                    app.execute(&Command::Choice { index, expected }, &limits)?;
                    let actions = app
                        .session()?
                        .simulation
                        .vm
                        .world
                        .actions
                        .as_ref()
                        .context("original action owner absent")?;
                    let serial = actions.serial;
                    let selected = actions
                        .actors
                        .get(&actor.object)
                        .context("actor action state absent")?;
                    ensure!(
                        selected
                            .active
                            .iter()
                            .chain(selected.queue.pending.iter())
                            .any(|action| action.id == serial
                                && action.target == package.object
                                && action.routine == 4097),
                        "player Open submission was not queued"
                    );
                    submitted_serial = Some(serial);
                    report["submission"] =
                        json!({"serial":serial,"tick":app.session()?.simulation.vm.tick});
                    submitted = true;
                    opening_started = Some(Instant::now());
                    stage = "opening";
                }
            }
            if let Some(package) = &selected_package {
                package.check(app.session()?)?;
                if !effects_seen
                    && app.session()?.simulation.vm.world.objects[&package.object]
                        .attributes
                        .get(7)
                        == Some(&1)
                {
                    ensure!(
                        package_dialog,
                        "package-owned animation dialog was not observed"
                    );
                    let after_contents: Vec<_> = CONTENTS
                        .iter()
                        .map(|guid| instances(app, *guid))
                        .collect::<Result<_>>()?;
                    report["content_counts"] = json!({"before":before_contents.iter().map(Vec::len).collect::<Vec<_>>(),
                        "after":after_contents.iter().map(Vec::len).collect::<Vec<_>>()});
                    for (before, after) in before_contents.iter().zip(&after_contents) {
                        ensure!(
                            after.len() == before.len() + 1,
                            "expected package content absent"
                        );
                    }
                    let added: Vec<_> = before_contents
                        .iter()
                        .zip(&after_contents)
                        .map(|(before, after)| {
                            *after
                                .iter()
                                .find(|identifier| !before.contains(identifier))
                                .unwrap()
                        })
                        .collect();
                    let edge = i32::try_from(
                        app.session()?
                            .simulation
                            .architecture
                            .as_ref()
                            .and_then(|architecture| architecture.original.as_ref())
                            .context("package lot geometry absent")?
                            .size,
                    )? - 1;
                    let world = &app.session()?.simulation.vm.world;
                    ensure!(
                        world.objects[&added[1]]
                            .container
                            .map(|container| container.0)
                            == Some(added[0]),
                        "charger is not contained by the new table"
                    );
                    ensure!(
                        world.objects[&added[0]].slots.contains(&Some(added[1])),
                        "table does not own the charger slot"
                    );
                    for index in [0, 2, 3] {
                        ensure!(
                            world.objects[&added[index]].container.is_none()
                                && world.positions.get(&added[index]).is_some_and(|position| {
                                    position.x >= 16
                                        && position.y >= 16
                                        && (position.x >> 4) < edge
                                        && (position.y >> 4) < edge
                                        && (1..=2).contains(&position.level)
                                }),
                            "package content is not placed on the lot"
                        );
                    }
                    ensure!(
                        world.objects[&package.object].attributes[..5]
                            .iter()
                            .all(|value| *value == 1),
                        "Open changed the separate uncollected-content flags"
                    );
                    report["effects"] = json!({"package_opened":true,"created_objects":added,
                        "package_dialog":true,"tick":app.session()?.simulation.vm.tick});
                    effects_seen = true;
                    stage = "action_return";
                }
                if let Some(serial) = submitted_serial {
                    let actions = app
                        .session()?
                        .simulation
                        .vm
                        .world
                        .actions
                        .as_ref()
                        .context("action owner disappeared")?;
                    let selected = actions
                        .actors
                        .get(&actor.object)
                        .context("actor action state disappeared")?;
                    if let Some(previous) = selected
                        .previous
                        .as_ref()
                        .filter(|action| action.id == serial)
                    {
                        report["action_return"] =
                            json!({"action":previous,"tick":app.session()?.simulation.vm.tick});
                        ensure!(
                            previous.flags & 32 != 0,
                            "original Open action returned false"
                        );
                        ensure!(effects_seen, "Open returned without the required contents");
                        ensure!(
                            !selected
                                .active
                                .iter()
                                .chain(selected.queue.pending.iter())
                                .any(|action| action.id == serial),
                            "completed Open remains active or queued"
                        );
                        app.execute(&Command::Playing { value: false }, &limits)?;
                        let counts: Vec<_> = CONTENTS
                            .iter()
                            .map(|guid| instances(app, *guid))
                            .collect::<Result<_>>()?;
                        app.execute(
                            &Command::Object {
                                identity: package.clone(),
                            },
                            &limits,
                        )?;
                        let menu = app.menu().context("opened package menu absent")?;
                        ensure!(
                            menu.actions()
                                .iter()
                                .all(|action| action.routine != 4097 || action.flags & 16 != 0),
                            "opened package still offers Open"
                        );
                        report["after_open_menu"] = json!(
                            menu.actions()
                                .iter()
                                .map(|action| app.viewer.action_label(action))
                                .collect::<Vec<_>>()
                        );
                        app.execute(&Command::CloseMenu, &limits)?;
                        ensure!(
                            CONTENTS
                                .iter()
                                .map(|guid| instances(app, *guid))
                                .collect::<Result<Vec<_>>>()?
                                == counts,
                            "reopening the package menu duplicated contents"
                        );
                        let checkpoint = app.checkpoint()?;
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(output.join("after-open.private.json"))?
                            .write_all(&checkpoint)?;
                        report["after_open_checkpoint_sha256"] = json!(engine::digest(&checkpoint));
                        restore_probe.finish()?;
                        stage = "opened_action_completed";
                        return Ok(());
                    }
                }
            }
            if frame % 128 == 0 {
                let observation = json!({"frame":frame,"stage":stage,
                    "tick":app.session()?.simulation.vm.tick,"calendar":calendar(app)?,"salesman":observed_salesman,
                    "packages":packages.len(),"runtime_seconds":runtime.elapsed().as_secs_f64()});
                println!("{observation}");
                observations.push(observation);
            }
            advance_observed(
                app,
                &limits,
                &mut dialogs,
                &mut package_dialog,
                actor.object,
                if restore_probe.pending() {
                    selected_package
                        .as_ref()
                        .zip(submitted_serial)
                        .map(|(package, serial)| {
                            (
                                &mut restore_probe,
                                output.as_path(),
                                &actor,
                                package,
                                opening_restore::Continuation {
                                    serial,
                                    before_contents: before_contents.clone(),
                                    package_dialog_seen: false,
                                },
                            )
                        })
                } else {
                    None
                },
                &mut |_, _| Ok(()),
            )?;
        }
        anyhow::bail!("package frame budget exhausted")
    })();
    report["stage"] = json!(stage);
    report["opening_restore"] = restore_probe.report();
    report["success"] = json!(result.is_ok());
    report["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    report["dialogs"] = json!(dialogs);
    report["observations"] = json!(observations);
    report["offers"] = json!(offers);
    if let Some(app) = &app {
        if let Ok(session) = app.session() {
            let vm = &session.simulation.vm;
            report["input_boundary"] = json!({"people_panel_visible":app.viewer.people_panel_visible(),
                "presentation_pending":app.viewer.presentation_pending(),"modal":vm.world.dialogs.modal,
                "selected_resident":session.simulation.native_selected_person().ok().flatten(),
                "package_objects":vm.world.objects.iter().filter_map(|(identifier,object)| {
                    object.definition.as_ref().is_some_and(|definition| definition.guid == PACKAGE).then(||
                        json!({"object":identifier,"attributes":object.attributes,"position":vm.world.positions.get(identifier)}))
                }).collect::<Vec<_>>()});
            report["selected_actions"] = json!(
                session
                    .simulation
                    .native_selected_person()
                    .ok()
                    .flatten()
                    .and_then(|actor| vm.world.actions.as_ref()?.actors.get(&actor))
            );
            report["delivery_controllers"] = json!(vm.world.objects.iter().filter_map(|(identifier, object)| {
                let definition = object.definition.as_ref()?;
                definition.source.ends_with("::NPC_Controller_Spellbound.iff").then(|| {
                    let thread = vm.object_threads.get(identifier).and_then(|thread| vm.threads.get(thread));
                    json!({"object":identifier,"attributes":object.attributes,
                        "frames":thread.map(|thread| thread.stack.iter().map(|frame| &frame.location).collect::<Vec<_>>()),
                        "status":thread.map(|thread| format!("{:?}",thread.status))})
                })
            }).collect::<Vec<_>>());
            report["final_calendar"] = json!(calendar(app).ok());
        }
    }
    if let Err(error) = &result {
        report["error"] = json!(format!("{error:#}"));
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("receipt.json"))?
        .write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!("package probe stage={stage}; success={}", result.is_ok());
    result
}
