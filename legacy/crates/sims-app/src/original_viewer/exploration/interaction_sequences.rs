//! Original-script interaction milestones through independent research saves.
//! Historical inferred-map fixtures remain explicitly diagnostic; never retag
//! their routing policy or synthesize native map ownership.
use super::*;
use clap::Parser;

#[test]
#[ignore = "private original assets; fresh SIMS_INTERACTION_MILESTONES_OUTPUT, headless checks only"]
fn shower_use_completion_and_active_cancel_restore_at_milestones() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(shower_branches)?
        .join()
        .expect("interaction milestone worker panicked")
}

fn output() -> Result<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_INTERACTION_MILESTONES_OUTPUT")?);
    ensure!(
        output.starts_with(repo.join("cache")),
        "research output must stay in cache"
    );
    std::fs::create_dir(&output)?;
    Ok(output)
}
pub(super) fn load(source: &str, output: &Path) -> Result<Application> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = repo.join(source).canonicalize()?;
    ensure!(
        source.starts_with(repo.join("cache")),
        "research source must stay in cache"
    );
    let bytes = std::fs::read(&source)?;
    let input: Value = serde_json::from_slice(&bytes)?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        input["session"]["source"]
            .as_str()
            .context("source absent")?,
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        output.join("unused-player-save.json").to_str().unwrap(),
    ]);
    let ready = Ready::load(&args)?;
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &bytes,
    )?;
    app.execute(&Command::Playing { value: false }, &Limits::default())?;
    app.drain(&Limits::default())?;
    let native = app
        .session()?
        .simulation
        .native_lot
        .as_ref()
        .context("native lot absent")?;
    write_json(
        &output.join("profile.json"),
        &json!({
            "source": source, "source_sha256": engine::digest(&bytes),
            "routing_policy":native.routing_policy,
            "map_owner":app.session()?.simulation.vm.world.map_size,
            "strict_native_parity_ticks":0, "original_engine_executed":false,
            "player_save_accessed":false,
        }),
    )?;
    std::fs::write(output.join("initial.json"), app.checkpoint()?)?;
    Ok(app)
}
pub(super) fn wait(milestone: Milestone, max_ticks: u64, frame_millis: u32) -> Command {
    Command::Wait {
        milestone,
        max_ticks,
        max_frames: (max_ticks * 10) as u32,
        frame_millis,
    }
}
fn shower_branches() -> Result<()> {
    let out = output()?;
    let base = load(
        "cache/gameplay-exploration-domestic-ready-newbie-20261023/initial.json",
        &out,
    )?;
    let actor = Identity::get(base.session()?, 105)?;
    let shower = Identity::get(base.session()?, 107)?;
    let mut menu = base.fork()?;
    menu.execute(
        &Command::Resident {
            identity: actor.clone(),
        },
        &Limits::default(),
    )?;
    menu.execute(
        &Command::Object {
            identity: shower.clone(),
        },
        &Limits::default(),
    )?;
    let choices: Vec<_> = menu
        .menu()
        .context("shower menu absent")?
        .actions()
        .iter()
        .enumerate()
        .filter(|(_, a)| a.flags & 16 == 0 && a.routine == 4098)
        .map(|(index, expected)| Command::Choice {
            index,
            expected: expected.clone(),
        })
        .collect();
    ensure!(
        choices.len() == 1,
        "expected the actual original shower action"
    );
    let mut reports = vec![];
    for cancel in [false, true] {
        let name = if cancel { "active-cancel" } else { "complete" };
        let branch = out.join(name);
        std::fs::create_dir(&branch)?;
        let mut app = base.fork()?;
        let mut commands = vec![
            Command::Resident {
                identity: actor.clone(),
            },
            Command::Object {
                identity: shower.clone(),
            },
            choices[0].clone(),
            Command::Playing { value: true },
            wait(
                Milestone::Animation {
                    actor: actor.clone(),
                    name: "a2o-shower1-scrub1loop".into(),
                },
                1500,
                40,
            ),
            Command::Check {
                milestone: Milestone::All {
                    conditions: vec![
                        Milestone::ObjectUseCount {
                            target: shower.clone(),
                            count: 1,
                        },
                        Milestone::PersonField {
                            actor: actor.clone(),
                            field: 8,
                            value: 1,
                        },
                    ],
                },
            },
        ];
        if cancel {
            commands.push(Command::CancelAll {
                actor: actor.clone(),
            });
        }
        commands.push(wait(
            Milestone::InteractionReleased {
                actor: actor.clone(),
                target: shower.clone(),
            },
            2500,
            40,
        ));
        let release_boundary = commands.len();
        commands.push(Command::Check {
            milestone: Milestone::All {
                conditions: vec![
                    Milestone::PersonField {
                        actor: actor.clone(),
                        field: 8,
                        value: 0,
                    },
                    Milestone::PersonField {
                        actor: actor.clone(),
                        field: 0,
                        value: 0,
                    },
                    Milestone::ObjectUseCount {
                        target: shower.clone(),
                        count: 0,
                    },
                ],
            },
        });
        commands.push(wait(Milestone::CompletedTicks { count: 32 }, 64, 250));
        let plan = checkpoint::schedule(
            Plan {
                seed: 1,
                minimum_completed_ticks: 64,
                compare_ticks: true,
                limits: Limits {
                    commands: 24,
                    completed_ticks: 4096,
                    service_polls: 10000,
                    wall_seconds: 600.,
                },
                ..Plan::default()
            },
            Some(&commands),
            &[5, release_boundary],
        )?;
        write_json(&branch.join("commands.json"), &commands)?;
        std::fs::write(branch.join("initial.json"), app.checkpoint()?)?;
        let mut budget = ArtifactBudget {
            screenshots: 0,
            failure_checkpoints: 1,
        };
        let report = run_case(
            &mut app,
            plan,
            Some(&commands),
            &branch,
            "run",
            &mut budget,
            0,
            &[5, release_boundary],
            &[],
        )?;
        preserve(&app, &branch, "run", &report, &mut budget)?;
        std::fs::write(branch.join("final.json"), app.checkpoint()?)?;
        if report.passed() {
            milestone_images(&app, &branch, &[5, release_boundary])?;
        }
        reports.push(
            json!({"branch":name,"outcome":report.outcome,"failure":report.failure,
            "completed_ticks":report.completed_ticks(),"tick_comparisons":report.tick_comparisons,
            "cold_restores":report.cold_restores,"milestone_boundaries":[5,release_boundary]}),
        );
    }
    write_json(
        &out.join("report.json"),
        &json!({"branches":reports,"strict_native_parity_ticks":0}),
    )?;
    ensure!(
        reports
            .iter()
            .all(|r| r["outcome"] == "passed" && r["cold_restores"] == 2),
        "interaction milestone branch failed; preserved reports/checkpoints"
    );
    Ok(())
}

