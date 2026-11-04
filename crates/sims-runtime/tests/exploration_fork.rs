//! Private whole-session branch isolation, including a cold comparison.
use anyhow::{Result, ensure};
use sims_content::Index;
use sims_runtime::Session;

#[test]
#[ignore = "three independent private checkpoints; SIMS_EXPLORATION_FORK_OUTPUT"]
fn trusted_forks_preserve_state_and_isolate_all_mutable_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .unwrap()
}
fn run() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_EXPLORATION_FORK_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let index =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let mut reports = vec![];
    for house in ["newbie", "goth", "mashuga"] {
        let path = repo.join(format!(
            "cache/managed-independent-20261022-third/{house}-tick16.json"
        ));
        let saved = serde_json::from_slice(&std::fs::read(path)?)?;
        let s = Session::restore(&index, saved)?;
        let before = serde_json::to_vec(&s.save())?;
        let mut a = s.fork_for_exploration()?;
        let b = s.fork_for_exploration()?;
        ensure!(
            serde_json::to_vec(&a.save())? == before && serde_json::to_vec(&b.save())? == before
        );
        let cold = Session::restore(&index, a.save_managed()?)?;
        ensure!(serde_json::to_vec(&cold.save())? == before);
        a.simulation.vm.rng_state ^= 0x1234;
        a.host.presentation.viewport[2] += 1;
        a.program.animations.clear();
        a.host.mesh_pool = Default::default();
        ensure!(
            serde_json::to_vec(&s.save())? == before && serde_json::to_vec(&b.save())? == before,
            "fork mutation leaked into source or sibling"
        );
        reports.push(serde_json::json!({"house":house,"same_snapshot":true,"cold_same":true,"independent_mutations":true}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
