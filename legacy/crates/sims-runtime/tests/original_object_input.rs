use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private current Mashuga fallback checkpoint; no player save or renderer"]
fn tile_scan_defers_escape_and_restores_its_retained_caller() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_TILE_CALLER_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let index =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let source = repo.join("cache/smoothness-published-20261023/pending-tile-fallback.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    let saved: Saved = serde_json::from_value(value["session"].clone())?;
    let mut results = vec![];
    let mut checkpoints = 0;
    for budget in [1, 10000] {
        let mut s = Session::restore(&index, saved.clone())?;
        let tick = s.simulation.vm.tick;
        let rng = (
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        for _ in 0..1000 {
            if !s.object_input_pending() {
                break;
            }
            ensure!(
                !s.host
                    .presentation
                    .object_input
                    .as_ref()
                    .unwrap()
                    .can_cancel(),
                "Escape entered a synchronous collector"
            );
            let before = serde_json::to_vec(&s.save())?;
            ensure!(s.cancel_object_input().is_err());
            ensure!(
                before == serde_json::to_vec(&s.save())?,
                "rejected Escape changed caller state"
            );
            s.advance_object_input(budget)?;
            if budget == 1 {
                let snapshot = s.save_managed()?;
                s = Session::restore(&index, snapshot)?;
                checkpoints += 1;
            }
        }
        ensure!(!s.object_input_pending());
        ensure!(s.simulation.vm.tick == tick);
        ensure!(
            rng == (
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
                s.simulation.vm.crt_rng_state
            )
        );
        results.push(serde_json::to_vec(&s.save())?);
    }
    ensure!(results[0] == results[1], "cold/sliced tile caller differed");
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
        "source":source,"cold_restores":checkpoints,"escape_deferred":true,"full_session_equal":true,
        "tick_and_all_rng_streams_unchanged":true,"strict_original_ticks_added":0}))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original assets; unsupported person menu retains its owner"]
