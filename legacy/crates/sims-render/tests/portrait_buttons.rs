use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::original_session::Session;
use std::collections::BTreeSet;

#[test]
#[ignore = "private current Newbie checkpoint and original assets; CPU portrait regeneration"]
fn original_button_portraits_regenerate_and_resume_temporary_person() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source = root.join("cache/newbie-panel-completion/session.json");
    let seed: serde_json::Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    let output = root.join(std::env::var("SIMS_BUTTON_REGEN_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output)?;
    let mut outcomes = vec![];
    let mut reports = vec![];
    for restoring in [false, true] {
        let mut s = Session::restore(&index, serde_json::from_value(seed["session"].clone())?)?;
        let mut client =
            Client::restore(&s, serde_json::from_value(seed["dialog_client"].clone())?)?;
        let tick = s.simulation.vm.tick;
        let visit = s.simulation.visit;
        let visited = s.simulation.visited.clone();
        let ids: BTreeSet<_> = s.simulation.vm.world.objects.keys().copied().collect();
        let betty = s
            .house
            .household
            .as_ref()
            .unwrap()
            .residents
            .iter()
            .find(|r| r.neighbor == 47)
            .context("Betty roster entry absent")?
            .clone();
        ensure!(
            !s.simulation
                .vm
                .world
                .objects
                .values()
                .any(|o| o.definition.as_ref().is_some_and(|d| d.guid == betty.guid))
        );
        // Explicit diagnostic: exercise the engine's real invalidation path,
        // then let4e92a8 regenerate the non-live resident through its native
        // temporary-person services. This is not a normal gameplay frontier.
        ensure!(
            s.simulation
                .vm
                .world
                .image_cache
                .invalidate(betty.guid)
                .map_err(anyhow::Error::msg)?
        );
        let inputs = Inputs::camera(OriginalHouse::native_camera(&s)?, 64)?;
        client.bind_resident_buttons(&mut s, inputs)?;
        let mut boundaries = BTreeSet::new();
        let mut saw_temporary = false;
        let mut restores = 0;
        for _ in 0..20000 {
            let done = client.advance_ui_lookup(&mut s, if restoring { 1 } else { 100_000 })?;
            ensure!(
                s.simulation.vm.tick == tick
                    && s.simulation.visit == visit
                    && s.simulation.visited == visited
            );
            saw_temporary |= s.simulation.portrait_person.is_some();
            let boundary = format!(
                "{done}/{}/{}/{}/{}",
                s.simulation.portrait_person.is_some(),
                s.simulation.person_factories.len(),
                !s.simulation.deletion.is_empty(),
                s.simulation.synchronous_removal.is_some()
            );
            if restoring && boundaries.insert(boundary) {
                let before = serde_json::to_vec(&s.save())?;
                let saved = client.save(&s)?;
                let before_client = serde_json::to_vec(&saved)?;
                s = Session::restore(&index, serde_json::from_slice(&before)?)?;
                client = Client::restore(&s, serde_json::from_slice(&before_client)?)?;
                ensure!(before == serde_json::to_vec(&s.save())?);
                ensure!(before_client == serde_json::to_vec(&client.save(&s)?)?);
                restores += 1;
            }
            if done {
                break;
            }
        }
        ensure!(!client.ui_lookup_pending());
        ensure!(ids == s.simulation.vm.world.objects.keys().copied().collect());
        ensure!(
            s.simulation.portrait_person.is_none() && s.simulation.synchronous_removal.is_none()
        );
        ensure!(client.buttons().unwrap().image(47).is_some());
        if restoring {
            ensure!(
                saw_temporary && restores > 2,
                "temporary services were not exercised"
            );
        }
        outcomes.push((
            serde_json::to_value(s.save())?,
            serde_json::to_value(client.save(&s)?)?,
        ));
        reports.push(serde_json::json!({"restoring":restoring,"tick":tick,"restores":restores,
            "saw_temporary":saw_temporary,"final_objects_unchanged":true,"lot_clock_and_scheduler_unchanged":true}));
        eprintln!("resident portrait regenerate restore={restoring}, {restores} checkpoints");
    }
    ensure!(
        outcomes[0] == outcomes[1],
        "portrait regeneration differs across slice budgets/restores"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source":source,"runs":reports,"identical_final_session_and_client":true,
            "scope":"Explicit dirty-cache diagnostic using original current-program Newbie files. Original temporary person services, no household tick or synthetic tutorial reply."
        }))?,
    )?;
    Ok(())
}
