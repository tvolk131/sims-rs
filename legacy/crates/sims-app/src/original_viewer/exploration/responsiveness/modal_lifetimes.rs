//! Replacement-client event-lifetime invariants using a genuinely suspended
//! original Car Pool notice. These are not native widget/screenshot parity.
use super::*;
use iced::keyboard::{self, key};

fn key_event(named: key::Named, down: bool) -> iced::Event {
    let physical = key::Physical::Code(match named {
        key::Named::Enter => key::Code::Enter,
        key::Named::Escape => key::Code::Escape,
        key::Named::Space => key::Code::Space,
        key::Named::ArrowLeft => key::Code::ArrowLeft,
        key::Named::ArrowRight => key::Code::ArrowRight,
        key::Named::ArrowUp => key::Code::ArrowUp,
        key::Named::ArrowDown => key::Code::ArrowDown,
        _ => unreachable!(),
    });
    let key = keyboard::Key::Named(named);
    iced::Event::Keyboard(if down {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        }
    } else {
        keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
        }
    })
}

pub(super) fn pointer(down: bool) -> iced::Event {
    iced::Event::Mouse(if down {
        iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)
    } else {
        iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)
    })
}

#[test]
fn subscription_capture_and_key_release_precedence() {
    let window = window::Id::unique();
    for named in [
        key::Named::Enter,
        key::Named::Escape,
        key::Named::Space,
        key::Named::ArrowLeft,
        key::Named::ArrowRight,
        key::Named::ArrowUp,
        key::Named::ArrowDown,
    ] {
        assert!(
            subscription_event(
                key_event(named, true),
                iced::event::Status::Captured,
                window
            )
            .is_none()
        );
        assert!(
            subscription_event(
                key_event(named, false),
                iced::event::Status::Ignored,
                window
            )
            .is_none()
        );
        assert!(
            subscription_event(key_event(named, true), iced::event::Status::Ignored, window)
                .is_some()
        );
    }
    assert!(matches!(
        subscription_event(
            iced::Event::Window(window::Event::Unfocused),
            iced::event::Status::Captured,
            window,
        ),
        Some(Message::FocusLost)
    ));
    assert!(matches!(
        subscription_event(
            iced::Event::Window(window::Event::Resized(Size::new(800., 600.))),
            iced::event::Status::Captured,
            window,
        ),
        Some(Message::DisplaySize(_))
    ));
    assert!(matches!(
        subscription_event(
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(
                keyboard::Modifiers::SHIFT
            )),
            iced::event::Status::Captured,
            window,
        ),
        Some(Message::Shift(true))
    ));
}

pub(super) fn load_car_pool(out: &Path) -> Result<Application> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = repo
        .join("cache/gameplay-goth-contention-20261023-third/normal-release/notice-1/initial.json");
    let bytes = std::fs::read(&source)?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let save = out.join("unused-player-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Ready::load(&args)?;
    let app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("restore index absent")?,
        &ready.icons,
        Instant::now(),
        &bytes,
    )?;
    std::fs::write(out.join("input.json"), &bytes)?;
    let r = modal(&app)?;
    ensure!(
        r.id == 149
            && r.caller == 8
            && r.stack_object == 21
            && r.title == "Car Pool"
            && engine::digest(r.body.as_bytes()) == "c4f983d1377ed8931ec9c56fbe8d1c7dd975c29b8732edab106c7ab5eeaa8c9f"
            && r.source == "GameData_ALL_Recursive/Objects/Objects.far::CarPortal.iff"
            && r.origin.bhav == 4125
            && r.origin.instruction == 11
            && r.buttons.len() == 1
            && r.buttons[0].result == 2
            && r.key_result(13) == Some(2)
            && r.key_result(27) == Some(2)
            && r.key_result(32) == Some(2),
        "private Car Pool fixture changed: {r:?}"
    );
    ensure!(app.observe()?.tick == 89344 && !app.viewer.playing);
    Ok(app)
}

fn modal(app: &Application) -> Result<&sims_vm::dialog::Record> {
    let d = &app.session()?.simulation.vm.world.dialogs;
    d.records
        .iter()
        .find(|r| Some(r.id) == d.modal)
        .context("modal record absent")
}

