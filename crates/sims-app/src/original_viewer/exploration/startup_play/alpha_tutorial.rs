use super::*;
use serde::{Deserialize, Serialize};
use sims_render::dialog::client::live_panel::{self, Category, Detail, Popup};
use sims_vm::dialog::{Button, Record};
use std::collections::{BTreeMap, BTreeSet};

const SOURCE: &str = "GameData_ALL_Recursive/Objects/Objects.far::Tutorial.iff";
const HOUSE: &str = "UserData/Houses/House07.iff";
const CHAPTERS: [u16; 6] = [4116, 4125, 4126, 4135, 4136, 4137];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    schema: u32,
    house_sha256: String,
    tutorial_sha256: String,
    controller: Identity,
    dialogs: Vec<DialogPin>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DialogPin {
    source: String,
    source_sha256: String,
    caller: Identity,
    origin: sims_vm::Location,
    operands: [u8; 8],
    title: String,
    body_sha256: String,
    buttons: Vec<Button>,
    result: Option<u32>,
}

impl DialogPin {
    fn matches(&self, session: &Session, record: &Record) -> bool {
        record.status == 1
            && record.source == self.source
            && record.origin == self.origin
            && record.operands == self.operands
            && record.title == self.title
            && record.buttons == self.buttons
            && engine::digest(record.body.as_bytes()) == self.body_sha256
            && Identity::get(session, record.caller).ok().as_ref() == Some(&self.caller)
            && session
                .program
                .modules
                .get(&record.source)
                .is_some_and(|module| module.content_hash == self.source_sha256)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersonPin {
    actor: Identity,
    widget: live_panel::Identity,
}

impl PersonPin {
    fn matches(&self, app: &Application) -> Result<bool> {
        let session = app.session()?;
        self.actor.check(session)?;
        Ok(self.actor.object == self.widget.object
            && live_panel::selected_identity(session)? == Some(self.widget)
            && app
                .viewer
                .client
                .as_ref()
                .and_then(|client| client.live_panel())
                .and_then(|panel| panel.person())
                .is_some_and(|person| person.identity() == self.widget))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PointerInput {
    Motion,
    Down,
    Up,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "predicate", rename_all = "snake_case", deny_unknown_fields)]
enum Predicate {
    Dialog {
        pin: usize,
    },
    TutorialFrame {
        routine: u16,
        instruction: usize,
    },
    TutorialCall {
        routine: u16,
        instruction: usize,
    },
    CameraWatch {
        query: [i16; 2],
    },
    PersonPanel {
        person: PersonPin,
        category: Category,
        popup: Option<Popup>,
    },
    Milestone {
        milestone: Milestone,
    },
    Complete {},
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Await {
        expected: Predicate,
        max_frames: u32,
        max_ticks: u64,
    },
    Input {
        command: Command,
    },
    Respond {
        pin: usize,
    },
    Pan {
        direction: [i32; 2],
    },
    RightButton {
        down: bool,
        pixel: [f32; 2],
    },
    RightResident {
        actor: Identity,
    },
    ClickVisible {
        target: Identity,
    },
    Motion {
        pixel: [f32; 2],
    },
    MotiveDetails {
        index: usize,
    },
    CloseMotiveDetails {},
    PersonCategory {
        person: PersonPin,
        category: Category,
    },
    PersonDetails {
        person: PersonPin,
        detail: Detail,
    },
    ClosePersonDetails {
        person: PersonPin,
        expected: Popup,
    },
    Space {},
    GameOptions {},
    SaveCompleted {},
    PointerGround {
        position: [i32; 2],
        input: PointerInput,
        expected: Option<Identity>,
    },
    Ground {
        position: [i32; 2],
    },
}

#[derive(Default)]
struct Evidence {
    events: Vec<Value>,
    chapters: BTreeSet<u16>,
    dialogs: BTreeSet<u64>,
    final_response: bool,
    managed_save: Option<Value>,
    frames: u32,
    completed_steps: usize,
}

fn private_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        relative.starts_with("cache/playable-alpha")
            && relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
        "tutorial paths must remain beneath cache/playable-alpha"
    );
    let parent = root
        .join(relative)
        .parent()
        .context("tutorial path parent absent")?
        .canonicalize()?;
    ensure!(
        parent.starts_with(root.join("cache/playable-alpha").canonicalize()?),
        "tutorial path escapes private evidence"
    );
    Ok(parent.join(relative.file_name().context("tutorial filename absent")?))
}

fn observe(app: &Application, scenario: &Scenario, evidence: &mut Evidence) -> Result<()> {
    app.check_errors()?;
    let session = app.session()?;
    for record in session
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .filter(|record| record.status == 1)
    {
        if evidence.dialogs.insert(record.id) {
            let pin = scenario
                .dialogs
                .iter()
                .position(|pin| pin.matches(session, record));
            evidence.events.push(
                json!({"kind":"original_dialog","tick":session.simulation.vm.tick,
                "pin":pin,"record":record,"response_sent":false}),
            );
            if record.source == SOURCE && pin.is_some() && CHAPTERS.contains(&record.origin.bhav) {
                evidence.chapters.insert(record.origin.bhav);
            }
        }
    }
    Ok(())
}

fn snapshot(app: &Application, controller: &Identity) -> Result<Value> {
    let session = app.session()?;
    let vm = &session.simulation.vm;
    let threads: Vec<_> = vm
        .threads
        .iter()
        .filter(|(_, thread)| {
            thread
                .stack
                .iter()
                .any(|frame| frame.caller == controller.object && frame.code_owner == SOURCE)
        })
        .map(|(identity, thread)| json!({"thread":identity,"state":thread}))
        .collect();
    let people: Vec<_> = vm.world.objects.iter().filter_map(|(identity, object)| {
        let person = object.person.as_ref()?;
        Some(json!({"identity":Identity::get(session,*identity).ok(),"static_motives":person.get(&29),
            "autonomy":person.get(&36),"motives":object.motives,"position":vm.world.positions.get(identity)}))
    }).collect();
    Ok(
        json!({"tick":vm.tick,"playing":app.viewer.playing,"pending":app.viewer.presentation_pending(),
        "calendar":calendar(app)?,"controller":vm.world.objects.get(&controller.object),
        "native_clock":session.host.presentation.clock,
        "native_transport":session.host.presentation.transport,
        "person_panel":panel_snapshot(app),"buy_catalog":catalog_snapshot(app),
        "move_cursor":session.host.presentation.move_tool.as_ref().map(|state|json!({
            "held":state.tool.held,"payload":state.tool.payload,"tool":state.tool.tool})),
        "controller_lifetime":Identity::get(session,controller.object).ok(),"threads":threads,
        "ui_owner":vm.world.ui_owner,"ui_wait":session.host.presentation.ui_wait,
        "dialogs":vm.world.dialogs,"people":people,"selected_resident":app.viewer.resident,
        "camera":OriginalHouse::native_camera(session).ok().map(|camera|json!({"rotation":camera.rotation,
            "zoom":camera.zoom,"center":[camera.center.x,camera.center.y,camera.center.z]})),
        "object_actions":app.menu().map(|menu|menu.actions()),
        "ground_actions":session.host.presentation.ground_input.as_ref().filter(|menu|menu.ready()).map(|menu|menu.actions())}),
    )
}

fn panel_snapshot(app: &Application) -> Option<Value> {
    let panel = app.viewer.client.as_ref()?.live_panel()?;
    Some(
        json!({"person":panel.person(),"category":panel.category(),"shown":panel.shown(),
        "popup":panel.retained_popup(),"texts":panel.texts()}),
    )
}

fn catalog_snapshot(app: &Application) -> Option<Value> {
    let catalog = app.viewer.device.as_ref()?.panel_widgets()?.catalog()?;
    let view = catalog.view()?;
    let products: Vec<_> = view
        .products
        .iter()
        .filter_map(|index| {
            let product = catalog.product(*index)?;
            Some(
                json!({"index":index,"definition":product.definition,"source":product.source,
            "price":product.fields.price,"name":product.display_name()}),
            )
        })
        .collect();
    Some(json!({"revision":catalog.revision(),"view":view,"products":products}))
}

fn completion_facts(app: &Application, scenario: &Scenario) -> Result<(bool, bool, bool, bool)> {
    let session = app.session()?;
    let vm = &session.simulation.vm;
    let controller_removed = !vm.world.objects.contains_key(&scenario.controller.object)
        && Identity::get(session, scenario.controller.object).is_err();
    let owner_released = vm.world.ui_owner.owner != Some(scenario.controller.object);
    let no_script = !vm.threads.values().any(|thread| {
        thread
            .stack
            .iter()
            .any(|frame| frame.caller == scenario.controller.object && frame.code_owner == SOURCE)
    });
    let residents = &session
        .house
        .household
        .as_ref()
        .context("tutorial household absent")?
        .residents;
    let motives_restored = residents_cleanup_restored(residents, |resident| {
        resident_object(session, resident)
            .and_then(|identity| vm.world.objects.get(&identity))
            .and_then(|object| object.person.as_ref())
    });
    Ok((
        controller_removed,
        owner_released,
        no_script,
        motives_restored,
    ))
}

fn residents_cleanup_restored<'person>(
    residents: &[sims_content::saves::Resident],
    mut resolve: impl FnMut(&sims_content::saves::Resident) -> Option<&'person BTreeMap<i16, i16>>,
) -> bool {
    !residents.is_empty()
        && residents.iter().all(|resident| {
            resolve(resident)
                .is_some_and(|person| person.get(&29) == Some(&0) && person.get(&36) == Some(&50))
        })
}

