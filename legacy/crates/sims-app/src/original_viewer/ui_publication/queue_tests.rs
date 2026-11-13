//! Synthetic publication and actual Iced queue-control regressions. No game
//! assets, sessions, engine cancellation, or private checkpoints are required.
use super::super::{headless_input::Input, widget_lifetimes};
use super::*;
use view::Frontend;

fn action() -> sims_vm::action::Action {
    sims_vm::action::Action {
        id: u32::MAX,
        target: 17,
        icon_owner: 17,
        label: Some("Read".into()),
        ..Default::default()
    }
}

fn entry(action: &sims_vm::action::Action, active: bool) -> QueueEntry {
    QueueEntry {
        action: QueueAction::capture(4, Some(91), action, active),
        label: "Read".into(),
        diagnostic_label: "Read".into(),
        target: "Bookcase".into(),
        image: None,
    }
}

#[test]
fn queue_cancel_publication_preserves_authoritative_flag_and_exact_identity() {
    for active in [false, true] {
        for flags in [
            0,
            sims_vm::action::CHECKED | sims_vm::action::AVAILABLE,
            sims_vm::action::CANCELLED,
            sims_vm::action::CANCELLED | sims_vm::action::AVAILABLE,
        ] {
            let mut source = action();
            source.flags = flags;
            let before = source.clone();
            let published = entry(&source, active);
            assert_eq!(source, before, "publication must only read the action");
            assert_eq!(published.action.resident, 4);
            assert_eq!(published.action.resident_lifetime, Some(91));
            assert_eq!(published.action.serial, u32::MAX);
            assert_eq!(published.action.target, 17);
            assert_eq!(published.action.active, active);
            let requested = flags & sims_vm::action::CANCELLED != 0;
            assert_eq!(published.action.cancel_requested, requested);
            match (published.action.cancel(), requested) {
                (Some(Intent::Cancel(serial)), false) => assert_eq!(serial, u64::from(u32::MAX)),
                (None, true) => {}
                _ => panic!("only an action without CANCELLED may publish a cancel intent"),
            }
            assert_eq!(
                published.action.status(),
                if requested {
                    "Cancelling…"
                } else if active {
                    "Active"
                } else {
                    "Queued"
                },
            );
        }
    }
}

#[test]
fn queue_cancel_requested_snapshot_keeps_the_action_and_cannot_issue_another_intent() {
    let mut source = action();
    let original = entry(&source, true);
    source.flags |= sims_vm::action::CANCELLED;
    let requested = entry(&source, true);
    // Keep both cuts alive: the frontend cannot change an earlier publication
    // or claim completion from a request. Only a later engine cut removes it.
    let queue = vec![requested.clone()];
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].action.serial, original.action.serial);
    assert_eq!(queue[0].action.target, original.action.target);
    assert!(!original.action.cancel_requested);
    assert!(matches!(original.action.cancel(), Some(Intent::Cancel(_))));
    for _ in 0..4 {
        assert!(queue[0].action.cancel().is_none());
    }
    source.flags = 0;
    assert!(
        requested.action.cancel_requested,
        "published facts cannot poll the source"
    );
}