fn dispatch(ui: &mut Input, app: &mut Application, event: iced::Event) -> Result<Vec<Message>> {
    let messages = ui.application_event(&app.viewer, event);
    for message in &messages {
        app.dispatch(message.clone())?;
    }
    Ok(messages)
}

pub(super) fn ok_point(ui: &mut Input, app: &Application) -> Result<[f32; 2]> {
    sequences::control(ui, app, "Dialog(149, 2)").map(|v| v.0)
}

#[test]
#[ignore = "private retained Goth Car Pool modal; actual Iced widgets/subscription and headless GPU"]
fn car_pool_modal_keyboard_and_pointer_lifetimes() -> Result<()> {
    run_worker(modal_lifetimes)
}

fn modal_lifetimes() -> Result<()> {
    let out = output()?;
    let base = load_car_pool(&out)?;
    let bytes = base.checkpoint()?;
    let mut ui = Input::new()?;
    let mut rows = vec![];
    let mut failures = vec![];
    for name in [
        "passive_and_shortcuts",
        "pointer_focus_loss",
        "enter",
        "escape",
        "space",
        "pointer_modal_dismissal",
        "pointer_resize",
    ] {
        eprintln!("Car Pool lifetime case: {name}");
        let mut app = base.fork()?;
        let result = run_case(name, &mut app, &base, &bytes, &mut ui, &out);
        match result {
            Ok(details) => {
                eprintln!("Car Pool lifetime case passed: {name}");
                rows.push(json!({"case":name,"passed":true,"details":details}));
            }
            Err(error) => {
                std::fs::write(out.join(format!("failure-{name}.json")), app.checkpoint()?)?;
                failures.push(name);
                eprintln!("Car Pool lifetime case failed: {name}: {error:#}");
                rows.push(json!({"case":name,"passed":false,"error":format!("{error:#}")}));
            }
        }
    }
    write_json(
        &out.join("report.json"),
        &json!({
            "cases":rows,"failures":failures,"real_retained_modal":true,
            "actual_iced_widgets_and_production_subscription":true,
            "strict_native_parity_ticks_added":0,
            "scope":"replacement-client invariants; original modal record/key protocol, not native widget appearance or focus policy parity"
        }),
    )?;
    ensure!(
        failures.is_empty(),
        "HUD lifetime failures preserved in {}: {failures:?}",
        out.display()
    );
    Ok(())
}