fn complete(
    chapters: &BTreeSet<u16>,
    final_response: bool,
    facts: (bool, bool, bool, bool),
) -> bool {
    CHAPTERS.iter().all(|chapter| chapters.contains(chapter))
        && final_response
        && facts == (true, true, true, true)
}

fn reached(
    app: &Application,
    scenario: &Scenario,
    evidence: &Evidence,
    predicate: &Predicate,
    start: u64,
) -> Result<bool> {
    let session = app.session()?;
    let vm = &session.simulation.vm;
    Ok(match predicate {
        Predicate::Dialog { pin } => {
            let pin = scenario
                .dialogs
                .get(*pin)
                .context("tutorial dialog pin absent")?;
            vm.world
                .dialogs
                .records
                .iter()
                .any(|record| pin.matches(session, record))
        }
        Predicate::TutorialFrame {
            routine,
            instruction,
        } => vm.threads.values().any(|thread| {
            thread.stack.last().is_some_and(|frame| {
                frame.caller == scenario.controller.object
                    && frame.location.source == SOURCE
                    && frame.location.bhav == *routine
                    && frame.location.instruction == *instruction
            })
        }),
        Predicate::TutorialCall {
            routine,
            instruction,
        } => vm.threads.values().any(|thread| {
            thread.stack.iter().any(|frame| {
                frame.caller == scenario.controller.object
                    && frame.location.source == SOURCE
                    && frame.location.bhav == *routine
                    && frame.location.instruction == *instruction
            })
        }),
        Predicate::CameraWatch { query } => {
            let wait = session.host.presentation.ui_wait.as_ref();
            vm.world.ui_owner.owner == Some(scenario.controller.object)
                && vm
                    .world
                    .objects
                    .get(&scenario.controller.object)
                    .is_some_and(|object| object.temps[..2] == *query)
                && wait.is_some_and(|wait| !wait.pending() && wait.baseline() != -1)
                && session.simulation.ui_wait_callback.is_none()
                && !app.viewer.presentation_pending()
        }
        Predicate::Milestone { milestone } => milestone.reached(app, start)?,
        Predicate::PersonPanel {
            person,
            category,
            popup,
        } => {
            person.matches(app)?
                && app
                    .viewer
                    .client
                    .as_ref()
                    .and_then(|client| client.live_panel())
                    .is_some_and(|panel| {
                        panel.shown()
                            && panel.category_selected(*category)
                            && panel.retained_popup() == *popup
                    })
        }
        Predicate::Complete {} => complete(
            &evidence.chapters,
            evidence.final_response,
            completion_facts(app, scenario)?,
        ),
    })
}

