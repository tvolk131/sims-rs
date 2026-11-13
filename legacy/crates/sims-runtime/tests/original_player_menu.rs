use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original Newbie checkpoint and assets"]
fn original_go_here_and_chair_live_menu_checks_resume_without_lot_ticks() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_PLAYER_MENU_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace player-menu evidence")?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/newbie-person-button-click-cold/session.json"),
    )?)?;
    let initial: Saved = serde_json::from_value(input["session"].clone())?;
    let actor = input["resident"]
        .as_i64()
        .context("selected resident absent")? as i16;
    let mut reports = vec![];
    // These are original loaded objects, including the lot's existing hidden
    // destination. No ground click, new helper construction or movement is claimed.
    for target in [139, 127] {
        let mut outcomes = vec![];
        for budget in [10000, 1] {
            let mut s = Session::restore(&index, initial.clone())?;
            let before = s.save();
            s.simulation
                .vm
                .begin_player_menu(&s.program, actor, target)
                .map_err(anyhow::Error::msg)?;
            let mut instructions = 0;
            let mut restores = 0;
            let mut slices = 0;
            let mut trace = vec![];
            let actions = loop {
                slices += 1;
                ensure!(slices < 10000, "player-menu diagnostic budget exceeded");
                let frozen = serde_json::to_vec(&s.simulation)?;
                ensure!(s.simulation.finish_registry_pass().is_err());
                ensure!(s.simulation.run_current_tick(&s.program, 1000).is_err());
                ensure!(frozen == serde_json::to_vec(&s.simulation)?);
                let (done, run) = s
                    .simulation
                    .vm
                    .advance_player_menu(&s.program, budget)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("target {target}, slice {slices}"))?;
                instructions += run.trace.len();
                trace.extend(run.trace);
                if let Some(actions) = done {
                    break actions;
                }
                if budget == 1 && restores < 2 {
                    let saved = s.save();
                    let file = output.join(format!("target-{target}-pending-{restores}.json"));
                    std::fs::write(&file, serde_json::to_vec(&saved)?)?;
                    s = Session::restore(&index, serde_json::from_slice(&std::fs::read(&file)?)?)?;
                    ensure!(
                        s.simulation == saved.simulation.state && s.host.snapshot() == saved.assets
                    );
                    restores += 1;
                }
            };
            ensure!(s.simulation.vm.tick == before.simulation.state.vm.tick);
            ensure!(
                s.simulation.visit == before.simulation.state.visit
                    && s.simulation.visited == before.simulation.state.visited
            );
            ensure!(s.simulation.vm.world.actions == before.simulation.state.vm.world.actions);
            ensure!(
                s.simulation.vm.world.objects[&actor].temps
                    == before.simulation.state.vm.world.objects[&actor].temps
            );
            if target == 139 {
                ensure!(
                    s.simulation.vm.world.objects[&target]
                        .definition
                        .as_ref()
                        .unwrap()
                        .guid
                        == 0x7c4
                );
                // The original lot's existing destination is already used.
                // BHAV4109 must reject it; do not reset its attribute to make
                // this look like a newly constructed ground-click helper.
                ensure!(s.simulation.vm.world.objects[&target].attributes[0] == 1);
                ensure!(!actions.iter().any(|a| a.menu_id == 2));
                ensure!(!actions.iter().any(|a| a.menu_id == 3));
            }
            ensure!(actions.iter().all(|a| a.priority == 50 && a.id == 0));
            let saved = s.save();
            let restored = Session::restore(&index, saved.clone())?;
            ensure!(
                restored.simulation == s.simulation
                    && restored.host.snapshot() == s.host.snapshot()
            );
            let report = serde_json::json!({"target":target,"budget":budget,"slices":slices,
                "instructions":instructions,"restores":restores,"tick":s.simulation.vm.tick,
                "actions":actions,"full_session_restore":true,"temps_restored":true,
                "ordinary_ticks_rejected_without_mutation":true});
            println!("{report}");
            std::fs::write(
                output.join(format!("target-{target}-budget-{budget}-trace.json")),
                serde_json::to_vec(&trace)?,
            )?;
            outcomes.push((saved, actions, trace));
            reports.push(report);
        }
        ensure!(outcomes[0].0.simulation == outcomes[1].0.simulation);
        ensure!(outcomes[0].0.assets == outcomes[1].0.assets);
        ensure!(outcomes[0].1 == outcomes[1].1 && outcomes[0].2 == outcomes[1].2);
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "runs":reports,"slice_and_resume_results_identical":true,
            "scope":"Original live ordinary player-menu checks only. No ground click, popup queue insertion, helper creation or tutorial advancement.",
            "native_execution_this_turn":false
        }))?,
    )?;
    Ok(())
}
