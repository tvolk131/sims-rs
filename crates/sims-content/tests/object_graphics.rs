use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_graphics::Sources};

#[path = "support/portrait.rs"]
#[allow(dead_code)]
mod support;

#[test]
#[ignore = "requires original Complete Collection files and retained Newbie checkpoint"]
fn original_shower_graphics_use_retained_definition_and_rebuild_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_OBJECT_GRAPHICS_OUTPUT")?);
    std::fs::create_dir(&output)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let checkpoint = root.join("cache/newbie-live-widget-final/session.json");
    let saved_bytes = std::fs::read(&checkpoint)?;
    let saved: serde_json::Value = serde_json::from_slice(&saved_bytes)?;
    let vm = &saved["session"]["simulation"]["state"]["vm"];
    ensure!(vm["tick"] == 193299);
    let definition: sims_vm::Definition =
        serde_json::from_value(vm["world"]["objects"]["107"]["definition"].clone())?;
    ensure!(definition.guid == 0x85e02a40 && definition.object == 16807);
    ensure!(definition.source == "GameData_ALL_Recursive/Objects/Objects.far::ShowerC.iff");
    let original = index.resources[index.find(&definition.source)?].read()?;
    let mut results = Vec::new();
    for _ in 0..2 {
        let mut catalog = support::catalog(&index)?;
        let handle = catalog
            .live_dialog_resource(&definition)
            .map_err(anyhow::Error::msg)?;
        let before = catalog.resources.overlay().map_err(anyhow::Error::msg)?;
        let mut sources = Sources::new(&mut catalog);
        let metadata = sources.definition(handle)?;
        ensure!(metadata.first_graphic == 128 && metadata.graphic_count == 6);
        ensure!(metadata.group == 0);
        let mut views = Vec::new();
        for selector in -1..=6 {
            for facing in [0, 2, 4, 6] {
                for zoom in 1..=3 {
                    let selected = sources.selected_view(handle, selector, facing, zoom)?;
                    if selector == -1 {
                        ensure!(selected.is_none());
                        continue;
                    }
                    let view = selected.context("original shower view absent")?;
                    let mut frames = Vec::new();
                    ensure!(!view.sprites.is_empty());
                    for sprite in view.sprites {
                        let count = sources.frame_count(handle, sprite.id)?;
                        ensure!(sprite.frame < count as u32);
                        let frame = sources
                            .frame(handle, sprite.id, sprite.frame as i32)?
                            .context("original shower frame absent")?;
                        // This is only the original metadata getter. Pixel
                        // drawing and the enclosing dialog remain unexecuted.
                        ensure!(frame.rect[0] < frame.rect[2] && frame.rect[1] < frame.rect[3]);
                        frames.push(serde_json::json!({
                            "id":sprite.id,"frame":sprite.frame,"count":count,
                            "screen":[sprite.x,sprite.y],"object":sprite.object,
                            "flags":sprite.flags,"metadata":frame,
                        }));
                    }
                    views.push(serde_json::json!({"selector":selector,"facing":facing,
                        "zoom":zoom,"frames":frames}));
                }
            }
        }
        for n in 0..12 {
            ensure!(
                views[n]["frames"] == views[72 + n]["frames"],
                "out-of-range selector did not reset to zero"
            );
        }
        drop(sources);
        ensure!(catalog.resources.overlay().map_err(anyhow::Error::msg)? == before);
        results.push(serde_json::json!({"definition":metadata,"views":views}));
    }
    ensure!(
        results[0] == results[1],
        "source cache rebuild changed selected resources"
    );
    ensure!(std::fs::read(&checkpoint)? == saved_bytes);
    ensure!(index.resources[index.find(&definition.source)?].read()? == original);
    let report = serde_json::json!({
        "checkpoint_tick":193299,"live_object":107,"definition":definition,
        "selection":results[0],"catalog_rebuilds_equal":true,"original_bytes_unchanged":true,
        "checkpoint_unchanged":true,"native_execution":false,
        "drawing_executed":false,"dialog_published":false,
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
