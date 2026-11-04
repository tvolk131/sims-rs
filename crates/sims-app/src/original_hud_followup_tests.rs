use super::*;

fn repository() -> Result<std::path::PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}

fn artwork(index: &Index, id: i32, cell: bool) -> Result<image::RgbaImage> {
    sims_content::dialog::ui_bitmaps::Bitmaps::load(index)?
        .image(
            index,
            sims_vm::dialog::GlobalBitmap {
                id,
                button_cell: cell,
            },
            1,
            [255, 0, 255],
        )?
        .context("HUD artwork absent")
}

#[test]
#[ignore = "installed original UI assets and captured native Options frame"]
fn native_options_icons_separators_and_background_match_reference() -> Result<()> {
    let repo = repository()?;
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let icons = dialog_view::Icons::load(&index)?;
    let reference =
        image::open(repo.join("cache/visual-parity-20261021/original-options.png"))?.into_rgba8();
    for (id, x, y, cell) in [
        (4950, 249, 506, true),
        (4951, 308, 506, true),
        (4952, 379, 506, true),
        (4957, 249, 558, true),
        (4958, 309, 558, true),
        (4959, 371, 558, true),
        (4916, 236, 551, false),
        (4917, 442, 506, false),
    ] {
        let art = artwork(&index, id, cell)?;
        let (w, h, frames) = icons.hud(id).context("Options icon not loaded")?;
        ensure!((*w, *h) == art.dimensions() && frames.len() == if cell { 4 } else { 1 });
        for (px, py, pixel) in art.enumerate_pixels().filter(|(_, _, p)| p[3] != 0) {
            let actual = reference.get_pixel(x + px, y + py);
            ensure!(
                (0..3).all(|c| pixel[c].abs_diff(actual[c]) <= 7),
                "native Options resource{id} differs at({px},{py})"
            );
        }
    }
    let art = artwork(&index, 4910, false)?;
    ensure!(art.dimensions() == (804, 100) && icons.hud_edge(4910).is_some());
    for y in 505..595 {
        for x in 500..800 {
            let pixel = art.get_pixel(x - 220, y - 500);
            let actual = reference.get_pixel(x, y);
            ensure!((0..3).all(|c| pixel[c].abs_diff(actual[c]) <= 7));
        }
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

fn click(v: &Viewer, display: [u32; 2], point: [f32; 2]) -> Result<Vec<Message>> {
    use iced::advanced::{layout, renderer::Headless, widget::Tree};
    let mut renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .context("headless adapter absent")?;
    let size = Size::new(display[0] as f32, display[1] as f32);
    let bounds = iced::Rectangle::with_size(size);
    let mut element = v.view();
    let mut tree = Tree::new(element.as_widget());
    let node = element.as_widget_mut().layout(
        &mut tree,
        &mut renderer,
        &layout::Limits::new(Size::ZERO, size),
    );
    let cursor = iced::mouse::Cursor::Available(iced::Point::new(point[0], point[1]));
    let mut messages = vec![];
    for event in [
        iced::Event::Window(window::Event::RedrawRequested(Instant::now())),
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
            iced::mouse::Button::Left,
        )),
    ] {
        element.as_widget_mut().update(
            &mut tree,
            &event,
            iced::advanced::Layout::new(&node),
            cursor,
            &renderer,
            &mut iced::advanced::clipboard::Null,
            &mut iced::advanced::Shell::new(&mut messages),
            &bounds,
        );
    }
    Ok(messages)
}

#[test]
#[ignore = "private no-selection Live checkpoint and offscreen Metal"]
fn clicked_application_panel_and_inactive_needs_preserve_native_state() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(render_followup)?
        .join()
        .expect("HUD follow-up panicked")
}

