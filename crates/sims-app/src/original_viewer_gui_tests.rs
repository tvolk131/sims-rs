use super::*;
use iced::advanced::{layout, renderer::Headless, widget::Tree};
use iced::{Point, Rectangle, mouse};
use sims_vm::dialog::{Button, Presentation, Record};

fn record(title: &str, source: &str, labels: &[&str]) -> Record {
    Record {
        presentation: Some(Presentation::default()),
        id: 47,
        caller: 1,
        origin: sims_vm::Location {
            source: source.to_owned(),
            bhav: 4119,
            instruction: 0,
        },
        source: source.to_owned(),
        operands: [0; 8],
        stack_object: 1,
        title: title.to_owned(),
        body: "A message from the household. Choose an offered response.".into(),
        buttons: labels
            .iter()
            .enumerate()
            .map(|(index, label)| Button {
                label: (*label).to_owned(),
                result: index as u32 + 2,
            })
            .collect(),
        icon_object: None,
        job_icon: None,
        bitmap_icon: None,
        global_bitmap_icon: None,
        guid_icon: None,
        portrait: None,
        status: 1,
    }
}

fn click_events(point: Point) -> [iced::Event; 3] {
    [
        iced::Event::Mouse(mouse::Event::CursorMoved { position: point }),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]
}

fn dialog_page<'a>(dialog: &'a Record, artwork: &'a dialog_view::HudArt) -> Element<'a, Message> {
    widget::stack![
        button("Behind dialog")
            .width(Fill)
            .height(Fill)
            .on_press(Message::Step),
        dialog_view::overlay(dialog, artwork, Message::Dialog),
    ]
    .into()
}

#[test]
fn gui_dialog_buttons_keep_labels_results_and_hit_regions() -> Result<()> {
    let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .context("offscreen adapter absent")?;
    let artwork = dialog_view::HudArt::synthetic_for_test(
        &[],
        &[
            ("HelpSystem.iff", 5068, 133, 120),
            ("Tutorial.iff", 5000, 133, 120),
        ],
    );
    let bounds = Rectangle::with_size(Size::new(800., 600.));
    let mut startup = record("Help System", "HelpSystem.iff", &["Yes", "No"]);
    startup.bitmap_icon = Some(5068);
    let mut work = record("Car Pool", "CarPortal.iff", &["OK"]);
    work.job_icon = Some(3);
    let mut school = record("School Bus", "CarPortal.iff", &["OK"]);
    school.job_icon = Some(-1);
    let mut tutorial = record("Hidden tutorial title", "Tutorial.iff", &["Go Here"]);
    tutorial.bitmap_icon = Some(5000);
    tutorial.presentation = Some(Presentation {
        hide_title: true,
        anchored: true,
        large: false,
    });
    let mut nonmodal = record("Notice", "Notice.iff", &["Dismiss"]);
    nonmodal.operands[7] = 128;
    let mut choice = record("Choose", "Choice.iff", &["First", "Second", "Third"]);
    choice.operands[7] = 128;
    for dialog in [startup, work, school, tutorial, nonmodal, choice] {
        let mut element = dialog_view::view(&dialog, &artwork, Message::Dialog);
        let mut tree = Tree::new(element.as_widget());
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, bounds.size()),
        );
        ensure!(node.size().width <= bounds.width && node.size().height <= bounds.height);
        ensure!(node.children().len() == dialog.buttons.len());
        let mut previous: Option<Rectangle> = None;
        for (button_node, offered) in node.children().iter().zip(&dialog.buttons) {
            let button_bounds = button_node.bounds();
            ensure!(Rectangle::with_size(node.size()).contains(button_bounds.center()));
            if let Some(previous) = previous {
                ensure!(previous.x + previous.width < button_bounds.x);
            }
            previous = Some(button_bounds);
            let mut messages = vec![];
            for event in click_events(button_bounds.center()) {
                element.as_widget_mut().update(
                    &mut tree,
                    &event,
                    iced::advanced::Layout::new(&node),
                    mouse::Cursor::Available(button_bounds.center()),
                    &renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut iced::advanced::Shell::new(&mut messages),
                    &bounds,
                );
            }
            ensure!(
                matches!(messages.as_slice(), [Message::Dialog(id, result)]
                if *id == dialog.id && *result == offered.result),
                "{messages:?}"
            );
            if dialog.modal() || dialog.buttons.len() > 1 {
                ensure!(button_bounds.width >= 100. && button_bounds.height == 33.);
            } else {
                ensure!(button_bounds.size() == Size::new(12., 11.));
            }
        }
    }
    Ok(())
}

#[test]
fn gui_dialog_overlay_preserves_modal_capture_and_tutorial_anchor() -> Result<()> {
    let artwork = dialog_view::HudArt::synthetic_for_test(&[], &[("Tutorial.iff", 5000, 133, 120)]);
    let mut input = headless_input::Input::new()?;
    let mut dialog = record("Notice", "Notice.iff", &["OK"]);
    let outside = click_events(Point::new(4., 4.));
    ensure!(
        input
            .element_batch(dialog_page(&dialog, &artwork), [800, 600], &outside)
            .messages
            .is_empty()
    );
    dialog.operands[7] = 128;
    ensure!(matches!(
        input
            .element_batch(dialog_page(&dialog, &artwork), [800, 600], &outside)
            .messages
            .as_slice(),
        [Message::Step]
    ));
    dialog.source = "Tutorial.iff".into();
    dialog.bitmap_icon = Some(5000);
    dialog.presentation = Some(Presentation {
        hide_title: true,
        anchored: true,
        large: false,
    });
    ensure!(
        input
            .element_batch(
                dialog_page(&dialog, &artwork),
                [800, 600],
                &click_events(Point::new(796., 4.))
            )
            .messages
            .is_empty()
    );
    Ok(())
}

