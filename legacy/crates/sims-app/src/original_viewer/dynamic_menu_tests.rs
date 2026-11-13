//! Actual preexisting meals; no access to the player's save or live GUI.
use super::*;
use clap::Parser;
use serde_json::json;

#[test]
#[ignore = "independent Mashuga checkpoint; SIMS_DYNAMIC_MENU_OUTPUT and headless Metal"]
fn actual_meal_menu_publishes_dynamic_entries_and_resumes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("meal menu test panicked")
}
fn finish(s: &mut Session) -> Result<()> {
    for _ in 0..10000 {
        if s.host
            .presentation
            .object_input
            .as_ref()
            .is_none_or(|p| p.ready())
        {
            return Ok(());
        }
        s.advance_object_input(1)?;
    }
    anyhow::bail!("meal query did not finish")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let source = repo.join("cache/no-selected-person-iced-20261023-third/no-selected-person.json");
    let out = repo.join(std::env::var("SIMS_DYNAMIC_MENU_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")));
    std::fs::create_dir(&out)?;
    let path = out.join("base.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData2/Houses/House09.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        path.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let mut inputs = vec![];
    transport_tests::drain(&mut v, &mut inputs)?;
    let _ = v.update(Message::Resident(v.resident));
    transport_tests::drain(&mut v, &mut inputs)?;
    v.save()?;
    let index = v.session.as_ref().unwrap().index().clone();
    let mut reports = vec![];
    for target in [197, 203, 204] {
        v.load(&path)?;
        let s = v.session.as_mut().unwrap();
        let before = s.simulation.vm.clone();
        ensure!(s.begin_object_input(target)?);
        let mut cuts = 0;
        for _ in 0..10000 {
            if s.host
                .presentation
                .object_input
                .as_ref()
                .is_none_or(|p| p.ready())
            {
                break;
            }
            s.advance_object_input(1)?;
            if cuts == 0
                && s.simulation.vm.player_menu.as_ref().is_some_and(|q| {
                    serde_json::to_value(q).unwrap()["actions"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                })
            {
                let saved = s.save_managed()?;
                std::fs::write(
                    out.join(format!("meal-{target}-pending.json")),
                    serde_json::to_vec(&saved)?,
                )?;
                let mut cold = Session::restore(&index, saved)?;
                finish(&mut cold)?;
                finish(s)?;
                ensure!(
                    serde_json::to_vec(&s.save())? == serde_json::to_vec(&cold.save())?,
                    "partial menu cold restore differs"
                );
                cuts += 1;
            }
        }
        ensure!(cuts == 1, "did not retain a post-publication cut");
        let actions = s
            .host
            .presentation
            .object_input
            .as_ref()
            .context("meal menu closed")?
            .actions()
            .to_vec();
        let eat = actions
            .iter()
            .position(|a| {
                matches!(
                    a.label.as_deref(),
                    Some("Prepare" | "Cook" | "Serve" | "Eat" | "Grab a Plate")
                )
            })
            .with_context(|| format!("dynamic food entry absent: {actions:?}"))?;
        ensure!(actions[eat].args == [target, 0, 0, 0]);
        ensure!(
            (
                s.simulation.vm.tick,
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
                s.simulation.vm.crt_rng_state
            ) == (
                before.tick,
                before.rng_state,
                before.lot_rng_state,
                before.crt_rng_state
            )
        );
        ensure!(
            s.simulation.vm.world.actions == before.world.actions,
            "menu query changed action queue"
        );
        ensure!(
            s.simulation.vm.world.objects[&v.resident].temps
                == before.world.objects[&v.resident].temps
        );
        v.refresh()?;
        v.args.save = out.join(format!("meal-{target}-ready.json"));
        v.save()?;
        let ready_path = v.args.save.clone();
        let ready_state = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
        v.load(&ready_path)?;
        v.refresh()?;
        ensure!(ready_state == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
        if target == 203 {
            render_headless_element(&root, &out.join("food-menu.png"), v.view())?;
            ensure!(
                ready_state == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
                "render progressed menu/VM"
            );
        }
        let _ = v.update(Message::ObjectChoice(eat));
        transport_tests::drain(&mut v, &mut inputs)?;
        ensure!(v.error.is_none(), "{:?}", v.error);
        let actor = &v
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .unwrap()
            .actors[&v.resident];
        ensure!(
            actor
                .queue
                .pending
                .iter()
                .chain(actor.active.iter())
                .any(|a| a.target == target
                    && a.args == [target, 0, 0, 0]
                    && a.label == actions[eat].label),
            "dynamic action was not enqueued"
        );
        v.load(&ready_path)?;
        let _ = v.update(Message::ObjectCancel);
        transport_tests::drain(&mut v, &mut inputs)?;
        ensure!(v.error.is_none() && !v.session.as_ref().unwrap().running_menu());
        reports.push(json!({"target":target,"actions":actions,"post_publication_cold_restore":true,"ready_cold_restore":true,"enqueue_and_cancel":true,"rng_and_tick_unchanged":true}));
        v.args.save = path.clone();
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"source":source,"cases":reports,"player_save_accessed":false,"new_strict_native_ticks":0}),
        )?,
    )?;
    Ok(())
}
