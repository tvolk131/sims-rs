use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private original assets and saved Goth/Mashuga households; headless software rendering"]
fn real_house_viewport_resize_preserves_vm_and_cold_restores_pixels() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(
        std::env::var("SIMS_VIEWPORT_OUTPUT")
            .unwrap_or_else(|_| "cache/native-window-viewport-frames".into()),
    );
    ensure!(!out.exists(), "refusing to overwrite viewport evidence");
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let mut s = Session::restore(&index, saved)?;
        let mut d = Device::restore(
            &s,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        let old = d.save(&s)?;
        ensure!(
            serde_json::to_value(&old)?.get("viewport").is_none(),
            "historical device unexpectedly acquired viewport"
        );
        d.bind_viewport()?;
        let before = serde_json::to_value(s.save())?;
        let original_viewport = s
            .host
            .presentation
            .presenter
            .as_ref()
            .context("presenter absent")?
            .camera
            .as_ref()
            .context("camera absent")?
            .settings
            .viewport;
        let pixels = software::render(d.scene());
        pixels.save(out.join(format!("{lot}-full-before.png")))?;
        let h = original_viewport[3];
        for (name, height) in [("short", h - 100), ("half", h / 2), ("full-restored", h)] {
            d.resize_viewport_height(&mut s, height)?;
            let image = software::render(d.scene());
            ensure!(
                image.width() == original_viewport[2] as u32 && image.height() == height as u32,
                "actual frame did not resize"
            );
            image.save(out.join(format!("{lot}-{name}.png")))?;
            let simulation = serde_json::to_value(&s.save().simulation)?;
            ensure!(
                simulation == before["simulation"],
                "viewport changed complete simulation snapshot"
            );
            let mut normalized = serde_json::to_value(s.save())?;
            normalized["view"]["presenter"]["camera"]["settings"]["viewport"] =
                before["view"]["presenter"]["camera"]["settings"]["viewport"].clone();
            normalized["view"]["camera"] = before["view"]["camera"].clone();
            normalized["view"]["viewport"] = before["view"]["viewport"].clone();
            ensure!(
                normalized == before,
                "viewport changed session state beyond its three projection owners"
            );
            let saved = s.save();
            let device = d.save(&s)?;
            let session_bytes = serde_json::to_vec(&saved)?;
            let device_bytes = serde_json::to_vec(&device)?;
            s = Session::restore(&index, serde_json::from_slice(&session_bytes)?)?;
            d = Device::restore(&s, serde_json::from_slice(&device_bytes)?)?;
            ensure!(
                serde_json::to_vec(&s.save())? == session_bytes,
                "session cold restore differs"
            );
            ensure!(
                serde_json::to_vec(&d.save(&s)?)? == device_bytes,
                "viewport device cold restore differs"
            );
            ensure!(
                software::render(d.scene()) == image,
                "viewport pixels changed after cold restore"
            );
            reports.push(json!({"lot":lot,"stage":name,"height":height,"tick":s.simulation.vm.tick,"simulation_unchanged":true,"only_session_changes":["presenter.camera.settings.viewport","presentation.viewport","presentation.camera"],"cold_restore_pixels_equal":true}));
        }
        ensure!(
            software::render(d.scene()) == pixels,
            "returning to full height changed original frame"
        );
        let before_invalid = serde_json::to_vec(&d.save(&s)?)?;
        ensure!(d.resize_viewport_height(&mut s, 0).is_err());
        ensure!(
            serde_json::to_vec(&d.save(&s)?)? == before_invalid,
            "invalid viewport mutated device"
        );
        ensure!(
            serde_json::to_value(s.save())? == before,
            "invalid viewport mutated session"
        );
        let mut bad = serde_json::to_value(d.save(&s)?)?;
        bad["viewport"]["bound_size"][0] = json!(1);
        ensure!(
            Device::restore(&s, serde_json::from_value(bad)?).is_err(),
            "mismatched backend viewport was accepted"
        );
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"scope":"Actual replacement viewport service; no VM slices, native pixel comparison or Buy2008 binding claimed","cases":reports}),
        )?,
    )?;
    Ok(())
}
