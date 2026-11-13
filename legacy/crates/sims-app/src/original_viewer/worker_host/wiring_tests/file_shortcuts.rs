//! Source-authored controls only. ALL UNRUN, including compilation. Parent owns
//! fixture execution, renderer/backend validation and native acceptance.
use super::*;
use load_picker::tests::{FocusUi, focus_enter, visit_opening_task};

fn shortcut(copy: bool) -> iced::Event {
    press(Key::Character(if copy { "s" } else { "o" }.into()),
        if copy { Code::KeyS } else { Code::KeyO }, false,
        player_shortcuts::primary_modifier() | if copy { Modifiers::SHIFT } else { Modifiers::NONE })
}
fn key_up(copy: bool) -> iced::Event { release(if copy { Code::KeyS } else { Code::KeyO }) }
fn message(event: iced::Event, status: Status) -> Message {
    subscription_event(event, status, window::Id::unique()).expect("file input lost by subscription")
}
fn direct(viewer: &mut Viewer, event: iced::Event, status: Status) -> Task<Message> {
    viewer.update(message(event, status))
}
fn hosted(host: &mut Host, event: iced::Event, status: Status) -> Task<Event> {
    host.update(Event::Widget { generation: host.generation(), message: message(event, status) })
}
fn widgets<T: 'static>(task: Task<T>) -> usize {
    let mut count = 0;
    visit_opening_task(task, |action| match action {
        iced_runtime::Action::Widget(_) => count += 1,
        _ => panic!("shell-only opening/rejection emitted owner work"),
    });
    count
}
fn purpose(copy: bool) -> load_picker::Purpose {
    if copy { load_picker::Purpose::Copy } else { load_picker::Purpose::Load }
}

#[test]
fn file_shortcut_adapters_open_each_purpose_focus_once_from_play_or_options() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            for options in [false, true] {
                let mut h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                h.viewer.options_open = options;
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 2);
                assert!(h.viewer.options_open && h.viewer.picker_enabled());
                assert_eq!(h.viewer.load_picker.as_ref().unwrap().purpose(), purpose(copy));
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 0);
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(!copy), Status::Ignored)), 0);
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &observed);
                assert_eq!(ordinal(&h.account), count);
                assert!(!h.destination.exists());

                let h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                let mut host = Host::from_viewer_for_test(h.viewer);
                host.frontend.options_open = options;
                let revision = host.shell_revision;
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
                assert!(host.frontend.options_open && host.picker_enabled());
                assert_eq!(host.shell_revision, revision + u64::from(!options));
                assert_eq!(host.load_picker.as_ref().unwrap().purpose(), purpose(copy));
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
                assert_eq!(widgets(hosted(&mut host, shortcut(!copy), Status::Ignored)), 0);
                assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
                assert!(host.in_flight_load.is_none() && host.in_flight_copy.is_none());
                let viewer = host.recover_for_test();
                assert_eq!(saved(&viewer)?, before);
                assert_eq!(origin(&viewer), &observed);
                assert_eq!(ordinal(&h.account), count);
                assert!(!h.destination.exists());
            }
        }
        Ok(())
    })
}

