//! Independent player-save restoration, distinct from original-file import.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Saved, Session};
use std::path::Path;
#[path = "support/registry_startup.rs"]
mod registry_startup;

fn immutable_installation(source: &Path, destination: &Path) -> Result<()> {
    // Only immutable assets are linked; every UserData namespace is absent.
    // Production readers open these files read-only. Resource modifications
    // belong to their existing in-memory Overlay owners.
    std::fs::create_dir(destination)?;
    fn copy(source: &Path, destination: &Path) -> Result<()> {
        for e in std::fs::read_dir(source)? {
            let e = e?;
            let next = destination.join(e.file_name());
            let kind = e.file_type()?;
            if kind.is_dir() {
                std::fs::create_dir(&next)?;
                copy(&e.path(), &next)?;
            } else {
                ensure!(kind.is_file(), "non-file immutable installation entry");
                std::fs::hard_link(e.path(), next)?;
            }
        }
        Ok(())
    }
    for e in std::fs::read_dir(source)? {
        let e = e?;
        if e.file_name()
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with("userdata")
        {
            continue;
        }
        let next = destination.join(e.file_name());
        if e.file_type()?.is_dir() {
            std::fs::create_dir(&next)?;
            copy(&e.path(), &next)?;
        } else {
            std::fs::hard_link(e.path(), next)?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "private original three-house assets; fresh output SIMS_MANAGED_OUTPUT"]
fn managed_three_houses_restore_without_original_save_namespaces() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("managed save test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_MANAGED_OUTPUT")?);
    ensure!(
        output.starts_with(repo.join("cache")),
        "private output required"
    );
    std::fs::create_dir(&output)?;
    let root = repo.join("game-data/cabinet-extracted");
    let assets_root = output.join("immutable-assets");
    immutable_installation(&root, &assets_root)?;
    let index = Index::scan(&root)?;
    let assets = Index::scan(&assets_root)?;
    ensure!(
        assets
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"))
    );
    let mut reports = vec![];
    for (name, source) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ] {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let before = serde_json::to_vec(&session.save())?;
        let managed = session.save_managed()?;
        ensure!(
            before == serde_json::to_vec(&session.save())?,
            "save capture mutated {name}"
        );
        let encoded = serde_json::to_vec(&managed)?;
        std::fs::write(output.join(format!("{name}.json")), &encoded)?;
        let decoded: Saved = serde_json::from_slice(&encoded)?;
        ensure!(
            Session::restore(&assets, session.save()).is_err(),
            "v1 unexpectedly restored without starting saves"
        );
        let mut restored = Session::restore(&assets, decoded)
            .with_context(|| format!("independent {name} restore"))?;
        ensure!(
            before == serde_json::to_vec(&restored.save())?,
            "managed restoration changed full {name} session"
        );
        let again = restored.save_managed()?;
        ensure!(
            encoded == serde_json::to_vec(&again)?,
            "managed {name} cold resave differs"
        );
        let mut corrupt = managed.clone();
        corrupt.managed.as_mut().unwrap().owned.entries[0].bytes[0] ^= 1;
        ensure!(
            Session::restore(&assets, corrupt).is_err(),
            "corrupt ownership accepted"
        );
        let mut wrong = managed.clone();
        wrong.managed.as_mut().unwrap().catalog.text_language = Some(255);
        ensure!(
            Session::restore(&assets, wrong).is_err(),
            "changed program catalog accepted"
        );
        let mut changed_assets = assets.clone();
        let immutable = session
            .house
            .sources
            .keys()
            .find(|s| !s.to_ascii_lowercase().starts_with("userdata"))
            .context("no immutable import source")?;
        let resource = changed_assets.find(immutable)?;
        let mut bytes = changed_assets.resources[resource].read()?;
        bytes[0] ^= 1;
        changed_assets.resources[resource].owned = Some(std::sync::Arc::from(bytes));
        ensure!(
            Session::restore(&changed_assets, managed.clone()).is_err(),
            "changed immutable import asset accepted"
        );
        // Explicit authored Play/Live inputs match the existing stock scheduler
        // fixture, not an original-engine input capture or a native-parity run.
        for s in [&mut session, &mut restored] {
            s.simulation.vm.world.globals.insert(17, 0);
            s.simulation.vm.world.globals.insert(19, 0);
        }
        let start = session.simulation.vm.tick;
        let mut completed = 0;
        for _ in 0..32 {
            let a = registry_startup::step_startup_inputs(&mut session)?;
            let b = registry_startup::step_startup_inputs(&mut restored)?;
            ensure!(
                a == b && a == sims_runtime::original_session::Progress::Tick,
                "unexpected storage continuation boundary: {a:?}/{b:?}"
            );
            ensure!(
                session.simulation == restored.simulation
                    && session.host.snapshot() == restored.host.snapshot(),
                "independent {name} continuation differs at{}",
                session.simulation.vm.tick
            );
            completed += 1;
            if completed == 16 {
                let checkpoint = restored.save_managed()?;
                std::fs::write(
                    output.join(format!("{name}-tick16.json")),
                    serde_json::to_vec(&checkpoint)?,
                )?;
                restored = Session::restore(
                    &assets,
                    serde_json::from_slice(&serde_json::to_vec(&checkpoint)?)?,
                )?;
                ensure!(
                    session.simulation == restored.simulation,
                    "second independent checkpoint differs"
                );
            }
        }
        ensure!(session.simulation.vm.tick == start + completed);
        reports.push(serde_json::json!({"household":name,"source":source,"tick":session.simulation.vm.tick,
            "owned_resources":managed.managed.as_ref().unwrap().owned.entries.len(),"bytes":encoded.len(),"independent_full_state_equal":true,
            "original_namespaces_physically_absent":true,"original_parity_ticks_added":0,"continuation_ticks_added":0,
            "storage_only_completed_ticks":completed,"storage_only_checkpoint_after":16}));
        eprintln!("{name}: independent restore and exact cold resave pass");
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