fn validate_content(app: &Application, scenario: &Scenario) -> Result<()> {
    ensure!(
        scenario.schema == 1 && !scenario.steps.is_empty() && scenario.steps.len() <= 512,
        "invalid tutorial scenario"
    );
    let session = app.session()?;
    scenario.controller.check(session)?;
    ensure!(
        session.house.source == HOUSE,
        "tutorial requires fresh original Newbie input"
    );
    ensure!(
        session.program.modules[SOURCE].content_hash == scenario.tutorial_sha256,
        "original tutorial content changed"
    );
    ensure!(
        session.simulation.vm.world.objects[&scenario.controller.object]
            .definition
            .as_ref()
            .is_some_and(|definition| definition.source == SOURCE),
        "pinned controller is not original tutorial"
    );
    let routines = &session.program.modules[SOURCE].routines;
    for (routine, instruction, opcode, operands) in [
        (4097, 11, 4139, [1, 0, 0, 0, 0, 0, 0, 0]),
        (4097, 13, 4144, [0; 8]),
        (4096, 4, 10, [1, 0, 0, 0, 0, 0, 0, 0]),
        (4096, 5, 18, [0; 8]),
        (4137, 5, 36, [0, 10, 54, 0, 0, 4, 0, 6]),
    ] {
        let actual = routines
            .get(&routine)
            .and_then(|routine| routine.body.instructions.get(instruction))
            .context("tutorial terminal content missing")?;
        ensure!(
            actual.opcode == opcode && actual.operands == operands,
            "tutorial terminal instruction changed"
        );
    }
    ensure!(
        matches!(
            scenario.steps.last(),
            Some(Step::Await {
                expected: Predicate::Complete {},
                ..
            })
        ),
        "tutorial plan must end with actual completion gate"
    );
    Ok(())
}