#[test]
fn file_shortcut_adapters_current_owner_modal_pending_and_overlay_guards_emit_no_focus() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            for guard in ["diagnostics", "confirmation", "modal", "pending", "fault", "absent", "unfocused"] {
                let mut h = Household::new()?;
                match guard {
                    "diagnostics" => h.viewer.diagnostics = true,
                    "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation {
                        choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                    "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                    "pending" => h.viewer.viewport_enter_requested = true,
                    "fault" => h.viewer.error = Some("authored owner fault".into()),
                    "absent" => h.viewer.replace_session(None),
                    "unfocused" => { drop(direct(&mut h.viewer, iced::Event::Window(window::Event::Unfocused), Status::Captured)); }
                    _ => unreachable!(),
                }
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 0, "direct {guard}");
                assert!(h.viewer.load_picker.is_none() && !h.viewer.options_open);
                assert!(!h.destination.exists());
            }
            for guard in ["diagnostics", "confirmation", "modal", "pending", "owner", "held", "fault", "absent", "unfocused", "closing", "replacing"] {
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                match guard {
                    "diagnostics" => host.frontend.diagnostics = true,
                    "confirmation" => host.application_confirmation = Some(neighborhood_usability::Confirmation {
                        choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                    "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                    "pending" => Arc::make_mut(&mut host.frontend.data).pending = true,
                    "owner" => Arc::make_mut(&mut host.frontend.data).load_picker_blocked = true,
                    "held" => Arc::make_mut(&mut host.frontend.data).player_shortcuts_blocked = true,
                    "fault" => host.error = Some("authored owner fault".into()),
                    "absent" => Arc::make_mut(&mut host.frontend.data).session_present = false,
                    "unfocused" => {
                        drop(hosted(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured));
                        host.settle_for_test()?;
                    }
                    "closing" => host.closing = Some(window::Id::unique()),
                    "replacing" => host.replacing = true,
                    _ => unreachable!(),
                }
                let authority = host.save_shortcut_authority.clone();
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0, "Host {guard}");
                assert!(host.load_picker.is_none() && !host.frontend.options_open && host.queued.is_empty());
                assert!(Arc::ptr_eq(&authority, &host.save_shortcut_authority));
                assert!(!host.worker.as_ref().unwrap().busy());
                assert!(!h.destination.exists());
                let _ = host.recover_for_test();
            }
        }
        Ok(())
    })
}

#[test]
fn file_shortcut_adapters_capture_plain_repeat_foreign_extra_and_s_conversion_wait_for_release() -> Result<()> {
    bounded(|| {
        let foreign = if cfg!(target_os = "macos") { Modifiers::CTRL } else { Modifiers::LOGO };
        for copy in [false, true] {
            for refusal in ["capture", "plain", "repeat", "foreign", "extra"] {
                let mut rejected = shortcut(copy);
                if let iced::Event::Keyboard(keyboard::Event::KeyPressed { modifiers, repeat, .. }) = &mut rejected {
                    *modifiers = match refusal { "plain" => Modifiers::NONE, "foreign" => foreign,
                        "extra" => *modifiers | Modifiers::ALT, _ => *modifiers };
                    *repeat = refusal == "repeat";
                }
                let status = if refusal == "capture" { Status::Captured } else { Status::Ignored };
                let mut h = Household::new()?;
                assert_eq!(widgets(direct(&mut h.viewer, rejected.clone(), status)), 0);
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 0);
                assert!(h.viewer.load_picker.is_none() && !h.viewer.options_open);
                assert_eq!(widgets(direct(&mut h.viewer, key_up(copy), Status::Captured)), 0);
                assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 2);
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                assert_eq!(widgets(hosted(&mut host, rejected, status)), 0);
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
                assert!(host.load_picker.is_none() && !host.frontend.options_open);
                assert_eq!(widgets(hosted(&mut host, key_up(copy), Status::Captured)), 0);
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
                let _ = host.recover_for_test();
            }
        }
        // Ordinary Save remains the S action until release, including adding
        // Shift without a repeat bit. A fresh released S may then open Copy.
        let save = press(Key::Character("s".into()), Code::KeyS, false, player_shortcuts::primary_modifier());
        let mut h = Household::new()?;
        drop(direct(&mut h.viewer, save.clone(), Status::Ignored));
        copy_bytes(&h.destination, &h.viewer)?;
        assert_eq!(widgets(direct(&mut h.viewer, shortcut(true), Status::Ignored)), 0);
        assert!(h.viewer.load_picker.is_none());
        drop(direct(&mut h.viewer, key_up(true), Status::Captured));
        assert_eq!(widgets(direct(&mut h.viewer, shortcut(true), Status::Ignored)), 2);
        let h = Household::new()?;
        let mut host = Host::from_viewer_for_test(h.viewer);
        drop(hosted(&mut host, save, Status::Ignored));
        host.settle_for_test()?;
        assert_eq!(widgets(hosted(&mut host, shortcut(true), Status::Ignored)), 0);
        assert!(host.load_picker.is_none());
        drop(hosted(&mut host, key_up(true), Status::Captured));
        assert_eq!(widgets(hosted(&mut host, shortcut(true), Status::Ignored)), 2);
        copy_bytes(&h.destination, &host.recover_for_test())?;
        Ok(())
    })
}

