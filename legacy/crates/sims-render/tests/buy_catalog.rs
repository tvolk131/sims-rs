//! Asset-backed catalog read at the actual Goth publication boundary.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::Session;

#[test]
#[ignore = "private Goth checkpoint; headless whole-session preservation"]
fn original_buy_catalog_matches_retained_household_without_advancing_vm() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("catalog check panicked")
}
fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_BUY_CATALOG_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let input: Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-panel-collapse-frames-2/session.json"),
    )?)?;
    let session = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
    let device = Device::restore(
        &session,
        serde_json::from_value(input["camera_device"].clone())?,
    )?;
    let before = json!({"session":session.save(),"device":device.save(&session)?});
    let pixels = software::render(device.scene());
    let products = session.buy_products()?;
    ensure!(!products.is_empty());
    ensure!(before == json!({"session":session.save(),"device":device.save(&session)?}));
    ensure!(pixels == software::render(device.scene()));
    let cold = Session::restore(&index, serde_json::from_value(before["session"].clone())?)?;
    let cold_device = Device::restore(&cold, serde_json::from_value(before["device"].clone())?)?;
    ensure!(cold.buy_products()? == products);
    ensure!(before == json!({"session":cold.save(),"device":cold_device.save(&cold)?}));
    ensure!(pixels == software::render(cold_device.scene()));
    pixels.save(output.join("goth.png"))?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":session.simulation.vm.tick,"products":products.len(),
            "entire_session_unchanged":true,"device_unchanged":true,"headless_pixels_equal":true,
            "cold_restore_equal":true,"new_gameplay_tick":false,"original_execution":false,
            "frontier":format!("{:?}", device.panel_widgets().unwrap().boundary()),
            "scope":"Real household-owned product identities; publication acknowledgement and purchase separate"
        }))?,
    )?;
    Ok(())
}
