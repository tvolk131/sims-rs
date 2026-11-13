use super::*;

fn repository() -> Result<std::path::PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn trim_image(index: &Index, id: i32) -> Result<image::RgbaImage> {
    sims_content::dialog::ui_bitmaps::Bitmaps::load(index)?
        .image(
            index,
            sims_vm::dialog::GlobalBitmap {
                id,
                button_cell: false,
            },
            1,
            [255, 0, 255],
        )?
        .context("original mode trim absent")
}

#[test]
#[ignore = "private original UI assets and captured original 800x600 frames"]
fn installed_trim_matches_original_pixels_and_expanded_visibility() -> Result<()> {
    let repo = repository()?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let icons = dialog_view::Icons::load(&index)?;
    for mode in 0..=3 {
        let (id, [x, y]) = mode_trim(mode, true).context("mode trim absent")?;
        let trim = trim_image(&index, id)?;
        let (width, height, frames) = icons.hud(id).context("trim not loaded")?;
        ensure!(frames.len() == 1, "trim split into button cells");
        ensure!((*width, *height) == trim.dimensions());
        // All four native patches meet the cluster's same lower-right edge.
        ensure!(x + *width as f32 == 220. && y + *height as f32 == 183.);
        ensure!(trim_layer(&icons, mode, false).is_none());
        ensure!(mode_trim(mode, false).is_none());
        ensure!(trim.pixels().any(|p| p[3] == 0));
        ensure!(trim.pixels().any(|p| p[3] == 255));
        for p in trim.pixels() {
            ensure!(p[3] == if p.0[..3] == [255, 0, 255] { 0 } else { 255 });
        }
    }
    for mode in [-1, 4, 5, 255] {
        ensure!(mode_trim(mode, true).is_none());
    }
    for (mode, reference, opaque) in [
        (2, "original-live-expanded.png", 1837),
        (3, "original-options.png", 584),
    ] {
        let (id, [x, y]) = mode_trim(mode, true).unwrap();
        let trim = trim_image(&index, id)?;
        let original =
            image::open(repo.join("cache/visual-parity-20261021").join(reference))?.into_rgba8();
        ensure!(original.dimensions() == (800, 600));
        let mut compared = 0;
        for (px, py, pixel) in trim.enumerate_pixels().filter(|(_, _, p)| p[3] != 0) {
            let actual = original.get_pixel(x as u32 + px, 417 + y as u32 + py);
            // The original reference is RGB565. Its expansion can differ by
            // at most seven levels from the unquantized shipped RGB artwork.
            ensure!(
                (0..3).all(|c| pixel[c].abs_diff(actual[c]) <= 7),
                "native trim{mode} differs at({px},{py}): {pixel:?}/{actual:?}"
            );
            compared += 1;
        }
        ensure!(compared == opaque);
    }
    Ok(())
}

fn retained(v: &Viewer) -> Result<Vec<u8>> {
    let s = v.session.as_ref().context("session absent")?;
    Ok(serde_json::to_vec(&(
        s.save(),
        v.client.as_ref().context("client absent")?.save(s)?,
        v.device.as_ref().context("device absent")?.save(s)?,
    ))?)
}

#[test]
#[ignore = "private expanded Live checkpoint and offscreen Metal adapter"]
fn headless_trim_frames_preserve_the_complete_retained_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(render_trim_frames)?
        .join()
        .expect("HUD trim check panicked")
}

fn render_trim_frames() -> Result<()> {
    use clap::Parser;
    let repo = repository()?;
    let root = repo.join("game-data/cabinet-extracted");
    let checkpoint = repo.join(std::env::var("SIMS_HUD_TRIM_CHECKPOINT")?);
    let output = repo.join(std::env::var("SIMS_HUD_TRIM_OUTPUT")?);
    let house = std::env::var("SIMS_HUD_TRIM_HOUSE")
        .unwrap_or_else(|_| "UserData/Houses/House07.iff".to_string());
    ensure!(checkpoint.starts_with(repo.join("cache")) && output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("HUD trim output must be fresh")?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        &house,
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        checkpoint.to_str().unwrap(),
        "--save",
        output.join("unused.json").to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let s = viewer.session.as_ref().unwrap();
    ensure!(
        s.host
            .presentation
            .transport
            .as_ref()
            .unwrap()
            .controls
            .mode
            == 2
    );
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(0));
    let tick = s.simulation.vm.tick;
    let before = retained(&viewer)?;
    for [width, height] in [[800, 600], [1000, 740], [1320, 900]] {
        super::super::super::support::render_headless_element_at_size(
            &root,
            &output.join(format!("live-{width}x{height}.png")),
            viewer.view(),
            [width, height],
        )?;
        ensure!(
            before == retained(&viewer)?,
            "HUD draw changed retained state"
        );
    }
    let _ = viewer.update(Message::Options);
    ensure!(viewer.options_open);
    super::super::super::support::render_headless_element_at_size(
        &root,
        &output.join("application-menu.png"),
        viewer.view(),
        [800, 600],
    )?;
    ensure!(
        before == retained(&viewer)?,
        "Application menu changed retained state"
    );
    let live = image::open(output.join("live-800x600.png"))?.into_rgba8();
    let options = image::open(output.join("application-menu.png"))?.into_rgba8();
    let (id, [x, y]) = mode_trim(2, true).unwrap();
    let live_trim = trim_image(viewer.session.as_ref().unwrap().index(), id)?;
    // The Options button changes its selected cell, but opening the app menu
    // must not change the current native mode or the Live trim itself.
    for (px, py, _) in live_trim.enumerate_pixels().filter(|(_, _, p)| p[3] != 0) {
        ensure!(
            live.get_pixel(x as u32 + px, 417 + y as u32 + py)
                == options.get_pixel(x as u32 + px, 417 + y as u32 + py),
            "Application popup incorrectly changed Live trim"
        );
    }
    let _ = viewer.update(Message::Options);
    ensure!(!viewer.options_open && before == retained(&viewer)?);

    let icons = viewer.icons.as_ref().context("HUD icons absent")?;
    let index = viewer.session.as_ref().unwrap().index();
    for mode in 0..=3 {
        let (id, [x, y]) = mode_trim(mode, true).unwrap();
        let (w, h, base) = icons.hud(2037).unwrap();
        let cluster = widget::stack![
            widget::image(base[0].clone())
                .width(*w as f32)
                .height(*h as f32)
        ]
        .push(trim_layer(icons, mode, true).unwrap())
        .width(220)
        .height(183);
        let path = output.join(format!("trim-component-mode{mode}.png"));
        super::super::super::support::render_headless_element_at_size(
            &root,
            &path,
            cluster.into(),
            [220, 183],
        )?;
        let frame = image::open(path)?.into_rgba8();
        let trim = trim_image(index, id)?;
        for (px, py, pixel) in trim.enumerate_pixels().filter(|(_, _, p)| p[3] != 0) {
            let actual = frame.get_pixel(x as u32 + px, y as u32 + py);
            ensure!((0..4).all(|c| pixel[c].abs_diff(actual[c]) <= 1));
        }
        ensure!(
            before == retained(&viewer)?,
            "trim component draw changed retained state"
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "checkpoint":checkpoint,"tick":tick,"frames":8,
            "full_retained_state_unchanged":true,
            "application_menu_keeps_original_live_trim":true,
            "four_mode_component_pixels_match_assets":true,
            "scope":"Static native HUD trim only; no new controls, guest execution or gameplay parity ticks."
        }))?,
    )?;
    Ok(())
}
