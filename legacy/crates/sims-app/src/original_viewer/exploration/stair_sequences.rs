//! Whole-application stair continuations, using real offered original actions.
//! Historical inferred-map inputs remain diagnostic, never strict parity.
use super::*;
use interaction_sequences::{
    GOTH_NOTICE_SOURCE, GOTH_NOTICES, load, offered, run_stage_cold as run_stage_with_minimum, wait,
};

const STAIR: &str = "GameData_ALL_Recursive/Objects/Objects.far::stair.iff";
const EASEL: &str = "GameData_ALL_Recursive/Objects/Objects.far::Easel.iff";
const SINK: &str = "GameData_ALL_Recursive/Objects/Objects.far::Sinks.iff";

fn run_stage(
    app: &mut Application,
    out: &Path,
    commands: &[Command],
    restores: &[usize],
) -> Result<Report<Command>> {
    run_stage_with_minimum(app, out, commands, restores, 1)
}

#[test]
#[ignore = "private original assets; fresh SIMS_STAIR_MILESTONES_OUTPUT, headless only"]
fn goth_stair_climb_cancel_descend_and_downstairs_use_resume() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(stair_branches)?
        .join()
        .expect("stair milestone worker panicked")
}

fn frame(actor: &Identity, source: &str, bhav: Option<u16>, present: bool) -> Milestone {
    Milestone::ScriptFrame {
        actor: actor.clone(),
        source: source.into(),
        bhav,
        present,
    }
}
fn level(actor: &Identity, value: i32) -> Milestone {
    Milestone::Level {
        actor: actor.clone(),
        value,
    }
}
fn phase(actor: &Identity, name: &str) -> Milestone {
    Milestone::AnimationPhase {
        actor: actor.clone(),
        name: name.into(),
        minimum: 0.3,
        maximum: 0.7,
    }
}
fn all(conditions: Vec<Milestone>) -> Milestone {
    Milestone::All { conditions }
}
fn released(actor: &Identity, target: &Identity) -> Milestone {
    Milestone::InteractionReleased {
        actor: actor.clone(),
        target: target.clone(),
    }
}
fn stairs_released(actor: &Identity, stairs: &[Identity]) -> Milestone {
    let mut conditions = vec![frame(actor, STAIR, None, false)];
    conditions.extend(stairs.iter().map(|target| released(actor, target)));
    Milestone::All { conditions }
}
fn reach_with_notices(
    app: &mut Application,
    branch: &Path,
    name: &str,
    mut commands: Vec<Command>,
    goal: Milestone,
    actor: &Identity,
    rows: &mut Vec<Value>,
    out: &Path,
    seen: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    for barrier in 0..=GOTH_NOTICES.len() {
        let mut alternatives = vec![goal.clone()];
        alternatives.extend(GOTH_NOTICES.iter().map(|(title, body_sha256)| Milestone::Modal {
            source: GOTH_NOTICE_SOURCE.into(),
            title: (*title).into(),
            body_sha256: (*body_sha256).into(),
        }));
        commands.push(wait(
            Milestone::Any {
                conditions: alternatives,
            },
            850,
            40,
        ));
        let boundary = commands.len();
        commands.push(Command::Poll);
        let stage = format!("{name}-{barrier}");
        let report = run_stage_with_minimum(app, &branch.join(&stage), &commands, &[boundary], 0)?;
        record_stage(
            out,
            &format!("{}/{stage}", branch.file_name().unwrap().to_string_lossy()),
            &report,
            app,
            actor,
            rows,
        )?;
        let dialogs = &app.session()?.simulation.vm.world.dialogs;
        let Some(id) = dialogs.modal else {
            ensure!(
                goal.reached(app, app.session()?.simulation.vm.tick)?,
                "desired milestone did not survive notice handling"
            );
            return Ok(());
        };
        let record = dialogs
            .records
            .iter()
            .find(|r| r.id == id)
            .context("notice record absent")?;
        ensure!(
            record.source == GOTH_NOTICE_SOURCE
                && GOTH_NOTICES
                    .iter()
                    .any(|(title, body_sha256)| record.title == *title && engine::digest(record.body.as_bytes()) == *body_sha256)
                && record.buttons.len() == 1
                && record.buttons[0].label == "OK"
                && record.buttons[0].result == 2
                && seen.insert(record.title.clone()),
            "unexpected or repeated modal; no replacement input"
        );
        ensure!(
            barrier < GOTH_NOTICES.len(),
            "notice input budget exhausted"
        );
        commands = vec![
            Command::Dialog {
                id,
                result: 2,
                body_sha256: engine::digest(record.body.as_bytes()),
            },
            Command::Playing { value: true },
        ];
    }
    anyhow::bail!("notice input budget exhausted before required milestone")
}
fn record_stage(
    out: &Path,
    name: &str,
    report: &Report<Command>,
    app: &Application,
    actor: &Identity,
    rows: &mut Vec<Value>,
) -> Result<()> {
    let sim = &app.session()?.simulation;
    rows.push(json!({"stage":name,"start_tick":report.initial.tick,
        "end_tick":sim.vm.tick,"completed_ticks":report.completed_ticks(),
        "tick_comparisons":report.tick_comparisons,"restores":report.cold_restores,
        "position":sim.vm.world.positions[&actor.object],
        "active":sim.vm.world.actions.as_ref().unwrap().actors[&actor.object].active,
        "previous":sim.vm.world.actions.as_ref().unwrap().actors[&actor.object].previous,
        "outcome":report.outcome}));
    write_json(&out.join("stages.json"), &rows)
}
fn check_stair_ownership(app: &Application, actor: &Identity, stairs: &[Identity]) -> Result<()> {
    ensure!(
        stairs_released(actor, stairs).reached(app, 0)?,
        "stair script/action-use/physical ownership remains"
    );
    let sim = &app.session()?.simulation;
    ensure!(
        sim.routes
            .values()
            .all(|r| r.caller != actor.object || stairs.iter().all(|s| s.object != r.target)),
        "stair walking route remains"
    );
    // Autonomous residents legitimately share the stairs. Its global count
    // must exactly match their live/pending-unwind uses; actor release above
    // must not demand that those other residents have also finished climbing.
    for target in stairs {
        ensure!(
            Milestone::ObjectUsesAccounted {
                target: target.clone(),
            }
            .reached(app, 0)?,
            "stair multipart use count disagrees with retained action-use owners"
        );
    }
    Ok(())
}
fn cancel_at_checkpoint(
    app: &mut Application,
    branch: &Path,
    actor: &Identity,
    easel: &Identity,
    rows: &mut Vec<Value>,
    out: &Path,
) -> Result<()> {
    let before = app.observe()?;
    let sim = &app.session()?.simulation;
    let active = sim.vm.world.actions.as_ref().unwrap().actors[&actor.object]
        .active
        .as_ref()
        .context("checkpoint has no active Paint")?
        .clone();
    ensure!(
        active.target == easel.object && active.flags & sims_vm::action::CANCELLED == 0,
        "checkpoint must retain uncancelled original Paint"
    );
    let thread = sim.vm.object_threads[&actor.object];
    let stack = sim.vm.threads[&thread].stack.clone();
    let commands = [
        Command::CancelAction {
            actor: actor.clone(),
            serial: active.id,
        },
        Command::Check {
            milestone: Milestone::ActionActive {
                actor: actor.clone(),
                target: easel.object,
            },
        },
        Command::Poll,
    ];
    let stage = branch.join("cancel-request");
    std::fs::create_dir(&stage)?;
    std::fs::write(stage.join("initial.json"), app.checkpoint()?)?;
    write_json(&stage.join("commands.json"), &commands)?;
    let mut budget = ArtifactBudget {
        screenshots: 0,
        failure_checkpoints: 1,
    };
    // Input publication is deliberately a zero-tick stage, unlike the
    // progressing gameplay stages. The cold sibling is replaced after Check.
    let report = run_case(
        app,
        checkpoint::schedule(
            Plan {
                seed: 1,
                compare_ticks: true,
                cold_restore_at: Some(0),
                minimum_completed_ticks: 0,
                limits: Limits {
                    commands: 8,
                    completed_ticks: 32,
                    service_polls: 10000,
                    wall_seconds: 120.,
                },
                ..Plan::default()
            },
            Some(&commands),
            &[2],
        )?,
        Some(&commands),
        &stage,
        "run",
        &mut budget,
        0,
        &[2],
        &[],
    )?;
    preserve(app, &stage, "run", &report, &mut budget)?;
    std::fs::write(stage.join("final.json"), app.checkpoint()?)?;
    ensure!(
        report.passed(),
        "cancellation input stage failed; evidence at {}",
        stage.display()
    );
    let after = app.observe()?;
    let sim = &app.session()?.simulation;
    let retained = sim.vm.world.actions.as_ref().unwrap().actors[&actor.object]
        .active
        .as_ref()
        .context("cancel request synchronously removed Paint")?;
    ensure!(
        retained.id == active.id
            && retained.target == active.target
            && retained.flags == active.flags | sims_vm::action::CANCELLED,
        "active cancellation was blocked or changed a different action"
    );
    ensure!(
        before.tick == after.tick && before.rng == after.rng,
        "cancel request advanced simulation time/RNG"
    );
    ensure!(
        sim.vm.threads[&thread].stack == stack,
        "input cancellation executed/unwound the suspended stair/Paint stack"
    );
    write_json(
        &branch.join("cancel-proof.json"),
        &json!({"tick":before.tick,
        "rng":before.rng,"serial":active.id,"flags_before":active.flags,
        "flags_after":retained.flags,"suspended_stack_unchanged":true,
        "completed_ticks":report.completed_ticks(),"cold_restores":report.cold_restores,
        "queue":sim.vm.world.actions.as_ref().unwrap().actors[&actor.object].queue}),
    )?;
    record_stage(
        out,
        &format!(
            "{}/cancel-request",
            branch.file_name().unwrap().to_string_lossy()
        ),
        &report,
        app,
        actor,
        rows,
    )
}
fn stair_branches() -> Result<()> {
    let out = research_output()?;
    let mut common = load(
        "cache/gameplay-exploration-domestic-final-goth-20261023/initial.json",
        &out,
    )?;
    run_stair_branches(&out, &mut common)
}
fn research_output() -> Result<PathBuf> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let relative = PathBuf::from(std::env::var("SIMS_STAIR_MILESTONES_OUTPUT")?);
    ensure!(
        !relative.is_absolute()
            && relative.starts_with("cache")
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "stair output must be a fresh cache child"
    );
    let out = repo.join(relative);
    std::fs::create_dir(&out)?;
    Ok(out)
}
fn run_stair_branches(out: &Path, common: &mut Application) -> Result<()> {
    let actor = Identity::get(common.session()?, 18)?;
    let easel = Identity::get(common.session()?, 216)?;
    let sink = Identity::get(common.session()?, 111)?;
    ensure!(
        level(&actor, 1).reached(&common, 0)?,
        "actor is not downstairs"
    );
    ensure!(
        common.session()?.simulation.vm.world.positions[&easel.object].level == 2
            && common.session()?.simulation.vm.world.positions[&sink.object].level == 1,
        "original upstairs/downstairs targets changed"
    );
    let stairs: Vec<_> = common
        .session()?
        .simulation
        .vm
        .world
        .objects
        .iter()
        .filter(|(_, o)| o.definition.as_ref().is_some_and(|d| d.source == STAIR))
        .map(|(&id, _)| Identity::get(common.session().unwrap(), id))
        .collect::<Result<_>>()?;
    ensure!(!stairs.is_empty(), "original multipart stairs absent");
    let mut rows = vec![];
    let mut staged = common.fork()?;
    let mut commands = offered(&mut staged, &actor, &easel, 4097)?;
    commands.push(Command::Playing { value: true });
    commands.push(Command::Floor { level: 2 });
    commands.push(wait(
        all(vec![
            phase(&actor, "a2o-stair-climb"),
            frame(&actor, STAIR, None, true),
            level(&actor, 2),
            Milestone::ActionActive {
                actor: actor.clone(),
                target: easel.object,
            },
        ]),
        512,
        40,
    ));
    commands.push(Command::Poll);
    let prefix = run_stage(common, &out.join("mid-climb"), &commands, &[6])?;
    record_stage(&out, "mid-climb", &prefix, &common, &actor, &mut rows)?;
    for mid_climb_cancel in [true, false] {
        let name = if mid_climb_cancel {
            "cancel-mid-climb"
        } else {
            "paint-then-return"
        };
        let branch = out.join(name);
        std::fs::create_dir(&branch)?;
        let mut app = common.fork()?;
        if !mid_climb_cancel {
            let commands = [
                wait(
                    all(vec![
                        level(&actor, 2),
                        frame(&actor, EASEL, Some(4104), true),
                        frame(&actor, STAIR, None, false),
                        Milestone::ActionActive {
                            actor: actor.clone(),
                            target: easel.object,
                        },
                    ]),
                    850,
                    40,
                ),
                Command::Poll,
            ];
            let report = run_stage(&mut app, &branch.join("painting"), &commands, &[1])?;
            check_stair_ownership(&app, &actor, &stairs)?;
            record_stage(
                &out,
                &format!("{name}/painting"),
                &report,
                &app,
                &actor,
                &mut rows,
            )?;
        }
        cancel_at_checkpoint(&mut app, &branch, &actor, &easel, &mut rows, &out)?;
        let commands = [
            wait(
                all(vec![
                    level(&actor, 2),
                    released(&actor, &easel),
                    stairs_released(&actor, &stairs),
                ]),
                850,
                40,
            ),
            Command::Poll,
        ];
        let report = run_stage(&mut app, &branch.join("upstairs-released"), &commands, &[1])?;
        check_stair_ownership(&app, &actor, &stairs)?;
        record_stage(
            &out,
            &format!("{name}/upstairs-released"),
            &report,
            &app,
            &actor,
            &mut rows,
        )?;
        // The actual offered downstairs action drives the return, never a
        // synthetic position edit or assumed reaction to Paint cancellation.
        let mut staged = app.fork()?;
        let mut commands = offered(&mut staged, &actor, &sink, 4104)?;
        commands.push(Command::Floor { level: 1 });
        let mut seen_notices = std::collections::BTreeSet::new();
        reach_with_notices(
            &mut app,
            &branch,
            "descend",
            commands,
            all(vec![
                phase(&actor, "a2o-stair-descend"),
                frame(&actor, STAIR, None, true),
                level(&actor, 1),
                Milestone::ActionActive {
                    actor: actor.clone(),
                    target: sink.object,
                },
            ]),
            &actor,
            &mut rows,
            &out,
            &mut seen_notices,
        )?;
        reach_with_notices(
            &mut app,
            &branch,
            "wash",
            vec![],
            all(vec![
                level(&actor, 1),
                frame(&actor, SINK, Some(4104), true),
                phase(&actor, "a2o-sink-washhands"),
                stairs_released(&actor, &stairs),
                Milestone::ActionActive {
                    actor: actor.clone(),
                    target: sink.object,
                },
                Milestone::ObjectUseCount {
                    target: sink.clone(),
                    count: 1,
                },
            ]),
            &actor,
            &mut rows,
            &out,
            &mut seen_notices,
        )?;
        check_stair_ownership(&app, &actor, &stairs)?;
        reach_with_notices(
            &mut app,
            &branch,
            "sink-release",
            vec![],
            released(&actor, &sink),
            &actor,
            &mut rows,
            &out,
            &mut seen_notices,
        )?;
        let commands = [
            wait(
                all(vec![
                    level(&actor, 1),
                    released(&actor, &sink),
                    stairs_released(&actor, &stairs),
                    Milestone::ObjectUseCount {
                        target: sink.clone(),
                        count: 0,
                    },
                    Milestone::PersonField {
                        actor: actor.clone(),
                        field: 0,
                        value: 0,
                    },
                ]),
                850,
                40,
            ),
            wait(Milestone::CompletedTicks { count: 32 }, 64, 40),
        ];
        let report = run_stage(&mut app, &branch.join("finish"), &commands, &[1])?;
        check_stair_ownership(&app, &actor, &stairs)?;
        record_stage(
            &out,
            &format!("{name}/finish"),
            &report,
            &app,
            &actor,
            &mut rows,
        )?;
    }
    write_json(
        &out.join("report.json"),
        &json!({"stages":rows,
        "compared_ticks":rows.iter().map(|r| r["tick_comparisons"].as_u64().unwrap()).sum::<u64>(),
        "cold_restores":rows.iter().map(|r| r["restores"].as_u64().unwrap()).sum::<u64>(),
        "strict_native_parity_ticks":0,"original_engine_executed":false,
        "claim":"actual original climb, active cancellation, descent, downstairs wash and release; warm/cold agreement"}),
    )
}

