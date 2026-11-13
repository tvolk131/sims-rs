use super::*;
use sims_sim::clock::LotClock;

fn formatted(calendar: &LotClock, language: u32) -> String {
    sims_vm::dialog::format_time(
        language,
        i32::from(calendar.hour),
        calendar.minute as u32,
        true,
        true,
    )
}

#[test]
fn gui_clock_calendar_minute_noon_and_midnight_formatting() {
    for (hour, minute, expected) in [
        (6, 30, "AM  6:31"),
        (11, 59, "PM 12:00"),
        (23, 59, "AM 12:00"),
    ] {
        let mut calendar = LotClock {
            hour,
            day: 1,
            period: LotClock::period_for_hour(hour),
            minute,
            second: 58,
            month: 1,
            year: 1997,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        };
        let initial = formatted(&calendar, 1);
        let before = calendar.clone();
        calendar.finish_tick(false);
        assert_eq!(calendar, before);
        assert_eq!(formatted(&calendar, 1), initial);
        calendar.finish_tick(true);
        assert_eq!(formatted(&calendar, 1), expected);
        assert_eq!(
            formatted(&calendar, 3),
            format!("{:2}:{:02}", calendar.hour, calendar.minute)
        );
        assert_eq!(calendar.elapsed_days, i16::from(hour == 23));
    }
}

#[test]
#[ignore = "installed Goth household with native Interface and original calendar updates"]
fn gui_clock_publication_uses_advancing_engine_calendar_not_retained_title() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(calendar_publication)?
        .join()
        .expect("clock publication test panicked")
}

fn calendar_publication() -> Result<()> {
    let mut viewer = paused_viewer(true)?;
    let mut startup_inputs = vec![];
    super::transport_tests::drain(&mut viewer, &mut startup_inputs)?;
    let initial = household_clock_label(viewer.session.as_ref().unwrap())?;
    let first_tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let _ = viewer.update(Message::NativeSpeed(sims_vm::interface::speed::NORMAL));
    super::transport_tests::drain(&mut viewer, &mut startup_inputs)?;
    ensure!(viewer.error.is_none(), "{:?}", viewer.error);
    ensure!(viewer.playing);
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals[&17] == 0);
    let mut now = viewer.last.unwrap_or_else(Instant::now);
    for _ in 0..240 {
        let responses = startup_inputs.len();
        now += std::time::Duration::from_millis(42);
        let _ = viewer.update(Message::Frame(now));
        super::transport_tests::drain(&mut viewer, &mut startup_inputs)?;
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        if startup_inputs.len() != responses {
            let _ = viewer.update(Message::NativeSpeed(sims_vm::interface::speed::NORMAL));
            super::transport_tests::drain(&mut viewer, &mut startup_inputs)?;
        }
        let current = household_clock_label(viewer.session.as_ref().unwrap())?;
        ensure!(
            viewer
                .hud_cluster
                .as_ref()
                .context("clock scheduled HUD publication absent")?
                .values
                .clock
                == current
        );
        if current != initial {
            break;
        }
    }
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.tick > first_tick);
    ensure!(
        household_clock_label(viewer.session.as_ref().unwrap())? != initial,
        "calendar label never progressed past {initial} after 240 real frames"
    );
    verify_publication(&mut viewer)?;
    Ok(())
}

#[test]
#[ignore = "installed Goth household clock boundary publication fixtures; no progression credit"]
fn gui_clock_boundary_publication_formats_engine_calendar() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(boundary_publication)?
        .join()
        .expect("clock boundary publication test panicked")
}

fn boundary_publication() -> Result<()> {
    let mut viewer = paused_viewer(true)?;
    for (hour, minute, expected) in [(11, 59, "PM 12:00"), (23, 59, "AM 12:00")] {
        let session = viewer.session.as_mut().unwrap();
        let mut calendar = session.simulation.household.as_ref().unwrap().clock.clone();
        calendar.hour = hour;
        calendar.minute = minute;
        calendar.second = 58;
        calendar.period = LotClock::period_for_hour(hour);
        calendar.finish_tick(true);
        seed_calendar(session, calendar);
        ensure!(household_clock_label(session)? == expected);
        viewer.publish_hud_cluster();
        verify_publication(&mut viewer)?;
    }
    Ok(())
}