#[test]
#[ignore = "private captured scrub checkpoint; fresh SIMS_INTERACTION_MILESTONES_OUTPUT"]
fn active_shower_cancellation_changes_retained_flags_before_time_advances() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(cancel_request)?
        .join()
        .expect("active cancellation worker panicked")
}
fn cancel_request() -> Result<()> {
    let out = output()?;
    let source = std::env::var("SIMS_CANCELLATION_SOURCE").unwrap_or_else(|_| {
        "cache/gameplay-shower-milestones-20261023-second/active-cancel/run-step-5.json".into()
    });
    let mut app = load(&source, &out)?;
    let actor = Identity::get(app.session()?, 105)?;
    let target = Identity::get(app.session()?, 107)?;
    ensure!(
        Milestone::Animation {
            actor: actor.clone(),
            name: "a2o-shower1-scrub1loop".into()
        }
        .reached(&app, app.session()?.simulation.vm.tick)?,
        "source is not scrubbing"
    );
    let before = app.observe()?;
    let old = app.session()?.simulation.vm.clone();
    let active = old.world.actions.as_ref().unwrap().actors[&actor.object]
        .active
        .as_ref()
        .context("scrub has no active action")?
        .clone();
    ensure!(
        active.target == target.object && active.flags & sims_vm::action::CANCELLED == 0,
        "source must have an uncancelled active shower"
    );
    std::fs::write(out.join("before.json"), app.checkpoint()?)?;
    app.execute(
        &Command::CancelAll {
            actor: actor.clone(),
        },
        &Limits::default(),
    )?;
    let after = app.observe()?;
    let vm = &app.session()?.simulation.vm;
    let actual = vm.world.actions.as_ref().unwrap().actors[&actor.object]
        .active
        .as_ref()
        .context("request synchronously removed the active shower")?;
    ensure!(
        actual.id == active.id
            && actual.target == active.target
            && actual.flags == active.flags | sims_vm::action::CANCELLED,
        "active cancellation flag was not published"
    );
    ensure!(
        before.tick == after.tick && before.rng == after.rng,
        "cancellation advanced time or RNG"
    );
    ensure!(
        vm.world.objects[&actor.object].person.as_ref().unwrap()[&33] == 0,
        "native cancellation priority not cleared"
    );
    ensure!(
        vm.world.objects[&target.object].data[&62] == 1
            && old.world.objects[&target.object].data[&62] == 1,
        "cancellation prematurely released the active use"
    );
    let tid = vm.object_threads[&actor.object];
    ensure!(
        vm.threads[&tid].stack == old.threads[&tid].stack,
        "cancellation executed/unwound the suspended scrub stack"
    );
    let bytes = app.checkpoint()?;
    std::fs::write(out.join("after.json"), &bytes)?;
    ensure!(
        app.restore(&bytes)?.observe()? == after,
        "cancelled scrub cold restore changed state"
    );
    write_json(
        &out.join("report.json"),
        &json!({"tick":after.tick,"rng":after.rng,
        "actor":actor,"target":target,"serial":active.id,"flags_before":active.flags,
        "flags_after":actual.flags,"priority_before":old.world.objects[&actor.object].person.as_ref().unwrap()[&33],
        "priority_after":0,"queue_before":old.world.actions.as_ref().unwrap().actors[&actor.object].queue,
        "queue_after":vm.world.actions.as_ref().unwrap().actors[&actor.object].queue,
        "use_count":1,"suspended_stack_unchanged":true,"cold_state_equal":true,
        "state_before":before.state_sha256,"state_after":after.state_sha256,
        "strict_native_parity_ticks":0}),
    )?;
    Ok(())
}