#[test]
#[ignore = "private preserved stair notice; fresh SIMS_STAIR_MILESTONES_OUTPUT"]
fn diagnose_goth_descend_phase_from_saved_notice() -> Result<()> {
    diagnose_saved_notice(false)
}
#[test]
#[ignore = "private preserved stair notice; fresh SIMS_STAIR_MILESTONES_OUTPUT"]
fn diagnose_goth_post_notice_sink_release_resume() -> Result<()> {
    diagnose_saved_notice(true)
}
fn diagnose_saved_notice(sink_failure: bool) -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(move || -> Result<()> {
        let out = research_output()?;
        let source = std::env::var("SIMS_STAIR_DIAGNOSTIC_SOURCE").unwrap_or_else(|_| {
            "cache/gameplay-stair-return-20261023-fifth/cancel-mid-climb/descend-2/initial.json".into()
        });
        let mut app = load(&source, &out)?;
        let actor = Identity::get(app.session()?, 18)?;
        let sink = Identity::get(app.session()?, 111)?;
        let source_already_released = released(&actor, &sink)
            .reached(&app, app.session()?.simulation.vm.tick)?;
        if sink_failure {
            ensure!(source_already_released, "post-release probe needs the recorded already-released notice source");
        }
        let dialogs = &app.session()?.simulation.vm.world.dialogs;
        let id = dialogs.modal.context("diagnostic requires actual School Bus notice")?;
        let record = dialogs.records.iter().find(|r| r.id == id).context("notice record absent")?;
        ensure!(record.source == GOTH_NOTICE_SOURCE && record.title == GOTH_NOTICES[1].0
            && engine::digest(record.body.as_bytes()) == GOTH_NOTICES[1].1 && record.buttons.len() == 1
            && record.buttons[0].label == "OK" && record.buttons[0].result == 2,
            "diagnostic source is not the exact recorded School Bus notice");
        // History can already be replaced at a completed-tick boundary. Observe
        // the first complete release, then record its actual native return bit.
        let goal = if sink_failure { released(&actor, &sink)
        } else { phase(&actor, "a2o-stair-descend") };
        let commands = [Command::Dialog { id, result: 2,
                body_sha256: engine::digest(record.body.as_bytes()) },
            Command::Playing { value: true },
            wait(goal, 512, 40), wait(Milestone::CompletedTicks { count: 32 }, 64, 40), Command::Poll];
        let report = run_stage(&mut app, &out.join(if sink_failure {
            "sink-release" } else { "first-descend-phase" }), &commands, &[3])?;
        let checkpoint = out.join(if sink_failure {"sink-release"} else {"first-descend-phase"})
            .join("run-step-3.json");
        let milestone = app.restore(&std::fs::read(checkpoint)?)?;
        let sim = &milestone.session()?.simulation;
        write_json(&out.join("diagnostic.json"), &json!({
            "tick":sim.vm.tick,"position":sim.vm.world.positions[&actor.object],
            "actor_actions":sim.vm.world.actions.as_ref().unwrap().actors[&actor.object],
            "frames":sim.vm.threads.values().flat_map(|t| t.stack.iter())
                .filter(|f| f.caller == actor.object).collect::<Vec<_>>(),
            "all_person_positions":sim.vm.world.objects.iter().filter(|(_, o)| o.person.is_some())
                .map(|(&id, _)| (id, sim.vm.world.positions.get(&id))).collect::<Vec<_>>(),
            "route_control":sim.route_control,"route_feedback":sim.route_feedback,
            "stair_users":sim.vm.threads.values().flat_map(|t| t.stack.iter())
                .filter(|f| f.location.source == STAIR).collect::<Vec<_>>(),
            "compared_ticks":report.tick_comparisons,"cold_restores":report.cold_restores,
            "continuation_end_tick":app.session()?.simulation.vm.tick,
            "post_milestone_compared_ticks":32,
            "strict_native_parity_ticks":0,
            "source_already_released":source_already_released,
            "kind":if sink_failure {"post_notice_sink_release_resume"} else {"first_actual_descend_phase"},
            "claim":"diagnostic checkpoint, not first commanded sink release or successful commanded sink use"}))
    })?.join().expect("stair diagnostic worker panicked")
}

