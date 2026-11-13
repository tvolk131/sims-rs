//! Diagnostic continuation of the actual post-sale managed homes. This is not
//! an original-input replay or a qualifying 4096-tick edit scenario.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Progress, Saved, Session};
use std::{io::Write, path::Path};
#[path = "support/registry_startup.rs"]
mod registry_startup;

fn queue_sit(s: &mut Session, actor: i16, target: i16) -> Result<serde_json::Value> {
    s.simulation
        .vm
        .begin_player_menu(&s.program, actor, target)
        .map_err(anyhow::Error::msg)?;
    let mut actions = None;
    for _ in 0..1000 {
        let (ready, _) = s
            .simulation
            .vm
            .advance_player_menu(&s.program, 128)
            .map_err(anyhow::Error::msg)?;
        if ready.is_some() {
            actions = ready;
            break;
        }
    }
    let actions = actions.context("chair menu query did not return")?;
    for action in actions {
        let label = s
            .simulation
            .vm
            .action_menu(&s.program, action.target)
            .map_err(anyhow::Error::msg)?
            .iter()
            .find(|entry| entry.id == action.menu_id)
            .context("checked action has no original menu entry")?
            .label
            .clone();
        if label.eq_ignore_ascii_case("Sit") {
            let input = serde_json::json!({"actor":actor,"target":target,"action":action,"label":label,
                "scope":"Authored action selection from actual original menu checks, not captured native mouse dispatch."});
            let (_, accepted) = s
                .simulation
                .vm
                .enqueue_action(&s.program, actor, action)
                .map_err(anyhow::Error::msg)?;
            ensure!(accepted, "chair Sit queue rejected");
            return Ok(input);
        }
    }
    anyhow::bail!("no original Sit action for actor{actor} and chair{target}")
}

#[test]
#[ignore = "private managed households and immutable assets; fresh SIMS_ROUTE_POLICY_HOMES_OUTPUT"]
fn three_managed_homes_keep_full_state_across_native_search_policy_and_cold_restore() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("route policy household check panicked")
}

fn same(a: &Session, b: &Session) -> bool {
    a.simulation == b.simulation
        && a.host.snapshot() == b.host.snapshot()
        && a.host.presentation == b.host.presentation
}

#[test]
#[ignore = "private Newbie managed walk state; fresh SIMS_ROUTE_PREFIX_OUTPUT"]
fn routing_callback_observes_cleared_path_and_cold_restore_keeps_entry_inputs() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_prefix)?
        .join()
        .expect("route prefix check panicked")
}
fn run_prefix() -> Result<()> {
    use sims_sim::StepStop;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_ROUTE_PREFIX_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let assets =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    // Start after the previous Sit has completed. At tick64 the actor is
    // still walking to that chair; queueing another Sit does not interrupt
    // that original interaction and cannot guarantee a new route in32ticks.
    let source = repo.join("cache/native-route-policy-homes-20261022-final/newbie/tick-512.json");
    let saved: Saved = serde_json::from_slice(&std::fs::read(&source)?)?;
    let mut s = Session::restore(&assets, saved)?;
    let original_path = s
        .simulation
        .native_lot
        .as_ref()
        .unwrap()
        .route_paths
        .get(&105)
        .cloned();
    ensure!(original_path.as_ref().is_some_and(|p| !p.is_empty()));
    let input = queue_sit(&mut s, 105, 128)?;
    let start = s.simulation.vm.tick;
    let mut captured = None;
    let result: Result<()> = (|| {
        for slice in 0..200000 {
            s.host.update_contexts(&mut s.simulation)?;
            // Session::step services pending host requests before examining
            // the scheduler result. A resource request can temporarily return
            // an error which that service resolves; don't invert this order
            // merely because the diagnostic instruction budget is one.
            let result = s.simulation.run_current_tick(&s.program, 1);
            s.host.presentation.selected = s
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?;
            let observed = s
                .simulation
                .native_lot
                .as_ref()
                .and_then(|n| n.route.as_ref())
                .filter(|e| {
                    e.potential.is_none()
                        && e.path.as_ref().is_some_and(|p| p.points.is_none())
                        && e.obstacles.as_ref().is_some_and(|o| {
                            s.simulation.vm.threads.iter().any(|(&id, t)| {
                                (0..t.stack.len()).any(|depth| {
                                    o.owns_callback(&s.simulation.vm, &s.program, id, depth)
                                })
                            })
                        })
                });
            if let Some(e) = observed {
                ensure!(
                    s.simulation
                        .native_lot
                        .as_ref()
                        .unwrap()
                        .route_paths
                        .get(&e.caller)
                        .is_some_and(Vec::is_empty),
                    "callback entered before the caller cleared its previous path"
                );
                let entry = e
                    .path
                    .as_ref()
                    .unwrap()
                    .search_actor
                    .context("entry inputs absent")?;
                captured = Some((slice, e.caller, entry));
                break;
            }
            if s.host.service(&mut s.simulation, &mut s.program)? > 0 {
                continue;
            }
            let step = result.map_err(anyhow::Error::msg)?;
            match step.stop {
                StepStop::Budget => (),
                StepStop::Dialog | StepStop::Faulted => {
                    anyhow::bail!("unexpected prefix stop: {:?}", step.stop)
                }
                _ => {
                    s.host.present_notifications(
                        s.simulation.vm.tick as u32,
                        &s.simulation.characters.keys().copied().collect::<Vec<_>>(),
                    )?;
                    s.simulation
                        .finish_registry_pass()
                        .map_err(anyhow::Error::msg)?;
                    s.simulation
                        .update_original_environment(&s.program)
                        .map_err(anyhow::Error::msg)?;
                }
            }
            ensure!(
                s.simulation.vm.tick < start + 512,
                "no owned routing callback in 512 completed ticks"
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        std::fs::write(output.join("failure.json"), serde_json::to_vec(&s.save())?)?;
        std::fs::write(output.join("failure.txt"), format!("{error:#}\n"))?;
        return Err(error);
    }
    let (slice, actor, entry) = captured.context("routing callback budget exceeded")?;
    let saved = s.save_managed()?;
    std::fs::write(output.join("pending.json"), serde_json::to_vec(&saved)?)?;
    let mut cold = Session::restore(
        &assets,
        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
    )?;
    ensure!(same(&s, &cold), "cold pending callback differs");
    let a = s.step()?;
    let b = cold.step()?;
    ensure!(
        a == b && a == Progress::Tick && same(&s, &cold),
        "resumed callback/tick state differs: {a:?}/{b:?}"
    );
    std::fs::write(
        output.join("continued.json"),
        serde_json::to_vec(&s.save_managed()?)?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"input":input,"captured_slice":slice,"actor":actor,"entry_inputs":entry,
            "previous_path":original_path,"path_empty_before_callback":true,"pending_cold_equal":true,
            "completed_tick_after_restore":s.simulation.vm.tick,"full_state_after_tick_equal":true,
            "strict_original_parity_ticks_added":0,"qualifying_4096_scenarios_added":0
        }))?,
    )?;
    Ok(())
}

fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_ROUTE_POLICY_HOMES_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let count: u64 = std::env::var("SIMS_ROUTE_POLICY_HOMES_TICKS")
        .unwrap_or_else(|_| "128".into())
        .parse()?;
    ensure!((1..=4096).contains(&count));
    let queue = std::env::var("SIMS_ROUTE_POLICY_QUEUE_SIT").as_deref() == Ok("1");
    let assets_root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let assets = Index::scan(&assets_root)?;
    ensure!(
        assets
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"))
    );
    let mut summaries = vec![];
    for name in ["newbie", "goth", "mashuga"] {
        let dir = output.join(name);
        std::fs::create_dir(&dir)?;
        let source = repo
            .join("cache/sale-20261022-seventh")
            .join(name)
            .join("sold-returned-live.json");
        let bundle: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
        let saved: Saved = serde_json::from_value(bundle["session"].clone())?;
        let actor = bundle["resident"]
            .as_i64()
            .context("frontend resident absent")? as i16;
        let chair = match name {
            "newbie" => 323,
            "goth" => 387,
            "mashuga" => 342,
            _ => unreachable!(),
        };
        ensure!(saved.version == 2);
        let mut a = Session::restore(&assets, saved)?;
        ensure!(a.simulation.room_environment.is_some());
        ensure!(!a.player_input_blocks_ticks() && !a.clock_pending());
        // An explicit controlled test input, not native UI dispatch evidence.
        // Direct stepping deliberately tests the scheduler rather than the
        // frontend's elapsed-clock/input producer, which has its own checks.
        let old_pause = a.simulation.vm.world.globals[&17];
        a.simulation.vm.world.globals.insert(17, 0);
        let initial = a.save_managed()?;
        std::fs::write(dir.join("entry.json"), serde_json::to_vec(&initial)?)?;
        let mut b = Session::restore(
            &assets,
            serde_json::from_slice(&serde_json::to_vec(&initial)?)?,
        )?;
        ensure!(same(&a, &b));
        let start = a.simulation.vm.tick;
        let mut changes = 0;
        let mut restores = 1;
        let mut log = std::fs::File::create_new(dir.join("ticks.jsonl"))?;
        let mut inputs = std::fs::File::create_new(dir.join("inputs.jsonl"))?;
        let begun = std::time::Instant::now();
        let result: Result<()> = (|| {
            for n in 1..=count {
                let before_paths = a
                    .simulation
                    .native_lot
                    .as_ref()
                    .unwrap()
                    .route_paths
                    .clone();
                let mut completed = None;
                for _ in 0..20 {
                    let pa = a.step().with_context(|| {
                        format!("{name}: warm step {n} tick{}", a.simulation.vm.tick)
                    })?;
                    let pb = b.step().with_context(|| {
                        format!("{name}: cold step {n} tick{}", b.simulation.vm.tick)
                    })?;
                    ensure!(
                        pa == pb && same(&a, &b),
                        "{name}: pending service state differs"
                    );
                    if pa == Progress::Dialog {
                        let record = a
                            .simulation
                            .vm
                            .world
                            .dialogs
                            .records
                            .iter()
                            .find(|r| Some(r.id) == a.simulation.vm.world.dialogs.modal)
                            .context("modal record absent")?;
                        writeln!(
                            inputs,
                            "{}",
                            serde_json::json!({"tick":a.simulation.vm.tick,"record":record,
                            "policy":"Explicit stock startup notice OK / Help No only; unrelated dialogs fail without a response."})
                        )?;
                        inputs.flush()?;
                        registry_startup::answer_startup_dialog(&mut a)?;
                        registry_startup::answer_startup_dialog(&mut b)?;
                        ensure!(same(&a, &b));
                    } else {
                        completed = Some((pa, pb));
                        break;
                    }
                }
                let (pa, pb) = completed.context("startup dialog budget exceeded")?;
                ensure!(
                    pa == pb && pa == Progress::Tick,
                    "{name}: expected completed ticks: {pa:?}/{pb:?}"
                );
                ensure!(
                    a.simulation.vm.tick == start + n,
                    "{name}: missing completed tick"
                );
                ensure!(
                    same(&a, &b),
                    "{name}: first full-state divergence at{}",
                    a.simulation.vm.tick
                );
                changes += usize::from(
                    before_paths != a.simulation.native_lot.as_ref().unwrap().route_paths,
                );
                writeln!(
                    log,
                    "{}",
                    serde_json::json!({"tick":a.simulation.vm.tick,"full_state_equal":true,
                    "rng":[a.simulation.vm.rng_state,a.simulation.vm.lot_rng_state,a.simulation.vm.crt_rng_state],
                    "path_changes":changes})
                )?;
                if queue && n == 1 {
                    let qa = queue_sit(&mut a, actor, chair)?;
                    let qb = queue_sit(&mut b, actor, chair)?;
                    ensure!(
                        qa == qb && same(&a, &b),
                        "{name}: chair action input differs"
                    );
                    writeln!(
                        inputs,
                        "{}",
                        serde_json::json!({"tick":a.simulation.vm.tick,"queue":qa})
                    )?;
                    inputs.flush()?;
                }
                if n % 64 == 0 {
                    let save = a.save_managed()?;
                    let encoded = serde_json::to_vec(&save)?;
                    std::fs::write(dir.join(format!("tick-{n}.json")), &encoded)?;
                    b = Session::restore(&assets, serde_json::from_slice(&encoded)?)?;
                    restores += 1;
                    ensure!(same(&a, &b), "{name}: cold checkpoint differs");
                }
                if n % 128 == 0 {
                    println!(
                        "{name}: {n}/{count} ticks, {changes} path changes, {restores} independent restores"
                    );
                }
            }
            Ok(())
        })();
        log.flush()?;
        if let Err(error) = result {
            std::fs::write(
                dir.join("failure-warm.json"),
                serde_json::to_vec(&a.save())?,
            )?;
            std::fs::write(
                dir.join("failure-cold.json"),
                serde_json::to_vec(&b.save())?,
            )?;
            std::fs::write(dir.join("failure.txt"), format!("{error:#}\n"))?;
            return Err(error);
        }
        let report = serde_json::json!({"household":name,"source":source,"start_tick":start,
            "end_tick":a.simulation.vm.tick,"completed_ticks":count,"path_changes":changes,
            "independent_restores":restores,"full_state_every_tick":true,"seconds":begun.elapsed().as_secs_f64(),
            "controlled_pause_input":[old_pause,0],"authored_sit_input":queue,"immutable_assets":assets_root,
            "strict_original_parity_ticks_added":0,"qualifying_4096_edit_scenarios_added":0,
            "scope":"Diagnostic scheduler continuation after actual edits; original UI/elapsed-clock capture and the complete qualifying edit scenario are not covered."});
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
        summaries.push(report);
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&summaries)?,
    )?;
    Ok(())
}
