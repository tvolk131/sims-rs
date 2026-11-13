use super::*;
use sims_render::camera_device::panels::options::Sources;

fn options() -> View {
    View { revision: 7, section: Section::Play,
        sources: Sources { floor: 1, zoom: 2, rotation: 0, free_will: Some(true) },
        mode: 3, paused: 0, window_paused: 1, pause_overlay: false, speed: 760 }
}

#[test]
fn tutorial_panel_options_rejects_hidden_modal_stale_and_wrong_mode_inputs() {
    let mut options = options();
    assert!(available(Some(&options), options.revision, true, false));
    assert!(!available(Some(&options), 6, true, false));
    assert!(!available(Some(&options), 7, false, false));
    assert!(!available(Some(&options), 7, true, true));
    assert!(!available(None, 7, true, false));
    options.mode = 2;
    assert!(!available(Some(&options), 7, true, false));
    assert_eq!(control_resource(&Message::GameOptions), Some(2009));
    assert_eq!(control_resource(&Message::Options), None);
    assert_eq!(control_resource(&Message::GameOptionsSave(7)), Some(4950));
    assert_eq!(control_resource(&Message::Save), None);
}

#[test]
fn tutorial_panel_options_actual_sections_save_and_separate_application_menu() -> Result<()> {
    let options = options();
    let mut input = headless_input::Input::new()?;
    let display = [480, 240];
    for section in Section::ALL {
        let observation = input.element_observation(view(&options, None, true), display)?;
        let bounds = observation.texts.iter().find(|entry| entry.text == section.label())
            .context("Options section control absent")?.bounds;
        let events = click(bounds.center());
        let batch = input.element_batch(view(&options, None, true), display, &events);
        ensure!(batch.messages.len() == 1 && matches!(&batch.messages[0],
            Message::GameOptionsSection(7, selected) if *selected == section));
        ensure!(control_resource(&batch.messages[0]) == Some(section.resource()));
        let batch = input.element_batch(view(&options, None, false), display, &events);
        ensure!(batch.messages.is_empty());
    }
    for (label, native) in [("Save household", true), ("Application save/load menu…", false)] {
        let observation = input.element_observation(view(&options, None, true), display)?;
        let bounds = observation.texts.iter().find(|entry| entry.text == label)
            .context("Options menu control absent")?.bounds;
        let batch = input.element_batch(view(&options, None, true), display, &click(bounds.center()));
        ensure!(batch.messages.len() == 1);
        ensure!(if native { matches!(batch.messages[0], Message::GameOptionsSave(7)) }
            else { matches!(batch.messages[0], Message::Options) });
    }
    let observation = input.element_observation(view(&options, None, true), display)?;
    ensure!(!observation.texts.iter().any(|entry| entry.text.contains("Reset")));
    Ok(())
}

fn click(position: iced::Point) -> [iced::Event; 3] {
    [iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }),
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))]
}

