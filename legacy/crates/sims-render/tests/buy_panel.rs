//! Complete the actual retained Goth50a8ec call through replacement widgets.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, publication::Progress};

#[test]
#[ignore = "private original Goth checkpoint; release/headless"]
fn goth_buy_publication_completes_with_saved_catalog_and_unchanged_vm() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("Buy panel panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_BUY_PANEL_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-panel-collapse-frames-2/session.json"),
    )?)?;
    let mut s = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
    let mut d = Device::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
    let before = serde_json::to_value(s.save())?;
    let pixels = software::render(d.scene());
    d.bind_panel_catalog(&s, 8)?;
    ensure!(d.bind_panel_catalog(&s, 8).is_err());
    ensure!(serde_json::to_value(s.save())? == before);
    // One step detaches the Build inspector. Save before Buy refresh so a cold
    // restore must still populate the actual widget inventory and return it.
    ensure!(d.advance_publication(&mut s, 1)? == Progress::Pending);
    let mid = json!({"session":s.save(),"camera_device":d.save(&s)?});
    let mut results = vec![];
    for resumed in [false, true] {
        if resumed {
            s = Session::restore(&index, serde_json::from_value(mid["session"].clone())?)?;
            d = Device::restore(&s, serde_json::from_value(mid["camera_device"].clone())?)?;
            ensure!(mid == json!({"session":s.save(),"camera_device":d.save(&s)?}));
        }
        for n in 0..80 {
            match d.advance_publication(&mut s, if resumed { 1 } else { 256 })? {
                Progress::Complete => break,
                Progress::Pending => ensure!(n < 79, "Buy publication stalled"),
                other => anyhow::bail!("Buy publication did not return: {other:?}"),
            }
        }
        ensure!(!s.publication_pending());
        let after = serde_json::to_value(s.save())?;
        let mut expected = before.clone();
        expected["view"]["publication"] = after["view"]["publication"].clone();
        expected["view"]["transport"]["controls"]["buy"]["dirty"] = json!(0);
        expected["view"]["transport"]["controls"]["publication"]["last"] =
            after["view"]["transport"]["controls"]["publication"]["last"].clone();
        ensure!(
            expected == after,
            "Buy refresh changed state outside its frame fields"
        );
        ensure!(after["view"]["publication"]["frame"]["stage"] == "Complete");
        ensure!(software::render(d.scene()) == pixels);
        let c = d.panel_widgets().unwrap().catalog().unwrap();
        ensure!(c.product_count() == 1831);
        let view = c.view().unwrap();
        ensure!(view.subcatalog == 0 && view.products.is_empty());
        ensure!(
            view.categories
                .iter()
                .all(|b| b.visible && b.enabled && !b.selected)
        );
        ensure!(view.labels.iter().all(|s| !s.is_empty()));
        results.push(json!({"session":s.save(),"camera_device":d.save(&s)?}));
    }
    ensure!(
        results[0] == results[1],
        "Buy publication differs after cold restore"
    );
    std::fs::write(
        output.join("session.json"),
        serde_json::to_vec(&results[1])?,
    )?;
    pixels.save(output.join("goth.png"))?;
    // Reuse the same widget owner in a later actual4ce390 frame. This isolated
    // redraw request is a test stimulus; it performs no gameplay or UI input.
    let context = serde_json::from_value(before["view"]["publication"]["context"].clone())?;
    s.host
        .presentation
        .transport
        .as_mut()
        .unwrap()
        .controls
        .buy
        .dirty = 0x400;
    s.begin_interface_publication(context, 0)?;
    let mut complete = false;
    for _ in 0..80 {
        match d.advance_publication(&mut s, 1)? {
            Progress::Complete => {
                complete = true;
                break;
            }
            Progress::Pending => (),
            other => anyhow::bail!("Repeated Buy publication: {other:?}"),
        }
    }
    ensure!(complete);
    ensure!(
        d.panel_widgets().unwrap().catalog().unwrap().view()
            == serde_json::from_value::<sims_render::camera_device::panels::Widgets>(
                results[1]["camera_device"]["panel_widgets"].clone()
            )?
            .catalog()
            .unwrap()
            .view()
    );
    d.save(&s)?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":s.simulation.vm.tick,"products":1831,"labels":d.panel_widgets().unwrap().catalog().unwrap().view().unwrap().labels,
            "native_frame_completed":true,"entire_simulation_unchanged":true,"only_session_frame_fields_changed":true,
            "cold_restore_equal":true,"subsequent_frame_reuses_widgets":true,"headless_house_pixels_equal":true,
            "gameplay_advanced":false,"purchase_or_category_input":false,
            "scope":"Original Buy publication; Iced catalog presentation; input callbacks remain separate"
        }))?,
    )?;
    Ok(())
}