#[test]
fn file_shortcut_host_stale_and_effect_openings_consume_without_shell_or_save_authority_changes() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            for effect in [false, true] {
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                let generation = host.generation();
                let source = if effect { generation } else { generation + 1 };
                let event = if effect { Event::Effect { generation: source, message: message(shortcut(copy), Status::Ignored) } }
                    else { Event::Widget { generation: source, message: message(shortcut(copy), Status::Ignored) } };
                let authority = host.save_shortcut_authority.clone();
                assert_eq!(widgets(host.update(event)), 0);
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
                assert!(host.load_picker.is_none() && !host.frontend.options_open && host.queued.is_empty());
                assert!(Arc::ptr_eq(&authority, &host.save_shortcut_authority));
                assert_eq!(widgets(host.update(Event::Widget { generation: source,
                    message: message(key_up(copy), Status::Captured) })), 0);
                assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
                assert!(!authority.load(Ordering::Acquire));
                assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
                let _ = host.recover_for_test();
            }
        }
        Ok(())
    })
}

#[test]
fn file_shortcut_adapters_cancel_reopen_stale_picker_and_pending_retry_keep_focus_once() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            let mut h = Household::new()?;
            let before = saved(&h.viewer)?;
            assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 2);
            let stale = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
            let escape = press(Key::Named(Named::Escape), Code::Escape, false, Modifiers::NONE);
            assert_eq!(widgets(direct(&mut h.viewer, escape.clone(), Status::Captured)), 0);
            assert!(h.viewer.load_picker.is_none());
            assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 0);
            drop(direct(&mut h.viewer, key_up(copy), Status::Captured));
            assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 2);
            assert_eq!(widgets(h.viewer.update(Message::LoadPicker(stale))), 0);
            let (_, ticket) = pending(h.viewer.load_picker.as_mut().unwrap());
            drop(direct(&mut h.viewer, key_up(copy), Status::Captured));
            assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 0);
            assert!(h.viewer.load_picker.as_ref().unwrap().pending_ticket(&ticket));
            assert!(h.viewer.load_picker.as_mut().unwrap().settle(&ticket, Some("authored retry".into())));
            let edit = h.viewer.load_picker.as_ref().unwrap().edit_event_for_test("./exact retry α.json".into());
            assert_eq!(widgets(h.viewer.update(Message::LoadPicker(edit))), 0);
            assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), "./exact retry α.json");
            assert_eq!(saved(&h.viewer)?, before);
            assert!(!h.destination.exists());

            let h = Household::new()?;
            let before = saved(&h.viewer)?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
            let stale = host.load_picker.as_ref().unwrap().submit_event_for_test();
            assert_eq!(widgets(hosted(&mut host, escape, Status::Captured)), 0);
            assert!(host.load_picker.is_none());
            assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
            drop(hosted(&mut host, key_up(copy), Status::Captured));
            assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
            send(&mut host, Message::LoadPicker(stale));
            let (_, ticket) = pending(host.load_picker.as_mut().unwrap());
            drop(hosted(&mut host, key_up(copy), Status::Captured));
            assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
            assert!(host.load_picker.as_ref().unwrap().pending_ticket(&ticket));
            assert!(host.load_picker.as_mut().unwrap().settle(&ticket, Some("authored retry".into())));
            let edit = host.load_picker.as_ref().unwrap().edit_event_for_test("./exact retry α.json".into());
            send(&mut host, Message::LoadPicker(edit));
            assert_eq!(host.load_picker.as_ref().unwrap().value(), "./exact retry α.json");
            assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
            assert_eq!(saved(&host.recover_for_test())?, before);
            assert!(!h.destination.exists());
        }
        Ok(())
    })
}

