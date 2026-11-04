use super::*;
use clap::Parser;
use widget::shader::Program as _;

pub(super) fn object_pixel(v: &Viewer, id: i16) -> Option<[f32; 2]> {
    let scene = v.scene.as_ref()?;
    for draw in scene.draws.iter().filter(|d| d.object == Some(id)) {
        let points: Vec<_> = scene.vertices
            [draw.vertices.start as usize..draw.vertices.end as usize]
            .iter()
            .map(|p| {
                [
                    (p.position[0] + 1.) * scene.width as f32 / 2.,
                    (1. - p.position[1]) * scene.height as f32 / 2.,
                ]
            })
            .collect();
        let min: [f32; 2] =
            std::array::from_fn(|i| points.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min));
        let max: [f32; 2] = std::array::from_fn(|i| {
            points
                .iter()
                .map(|p| p[i])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        for y in (min[1].max(0.) as u32..(max[1].ceil() as u32).min(scene.height)).step_by(2) {
            for x in (min[0].max(0.) as u32..(max[0].ceil() as u32).min(scene.width)).step_by(2) {
                let pixel = [x as f32 + 0.5, y as f32 + 0.5];
                if scene.pick(pixel) == Some(id) {
                    return Some(pixel);
                }
            }
        }
    }
    None
}

#[test]
#[ignore = "private stock furniture; actual Iced input and offscreen Metal"]
fn actual_viewer_picks_up_stock_furniture_and_resumes_cancel_or_drop() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(pickup_viewer)?
        .join()
        .expect("pickup Viewer panicked")
}
fn pickup_viewer() -> Result<()> {
    pickup_viewer_with_camera(false)
}
#[test]
#[ignore = "private stock furniture; actual Iced camera input and offscreen Metal"]
fn actual_viewer_rotates_held_stock_furniture_then_resumes_cancel_or_drop() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| pickup_viewer_with_camera(true))?
        .join()
        .expect("camera/pickup Viewer panicked")
}
fn pickup_viewer_with_camera(camera: bool) -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let out = repo.join(std::env::var(if camera {
        "SIMS_BUY_CAMERA_ICED_OUTPUT"
    } else {
        "SIMS_MOVE_PICKUP_ICED_OUTPUT"
    })?);
    ensure!(out.starts_with(repo.join("cache")) && !out.exists());
    std::fs::create_dir(&out)?;
    let save = out.join("session.json");
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
        repo.join("cache/original-move-button-iced-3/session.json"),
    )?)?;
    let s = Session::restore(&index, serde_json::from_value(source["session"].clone())?)?;
    let client =
        DialogClient::restore(&s, serde_json::from_value(source["dialog_client"].clone())?)?;
    let d = CameraDevice::restore(&s, serde_json::from_value(source["camera_device"].clone())?)?;
    let tick = s.simulation.vm.tick;
    let budget = s.simulation.vm.world.budget.clone();
    let (mut v, _) = Viewer::new(args);
    v.resident = source["resident"].as_i64().unwrap() as i16;
    v.floor = d.controls().floor;
    v.zoom = (1u32 << d.controls().zoom) as f32 / 8.;
    v.session = Some(s);
    v.client = Some(client);
    v.device = Some(d);
    v.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    v.refresh()?;
    fn drain(v: &mut Viewer) -> Result<()> {
        for _ in 0..2000 {
            ensure!(v.error.is_none(), "{:?}", v.error);
            if !v.presentation_pending() {
                return Ok(());
            }
            let _ = v.update(Message::Frame(Instant::now()));
        }
        anyhow::bail!("pickup Iced input stalled")
    }
    fn state(v: &Viewer) -> Result<serde_json::Value> {
        let s = v.session.as_ref().unwrap();
        Ok(
            serde_json::json!({"session":s.save(),"camera_device":v.device.as_ref().unwrap().save(s)?,"dialog_client":v.client.as_ref().unwrap().save(s)?,"inputs":v.move_inputs}),
        )
    }
    fn buttons(v: &mut Viewer, pixel: [f32; 2], expected: Option<i16>) -> Result<()> {
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
        let shader = Viewport(scene);
        let mut pressed = viewport_drag::Pointer::default();
        for down in [true, false] {
            let event = if down {
                iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)
            } else {
                iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)
            };
            let (message, _, status) = shader
                .update(&mut pressed, &iced::Event::Mouse(event), bounds, cursor)
                .context("pickup pointer ignored")?
                .into_inner();
            ensure!(status == iced::event::Status::Captured);
            let message = message.context("pickup pointer message absent")?;
            if down && let Some(id) = expected {
                ensure!(
                    matches!(message,Message::ViewportButton{picked:Some(p),..} if p==id),
                    "shader picked wrong furniture"
                );
            }
            let _ = v.update(message);
        }
        Ok(())
    }
    // Select genuinely visible stock furniture through depth-aware picking.
    let (target, price, mut pixel) = [(198, 150), (191, 360), (192, 360), (173, 250), (174, 250)]
        .into_iter()
        .find_map(|(id, price)| object_pixel(&v, id).map(|p| (id, price, p)))
        .context("no supported stock furniture visible")?;
    let original = v.session.as_ref().unwrap().simulation.vm.world.positions[&target].clone();
    let mut offsets = vec![];
    let mut placements = vec![];
    for cancel in [true, false] {
        buttons(&mut v, pixel, Some(target))?;
        drain(&mut v)?;
        let s = v.session.as_ref().unwrap();
        let selection = v
            .device
            .as_ref()
            .unwrap()
            .furniture_selection()
            .context("selected controls absent")?;
        ensure!(selection.price == price && selection.move_allowed && selection.sell_allowed);
        let mut offset = s
            .host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .tool
            .tool
            .position;
        ensure!(
            offset != [0, 0],
            "test must grab above furniture's ground origin"
        );
        offsets.push(offset);
        if camera {
            let held = s.host.presentation.move_tool.as_ref().unwrap().tool.held;
            let _ = v.update(Message::ZoomIn);
            drain(&mut v)?;
            let _ = v.update(Message::ZoomOut);
            drain(&mut v)?;
            let _ = v.update(Message::Rotate);
            drain(&mut v)?;
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
                    == held
            );
            if cancel {
                let before = state(&v)?;
                render_headless_element(&root, &out.join("held-rotated.png"), v.view())?;
                ensure!(before == state(&v)?);
            }
            let _ = v.update(Message::RotateBack);
            drain(&mut v)?;
            offset = v
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .tool
                .position;
            ensure!(
                offset == [0, 0],
                "camera did not reset held furniture grab offsets"
            );
        }
        let mut accepted = None;
        for physical in [[376, 488], [392, 488], [408, 488], [424, 520], [376, 520]] {
            let s = v.session.as_ref().unwrap();
            let camera = OriginalHouse::native_camera(s)?;
            let ground = camera
                .screen(OriginalHouse::ground_point(s, physical, 1)?)
                .to_array();
            let destination = [ground[0] + offset[1] as f32, ground[1] + offset[0] as f32];
            let _ = v.update(Message::ViewportMotion {
                pixel: Some(destination),
                pan: [0, 0],
            });
            drain(&mut v)?;
            let s = v.session.as_ref().unwrap();
            let h = s
                .host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .held
                .context("motion dropped cursor")?;
            let cursor = s
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .unwrap()
                .object_for_handle(h)
                .unwrap();
            let valid = s.simulation.vm.world.objects[&cursor]
                .cursor
                .as_ref()
                .unwrap()
                .valid
                != 0;
            placements.push(serde_json::json!({"cancel":cancel,"physical":physical,"valid":valid,"position":s.simulation.vm.world.positions[&target]}));
            if valid {
                accepted = Some(destination);
                break;
            }
        }
        let destination = accepted.context("no valid lawn placement for stock furniture")?;
        let current = &v.session.as_ref().unwrap().simulation.vm.world.positions[&target];
        ensure!(
            *current != original && current.x > 0 && current.y > 0,
            "valid pointer did not place held furniture"
        );
        v.save()?;
        let held = state(&v)?;
        std::fs::copy(
            &save,
            out.join(if cancel {
                "held-cancel.json"
            } else {
                "held-drop.json"
            }),
        )?;
        if cancel {
            render_headless_element(&root, &out.join("held.png"), v.view())?;
            ensure!(held == state(&v)?);
        }
        v.load(&save)?;
        ensure!(held == state(&v)?);
        v.refresh()?;
        if cancel {
            let _ = v.update(Message::DialogKey(27));
        } else {
            buttons(&mut v, destination, None)?;
        }
        drain(&mut v)?;
        let s = v.session.as_ref().unwrap();
        ensure!(s.simulation.vm.tick == tick && s.simulation.vm.world.budget == budget);
        ensure!(
            s.host
                .presentation
                .move_tool
                .as_ref()
                .unwrap()
                .tool
                .held
                .is_none()
                && v.device.as_ref().unwrap().furniture_selection().is_none()
        );
        if cancel {
            ensure!(
                s.simulation.vm.world.positions[&target] == original,
                "Escape changed original furniture placement"
            );
            pixel = object_pixel(&v, target).context("restored furniture not visible")?;
        } else {
            ensure!(s.simulation.vm.world.positions[&target] != original);
        }
    }
    v.save()?;
    let complete = state(&v)?;
    render_headless_element(&root, &out.join("moved.png"), v.view())?;
    ensure!(complete == state(&v)?);
    v.load(&save)?;
    ensure!(complete == state(&v)?);
    v.refresh()?;
    render_headless_element(&root, &out.join("restored.png"), v.view())?;
    ensure!(
        image::open(out.join("moved.png"))?.to_rgba8()
            == image::open(out.join("restored.png"))?.to_rgba8()
    );
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"actual_iced_buttons":true,"actual_iced_camera":camera,"cold_restores":3,"tick":tick,"budget_unchanged":true,"grab_offsets":offsets,"placement_attempts":placements,"selected_value":price,"selected_stock_object":target,"cancel_restores_original_placement":true,"move_commits":true,"frames_equal":true,"render_preserves_save":true}),
        )?,
    )?;
    Ok(())
}
pub(super) fn click(v: &mut Viewer, id: i16, pixel: [f32; 2]) -> Result<()> {
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
    let (message, _, captured) = Viewport(scene)
        .update(
            &mut Default::default(),
            &iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            bounds,
            cursor,
        )
        .context("object click not captured")?
        .into_inner();
    ensure!(captured == iced::event::Status::Captured);
    let message = message.context("object click message absent")?;
    ensure!(
        matches!(message, Message::ViewportButton { down: true, picked: Some(picked), .. } if picked == id)
    );
    let _ = v.update(message);
    ensure!(v.error.is_none(), "click: {:?}", v.error);
    Ok(())
}
pub(super) fn menu(v: &mut Viewer) -> Result<()> {
    for _ in 0..1000 {
        if v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .object_input
            .as_ref()
            .is_some_and(|s| s.ready())
        {
            return Ok(());
        }
        v.step()?;
    }
    anyhow::bail!("object menu exceeded budget")
}

