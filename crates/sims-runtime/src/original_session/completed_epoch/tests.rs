use super::*;
use sims_fixtures::Store;
use std::{fs, path::Path};

#[test]
#[ignore = "private actual completed Goth Session and frontend readsets"]
fn completed_root_contract_rejects_uncompared_camera_and_extra_callers() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("completed contract test worker panicked"))?
}
fn failed(report: &Value, field: &str) -> bool {
    report["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["field"] == field && r["matched"] == false)
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(root.join(p))?)?) };
    let p = "fixtures/private/goth-editor-settings-20261029-first/session-first";
    let refs = read(&format!("{p}/refs.json"))?;
    let source = read(
        "fixtures/private/goth-session-contract-20261029-first/native-unchanged-frontend.json",
    )?;
    let closure = read(
        "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
    )?;
    let store = Store::open(root.join(format!("{p}/store")))?;
    let saved: Saved = store.get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let effect_source =
        read("fixtures/private/goth-session-contract-20261029-first/effect-proof-first.json")?;
    let effects = effects::InactiveEffects::import(effect_source.clone())?;
    let original = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let report = view(&saved, &source, &closure["source_closure"])?;
    ensure!(
        report["difference_count"] == 0 && report["comparisons"].as_array().unwrap().len() == 43,
        "actual View contract failed"
    );
    let sim = simulation::evaluate(&saved)?;
    ensure!(
        sim["difference_count"] == 0 && sim["comparisons"].as_array().unwrap().len() == 53,
        "actual Simulation wrapper failed"
    );
    let asset = assets::evaluate(&saved, &effects)?;
    ensure!(
        asset["difference_count"] == 0 && asset["comparisons"].as_array().unwrap().len() == 16,
        "actual asset wrapper failed"
    );
    for (report, count) in [
        (world::evaluate(&saved)?, 27),
        (options::evaluate(&saved)?, 8),
        (vm::evaluate(&saved)?, 17),
        (saved::evaluate(&saved)?, 13),
    ] {
        ensure!(
            report["difference_count"] == 0
                && report["comparisons"].as_array().unwrap().len() == count,
            "actual root contract failed: {report}"
        );
    }
    let mut wrong = saved.clone();
    wrong.view.camera.as_mut().unwrap().center[0] ^= 1;
    let report = view(&wrong, &source, &closure["source_closure"])?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "camera"),
        "uncompared duplicate portrait camera accepted"
    );
    let mut wrong = saved.clone();
    wrong.view.ui_portrait_tick = Some(wrong.simulation.state.vm.tick);
    let report = view(&wrong, &source, &closure["source_closure"])?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "ui_portrait_tick"),
        "new portrait caller was silently accepted"
    );
    let mut wrong = saved.clone();
    wrong.simulation.state.visit = Some((0, None));
    let report = simulation::evaluate(&wrong)?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "visit"),
        "new registry caller was silently accepted"
    );
    let mut wrong = saved.clone();
    wrong.assets.skeletons = None;
    let report = assets::evaluate(&wrong, &effects)?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "skeletons"),
        "unknown native prototype cache accepted"
    );
    let mut wrong = saved.clone();
    wrong.simulation.state.vm.world.actions = None;
    let report = world::evaluate(&wrong)?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "actions"),
        "missing native action owner accepted"
    );
    let mut wrong = saved.clone();
    wrong.options.native_postload = true;
    let report = options::evaluate(&wrong)?;
    ensure!(
        report["difference_count"] == 1 && failed(&report, "native_postload"),
        "new postload epoch accepted"
    );
    let mut wrong = saved.clone();
    wrong
        .simulation
        .state
        .vm
        .threads
        .values_mut()
        .next()
        .unwrap()
        .callback_result = Some(false);
    ensure!(
        vm::evaluate(&wrong)?["difference_count"] == 1,
        "extra replacement callback result accepted"
    );
    let endpoint = read(&format!("{p}/managed-endpoint.json"))?;
    let after: Saved = store.get_value(&serde_json::from_value(endpoint["session"].clone())?)?;
    let native_code = read(
        "fixtures/private/goth-original-replay-20261028-first/exact-cold-second/native-bytecode.json",
    )?;
    let native_threads =
        read("fixtures/private/goth-supplemental-owners-20261029-first/frames-before-first.json")?;
    let native_threads_after =
        read("fixtures/private/goth-supplemental-owners-20261029-first/frames-after-first.json")?;
    for (state, source) in [(&saved, &native_threads), (&after, &native_threads_after)] {
        ensure!(
            thread_owners::evaluate(state, closure.clone(), source, &native_code)?["difference_count"]
                == 0,
            "actual source thread/delivery contract failed"
        );
    }
    let mut wrong = saved.clone();
    wrong
        .simulation
        .state
        .vm
        .world
        .objects
        .get_mut(&1)
        .unwrap()
        .last_update = Some(88448);
    ensure!(
        thread_owners::evaluate(&wrong, closure.clone(), &native_threads, &native_code)?["difference_count"]
            == 1,
        "previously excluded update timestamp silently accepted"
    );
    let mut wrong = saved.clone();
    wrong
        .simulation
        .state
        .vm
        .threads
        .get_mut(&16)
        .unwrap()
        .interrupted = true;
    ensure!(
        thread_owners::evaluate(&wrong, closure.clone(), &native_threads, &native_code)?["difference_count"]
            == 1,
        "changed thread control silently accepted"
    );
    let mut wrong = after.clone();
    let sims_vm::Status::AwaitingAnimation(request) = &mut wrong
        .simulation
        .state
        .vm
        .threads
        .get_mut(&16)
        .unwrap()
        .status
    else {
        anyhow::bail!("animation continuation absent")
    };
    request.operands[0] ^= 1;
    ensure!(
        thread_owners::evaluate(&wrong, closure.clone(), &native_threads_after, &native_code)?["difference_count"]
            == 1,
        "changed pending animation payload silently accepted"
    );
    let mut wrong = saved.clone();
    let mut t = serde_json::to_value(&wrong.simulation.state.vm.threads[&16])?;
    let binding = t["retained_frame_arena"]["bindings"]["16"]["native_context"]
        .as_u64()
        .context("retained binding absent")?;
    t["retained_frame_arena"]["bindings"]["16"]["native_context"] = (binding ^ 16).into();
    wrong
        .simulation
        .state
        .vm
        .threads
        .insert(16, serde_json::from_value(t)?);
    ensure!(
        thread_owners::evaluate(&wrong, closure.clone(), &native_threads, &native_code)?["difference_count"]
            == 1,
        "changed native context annotation silently accepted"
    );
    for report in [
        view(&after, &source, &closure["source_closure"])?,
        simulation::evaluate(&after)?,
        assets::evaluate(&after, &effects)?,
        world::evaluate(&after)?,
        options::evaluate(&after)?,
        vm::evaluate(&after)?,
        saved::evaluate(&after)?,
    ] {
        ensure!(
            report["difference_count"] == 0,
            "actual endpoint root contract differs: {report}"
        );
        ensure!(
            report["complete_native_Session_admitted"] == false,
            "root shape promoted to native parity"
        );
    }
    ensure!(
        postcard::to_allocvec(&saved.simulation.state.vm)? == original,
        "read-only root audit changed VM/RNG"
    );
    let mut bad = effect_source;
    bad["facts"]["consumers"][2]["instruction_visits"] = 1.into();
    ensure!(
        effects::InactiveEffects::import(bad).is_err(),
        "reached native effect consumer accepted"
    );
    let mut wrong = saved.clone();
    wrong
        .simulation
        .state
        .vm
        .world
        .objects
        .get_mut(&16)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(1, 1);
    ensure!(
        effects.require_current_inputs(&wrong).is_err(),
        "notification falsely suppressed by unbound renderer"
    );
    Ok(())
}