#[test]
fn file_shortcut_adapters_existing_managed_load_and_copy_fail_retry_and_preserve_save_destination() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            for responsive in [false, true] {
                let mut h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                let output = if copy { h.files.0.join("shortcut real copy α.json") } else { h.second.clone() };
                let rejected = if copy { h.files.0.clone() } else { h.files.0.join("absent.json") };
                if responsive {
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 2);
                    host_edit(&mut host, &rejected);
                    host_submit(&mut host);
                    host.settle_for_test()?;
                    assert!(!host.load_picker.as_ref().unwrap().pending());
                    assert_eq!(host.load_picker.as_ref().unwrap().value(), rejected.to_str().unwrap());
                    assert!(host.frontend.save_load_feedback.is_some());
                    assert!(host.frontend.error.is_none() && host.error.is_none());
                    host_edit(&mut host, &output);
                    host_submit(&mut host);
                    host.settle_for_test()?;
                    assert!(host.load_picker.is_none());
                    h.viewer = host.recover_for_test();
                } else {
                    assert_eq!(widgets(direct(&mut h.viewer, shortcut(copy), Status::Ignored)), 2);
                    edit(&mut h.viewer, &rejected);
                    submit(&mut h.viewer);
                    assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                    assert!(h.viewer.error.is_none());
                    assert_eq!(saved(&h.viewer)?, before);
                    assert_eq!(origin(&h.viewer), &observed);
                    edit(&mut h.viewer, &output);
                    submit(&mut h.viewer);
                    assert!(h.viewer.load_picker.is_none());
                }
                assert!(h.viewer.error.is_none());
                assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
                assert!(!h.destination.exists());
                if copy {
                    copy_bytes(&output, &h.viewer)?;
                    assert_eq!(saved(&h.viewer)?, before);
                    assert_eq!(origin(&h.viewer), &observed);
                    assert_eq!(ordinal(&h.account), count);
                } else {
                    assert_eq!((h.viewer.zoom, h.viewer.floor), (0.75, 2));
                    assert_eq!(origin(&h.viewer).requested_path.as_bytes(), output.to_str().unwrap().as_bytes());
                    assert_eq!(ordinal(&h.account), count + 1);
                }
            }
        }
        Ok(())
    })
}

#[test]
fn file_shortcut_host_accepted_opening_retains_single_head_poll_and_two_focus_actions() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            host.window_capture = super::super::super::window_capture::Capture::new(true);
            let root = window::Id::unique();
            let event = host.window_capture.opened_for_test(root);
            drop(host.window_capture.handle(event, Instant::now()));
            drop(host.window_capture.synchronize(host.generation() + 1, Instant::now()));
            let mut runs = 0;
            let mut focus = 0;
            visit_opening_task(hosted(&mut host, shortcut(copy), Status::Ignored), |action| match action {
                iced_runtime::Action::Window(iced_runtime::window::Action::Run(id, callback)) => {
                    assert_eq!(id, root); drop(callback); runs += 1;
                }
                iced_runtime::Action::Widget(_) => focus += 1,
                _ => panic!("unexpected picker opening work"),
            });
            assert_eq!((runs, focus), (1, 2));
            assert!(host.load_picker.is_some() && host.queued.is_empty());
            let _ = host.recover_for_test();
        }
        Ok(())
    })
}

struct Resume(Option<std::sync::mpsc::SyncSender<()>>);
impl Drop for Resume {
    fn drop(&mut self) { if let Some(sender) = self.0.take() { let _ = sender.send(()); } }
}

#[test]
fn file_shortcut_host_busy_and_queued_save_rejection_preserves_existing_save_authority() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            let (entered, resume) = host.gate_next_for_test();
            let mut resume = Resume(Some(resume));
            send(&mut host, Message::Shift(false));
            entered.recv_timeout(std::time::Duration::from_secs(5))?;
            let save = press(Key::Character("s".into()), Code::KeyS, false, player_shortcuts::primary_modifier());
            drop(hosted(&mut host, save, Status::Ignored));
            assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(0, Message::Save)]));
            // Release the real Save hold before trying Copy on the same key.
            drop(hosted(&mut host, release(Code::KeyS), Status::Captured));
            let authority = host.save_shortcut_authority.clone();
            assert_eq!(widgets(hosted(&mut host, shortcut(copy), Status::Ignored)), 0);
            assert!(host.load_picker.is_none() && !host.frontend.options_open);
            assert!(Arc::ptr_eq(&authority, &host.save_shortcut_authority) && authority.load(Ordering::Acquire));
            assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(0, Message::Save)]));
            resume.0.take().unwrap().send(())?;
            host.settle_for_test()?;
            copy_bytes(&h.destination, &host.recover_for_test())?;
        }
        Ok(())
    })
}

