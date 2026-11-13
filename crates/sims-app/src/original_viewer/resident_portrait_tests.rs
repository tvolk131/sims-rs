use crate::original_viewer::*;
use clap::Parser;
use serde_json::{Value, json};
use std::time::Duration;

fn state(viewer: &Viewer) -> Result<Value> {
    let session = viewer
        .session
        .as_ref()
        .context("portrait test session absent")?;
    Ok(json!({
        "session":session.save(),
        "client":viewer.client.as_ref().context("portrait test client absent")?.save(session)?,
        "device":viewer.device.as_ref().context("portrait test device absent")?.save(session)?,
    }))
}

fn tick_rng(viewer: &Viewer) -> Result<(u64, [Option<u32>; 3])> {
    let vm = &viewer
        .session
        .as_ref()
        .context("portrait test session absent")?
        .simulation
        .vm;
    Ok((
        vm.tick,
        [Some(vm.rng_state), vm.lot_rng_state, vm.crt_rng_state],
    ))
}

fn restore(viewer: &mut Viewer) -> Result<()> {
    let before = state(viewer)?;
    let index = viewer.session.as_ref().unwrap().index().clone();
    let session = Session::restore(&index, serde_json::from_value(before["session"].clone())?)?;
    let client =
        DialogClient::restore(&session, serde_json::from_value(before["client"].clone())?)?;
    let device =
        CameraDevice::restore(&session, serde_json::from_value(before["device"].clone())?)?;
    viewer.session = Some(session);
    viewer.client = Some(client);
    viewer.device = Some(device);
    ensure!(
        state(viewer)? == before,
        "portrait camera cold restore changed full components"
    );
    Ok(())
}

fn advance_cold(viewer: &mut Viewer) -> Result<()> {
    let session = viewer.session.as_ref().unwrap();
    let mut warm_session = Session::restore(session.index(), session.save())?;
    let mut warm_device = viewer.device.as_ref().unwrap().fork_for_exploration();
    warm_device.advance(&mut warm_session, 1)?;
    restore(viewer)?;
    viewer
        .device
        .as_mut()
        .unwrap()
        .advance(viewer.session.as_mut().unwrap(), 1)?;
    let session = viewer.session.as_ref().unwrap();
    ensure!(
        serde_json::to_value(session.save())? == serde_json::to_value(warm_session.save())?,
        "cold camera service changed native continuation"
    );
    ensure!(
        serde_json::to_value(viewer.device.as_ref().unwrap().save(session)?)?
            == serde_json::to_value(warm_device.save(&warm_session)?)?,
        "cold camera service changed retained renderer state"
    );
    ensure!(
        sims_render::software::render(viewer.device.as_ref().unwrap().scene()).into_raw()
            == sims_render::software::render(warm_device.scene()).into_raw(),
        "cold camera service published different pixels"
    );
    Ok(())
}

