use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::original_session::Session;

#[test]
#[ignore = "private original Newbie checkpoint and assets; software rendering only"]
fn original_shower_dialog_constructs_without_ticks_and_restores_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_OBJECT_PREVIEW_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output)?;
    let source = root.join("cache/newbie-live-widget-final/session.json");
    let original = std::fs::read(&source)?;
    let mut saved: serde_json::Value = serde_json::from_slice(&original)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut outcomes = vec![];
    for restoring in [false, true] {
        let mut session =
            Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
        let mut client = Client::restore(
            &session,
            serde_json::from_value(saved["dialog_client"].clone())?,
        )?;
        let before = serde_json::to_value(session.save())?;
        let original_threads = session.simulation.vm.threads.clone();
        let thread = session
            .simulation
            .vm
            .threads
            .iter()
            .find_map(|(&id, t)| {
                matches!(t.status, sims_vm::Status::AwaitingDialog(_)).then_some(id)
            })
            .context("pending dialog absent")?;
        let request = session
            .simulation
            .vm
            .dialog_construction(thread)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            request.frame().location.bhav == 4125
                && request.frame().location.instruction == 12
                && request.frame().stack_object == 107
        );
        let camera = OriginalHouse::camera(&session, 1000, 740, 0, 0.55, 1);
        client.begin(&session, thread, Inputs::camera(camera, 64)?)?;
        ensure!(
            serde_json::to_value(session.save())? == before,
            "begin changed household"
        );
        let (complete, step) = client.advance(&mut session, 0)?;
        ensure!(!complete && step.trace.is_empty() && step.events.is_empty());
        ensure!(
            serde_json::to_value(session.save())? == before,
            "zero budget changed household"
        );
        if restoring {
            let owner = client.save(&session)?;
            session = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
            )?;
            client = Client::restore(
                &session,
                serde_json::from_slice(&serde_json::to_vec(&owner)?)?,
            )?;
        }
        let (complete, step) = client.advance(&mut session, if restoring { 1 } else { 10000 })?;
        ensure!(complete && step.trace.is_empty() && step.events.is_empty());
        ensure!(session.simulation.vm.tick == 193299);
        for (&id, original_thread) in &original_threads {
            if id != thread {
                ensure!(
                    session.simulation.vm.threads.get(&id) == Some(original_thread),
                    "preview changed another VM thread"
                );
            }
        }
        // Constructors may publish dialog/image ownership and wake the waiting
        // thread; no objects, RNG, schedules or motive values may change.
        let mut after = serde_json::to_value(session.save())?;
        let mut expected = before.clone();
        for path in [
            "/simulation/state/vm/world/image_cache",
            "/simulation/state/vm/world/dialogs",
            "/simulation/state/vm/world/ui_owner/dialog_owners",
            "/simulation/state/vm/threads",
        ] {
            *after.pointer_mut(path).context("saved field absent")? = serde_json::Value::Null;
            *expected.pointer_mut(path).context("saved field absent")? = serde_json::Value::Null;
        }
        ensure!(
            after == expected,
            "object preview changed non-dialog simulation state"
        );
        let record = session
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.origin.bhav == 4125 && r.origin.instruction == 12)
            .context("shower record absent")?;
        let image = record.portrait.as_ref().context("object preview absent")?;
        ensure!(image.rect == [0, 0, 120, 120]);
        let raster = client.images().get(image.handle)?;
        ensure!(raster.rect() == [0, 0, 125, 127]);
        let alpha = raster
            .image
            .alpha
            .as_ref()
            .context("software alpha plane absent")?;
        ensure!(alpha[0] == 0 && alpha.iter().filter(|&&a| a != 0).count() > 1000);
        ensure!(
            raster
                .image
                .pixels
                .iter()
                .filter(|&&p| Some(p) != raster.key)
                .count()
                > 1000
        );
        let pixels = raster.rgba()?;
        if restoring {
            pixels.save(output.join("shower-preview.png"))?;
        }
        let s = serde_json::to_value(session.save())?;
        let c = serde_json::to_value(client.save(&session)?)?;
        let restored = Session::restore(&index, serde_json::from_value(s.clone())?)?;
        let restored_client = Client::restore(&restored, serde_json::from_value(c.clone())?)?;
        ensure!(
            s == serde_json::to_value(restored.save())?
                && c == serde_json::to_value(restored_client.save(&restored)?)?
        );
        ensure!(pixels == restored_client.images().get(image.handle)?.rgba()?);
        outcomes.push((s, c, pixels));
    }
    ensure!(outcomes[0] == outcomes[1], "restored constructor differed");
    saved["session"] = outcomes[1].0.clone();
    saved["dialog_client"] = outcomes[1].1.clone();
    std::fs::write(output.join("constructed.json"), serde_json::to_vec(&saved)?)?;
    ensure!(std::fs::read(&source)? == original);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":193299,"construction_published":true,"clock_advanced":false,"source_checkpoint_unchanged":true,
            "non_dialog_simulation_state_equal":true,"pending_and_completed_restore_equal":true,
            "surface":[125,127],"cell":[120,120],"native_execution":false,"native_pixel_equivalence":false,
            "pixel_backend":"Rust software: original SPR2 assets, source-derived placement and fitting; protected native blitter unverified"
        }))?,
    )?;
    Ok(())
}