fn step(
    app: &mut Application,
    scenario: &Scenario,
    evidence: &mut Evidence,
    input: &Step,
    limits: &Limits,
    wall: Instant,
) -> Result<()> {
    match input {
        Step::Await {
            expected,
            max_frames,
            max_ticks,
        } => {
            ensure!(
                *max_frames > 0 && *max_frames <= 10_000 && *max_ticks > 0 && *max_ticks <= 50_000,
                "invalid tutorial wait bounds"
            );
            let start = app.session()?.simulation.vm.tick;
            for frame in 0..=*max_frames {
                observe(app, scenario, evidence)?;
                if reached(app, scenario, evidence, expected, start)? {
                    evidence.events.push(json!({"kind":"predicate_reached","predicate":expected,
                        "start_tick":start,"tick":app.session()?.simulation.vm.tick,"frames":frame}));
                    return Ok(());
                }
                ensure!(
                    frame < *max_frames
                        && app.session()?.simulation.vm.tick.saturating_sub(start) < *max_ticks,
                    "tutorial bounded predicate not reached: {expected:?}; frames={frame}; ticks={}",
                    app.session()?.simulation.vm.tick.saturating_sub(start)
                );
                ensure!(
                    wall.elapsed().as_secs_f64() < limits.wall_seconds && evidence.frames < 30_000,
                    "tutorial total budget exhausted"
                );
                if let Some(modal) = app.session()?.simulation.vm.world.dialogs.modal {
                    anyhow::bail!(
                        "unplanned blocking original modal {modal}; freeze its actual response before replay"
                    );
                }
                evidence.frames += 1;
                app.execute(&Command::Advance { millis: 250 }, limits)?;
            }
        }
        Step::SaveCompleted {} => {
            ensure!(
                complete(
                    &evidence.chapters,
                    evidence.final_response,
                    completion_facts(app, scenario)?
                ),
                "completed tutorial Save requires every original completion gate"
            );
            ensure!(
                evidence.managed_save.is_none() && !app.viewer.args.save.try_exists()?,
                "refusing to overwrite a retained tutorial player save"
            );
            let path = app.viewer.args.save.clone();
            evidence
                .events
                .push(json!({"kind":"production_message","message":"Save",
                "path":path,"tick":app.session()?.simulation.vm.tick}));
            app.dispatch(Message::Save)?;
            app.drain(limits)?;
            let bytes =
                std::fs::read(&path).context("production tutorial Save produced no file")?;
            let value: Value = serde_json::from_slice(&bytes)?;
            let saved: sims_runtime::original_session::Saved =
                serde_json::from_value(value["session"].clone())?;
            ensure!(
                saved.version == 2 && saved.source == HOUSE,
                "tutorial Save did not produce a managed Newbie session"
            );
            ensure!(
                complete(
                    &evidence.chapters,
                    evidence.final_response,
                    completion_facts(app, scenario)?
                ),
                "production tutorial Save lost original completion state"
            );
            evidence.managed_save = Some(json!({"path":path,
                "sha256":engine::digest(&bytes),"bytes":bytes.len(),
                "production_message":"Save","managed_version":saved.version,
                "source":saved.source,"tick":app.session()?.simulation.vm.tick,
                "cold_load_verified":false,"continued_household_acceptance":false}));
        }
        Step::Input { command } => app.execute(command, limits)?,
        Step::Respond { pin } => {
            let pin = scenario
                .dialogs
                .get(*pin)
                .context("tutorial response pin absent")?;
            let session = app.session()?;
            let record = session
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|record| {
                    Some(record.id) == session.simulation.vm.world.dialogs.modal
                        && pin.matches(session, record)
                })
                .context("pinned original modal response is not currently offered")?
                .clone();
            let result = pin.result.context("frozen modal has no response")?;
            ensure!(
                record.buttons.iter().any(|button| button.result == result),
                "response is not an original offered button"
            );
            app.execute(
                &Command::Dialog {
                    id: record.id,
                    result,
                    body_sha256: pin.body_sha256.clone(),
                },
                limits,
            )?;
            if record.source == SOURCE
                && record.origin.bhav == 4137
                && record.origin.instruction == 5
                && result == 2
            {
                evidence.final_response = true;
            }
            evidence
                .events
                .push(json!({"kind":"original_response","record":record,"result":result}));
        }
        Step::Pan { direction } => {
            ensure!(
                direction.iter().all(|axis| (-1..=1).contains(axis)),
                "invalid tutorial pan"
            );
            app.dispatch(Message::Pan(*direction))?;
            app.drain(limits)?;
        }
        Step::RightButton { down, pixel } => {
            ensure!(
                pixel.iter().all(|axis| axis.is_finite() && *axis >= 0.),
                "invalid tutorial right-button pixel"
            );
            app.dispatch(Message::ViewportRightButton {
                down: *down,
                pixel: Some(*pixel),
            })?;
            app.drain(limits)?;
        }
        Step::ClickVisible { target } => {
            target.check(app.session()?)?;
            let scene = app
                .viewer
                .scene
                .as_ref()
                .context("tutorial viewport scene absent")?;
            evidence.events.push(json!({"kind":"visible_object_query","target":target,
                "scene_size":[scene.width,scene.height],
                "draw_bounds":crate::original_viewer::object_tests::object_bounds(scene,target.object)}));
            let pixel =
                crate::original_viewer::object_tests::object_pixel(&app.viewer, target.object)
                    .context("tutorial object has no visible pickable pixel")?;
            app.execute(
                &Command::Viewport {
                    pixel: pixel.map(f32::to_bits),
                    expected: Some(target.clone()),
                },
                limits,
            )?;
            evidence.events.push(json!({"kind":"visible_object_input","target":target,
                "pixel":pixel,"actual_pick_verified":true,"tick":app.session()?.simulation.vm.tick}));
        }
        Step::RightResident { actor } => {
            actor.check(app.session()?)?;
            let pixel =
                crate::original_viewer::object_tests::object_pixel(&app.viewer, actor.object)
                    .context("tutorial resident has no visible pickable pixel")?;
            for down in [true, false] {
                actor.check(app.session()?)?;
                ensure!(
                    app.viewer
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.pick(pixel))
                        == Some(actor.object),
                    "tutorial right-click resident hit changed"
                );
                app.dispatch(Message::ViewportRightButton {
                    down,
                    pixel: Some(pixel),
                })?;
                app.drain(limits)?;
            }
            app.execute(
                &Command::Check {
                    milestone: super::super::milestone::Milestone::Selected {
                        actor: actor.clone(),
                    },
                },
                limits,
            )?;
            evidence
                .events
                .push(json!({"kind":"right_resident_input","actor":actor,
                "pixel":pixel,"actual_pick_verified_on_down_and_up":true,
                "selected":app.viewer.resident,"tick":app.session()?.simulation.vm.tick}));
        }
        Step::Motion { pixel } => {
            ensure!(
                pixel.iter().all(|axis| axis.is_finite() && *axis >= 0.),
                "invalid tutorial motion pixel"
            );
            app.dispatch(Message::ViewportMotion {
                pixel: Some(*pixel),
                pan: [0, 0],
            })?;
            app.drain(limits)?;
        }
        Step::MotiveDetails { index } => {
            ensure!(*index < 8, "invalid tutorial motive bar");
            app.dispatch(Message::MotiveDetails(*index))?;
            app.drain(limits)?;
        }
        Step::CloseMotiveDetails {} => {
            ensure!(
                app.viewer
                    .client
                    .as_ref()
                    .and_then(|client| client.live_panel())
                    .is_some_and(|panel| panel.popup().is_some()),
                "no actual motive popup to close"
            );
            app.dispatch(Message::CloseMotiveDetails)?;
            app.drain(limits)?;
        }
        Step::PersonCategory { person, category } => {
            ensure!(
                person.matches(app)?,
                "tutorial person category owner changed"
            );
            app.dispatch(match category {
                Category::Needs => Message::Needs,
                Category::Personality => Message::Personality(person.widget),
                Category::Relationships => Message::Relationships(person.widget),
                Category::Job => Message::Job(person.widget),
            })?;
            app.drain(limits)?;
            evidence.events.push(
                json!({"kind":"person_panel_input","panel":panel_snapshot(app),
                "tick":app.session()?.simulation.vm.tick}),
            );
        }
        Step::PersonDetails { person, detail } => {
            ensure!(
                person.matches(app)?
                    && app
                        .viewer
                        .client
                        .as_ref()
                        .and_then(|client| client.live_panel())
                        .is_some_and(|panel| panel.detail_available(*detail)),
                "tutorial person detail is not an actual available widget"
            );
            app.dispatch(Message::PersonDetails(person.widget, *detail))?;
            app.drain(limits)?;
            evidence.events.push(
                json!({"kind":"person_panel_input","panel":panel_snapshot(app),
                "tick":app.session()?.simulation.vm.tick}),
            );
        }
        Step::ClosePersonDetails { person, expected } => {
            ensure!(
                person.matches(app)?
                    && app
                        .viewer
                        .client
                        .as_ref()
                        .and_then(|client| client.live_panel())
                        .is_some_and(|panel| panel.retained_popup() == Some(*expected)),
                "tutorial person popup lifetime changed"
            );
            app.dispatch(Message::ClosePersonDetails(person.widget, *expected))?;
            app.drain(limits)?;
            evidence.events.push(
                json!({"kind":"person_panel_input","panel":panel_snapshot(app),
                "tick":app.session()?.simulation.vm.tick}),
            );
        }
        Step::Space {} => {
            app.dispatch(Message::DialogKey(32))?;
            app.drain(limits)?;
        }
        Step::GameOptions {} => {
            app.dispatch(Message::GameOptions)?;
            app.drain(limits)?;
        }
        Step::PointerGround {
            position,
            input,
            expected,
        } => {
            ensure!(
                app.viewer.current_move(),
                "tutorial Move pointer is not active"
            );
            let session = app.session()?;
            let mut pixel = OriginalHouse::native_camera(session)?
                .screen(OriginalHouse::ground_point(
                    session,
                    *position,
                    app.viewer.floor,
                )?)
                .to_array();
            let offset = session
                .host
                .presentation
                .move_tool
                .as_ref()
                .context("tutorial Move owner absent")?
                .tool
                .tool
                .position;
            pixel[0] += offset[1] as f32;
            pixel[1] += offset[0] as f32;
            let scene = app
                .viewer
                .scene
                .as_ref()
                .context("tutorial Move viewport absent")?;
            ensure!(
                pixel.iter().all(|axis| axis.is_finite() && *axis >= 1.)
                    && pixel[0] < scene.width as f32 - 1.
                    && pixel[1] < scene.height as f32 - 1.,
                "tutorial Move ground is outside actual viewport"
            );
            ensure!(
                move_ground(session, pixel)?.map(|axis| axis.div_euclid(16))
                    == position.map(|axis| axis.div_euclid(16)),
                "tutorial Move ground projection changed"
            );
            if let Some(identity) = expected {
                identity.check(session)?;
            }
            ensure!(
                scene.pick(pixel) == expected.as_ref().map(|identity| identity.object),
                "tutorial Move viewport hit changed"
            );
            let command = match input {
                PointerInput::Motion => Command::ViewportMotion {
                    pixel: pixel.map(f32::to_bits),
                },
                PointerInput::Down | PointerInput::Up => Command::ViewportButton {
                    down: matches!(input, PointerInput::Down),
                    pixel: pixel.map(f32::to_bits),
                    expected: expected.clone(),
                },
            };
            app.execute(&command, limits)?;
            evidence.events.push(json!({"kind":"move_ground_input","position":position,
                "pixel":pixel,"input":input,"expected":expected,"tick":app.session()?.simulation.vm.tick}));
        }
        Step::Ground { position } => {
            let session = app.session()?;
            let camera = OriginalHouse::native_camera(session)?;
            let pixel = camera
                .screen(OriginalHouse::ground_point(
                    session,
                    *position,
                    app.viewer.floor,
                )?)
                .to_array();
            let lot = session
                .simulation
                .architecture
                .as_ref()
                .and_then(|architecture| architecture.original.as_ref())
                .context("tutorial ground absent")?;
            ensure!(
                camera.pick_original_ground_in(
                    lot,
                    sims_render::original_lot_size(&session.simulation)?,
                    pixel
                )? == Some(*position),
                "tutorial ground projection changed"
            );
            app.execute(
                &Command::Viewport {
                    pixel: pixel.map(f32::to_bits),
                    expected: None,
                },
                limits,
            )?;
            evidence
                .events
                .push(json!({"kind":"ground_input","position":position,"pixel":pixel}));
        }
    }
    observe(app, scenario, evidence)
}