fn drain_camera_cold(
    viewer: &mut Viewer,
    label: &str,
    tick_rng_before: (u64, [Option<u32>; 3]),
    restored_boundaries: &mut Vec<Value>,
) -> Result<()> {
    let mut boundaries = std::collections::BTreeSet::new();
    for slice in 0..2048 {
        let session = viewer.session.as_mut().unwrap();
        if !session.camera_command_pending() {
            return Ok(());
        }
        let presenter = session.host.presentation.presenter.as_ref().unwrap();
        let viewport = presenter.camera.as_ref().unwrap().settings.viewport;
        let input = sims_sim::pose::portrait::view::Input {
            viewport: [0, 0, viewport[2] as u32, viewport[3] as u32],
            center: presenter.view.origin,
            zoom: presenter.view.zoom,
            rotation: presenter.view.display_rotation,
        };
        session.advance_camera_command(input, 1)?;
        let session = viewer.session.as_ref().unwrap();
        if !session.camera_command_pending() {
            viewer
                .device
                .as_mut()
                .unwrap()
                .advance(viewer.session.as_mut().unwrap(), 1)?;
            return Ok(());
        }
        let command = session.host.presentation.camera_command.as_ref().unwrap();
        let boundary = format!(
            "{:?}:{:?}",
            serde_json::to_value(command)?["portrait"],
            command.pending_camera()
        );
        let view = &session.host.presentation.presenter.as_ref().unwrap().view;
        let pending = view.pending;
        let origin = view.origin;
        if boundaries.insert(boundary.clone()) {
            advance_cold(viewer)
                .with_context(|| format!("cold camera {label} at {slice}: {boundary}"))?;
            restored_boundaries.push(json!({
                "root":label,"slice":slice,"boundary":boundary,"pending":pending,"origin":origin,
            }));
        } else {
            viewer
                .device
                .as_mut()
                .unwrap()
                .advance(viewer.session.as_mut().unwrap(), 1)?;
        }
        ensure!(
            tick_rng(viewer)? == tick_rng_before,
            "camera services advanced native tick/RNG"
        );
    }
    anyhow::bail!("camera continuation budget exceeded")
}

fn settle(viewer: &mut Viewer) -> Result<()> {
    for _ in 0..10000 {
        ensure!(
            viewer.error.is_none(),
            "portrait input failed: {:?}",
            viewer.error
        );
        let session = viewer.session.as_ref().unwrap();
        ensure!(
            session.simulation.vm.world.dialogs.modal.is_none(),
            "unexpected portrait test modal"
        );
        ensure!(
            !session.clock_pending(),
            "portrait test cannot drain household clock work"
        );
        if !viewer.presentation_pending() {
            return viewer.refresh();
        }
        viewer.step()?;
    }
    anyhow::bail!("portrait test presentation budget exceeded")
}

#[test]
#[ignore = "private frozen tutorial centering checkpoint; no native window"]
fn portrait_repeat_centers_real_live_identity_with_cold_camera_continuations() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("portrait centering worker panicked")
}

fn load_viewer() -> Result<(
    Viewer,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
)> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = repo
        .join(std::env::var("SIMS_PORTRAIT_CENTER_INPUT")?)
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_PORTRAIT_CENTER_OUTPUT")?);
    ensure!(input.starts_with(repo.join("cache")) && output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let save = output.join("player-save.json");
    let checkpoint: Value = serde_json::from_slice(&std::fs::read(&input)?)?;
    ensure!(
        checkpoint["schema"] == 1,
        "unknown portrait checkpoint schema"
    );
    let mut load_value = json!({
        "session":checkpoint["session"],
        "dialog_client":checkpoint["dialog_client"],
        "camera_device":checkpoint["camera_device"],
    });
    for key in [
        "floor",
        "zoom",
        "resident",
        "selected",
        "tutorial_highlight",
        "viewport_enter_requested",
        "move_inputs",
    ] {
        load_value[key] = checkpoint["application"]
            .get(key)
            .with_context(|| format!("portrait checkpoint application field absent: {key}"))?
            .clone();
    }
    let load = output.join("restorable-player-save.json");
    std::fs::write(&load, serde_json::to_vec(&load_value)?)?;
    let root = repo.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        load.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    settle(&mut viewer)?;
    Ok((viewer, input, load, output))
}