#[test]
#[ignore = "private original household assets; no native window"]
fn tutorial_panel_native_options_mode_cursor_boundary_restore_and_separate_menu() -> Result<()> {
    use clap::Parser;
    use sims_runtime::original_session::publication::Progress;
    use sims_vm::interface::world_publication::Request;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let directory = std::env::temp_dir().join(format!("sims-options-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
    std::fs::create_dir(&directory)?;
    let save = directory.join("options.json");
    let args = Args::parse_from(["sims-room", "--original", "UserData/Houses/House05.iff",
        "--paused", "--buy-preview", "--root", root.to_str().unwrap(), "--save", save.to_str().unwrap()]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut viewer, _) = Viewer::new(args);
    viewer.install(ready)?;
    let mut inputs = Vec::new();
    transport_tests::drain(&mut viewer, &mut inputs)?;
    let session = viewer.session.as_mut().unwrap();
    let before = serde_json::to_vec(&session.save())?;
    ensure!(session.complete_publication_window_reset(0).is_err());
    ensure!(session.complete_publication_window_reset(1).is_err());
    ensure!(serde_json::to_vec(&session.save())? == before);
    let mode = session.host.presentation.transport.as_ref().context("Options test has no native transport")?.controls.mode;
    eprintln!("Options startup: mode={mode}, panel_state={:?}, people_visible={}",
        viewer.device.as_ref().unwrap().panel_state(), viewer.people_panel_visible());
    if mode != 2 || !viewer.people_panel_visible() {
        let _ = viewer.update(Message::Live);
        transport_tests::drain(&mut viewer, &mut inputs)?;
    }
    if viewer.device.as_ref().unwrap().panel_state() == Some(1) {
        let _ = viewer.update(Message::ExpandPanel);
        transport_tests::drain(&mut viewer, &mut inputs)?;
    }
    ensure!(viewer.error.is_none(), "Live setup failed: {:?}", viewer.error);
    ensure!(!viewer.presentation_pending() && viewer.people_panel_visible(),
        "Live setup did not expose the actual People panel");
    ensure!(viewer.device.as_ref().unwrap().panel_state() == Some(0));
    let session = viewer.session.as_ref().unwrap();
    ensure!(session.host.presentation.transport.as_ref().unwrap().controls.mode == 2);
    ensure!(!session.player_input_pending() && session.simulation.vm.world.dialogs.modal.is_none());
    let initial = sims_render::dialog::client::live_panel::selected_identity(session)?
        .context("Options test has no selected live person")?;
    let panel = viewer.client.as_ref().unwrap().live_panel().context("Options test has no live panel owner")?;
    ensure!(panel.shown() && panel.person().map(|person| person.identity()) == Some(initial));
    let _ = viewer.update(Message::Personality(initial));
    transport_tests::drain(&mut viewer, &mut inputs)?;
    let detail = sims_render::dialog::client::live_panel::Detail::Personality(0);
    let client = viewer.client.as_ref().unwrap();
    let panel = client.live_panel().unwrap();
    ensure!(viewer.people_panel_visible()
        && panel.category_selected(sims_render::dialog::client::live_panel::Category::Personality)
        && panel.person().map(|person| person.identity()) == Some(initial) && panel.detail_available(detail),
        "Personality activation did not expose the selected person's actual detail control");
    ensure!(client.buttons().and_then(|buttons| buttons.focused_resource()) == Some(4501));
    let _ = viewer.update(Message::PersonDetails(initial, detail));
    transport_tests::drain(&mut viewer, &mut inputs)?;
    ensure!(viewer.error.is_none(), "personality input failed: {:?}", viewer.error);
    let panel = viewer.client.as_ref().unwrap().live_panel().unwrap();
    ensure!(viewer.people_panel_visible() && panel.person().map(|person| person.identity()) == Some(initial));
    ensure!(panel.retained_popup() == Some(sims_render::dialog::client::live_panel::Popup::Detail(detail)));
    ensure!(person_panel::popup(panel).is_some());
    let _ = viewer.update(Message::GameOptions);
    ensure!(!viewer.options_open && viewer.error.is_none());
    for iteration in 0..10000 {
        if viewer.session.as_ref().unwrap().publication_pending() { break; }
        viewer.step_with_clock(false)?;
        ensure!(iteration < 9999, "Options mode did not publish");
    }
    let mut reset_pending = false;
    for _ in 0..10000 {
        let session = viewer.session.as_mut().unwrap();
        match session.advance_interface_publication(1)? {
            Progress::World(Request::WindowReset) => { reset_pending = true; break; }
            Progress::Complete => anyhow::bail!("Options completed without its cursor reset"),
            Progress::Pending => (),
            _ => {
                let device = viewer.device.as_mut().unwrap();
                if matches!(device.advance_publication(session, 1)?, Progress::Panels(_)) {
                    device.publish_replacement_people(session, viewer.client.as_mut().unwrap())?;
                }
            }
        }
    }
    ensure!(reset_pending, "Options cursor boundary not reached");
    let session = viewer.session.as_mut().unwrap();
    let before = serde_json::to_vec(&session.save())?;
    ensure!(session.complete_publication_window_reset(1).is_err());
    ensure!(serde_json::to_vec(&session.save())? == before);
    ensure!(!session.clock_pending() && !session.simulation.selection_work_pending());
    let selected = sims_render::dialog::client::live_panel::selected_identity(session)?;
    eprintln!("Options WindowReset: initial={initial:?}, selected={selected:?}, retained_before={:?}, visible_panel={:?}",
        viewer.client.as_ref().unwrap().live_panel().unwrap().person().map(|person| person.identity()),
        viewer.device.as_ref().unwrap().panel_widgets().unwrap().presentation().unwrap().active());
    viewer.refresh()?;
    let client = viewer.client.as_ref().unwrap();
    client.validate(viewer.session.as_ref().unwrap())?;
    let panel = client.live_panel().unwrap();
    ensure!(panel.person().map(|person| person.identity()) == selected,
        "pending Options publication retained {:?}, actual selected={selected:?}",
        panel.person().map(|person| person.identity()));
    ensure!(panel.category() == sims_render::dialog::client::live_panel::Category::Personality);
    if selected != Some(initial) {
        ensure!(panel.retained_popup().is_none(), "selection change retained the previous person's popup");
    } else {
        ensure!(panel.retained_popup() == Some(sims_render::dialog::client::live_panel::Popup::Detail(detail)));
    }
    let retained = serde_json::to_value(panel)?;
    viewer.save()?;
    viewer.load(&save)?;
    ensure!(serde_json::to_value(viewer.client.as_ref().unwrap().live_panel().unwrap())? == retained);
    ensure!(viewer.session.as_ref().unwrap().host.presentation.publication.as_ref()
        .and_then(|publication| publication.request()) == Some(Request::WindowReset));
    viewer.device.as_mut().unwrap().advance_publication(viewer.session.as_mut().unwrap(), 1)?;
    ensure!(viewer.device.as_ref().unwrap().tool_cursor() == Some(0));
    transport_tests::drain(&mut viewer, &mut inputs)?;
    let session = viewer.session.as_ref().unwrap();
    ensure!(session.simulation.vm.world.globals.get(&19) == Some(&3));
    ensure!(session.host.presentation.transport.as_ref().unwrap().controls.mode == 3);
    ensure!(session.host.presentation.tool_selection.as_ref().unwrap().mode_context().unwrap().window_pause() == Some([1, 0]));
    let device = viewer.device.as_ref().unwrap();
    ensure!(device.mode_window().unwrap().fields().paused == 1);
    ensure!(device.mode_window().unwrap().fields().timer_active == 0);
    ensure!(device.mode_window().unwrap().indicator_visible());
    let widgets = device.panel_widgets().unwrap();
    ensure!(widgets.presentation().unwrap().visible(sims_render::camera_device::panels::Panel::Options));
    ensure!(widgets.button(sims_vm::interface::view_control::Widget::Options).selected);
    ensure!(viewer.client.as_ref().unwrap().live_panel().unwrap().person().map(|person| person.identity())
        == sims_render::dialog::client::live_panel::selected_identity(session)?);
    let revision = widgets.options().unwrap().view().unwrap().revision;
    let _ = viewer.update(Message::GameOptionsSection(revision, Section::Graphics));
    ensure!(viewer.device.as_ref().unwrap().panel_widgets().unwrap().options().unwrap().view().unwrap().section == Section::Graphics);
    let _ = viewer.update(Message::Options);
    ensure!(viewer.options_open && viewer.session.as_ref().unwrap().simulation.vm.world.globals.get(&19) == Some(&3));
    let _ = viewer.update(Message::Options);
    let _ = viewer.update(Message::Live);
    transport_tests::drain(&mut viewer, &mut inputs)?;
    ensure!(viewer.session.as_ref().unwrap().simulation.vm.world.globals.get(&19) == Some(&0));
    std::fs::remove_file(save)?;
    std::fs::remove_dir(directory)?;
    Ok(())
}