#[test]
#[ignore = "installed Goth household for native wall camera ownership rejection"]
fn gui_wall_camera_rejects_missing_conflicting_and_malformed_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(wall_camera_owners)?
        .join()
        .expect("wall camera owner validation test panicked")
}

fn wall_camera_owners() -> Result<()> {
    use sims_runtime::original_session::camera_command::State;
    let mut viewer = paused_viewer(false)?;
    let session = viewer.session.as_mut().unwrap();
    session.begin_walls_change(4)?;
    let command = session.host.presentation.camera_command.as_ref().unwrap();
    command.validate(session)?;
    let valid = serde_json::to_value(command)?;
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("walls");
    let missing: State = serde_json::from_value(missing)?;
    ensure!(
        missing.validate(session).unwrap_err().to_string()
            == "wall camera lost its publication caller"
    );
    for (field, value) in [("mode", 0), ("mode", 3), ("next", 0), ("next", 4)] {
        let mut malformed = valid.clone();
        malformed["walls"][field] = serde_json::json!(value);
        let malformed: State = serde_json::from_value(malformed)?;
        ensure!(
            malformed.validate(session).unwrap_err().to_string()
                == "wall camera command lost its native children"
        );
    }
    let mut conflicting = valid;
    conflicting["publication"] = serde_json::json!(1);
    let conflicting: State = serde_json::from_value(conflicting)?;
    ensure!(
        conflicting.validate(session).unwrap_err().to_string()
            == "wall camera command lost its native children"
    );
    Ok(())
}

fn paused_viewer(bound: bool) -> Result<Viewer> {
    use clap::Parser;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let mut arguments = vec![
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
    ];
    if bound {
        arguments.push("--buy-preview");
    }
    let args = Args::parse_from(arguments);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let mut startup_inputs = vec![];
    super::transport_tests::drain(&mut viewer, &mut startup_inputs)?;
    Ok(viewer)
}

fn seed_calendar(session: &mut Session, calendar: LotClock) {
    for (index, value) in [
        (0, calendar.hour),
        (1, calendar.day),
        (4, calendar.period),
        (5, calendar.minute),
        (6, calendar.second),
        (7, calendar.month),
        (8, calendar.year),
        (28, calendar.elapsed_days),
        (29, calendar.month_days),
    ] {
        session.simulation.vm.world.globals.insert(index, value);
    }
    session.simulation.vm.tick = u64::from(calendar.ticks);
    session.simulation.household.as_mut().unwrap().clock = calendar;
}

fn verify_publication(viewer: &mut Viewer) -> Result<()> {
    let session = viewer.session.as_ref().unwrap();
    let before = serde_json::to_vec(&session.save())?;
    let expected = household_clock_label(session)?;
    let native = viewer
        .device
        .as_ref()
        .unwrap()
        .panel_widgets()
        .unwrap()
        .texts()
        .unwrap()
        .clock();
    let cluster = viewer.observe_hud_cluster().context("clock HUD absent")?;
    ensure!(cluster.values.clock == expected);
    ensure!(
        viewer
            .hud_cluster
            .as_ref()
            .context("clock scheduled HUD publication absent")?
            .values
            .clock
            == expected
    );
    let published = ui_publication::Published::capture(viewer);
    ensure!(published.cluster.as_ref().unwrap().values.clock == expected);
    ensure!(
        published
            .money
            .as_ref()
            .context("clock money snapshot absent")?
            .clock
            == expected
    );
    ensure!(
        viewer
            .device
            .as_ref()
            .unwrap()
            .panel_widgets()
            .unwrap()
            .texts()
            .unwrap()
            .clock()
            == native
    );
    ensure!(serde_json::to_vec(&viewer.session.as_ref().unwrap().save())? == before);
    Ok(())
}