fn run() -> Result<()> {
    let (mut viewer, input, load, output) = load_viewer()?;
    let initial_tick_rng = tick_rng(&viewer)?;
    let session = viewer.session.as_ref().unwrap();
    let identity = session
        .simulation
        .native_selected_person()
        .map_err(anyhow::Error::msg)?
        .context("portrait test has no actual selected person")?;
    let resident =
        live_resident(session, identity).context("portrait test selected person is not live")?;
    let guid = resident.guid;
    let handle = session
        .simulation
        .vm
        .world
        .scheduler_storage
        .as_ref()
        .and_then(|storage| storage.handle(identity))
        .context("portrait test lifetime absent")?;
    let target = super::target(&session.simulation.vm.world, identity)?
        .context("portrait test selected person cannot be centered")?;
    let camera = session.host.presentation.presenter.as_ref().unwrap();
    let initial_attempts = camera.camera.as_ref().unwrap().settings.attempts;
    let initial_origin = camera.view.origin;
    let _ = viewer.update(Message::Resident(identity));
    ensure!(
        viewer.error.is_none(),
        "actual portrait failed: {:?}",
        viewer.error
    );
    ensure!(viewer.session.as_ref().unwrap().camera_command_pending());
    let mut restored_boundaries = Vec::new();
    drain_camera_cold(
        &mut viewer,
        "portrait",
        initial_tick_rng,
        &mut restored_boundaries,
    )?;
    ensure!(!viewer.session.as_ref().unwrap().camera_command_pending());
    ensure!(
        restored_boundaries.len() >= 2,
        "portrait test missed center/roof continuations"
    );
    viewer
        .device
        .as_mut()
        .unwrap()
        .advance_mode_clock(Duration::from_millis(50))?;
    viewer.device.as_ref().unwrap().begin(
        viewer.session.as_mut().unwrap(),
        sims_runtime::original_session::camera_command::Change::MotionFrame,
    )?;
    drain_camera_cold(
        &mut viewer,
        "motion_frame",
        initial_tick_rng,
        &mut restored_boundaries,
    )?;
    let epoch = viewer.last.unwrap_or_else(Instant::now);
    let mut frames = 0;
    for frame in 0..32 {
        let _ = viewer.update(Message::Frame(epoch + Duration::from_millis(frame * 50)));
        settle(&mut viewer)?;
        ensure!(
            tick_rng(&viewer)? == initial_tick_rng,
            "camera animation advanced native tick/RNG"
        );
        frames += 1;
        if !viewer.session.as_ref().unwrap().camera_motion_active() {
            break;
        }
    }
    std::fs::write(
        output.join("camera-animation-final.json"),
        serde_json::to_vec_pretty(&json!({
            "components":state(&viewer)?,
            "frames":frames,
            "presentation_pending":viewer.presentation_pending(),
            "camera_motion_active":viewer.session.as_ref().unwrap().camera_motion_active(),
        }))?,
    )?;
    let session = viewer.session.as_ref().unwrap();
    let presenter = session.host.presentation.presenter.as_ref().unwrap();
    let camera = presenter.camera.as_ref().unwrap();
    ensure!(
        camera.settings.attempts.wrapping_sub(initial_attempts) > 0,
        "repeated portrait never called native scrolling"
    );
    ensure!(
        presenter.view.origin != initial_origin,
        "repeated portrait did not move the actual camera"
    );
    ensure!(
        camera.settings.motion.active == 0 && presenter.view.pending == [0, 0],
        "portrait retained an active curve or unconsumed displacement"
    );
    ensure!(
        !session.camera_motion_active(),
        "portrait camera animation did not finish"
    );
    ensure!(
        session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            == Some(identity)
    );
    ensure!(
        session
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .unwrap()
            .handle(identity)
            == Some(handle)
    );
    ensure!(live_resident(session, identity).is_some_and(|resident| resident.guid == guid));
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "input":input,"derived_player_save":load,"identity":identity,"handle":handle,"guid":guid,
            "point":target.point,"level":target.level,"tick_rng":initial_tick_rng,
            "initial_origin":initial_origin,"final_origin":presenter.view.origin,
            "initial_attempts":initial_attempts,"final_attempts":camera.settings.attempts,
            "retained_motion":camera.settings.motion,"retained_moving":camera.settings.moving,
            "retained_camera_mode":camera.settings.camera_mode,"retained_locked":camera.settings.locked,
            "frames":frames,"restored_boundaries":restored_boundaries,
            "native_ticks_advanced":0,"tutorial_complete":false,"qualification_complete":false,
        }))?,
    )?;
    Ok(())
}