fn run_case(
    name: &str,
    app: &mut Application,
    base: &Application,
    bytes: &[u8],
    ui: &mut Input,
    out: &Path,
) -> Result<Value> {
    let start = app.observe()?;
    let viewport = ui.viewport_bounds(&app.viewer)?;
    match name {
        "passive_and_shortcuts" => {
            sequences::passive_frame(ui, app)?;
            for named in [
                key::Named::ArrowLeft,
                key::Named::ArrowRight,
                key::Named::ArrowUp,
                key::Named::ArrowDown,
            ] {
                dispatch(ui, app, key_event(named, true))?;
                dispatch(ui, app, key_event(named, false))?;
                ensure!(
                    app.observe()? == start,
                    "hidden shortcut {named:?} changed modal game/camera state"
                );
            }
            ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
            ui.save(&app.viewer, &out.join("car-pool-modal.png"))?;
            ensure!(app.observe()? == start);
        }
        "enter" | "escape" | "space" => {
            let named = match name {
                "enter" => key::Named::Enter,
                "escape" => key::Named::Escape,
                _ => key::Named::Space,
            };
            let mut cold = base.restore(bytes)?;
            ensure!(
                cold.observe()? == app.observe()?,
                "modal cold restore changed state/RNG"
            );
            let messages = dispatch(ui, app, key_event(named, true))?;
            let expected = match name {
                "enter" => 13,
                "escape" => 27,
                _ => 32,
            };
            ensure!(matches!(messages.as_slice(), [Message::DialogKey(k)] if *k == expected));
            dispatch(ui, &mut cold, key_event(named, true))?;
            app.drain(&Limits::default())?;
            cold.drain(&Limits::default())?;
            ensure!(
                app.session()?.simulation.vm.world.dialogs.modal != Some(149),
                "known key did not answer Car Pool"
            );
            ensure!(
                app.observe()? == cold.observe()?,
                "modal answer continuation/cold restore diverged"
            );
            let after = app.observe()?;
            let released = dispatch(ui, app, key_event(named, false))?;
            ensure!(
                released.iter().all(|message| matches!(
                    message,
                    Message::ViewportEnter
                        | Message::ViewportLeave
                        | Message::ViewportMotion { .. }
                )),
                "key-release emitted activation: {released:?}"
            );
            ensure!(
                app.observe()? == after,
                "key release answered another owner"
            );
            ensure!(
                ui.viewport_bounds(&app.viewer)? == viewport,
                "modal dismissal resized viewport"
            );
            return Ok(
                json!({"warm_cold_equal":true,"key":format!("{named:?}"),"before":start,"after":after,
                    "release_messages":released.iter().map(|message|format!("{message:?}")).collect::<Vec<_>>(),
                    "release_handlers_applied_state_neutral":true}),
            );
        }
        "pointer_focus_loss" => {
            let point = ok_point(ui, app)?;
            let _ = ui.move_to(&app.viewer, point);
            ensure!(dispatch(ui, app, pointer(true))?.is_empty());
            dispatch(
                ui,
                app,
                iced::Event::Keyboard(keyboard::Event::ModifiersChanged(
                    keyboard::Modifiers::SHIFT,
                )),
            )?;
            ensure!(app.viewer.shift);
            let lost = dispatch(ui, app, iced::Event::Window(window::Event::Unfocused))?;
            ensure!(
                lost.iter()
                    .any(|message| matches!(message, Message::FocusLost))
                    && !app.viewer.shift
                    && app.viewer.pan == [0, 0]
            );
            dispatch(ui, app, iced::Event::Window(window::Event::Focused))?;
            ensure!(
                app.observe()? == start,
                "focus reset changed retained state/RNG"
            );
            let before_release = app.observe()?;
            let released = dispatch(ui, app, pointer(false))?;
            ensure!(
                released.is_empty(),
                "focus-lost press activated on release: {released:?}"
            );
            ensure!(app.observe()? == before_release && modal(app)?.id == 149);
            ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
            // A fresh intentional click still answers the known real notice.
            ensure!(dispatch(ui, app, pointer(true))?.is_empty());
            let clicked = dispatch(ui, app, pointer(false))?;
            ensure!(matches!(clicked.as_slice(), [Message::Dialog(149, 2)]));
            app.drain(&Limits::default())?;
            ensure!(app.session()?.simulation.vm.world.dialogs.modal != Some(149));
        }
        "pointer_modal_dismissal" => {
            let point = ok_point(ui, app)?;
            let _ = ui.move_to(&app.viewer, point);
            ensure!(dispatch(ui, app, pointer(true))?.is_empty());
            dispatch(ui, app, key_event(key::Named::Enter, true))?;
            app.drain(&Limits::default())?;
            let before_release = app.observe()?;
            let released = dispatch(ui, app, pointer(false))?;
            // The newly uncovered viewport can observe the physical release.
            // Apply that real handler before proving that it did not activate
            // anything or change any retained owner/RNG, rather than relying
            // only on its message name.
            ensure!(
                released
                    .iter()
                    .all(|message| matches!(message, Message::ViewportButton { down: false, .. })),
                "ended modal press leaked activation: {released:?}"
            );
            ensure!(
                app.observe()? == before_release,
                "ended modal release changed retained state/RNG"
            );
            return Ok(
                json!({"before":start,"after":app.observe()?,"release_messages":released.iter().map(|message| format!("{message:?}")).collect::<Vec<_>>(),"release_applied_state_neutral":true}),
            );
        }
        "pointer_resize" => {
            let point = ok_point(ui, app)?;
            let _ = ui.move_to(&app.viewer, point);
            ensure!(dispatch(ui, app, pointer(true))?.is_empty());
            dispatch(
                ui,
                app,
                iced::Event::Window(window::Event::Resized(Size::new(800., 600.))),
            )?;
            let before_release = app.observe()?;
            let released = dispatch(ui, app, pointer(false))?;
            ensure!(
                released.is_empty(),
                "resized modal press leaked activation: {released:?}"
            );
            ensure!(app.observe()? == before_release && modal(app)?.id == 149);
            ensure!(app.observe()?.tick == start.tick && app.observe()?.rng == start.rng);
        }
        _ => unreachable!(),
    }
    Ok(json!({"before":start,"after":app.observe()?,"viewport":viewport}))
}

