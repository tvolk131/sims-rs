use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Session, publication::Progress};
#[test]
#[ignore = "private original Buy Viewer checkpoint; release/headless"]
fn original_catalog_product_selects_real_definition_and_resumes_tool_callback() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("Buy product panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_BUY_PRODUCT_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-buy-input-iced-2/session.json"),
    )?)?;
    let mut s = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let mut d = Device::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
    let before = s.simulation.clone();
    let pixels = software::render(d.scene());
    let c = d.panel_widgets().unwrap().catalog().unwrap();
    let revision = c.revision();
    let row = c.view().unwrap().products[0];
    let product = c.product(row).unwrap().clone();
    let item = sims_render::camera_device::panels::catalog::Catalog::item_id(row)?;
    ensure!(
        product.fields.price == 80,
        "expected original cheapest dining chair"
    );
    ensure!(d.begin_buy_product(&mut s, revision, row)?);
    let mut mid = None;
    for _ in 0..300 {
        ensure!(
            !d.advance_tool_selection(&mut s, 1)?,
            "tool returned before callback checkpoint"
        );
        let state = serde_json::to_value(s.host.presentation.tool_selection.as_ref().unwrap())?;
        if !state["call"]["child"].is_null()
            && state["call"]["frames"].as_array().unwrap().len() >= 3
        {
            mid = Some(json!({"session":s.save(),"camera_device":d.save(&s)?}));
            break;
        }
    }
    let mid = mid.expect("product tool callback not reached");
    std::fs::write(out.join("pending-product.json"), serde_json::to_vec(&mid)?)?;
    let mut results = vec![];
    for resumed in [false, true] {
        if resumed {
            s = Session::restore(&index, serde_json::from_value(mid["session"].clone())?)?;
            d = Device::restore(&s, serde_json::from_value(mid["camera_device"].clone())?)?;
        }
        ensure!(s.begin_clock_frame().is_err());
        for _ in 0..1000 {
            if d.advance_tool_selection(&mut s, if resumed { 1 } else { 256 })? {
                break;
            }
        }
        ensure!(!s.tool_selection_pending());
        let buy = &s.host.presentation.transport.as_ref().unwrap().controls.buy;
        ensure!(buy.selected_object == item && buy.selection_aux == 0 && buy.dirty & 0x200 != 0);
        ensure!(d.panel_widgets().unwrap().catalog().unwrap().selected(row));
        ensure!(s.republish_interface()?);
        for _ in 0..1000 {
            match d.advance_publication(&mut s, 256)? {
                Progress::Complete => break,
                Progress::Pending => (),
                p => anyhow::bail!("product publication {p:?}"),
            }
        }
        ensure!(!s.publication_pending());
        ensure!(s.simulation == before && software::render(d.scene()) == pixels);
        results.push(json!({"session":s.save(),"camera_device":d.save(&s)?}));
    }
    ensure!(
        results[0] == results[1],
        "product selection changed after cold restore"
    );
    let mut saved = source;
    saved["session"] = results[1]["session"].clone();
    saved["camera_device"] = results[1]["camera_device"].clone();
    std::fs::write(out.join("session.json"), serde_json::to_vec(&saved)?)?;
    pixels.save(out.join("goth.png"))?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":s.simulation.vm.tick,"product":product,"item":item,"index":row,"native_tool_callbacks":true,
            "cold_restore_equal":true,"whole_simulation_unchanged":true,"headless_pixels_equal":true,
            "selected_style":true,"subsequent_publication_complete":true,"purchased":false
        }))?,
    )?;
    Ok(())
}