fn original_unimplemented_person_menu_restores_and_cancels_without_removal() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_OBJECT_BOUNDARY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/newbie-ground-ui-final/session.json"),
    )?)?;
    let saved: Saved = serde_json::from_value(value["session"].clone())?;
    let mut reports = vec![];
    for (target, expected) in [(105, "person menu requires original join enumeration")] {
        let mut s = Session::restore(&index, saved.clone())?;
        s.begin_object_input(target)?;
        ensure!(s.advance_object_input(10000).unwrap_err().to_string() == expected);
        let before = serde_json::to_vec(&s.save())?;
        std::fs::write(output.join(format!("pending-{target}.json")), &before)?;
        s = Session::restore(&index, serde_json::from_slice(&before)?)?;
        ensure!(before == serde_json::to_vec(&s.save())?);
        ensure!(s.advance_object_input(1).unwrap_err().to_string() == expected);
        ensure!(before == serde_json::to_vec(&s.save())?);
        let state = s.save();
        s.cancel_object_input()?;
        ensure!(
            !s.player_input_pending()
                && s.simulation == state.simulation.state
                && s.host.snapshot() == state.assets
                && s.simulation.vm.world.objects.contains_key(&target)
        );
        reports.push(serde_json::json!({"target":target,"error":expected,"restored":true,"blocked_repeat_unchanged":true,"cancel_preserves_vm_and_object":true}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private original Newbie checkpoint; explicit evidence output"]
fn original_object_menu_slices_cancel_and_queue_preserve_full_sessions() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_OBJECT_INPUT_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace object-input evidence")?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/newbie-ground-ui-final/session.json"),
    )?)?;
    let initial: Saved = serde_json::from_value(value["session"].clone())?;
    let mut outcomes = vec![];
    let mut runs = vec![];
    for budget in [10000, 1] {
        let mut s = Session::restore(&index, initial.clone())?;
        let tick = s.simulation.vm.tick;
        let temps = s.simulation.vm.world.objects[&105].temps;
        let position = s.simulation.vm.world.positions[&127];
        let object_count = s.simulation.vm.world.objects.len();
        s.begin_object_input(127)?;
        let mut slices = 0;
        while !s.host.presentation.object_input.as_ref().unwrap().ready() {
            let frozen = serde_json::to_vec(&s.save())?;
            ensure!(s.step().is_err());
            ensure!(s.begin_selection(Some(105)).is_err());
            ensure!(s.begin_ground_input([440, 360], 1).is_err());
            ensure!(frozen == serde_json::to_vec(&s.save())?);
            if budget == 1 && slices < 2 {
                let path = output.join(format!("query-{slices}.json"));
                std::fs::write(&path, &frozen)?;
                s = Session::restore(&index, serde_json::from_slice(&std::fs::read(path)?)?)?;
                ensure!(frozen == serde_json::to_vec(&s.save())?);
            }
            s.advance_object_input(budget)?;
            slices += 1;
            ensure!(slices < 10000);
        }
        ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.objects[&105].temps == temps);
        ensure!(
            s.simulation.vm.world.positions[&127] == position
                && s.simulation.vm.world.objects.len() == object_count
        );
        ensure!(s.host.presentation.picker_target == Some(127));
        ensure!(s.simulation.vm.world.objects[&127].native_flags.unwrap() & 1 != 0);
        let actions = s
            .host
            .presentation
            .object_input
            .as_ref()
            .unwrap()
            .actions()
            .to_vec();
        ensure!(!actions.is_empty() && actions.iter().all(|a| a.priority == 50));
        if budget == 10000 {
            let owner = s.host.presentation.object_input.clone().unwrap();
            for field in ["clock", "caller", "phase", "target", "priority", "flags"] {
                let mut malformed = serde_json::to_value(&owner)?;
                match field {
                    "clock" => malformed["tick"] = (tick + 1).into(),
                    "caller" => malformed["actor"] = 106.into(),
                    "phase" => malformed["phase"] = "PersonJoin".into(),
                    "target" => malformed["target"] = 128.into(),
                    "priority" => malformed["actions"][0]["priority"] = 100.into(),
                    "flags" => malformed["actions"][0]["flags"] = (actions[0].flags ^ 1).into(),
                    _ => unreachable!(),
                }
                s.host.presentation.object_input = Some(serde_json::from_value(malformed)?);
                let frozen = serde_json::to_vec(&s.save())?;
                ensure!(
                    s.choose_object_action(0).is_err(),
                    "accepted malformed {field}"
                );
                ensure!(
                    frozen == serde_json::to_vec(&s.save())?,
                    "mutated rejected {field}"
                );
                if field == "flags" {
                    ensure!(Session::restore(&index, s.save()).is_err());
                }
                s.host.presentation.object_input = Some(owner.clone());
            }
        }
        let ready = s.save();
        std::fs::write(
            output.join(format!("budget-{budget}-menu.json")),
            serde_json::to_vec(&ready)?,
        )?;
        s.cancel_object_input()?;
        ensure!(s.simulation == ready.simulation.state && s.host.snapshot() == ready.assets);
        ensure!(s.host.presentation.object_input.is_none());
        s = Session::restore(&index, ready)?;
        let (_, accepted) = s.choose_object_action(0)?;
        ensure!(accepted && !s.object_input_pending() && s.simulation.vm.tick == tick);
        let saved = s.save();
        let path = output.join(format!("budget-{budget}-queued.json"));
        std::fs::write(&path, serde_json::to_vec(&saved)?)?;
        s = Session::restore(&index, serde_json::from_slice(&std::fs::read(path)?)?)?;
        ensure!(serde_json::to_vec(&saved)? == serde_json::to_vec(&s.save())?);
        runs.push(serde_json::json!({"budget":budget,"slices":slices,"tick":tick,"actions":actions,
            "cancel_keeps_object_and_vm":true,"blocked_ticks_and_inputs_unchanged":true,"full_session_restore":true}));
        outcomes.push(serde_json::to_vec(&s.save())?);
    }
    ensure!(outcomes[0] == outcomes[1]);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "runs":runs,"slice_and_resume_results_identical":true,"native_execution_this_turn":false,
            "malformed_owners_rejected_without_mutation":6,"tampered_flags_restore_rejected":true
        }))?,
    )?;
    Ok(())
}