#[test]
#[ignore = "private mid-climb-cancel return input; fresh SIMS_STAIR_MILESTONES_OUTPUT"]
fn goth_commanded_sink_first_release_keeps_preceding_tick() -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| -> Result<()> {
        let out = research_output()?;
        let source = "cache/gameplay-stair-return-20261023-fifth/cancel-mid-climb/descend-0/initial.json";
        let recorded_path = "cache/gameplay-stair-return-20261023-fifth/cancel-mid-climb/descend-0/commands.json";
        let base = load(source, &out)?;
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let input = std::fs::read(repo.join(recorded_path))?;
        let recorded: Vec<Command> = serde_json::from_slice(&input)?;
        let actor = Identity::get(base.session()?, 18)?;
        let sink = Identity::get(base.session()?, 111)?;
        ensure!(recorded.len() >= 4
            && matches!(&recorded[0], Command::Resident { identity } if *identity == actor)
            && matches!(&recorded[1], Command::Object { identity } if *identity == sink)
            && matches!(&recorded[2], Command::Choice { expected, .. }
                if expected.target == sink.object && expected.routine == 4104)
            && matches!(recorded[3], Command::Floor { level: 1 }),
            "preserved original menu/input prefix changed");
        let prefix = &recorded[..4];
        let mut commands = prefix.to_vec();
        commands.push(Command::Playing { value: true });
        commands.push(wait(released(&actor, &sink), 625, 40));
        commands.push(Command::Poll);
        let mut app = base.fork()?;
        let first = run_stage(&mut app, &out.join("first-release"), &commands, &[6])?;
        let release_tick = app.session()?.simulation.vm.tick;
        let playing_tick = first.steps[4].after.as_ref().context("Play observation absent")?.tick;
        let preceding_tick = release_tick.checked_sub(1).context("release at tick zero")?;
        ensure!(release_tick > base.session()?.simulation.vm.tick, "sink released without any gameplay");
        let mut commands = prefix.to_vec();
        let preceding_boundary;
        if preceding_tick < playing_tick {
            ensure!(preceding_tick == base.session()?.simulation.vm.tick,
                "Play skipped the exact preceding completed tick");
            commands.push(Command::Check { milestone: Milestone::CompletedTicks { count: 0 } });
            preceding_boundary = commands.len();
            commands.push(Command::Playing { value: true });
        } else {
            commands.push(Command::Playing { value: true });
            commands.push(wait(Milestone::CompletedTicks { count: preceding_tick - playing_tick }, 625, 40));
            preceding_boundary = commands.len();
        }
        commands.push(wait(released(&actor, &sink), 625, 40));
        let release_boundary = commands.len();
        commands.push(Command::Poll);
        let mut replay = base.fork()?;
        let paired = run_stage(&mut replay, &out.join("preceding-and-release"), &commands,
            &[preceding_boundary, release_boundary])?;
        let previous_bytes = std::fs::read(out.join(format!("preceding-and-release/run-step-{preceding_boundary}.json")))?;
        let released_bytes = std::fs::read(out.join(format!("preceding-and-release/run-step-{release_boundary}.json")))?;
        let previous = replay.restore(&previous_bytes)?;
        let released_app = replay.restore(&released_bytes)?;
        ensure!(previous.session()?.simulation.vm.tick == preceding_tick
            && released_app.session()?.simulation.vm.tick == release_tick
            && !released(&actor, &sink).reached(&previous, preceding_tick)?
            && released(&actor, &sink).reached(&released_app, release_tick)?,
            "replay did not isolate the first actual ownership release");
        let mut boundaries = vec![];
        for (label, app) in [("preceding", &previous), ("release", &released_app)] {
            let sim = &app.session()?.simulation;
            boundaries.push(json!({"boundary":label,"tick":sim.vm.tick,
                "position":sim.vm.world.positions[&actor.object],
                "actions":sim.vm.world.actions.as_ref().unwrap().actors[&actor.object],
                "frames":sim.vm.threads.values().flat_map(|t| t.stack.iter())
                    .filter(|f| f.caller == actor.object).collect::<Vec<_>>(),
                "person_positions":sim.vm.world.objects.iter().filter(|(_, o)| o.person.is_some())
                    .map(|(&id, _)| (id, sim.vm.world.positions.get(&id))).collect::<Vec<_>>(),
                "stair_uses":sim.vm.threads.values().flat_map(|t| t.stack.iter())
                    .filter(|f| f.action_target.is_some() && f.location.source == STAIR).collect::<Vec<_>>(),
                "route_control":sim.route_control,"route_feedback":sim.route_feedback,
                "sink_use_count":sim.vm.world.objects[&sink.object].data[&62]}));
        }
        write_json(&out.join("diagnostic.json"), &json!({"branch":"cancel-mid-climb",
            "source":source,"recorded_commands":recorded_path,"recorded_commands_sha256":engine::digest(&input),
            "prefix_replayed_unchanged":true,"boundaries":boundaries,
            "first_probe_tick_comparisons":first.tick_comparisons,
            "boundary_replay_tick_comparisons":paired.tick_comparisons,
            "cold_restores":first.cold_restores+paired.cold_restores,
            "preceding_sha256":engine::digest(&previous_bytes),"release_sha256":engine::digest(&released_bytes),
            "strict_native_parity_ticks":0,"native_return_bit_assumed":false}))
    })?.join().expect("first sink release worker panicked")
}
