use super::*;
use clap::Parser;
use widget::shader::Program as _;

pub(super) fn click_point(v: &Viewer) -> Result<([f32; 2], [i32; 2])> {
    let s = v.session.as_ref().unwrap();
    let camera = OriginalHouse::native_camera(s)?;
    let at = s.simulation.vm.world.positions[&v.resident];
    let lot = s
        .simulation
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap();
    let floor = lot.floor(at.level).unwrap();
    let room = floor.rooms.primary[(at.y >> 4) as usize * lot.size + (at.x >> 4) as usize];
    for distance in [32, 48, 64, 16] {
        for offset in [
            [distance, 0],
            [0, distance],
            [-distance, 0],
            [0, -distance],
            [distance, distance],
            [-distance, -distance],
        ] {
            let point = [at.x + offset[0], at.y + offset[1]];
            if point
                .iter()
                .any(|v| *v < 16 || *v >= ((lot.size - 1) * 16) as i32)
            {
                continue;
            }
            if floor.rooms.primary[(point[1] >> 4) as usize * lot.size + (point[0] >> 4) as usize]
                != room
            {
                continue;
            }
            let height = sims_sim::pose::transform::Terrain {
                size: lot.size,
                corners: &lot.heights,
            }
            .height(point)
            .map_err(anyhow::Error::msg)?
            .unwrap_or(0.);
            let scale = (1. / (2. * 6_f64.sqrt())) as f32;
            let y = sims_render::floor_elevation(at.level)
                + (f64::from(scale) * 3. * height) as f32 / 3.;
            let pixel = camera
                .screen([point[1] as f32 / 16., y, point[0] as f32 / 16.].into())
                .to_array();
            if camera.pick_original_ground(lot, pixel)? == Some(point)
                && v.scene.as_ref().unwrap().pick(pixel).is_none()
            {
                return Ok((pixel, point));
            }
        }
    }
    anyhow::bail!("no visible empty floor near the selected resident")
}
fn viewport_click(v: &mut Viewer, pixel: [f32; 2]) -> Result<()> {
    let scene = v.scene.as_ref().unwrap().clone();
    // Exercise Iced's actual viewport event and letterbox conversion, not a
    // direct Session call or a preselected destination helper.
    let bounds = iced::Rectangle {
        x: 23.,
        y: 51.,
        width: scene.width as f32 * 1.25 + 80.,
        height: scene.height as f32 * 1.25,
    };
    let scale = (bounds.width / scene.width as f32).min(bounds.height / scene.height as f32);
    let mouse = iced::Point::new(
        bounds.x + (bounds.width - scene.width as f32 * scale) * 0.5 + pixel[0] * scale,
        bounds.y + (bounds.height - scene.height as f32 * scale) * 0.5 + pixel[1] * scale,
    );
    let action = Viewport(scene)
        .update(
            &mut Default::default(),
            &iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            bounds,
            iced::mouse::Cursor::Available(mouse),
        )
        .context("viewport did not capture click")?;
    let (message, _, captured) = action.into_inner();
    ensure!(captured == iced::event::Status::Captured);
    let message = message.context("viewport message absent")?;
    ensure!(
        matches!(
            message,
            Message::ViewportButton {
                down: true,
                picked: None,
                ..
            }
        ),
        "ground point picked an object"
    );
    let _ = v.update(message);
    ensure!(v.error.is_none(), "viewport click: {:?}", v.error);
    Ok(())
}
fn pump_menu(v: &mut Viewer) -> Result<()> {
    for _ in 0..1000 {
        if v.session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .ground_input
            .as_ref()
            .is_none_or(|s| s.ready())
        {
            return Ok(());
        }
        v.step()?;
    }
    anyhow::bail!("ground UI continuation exceeded budget")
}

