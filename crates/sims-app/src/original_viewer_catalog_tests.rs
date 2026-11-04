use super::*;
use sims_render::camera_device::panels::Widgets;

#[test]
#[ignore = "published Goth Buy catalog and headless Iced Metal"]
fn actual_catalog_widget_renders_installed_labels_without_changing_state() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = root.join(std::env::var("SIMS_BUY_PANEL_INPUT")?);
    let output = root.join(std::env::var("SIMS_BUY_PANEL_VISUAL")?);
    ensure!(
        input.starts_with(root.join("cache"))
            && output.starts_with(root.join("cache"))
            && !output.exists()
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input.join("session.json"))?)?;
    let widgets: Widgets = serde_json::from_value(saved["camera_device"]["panel_widgets"].clone())?;
    let before = serde_json::to_value(&widgets)?;
    let catalog = widgets.catalog().context("published catalog absent")?;
    ensure!(catalog.product_count() == 1831 && catalog.view().unwrap().labels.len() == 8);
    // Render the exact production catalog element beside its unchanged house.
    // The retained whole-panel collapse is not changed by this component check.
    let pixels = image::open(input.join("goth.png"))?
        .thumbnail(800, 592)
        .into_rgba8();
    let image =
        widget::image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw());
    let page = container(
        widget::column![
            text("Goth household · Buy catalog").size(28),
            text(widgets.texts().unwrap().funds()),
            row![
                widget::image(image).width(Fill).height(Fill),
                container(buy_catalog_view(catalog, true))
                    .width(300)
                    .padding(16)
            ]
            .spacing(24)
            .height(Fill),
        ]
        .spacing(16)
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
    ensure!(before == serde_json::to_value(&widgets)?);
    Ok(())
}

