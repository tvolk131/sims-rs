//! Read-only original Goth rendering of both diagonal covering orientations.
use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::Session;

#[test]
#[ignore = "requires private original Goth checkpoint and immutable assets"]
fn goth_diagonal_coverings_and_background_render_without_advancing_simulation() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("wallpaper render panicked")
}

fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out = root.join(std::env::var("SIMS_WALLPAPER_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(
        root.join("cache/controller-goth-purchase-20261022/session.json"),
    )?)?;
    let session = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let before = serde_json::to_vec(&session.save())?;
    let mut device = Device::restore(
        &session,
        serde_json::from_value(source["camera_device"].clone())?,
    )?;
    device.refresh(&session)?;
    let frame = software::render(device.scene());
    ensure!(
        frame == software::render(device.scene()),
        "repeated render changed pixels"
    );
    ensure!(
        before == serde_json::to_vec(&session.save())?,
        "render advanced simulation"
    );
    // This preserved camera puts the top-right fifth wholly outside Goth's
    // lot. The backdrop must not expose the renderer's dark clear color.
    for y in 0..frame.height() / 10 {
        for x in frame.width() * 4 / 5..frame.width() {
            ensure!(
                frame.get_pixel(x, y).0 == [112, 112, 112, 255],
                "off-lot background artifact at ({x},{y})"
            );
        }
    }
    frame.save(out.join("goth.png"))?;
    std::fs::copy(
        root.join("cache/controller-goth-purchase-20261022/session.json"),
        out.join("source.json"),
    )?;
    std::fs::write(
        out.join("verification.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source": "cache/controller-goth-purchase-20261022/session.json",
            "preserved_input": "source.json",
            "tick": session.simulation.vm.tick,
            "simulation_unchanged": true,
            "repeat_pixels_equal": true,
            "top_right_background_solid": true,
            "scope": "Read-only original Goth scene; no completed gameplay parity ticks"
        }))?,
    )?;
    Ok(())
}