fn frontend() -> Frontend {
    // A complete detached publication, with no Viewer construction or loader
    // task. Keep engine/asset owners out of the widget fixtures too.
    Frontend {
        data: Arc::new(Published {
            player_shortcuts_blocked: false,
            camera_shortcuts_blocked: true,
            load_picker_blocked: true,
            load_seed: std::path::PathBuf::new(),
            application: neighborhood_usability::Snapshot {
                provenance: "synthetic".into(),
                neighborhood: None,
                lot: None,
                count: None,
                manifest_sha256: None,
                routing: None,
                buy_frontend: None,
                requested_policy: None,
                requested_buy_preview: false,
                requested_inferred_routing: false,
                notice: String::new(),
            },
            scene: None,
            art: None,
            cluster: None,
            widget_epoch: 7,
            title: String::new(),
            session_present: true,
            cursor_idle: false,
            native_clock: false,
            native_transport: false,
            native_speed: None,
            playing: false,
            speed: 1.,
            zoom: 1.,
            device_zoom: None,
            pending: false,
            modal: false,
            pause: None,
            paused_indicator: false,
            people_visible: true,
            panel_state: None,
            active_panel: None,
            root_visible: true,
            controls: vec![],
            pause_button: None,
            can_buy: false,
            hud_height: 0,
            tutorial_highlight: None,
            status: String::new(),
            save_load_feedback: None,
            error: None,
            policy: Default::default(),
            resident: 4,
            resident_selection: Default::default(),
            residents: vec![],
            money: None,
            clock_fallback: None,
            funds: String::new(),
            needs: None,
            legacy_motives: None,
            mood: None,
            menu: None,
            queue: vec![entry(&action(), true)],
            catalog: None,
            furniture: None,
            tick_label: None,
            dialog: None,
            native_popup: None,
            native_popup_epoch: 0,
            window_observation: None,
            audit: None,
            native_selection: None,
            journey_targets: BTreeMap::new(),
            journey_camera: None,
            journey_tutorial_buttons: None,
            journey_camera_watch: None,
            journey_ground: None,
            journey_walking: None,
            journey_object: None,
        }),
        diagnostics: false,
        options_open: false,
        widget_epoch: 7,
    }
}

fn queue_view(frontend: &Frontend, diagnostics: bool) -> Element<'_, Message> {
    widget_lifetimes::retain(
        frontend.widget_epoch,
        if diagnostics {
            frontend.queue_diagnostics()
        } else {
            frontend.queue_hud()
        },
    )
}

const SIZE: [u32; 2] = [600, 150];

fn point(input: &mut Input, frontend: &Frontend, diagnostics: bool, label: &str) -> iced::Point {
    let observation = input
        .element_observation(queue_view(frontend, diagnostics), SIZE)
        .unwrap();
    let bounds = observation
        .texts
        .iter()
        .find(|t| t.text == label)
        .expect("actual queue control must remain discoverable")
        .bounds;
    iced::Point::new(bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.)
}

fn move_to(point: iced::Point) -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: point })
}
fn down() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}
fn up() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))
}
fn cancels(messages: &[Message]) -> Vec<u64> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Cancel(serial) => Some(*serial),
            _ => None,
        })
        .collect()
}
fn send(
    input: &mut Input,
    frontend: &Frontend,
    diagnostics: bool,
    events: &[iced::Event],
) -> Vec<u64> {
    cancels(
        &input
            .element_batch(queue_view(frontend, diagnostics), SIZE, events)
            .messages,
    )
}

#[test]
#[ignore = "synthetic queue controls; opt-in headless Iced GPU adapter, no private assets"]
fn queue_cancel_synthetic_widgets_disable_repeats_and_preserve_requested_feedback() -> Result<()> {
    let mut input = Input::new()?;
    for diagnostics in [false, true] {
        input.reset();
        let mut frontend = frontend();
        let location = point(&mut input, &frontend, diagnostics, "Cancel");
        assert_eq!(
            send(
                &mut input,
                &frontend,
                diagnostics,
                &[move_to(location), down(), up(), up()]
            ),
            [u64::from(u32::MAX)],
        );
        let mut cancelled = action();
        cancelled.flags |= sims_vm::action::CANCELLED;
        Arc::make_mut(&mut frontend.data).queue = vec![entry(&cancelled, true)];
        let location = point(&mut input, &frontend, diagnostics, "Cancelling…");
        assert!(
            send(
                &mut input,
                &frontend,
                diagnostics,
                &[move_to(location), down(), up(), down(), up(), up(),]
            )
            .is_empty()
        );
        assert_eq!(
            frontend.queue.len(),
            1,
            "a request must not hide the action"
        );
        assert_eq!(frontend.queue[0].action.serial, u32::MAX);
        // A flagged pending record must also disable its button; queue position
        // never substitutes for the authoritative flag.
        Arc::make_mut(&mut frontend.data).queue[0].action.active = false;
        assert!(send(&mut input, &frontend, diagnostics, &[down(), up()]).is_empty());
    }
    Ok(())
}