#[test]
#[ignore = "private Goth checkpoint; actual Iced input and headless Metal"]
fn actual_viewer_navigates_buy_pages_and_restores_before_tool_return() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(navigate_viewer)?
        .join()
        .expect("Buy Viewer test panicked")
}
fn navigate_viewer() -> Result<()> {
    use clap::Parser;
    use sims_vm::interface::buy::input::Event;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_BUY_INPUT_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let index = Index::scan(&root)?;
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-buy-panel-frames-2/session.json"),
    )?)?;
    let s = Session::restore(&index, serde_json::from_value(input["session"].clone())?)?;
    // These retained panel-only checkpoints descend from this full Viewer
    // save. Restore its actual portrait/window owner, never clear or recreate
    // the simulation's existing image cache to manufacture a fresh client.
    let owner: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-ui-timer-dispatch-iced/goth/resumed.json"),
    )?)?;
    let client =
        DialogClient::restore(&s, serde_json::from_value(owner["dialog_client"].clone())?)?;
    let d = CameraDevice::restore(&s, serde_json::from_value(input["camera_device"].clone())?)?;
    let simulation = s.simulation.clone();
    let pixels = sims_render::software::render(d.scene());
    let (mut viewer, _) = Viewer::new(args);
    viewer.resident = 21;
    viewer.floor = d.controls().floor;
    viewer.zoom = (1u32 << d.controls().zoom) as f32 / 8.;
    viewer.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    viewer.session = Some(s);
    viewer.client = Some(client);
    viewer.device = Some(d);
    viewer.refresh()?;
    fn drain(v: &mut Viewer) -> Result<()> {
        for _ in 0..100 {
            ensure!(v.error.is_none(), "{:?}", v.error);
            if !v.presentation_pending() {
                return Ok(());
            }
            let _ = v.update(Message::Frame(Instant::now()));
        }
        anyhow::bail!("Iced Buy input did not return")
    }
    let _ = viewer.update(Message::ExpandPanel);
    drain(&mut viewer)?;
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(0));
    let revision = viewer
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap()
        .revision();
    let _ = viewer.update(Message::BuyNavigate(revision, Event::Category(1)));
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(
        !viewer
            .device
            .as_mut()
            .unwrap()
            .advance_tool_selection(viewer.session.as_mut().unwrap(), 1)?
    );
    viewer.save()?;
    let mid = std::fs::read(&save)?;
    std::fs::write(output.join("pending-input.json"), &mid)?;
    drain(&mut viewer)?;
    let continuous = serde_json::to_value(viewer.session.as_ref().unwrap().save())?;
    let continuous_device = viewer
        .device
        .as_ref()
        .unwrap()
        .save(viewer.session.as_ref().unwrap())?;
    viewer.load(&save)?;
    drain(&mut viewer)?;
    ensure!(continuous == serde_json::to_value(viewer.session.as_ref().unwrap().save())?);
    ensure!(
        continuous_device
            == viewer
                .device
                .as_ref()
                .unwrap()
                .save(viewer.session.as_ref().unwrap())?
    );
    let view = viewer
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap()
        .view()
        .unwrap();
    ensure!(view.heading == "Surfaces" && view.page == 0 && view.next);
    let mut steps = vec![];
    for event in [
        Event::Next,
        Event::Previous,
        Event::Category(7),
        Event::Back,
        Event::Category(0),
    ] {
        let revision = viewer
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .catalog()
            .unwrap()
            .revision();
        let _ = viewer.update(Message::BuyNavigate(revision, event));
        drain(&mut viewer)?;
        let view = viewer
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .catalog()
            .unwrap()
            .view()
            .unwrap();
        steps.push(serde_json::json!({"event":event,"view":view}));
        ensure!(viewer.session.as_ref().unwrap().simulation == simulation);
        ensure!(sims_render::software::render(viewer.device.as_ref().unwrap().scene()) == pixels);
    }
    viewer.save()?;
    let snapshot = |v: &Viewer| -> Result<serde_json::Value> {
        let s = v.session.as_ref().unwrap();
        Ok(serde_json::json!({"session":s.save(),
            "camera_device":v.device.as_ref().unwrap().save(s)?,
            "dialog_client":v.client.as_ref().unwrap().save(s)?}))
    };
    let before = snapshot(&viewer)?;
    render_headless_element(&root, &output.join("catalog.png"), viewer.view())?;
    ensure!(before == snapshot(&viewer)?);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":viewer.session.as_ref().unwrap().simulation.vm.tick,
            "actual_viewer_messages":true,"native_panel_expansion":true,"automatic_dirty_publication":true,
            "cold_restore_equal":true,"entire_simulation_unchanged_after_frontend_attach":true,
            "headless_house_pixels_equal":true,"render_preserves_save":true,"steps":steps,
            "scope":"Category and page input through the actual Viewer, tool callback and dirty publication. No purchase or gameplay tick."
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "original Buy products, full Iced callbacks and offscreen Metal"]
fn actual_viewer_selects_chair_and_rejects_unaffordable_bed_after_tool_callbacks() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(select_products)?
        .join()
        .expect("product Viewer panicked")
}
fn select_products() -> Result<()> {
    use clap::Parser;
    use sims_vm::interface::buy::input::Event;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_BUY_PRODUCT_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let index = Index::scan(&root)?;
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-buy-input-iced-2/session.json"),
    )?)?;
    let s = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let client =
        DialogClient::restore(&s, serde_json::from_value(source["dialog_client"].clone())?)?;
    let d = CameraDevice::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
    let original = s.simulation.clone();
    let pixels = sims_render::software::render(d.scene());
    let (mut v, _) = Viewer::new(args);
    v.resident = 21;
    v.floor = d.controls().floor;
    v.zoom = (1u32 << d.controls().zoom) as f32 / 8.;
    v.session = Some(s);
    v.client = Some(client);
    v.device = Some(d);
    v.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    v.refresh()?;
    fn drain(v: &mut Viewer) -> Result<()> {
        for _ in 0..100 {
            ensure!(v.error.is_none(), "{:?}", v.error);
            if !v.presentation_pending() {
                return Ok(());
            }
            let _ = v.update(Message::Frame(Instant::now()));
        }
        anyhow::bail!("product Viewer did not return")
    }
    fn catalog(v: &Viewer) -> &sims_render::camera_device::panels::catalog::Catalog {
        v.device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .catalog()
            .unwrap()
    }
    let revision = catalog(&v).revision();
    let chair = catalog(&v).view().unwrap().products[0];
    let _ = v.update(Message::BuyProduct(revision, chair));
    drain(&mut v)?;
    ensure!(catalog(&v).selected(chair));
    ensure!(v.session.as_ref().unwrap().simulation == original);
    let before = serde_json::to_value(v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("selected-chair.png"), v.view())?;
    ensure!(before == serde_json::to_value(v.session.as_ref().unwrap().save())?);
    let _ = v.update(Message::BuyNavigate(
        catalog(&v).revision(),
        Event::Category(3),
    ));
    drain(&mut v)?;
    ensure!(catalog(&v).view().unwrap().labels[3] == "Beds");
    let funds = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .budget
        .as_ref()
        .unwrap()
        .balance;
    let mut expensive = None;
    for _ in 0..20 {
        let c = catalog(&v);
        expensive = c
            .view()
            .unwrap()
            .products
            .iter()
            .copied()
            .find(|&i| c.product(i).unwrap().fields.price > funds);
        if expensive.is_some() {
            break;
        }
        ensure!(c.view().unwrap().next, "stock unaffordable bed not found");
        let _ = v.update(Message::BuyNavigate(c.revision(), Event::Next));
        drain(&mut v)?;
    }
    let expensive = expensive.expect("expensive stock bed");
    let product = catalog(&v).product(expensive).unwrap().clone();
    let revision = catalog(&v).revision();
    let saved = serde_json::to_value(v.session.as_ref().unwrap().save())?;
    let _ = v.update(Message::BuyProduct(revision.wrapping_sub(1), expensive));
    ensure!(saved == serde_json::to_value(v.session.as_ref().unwrap().save())?);
    let _ = v.update(Message::BuyProduct(revision, expensive));
    drain(&mut v)?;
    ensure!(catalog(&v).insufficient_funds() && !catalog(&v).selected(expensive));
    let s = v.session.as_ref().unwrap();
    let b = &s.host.presentation.transport.as_ref().unwrap().controls.buy;
    ensure!(b.selected_object == 0 && b.selection_aux == u32::MAX);
    ensure!(
        s.simulation == original
            && sims_render::software::render(v.device.as_ref().unwrap().scene()) == pixels
    );
    v.save()?;
    let before = serde_json::to_value(v.session.as_ref().unwrap().save())?;
    render_headless_element(&root, &output.join("unaffordable-bed.png"), v.view())?;
    ensure!(before == serde_json::to_value(v.session.as_ref().unwrap().save())?);
    v.load(&save)?;
    v.refresh()?;
    ensure!(before == serde_json::to_value(v.session.as_ref().unwrap().save())?);
    ensure!(catalog(&v).insufficient_funds() && !catalog(&v).selected(expensive));
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":v.session.as_ref().unwrap().simulation.vm.tick,"funds":funds,"expensive_product":product,
            "actual_viewer_messages":true,"original_chair_selected":true,"original_bed_rejected_after_callbacks":true,
            "whole_simulation_unchanged":true,"house_pixels_unchanged":true,"cold_restore_equal":true,
            "stale_product_rejected":true,"render_preserves_session":true,"purchased":false
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "original Goth Buy checkpoint, actual Iced viewport input and offscreen Metal"]
fn actual_viewport_enter_creates_preview_and_restores_queued_and_active_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(preview_viewer)?
        .join()
        .expect("preview Viewer panicked")
}
fn preview_viewer() -> Result<()> {
    use clap::Parser;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_MOVE_PREVIEW_ICED_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")) && !output.exists());
    std::fs::create_dir(&output)?;
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let index = Index::scan(&root)?;
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(
        repo.join("cache/original-buy-product-frames-1/session.json"),
    )?)?;
    let s = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let client =
        DialogClient::restore(&s, serde_json::from_value(source["dialog_client"].clone())?)?;
    let device =
        CameraDevice::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
    let tick = s.simulation.vm.tick;
    let funds = s.simulation.vm.world.budget.clone();
    let (mut v, _) = Viewer::new(args);
    v.resident = source["resident"].as_i64().unwrap() as i16;
    v.floor = device.controls().floor;
    v.zoom = (1u32 << device.controls().zoom) as f32 / 8.;
    v.session = Some(s);
    v.client = Some(client);
    v.device = Some(device);
    v.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    v.refresh()?;
    fn saved(v: &Viewer) -> Result<serde_json::Value> {
        let s = v.session.as_ref().unwrap();
        //Compare typed persisted state, not a second untyped f64 parse of f32
        //decimal strings. This also includes the real dialog and device owners.
        Ok(
            serde_json::json!({"session":s.save(),"dialog_client":v.client.as_ref().unwrap().save(s)?,
            "camera_device":v.device.as_ref().unwrap().save(s)?,"floor":v.floor,"zoom":v.zoom,
            "resident":v.resident,"selected":v.selected,"tutorial_highlight":v.tutorial_highlight,
            "viewport_enter_requested":v.viewport_enter_requested,"move_inputs":v.move_inputs}),
        )
    }
    // The same message emitted by the production viewport's mouse-area enter.
    let _ = v.update(Message::ViewportEnter);
    ensure!(v.viewport_enter_requested && v.presentation_pending());
    v.save()?;
    let queued = saved(&v)?;
    std::fs::write(output.join("queued.json"), serde_json::to_vec(&queued)?)?;
    v.load(&save)?;
    ensure!(queued == saved(&v)?);
    ensure!(v.viewport_enter_requested && !v.step()?);
    ensure!(!v.viewport_enter_requested && v.session.as_ref().unwrap().move_tool_pending());
    v.save()?;
    let active = saved(&v)?;
    v.load(&save)?;
    ensure!(active == saved(&v)?);
    for n in 0..1000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            break;
        }
        let _ = v.update(Message::Frame(Instant::now()));
        ensure!(n < 999, "viewport preview did not return");
    }
    ensure!(v.error.is_none(), "{:?}", v.error);
    let s = v.session.as_ref().unwrap();
    ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.budget == funds);
    let tool = s.host.presentation.move_tool.as_ref().unwrap();
    ensure!(tool.tool.held.is_some() && tool.tool.payload.is_some());
    ensure!(
        tool.tool.byte(0x97) == 1,
        "viewport wrapper did not enable movement"
    );
    ensure!(v.device.as_ref().unwrap().tool_cursor() == Some(0));
    v.save()?;
    let complete = saved(&v)?;
    let pixels = sims_render::software::render(v.device.as_ref().unwrap().scene());
    render_headless_element(&root, &output.join("preview.png"), v.view())?;
    ensure!(complete == saved(&v)?);
    v.load(&save)?;
    ensure!(complete == saved(&v)?);
    v.refresh()?;
    ensure!(pixels == sims_render::software::render(v.device.as_ref().unwrap().scene()));
    render_headless_element(&root, &output.join("restored.png"), v.view())?;
    ensure!(
        image::open(output.join("preview.png"))?.to_rgba8()
            == image::open(output.join("restored.png"))?.to_rgba8()
    );
    std::fs::copy(&save, output.join("completed-session.json"))?;
    let camera = OriginalHouse::native_camera(v.session.as_ref().unwrap())?;
    let pixel = camera
        .screen([488. / 16., 0., 360. / 16.].into())
        .to_array();
    let _ = v.update(Message::ViewportMotion {
        pixel: Some(pixel),
        pan: [0, 0],
    });
    ensure!(v.move_inputs.len() == 1 && v.presentation_pending());
    v.save()?;
    let queued_motion = saved(&v)?;
    v.load(&save)?;
    ensure!(queued_motion == saved(&v)?);
    ensure!(!v.step()? && v.session.as_ref().unwrap().move_tool_pending());
    v.save()?;
    let active_motion = saved(&v)?;
    v.load(&save)?;
    ensure!(active_motion == saved(&v)?);
    for n in 0..1000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            break;
        }
        let _ = v.update(Message::Frame(Instant::now()));
        ensure!(n < 999, "pointer movement did not return");
    }
    ensure!(v.error.is_none(), "{:?}", v.error);
    ensure!(
        v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .tool
            .byte(0x95)
            == 1
    );
    v.save()?;
    std::fs::copy(&save, output.join("moved-session.json"))?;
    render_headless_element(&root, &output.join("moved.png"), v.view())?;
    let _ = v.update(Message::ViewportLeave);
    for n in 0..1000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            break;
        }
        let _ = v.update(Message::Frame(Instant::now()));
        ensure!(n < 999, "pointer leave did not return");
    }
    ensure!(v.error.is_none(), "{:?}", v.error);
    let session = v.session.as_ref().unwrap();
    let tool = session.host.presentation.move_tool.as_ref().unwrap();
    ensure!(tool.tool.byte(0x97) == 0);
    let held = session
        .simulation
        .vm
        .world
        .scheduler_storage
        .as_ref()
        .unwrap()
        .object_for_handle(tool.tool.held.unwrap())
        .unwrap();
    ensure!(session.simulation.vm.world.positions[&held].x == -16);
    v.save()?;
    let left = saved(&v)?;
    let left_pixels = sims_render::software::render(v.device.as_ref().unwrap().scene());
    v.load(&save)?;
    ensure!(left == saved(&v)?);
    std::fs::copy(&save, output.join("left-session.json"))?;
    //Direct load clears the cached scene; the real Load message refreshes it.
    //Compare saved state before that refresh, then verify the visible lot.
    v.refresh()?;
    ensure!(left_pixels == sims_render::software::render(v.device.as_ref().unwrap().scene()));
    render_headless_element(&root, &output.join("left.png"), v.view())?;
    let previous = v
        .session
        .as_ref()
        .unwrap()
        .host
        .presentation
        .move_tool
        .as_ref()
        .unwrap()
        .tool
        .held
        .unwrap();
    let count = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .objects
        .len();
    let _ = v.update(Message::ViewportEnter);
    for n in 0..1000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            break;
        }
        let _ = v.update(Message::Frame(Instant::now()));
        ensure!(n < 999, "reentered viewport did not return");
    }
    ensure!(v.error.is_none(), "{:?}", v.error);
    let s = v.session.as_ref().unwrap();
    let held = s
        .host
        .presentation
        .move_tool
        .as_ref()
        .unwrap()
        .tool
        .held
        .unwrap();
    ensure!(
        held != previous
            && s.simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .unwrap()
                .object_for_handle(previous)
                .is_none()
    );
    ensure!(
        s.simulation.vm.world.objects.len() == count
            && s.simulation.vm.world.budget == funds
            && s.simulation.vm.tick == tick
    );
    v.save()?;
    render_headless_element(&root, &output.join("reentered.png"), v.view())?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tick":tick, "funds_unchanged":true, "actual_viewport_enter":true,
            "viewport_wrapper_487d3d":true, "pointer_enabled_after_preview":true,
            "full_cold_restores":6, "queued_active_and_complete_equal":true,
            "actual_pointer_motion_and_leave":true, "held_preview_moves_without_commit":true,
            "headless_render_preserves_state":true, "restored_pixels_equal":true,
            "move_tool_bound":true, "held_cursor_and_undo_payload":true, "placement_commit":false,"reentry_replaces_held_without_spending_or_leaking":true
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private Goth house; actual Iced buttons and offscreen Metal"]
fn actual_viewport_purchase_buttons_resume_and_cancel_without_duplicate_charge() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(button_viewer)?
        .join()
        .expect("button Viewer panicked")
}
fn button_viewer() -> Result<()> {
    use clap::Parser;
    use widget::shader::Program as _;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let out = repo.join(std::env::var("SIMS_MOVE_BUTTON_ICED_OUTPUT")?);
    ensure!(out.starts_with(repo.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let save = out.join("session.json");
    let house_source = std::env::var("SIMS_MOVE_BUTTON_HOUSE")
        .unwrap_or_else(|_| "UserData/Houses/House05.iff".into());
    let args = Args::parse_from([
        "sims-room",
        "--original",
        &house_source,
        "--inferred-routing",
        "--buy-preview",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    // A fresh imported home avoids pinning this behavioral regression to a
    // historical checkpoint's program fingerprint. All tools are initialized
    // by the same explicit preview profile the player can open.
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    fn drain(v: &mut Viewer) -> Result<()> {
        for _ in 0..1000 {
            ensure!(v.error.is_none(), "{:?}", v.error);
            if !v.presentation_pending() {
                return Ok(());
            }
            let _ = v.update(Message::Frame(Instant::now()));
        }
        anyhow::bail!("button presentation did not finish")
    }
    fn state(v: &Viewer) -> Result<serde_json::Value> {
        let s = v.session.as_ref().unwrap();
        Ok(
            serde_json::json!({"session":s.save(),"camera_device":v.device.as_ref().unwrap().save(s)?,"dialog_client":v.client.as_ref().unwrap().save(s)?,"move_inputs":v.move_inputs,"enter":v.viewport_enter_requested}),
        )
    }
    fn acknowledge_notices(v: &mut Viewer, inputs: &mut Vec<serde_json::Value>) -> Result<()> {
        for _ in 0..8 {
            let dialogs = &v.session.as_ref().unwrap().simulation.vm.world.dialogs;
            let Some(id) = dialogs.modal else {
                return Ok(());
            };
            let record = dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("purchase notice absent")?
                .clone();
            ensure!(
                record.buttons.len() == 1,
                "purchase requires a choice: {}",
                record.title
            );
            let result = record.buttons[0].result;
            inputs.push(
                serde_json::json!({"id":id,"source":record.source,"title":record.title,
                "body":record.body,"result":result}),
            );
            let _ = v.update(Message::Dialog(id, result));
            drain(v)?;
        }
        anyhow::bail!("purchase notice sequence did not finish")
    }
    drain(&mut v)?;
    let _ = v.update(Message::Buy);
    drain(&mut v)?;
    let _ = v.update(Message::ExpandPanel);
    drain(&mut v)?;
    let catalog = v
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap();
    let _ = v.update(Message::BuyNavigate(
        catalog.revision(),
        sims_vm::interface::buy::input::Event::Category(0),
    ));
    drain(&mut v)?;
    let catalog = v
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .catalog()
        .unwrap();
    let product = catalog
        .view()
        .unwrap()
        .products
        .iter()
        .copied()
        .find(|&i| {
            catalog.product(i).is_some_and(|p| {
                p.source.source.ends_with("DiningChairs.iff") && p.fields.price == 80
            })
        })
        .context("original $80 chair absent")?;
    let _ = v.update(Message::BuyProduct(catalog.revision(), product));
    drain(&mut v)?;
    let s = v.session.as_ref().unwrap();
    let tick = s.simulation.vm.tick;
    let balance = s.simulation.vm.world.budget.as_ref().unwrap().balance;
    let _ = v.update(Message::ViewportEnter);
    drain(&mut v)?;
    let camera = OriginalHouse::native_camera(v.session.as_ref().unwrap())?;
    let pixel = camera
        .screen([488. / 16., 0., 360. / 16.].into())
        .to_array();
    let _ = v.update(Message::ViewportMotion {
        pixel: Some(pixel),
        pan: [0, 0],
    });
    drain(&mut v)?;
    let scene = v.scene.as_ref().unwrap().clone();
    let bounds = iced::Rectangle {
        x: 13.,
        y: 37.,
        width: scene.width as f32 * 1.2 + 80.,
        height: scene.height as f32 * 1.2,
    };
    let cursor = iced::mouse::Cursor::Available(iced::Point::new(
        bounds.x + 40. + pixel[0] * 1.2,
        bounds.y + pixel[1] * 1.2,
    ));
    let shader = Viewport(scene.clone());
    let mut pressed = viewport_drag::Pointer::default();
    for down in [true, false] {
        let event = if down {
            iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)
        } else {
            iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)
        };
        let (message, _, status) = shader
            .update(&mut pressed, &iced::Event::Mouse(event), bounds, cursor)
            .context("button ignored")?
            .into_inner();
        ensure!(status == iced::event::Status::Captured);
        let _ = v.update(message.context("button message absent")?);
    }
    ensure!(pressed.left.is_none() && v.move_inputs.len() == 2);
    //The queued Down and Up are immutable player input across a cold restore.
    v.save()?;
    let queued = state(&v)?;
    std::fs::copy(&save, out.join("queued.json"))?;
    v.load(&save)?;
    ensure!(queued == state(&v)?);
    ensure!(!v.step()? && v.session.as_ref().unwrap().move_button_pending());
    v.save()?;
    let active = state(&v)?;
    std::fs::copy(&save, out.join("active.json"))?;
    v.load(&save)?;
    ensure!(active == state(&v)?);
    drain(&mut v)?;
    let s = v.session.as_ref().unwrap();
    ensure!(
        s.simulation.vm.tick == tick
            && s.simulation.vm.world.budget.as_ref().unwrap().balance == balance - 80
    );
    v.save()?;
    std::fs::copy(&save, out.join("post-purchase.json"))?;
    let mut notice_inputs = vec![];
    acknowledge_notices(&mut v, &mut notice_inputs)?;
    let s = v.session.as_ref().unwrap();
    let repeated_preview = s
        .host
        .presentation
        .move_tool
        .as_ref()
        .unwrap()
        .tool
        .held
        .is_some();
    //A completed purchase need not leave a held repeat preview. Exercise
    //Escape against an unpaid preview in either case, using another actual
    //catalog click when the caller has already released its held object.
    if !repeated_preview {
        let _ = v.update(Message::ViewportLeave);
        drain(&mut v)?;
        let revision = v
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .catalog()
            .context("purchase catalog absent")?
            .revision();
        let _ = v.update(Message::BuyProduct(revision, product));
        drain(&mut v)?;
        let _ = v.update(Message::ViewportEnter);
        drain(&mut v)?;
        let _ = v.update(Message::ViewportMotion {
            pixel: Some(pixel),
            pan: [0, 0],
        });
        drain(&mut v)?;
        v.save()?;
        std::fs::copy(&save, out.join("preview-reselected.json"))?;
        acknowledge_notices(&mut v, &mut notice_inputs)?;
        ensure!(
            v.session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .held
                .is_some(),
            "second catalog click did not create a preview"
        );
    }
    let _ = v.update(Message::DialogKey(27));
    drain(&mut v)?;
    let s = v.session.as_ref().unwrap();
    ensure!(
        s.simulation.vm.tick == tick
            && s.simulation.vm.world.budget.as_ref().unwrap().balance == balance - 80
    );
    ensure!(
        s.simulation
            .vm
            .world
            .objects
            .values()
            .all(|o| o.cursor.is_none())
    );
    v.refresh()?;
    v.save()?;
    let complete = state(&v)?;
    let published = v
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .texts()
        .unwrap()
        .funds();
    let displayed = household_funds_label(v.session.as_ref().unwrap(), published)?;
    ensure!(displayed.replace(['$', ','], "").parse::<i32>()? == balance - 80);
    render_headless_element(&root, &out.join("purchased.png"), v.view())?;
    ensure!(complete == state(&v)?);
    v.load(&save)?;
    ensure!(complete == state(&v)?);
    v.refresh()?;
    render_headless_element(&root, &out.join("restored.png"), v.view())?;
    ensure!(
        image::open(out.join("purchased.png"))?.to_rgba8()
            == image::open(out.join("restored.png"))?.to_rgba8()
    );
    //A release outside the viewport still ends a drag; an unrelated outside
    //release is ignored. Focus loss clears physical Shift and edge scrolling.
    let press = iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left));
    shader
        .update(&mut pressed, &press, bounds, cursor)
        .context("press ignored")?;
    let release = iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ));
    let outside = iced::mouse::Cursor::Available(iced::Point::new(bounds.x - 20., bounds.y - 10.));
    let (message, _, _) = shader
        .update(&mut pressed, &release, bounds, outside)
        .context("outside release lost")?
        .into_inner();
    ensure!(
        matches!(message,Some(Message::ViewportButton{down:false,pixel,..}) if pixel[0]<0. && pixel[1]<0.)
    );
    ensure!(
        pressed.left.is_none()
            && shader
                .update(&mut pressed, &release, bounds, outside)
                .is_none()
    );
    let _ = v.update(Message::Shift(true));
    let _ = v.update(Message::Pan([1, 1]));
    let _ = v.update(Message::FocusLost);
    ensure!(!v.shift && v.pan == [0, 0]);
    // Keep the actual purchased-Buy checkpoint for the controller differential.
    std::fs::copy(&save, out.join("buy-session.json"))?;
    let before_live_ids = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .objects
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let _ = v.update(Message::Live);
    let mut picker_selection_checkpoints = 0;
    for _ in 0..1000 {
        ensure!(v.error.is_none(), "{:?}", v.error);
        if !v.presentation_pending() {
            break;
        }
        if picker_selection_checkpoints == 0
            && v.session
                .as_ref()
                .unwrap()
                .simulation
                .native_selection
                .is_some()
            && v.session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .current_tool
                .as_ref()
                .is_some_and(|c| {
                    matches!(
                        c.request(),
                        Some(sims_vm::interface::current_tool::Request::Enter(_))
                    )
                })
        {
            v.save()?;
            let pending = state(&v)?;
            std::fs::copy(&save, out.join("picker-selection.json"))?;
            let mut invalid = pending["session"].clone();
            let owner = invalid["view"]["current_tool"]["call"]["publication"]
                .as_u64()
                .context("picker selection publication owner absent")?;
            invalid["view"]["current_tool"]["call"]["publication"] = (owner + 1).into();
            ensure!(
                Session::restore(&Index::scan(&root)?, serde_json::from_value(invalid)?).is_err(),
                "picker selection accepted a foreign publication owner"
            );
            v.load(&save)?;
            ensure!(state(&v)? == pending, "picker selection changed on restore");
            picker_selection_checkpoints += 1;
        }
        let _ = v.update(Message::Frame(Instant::now()));
    }
    ensure!(
        !v.presentation_pending(),
        "Live presentation did not finish"
    );
    let s = v.session.as_ref().unwrap();
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
    ensure!(
        s.simulation.vm.tick == tick
            && s.simulation.vm.world.budget.as_ref().unwrap().balance == balance - 80
    );
    let helpers = s
        .simulation
        .vm
        .world
        .objects
        .iter()
        .filter(|(id, _)| !before_live_ids.contains(id))
        .map(|(&id, o)| (id, o.definition.as_ref().map(|d| d.source.clone())))
        .collect::<Vec<_>>();
    ensure!(
        helpers.iter().any(|(_, source)| source
            .as_ref()
            .is_some_and(|s| s.to_ascii_lowercase().contains("comeandsee"))),
        "return to Live did not create a controller for the newly purchased chair"
    );
    v.args.save = out.join("live-session.json");
    v.save()?;
    let post_live = state(&v)?;
    let live_save = v.args.save.clone();
    render_headless_element(&root, &out.join("live.png"), v.view())?;
    ensure!(
        post_live == state(&v)?,
        "post-Live drawing changed simulation"
    );
    v.load(&live_save)?;
    ensure!(post_live == state(&v)?, "post-Live cold restore differs");
    v.refresh()?;
    render_headless_element(&root, &out.join("live-restored.png"), v.view())?;
    ensure!(
        image::open(out.join("live.png"))?.to_rgba8()
            == image::open(out.join("live-restored.png"))?.to_rgba8()
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"fresh_original_startup":true,"actual_iced_buttons":true,"cold_restores":4+picker_selection_checkpoints,"picker_selection_checkpoints":picker_selection_checkpoints,"queue_and_active_equal":true,"funds_before":balance,"funds_after":balance-80,"tick":tick,"repeated_preview":repeated_preview,"notice_inputs":notice_inputs,"cancel_preserves_purchase":true,"frames_equal":true,"outside_release_and_focus_checked":true,"displayed_funds":displayed,"replacement_funds_read_preserves_native_widget":true,"returned_to_live":true,"post_live_cold_restore_equal":true,"created_helpers":helpers}),
        )?,
    )?;
    Ok(())
}