fn render_followup() -> Result<()> {
    use clap::Parser;
    let repo = repository()?;
    let root = repo.join("game-data/cabinet-extracted");
    let checkpoint = repo.join(std::env::var("SIMS_HUD_FOLLOWUP_CHECKPOINT")?);
    let output = repo.join(std::env::var("SIMS_HUD_FOLLOWUP_OUTPUT")?);
    ensure!(checkpoint.starts_with(repo.join("cache")) && output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("HUD follow-up output must be fresh")?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
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
        s.simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            .is_none()
    );
    ensure!(
        viewer
            .client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .person()
            .is_none()
    );
    let tick = s.simulation.vm.tick;
    let mode = s
        .host
        .presentation
        .transport
        .as_ref()
        .unwrap()
        .controls
        .mode;
    let before = retained(&viewer)?;
    let index = Index::scan(&root)?;
    let background = artwork(&index, 4910, false)?;
    for size in [[800, 600], [1320, 900]] {
        let live_path = output.join(format!("live-{}x{}.png", size[0], size[1]));
        super::super::super::support::render_headless_element_at_size(
            &root,
            &live_path,
            viewer.view(),
            size,
        )?;
        let frame = image::open(live_path)?.into_rgba8();
        // No title/placeholder/fictional need bars in the inactive content.
        let left = if size[0] == 800 { 525 } else { 1100 };
        let right = if size[0] == 800 { 720 } else { size[0] };
        for y in size[1] - 95..size[1] - 5 {
            for x in left..right {
                let expected = background.get_pixel((x - 220).min(803), y - (size[1] - 100));
                let actual = frame.get_pixel(x, y);
                ensure!(
                    (0..4).all(|c| expected[c].abs_diff(actual[c]) <= 1),
                    "inactive/wide panel differs at({x},{y})"
                );
            }
        }
        let messages = click(&viewer, size, [204., size[1] as f32 - 47.])?;
        ensure!(
            matches!(messages.as_slice(), [Message::Options]),
            "Options icon dispatched wrong action"
        );
        for m in messages {
            let _ = viewer.update(m);
        }
        ensure!(viewer.options_open && before == retained(&viewer)?);
        super::super::super::support::render_headless_element_at_size(
            &root,
            &output.join(format!("clicked-application-{}x{}.png", size[0], size[1])),
            viewer.view(),
            size,
        )?;
        ensure!(before == retained(&viewer)?);
        let y = size[1] as f32 - 100.;
        ensure!(matches!(
            click(&viewer, size, [268., y + 25.])?.as_slice(),
            [Message::Save]
        ));
        ensure!(matches!(
            click(&viewer, size, [487., y + 37.])?.as_slice(),
            [Message::Load]
        ));
        ensure!(matches!(
            click(&viewer, size, [573., y + 37.])?.as_slice(),
            [Message::Diagnostics]
        ));
        ensure!(matches!(
            click(&viewer, size, [487., y + 67.])?.as_slice(),
            [Message::Restart]
        ));
        for p in [
            [332., y + 24.],
            [404., y + 26.],
            [268., y + 78.],
            [332., y + 78.],
            [398., y + 78.],
        ] {
            ensure!(
                click(&viewer, size, p)?.is_empty(),
                "unsupported native control dispatched an unrelated command"
            );
        }
        let messages = click(&viewer, size, [573., y + 67.])?;
        ensure!(matches!(messages.as_slice(), [Message::Options]));
        for m in messages {
            let _ = viewer.update(m);
        }
        ensure!(!viewer.options_open && before == retained(&viewer)?);
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "checkpoint":checkpoint,"tick":tick,"native_mode":mode,"native_selected":null,"live_person":null,
            "clicked_open_viewports":[[800,600],[1320,900]],"frames":4,
            "full_session_device_client_unchanged":true,"native_options_mode_claimed":false,
            "application_actions_dispatch_preserved":true,"unsupported_original_icons_disabled":5,
            "native_background_pixels_and_wide_edge_verified":true,"original_parity_ticks_added":0
        }))?,
    )?;
    Ok(())
}
