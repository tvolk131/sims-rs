use super::*;
use sims_render::camera_device::panels::Widgets;
use sims_vm::interface::view_control::Widget;

#[test]
#[ignore = "saved Goth panel widgets and headless Iced Metal rendering"]
fn published_panel_buttons_render_from_the_saved_widget_owner() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = root.join(std::env::var("SIMS_PANEL_WIDGET_INPUT")?);
    let output = root.join(std::env::var("SIMS_PANEL_WIDGET_VISUAL")?);
    ensure!(input.starts_with(root.join("cache")) && output.starts_with(root.join("cache")));
    ensure!(!output.exists(), "refusing to replace widget image");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input.join("session.json"))?)?;
    let widgets: Widgets = serde_json::from_value(saved["camera_device"]["panel_widgets"].clone())?;
    ensure!(widgets.pause().selected && widgets.pause().auxiliary == 1);
    let toolbar = row![
        published_control("Play", Message::Toggle, Some(widgets.pause()), false),
        published_control(
            "Live Mode",
            Message::Live,
            Some(widgets.button(Widget::Live)),
            false
        ),
        published_control(
            "Normal",
            Message::NativeSpeed(760),
            Some(widgets.button(Widget::Normal)),
            false
        ),
        published_control(
            "Fast",
            Message::NativeSpeed(924),
            Some(widgets.button(Widget::Fast)),
            false
        ),
        published_control(
            "Ultra",
            Message::NativeSpeed(994),
            Some(widgets.button(Widget::Ultra)),
            false
        ),
        published_control(
            "Zoom in",
            Message::ZoomIn,
            Some(widgets.button(Widget::ZoomIn)),
            false
        ),
        published_control(
            "Zoom out",
            Message::ZoomOut,
            Some(widgets.button(Widget::ZoomOut)),
            false
        ),
        published_control(
            "Downstairs",
            Message::Floor(1),
            Some(widgets.button(Widget::FloorOne)),
            false
        ),
        published_control(
            "Upstairs",
            Message::Floor(2),
            Some(widgets.button(Widget::FloorTwo)),
            false
        ),
    ]
    .spacing(8);
    // Iced uses image-without-codecs. Supply decoded pixels just like the
    // production viewport/icon adapters, rather than an undecodable file path.
    // Keep this static preview below Iced's 2 MiB asynchronous upload threshold:
    // the headless helper captures one frame. The production lot uses Viewport.
    let pixels = image::open(input.join("goth.png"))?
        .thumbnail(800, 592)
        .into_rgba8();
    let house_image =
        widget::image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw());
    let page = container(
        widget::column![
            text("Goth household").size(28),
            text("Paused"),
            toolbar,
            widget::image(house_image).width(Fill).height(Fill),
        ]
        .spacing(14)
        .width(Fill)
        .height(Fill),
    )
    .padding(24)
    .width(Fill)
    .height(Fill);
    render_headless_element(
        &root.join("game-data/cabinet-extracted"),
        &output,
        page.into(),
    )?;
    let frame = image::open(&output)?.into_rgba8();
    let grass_pixels = frame
        .pixels()
        .filter(|p| {
            u16::from(p[1]) > u16::from(p[0]) + 10 && u16::from(p[1]) > u16::from(p[2]) + 10
        })
        .count();
    ensure!(
        grass_pixels > 30_000,
        "headless house preview did not render: {grass_pixels} grass pixels"
    );
    Ok(())
}

#[test]
#[ignore = "private Goth checkpoint and headless Metal adapter"]
fn replacement_buy_panel_hides_live_actions_and_rejects_stale_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(render_replacement_buy_panel)?
        .join()
        .expect("panel Iced check panicked")
}

fn render_replacement_buy_panel() -> Result<()> {
    use clap::Parser;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = root.join(std::env::var("SIMS_PANEL_PRESENTATION_INPUT")?);
    let output = root.join(std::env::var("SIMS_PANEL_PRESENTATION_VISUAL")?);
    ensure!(input.starts_with(root.join("cache")) && output.starts_with(root.join("cache")));
    ensure!(!output.exists());
    let resources = root.join("game-data/cabinet-extracted");
    let index = Index::scan(&resources)?;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input.join("session.json"))?)?;
    let session = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
    let device = CameraDevice::restore(
        &session,
        serde_json::from_value(saved["camera_device"].clone())?,
    )?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        resources.to_str().unwrap(),
    ]);
    let (mut viewer, _) = Viewer::new(args);
    viewer.floor = device.controls().floor;
    viewer.zoom = (1u32 << device.controls().zoom) as f32 / 8.;
    viewer.scene = Some(device.scene().clone());
    viewer.resident = 21;
    viewer.device = Some(device);
    viewer.session = Some(session);
    viewer.status = "Buy mode".into();
    ensure!(!viewer.people_panel_visible());
    let before = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    for message in [
        Message::Resident(21),
        Message::Needs,
        Message::MotiveDetails(0),
        Message::Queue(1, 0),
        Message::Cancel(1),
        Message::Select(Some(1)),
        Message::ViewportClick([500., 370.]),
        Message::GroundChoice(0),
        Message::GroundCancel,
        Message::ObjectChoice(0),
        Message::ObjectCancel,
        Message::Toggle,
        Message::NativeSpeed(994),
        Message::Floor(2),
        Message::ZoomIn,
    ] {
        let _ = viewer.update(message);
        ensure!(viewer.error.is_none() && viewer.selected.is_none());
        ensure!(before == serde_json::to_value(viewer.session.as_ref().unwrap().save())?);
    }
    render_headless_element(&resources, &output, viewer.view())?;
    ensure!(before == serde_json::to_value(viewer.session.as_ref().unwrap().save())?);
    let device_before = viewer
        .device
        .as_ref()
        .unwrap()
        .save(viewer.session.as_ref().unwrap())?;
    let pending = format!(
        "{:?}",
        viewer
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .boundary()
    );
    let error = viewer
        .step()
        .expect_err("unsupported panel service must stay pending");
    ensure!(
        error.to_string().contains("unsupported service") && error.to_string().contains(&pending),
        "{error:#}"
    );
    ensure!(before == serde_json::to_value(viewer.session.as_ref().unwrap().save())?);
    ensure!(
        device_before
            == viewer
                .device
                .as_ref()
                .unwrap()
                .save(viewer.session.as_ref().unwrap())?
    );
    Ok(())
}