#[test]
#[ignore = "opt-in real Iced focus/select-all, synthetic paste/Enter and managed-file fixtures; ALL UNRUN"]
fn file_shortcut_widgets_both_adapters_focus_type_retry_and_submit_existing_load_or_copy() -> Result<()> {
    bounded(|| {
        for copy in [false, true] {
            for responsive in [false, true] {
                let mut h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                let output = if copy { h.files.0.join("shortcut copy α.json") } else { h.second.clone() };
                let rejected = if copy { h.files.0.clone() } else { h.files.0.join("absent.json") };
                let mut ui = FocusUi::new()?;
                if responsive {
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    let task = hosted(&mut host, shortcut(copy), Status::Ignored);
                    assert_eq!(ui.task(host.view_messages(), task), 2);
                    for event in ui.paste(host.view_messages(), rejected.to_str().unwrap()) { send(&mut host, event); }
                    assert_eq!(host.load_picker.as_ref().unwrap().value(), rejected.to_str().unwrap(), "focus/select-all ordering did not replace seed");
                    let submits = ui.events(host.view_messages(), &[focus_enter()]);
                    assert_eq!(submits.len(), 1, "shortcut opening required a pointer click");
                    for event in submits { send(&mut host, event); }
                    for task in host.settle_for_test()? { assert_eq!(widgets(task), 0); }
                    assert!(!host.load_picker.as_ref().unwrap().pending());
                    // Correct the retained field entirely through keyboard
                    // selection and paste, without another focus operation.
                    assert!(ui.events(host.view_messages(), &[
                        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::COMMAND)),
                        press(Key::Character("a".into()), Code::KeyA, false, Modifiers::COMMAND),
                        release(Code::KeyA),
                        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::NONE)),
                    ]).is_empty());
                    for event in ui.paste(host.view_messages(), output.to_str().unwrap()) { send(&mut host, event); }
                    assert_eq!(host.load_picker.as_ref().unwrap().value(), output.to_str().unwrap(), "keyboard retry must replace the complete rejected path");
                    let submits = ui.events(host.view_messages(), &[focus_enter()]);
                    assert_eq!(submits.len(), 1);
                    for event in submits { send(&mut host, event); }
                    host.settle_for_test()?;
                    assert!(host.load_picker.is_none());
                    h.viewer = host.recover_for_test();
                } else {
                    let task = direct(&mut h.viewer, shortcut(copy), Status::Ignored);
                    assert_eq!(ui.task(h.viewer.view(), task), 2);
                    for event in ui.paste(h.viewer.view(), rejected.to_str().unwrap()) { drop(h.viewer.update(event)); }
                    assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), rejected.to_str().unwrap(), "focus/select-all ordering did not replace seed");
                    let submits = ui.events(h.viewer.view(), &[focus_enter()]);
                    assert_eq!(submits.len(), 1);
                    for event in submits { assert_eq!(widgets(h.viewer.update(event)), 0); }
                    assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                    assert!(ui.events(h.viewer.view(), &[
                        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::COMMAND)),
                        press(Key::Character("a".into()), Code::KeyA, false, Modifiers::COMMAND),
                        release(Code::KeyA),
                        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::NONE)),
                    ]).is_empty());
                    for event in ui.paste(h.viewer.view(), output.to_str().unwrap()) { drop(h.viewer.update(event)); }
                    assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), output.to_str().unwrap(), "keyboard retry must replace the complete rejected path");
                    let submits = ui.events(h.viewer.view(), &[focus_enter()]);
                    assert_eq!(submits.len(), 1);
                    for event in submits { drop(h.viewer.update(event)); }
                    assert!(h.viewer.load_picker.is_none());
                }
                assert!(h.viewer.error.is_none());
                assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
                assert!(!h.destination.exists());
                if copy {
                    copy_bytes(&output, &h.viewer)?;
                    assert_eq!(saved(&h.viewer)?, before);
                    assert_eq!(origin(&h.viewer), &observed);
                    assert_eq!(ordinal(&h.account), count);
                } else {
                    assert_eq!((h.viewer.zoom, h.viewer.floor), (0.75, 2));
                    assert_eq!(origin(&h.viewer).requested_path.as_bytes(), output.to_str().unwrap().as_bytes());
                    assert_eq!(ordinal(&h.account), count + 1);
                }
            }
        }
        Ok(())
    })
}