#[test]
#[ignore = "synthetic queue controls; opt-in headless Iced GPU adapter, no private assets"]
fn queue_cancel_synthetic_widgets_retain_repaints_but_end_replaced_action_presses() -> Result<()> {
    let mut input = Input::new()?;
    for diagnostics in [false, true] {
        input.reset();
        let original = frontend();
        let location = point(&mut input, &original, diagnostics, "Cancel");
        assert!(
            send(
                &mut input,
                &original,
                diagnostics,
                &[move_to(location), down()]
            )
            .is_empty()
        );
        let mut repaint = frontend();
        let publication = Arc::make_mut(&mut repaint.data);
        publication.status = "ordinary new publication".into();
        publication.queue[0].label = "A new display label".into();
        assert_eq!(
            send(&mut input, &repaint, diagnostics, &[up()]),
            [u64::from(u32::MAX)]
        );
        for change in 0..7 {
            input.reset();
            let original = frontend();
            let location = point(&mut input, &original, diagnostics, "Cancel");
            assert!(
                send(
                    &mut input,
                    &original,
                    diagnostics,
                    &[move_to(location), down()]
                )
                .is_empty()
            );
            let mut replacement = frontend();
            let publication = Arc::make_mut(&mut replacement.data);
            let entry = &mut publication.queue[0];
            match change {
                0 => entry.action.serial -= 1,
                1 => entry.action.target += 1,
                2 => {
                    entry.action.resident += 1;
                    publication.resident += 1;
                }
                3 => entry.action.resident_lifetime = Some(92),
                4 => entry.action.active = false,
                5 => entry.action.cancel_requested = true,
                6 => {
                    let mut other = entry.clone();
                    other.action.serial -= 1;
                    publication.queue.insert(0, other);
                }
                _ => unreachable!(),
            }
            assert!(
                send(&mut input, &replacement, diagnostics, &[up()]).is_empty(),
                "held press transferred across queue identity/state change {change}"
            );
            if change != 5 {
                let location = point(&mut input, &replacement, diagnostics, "Cancel");
                assert_eq!(
                    send(
                        &mut input,
                        &replacement,
                        diagnostics,
                        &[move_to(location), down(), up()]
                    ),
                    [u64::from(replacement.queue[0].action.serial)]
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "synthetic queue controls; opt-in headless Iced GPU adapter, no private assets"]
fn queue_cancel_synthetic_widgets_keep_blocking_and_lifetime_gates() -> Result<()> {
    let mut input = Input::new()?;
    for diagnostics in [false, true] {
        for gate in 0..8 {
            input.reset();
            let original = frontend();
            let location = point(&mut input, &original, diagnostics, "Cancel");
            assert!(
                send(
                    &mut input,
                    &original,
                    diagnostics,
                    &[move_to(location), down()]
                )
                .is_empty()
            );
            let mut blocked = frontend();
            let data = Arc::make_mut(&mut blocked.data);
            match gate {
                0 => data.pending = true,
                1 => data.modal = true,
                2 => data.error = Some("existing failure".into()),
                3 => data.people_visible = false,
                4 => data.session_present = false,
                5 => data.resident += 1,
                6 => data.queue.clear(),
                7 => blocked.widget_epoch += 1,
                _ => unreachable!(),
            }
            assert!(send(&mut input, &blocked, diagnostics, &[]).is_empty());
            // Once an intervening cut ends the press, re-enabling the same
            // identity must require a new press, not just the old release.
            assert!(send(&mut input, &original, diagnostics, &[up()]).is_empty());
            assert_eq!(
                send(
                    &mut input,
                    &original,
                    diagnostics,
                    &[move_to(location), down(), up()]
                ),
                [u64::from(u32::MAX)]
            );
            if gate != 7 {
                assert!(
                    send(&mut input, &blocked, diagnostics, &[down(), up()]).is_empty(),
                    "blocked queue emitted a cancel intent for gate {gate}"
                );
            }
        }
        for barrier in [
            iced::Event::Window(window::Event::Unfocused),
            iced::Event::Window(window::Event::Resized(Size::new(600., 150.))),
        ] {
            input.reset();
            let original = frontend();
            let location = point(&mut input, &original, diagnostics, "Cancel");
            assert!(
                send(
                    &mut input,
                    &original,
                    diagnostics,
                    &[move_to(location), down(), barrier, up()]
                )
                .is_empty()
            );
        }
    }
    Ok(())
}