#[test]
#[ignore = "private original Newbie tutorial; headless Iced/Metal rendering"]
fn original_ground_click_queue_cancel_and_movement_lesson() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_GROUND_UI_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace ground UI evidence")?;
    let source = repo.join("cache/newbie-person-button-click-cold/session.json");
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
    let before = v.session.as_ref().unwrap().simulation.vm.world.positions[&actor];
    let (pixel, physical) = click_point(&v)?;
    viewport_click(&mut v, pixel)?;
    super::person_tests::checkpoint(&mut v, &output, "constructing-ground")?;
    pump_menu(&mut v)?;
    super::person_tests::checkpoint(&mut v, &output, "ground-menu")?;
    let s = v.session.as_ref().unwrap();
    let target = s
        .host
        .presentation
        .ground_input
        .as_ref()
        .unwrap()
        .target()
        .unwrap();
    ensure!(s.host.presentation.picker_target == Some(target));
    ensure!(s.simulation.vm.world.objects[&target].native_flags.unwrap() & 1 != 0);
    ensure!(s.simulation.vm.tick == start);
    render_headless_element(&root, &output.join("ground-menu.png"), v.view())?;
    let _ = v.update(Message::GroundCancel);
    ensure!(v.error.is_none());
    // The shared destructor belongs to this ground cancellation. Removing its
    // owner must still fail the dialog client's validation, not become a broad
    // exemption for every non-portrait removal.
    let owner = v
        .session
        .as_mut()
        .unwrap()
        .host
        .presentation
        .ground_input
        .take();
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .save(v.session.as_ref().unwrap())
            .is_err()
    );
    v.session.as_mut().unwrap().host.presentation.ground_input = owner;
    super::person_tests::checkpoint(&mut v, &output, "cancelling-ground")?;
    pump_menu(&mut v)?;
    ensure!(
        !v.session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .objects
            .contains_key(&target)
    );
    v.refresh()?;
    viewport_click(&mut v, pixel)?;
    pump_menu(&mut v)?;
    let s = v.session.as_ref().unwrap();
    let input = s.host.presentation.ground_input.as_ref().unwrap();
    let target = input.target().unwrap();
    let destination = s.simulation.vm.world.positions[&target];
    let choice = input
        .actions()
        .iter()
        .position(|a| a.menu_id == 2)
        .context("Go Here choice missing")?;
    let _ = v.update(Message::GroundChoice(choice));
    ensure!(v.error.is_none() && !v.session.as_ref().unwrap().ground_input_pending());
    let a = &v
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
        .pending;
    let action = a
        .iter()
        .find(|a| a.target == target && a.menu_id == 2)
        .context("native Go Here was not queued")?
        .clone();
    ensure!(action.priority == 50);
    super::person_tests::checkpoint(&mut v, &output, "queued-go-here")?;
    render_headless_element(&root, &output.join("queued-go-here.png"), v.view())?;
    let mut moving_snapshot = false;
    let mut reached = false;
    let mut dialogs = vec![];
    let mut walking_notice = None;
    // Go Here sets object9=1. Native58dc72 stops within 1.5 times the
    // walking endpoint threshold, and589d47 projects the current coordinate
    // onto the path. Completion does not snap to the exact destination.
    let endpoint_threshold = f64::from(
        sims_sim::pose::movement::path::Settings::COMPLETE_COLLECTION.endpoint_distance[0],
    ) * 1.5;
    let distance_squared = |at: sims_vm::autonomy::Placement| {
        f64::from(at.x - destination.x).powi(2) + f64::from(at.y - destination.y).powi(2)
    };
    for i in 0..1000 {
        let _ = v.update(Message::Step);
        if let Some(error) = &v.error {
            std::fs::write(output.join("boundary-error.txt"), error)?;
            v.args.save = output.join("boundary-session.json");
            v.save()?;
            anyhow::bail!("movement UI: {}", v.error.as_ref().unwrap());
        }
        let s = v.session.as_ref().unwrap();
        let at = s.simulation.vm.world.positions[&actor];
        reached |=
            at.level == destination.level && distance_squared(at) < endpoint_threshold.powi(2);
        let active = s.simulation.vm.world.actions.as_ref().unwrap().actors[&actor]
            .active
            .as_ref();
        let running = active.is_some_and(|a| a.id == action.id);
        let next: Vec<_> = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .filter(|r| r.status == 1 && r.origin.bhav == 4125 && r.origin.instruction == 7)
            .map(|r| serde_json::to_value(r).unwrap())
            .collect();
        if !next.is_empty() {
            dialogs = next;
        }
        let modal = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| Some(r.id) == s.simulation.vm.world.dialogs.modal)
            .cloned();
        if let Some(record) = modal {
            ensure!(
                walking_notice.is_none()
                    && record.origin.bhav == 4125
                    && record.origin.instruction == 5
                    && record.body == "Good, now watch as Bob walks to that location."
                    && record.buttons.len() == 1
                    && record.buttons[0].result == 2,
                "unexpected tutorial notice: {record:?}"
            );
            super::person_tests::checkpoint(&mut v, &output, "walking-notice")?;
            render_headless_element(&root, &output.join("walking-notice.png"), v.view())?;
            let _ = v.update(Message::Dialog(record.id, record.buttons[0].result));
            ensure!(v.error.is_none(), "walking notice: {:?}", v.error);
            walking_notice = Some(record);
            continue;
        }
        if running && at != before && !moving_snapshot {
            super::person_tests::checkpoint(&mut v, &output, "walking")?;
            render_headless_element(&root, &output.join("walking.png"), v.view())?;
            moving_snapshot = true;
        }
        if reached && !running && !dialogs.is_empty() {
            break;
        }
        if i % 25 == 0 {
            println!(
                "movement slice {i}: tick{} position{at:?} running{running} reached{reached}",
                v.session.as_ref().unwrap().simulation.vm.tick
            );
        }
        if i == 999 {
            v.args.save = output.join("boundary-session.json");
            v.save()?;
            anyhow::bail!("movement/lesson did not complete");
        }
    }
    ensure!(reached && moving_snapshot && !dialogs.is_empty() && walking_notice.is_some());
    let s = v.session.as_ref().unwrap();
    let final_position = s.simulation.vm.world.positions[&actor];
    let object = &s.simulation.vm.world.objects[&actor];
    ensure!(
        final_position.level == destination.level
            && distance_squared(final_position) < endpoint_threshold.powi(2)
            && object.data[&9] == 0
            && object.person.as_ref().unwrap()[&62] == 0
            && object.person.as_ref().unwrap()[&39] == 1,
        "Go Here did not finish with native route success"
    );
    super::person_tests::checkpoint(&mut v, &output, "movement-lesson")?;
    v.args.save = save;
    v.save()?;
    render_headless_element(&root, &output.join("movement-lesson.png"), v.view())?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start":start,"tick":v.session.as_ref().unwrap().simulation.vm.tick,"actor":actor,"target":target,
            "pixel":pixel,"physical":physical,"destination":destination,"action":action,"reached":reached,
            "saved_during_movement":moving_snapshot,"next_original_dialogs":dialogs,
            "acknowledged_original_walking_notice":walking_notice,
            "final_position":final_position,"endpoint_distance":distance_squared(final_position).sqrt(),
            "native_walk_endpoint_threshold":endpoint_threshold,"route_success":true,
            "viewport_event_used":true,"cancel_branch_removed_helper":true,"native_execution_this_turn":false
        }))?,
    )?;
    Ok(())
}