#[test]
#[ignore = "private Newbie household and headless Iced Metal rendering"]
fn original_object_click_sit_queue_cancel_and_tutorial_boundary_resume() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_OBJECT_UI_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace object UI evidence")?;
    let source = repo.join("cache/newbie-ground-ui-final/session.json");
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    let actor = v.resident;
    let (clicked, pixel) = [127, 128, 158, 159]
        .into_iter()
        .find_map(|id| object_pixel(&v, id).map(|p| (id, p)))
        .context("no original chair has a visible selectable pixel")?;
    click(&mut v, clicked, pixel)?;
    super::person_tests::checkpoint(&mut v, &output, "query")?;
    menu(&mut v)?;
    super::person_tests::checkpoint(&mut v, &output, "chair-menu")?;
    render_headless_element(&root, &output.join("chair-menu.png"), v.view())?;
    let before_cancel = v.session.as_ref().unwrap().save();
    let _ = v.update(Message::ObjectCancel);
    ensure!(v.error.is_none());
    ensure!(v.session.as_ref().unwrap().simulation == before_cancel.simulation.state);
    click(&mut v, clicked, pixel)?;
    menu(&mut v)?;
    let s = v.session.as_ref().unwrap();
    let input = s.host.presentation.object_input.as_ref().unwrap();
    let target = input.target();
    let choice = input
        .actions()
        .iter()
        .position(|a| {
            s.simulation
                .vm
                .action_menu(&s.program, a.target)
                .unwrap()
                .iter()
                .any(|e| e.id == a.menu_id && e.label == "Sit")
        })
        .context("Sit absent")?;
    let _ = v.update(Message::ObjectChoice(choice));
    ensure!(v.error.is_none());
    let action = v
        .session
        .as_ref()
        .unwrap()
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .unwrap()
        .actors[&actor]
        .queue
        .pending
        .iter()
        .find(|a| a.target == target)
        .context("queued Sit absent")?
        .clone();
    ensure!(action.priority == 50);
    super::person_tests::checkpoint(&mut v, &output, "queued-sit")?;
    render_headless_element(&root, &output.join("queued-sit.png"), v.view())?;
    let mut active_saved = false;
    let mut active_seen = false;
    let expected = "original UI wait requires its widget owner: Control(4500)";
    let mut boundary = false;
    for i in 0..4000 {
        let _ = v.update(Message::Step);
        if let Some(error) = &v.error {
            std::fs::write(output.join("boundary-error.txt"), error)?;
            ensure!(
                error == expected,
                "unexpected object lesson boundary: {error}"
            );
            let s = v.session.as_ref().unwrap();
            ensure!(
                s.pending_ui_observation()
                    == Some(sims_vm::ui_owner::wait::Condition::Control(4500))
            );
            ensure!(s.simulation.vm.threads.values().any(|t| t.stack.iter().any(
                |f| f.location.bhav == 4113
                    && f.location.instruction == 10
                    && f.location.source.ends_with("Tutorial.iff")
            )));
            boundary = true;
            break;
        }
        let s = v.session.as_ref().unwrap();
        let running = s.simulation.vm.world.actions.as_ref().unwrap().actors[&actor]
            .active
            .as_ref()
            .is_some_and(|a| a.id == action.id);
        active_seen |= running;
        if running && !active_saved && i > 4 {
            super::person_tests::checkpoint(&mut v, &output, "active-sit")?;
            render_headless_element(&root, &output.join("active-sit.png"), v.view())?;
            active_saved = true;
        }
        if i % 50 == 0 {
            println!(
                "object slice{i}: tick{} running{running}",
                v.session.as_ref().unwrap().simulation.vm.tick
            );
        }
        if i == 3999 {
            v.args.save = output.join("boundary-session.json");
            v.save()?;
            anyhow::bail!("object lesson did not advance");
        }
    }
    ensure!(active_seen && active_saved && boundary);
    super::person_tests::checkpoint(&mut v, &output, "object-lesson")?;
    let frozen = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    let _ = v.update(Message::Step);
    ensure!(v.error.as_deref() == Some(expected));
    ensure!(frozen == serde_json::to_vec(&v.session.as_ref().unwrap().save())?);
    v.args.save = save;
    v.save()?;
    render_headless_element(&root, &output.join("object-lesson.png"), v.view())?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start":start,"tick":v.session.as_ref().unwrap().simulation.vm.tick,"actor":actor,"clicked":clicked,
            "target":target,"pixel":pixel,"action":action,"next_original_boundary":expected,"active_seen":active_seen,
            "active_saved":active_saved,"actual_viewport_click":true,"cancel_preserves_vm_and_object":true,
            "boundary_repeats_without_mutation":true,"tutorial_bhav":4113,"tutorial_instruction":10,
            "native_execution_this_turn":false
        }))?,
    )?;
    Ok(())
}