#[test]
#[ignore = "private original assets; frozen SIMS_ALPHA_TUTORIAL_SCENARIO and fresh SIMS_ALPHA_TUTORIAL_OUTPUT"]
fn original_newbie_tutorial_completes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("tutorial worker panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let scenario_path = private_path(
        &root,
        Path::new(&std::env::var("SIMS_ALPHA_TUTORIAL_SCENARIO")?),
    )?;
    let bytes = std::fs::read(&scenario_path)?;
    let scenario: Scenario = serde_json::from_slice(&bytes)?;
    let output = private_path(
        &root,
        Path::new(&std::env::var("SIMS_ALPHA_TUTORIAL_OUTPUT")?),
    )?;
    std::fs::create_dir(&output).context("refusing to overwrite retained tutorial evidence")?;
    std::fs::write(output.join("scenario.json"), &bytes)?;
    let assets = root.join("game-data/cabinet-extracted");
    validate_output(&assets, &output.join("player-save.json"))?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        HOUSE,
        "--paused",
        "--inferred-routing",
        "--buy-preview",
        "--neighborhood-policy",
        "installed-houses-ascending-v1",
        "--root",
        assets.to_str().unwrap(),
        "--save",
        output.join("player-save.json").to_str().unwrap(),
    ]);
    let limits = Limits {
        commands: 30_000,
        completed_ticks: 100_000,
        service_polls: 20_000,
        wall_seconds: 900.,
    };
    let wall = Instant::now();
    let mut app = None;
    let mut evidence = Evidence::default();
    let mut active_step = None;
    let result = (|| -> Result<()> {
        ensure!(
            engine::digest(&std::fs::read(assets.join(HOUSE))?) == scenario.house_sha256,
            "original Newbie starting input changed"
        );
        let ready = Arc::new(Ready::load(&args)?);
        app = Some(Application::install_ready(
            &args,
            ready,
            Instant::now(),
            &limits,
        )?);
        let app = app.as_mut().unwrap();
        validate_content(app, &scenario)?;
        observe(app, &scenario, &mut evidence)?;
        for (index, input) in scenario.steps.iter().enumerate() {
            active_step = Some(index);
            ensure!(
                wall.elapsed().as_secs_f64() < limits.wall_seconds,
                "tutorial wall limit"
            );
            eprintln!("tutorial step {index}: {input:?}");
            evidence.events.push(json!({"kind":"frozen_input","index":index,"input":input,"tick":app.session()?.simulation.vm.tick}));
            step(app, &scenario, &mut evidence, input, &limits, wall)
                .with_context(|| format!("tutorial step {index}"))?;
            evidence.completed_steps += 1;
        }
        ensure!(
            complete(
                &evidence.chapters,
                evidence.final_response,
                completion_facts(app, &scenario)?
            ),
            "original tutorial completion not established"
        );
        Ok(())
    })();
    if let Some(app) = app.as_ref() {
        match app.checkpoint() {
            Ok(checkpoint) => std::fs::write(
                output.join(if result.is_ok() {
                    "completed-checkpoint.json"
                } else {
                    "failure-checkpoint.json"
                }),
                checkpoint,
            )?,
            Err(error) => evidence
                .events
                .push(json!({"kind":"checkpoint_failed","error":format!("{error:#}")})),
        }
        if result.is_err() {
            let screenshot = output.join("failure-frame.png");
            match app.screenshot(&screenshot).and_then(|_| {
                Ok(engine::digest(
                    &std::fs::read(&screenshot).context("reading tutorial failure screenshot")?,
                ))
            }) {
                Ok(sha256) => evidence.events.push(json!({"kind":"failure_screenshot",
                    "path":screenshot,"sha256":sha256})),
                Err(error) => evidence
                    .events
                    .push(json!({"kind":"failure_screenshot_error",
                    "error":format!("{error:#}")})),
            }
        }
    }
    write_json(
        &output.join("tutorial-report.json"),
        &json!({"schema":1,"tutorial_completed":result.is_ok(),
        "whole_alpha_complete":false,"fresh_original_input":HOUSE,"scenario_sha256":engine::digest(&bytes),
        "error":result.as_ref().err().map(|error|format!("{error:#}")),"active_step":active_step,
        "completed_steps":evidence.completed_steps,"frames":evidence.frames,"chapters":evidence.chapters,
        "final_response":evidence.final_response,"managed_save":evidence.managed_save,
        "wall_seconds":wall.elapsed().as_secs_f64(),
        "state":app.as_ref().map(|app|snapshot(app,&scenario.controller)).transpose()?,"events":evidence.events}),
    )?;
    result
}