#[test]
#[ignore = "private Mashuga checkpoint; actual scrolled Buy catalogue and Iced focus events"]
fn buy_scroll_and_hud_press_survive_focus_loss() -> Result<()> {
    run_worker(buy_focus)
}

fn buy_focus() -> Result<()> {
    let out = output()?;
    let mut app = load(&out)?;
    let mut ui = Input::new()?;
    select(&mut app, 196)?;
    sequences::activate(&mut ui, &mut app, "Buy")?;
    let start = app.observe()?;
    let viewport = ui.viewport_bounds(&app.viewer)?;
    let original = ui.widget_owners(&app.viewer);
    let scroll = original
        .scrolls
        .iter()
        .find(|s| s.content[3] > s.bounds[3] + 1.)
        .context("actual Buy catalogue has no vertically overflowing scroll owner")?;
    let point = [
        scroll.bounds[0] + scroll.bounds[2] * 0.75,
        scroll.bounds[1] + scroll.bounds[3] * 0.5,
    ];
    let _ = ui.move_to(&app.viewer, point);
    dispatch(
        &mut ui,
        &mut app,
        iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
            delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -3. },
        }),
    )?;
    let scrolled = ui.widget_owners(&app.viewer);
    ensure!(
        scrolled != original && scrolled.scrolls.iter().any(|s| s.translation[1] > 0.),
        "actual catalogue wheel event did not scroll: {scrolled:?}"
    );
    ensure!(
        app.observe()? == start,
        "catalogue scrolling changed retained state/RNG"
    );
    // Use a real HUD button outside the scrolled content. Its stale release
    // must be cancelled while the catalogue's independent offsets survive.
    let (point, _) = sequences::control(&mut ui, &app, "Options")?;
    let _ = ui.move_to(&app.viewer, point);
    ensure!(dispatch(&mut ui, &mut app, pointer(true))?.is_empty());
    dispatch(
        &mut ui,
        &mut app,
        iced::Event::Window(window::Event::Unfocused),
    )?;
    dispatch(
        &mut ui,
        &mut app,
        iced::Event::Window(window::Event::Focused),
    )?;
    let after_focus = ui.widget_owners(&app.viewer);
    write_json(
        &out.join("focus-owners.json"),
        &json!({"original":original,"scrolled":scrolled,"after_focus":after_focus}),
    )?;
    ensure!(
        after_focus == scrolled,
        "focus loss reset a non-button scroll/focus owner"
    );
    ensure!(
        dispatch(&mut ui, &mut app, pointer(false))?.is_empty(),
        "focus-lost HUD press activated on release"
    );
    ensure!(!app.viewer.options_open && app.observe()? == start);
    ensure!(ui.viewport_bounds(&app.viewer)? == viewport);
    std::fs::write(out.join("focus-safe-boundary.json"), app.checkpoint()?)?;
    ui.save(&app.viewer, &out.join("scrolled-buy-after-focus.png"))?;
    let before_click = app.capture(false)?;
    ensure!(dispatch(&mut ui, &mut app, pointer(true))?.is_empty());
    let click = dispatch(&mut ui, &mut app, pointer(false))?;
    ensure!(
        matches!(click.as_slice(), [Message::Options]) && app.viewer.options_open,
        "fresh deliberate HUD click failed: {click:?}"
    );
    // Opening another application panel can change that panel's own widget
    // geometry. Verify actual retained owners rather than requiring identical
    // Iced owner lists across two different page structures.
    let mut after_click = app.capture(false)?;
    after_click["application"]["options_open"] =
        before_click["application"]["options_open"].clone();
    ensure!(
        after_click == before_click,
        "fresh Options click changed retained owners beyond options_open"
    );
    sequences::passive_frame(&mut ui, &app)?;
    write_json(
        &out.join("report.json"),
        &json!({"before_scroll":original,"scrolled":scrolled,
            "after_focus":after_focus,"viewport":viewport,"tick":start.tick,
        "rng":start.rng,"held_hud_release_cancelled":true,"fresh_hud_click_works":true,
            "fresh_options_changes_only_options_open":true,
        "native_parity_ticks_added":0,"scope":"replacement-client focus and scroll invariants"}),
    )?;
    Ok(())
}