#[test]
#[ignore = "private installed scene with an explicit deferred-camera test fixture; no activity credit"]
fn deferred_camera_fixture_preserves_cold_displacement_flush_pixels() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run_deferred_fixture)?
        .join()
        .expect("deferred camera fixture worker panicked")
}

fn run_deferred_fixture() -> Result<()> {
    use sims_runtime::original_session::camera_command::Change;
    let (mut viewer, input, load, output) = load_viewer()?;
    let initial_tick_rng = tick_rng(&viewer)?;
    let session = viewer.session.as_mut().unwrap();
    ensure!(
        !session.camera_motion_active(),
        "fixture camera already moving"
    );
    let presenter = session.host.presentation.presenter.as_mut().unwrap();
    let initial_origin = presenter.view.origin;
    let settings = &mut presenter.camera.as_mut().unwrap().settings;
    let original_deferred = settings.deferred;
    let initial_attempts = settings.attempts;
    settings.deferred = 1;
    let identity = session
        .simulation
        .native_selected_person()
        .map_err(anyhow::Error::msg)?
        .context("deferred fixture selected person absent")?;
    let target = super::target(&session.simulation.vm.world, identity)?
        .context("deferred fixture person cannot be centered")?;
    viewer.device.as_ref().unwrap().begin(
        session,
        Change::CenterPerson {
            point: target.point,
            level: target.level,
        },
    )?;
    let mut restored_boundaries = Vec::new();
    drain_camera_cold(
        &mut viewer,
        "deferred_portrait_fixture",
        initial_tick_rng,
        &mut restored_boundaries,
    )?;
    viewer
        .device
        .as_mut()
        .unwrap()
        .advance_mode_clock(Duration::from_millis(50))?;
    viewer
        .device
        .as_ref()
        .unwrap()
        .begin(viewer.session.as_mut().unwrap(), Change::MotionFrame)?;
    drain_camera_cold(
        &mut viewer,
        "displacement_fixture",
        initial_tick_rng,
        &mut restored_boundaries,
    )?;
    std::fs::write(
        output.join("displacement-boundaries.json"),
        serde_json::to_vec_pretty(&json!({
            "components":state(&viewer)?,"restored_boundaries":restored_boundaries,
        }))?,
    )?;
    let flush = restored_boundaries
        .iter()
        .find(|boundary| {
            boundary["root"] == "displacement_fixture"
                && boundary["boundary"]
                    .as_str()
                    .is_some_and(|value| value.contains("FlushSurface"))
        })
        .context("deferred fixture missed the displacement flush boundary")?;
    let pending: [i32; 2] = serde_json::from_value(flush["pending"].clone())?;
    ensure!(
        pending != [0, 0],
        "fixture motion did not defer displacement"
    );
    let session = viewer.session.as_ref().unwrap();
    let presenter = session.host.presentation.presenter.as_ref().unwrap();
    let settings = &presenter.camera.as_ref().unwrap().settings;
    ensure!(presenter.view.pending == [0, 0]);
    ensure!(settings.attempts.wrapping_sub(initial_attempts) == 1);
    ensure!(
        presenter.view.origin
            == std::array::from_fn(|axis| initial_origin[axis].wrapping_add(pending[axis]))
    );
    ensure!(tick_rng(&viewer)? == initial_tick_rng);
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "input":input,"derived_player_save":load,"explicit_camera_fixture":true,
            "fixture_change":{"original_deferred":original_deferred,"deferred":1},
            "portrait_point":target.point,"portrait_level":target.level,
            "initial_origin":initial_origin,"actual_pending":pending,
            "final_origin":presenter.view.origin,"restored_boundaries":restored_boundaries,
            "native_ticks_advanced":0,"activity_acceptance_credit":false,
            "tutorial_complete":false,"qualification_complete":false,
        }))?,
    )?;
    Ok(())
}