#[test]
fn tutorial_completion_requires_original_final_response_every_chapter_and_cleanup() {
    let chapters: BTreeSet<_> = CHAPTERS.into_iter().collect();
    assert!(complete(&chapters, true, (true, true, true, true)));
    assert!(!complete(&chapters, false, (true, true, true, true)));
    for chapter in CHAPTERS {
        let mut missing = chapters.clone();
        missing.remove(&chapter);
        assert!(!complete(&missing, true, (true, true, true, true)));
    }
    for facts in [
        (false, true, true, true),
        (true, false, true, true),
        (true, true, false, true),
        (true, true, true, false),
    ] {
        assert!(!complete(&chapters, true, facts));
    }
}

#[test]
fn tutorial_cleanup_requires_every_original_resident_unique_live_identity_and_exact_fields() {
    use sims_content::saves::Resident;
    use sims_vm::{
        Definition, Object, World,
        person::{Neighbor, WorldState},
    };

    let residents = [
        Resident {
            guid: 844195932,
            neighbor: 47,
            object: 137,
            name: "Betty".into(),
            character_source: "UserData/Characters/User00001.iff".into(),
            body_strings: Vec::new(),
        },
        Resident {
            guid: 850010198,
            neighbor: 48,
            object: 105,
            name: "Bob".into(),
            character_source: "UserData/Characters/User00000.iff".into(),
            body_strings: Vec::new(),
        },
    ];
    let mut world = World::default();
    let mut people = WorldState::default();
    people.controlled_family = 1;
    people.families.insert(1, vec![47, 48]);
    for (resident, identity) in residents.iter().zip([134, 105]) {
        people.neighbors.insert(
            resident.neighbor,
            Neighbor {
                guid: resident.guid,
                ..Default::default()
            },
        );
        world.objects.insert(
            identity,
            Object {
                definition: Some(Definition {
                    source: resident.character_source.clone(),
                    object: 128,
                    guid: resident.guid,
                    slots: 0,
                }),
                person: Some(BTreeMap::from([(29, 0), (36, 50)])),
                ..Default::default()
            },
        );
    }
    world.person = Some(people);
    let restored = |world: &World, residents: &[Resident]| {
        residents_cleanup_restored(residents, |resident| {
            resident_identity::object(world, resident)
                .and_then(|identity| world.objects.get(&identity))
                .and_then(|object| object.person.as_ref())
        })
    };
    assert!(restored(&world, &residents));
    assert!(!restored(&world, &[]));
    let mut reused = world.objects[&134].clone();
    reused.definition.as_mut().unwrap().guid = 999;
    world.objects.insert(137, reused);
    assert!(restored(&world, &residents));
    let missing = world.objects.remove(&105).unwrap();
    assert!(!restored(&world, &residents));
    world.objects.insert(105, missing);
    world.objects.insert(136, world.objects[&134].clone());
    assert!(!restored(&world, &residents));
    world.objects.remove(&136);
    for (field, wrong) in [(29, 1), (36, 49)] {
        let person = world
            .objects
            .get_mut(&134)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        let expected = person.insert(field, wrong).unwrap();
        assert!(!restored(&world, &residents));
        world
            .objects
            .get_mut(&134)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .remove(&field);
        assert!(!restored(&world, &residents));
        world
            .objects
            .get_mut(&134)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(field, expected);
    }
    world
        .objects
        .get_mut(&134)
        .unwrap()
        .definition
        .as_mut()
        .unwrap()
        .guid = 999;
    assert!(!restored(&world, &residents));
    world
        .objects
        .get_mut(&134)
        .unwrap()
        .definition
        .as_mut()
        .unwrap()
        .guid = residents[0].guid;
    world
        .objects
        .get_mut(&134)
        .unwrap()
        .definition
        .as_mut()
        .unwrap()
        .source = "UserData2/Characters/User00001.iff".into();
    assert!(!restored(&world, &residents));
}