#[test]
#[ignore = "installed Goth household assets and offscreen renderer"]
fn gui_wall_modes_publish_render_restore_and_reject_modal_input() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(wall_modes)?
        .join()
        .expect("wall publication test panicked")
}

fn wall_modes() -> Result<()> {
    for bound in [false, true] {
        wall_profile(bound)?;
    }
    Ok(())
}

fn wall_profile(bound: bool) -> Result<()> {
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
    let mut inputs = vec![];
    super::transport_tests::drain(&mut viewer, &mut inputs)?;
    if bound {
        let session = viewer.session.as_mut().unwrap();
        ensure!(session.host.presentation.publication.is_some());
        if session.host.presentation.frontend_context.is_none() {
            ensure!(session.host.presentation.tool_selection.is_some());
        }
        reject_missing_wall_publication_owners(session)?;
    }
    let tick = viewer.session.as_ref().unwrap().simulation.vm.tick;
    let mut frames = vec![];
    for walls in [1, 4, 2, 4] {
        let _ = viewer.update(Message::Walls(walls));
        ensure!(viewer.error.is_none(), "{:?}", viewer.error);
        let session = viewer.session.as_ref().unwrap();
        ensure!(if bound {
            session.publication_pending()
        } else {
            session.camera_command_pending()
        });
        let restored = Session::restore(session.index(), session.save_managed()?)?;
        ensure!(if bound {
            restored.publication_pending()
        } else {
            restored.camera_command_pending()
        });
        ensure!(
            viewer
                .session
                .as_mut()
                .unwrap()
                .begin_walls_change(1)
                .is_err()
        );
        super::transport_tests::drain(&mut viewer, &mut inputs)?;
        let session = viewer.session.as_ref().unwrap();
        if bound {
            ensure!(
                session
                    .host
                    .presentation
                    .publication
                    .as_ref()
                    .unwrap()
                    .walls()
                    == walls as i32
            );
            if let Some(frontend) = &session.host.presentation.frontend_context {
                ensure!(frontend.walls == walls as i32);
            } else {
                ensure!(session.host.presentation.tool_selection.is_some());
            }
        }
        let floor = &session
            .host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .view
            .floor;
        ensure!(
            floor.all_walls == u8::from(walls == 2)
                && floor.visibility.enabled == u8::from(walls == 4)
        );
        ensure!(session.simulation.vm.tick == tick);
        let saved = session.save_managed()?;
        let restored = Session::restore(session.index(), saved)?;
        ensure!(
            restored
                .host
                .presentation
                .presenter
                .as_ref()
                .unwrap()
                .view
                .floor
                == *floor
        );
        let observed = viewer.observe_hud_cluster().unwrap();
        ensure!(observed.values.buttons.walls == Some(walls as i32));
        for (mode, state) in [
            (1, observed.values.buttons.walls_down),
            (4, observed.values.buttons.walls_up),
            (2, observed.values.buttons.cutaway),
        ] {
            if bound {
                ensure!(
                    state.is_some_and(|state| state.enabled && state.selected == (walls == mode))
                );
            } else {
                ensure!(state.is_none());
            }
        }
        let frame = sims_render::software::render(viewer.device.as_ref().unwrap().scene());
        if let Ok(output) = std::env::var("SIMS_GUI_EVIDENCE") {
            let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(output);
            ensure!(output.starts_with(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache")));
            std::fs::create_dir_all(&output)?;
            frame.save(output.join(format!("walls-bound-{bound}-{walls}-{}.png", frames.len())))?;
        }
        frames.push(frame);
    }
    ensure!(
        frames[0] != frames[2],
        "walls up and down produced identical lot pixels"
    );
    let session = viewer.session.as_mut().unwrap();
    for invalid in [0, 3, 7, 8] {
        ensure!(session.begin_walls_change(invalid).is_err());
    }
    session.simulation.vm.world.dialogs.modal = Some(99);
    ensure!(session.begin_walls_change(1).is_err());
    ensure!(
        session
            .host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .view
            .floor
            .visibility
            .enabled
            == 1
    );
    session.simulation.vm.world.dialogs.modal = None;
    Ok(())
}

fn reject_missing_wall_publication_owners(session: &mut Session) -> Result<()> {
    let before = serde_json::to_vec(&session.save())?;
    let transport = session.host.presentation.transport.take();
    let result = session.begin_walls_change(1);
    session.host.presentation.transport = transport;
    ensure!(result.unwrap_err().to_string() == "walls change publication owners absent");
    ensure!(serde_json::to_vec(&session.save())? == before);
    if session.host.presentation.frontend_context.is_none() {
        let tools = session.host.presentation.tool_selection.take();
        let result = session.begin_walls_change(1);
        session.host.presentation.tool_selection = tools;
        ensure!(result.unwrap_err().to_string() == "publication frontend inputs absent");
        ensure!(serde_json::to_vec(&session.save())? == before);
    }
    let presenter = session.host.presentation.presenter.take();
    let result = session.begin_walls_change(1);
    session.host.presentation.presenter = presenter;
    ensure!(result.unwrap_err().to_string() == "publication presenter absent");
    ensure!(serde_json::to_vec(&session.save())? == before);
    Ok(())
}