#[test]
#[ignore = "private Goth assets; fresh SIMS_INTERACTION_MILESTONES_OUTPUT, headless checks only"]
fn goth_two_residents_contend_for_one_chair_with_cancel_and_release() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(goth_contention)?
        .join()
        .expect("Goth contention worker panicked")
}
fn plan() -> Plan {
    Plan {
        seed: 1,
        minimum_completed_ticks: 1,
        compare_ticks: true,
        limits: Limits {
            commands: 32,
            completed_ticks: 4096,
            service_polls: 10000,
            wall_seconds: 600.,
        },
        ..Plan::default()
    }
}
pub(super) fn run_stage(
    app: &mut Application,
    out: &Path,
    commands: &[Command],
    restores: &[usize],
) -> Result<Report<Command>> {
    run_stage_with_minimum(app, out, commands, restores, 1)
}
pub(super) fn run_stage_with_minimum(
    app: &mut Application,
    out: &Path,
    commands: &[Command],
    restores: &[usize],
    minimum_completed_ticks: u64,
) -> Result<Report<Command>> {
    run_stage_planned(
        app,
        out,
        commands,
        restores,
        Plan {
            minimum_completed_ticks,
            ..plan()
        },
    )
}
/// The next semantic stage compares every tick against an independent cold
/// initial state; it does not rely on the previous stage retaining its shadow.
pub(super) fn run_stage_cold(
    app: &mut Application,
    out: &Path,
    commands: &[Command],
    restores: &[usize],
    minimum_completed_ticks: u64,
) -> Result<Report<Command>> {
    run_stage_planned(
        app,
        out,
        commands,
        restores,
        Plan {
            minimum_completed_ticks,
            cold_restore_at: Some(0),
            ..plan()
        },
    )
}
fn run_stage_planned(
    app: &mut Application,
    out: &Path,
    commands: &[Command],
    restores: &[usize],
    stage_plan: Plan,
) -> Result<Report<Command>> {
    std::fs::create_dir(out)?;
    std::fs::write(out.join("initial.json"), app.checkpoint()?)?;
    write_json(&out.join("commands.json"), &commands)?;
    let mut budget = ArtifactBudget {
        screenshots: 0,
        failure_checkpoints: 1,
    };
    let report = run_case(
        app,
        checkpoint::schedule(stage_plan, Some(commands), restores)?,
        Some(commands),
        out,
        "run",
        &mut budget,
        0,
        restores,
        &[],
    )?;
    preserve(app, out, "run", &report, &mut budget)?;
    std::fs::write(out.join("final.json"), app.checkpoint()?)?;
    ensure!(
        report.passed(),
        "Goth interaction stage failed; preserved report at {}",
        out.display()
    );
    milestone_images(app, out, restores)?;
    Ok(report)
}
fn milestone_images(app: &Application, out: &Path, boundaries: &[usize]) -> Result<()> {
    let mut rows = vec![];
    for boundary in boundaries {
        let path = out.join(format!("run-step-{boundary}.json"));
        let restored = app.restore(&std::fs::read(&path)?)?;
        let before = restored.observe()?;
        let scene = restored
            .viewer
            .device
            .as_ref()
            .context("milestone camera absent")?
            .scene();
        let pixels = sims_render::software::render(scene);
        ensure!(
            restored.observe()? == before,
            "software milestone image advanced retained state"
        );
        image::save_buffer(
            out.join(format!("run-step-{boundary}.png")),
            &pixels,
            scene.width,
            scene.height,
            image::ColorType::Rgba8,
        )?;
        rows.push(
            json!({"boundary":boundary,"tick":before.tick,"rng":before.rng,
            "state_sha256":before.state_sha256,"rgba_sha256":engine::digest(&pixels),
            "retained_state_unchanged":true}),
        );
    }
    write_json(&out.join("images.json"), &rows)
}
pub(super) fn offered(
    app: &mut Application,
    actor: &Identity,
    target: &Identity,
    routine: u16,
) -> Result<Vec<Command>> {
    let mut commands = vec![
        Command::Resident {
            identity: actor.clone(),
        },
        Command::Object {
            identity: target.clone(),
        },
    ];
    for command in &commands {
        app.execute(command, &Limits::default())?;
    }
    let choices: Vec<_> = app
        .menu()
        .context("interaction menu absent")?
        .actions()
        .iter()
        .enumerate()
        .filter(|(_, a)| a.flags & 16 == 0 && a.routine == routine)
        .map(|(index, expected)| Command::Choice {
            index,
            expected: expected.clone(),
        })
        .collect();
    ensure!(
        choices.len() == 1,
        "expected one original offered routine {routine}"
    );
    app.execute(&choices[0], &Limits::default())?;
    commands.push(choices[0].clone());
    Ok(commands)
}
fn goth_contention() -> Result<()> {
    let out = output()?;
    let mut common = load(
        "cache/gameplay-exploration-domestic-final-goth-20261023/initial.json",
        &out,
    )?;
    let actors = [
        Identity::get(common.session()?, 18)?,
        Identity::get(common.session()?, 21)?,
    ];
    let chair = Identity::get(common.session()?, 198)?;
    let sink = Identity::get(common.session()?, 111)?;
    for actor in &actors {
        let p = common.session()?.simulation.vm.world.positions[&actor.object];
        ensure!(
            p.x >= 0 && p.y >= 0 && p.level == 1,
            "contention requires actual on-lot residents"
        );
    }
    let mut staged = common.fork()?;
    let mut commands = offered(&mut staged, &actors[0], &chair, 4097)?;
    commands.extend(offered(&mut staged, &actors[1], &chair, 4097)?);
    commands.push(Command::Playing { value: true });
    commands.push(wait(
        Milestone::All {
            conditions: actors
                .iter()
                .map(|actor| Milestone::ActionActive {
                    actor: actor.clone(),
                    target: chair.object,
                })
                .collect(),
        },
        512,
        40,
    ));
    commands.push(wait(Milestone::CompletedTicks { count: 1 }, 16, 40));
    let shared = run_stage(&mut common, &out.join("both-active"), &commands, &[8])?;
    let mut reports = vec![
        json!({"stage":"both-active","ticks":shared.completed_ticks(),"restores":shared.cold_restores}),
    ];
    for cancel in [false, true] {
        let name = if cancel {
            "cancel-contender"
        } else {
            "normal-release"
        };
        let branch = out.join(name);
        std::fs::create_dir(&branch)?;
        let mut app = common.fork()?;
        let mut commands = vec![];
        if cancel {
            commands.push(Command::Resident {
                identity: actors[1].clone(),
            });
            commands.push(Command::CancelAll {
                actor: actors[1].clone(),
            });
            commands.push(wait(
                Milestone::InteractionReleased {
                    actor: actors[1].clone(),
                    target: chair.clone(),
                },
                1500,
                40,
            ));
        }
        commands.push(wait(
            Milestone::Any {
                conditions: actors
                    .iter()
                    .map(|actor| Milestone::SolePersonContained {
                        actor: actor.clone(),
                        target: chair.clone(),
                    })
                    .collect(),
            },
            2000,
            40,
        ));
        let occupied = commands.len();
        commands.push(wait(Milestone::CompletedTicks { count: 1 }, 16, 40));
        let entry = run_stage(&mut app, &branch.join("entry"), &commands, &[occupied])?;
        let winner = actors
            .iter()
            .find(|a| {
                app.session().unwrap().simulation.vm.world.objects[&a.object]
                    .container
                    .is_some_and(|(id, _)| id == chair.object)
            })
            .context("seat milestone has no retained occupant")?
            .clone();
        // A completed Sit may leave physical posture retained. Request the
        // actual sink action to run the original chair exit callback.
        let mut staged = app.fork()?;
        let mut commands = offered(&mut staged, &winner, &sink, 4104)?;
        commands.push(wait(
            Milestone::InteractionReleased {
                actor: winner.clone(),
                target: chair.clone(),
            },
            1500,
            40,
        ));
        let released = commands.len();
        commands.push(wait(goth_sink_or_notice(&winner, &sink), 1500, 40));
        let barrier = commands.len();
        commands.push(Command::Poll);
        let exit = run_stage(
            &mut app,
            &branch.join("exit-barrier"),
            &commands,
            &[released, barrier],
        )?;
        let mut notices = 0;
        let mut notice_ticks = 0;
        let mut notice_restores = 0;
        while let Some(id) = app.session()?.simulation.vm.world.dialogs.modal {
            ensure!(
                notices < GOTH_NOTICES.len(),
                "unexpected extra notice; finite input budget exhausted"
            );
            let record = app
                .session()?
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("Goth notice record absent")?;
            ensure!(
                record.source == GOTH_NOTICE_SOURCE
                    && GOTH_NOTICES
                        .iter()
                        .any(|(title, body_sha256)| record.title == *title && engine::digest(record.body.as_bytes()) == *body_sha256)
                    && record.buttons.len() == 1
                    && record.buttons[0].label == "OK"
                    && record.buttons[0].result == 2,
                "unexpected modal; no automatic replacement response"
            );
            let commands = vec![
                Command::Dialog {
                    id,
                    result: 2,
                    body_sha256: engine::digest(record.body.as_bytes()),
                },
                Command::Playing { value: true },
                wait(goth_sink_or_notice(&winner, &sink), 1500, 40),
                Command::Poll,
            ];
            let notice = run_stage(
                &mut app,
                &branch.join(format!("notice-{}", notices + 1)),
                &commands,
                &[3],
            )?;
            notices += 1;
            notice_ticks += notice.tick_comparisons;
            notice_restores += notice.cold_restores;
        }
        let mut commands = vec![];
        commands.push(wait(
            Milestone::InteractionReleased {
                actor: winner.clone(),
                target: sink.clone(),
            },
            1500,
            40,
        ));
        let washed = commands.len();
        commands.push(wait(Milestone::CompletedTicks { count: 32 }, 64, 250));
        let finish = run_stage(&mut app, &branch.join("finish"), &commands, &[washed])?;
        reports.push(
            json!({"branch":name,"winner":winner,"entry_ticks":entry.completed_ticks(),
            "exit_ticks":exit.completed_ticks(),"finish_ticks":finish.completed_ticks(),
            "recorded_notices":notices,"notice_ticks":notice_ticks,
            "tick_comparisons":entry.tick_comparisons+exit.tick_comparisons+notice_ticks+finish.tick_comparisons,
            "restores":entry.cold_restores+exit.cold_restores+notice_restores+finish.cold_restores}),
        );
    }
    write_json(
        &out.join("report.json"),
        &json!({"stages":reports,"strict_native_parity_ticks":0,
        "claim":"actual offered concurrent chair actions, cancellation or native release, cold-state agreement"}),
    )?;
    Ok(())
}
pub(super) const GOTH_NOTICE_SOURCE: &str =
    "GameData_ALL_Recursive/Objects/Objects.far::CarPortal.iff";
pub(super) const GOTH_NOTICES: [(&str, &str); 2] = [
    (
        "Car Pool",
        "c4f983d1377ed8931ec9c56fbe8d1c7dd975c29b8732edab106c7ab5eeaa8c9f",
    ),
    ("School Bus", "ccdb5655d8be72b82dc54a592be42c38098d917904ba97c924560611884a3cbb"),
];
pub(super) fn goth_sink_or_notice(actor: &Identity, sink: &Identity) -> Milestone {
    let mut conditions = vec![Milestone::InteractionReleased {
        actor: actor.clone(),
        target: sink.clone(),
    }];
    conditions.extend(GOTH_NOTICES.iter().map(|(title, body_sha256)| Milestone::Modal {
        source: GOTH_NOTICE_SOURCE.into(),
        title: (*title).into(),
        body_sha256: (*body_sha256).into(),
    }));
    Milestone::Any { conditions }
}