#[test]
fn tutorial_steps_cannot_patch_flags_motives_or_reset() {
    for rejected in [
        json!({"step":"reset"}),
        json!({"step":"teleport"}),
        json!({"step":"motive_replenish"}),
        json!({"step":"tutorial_flag","value":false}),
        json!({"step":"space","key":27}),
        json!({"step":"game_options","reset":true}),
        json!({"step":"save_completed","tutorial_flag":false}),
        json!({"step":"save_completed","path":"elsewhere.json"}),
        json!({"step":"close_motive_details","tutorial_flag":false}),
        json!({"step":"await","expected":{"predicate":"complete","tutorial_flag":false},
            "max_frames":1,"max_ticks":1}),
    ] {
        assert!(serde_json::from_value::<Step>(rejected).is_err());
    }
}

#[test]
fn tutorial_typed_widget_inputs_preserve_original_identity_and_reject_unknown_fields() -> Result<()>
{
    let person = json!({"actor":{"object":105,"handle":77},"widget":{
        "object":105,"guid":850010198,"cache_index":256,"neighbor":48}});
    for input in [
        json!({"step":"person_category","person":person,"category":"Personality"}),
        json!({"step":"person_details","person":person,"detail":{"Personality":0}}),
        json!({"step":"person_details","person":person,"detail":{"Relationship":{
            "neighbor":47,"guid":844195932,"cache_index":266}}}),
        json!({"step":"person_details","person":person,"detail":"Job"}),
        json!({"step":"person_details","person":person,"detail":{"Skill":0}}),
        json!({"step":"close_person_details","person":person,"expected":{"Personality":0}}),
        json!({"step":"pointer_ground","position":[440,360],"input":"motion","expected":null}),
        json!({"step":"close_motive_details"}),
        json!({"step":"space"}),
        json!({"step":"right_resident","actor":{"object":105,"handle":77}}),
        json!({"step":"click_visible","target":{"object":105,"handle":77}}),
        json!({"step":"game_options"}),
        json!({"step":"save_completed"}),
        json!({"step":"await","expected":{"predicate":"complete"},"max_frames":1,"max_ticks":1}),
    ] {
        let parsed: Step = serde_json::from_value(input.clone())?;
        ensure!(serde_json::to_value(parsed)? == input);
        let mut unknown = input;
        unknown["tutorial_flag"] = json!(false);
        ensure!(serde_json::from_value::<Step>(unknown).is_err());
    }
    let complete = json!({"predicate":"complete"});
    let parsed: Predicate = serde_json::from_value(complete.clone())?;
    ensure!(serde_json::to_value(parsed)? == complete);
    ensure!(
        serde_json::from_value::<Predicate>(json!({"predicate":"complete","reset":true})).is_err()
    );
    Ok(())
}
