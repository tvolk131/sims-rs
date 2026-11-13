use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Progress, Saved, Session};
use sims_sim::pose::movement::route::Stop;
use std::path::Path;

#[test]
#[ignore = "private Newbie replay checkpoint and immutable asset installation"]
fn captured_newbie_walk_refresh_survives_independent_restore_and_real_person_update() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("walk refresh test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_REFRESH_WALK_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let baseline =
        repo.join("cache/verified-input-routing-20261021/replay-inferred/final-session.json");
    let source: Saved = serde_json::from_slice(&std::fs::read(&baseline)?)?;
    // This replay came from the preserved Windows save, not stock UserData.
    // Version1 migration must validate against that exact starting input.
    let full = Index::scan(&repo.join("cache/wine-parity-import-1/assets"))?;
    let mut a = Session::restore(&full, source)?;
    let actor = 105;
    let initial = a
        .simulation
        .characters
        .get(&actor)
        .and_then(|c| c.native_walk.as_ref())
        .context("expected captured Bob walking continuation")?
        .clone();
    let tick = a.simulation.vm.tick;
    ensure!(initial.request == 60 && initial.thread == 283);
    let threads = a.simulation.vm.threads.clone();
    let positions = a.simulation.vm.world.positions.clone();
    let actions = a.simulation.vm.world.actions.clone();
    a.refresh_original_house()?;
    ensure!(a.simulation.characters[&actor].native_walk.as_ref() == Some(&initial));
    ensure!(
        !a.simulation.route_control[&initial.request]
            .original
            .as_ref()
            .unwrap()
            .cache_valid
    );
    ensure!(
        a.simulation.vm.threads == threads
            && a.simulation.vm.world.positions == positions
            && a.simulation.vm.world.actions == actions
    );
    let saved = a.save_managed()?;
    std::fs::write(output.join("invalidated.json"), serde_json::to_vec(&saved)?)?;
    let assets =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let mut b = Session::restore(&assets, saved)?;
    ensure!(
        serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?,
        "independent invalidated restore differs"
    );
    let pa = a.step()?;
    let pb = b.step()?;
    ensure!(
        matches!(pa, Progress::Tick) && matches!(pb, Progress::Tick),
        "expected one completed person-update tick: {pa:?}, {pb:?}"
    );
    ensure!(a.simulation.vm.tick == tick + 1 && b.simulation.vm.tick == tick + 1);
    ensure!(
        serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?,
        "independent first-tick state/RNG differs"
    );
    let walk = a.simulation.characters[&actor]
        .native_walk
        .as_ref()
        .context("expected suspended stop after first tick")?;
    ensure!(walk.stopping == Some(Stop::Refresh));
    ensure!(
        a.simulation.vm.threads[&initial.thread]
            .stack
            .last()
            .unwrap()
            .continuation
            == 15
    );
    ensure!(
        a.simulation.route_control[&initial.request]
            .original
            .as_ref()
            .unwrap()
            .cache_valid
    );
    let saved = a.save_managed()?;
    std::fs::write(output.join("stopping.json"), serde_json::to_vec(&saved)?)?;
    let restored = Session::restore(&assets, saved)?;
    ensure!(
        serde_json::to_vec(&a.save())? == serde_json::to_vec(&restored.save())?,
        "independent state15 restore differs"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":baseline,"before_tick":tick,"after_tick":a.simulation.vm.tick,"actor":actor,
            "request":initial.request,"thread":initial.thread,"full_restore_equal":true,
            "one_completed_rust_tick_equal":true,"stopping_continuation":15,"state15_restore_equal":true,
            "original_gameplay_ticks_added":0,"qualifying_4096_scenarios":0,
            "scope":"Rust replay-derived active walk, supplied refresh call, independent cold restore and actual scheduled person update. This uses the existing inferred path; it is not original-engine parity or a complete Buy/Live loop."
        }))?,
    )?;
    Ok(())
}
