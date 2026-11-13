//! Focused application-adapter controls. ALL UNRUN, including compilation.
//! Existing source-authored household/resources only; no native-window fixture.
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use sims_runtime::exploration as engine;
use std::path::PathBuf;
use iced::{event::Status, keyboard::{self, Key, Location, Modifiers, key::{Code, Named, Physical}}};
use super::super::super::support::window_session::loaded_input::Account;

// Actual subscription -> both adapters -> existing pickers. ALL UNRUN.
#[path = "wiring_tests/file_shortcuts.rs"]
mod file_shortcuts;

// Actual authored-file Load controls. ALL UNRUN, including compilation.
#[path = "wiring_tests/recent_load_controls.rs"]
mod recent_load_controls;

// ALL picker_focus_ controls are authored and UNRUN, including compilation.
mod picker_focus_controls {
    use super::*;
    use load_picker::tests::{FocusUi, focus_enter, visit_opening_task};

    fn choose(copy: bool) -> Message { if copy { Message::ChooseSaveCopy } else { Message::ChooseLoad } }
    fn widget_actions<T: 'static>(task: Task<T>) -> usize {
        let mut count = 0;
        visit_opening_task(task, |action| match action {
            iced_runtime::Action::Widget(_) => count += 1,
            _ => panic!("picker-only dispatch emitted non-UI work"),
        });
        count
    }
    fn host_task(host: &mut Host, message: Message) -> Task<Event> {
        host.update(Event::Widget { generation: host.generation(), message })
    }

    #[test]
    fn picker_focus_adapters_open_once_edits_pending_retry_and_stale_events_do_not_refocus() -> Result<()> {
        bounded(|| {
            for copy in [false, true] {
                let mut h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                h.viewer.options_open = true;
                assert_eq!(widget_actions(h.viewer.update(choose(copy))), 2);
                assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                assert_eq!(saved(&h.viewer)?, before, "opening consumed or saved a household");
                let stale = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
                for message in [choose(copy), choose(!copy)] {
                    assert_eq!(widget_actions(h.viewer.update(message)), 0);
                }
                let edit = h.viewer.load_picker.as_ref().unwrap().edit_event_for_test(" ./exact//retry.json ".into());
                assert_eq!(widget_actions(h.viewer.update(Message::LoadPicker(edit))), 0);
                assert_eq!(widget_actions(h.viewer.update(Message::LoadPicker(stale))), 0);
                let (_, ticket) = pending(h.viewer.load_picker.as_mut().unwrap());
                assert_eq!(widget_actions(h.viewer.update(choose(copy))), 0);
                assert!(h.viewer.load_picker.as_mut().unwrap().settle(&ticket, Some("retry retained".into())));
                let edit = h.viewer.load_picker.as_ref().unwrap().edit_event_for_test(" ./exact//repaired.json ".into());
                assert_eq!(widget_actions(h.viewer.update(Message::LoadPicker(edit))), 0);
                assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), " ./exact//repaired.json ");
                assert_eq!(widget_actions(h.viewer.update(Message::PlayerShortcut(player_shortcuts::Input::FocusGained))), 0);
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &observed);
                assert_eq!(ordinal(&h.account), count);
                assert!(!h.destination.exists());

                let h = Household::new()?;
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let count = ordinal(&h.account);
                let mut host = Host::from_viewer_for_test(h.viewer);
                host.frontend.options_open = true;
                assert_eq!(widget_actions(host_task(&mut host, choose(copy))), 2);
                let stale = host.load_picker.as_ref().unwrap().submit_event_for_test();
                for message in [choose(copy), choose(!copy)] {
                    assert_eq!(widget_actions(host_task(&mut host, message)), 0);
                }
                let edit = host.load_picker.as_ref().unwrap().edit_event_for_test(" ./exact//retry.json ".into());
                assert_eq!(widget_actions(host_task(&mut host, Message::LoadPicker(edit))), 0);
                assert_eq!(widget_actions(host_task(&mut host, Message::LoadPicker(stale))), 0);
                let (_, ticket) = pending(host.load_picker.as_mut().unwrap());
                assert_eq!(widget_actions(host_task(&mut host, choose(copy))), 0);
                assert_eq!(widget_actions(host.update(Event::Completion)), 0);
                assert!(host.load_picker.as_ref().unwrap().pending_ticket(&ticket));
                assert!(host.load_picker.as_mut().unwrap().settle(&ticket, Some("retry retained".into())));
                let edit = host.load_picker.as_ref().unwrap().edit_event_for_test(" ./exact//repaired.json ".into());
                assert_eq!(widget_actions(host_task(&mut host, Message::LoadPicker(edit))), 0);
                assert_eq!(widget_actions(host_task(&mut host, Message::PlayerShortcut(player_shortcuts::Input::FocusGained))), 0);
                assert_eq!(host.load_picker.as_ref().unwrap().value(), " ./exact//repaired.json ");
                assert!(host.queued.is_empty() && host.in_flight_load.is_none() && host.in_flight_copy.is_none());
                assert!(!host.worker.as_ref().unwrap().busy());
                let viewer = host.recover_for_test();
                assert_eq!(saved(&viewer)?, before);
                assert_eq!(origin(&viewer), &observed);
                assert_eq!(ordinal(&h.account), count);
                assert!(!h.destination.exists());
            }
            Ok(())
        })
    }

    #[test]
    fn picker_focus_adapters_rejected_openings_emit_no_ui_or_owner_work() -> Result<()> {
        bounded(|| {
            for copy in [false, true] {
                for guard in ["Options", "diagnostics", "confirmation", "modal", "pending", "fault", "absent"] {
                    let mut h = Household::new()?;
                    h.viewer.options_open = true;
                    match guard {
                        "Options" => h.viewer.options_open = false,
                        "diagnostics" => h.viewer.diagnostics = true,
                        "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                        "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                        "pending" => h.viewer.viewport_enter_requested = true,
                        "fault" => h.viewer.error = Some("authored current fault".into()),
                        "absent" => h.viewer.replace_session(None),
                        _ => unreachable!(),
                    }
                    assert_eq!(widget_actions(h.viewer.update(choose(copy))), 0, "direct {guard}");
                    assert!(h.viewer.load_picker.is_none());
                    assert!(!h.destination.exists());
                }
                for guard in ["generation", "effect", "Options", "diagnostics", "confirmation", "modal", "pending", "fault", "absent", "closing", "replacing"] {
                    let h = Household::new()?;
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    host.frontend.options_open = true;
                    match guard {
                        "Options" => host.frontend.options_open = false,
                        "diagnostics" => host.frontend.diagnostics = true,
                        "confirmation" => host.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                        "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                        "pending" => Arc::make_mut(&mut host.frontend.data).load_picker_blocked = true,
                        "fault" => host.error = Some("authored current fault".into()),
                        "absent" => Arc::make_mut(&mut host.frontend.data).session_present = false,
                        "closing" => host.closing = Some(window::Id::unique()),
                        "replacing" => host.replacing = true,
                        _ => {},
                    }
                    let event = if guard == "effect" { Event::Effect { generation: host.generation(), message: choose(copy) } }
                        else { Event::Widget { generation: if guard == "generation" { u64::MAX } else { host.generation() }, message: choose(copy) } };
                    assert_eq!(widget_actions(host.update(event)), 0, "Host {guard}");
                    assert!(host.load_picker.is_none() && host.queued.is_empty());
                    assert!(!host.worker.as_ref().unwrap().busy());
                    assert!(!h.destination.exists());
                    let _ = host.recover_for_test();
                }
                if copy {
                    let mut h = Household::new()?;
                    h.viewer.options_open = true;
                    h.viewer.copy_focused = false;
                    assert_eq!(widget_actions(h.viewer.update(choose(true))), 0);
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    host.copy_focused = false;
                    assert_eq!(widget_actions(host_task(&mut host, choose(true))), 0);
                    assert!(host.load_picker.is_none());
                    let _ = host.recover_for_test();
                }
            }
            Ok(())
        })
    }

    #[cfg(unix)]
    #[test]
    fn picker_focus_invalid_seed_returns_feedback_without_a_task_in_both_adapters() -> Result<()> {
        bounded(|| {
            use std::os::unix::ffi::OsStringExt;
            for copy in [false, true] {
                let mut h = Household::new()?;
                let seed = PathBuf::from(std::ffi::OsString::from_vec(b"invalid-\xff.json".to_vec()));
                if copy { h.viewer.last_copy_path = Some(seed.clone()); } else { h.viewer.last_load_path = Some(seed.clone()); }
                h.viewer.options_open = true;
                assert_eq!(widget_actions(h.viewer.update(choose(copy))), 0);
                assert!(h.viewer.load_picker.is_none());
                let feedback = if copy { &h.viewer.save_feedback } else { &h.viewer.load_feedback };
                assert!(feedback.as_ref().unwrap().contains("UTF-8"));
                let mut host = Host::from_viewer_for_test(h.viewer);
                if copy { host.last_copy_path = Some(seed); } else { host.last_load_path = Some(seed); }
                assert_eq!(widget_actions(host_task(&mut host, choose(copy))), 0);
                assert!(host.load_picker.is_none());
                assert!(host.picker_feedback.as_ref().unwrap().contains("UTF-8"));
                let _ = host.recover_for_test();
            }
            Ok(())
        })
    }

    #[test]
    fn picker_focus_host_accepted_opening_retains_head_poll_action() -> Result<()> {
        bounded(|| {
            for copy in [false, true] {
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                host.frontend.options_open = true;
                host.window_capture = super::super::super::window_capture::Capture::new(true);
                let root = window::Id::unique();
                let event = host.window_capture.opened_for_test(root);
                drop(host.window_capture.handle(event, Instant::now()));
                // Leave Capture on another generation so the actual head poll
                // has a real exact-root Run action to preserve. Never call it.
                drop(host.window_capture.synchronize(host.generation() + 1, Instant::now()));
                let mut runs = 0;
                let mut widgets = 0;
                visit_opening_task(host_task(&mut host, choose(copy)), |action| match action {
                    iced_runtime::Action::Window(iced_runtime::window::Action::Run(id, callback)) => {
                        assert_eq!(id, root);
                        drop(callback);
                        runs += 1;
                    }
                    iced_runtime::Action::Widget(_) => widgets += 1,
                    _ => panic!("unexpected opening action"),
                });
                assert_eq!((runs, widgets), (1, 2));
                assert!(host.load_picker.is_some() && host.queued.is_empty());
                let _ = host.recover_for_test();
            }
            Ok(())
        })
    }

    #[test]
    #[ignore = "opt-in real Iced wgpu adapter tasks, synthetic paste/Enter, authored household; UNRUN"]
    fn picker_focus_widgets_both_adapters_paste_enter_use_existing_load_and_copy() -> Result<()> {
        bounded(|| {
            for copy in [false, true] {
                for hosted in [false, true] {
                    let mut h = Household::new()?;
                    let before = saved(&h.viewer)?;
                    let observed = origin(&h.viewer).clone();
                    let count = ordinal(&h.account);
                    let output = if copy { h.files.0.join("keyboard copy α.json") } else { h.second.clone() };
                    let mut ui = FocusUi::new()?;
                    if hosted {
                        let mut host = Host::from_viewer_for_test(h.viewer);
                        host.frontend.options_open = true;
                        let task = host_task(&mut host, choose(copy));
                        assert_eq!(ui.task(host.view_messages(), task), 2);
                        assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
                        assert_eq!(ordinal(&h.account), count);
                        for message in ui.paste(host.view_messages(), output.to_str().unwrap()) {
                            let task = host_task(&mut host, message);
                            assert_eq!(ui.task(host.view_messages(), task), 0);
                        }
                        let messages = ui.events(host.view_messages(), &[focus_enter()]);
                        assert_eq!(messages.len(), 1, "Enter needed an initial pointer click");
                        for message in messages { drop(host_task(&mut host, message)); }
                        for task in host.settle_for_test()? { assert_eq!(widget_actions(task), 0); }
                        assert!(host.load_picker.is_none());
                        h.viewer = host.recover_for_test();
                    } else {
                        h.viewer.options_open = true;
                        let task = h.viewer.update(choose(copy));
                        assert_eq!(ui.task(h.viewer.view(), task), 2);
                        assert_eq!(saved(&h.viewer)?, before);
                        assert_eq!(ordinal(&h.account), count);
                        for message in ui.paste(h.viewer.view(), output.to_str().unwrap()) {
                            let task = h.viewer.update(message);
                            assert_eq!(ui.task(h.viewer.view(), task), 0);
                        }
                        let messages = ui.events(h.viewer.view(), &[focus_enter()]);
                        assert_eq!(messages.len(), 1, "Enter needed an initial pointer click");
                        for message in messages { assert_eq!(widget_actions(h.viewer.update(message)), 0); }
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
}

#[path = "../save_feedback/fixture.rs"]
mod fixture;

struct Files(PathBuf);
impl Files {
    fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("sims-control-wiring-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&path)?;
        std::fs::create_dir(path.join("original-data"))?;
        Ok(Self(path))
    }
}
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn bounded(f: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(f)?.join()
        .map_err(|_| anyhow::anyhow!("authored wiring control panicked"))?
}
fn saved(viewer: &Viewer) -> Result<Value> {
    let s = viewer.session.as_ref().context("authored household absent")?;
    Ok(json!({"session":s.save_managed()?,
        "dialog_client":viewer.client.as_ref().unwrap().save_managed(s)?,
        "camera_device":viewer.device.as_ref().map(|d| d.save(s)).transpose()?,
        "floor":viewer.floor,"zoom":viewer.zoom,"resident":viewer.resident,
        "selected":viewer.selected,"tutorial_highlight":viewer.tutorial_highlight,
        "viewport_enter_requested":viewer.viewport_enter_requested,"move_inputs":viewer.move_inputs}))
}
fn origin(viewer: &Viewer) -> &sims_runtime::original_session::player_input::Observation {
    viewer.session.as_ref().unwrap().loaded_input_origin().unwrap()
}
fn ordinal(account: &Account) -> u64 { account.inspect_for_test()["transition_ordinal"].as_u64().unwrap() }
struct Household {
    files: Files,
    viewer: Viewer,
    account: Account,
    first: PathBuf,
    second: PathBuf,
    second_bytes: Vec<u8>,
    second_value: Value,
    destination: PathBuf,
}
impl Household {
    fn new() -> Result<Self> {
        let files = Files::new()?;
        let first = files.0.join("first.json");
        // Keep an observable spelling distinction; Path equality alone would
        // normalize the dot. The actual Input observation must retain it.
        let second = PathBuf::from(format!("{}/./second household α.json", files.0.to_str().unwrap()));
        let destination = files.0.join("save-destination.json");
        let mut viewer = fixture::viewer(&files.0.join("original-data"), &destination)?;
        viewer.zoom = 0.5;
        let value = saved(&viewer)?;
        std::fs::write(&first, serde_json::to_vec(&value)?)?;
        let mut second_value = value;
        second_value["zoom"] = json!(0.75);
        second_value["floor"] = json!(2);
        let mut second_bytes = serde_json::to_vec(&second_value)?;
        second_bytes.extend_from_slice(b" \n");
        std::fs::write(&second, &second_bytes)?;
        let index = viewer.session.as_ref().unwrap().index().clone();
        viewer.replace_session(None);
        viewer.client = None;
        viewer.device = None;
        let account = Account::default();
        viewer.loaded_input = Some(account.register());
        viewer.load_with_index(&first, &index)?;
        viewer.args.save = first.clone();
        drop(viewer.update(Message::Load));
        ensure!(viewer.error.is_none(), "fixture prepare: {:?}", viewer.error);
        viewer.args.save = destination.clone();
        viewer.diagnostics = false;
        viewer.options_open = false;
        viewer.load_feedback = None;
        ensure!(!viewer.load_owner_blocked(), "authored fixture not at player boundary");
        Ok(Self { files, viewer, account, first, second, second_bytes, second_value, destination })
    }
}
fn open(viewer: &mut Viewer) {
    drop(viewer.update(Message::Options));
    drop(viewer.update(Message::ChooseLoad));
    assert!(viewer.load_picker.is_some());
}
fn edit(viewer: &mut Viewer, path: &Path) {
    let event = viewer.load_picker.as_ref().unwrap().edit_event_for_test(path.to_str().unwrap().into());
    drop(viewer.update(Message::LoadPicker(event)));
}
fn submit(viewer: &mut Viewer) {
    let event = viewer.load_picker.as_ref().unwrap().submit_event_for_test();
    drop(viewer.update(Message::LoadPicker(event)));
}
fn send(host: &mut Host, message: Message) {
    let generation = host.generation();
    drop(host.update(Event::Widget { generation, message }));
}
fn host_open(host: &mut Host) {
    send(host, Message::Options);
    send(host, Message::ChooseLoad);
    assert!(host.load_picker.is_some());
}
fn host_edit(host: &mut Host, path: &Path) {
    let event = host.load_picker.as_ref().unwrap().edit_event_for_test(path.to_str().unwrap().into());
    send(host, Message::LoadPicker(event));
}
fn host_submit(host: &mut Host) {
    let event = host.load_picker.as_ref().unwrap().submit_event_for_test();
    send(host, Message::LoadPicker(event));
}
fn press(key: Key, code: Code, repeat: bool, modifiers: Modifiers) -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        modified_key: key.clone(), key, physical_key: Physical::Code(code),
        location: Location::Standard, modifiers, text: None, repeat,
    })
}
fn character(value: &str, code: Code) -> iced::Event {
    press(Key::Character(value.into()), code, false, Modifiers::NONE)
}
fn release(code: Code) -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyReleased {
        key: Key::Unidentified, modified_key: Key::Unidentified,
        physical_key: Physical::Code(code), location: Location::Standard, modifiers: Modifiers::NONE,
    })
}
fn direct_event(viewer: &mut Viewer, event: iced::Event, status: Status) {
    if let Some(message) = subscription_event(event, status, window::Id::unique()) {
        drop(viewer.update(message));
    }
}
fn host_event(host: &mut Host, generation: u64, event: iced::Event, status: Status) {
    if let Some(message) = subscription_event(event, status, window::Id::unique()) {
        drop(host.update(Event::Widget { generation, message }));
    }
}
fn current(host: &mut Host, event: iced::Event, status: Status) {
    let generation = host.generation();
    host_event(host, generation, event, status);
}
fn camera(viewer: &Viewer) -> (i32, f32, i32) {
    (viewer.floor, viewer.zoom, i32::from(viewer.session.as_ref().unwrap().simulation.vm.world.camera_rotation))
}

#[test]
fn wiring_load_direct_two_files_exact_origin_and_unchanged_save() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let before = ordinal(&h.account);
        for (path, zoom, floor) in [(h.second.clone(), 0.75, 2), (h.first.clone(), 0.5, 1)] {
            open(&mut h.viewer);
            edit(&mut h.viewer, &path);
            submit(&mut h.viewer);
            ensure!(h.viewer.error.is_none(), "{:?}", h.viewer.error);
            assert!(h.viewer.load_picker.is_none());
            assert!(!h.viewer.options_open);
            assert_eq!((h.viewer.zoom, h.viewer.floor), (zoom, floor));
            assert_eq!(origin(&h.viewer).requested_path.as_bytes(), path.to_str().unwrap().as_bytes());
            assert_eq!(origin(&h.viewer).sha256, sims_runtime::exploration::digest(&std::fs::read(&path)?));
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert!(!h.destination.exists());
            assert_eq!(h.viewer.loaded_input_capture()["eligible"], true);
        }
        assert_eq!(ordinal(&h.account), before + 2);
        Ok(())
    })
}

#[test]
fn wiring_load_host_two_files_real_replies_exact_input_and_save() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let before = ordinal(&h.account);
        let mut host = Host::from_viewer_for_test(h.viewer);
        for path in [&h.second, &h.first] {
            host_open(&mut host);
            host_edit(&mut host, path);
            host_submit(&mut host);
            host.settle_for_test()?;
            assert!(host.load_picker.is_none());
            assert!(!host.frontend.options_open && host.in_flight_load.is_none());
        }
        assert_eq!(host.generation(), 2);
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.first.to_str().unwrap());
        assert_eq!(origin(&owner).sha256, sims_runtime::exploration::digest(&std::fs::read(&h.first)?));
        assert_eq!((owner.zoom, owner.floor), (0.5, 1));
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        assert_eq!(ordinal(&h.account), before + 2);
        assert!(!h.destination.exists());
        Ok(())
    })
}

fn rejected_file(h: &Household, kind: &str) -> Result<PathBuf> {
    let path = h.files.0.join(format!("rejected-{kind}.json"));
    match kind {
        "missing" => {}
        "malformed" => std::fs::write(&path, b"{invalid JSON")?,
        "wrong-house" => {
            let mut value = h.second_value.clone();
            value["session"]["source"] = json!("UserData/Houses/House99.iff");
            std::fs::write(&path, serde_json::to_vec(&value)?)?;
        }
        _ => unreachable!(),
    }
    Ok(path)
}

#[test]
fn wiring_load_direct_rejection_retains_owner_path_and_retries() -> Result<()> {
    bounded(|| {
        for kind in ["missing", "malformed", "wrong-house"] {
            let mut h = Household::new()?;
            let path = rejected_file(&h, kind)?;
            let before = saved(&h.viewer)?;
            let observed = origin(&h.viewer).clone();
            let ordinal_before = ordinal(&h.account);
            open(&mut h.viewer);
            edit(&mut h.viewer, &path);
            h.viewer.loaded_input_test_fault = Some("ui_load_common_prepare");
            submit(&mut h.viewer);
            ensure!(h.viewer.error.is_none(), "preflight reached common tail: {:?}", h.viewer.error);
            assert!(h.viewer.options_open);
            assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
            assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), path.to_str().unwrap());
            assert!(h.viewer.load_feedback.as_deref().unwrap().contains("Load was rejected"));
            assert_eq!(saved(&h.viewer)?, before);
            assert_eq!(origin(&h.viewer), &observed);
            assert_eq!(ordinal(&h.account), ordinal_before);
            h.viewer.loaded_input_test_fault = None;
            std::fs::write(&path, &h.second_bytes)?;
            submit(&mut h.viewer);
            assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
            assert_eq!(origin(&h.viewer).requested_path, path.to_str().unwrap());
            assert_eq!(origin(&h.viewer).sha256, sims_runtime::exploration::digest(&h.second_bytes));
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert_eq!(ordinal(&h.account), ordinal_before + 1);
        }
        Ok(())
    })
}

#[test]
fn wiring_load_host_rejection_retains_path_and_real_retry() -> Result<()> {
    bounded(|| {
        for kind in ["missing", "malformed", "wrong-house"] {
            let h = Household::new()?;
            let path = rejected_file(&h, kind)?;
            let before = ordinal(&h.account);
            let before_slot = h.account.inspect_for_test()["accounted_slot"].clone();
            let mut host = Host::from_viewer_for_test(h.viewer);
            host_open(&mut host);
            host_edit(&mut host, &path);
            host_submit(&mut host);
            host.settle_for_test()?;
            assert!(host.frontend.options_open && !host.replacing);
            assert!(!host.load_picker.as_ref().unwrap().pending());
            assert_eq!(host.load_picker.as_ref().unwrap().value(), path.to_str().unwrap());
            assert!(host.frontend.save_load_feedback.as_deref().unwrap().contains("Load was rejected"));
            assert_eq!(ordinal(&h.account), before);
            assert_eq!(h.account.inspect_for_test()["accounted_slot"], before_slot);
            std::fs::write(&path, &h.second_bytes)?;
            host_submit(&mut host);
            host.settle_for_test()?;
            assert!(host.load_picker.is_none());
            let owner = host.recover_for_test();
            assert_eq!(origin(&owner).requested_path, path.to_str().unwrap());
            assert_eq!(origin(&owner).sha256, sims_runtime::exploration::digest(&h.second_bytes));
            assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
            assert_eq!(ordinal(&h.account), before + 1);
        }
        Ok(())
    })
}

#[test]
fn wiring_load_direct_cancel_stale_events_and_current_guards() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let before = saved(&h.viewer)?;
        let count = ordinal(&h.account);
        open(&mut h.viewer);
        let old_submit = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
        drop(h.viewer.update(Message::DialogKey(27)));
        assert!(h.viewer.load_picker.is_none() && h.viewer.options_open);
        drop(h.viewer.update(Message::ChooseLoad));
        drop(h.viewer.update(Message::LoadPicker(old_submit)));
        assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
        let event = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
        h.viewer.options_open = false;
        drop(h.viewer.update(Message::LoadPicker(event)));
        assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
        assert_eq!(saved(&h.viewer)?, before);
        assert_eq!(ordinal(&h.account), count);
        Ok(())
    })
}

#[test]
fn wiring_load_host_pending_cancel_and_unrelated_publication_never_fake_success() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_open(&mut host);
        host_edit(&mut host, &h.second);
        let publication = host.frontend.data.clone();
        let (entered, resume) = host.gate_next_for_test();
        host_submit(&mut host);
        entered.recv_timeout(std::time::Duration::from_secs(5))?;
        assert!(host.load_picker.as_ref().unwrap().pending());
        let cancel = host.load_picker.as_ref().unwrap().cancel_event();
        send(&mut host, Message::LoadPicker(cancel));
        send(&mut host, Message::DialogKey(27));
        drop(host.update(Event::Completion)); // wake without a result
        assert!(Arc::ptr_eq(&publication, &host.frontend.data));
        assert!(host.load_picker.as_ref().unwrap().pending());
        resume.send(())?;
        host.settle_for_test()?;
        assert!(host.load_picker.is_none());
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.second.to_str().unwrap());
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        Ok(())
    })
}

fn pending(picker: &mut load_picker::Picker) -> (PathBuf, load_picker::Ticket) {
    let event = picker.submit_event_for_test();
    match picker.update(event) {
        load_picker::Outcome::Submit { path, ticket } => (path, ticket),
        _ => panic!("sealed picker submission did not produce a request"),
    }
}

#[test]
fn wiring_load_host_stale_foreign_and_queued_guard_rejections_settle_exact_attempt() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let before = ordinal(&h.account);
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_open(&mut host);
        host_edit(&mut host, &h.second);
        let stale_event = host.load_picker.as_ref().unwrap().submit_event_for_test();
        drop(host.update(Event::Widget { generation: u64::MAX, message: Message::LoadPicker(stale_event) }));
        assert!(!host.load_picker.as_ref().unwrap().pending());
        let mut foreign = load_picker::Picker::new(&h.first)?;
        let (_, foreign_ticket) = pending(&mut foreign);
        let (path, ticket) = pending(host.load_picker.as_mut().unwrap());
        host.settle_picker(control_wiring::LoadCompletion { ticket: foreign_ticket, feedback: None });
        assert!(host.load_picker.as_ref().unwrap().pending());
        // A real accepted ordinary publication cannot complete a picker request.
        host.enqueue(host.generation(), Message::FocusLost);
        host.settle_for_test()?;
        assert!(host.load_picker.as_ref().unwrap().pending());
        // Exercise the actual dispatch queue's stale-generation rejection.
        host.replacing = true;
        host.enqueue(u64::MAX, Message::LoadFrom(path, ticket.clone()));
        host.settle_for_test()?;
        assert!(!host.load_picker.as_ref().unwrap().pending());
        assert!(!host.replacing);
        let (_, retry) = pending(host.load_picker.as_mut().unwrap());
        host.settle_picker(control_wiring::LoadCompletion { ticket, feedback: None });
        assert!(host.load_picker.as_ref().unwrap().pending());
        host.replacing = true;
        host.frontend.options_open = false;
        host.enqueue(host.generation(), Message::LoadFrom(h.second.clone(), retry));
        host.settle_for_test()?;
        assert!(!host.load_picker.as_ref().unwrap().pending());
        assert_eq!(ordinal(&h.account), before);
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.first.to_str().unwrap());
        Ok(())
    })
}

#[test]
fn wiring_load_host_diagnostic_load_supersedes_the_previous_picker_seed() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        h.viewer.args.save = h.first.clone();
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_open(&mut host);
        host_edit(&mut host, &h.second);
        host_submit(&mut host);
        host.settle_for_test()?;
        assert_eq!(host.last_load_path.as_deref(), Some(h.second.as_path()));
        send(&mut host, Message::Diagnostics);
        assert!(host.frontend.diagnostics);
        send(&mut host, Message::Load);
        host.settle_for_test()?;
        assert_eq!(host.frontend.load_seed.as_os_str(), h.first.as_os_str());
        send(&mut host, Message::Diagnostics);
        host_open(&mut host);
        assert_eq!(host.load_picker.as_ref().unwrap().value(), h.first.to_str().unwrap());
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.first.to_str().unwrap());
        assert_eq!(owner.args.save.as_os_str(), h.first.as_os_str());
        Ok(())
    })
}

#[test]
fn wiring_load_seed_uses_typed_request_then_configuration_without_display_authority() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        h.viewer.last_load_path = None;
        h.viewer.args.load = Some(h.second.clone());
        // The current origin displays first.json, but args.load is the real
        // configured seed. No Observation string is converted back to a Path.
        open(&mut h.viewer);
        assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), h.second.to_str().unwrap());
        drop(h.viewer.update(Message::DialogKey(27)));
        h.viewer.args.load = None;
        drop(h.viewer.update(Message::ChooseLoad));
        assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), h.destination.to_str().unwrap());
        let host = Host::from_viewer_for_test(h.viewer);
        assert_eq!(host.frontend.load_seed.as_os_str(), h.destination.as_os_str());
        let _ = host.recover_for_test();
        Ok(())
    })
}

#[cfg(unix)]
#[test]
fn wiring_load_non_utf8_seed_refuses_clearly_in_both_adapters() -> Result<()> {
    bounded(|| {
        use std::os::unix::ffi::OsStringExt;
        let mut h = Household::new()?;
        h.viewer.last_load_path = Some(PathBuf::from(std::ffi::OsString::from_vec(b"invalid-\xff.json".to_vec())));
        drop(h.viewer.update(Message::Options));
        drop(h.viewer.update(Message::ChooseLoad));
        assert!(h.viewer.load_picker.is_none());
        assert!(h.viewer.load_feedback.as_deref().unwrap().contains("UTF-8"));
        let mut host = Host::from_viewer_for_test(h.viewer);
        send(&mut host, Message::ChooseLoad);
        assert!(host.load_picker.is_none());
        assert!(host.picker_feedback.as_deref().unwrap().contains("UTF-8"));
        assert_eq!(host.recover_for_test().args.save.as_os_str(), h.destination.as_os_str());
        Ok(())
    })
}

fn candidates() -> [(iced::Event, Code, (i32, f32, i32)); 7] {
    [
        (character(",", Code::Comma), Code::Comma, (1, 0.5, 3)),
        (character(".", Code::Period), Code::Period, (1, 0.5, 1)),
        (character("=", Code::Equal), Code::Equal, (1, 1.0, 0)),
        (press(Key::Character("+".into()), Code::Equal, false, Modifiers::SHIFT), Code::Equal, (1, 1.0, 0)),
        (character("-", Code::Minus), Code::Minus, (1, 0.25, 0)),
        (press(Key::Named(Named::PageDown), Code::PageDown, false, Modifiers::NONE), Code::PageDown, (1, 0.5, 0)),
        (press(Key::Named(Named::PageUp), Code::PageUp, false, Modifiers::NONE), Code::PageUp, (2, 0.5, 0)),
    ]
}

#[test]
fn wiring_camera_direct_subscription_dispatches_all_existing_handlers() -> Result<()> {
    bounded(|| {
        for (event, _, expected) in candidates() {
            let mut h = Household::new()?;
            h.viewer.floor = 2; // PageDown must produce an observable change.
            if expected.0 == 2 { h.viewer.floor = 1; }
            let mut expected = expected;
            if !matches!(&event, iced::Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::PageDown | Named::PageUp), .. })) {
                h.viewer.floor = 1;
            }
            let observed = origin(&h.viewer).clone();
            let count = ordinal(&h.account);
            let rotation = camera(&h.viewer).2;
            direct_event(&mut h.viewer, event, Status::Ignored);
            ensure!(h.viewer.error.is_none(), "{:?}", h.viewer.error);
            expected.2 = (rotation + expected.2) % 4;
            assert_eq!(camera(&h.viewer), expected);
            assert_eq!(origin(&h.viewer), &observed);
            assert_eq!(ordinal(&h.account), count);
        }
        Ok(())
    })
}

#[test]
fn wiring_camera_host_subscription_dispatches_all_existing_handlers_and_keeps_owner() -> Result<()> {
    bounded(|| {
        for (event, _, mut expected) in candidates() {
            let mut h = Household::new()?;
            if matches!(&event, iced::Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::PageDown), .. })) {
                h.viewer.floor = 2;
            }
            let observed = origin(&h.viewer).clone();
            let count = ordinal(&h.account);
            let rotation = camera(&h.viewer).2;
            expected.2 = (rotation + expected.2) % 4;
            let mut host = Host::from_viewer_for_test(h.viewer);
            current(&mut host, event, Status::Ignored);
            host.settle_for_test()?;
            let owner = host.recover_for_test();
            assert_eq!(camera(&owner), expected);
            assert_eq!(origin(&owner), &observed);
            assert_eq!(ordinal(&h.account), count);
            assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        }
        Ok(())
    })
}

#[test]
fn wiring_camera_direct_refused_gestures_latch_and_shared_release_rearms() -> Result<()> {
    bounded(|| {
        for refusal in ["captured", "repeat", "modified", "Options", "Diagnostics", "picker", "modal", "unfocused"] {
            let mut h = Household::new()?;
            let observed = origin(&h.viewer).clone();
            let before = camera(&h.viewer);
            match refusal {
                "Options" => h.viewer.options_open = true,
                "Diagnostics" => h.viewer.diagnostics = true,
                "picker" => open(&mut h.viewer),
                "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                "unfocused" => direct_event(&mut h.viewer, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                _ => {}
            }
            let event = press(Key::Character(".".into()), Code::Period, refusal == "repeat",
                if refusal == "modified" { Modifiers::CTRL } else { Modifiers::NONE });
            direct_event(&mut h.viewer, event, if refusal == "captured" { Status::Captured } else { Status::Ignored });
            h.viewer.options_open = false;
            h.viewer.diagnostics = false;
            h.viewer.load_picker = None; // test-only dismissal of an unsubmitted picker
            h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = None;
            direct_event(&mut h.viewer, iced::Event::Window(window::Event::Focused), Status::Captured);
            direct_event(&mut h.viewer, character(".", Code::Period), Status::Ignored);
            assert_eq!(camera(&h.viewer), before, "refused gesture resumed: {refusal}");
            direct_event(&mut h.viewer, release(Code::Period), Status::Captured);
            direct_event(&mut h.viewer, character(".", Code::Period), Status::Ignored);
            assert_eq!(camera(&h.viewer).2, (before.2 + 1) % 4, "release lost through Space Ignore: {refusal}");
            assert_eq!(origin(&h.viewer), &observed);
        }
        Ok(())
    })
}

#[test]
fn wiring_camera_host_refusals_focus_and_release_are_window_owned() -> Result<()> {
    bounded(|| {
        for refusal in ["captured", "repeat", "modified", "Options", "Diagnostics", "picker", "modal", "unfocused"] {
            let h = Household::new()?;
            let before = camera(&h.viewer);
            let observed = origin(&h.viewer).clone();
            let mut host = Host::from_viewer_for_test(h.viewer);
            match refusal {
                "Options" => send(&mut host, Message::Options),
                "Diagnostics" => send(&mut host, Message::Diagnostics),
                "picker" => host_open(&mut host),
                "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                "unfocused" => current(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                _ => {}
            }
            current(&mut host, press(Key::Character(".".into()), Code::Period, refusal == "repeat",
                if refusal == "modified" { Modifiers::ALT } else { Modifiers::NONE }),
                if refusal == "captured" { Status::Captured } else { Status::Ignored });
            host.frontend.options_open = false;
            host.frontend.diagnostics = false;
            host.load_picker = None;
            Arc::make_mut(&mut host.frontend.data).modal = false;
            current(&mut host, iced::Event::Window(window::Event::Focused), Status::Captured);
            current(&mut host, character(".", Code::Period), Status::Ignored);
            host.settle_for_test()?;
            assert_eq!(host.frontend.data.zoom, before.1);
            current(&mut host, release(Code::Period), Status::Captured);
            current(&mut host, character(".", Code::Period), Status::Ignored);
            host.settle_for_test()?;
            let owner = host.recover_for_test();
            assert_eq!(camera(&owner).2, (before.2 + 1) % 4, "{refusal}");
            assert_eq!(origin(&owner), &observed);
        }
        Ok(())
    })
}

#[test]
fn wiring_camera_host_stale_focus_and_release_cannot_rearm_after_actual_load() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let initial_rotation = camera(&h.viewer).2;
        let mut host = Host::from_viewer_for_test(h.viewer);
        current(&mut host, character(".", Code::Period), Status::Captured);
        host_open(&mut host);
        host_edit(&mut host, &h.second);
        host_submit(&mut host);
        host.settle_for_test()?;
        assert_eq!(host.generation(), 1);
        // The new-generation repeat advances the latch's release fence.
        current(&mut host, character(".", Code::Period), Status::Ignored);
        host_event(&mut host, 0, release(Code::Period), Status::Captured);
        current(&mut host, character(".", Code::Period), Status::Ignored);
        current(&mut host, release(Code::Period), Status::Captured);
        // Stale focus loss always fences; stale gain cannot un-fence it.
        host_event(&mut host, 0, iced::Event::Window(window::Event::Unfocused), Status::Captured);
        host_event(&mut host, 0, iced::Event::Window(window::Event::Focused), Status::Captured);
        current(&mut host, character(".", Code::Period), Status::Ignored);
        current(&mut host, release(Code::Period), Status::Captured);
        current(&mut host, iced::Event::Window(window::Event::Focused), Status::Captured);
        current(&mut host, character(".", Code::Period), Status::Ignored);
        host.settle_for_test()?;
        let owner = host.recover_for_test();
        assert_eq!(camera(&owner).2, (initial_rotation + 1) % 4);
        assert_eq!(origin(&owner).requested_path, h.second.to_str().unwrap());
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        Ok(())
    })
}

#[test]
fn wiring_camera_direct_hold_survives_actual_selected_input_replacement() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        direct_event(&mut h.viewer, character("=", Code::Equal), Status::Captured);
        open(&mut h.viewer);
        edit(&mut h.viewer, &h.second);
        submit(&mut h.viewer);
        assert!(h.viewer.load_picker.is_none());
        direct_event(&mut h.viewer, character("=", Code::Equal), Status::Ignored);
        assert_eq!(h.viewer.zoom, 0.75);
        direct_event(&mut h.viewer, release(Code::Equal), Status::Captured);
        direct_event(&mut h.viewer, character("=", Code::Equal), Status::Ignored);
        assert_eq!(h.viewer.zoom, 1.5);
        assert_eq!(origin(&h.viewer).sha256, sims_runtime::exploration::digest(&h.second_bytes));
        Ok(())
    })
}

#[test]
fn wiring_camera_owner_rechecks_fault_modal_and_options_before_existing_handler() -> Result<()> {
    bounded(|| {
        for boundary in ["fault", "modal", "Options"] {
            let mut h = Household::new()?;
            match boundary {
                "fault" => h.viewer.menu_error = Some("owner menu fault after input mapping".into()),
                "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                _ => {}
            }
            let before = camera(&h.viewer);
            let observed = origin(&h.viewer).clone();
            let mut engine = Engine::new(h.viewer);
            drop(engine.apply(Work {
                message: Message::Rotate, camera_shortcut: true, save_shortcut: None,
                shell: Shell { diagnostics: false, options_open: boundary == "Options", revision: 0 },
                popup: None, gate: None,
            }).map_err(anyhow::Error::msg)?);
            assert_eq!(camera(&engine.viewer), before);
            assert_eq!(origin(&engine.viewer), &observed);
        }
        Ok(())
    })
}

#[test]
fn wiring_load_fatal_native_and_publication_tails_never_report_picker_success() -> Result<()> {
    bounded(|| {
        for fault in ["load_native_prepare", "load_postcommit", "ui_load_common_prepare", "worker_publication"] {
            let mut h = Household::new()?;
            h.viewer.loaded_input_test_fault = Some(fault);
            if fault == "worker_publication" {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_open(&mut host);
                host_edit(&mut host, &h.second);
                host_submit(&mut host);
                assert!(host.settle_for_test().is_err());
                assert!(host.error.is_some());
                assert!(host.load_picker.is_some());
                assert!(!host.load_picker.as_ref().unwrap().pending());
                assert!(host.in_flight_load.is_none());
                let _ = host.recover_for_test();
            } else {
                open(&mut h.viewer);
                edit(&mut h.viewer, &h.second);
                submit(&mut h.viewer);
                assert!(h.viewer.error.as_deref().unwrap().contains(fault));
                assert!(h.viewer.load_picker.is_some());
                assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                assert!(h.viewer.load_feedback.is_none());
                assert!(!h.viewer.picker_owns_input());
                let latch = h.viewer.camera_shortcuts.clone();
                let task = h.viewer.update(Message::Restart);
                assert!(task.units() > 0);
                drop(task); // never run the loader in this recovery control
                assert!(h.viewer.session.is_none() && h.viewer.error.is_none());
                assert!(h.viewer.load_picker.is_some());
                assert_eq!(h.viewer.camera_shortcuts, latch);
            }
        }
        Ok(())
    })
}

mod widgets {
    use super::*;
    use super::super::super::headless_input;
    use iced::{Point, mouse};
    const SIZE: [u32; 2] = [1000, 760];
    fn motion(position: Point) -> iced::Event { iced::Event::Mouse(mouse::Event::CursorMoved { position }) }
    fn pointer(down: bool) -> iced::Event {
        iced::Event::Mouse(if down { mouse::Event::ButtonPressed(mouse::Button::Left) }
            else { mouse::Event::ButtonReleased(mouse::Button::Left) })
    }
    fn enter() -> iced::Event { press(Key::Named(Named::Enter), Code::Enter, false, Modifiers::NONE) }
    fn type_text(value: &str) -> Vec<iced::Event> {
        value.chars().flat_map(|character| {
            let text = character.to_string();
            let key = Key::Character(text.clone().into());
            [
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key: key.clone(), modified_key: key.clone(),
                    physical_key: Physical::Code(Code::KeyA), location: Location::Standard,
                    modifiers: Modifiers::NONE, repeat: false, text: Some(text.into()),
                }),
                iced::Event::Keyboard(keyboard::Event::KeyReleased {
                    key: key.clone(), modified_key: key,
                    physical_key: Physical::Code(Code::KeyA), location: Location::Standard,
                    modifiers: Modifiers::NONE,
                }),
            ]
        }).collect()
    }
    fn label(driver: &mut headless_input::Input, view: Element<'_, Message>, label: &str) -> Result<Point> {
        let observation = driver.element_observation(view, SIZE)?;
        let bounds = observation.texts.iter().find(|t| t.text == label)
            .with_context(|| format!("actual widget label absent: {label}"))?.bounds;
        Ok(bounds.center())
    }
    fn direct_batch(driver: &mut headless_input::Input, viewer: &mut Viewer, events: &[iced::Event]) {
        let messages = driver.application_batch(viewer, events).messages;
        for message in messages { drop(viewer.update(message)); }
    }
    fn host_batch(driver: &mut headless_input::Input, host: &mut Host, events: &[iced::Event]) {
        let generation = host.generation();
        let popup = PopupSource { owner: host.frontend.native_popup.clone(), epoch: host.frontend.native_popup_epoch };
        let messages = driver.element_batch(host.view_messages(), SIZE, events).messages;
        for message in messages {
            drop(host.update(Event::Ui { generation, message, popup: popup.clone() }));
        }
    }

    #[test]
    #[ignore = "opt-in source-authored household and offscreen Iced wgpu; ALL UNRUN"]
    fn wiring_widgets_direct_options_load_entry_submit_and_cancel() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            h.viewer.display_size = SIZE;
            h.viewer.last_load_path = None;
            h.viewer.args.load = Some(PathBuf::new());
            let mut driver = headless_input::Input::new()?;
            drop(h.viewer.update(Message::Options));
            let load = label(&mut driver, h.viewer.view(), "Load")?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(load), pointer(true), pointer(false)]);
            assert!(h.viewer.load_picker.is_some());
            let batch = driver.application_batch(&h.viewer, &[]);
            let [x, y, w, hgt] = batch.owners.focused.first().context("picker text field absent")?.0;
            direct_batch(&mut driver, &mut h.viewer, &[motion(Point::new(x+w/2., y+hgt/2.)), pointer(true), pointer(false)]);
            direct_batch(&mut driver, &mut h.viewer, &type_text(h.second.to_str().unwrap()));
            assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), h.second.to_str().unwrap());
            direct_batch(&mut driver, &mut h.viewer, &[enter()]);
            assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
            assert_eq!(origin(&h.viewer).requested_path, h.second.to_str().unwrap());
            drop(h.viewer.update(Message::Options));
            let load = label(&mut driver, h.viewer.view(), "Load")?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(load), pointer(true), pointer(false)]);
            let cancel = label(&mut driver, h.viewer.view(), "Cancel")?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(cancel), pointer(true), pointer(false)]);
            assert!(h.viewer.load_picker.is_none());
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            Ok(())
        })
    }

    #[test]
    #[ignore = "opt-in source-authored household and offscreen Iced wgpu; ALL UNRUN"]
    fn wiring_widgets_host_options_load_real_submission_rejection_and_cancel() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            h.viewer.last_load_path = None;
            h.viewer.args.load = Some(PathBuf::new());
            let mut host = Host::from_viewer_for_test(h.viewer);
            let mut driver = headless_input::Input::new()?;
            send(&mut host, Message::Options);
            let load = label(&mut driver, host.view_messages(), "Load")?;
            host_batch(&mut driver, &mut host, &[motion(load), pointer(true), pointer(false)]);
            assert!(host.load_picker.is_some());
            let batch = driver.element_batch(host.view_messages(), SIZE, &[]);
            let [x, y, w, hgt] = batch.owners.focused.first().context("Host picker field absent")?.0;
            host_batch(&mut driver, &mut host, &[motion(Point::new(x+w/2., y+hgt/2.)), pointer(true), pointer(false)]);
            let missing = h.files.0.join("missing typed path.json");
            host_batch(&mut driver, &mut host, &type_text(missing.to_str().unwrap()));
            let submit = label(&mut driver, host.view_messages(), "Load selected file")?;
            host_batch(&mut driver, &mut host, &[motion(submit), pointer(true), pointer(false)]);
            host.settle_for_test()?;
            assert_eq!(host.load_picker.as_ref().unwrap().value(), missing.to_str().unwrap());
            assert!(!host.load_picker.as_ref().unwrap().pending());
            let view = driver.element_observation(host.view_messages(), SIZE)?;
            assert!(view.texts.iter().any(|t| t.text.contains("Load was rejected")));
            let cancel = label(&mut driver, host.view_messages(), "Cancel")?;
            host_batch(&mut driver, &mut host, &[motion(cancel), pointer(true), pointer(false)]);
            assert!(host.load_picker.is_none());
            let owner = host.recover_for_test();
            assert_eq!(origin(&owner).requested_path, h.first.to_str().unwrap());
            assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
            Ok(())
        })
    }

    // Opt-in real widget dispatch for both application adapters. ALL UNRUN.
    fn copy_action_position(driver: &mut headless_input::Input, view: Element<'_, Message>) -> Result<Point> {
        let observation = driver.element_observation(view, SIZE)?;
        let copy = observation.texts.iter().find(|t| t.text == "Save a Copy…").context("Options copy action absent")?.bounds;
        let load = observation.texts.iter().find(|t| t.text == "Load").context("existing Load action absent")?.bounds;
        assert!(copy.y + copy.height <= load.y, "Copy overlaps the existing Load row");
        Ok(copy.center())
    }
    #[test]
    #[ignore = "opt-in authored household and headless Iced wgpu; ALL UNRUN"]
    fn save_copy_widgets_direct_path_feedback_retry_success_and_cancel() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            h.viewer.display_size = SIZE;
            h.viewer.last_copy_path = Some(PathBuf::new());
            std::fs::write(&h.destination, b"Continue stays")?;
            let output = h.files.0.join("家族 widget copy.json");
            std::fs::create_dir(&output)?;
            let mut driver = headless_input::Input::new()?;
            drop(h.viewer.update(Message::Options));
            let action = copy_action_position(&mut driver, h.viewer.view())?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(action), pointer(true), pointer(false)]);
            assert!(h.viewer.copy_picker());
            let batch = driver.application_batch(&h.viewer, &[]);
            let [x, y, w, height] = batch.owners.focused.first().context("Copy path editor absent")?.0;
            direct_batch(&mut driver, &mut h.viewer, &[motion(Point::new(x+w/2., y+height/2.)), pointer(true), pointer(false)]);
            direct_batch(&mut driver, &mut h.viewer, &type_text(output.to_str().unwrap()));
            assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), output.to_str().unwrap());
            direct_batch(&mut driver, &mut h.viewer, &[enter()]);
            assert!(h.viewer.error.is_none() && h.viewer.copy_picker());
            let shown = driver.element_observation(h.viewer.view(), SIZE)?;
            assert!(shown.texts.iter().any(|t| t.text.contains("retry Save a Copy…")));
            assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
            std::fs::remove_dir(&output)?;
            direct_batch(&mut driver, &mut h.viewer, &[enter()]);
            assert!(h.viewer.load_picker.is_none());
            copy_bytes(&output, &h.viewer)?;
            let shown = driver.element_observation(h.viewer.view(), SIZE)?;
            assert!(shown.texts.iter().any(|t| t.text.contains("Saved copy to") && t.text.contains(output.to_str().unwrap())));
            let action = copy_action_position(&mut driver, h.viewer.view())?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(action), pointer(true), pointer(false)]);
            let cancel = label(&mut driver, h.viewer.view(), "Cancel")?;
            direct_batch(&mut driver, &mut h.viewer, &[motion(cancel), pointer(true), pointer(false)]);
            assert!(h.viewer.load_picker.is_none() && h.viewer.options_open);
            assert_eq!(std::fs::read(&h.destination)?, b"Continue stays");
            Ok(())
        })
    }
    #[test]
    #[ignore = "opt-in authored household and headless Iced wgpu; ALL UNRUN"]
    fn save_copy_widgets_host_path_pending_feedback_retry_success_and_cancel() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            h.viewer.last_copy_path = Some(PathBuf::new());
            let output = h.files.0.join("家族 responsive widget copy.json");
            std::fs::create_dir(&output)?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            // Host's seed is typed shell state; no Published field is added.
            host.last_copy_path = Some(PathBuf::new());
            let mut driver = headless_input::Input::new()?;
            send(&mut host, Message::Options);
            let action = copy_action_position(&mut driver, host.view_messages())?;
            host_batch(&mut driver, &mut host, &[motion(action), pointer(true), pointer(false)]);
            assert!(host.copy_picker());
            let batch = driver.element_batch(host.view_messages(), SIZE, &[]);
            let [x, y, w, height] = batch.owners.focused.first().context("Host Copy editor absent")?.0;
            host_batch(&mut driver, &mut host, &[motion(Point::new(x+w/2., y+height/2.)), pointer(true), pointer(false)]);
            host_batch(&mut driver, &mut host, &type_text(output.to_str().unwrap()));
            let (entered, resume) = host.gate_next_for_test();
            host_batch(&mut driver, &mut host, &[enter()]);
            entered.recv_timeout(std::time::Duration::from_secs(5))?;
            let shown = driver.element_observation(host.view_messages(), SIZE)?;
            assert!(shown.texts.iter().any(|t| t.text == "Saving copy…"));
            let cancel = label(&mut driver, host.view_messages(), "Cancel")?;
            host_batch(&mut driver, &mut host, &[motion(cancel), pointer(true), pointer(false)]);
            assert!(host.load_picker.as_ref().unwrap().pending());
            resume.send(())?;
            host.settle_for_test()?;
            let shown = driver.element_observation(host.view_messages(), SIZE)?;
            assert!(shown.texts.iter().any(|t| t.text.contains("retry Save a Copy…")));
            assert_eq!(host.load_picker.as_ref().unwrap().value(), output.to_str().unwrap());
            std::fs::remove_dir(&output)?;
            // Click the submit button (the last matching label), after Cancel
            // moved focus. This goes through actual retained Iced controls.
            let shown = driver.element_observation(host.view_messages(), SIZE)?;
            let submit = shown.texts.iter().rev().find(|t| t.text == "Save a Copy…").unwrap().bounds.center();
            host_batch(&mut driver, &mut host, &[motion(submit), pointer(true), pointer(false)]);
            host.settle_for_test()?;
            assert!(host.load_picker.is_none() && output.is_file());
            let shown = driver.element_observation(host.view_messages(), SIZE)?;
            assert!(shown.texts.iter().any(|t| t.text.contains("Saved copy to")));
            let action = copy_action_position(&mut driver, host.view_messages())?;
            host_batch(&mut driver, &mut host, &[motion(action), pointer(true), pointer(false)]);
            let cancel = label(&mut driver, host.view_messages(), "Cancel")?;
            host_batch(&mut driver, &mut host, &[motion(cancel), pointer(true), pointer(false)]);
            assert!(host.load_picker.is_none());
            let owner = host.recover_for_test();
            copy_bytes(&output, &owner)?;
            Ok(())
        })
    }

}

#[test]
fn wiring_load_host_close_drains_actual_consumption_before_settling_picker() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_open(&mut host);
        host_edit(&mut host, &h.second);
        let (entered, resume) = host.gate_next_for_test();
        host_submit(&mut host);
        entered.recv_timeout(std::time::Duration::from_secs(5))?;
        drop(host.update(Event::Close(window::Id::unique())));
        assert!(host.closing.is_some() && !host.close_sent);
        assert!(host.load_picker.as_ref().unwrap().pending());
        resume.send(())?;
        host.settle_for_test()?;
        assert!(host.load_picker.is_none());
        drop(host.update(Event::Completion));
        assert!(host.close_sent);
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.second.to_str().unwrap());
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        Ok(())
    })
}

#[test]
fn wiring_load_direct_foreign_attempt_cannot_consume_or_close_current_picker() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let observed = origin(&h.viewer).clone();
        let count = ordinal(&h.account);
        open(&mut h.viewer);
        let (_, ticket) = pending(h.viewer.load_picker.as_mut().unwrap());
        let mut foreign = load_picker::Picker::new(&h.second)?;
        let (path, foreign_ticket) = pending(&mut foreign);
        drop(h.viewer.update(Message::LoadFrom(path, foreign_ticket)));
        assert!(h.viewer.load_picker.as_ref().unwrap().pending_ticket(&ticket));
        let cancel = h.viewer.load_picker.as_ref().unwrap().cancel_event();
        drop(h.viewer.update(Message::LoadPicker(cancel)));
        drop(h.viewer.update(Message::DialogKey(27)));
        assert!(h.viewer.load_picker.as_ref().unwrap().pending_ticket(&ticket));
        assert_eq!(origin(&h.viewer), &observed);
        assert_eq!(ordinal(&h.account), count);
        h.viewer.finish_picker_load(control_wiring::LoadCompletion { ticket, feedback: Some("guarded test rejection".into()) });
        let cancel = h.viewer.load_picker.as_ref().unwrap().cancel_event();
        drop(h.viewer.update(Message::LoadPicker(cancel)));
        assert!(h.viewer.load_picker.is_none());
        Ok(())
    })
}

#[test]
fn wiring_camera_overflow_remains_fail_closed_through_real_load_in_both_adapters() -> Result<()> {
    bounded(|| {
        let codes = [Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE,
            Code::KeyF, Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ,
            Code::KeyK, Code::KeyL, Code::KeyM, Code::KeyN, Code::KeyO,
            Code::KeyP, Code::KeyQ];
        let mut h = Household::new()?;
        open(&mut h.viewer);
        for code in codes {
            direct_event(&mut h.viewer, character(".", code), Status::Captured);
        }
        edit(&mut h.viewer, &h.second);
        submit(&mut h.viewer);
        assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
        let before = camera(&h.viewer);
        for code in codes { direct_event(&mut h.viewer, release(code), Status::Captured); }
        direct_event(&mut h.viewer, character(".", Code::Period), Status::Ignored);
        assert_eq!(camera(&h.viewer), before);
        let h = Household::new()?;
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_open(&mut host);
        for code in codes { current(&mut host, character(".", code), Status::Captured); }
        host_edit(&mut host, &h.second);
        host_submit(&mut host);
        host.settle_for_test()?;
        for code in codes { current(&mut host, release(code), Status::Captured); }
        current(&mut host, character(".", Code::Period), Status::Ignored);
        host.settle_for_test()?;
        let owner = host.recover_for_test();
        assert_eq!(camera(&owner), before);
        assert_eq!(origin(&owner).requested_path, h.second.to_str().unwrap());
        Ok(())
    })
}

#[test]
fn wiring_load_captured_escape_cancels_only_current_picker_with_native_precedence() -> Result<()> {
    bounded(|| {
        let escape = || press(Key::Named(Named::Escape), Code::Escape, false, Modifiers::NONE);
        let mut h = Household::new()?;
        let before = saved(&h.viewer)?;
        direct_event(&mut h.viewer, escape(), Status::Captured);
        assert!(!h.viewer.options_open);
        assert_eq!(saved(&h.viewer)?, before);
        open(&mut h.viewer);
        h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999);
        direct_event(&mut h.viewer, escape(), Status::Captured);
        assert!(h.viewer.load_picker.is_some());
        assert_eq!(h.viewer.session.as_ref().unwrap().simulation.vm.world.dialogs.modal, Some(999));
        h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = None;
        direct_event(&mut h.viewer, escape(), Status::Captured);
        assert!(h.viewer.load_picker.is_none() && h.viewer.options_open);
        let mut host = Host::from_viewer_for_test(h.viewer);
        send(&mut host, Message::ChooseLoad);
        Arc::make_mut(&mut host.frontend.data).modal = true;
        current(&mut host, escape(), Status::Captured);
        assert!(host.load_picker.is_some());
        Arc::make_mut(&mut host.frontend.data).modal = false;
        current(&mut host, escape(), Status::Captured);
        assert!(host.load_picker.is_none() && host.frontend.options_open);
        let owner = host.recover_for_test();
        assert_eq!(saved(&owner)?, before);
        Ok(())
    })
}

// Concrete Save a Copy adapter controls. ALL UNRUN, including compilation.
fn copy_open(viewer: &mut Viewer) {
    if !viewer.options_open { drop(viewer.update(Message::Options)); }
    drop(viewer.update(Message::ChooseSaveCopy));
    assert_eq!(viewer.load_picker.as_ref().unwrap().purpose(), load_picker::Purpose::Copy);
}
fn host_copy_open(host: &mut Host) {
    if !host.frontend.options_open { send(host, Message::Options); }
    send(host, Message::ChooseSaveCopy);
    assert_eq!(host.load_picker.as_ref().unwrap().purpose(), load_picker::Purpose::Copy);
}
fn copy_bytes(path: &Path, viewer: &Viewer) -> Result<()> {
    let actual: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    assert_eq!(actual, saved(viewer)?);
    assert_eq!(actual["session"]["version"], 2);
    assert!(actual["session"]["managed"].is_object());
    Ok(())
}
fn transport(viewer: &Viewer) -> (bool, f32, Option<Instant>, Option<(Instant, u32)>) {
    (viewer.playing, viewer.accumulator, viewer.last, viewer.clock_origin)
}

#[test]
fn save_copy_direct_real_managed_bytes_preserve_save_input_continue_and_transport() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        std::fs::write(&h.destination, b"canonical and Continue stay here")?;
        let canonical = std::fs::read(&h.destination)?;
        let input = std::fs::read(&h.first)?;
        let observed = origin(&h.viewer).clone();
        let count = ordinal(&h.account);
        let state = transport(&h.viewer);
        let owner = h.viewer.copy_owner.clone();
        let output = h.files.0.join("copies/家族 copy with spaces.json");
        copy_open(&mut h.viewer);
        edit(&mut h.viewer, &output);
        submit(&mut h.viewer);
        assert!(h.viewer.error.is_none(), "{:?}", h.viewer.error);
        assert!(h.viewer.load_picker.is_none());
        assert!(h.viewer.copy_completion.as_ref().unwrap().feedback.is_none());
        assert!(h.viewer.feedback_status().contains(&output.display().to_string()));
        assert!(h.viewer.feedback_status().contains("Save and Continue destinations are unchanged"));
        copy_bytes(&output, &h.viewer)?;
        assert_eq!(std::fs::read(&h.destination)?, canonical);
        assert_eq!(std::fs::read(&h.first)?, input);
        assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
        assert_eq!(origin(&h.viewer), &observed);
        assert_eq!(ordinal(&h.account), count);
        assert!(h.viewer.copy_owner.same(&owner));
        assert_eq!(transport(&h.viewer), state);
        // Ordinary Save still writes its configured destination afterward.
        drop(h.viewer.update(Message::Save));
        copy_bytes(&h.destination, &h.viewer)?;
        assert_eq!(origin(&h.viewer), &observed);
        Ok(())
    })
}

#[test]
fn save_copy_direct_recovers_write_failure_retries_new_path_and_keeps_unwritable_save() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let blocker = h.files.0.join("blocked canonical parent");
        std::fs::write(&blocker, b"leave this alone")?;
        h.viewer.args.save = blocker.join("canonical.json");
        let failed = h.files.0.join("copy is a directory.json");
        std::fs::create_dir(&failed)?;
        let sentinel = failed.join("sentinel");
        std::fs::write(&sentinel, b"existing directory content")?;
        let observed = origin(&h.viewer).clone();
        let count = ordinal(&h.account);
        let before = saved(&h.viewer)?;
        let state = transport(&h.viewer);
        copy_open(&mut h.viewer);
        edit(&mut h.viewer, &failed);
        submit(&mut h.viewer);
        assert!(h.viewer.error.is_none());
        let picker = h.viewer.load_picker.as_ref().unwrap();
        assert!(!picker.pending());
        assert_eq!(picker.value(), failed.to_str().unwrap());
        assert!(h.viewer.feedback_status().contains("Write completion was not confirmed"));
        assert!(h.viewer.feedback_status().contains("retry Save a Copy…"));
        assert_eq!(saved(&h.viewer)?, before);
        assert_eq!(transport(&h.viewer), state);
        assert_eq!(ordinal(&h.account), count);
        assert_eq!(origin(&h.viewer), &observed);
        assert_eq!(std::fs::read(&sentinel)?, b"existing directory content");
        let output = h.files.0.join("new copy α.json");
        edit(&mut h.viewer, &output);
        submit(&mut h.viewer);
        assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
        copy_bytes(&output, &h.viewer)?;
        assert_eq!(std::fs::read(&blocker)?, b"leave this alone");
        assert!(!h.viewer.args.save.exists());
        assert_eq!(origin(&h.viewer), &observed);
        assert_eq!(transport(&h.viewer), state);
        Ok(())
    })
}

#[test]
fn save_copy_host_healthy_session_write_failure_and_real_retry_preserve_ownership() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        std::fs::write(&h.destination, b"canonical Continue bytes")?;
        let input = std::fs::read(&h.first)?;
        let observed = origin(&h.viewer).clone();
        let state = transport(&h.viewer);
        let count = ordinal(&h.account);
        let before = saved(&h.viewer)?;
        let failed = h.files.0.join("copy directory.json");
        std::fs::create_dir(&failed)?;
        let output = h.files.0.join("new directory/家族 host copy.json");
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_copy_open(&mut host);
        host_edit(&mut host, &failed);
        host_submit(&mut host);
        host.settle_for_test()?;
        assert!(host.error.is_none() && host.frontend.error.is_none());
        assert_eq!(host.load_picker.as_ref().unwrap().value(), failed.to_str().unwrap());
        assert!(!host.load_picker.as_ref().unwrap().pending());
        assert!(host.frontend.status.contains("retry Save a Copy…"));
        assert_eq!(host.generation(), 0);
        host_edit(&mut host, &output);
        host_submit(&mut host);
        host.settle_for_test()?;
        assert!(host.load_picker.is_none());
        assert!(host.frontend.options_open); // Copy does not install a new owner.
        assert!(host.frontend.status.contains(&output.display().to_string()));
        let owner = host.recover_for_test();
        copy_bytes(&output, &owner)?;
        assert_eq!(saved(&owner)?, before);
        assert_eq!(origin(&owner), &observed);
        assert_eq!(ordinal(&h.account), count);
        assert_eq!(transport(&owner), state);
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        assert_eq!(std::fs::read(&h.destination)?, b"canonical Continue bytes");
        assert_eq!(std::fs::read(&h.first)?, input);
        Ok(())
    })
}

#[test]
fn save_copy_host_pending_cancel_unrelated_wake_and_close_wait_for_real_write() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let output = h.files.0.join("held copy.json");
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_copy_open(&mut host);
        host_edit(&mut host, &output);
        let publication = host.frontend.data.clone();
        let (entered, resume) = host.gate_next_for_test();
        host_submit(&mut host);
        entered.recv_timeout(std::time::Duration::from_secs(5))?;
        assert!(!output.exists());
        let cancel = host.load_picker.as_ref().unwrap().cancel_event();
        send(&mut host, Message::LoadPicker(cancel));
        send(&mut host, Message::PickerEscape);
        drop(host.update(Event::Completion));
        assert!(host.load_picker.as_ref().unwrap().pending());
        assert!(host.in_flight_copy.is_some());
        assert!(Arc::ptr_eq(&publication, &host.frontend.data));
        let window = window::Id::unique();
        drop(host.update(Event::Close(window)));
        assert!(host.closing.is_some() && !host.close_sent);
        assert!(!output.exists());
        resume.send(())?;
        host.settle_for_test()?;
        assert!(output.is_file());
        assert!(host.load_picker.is_none());
        let close = host.close_when_idle();
        assert!(close.units() > 0 && host.close_sent);
        drop(close);
        let owner = host.recover_for_test();
        copy_bytes(&output, &owner)?;
        Ok(())
    })
}

#[test]
fn save_copy_direct_current_guards_and_focus_invalidate_old_submit() -> Result<()> {
    bounded(|| {
        for guard in ["Options", "diagnostics", "confirmation", "modal", "held", "pending", "menu", "fault", "focus", "replacement"] {
            let mut h = Household::new()?;
            let output = h.files.0.join("guarded copy.json");
            copy_open(&mut h.viewer);
            edit(&mut h.viewer, &output);
            let event = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
            match guard {
                "Options" => h.viewer.options_open = false,
                "diagnostics" => h.viewer.diagnostics = true,
                "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation {
                    choice: neighborhood_usability::Choice::BuyPreview, revision: 1,
                }),
                "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                "held" => h.viewer.selected = Some(7),
                "pending" => h.viewer.viewport_enter_requested = true,
                "menu" => h.viewer.menu_error = Some("current owner menu fault".into()),
                "fault" => h.viewer.session.as_mut().unwrap().simulation.vm.threads.values_mut().next().unwrap().status = sims_vm::Status::Faulted(sims_vm::Fault { message: "current fault".into(), stack: vec![] }),
                "focus" => {
                    direct_event(&mut h.viewer, iced::Event::Window(window::Event::Unfocused), Status::Captured);
                    direct_event(&mut h.viewer, iced::Event::Window(window::Event::Focused), Status::Captured);
                }
                "replacement" => h.viewer.replace_session(None),
                _ => unreachable!(),
            }
            drop(h.viewer.update(Message::LoadPicker(event)));
            assert!(!output.exists(), "guard failed: {guard}");
            assert!(!h.viewer.copy_completion.as_ref().is_some_and(|c| c.feedback.is_none()), "false success: {guard}");
        }
        Ok(())
    })
}

#[test]
fn save_copy_host_generation_effect_replay_foreign_and_current_queue_guards() -> Result<()> {
    bounded(|| {
        for guard in ["generation", "effect", "closing", "replacing", "modal", "pending", "held", "fault", "confirmation", "focus"] {
            let h = Household::new()?;
            let output = h.files.0.join("guarded host copy.json");
            let mut host = Host::from_viewer_for_test(h.viewer);
            host_copy_open(&mut host);
            host_edit(&mut host, &output);
            let event = host.load_picker.as_ref().unwrap().submit_event_for_test();
            if guard == "effect" {
                drop(host.update(Event::Effect { generation: host.generation(), message: Message::LoadPicker(event) }));
            } else if guard == "generation" {
                drop(host.update(Event::Widget { generation: u64::MAX, message: Message::LoadPicker(event) }));
            } else {
                match guard {
                    "closing" => host.closing = Some(window::Id::unique()),
                    "replacing" => host.replacing = true,
                    "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                    "pending" | "fault" => Arc::make_mut(&mut host.frontend.data).load_picker_blocked = true,
                    "held" => Arc::make_mut(&mut host.frontend.data).player_shortcuts_blocked = true,
                    "confirmation" => host.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                    "focus" => current(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                    _ => unreachable!(),
                }
                send(&mut host, Message::LoadPicker(event));
            }
            assert!(!output.exists(), "guard failed: {guard}");
            assert!(!host.load_picker.as_ref().unwrap().pending());
        }
        // Exercise queue checks separately from UI checks, with genuine tickets.
        let h = Household::new()?;
        let output = h.files.0.join("stale queued copy.json");
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_copy_open(&mut host);
        host_edit(&mut host, &output);
        let (path, ticket) = pending(host.load_picker.as_mut().unwrap());
        let mut foreign = load_picker::Picker::new(&path)?;
        let (_, load_ticket) = pending(&mut foreign);
        host.settle_copy(control_wiring::CopyCompletion { ticket: load_ticket.clone(), feedback: None });
        host.settle_picker(control_wiring::LoadCompletion { ticket: ticket.clone(), feedback: None });
        let mut other_copy = load_picker::Picker::new_copy(&path, host.copy_owner.clone())?;
        let (_, foreign_copy) = pending(&mut other_copy);
        host.settle_copy(control_wiring::CopyCompletion { ticket: foreign_copy, feedback: None });
        assert!(host.load_picker.as_ref().unwrap().pending());
        host.enqueue(host.generation(), Message::FocusLost);
        host.settle_for_test()?; // A real ordinary worker reply is not Copy completion.
        assert!(host.load_picker.as_ref().unwrap().pending());
        assert!(!output.exists());
        host.enqueue(u64::MAX, Message::CopyTo(path, ticket.clone()));
        host.settle_for_test()?;
        assert!(!output.exists());
        assert!(!host.load_picker.as_ref().unwrap().pending());
        let (path, retry) = pending(host.load_picker.as_mut().unwrap());
        host.settle_copy(control_wiring::CopyCompletion { ticket, feedback: None });
        assert!(host.load_picker.as_ref().unwrap().pending());
        // Public UI/effect CopyTo is never an enqueue route, even with a ticket.
        send(&mut host, Message::CopyTo(path.clone(), retry.clone()));
        drop(host.update(Event::Effect { generation: host.generation(), message: Message::CopyTo(path.clone(), retry.clone()) }));
        assert!(!output.exists());
        host.frontend.options_open = false;
        host.enqueue(host.generation(), Message::CopyTo(path, retry));
        host.settle_for_test()?;
        assert!(!output.exists());
        assert!(!host.load_picker.as_ref().unwrap().pending());
        Ok(())
    })
}

#[test]
fn save_copy_owner_rechecks_current_fault_modal_held_and_options_after_shell_acceptance() -> Result<()> {
    bounded(|| {
        for guard in ["modal", "held", "pending", "menu", "fault", "confirmation", "Options"] {
            let mut h = Household::new()?;
            let output = h.files.0.join("current owner guarded.json");
            let mut picker = load_picker::Picker::new_copy(&output, h.viewer.copy_owner.clone())?;
            let (path, ticket) = pending(&mut picker);
            match guard {
                "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                "held" => h.viewer.selected = Some(7),
                "menu" => h.viewer.menu_error = Some("current menu failure".into()),
                "pending" => h.viewer.viewport_enter_requested = true,
                "fault" => h.viewer.session.as_mut().unwrap().simulation.vm.threads.values_mut().next().unwrap().status = sims_vm::Status::Faulted(sims_vm::Fault { message: "current worker fault".into(), stack: vec![] }),
                "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                _ => {}
            }
            let mut engine = Engine::new(h.viewer);
            let reply = engine.apply(Work { message: Message::CopyTo(path, ticket), camera_shortcut: false, save_shortcut: None,
                shell: Shell { diagnostics: false, options_open: guard != "Options", revision: 0 }, popup: None, gate: None,
            }).map_err(anyhow::Error::msg)?;
            assert!(!output.exists());
            assert!(reply.copy_completion.as_ref().unwrap().feedback.is_some());
        }
        Ok(())
    })
}

#[test]
fn save_copy_capture_and_post_write_prepare_failures_never_settle_success() -> Result<()> {
    bounded(|| {
        for failure in ["capture", "copy_common_prepare"] {
            let mut h = Household::new()?;
            let output = h.files.0.join("failed copy.json");
            copy_open(&mut h.viewer);
            edit(&mut h.viewer, &output);
            if failure == "capture" { h.viewer.client = None; }
            else { h.viewer.loaded_input_test_fault = Some("copy_common_prepare"); }
            submit(&mut h.viewer);
            assert!(h.viewer.error.is_some());
            assert!(h.viewer.save_feedback.is_none());
            assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
            assert!(h.viewer.copy_completion.as_ref().unwrap().feedback.is_some());
            assert_eq!(output.exists(), failure != "capture");
        }
        let mut h = Household::new()?;
        let output = h.files.0.join("written but publication failed.json");
        h.viewer.loaded_input_test_fault = Some("worker_publication");
        let mut host = Host::from_viewer_for_test(h.viewer);
        host_copy_open(&mut host);
        host_edit(&mut host, &output);
        host_submit(&mut host);
        assert!(host.settle_for_test().is_err());
        assert!(output.is_file());
        assert!(host.error.is_some());
        assert!(host.load_picker.is_some());
        assert!(!host.load_picker.as_ref().unwrap().pending());
        Ok(())
    })
}

#[cfg(unix)]
#[test]
fn save_copy_both_adapters_refuse_input_canonical_and_existing_parent_aliases() -> Result<()> {
    bounded(|| {
        use std::os::unix::fs::symlink;
        for responsive in [false, true] {
            let mut h = Household::new()?;
            std::fs::write(&h.destination, b"canonical stays")?;
            let original = std::fs::read(&h.first)?;
            let alias = h.files.0.join("parent alias");
            symlink(&h.files.0, &alias)?;
            let paths = [h.first.clone(), h.destination.clone(), alias.join(h.first.file_name().unwrap()), alias.join(h.destination.file_name().unwrap())];
            if responsive {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_copy_open(&mut host);
                for path in paths {
                    host_edit(&mut host, &path);
                    host_submit(&mut host);
                    host.settle_for_test()?;
                    assert!(host.frontend.status.contains("Choose another copy path"));
                    assert!(!host.load_picker.as_ref().unwrap().pending());
                }
            } else {
                copy_open(&mut h.viewer);
                for path in paths {
                    edit(&mut h.viewer, &path);
                    submit(&mut h.viewer);
                    assert!(h.viewer.error.is_none());
                    assert!(h.viewer.feedback_status().contains("Choose another copy path"));
                    assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                }
            }
            assert_eq!(std::fs::read(&h.first)?, original);
            assert_eq!(std::fs::read(&h.destination)?, b"canonical stays");
        }
        Ok(())
    })
}

#[test]
fn save_copy_direct_idle_captured_escape_stale_event_and_mixed_load_do_not_write() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let output = h.files.0.join("cancelled copy.json");
        copy_open(&mut h.viewer);
        edit(&mut h.viewer, &output);
        let stale = h.viewer.load_picker.as_ref().unwrap().submit_event_for_test();
        direct_event(&mut h.viewer, press(Key::Named(Named::Escape), Code::Escape, false, Modifiers::NONE), Status::Captured);
        assert!(h.viewer.load_picker.is_none());
        copy_open(&mut h.viewer);
        drop(h.viewer.update(Message::LoadPicker(stale)));
        assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
        let (_, ticket) = pending(h.viewer.load_picker.as_mut().unwrap());
        let observed = origin(&h.viewer).clone();
        drop(h.viewer.update(Message::LoadFrom(output.clone(), ticket.clone())));
        drop(h.viewer.update(Message::CopyTo(output.clone(), ticket)));
        assert!(!output.exists());
        assert_eq!(origin(&h.viewer), &observed);
        assert!(h.viewer.load_picker.as_ref().unwrap().pending());
        Ok(())
    })
}

#[test]
fn save_copy_worker_cloned_ticket_cannot_replay_or_target_a_different_owner() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let output = h.files.0.join("one shot.json");
        let mut picker = load_picker::Picker::new_copy(&output, h.viewer.copy_owner.clone())?;
        let (path, ticket) = pending(&mut picker);
        let work = |path: PathBuf, ticket: load_picker::Ticket| Work { message: Message::CopyTo(path, ticket),
            camera_shortcut: false, save_shortcut: None, shell: Shell { diagnostics: false, options_open: true, revision: 0 }, popup: None, gate: None };
        let mut engine = Engine::new(h.viewer);
        let first = engine.apply(work(path.clone(), ticket.clone())).map_err(anyhow::Error::msg)?;
        assert!(first.copy_completion.unwrap().feedback.is_none());
        copy_bytes(&output, &engine.viewer)?;
        std::fs::write(&output, b"sentinel after first write")?;
        let replay = engine.apply(work(path.clone(), ticket.clone())).map_err(anyhow::Error::msg)?;
        assert!(replay.copy_completion.unwrap().feedback.is_some());
        assert_eq!(std::fs::read(&output)?, b"sentinel after first write");
        let another = Household::new()?;
        let mut engine = Engine::new(another.viewer);
        let foreign = engine.apply(work(path, ticket)).map_err(anyhow::Error::msg)?;
        assert!(foreign.copy_completion.unwrap().feedback.is_some());
        assert_eq!(std::fs::read(&output)?, b"sentinel after first write");
        Ok(())
    })
}

#[cfg(unix)]
#[test]
fn save_copy_retained_input_descriptor_remains_protected_after_file_rename() -> Result<()> {
    bounded(|| {
        let mut h = Household::new()?;
        let renamed = h.files.0.join("renamed original input.json");
        std::fs::rename(&h.first, &renamed)?;
        let original = std::fs::read(&renamed)?;
        copy_open(&mut h.viewer);
        edit(&mut h.viewer, &renamed);
        submit(&mut h.viewer);
        assert!(h.viewer.error.is_none());
        assert!(h.viewer.feedback_status().contains("aliases the loaded input"));
        assert_eq!(std::fs::read(&renamed)?, original);
        assert_eq!(origin(&h.viewer).requested_path, h.first.to_str().unwrap());
        Ok(())
    })
}

#[test]
fn save_copy_output_failure_cancel_leaves_actual_transport_usable_in_both_adapters() -> Result<()> {
    bounded(|| {
        for responsive in [false, true] {
            let mut h = Household::new()?;
            let failure = h.files.0.join("copy failure directory.json");
            std::fs::create_dir(&failure)?;
            let observed = origin(&h.viewer).clone();
            if responsive {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_copy_open(&mut host);
                host_edit(&mut host, &failure);
                host_submit(&mut host);
                host.settle_for_test()?;
                assert!(host.frontend.status.contains("retry Save a Copy…"));
                send(&mut host, Message::PickerEscape);
                assert!(host.load_picker.is_none());
                send(&mut host, Message::Options);
                send(&mut host, Message::Toggle);
                host.settle_for_test()?;
                assert!(host.frontend.playing);
                send(&mut host, Message::Toggle);
                host.settle_for_test()?;
                assert!(!host.frontend.playing);
                let owner = host.recover_for_test();
                assert!(owner.error.is_none());
                assert_eq!(origin(&owner), &observed);
            } else {
                copy_open(&mut h.viewer);
                edit(&mut h.viewer, &failure);
                submit(&mut h.viewer);
                assert!(h.viewer.feedback_status().contains("retry Save a Copy…"));
                drop(h.viewer.update(Message::PickerEscape));
                assert!(h.viewer.load_picker.is_none());
                drop(h.viewer.update(Message::Options));
                drop(h.viewer.update(Message::Toggle));
                assert!(h.viewer.playing && h.viewer.error.is_none());
                drop(h.viewer.update(Message::Toggle));
                assert!(!h.viewer.playing && h.viewer.error.is_none());
                assert_eq!(origin(&h.viewer), &observed);
            }
        }
        Ok(())
    })
}

#[cfg(unix)]
#[test]
fn save_copy_non_utf8_seed_is_refused_without_lossy_path_adoption_in_both_adapters() -> Result<()> {
    bounded(|| {
        use std::os::unix::ffi::OsStringExt;
        let mut h = Household::new()?;
        let path = h.files.0.join(std::ffi::OsString::from_vec(b"copy-\xff.json".to_vec()));
        h.viewer.last_copy_path = Some(path.clone());
        drop(h.viewer.update(Message::Options));
        drop(h.viewer.update(Message::ChooseSaveCopy));
        assert!(h.viewer.load_picker.is_none());
        assert!(h.viewer.feedback_status().contains("represented exactly"));
        let mut host = Host::from_viewer_for_test(h.viewer);
        host.last_copy_path = Some(path);
        send(&mut host, Message::ChooseSaveCopy);
        assert!(host.load_picker.is_none());
        assert!(host.picker_feedback.as_ref().unwrap().contains("represented exactly"));
        Ok(())
    })
}

// Authored actual-subscription, real-file controls: ALL UNRUN, compilation too.
// These are source-authored managed households, not native-window qualification.
mod save_shortcut_tests {
    use super::*;

    fn save_press(repeat: bool, modifiers: Modifiers) -> iced::Event {
        press(Key::Character("s".into()), Code::KeyS, repeat, modifiers)
    }
    fn save() -> iced::Event { save_press(false, player_shortcuts::primary_modifier()) }
    struct Resume(Option<std::sync::mpsc::SyncSender<()>>);
    impl Resume {
        fn release(mut self) -> Result<()> { self.0.take().unwrap().send(())?; Ok(()) }
    }
    impl Drop for Resume {
        fn drop(&mut self) { if let Some(sender) = self.0.take() { let _ = sender.send(()); } }
    }
    fn hold(host: &mut Host, message: Message) -> Result<Resume> {
        let (entered, resume) = host.gate_next_for_test();
        let resume = Resume(Some(resume));
        send(host, message);
        entered.recv_timeout(std::time::Duration::from_secs(5))?;
        Ok(resume)
    }

    #[test]
    fn save_shortcut_direct_real_managed_file_feedback_owner_and_destination() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            let observed = origin(&h.viewer).clone();
            let input = std::fs::read(&h.first)?;
            let owner = h.viewer.copy_owner.clone();
            let count = ordinal(&h.account);
            let state = transport(&h.viewer);
            let load = h.viewer.args.load.clone();
            direct_event(&mut h.viewer, save(), Status::Ignored);
            assert!(h.viewer.error.is_none(), "{:?}", h.viewer.error);
            copy_bytes(&h.destination, &h.viewer)?;
            let feedback = h.viewer.save_feedback.clone().context("actual Save feedback absent")?;
            assert_eq!(feedback, format!("Saved {}", h.destination.display()));
            assert_eq!(ui_publication::Published::capture(&h.viewer).save_load_feedback.as_ref(), Some(&feedback));
            assert_eq!(origin(&h.viewer), &observed);
            assert!(h.viewer.copy_owner.same(&owner));
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert_eq!(h.viewer.args.load, load);
            assert_eq!(ordinal(&h.account), count);
            assert_eq!(transport(&h.viewer), state);
            assert_eq!(std::fs::read(&h.first)?, input);
            // A second writer would destroy this sentinel even if Save's
            // serialized state and apparent success text happened to match.
            std::fs::write(&h.destination, b"held gesture must not write again")?;
            for repeat in [false, true, false] {
                direct_event(&mut h.viewer, save_press(repeat, player_shortcuts::primary_modifier()), Status::Ignored);
            }
            assert_eq!(std::fs::read(&h.destination)?, b"held gesture must not write again");
            direct_event(&mut h.viewer, release(Code::KeyS), Status::Captured);
            direct_event(&mut h.viewer, save(), Status::Ignored);
            copy_bytes(&h.destination, &h.viewer)?;
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_host_real_managed_file_and_existing_reply_feedback() -> Result<()> {
        bounded(|| {
            let h = Household::new()?;
            let observed = origin(&h.viewer).clone();
            let input = std::fs::read(&h.first)?;
            let owner = h.viewer.copy_owner.clone();
            let state = transport(&h.viewer);
            let count = ordinal(&h.account);
            let mut host = Host::from_viewer_for_test(h.viewer);
            current(&mut host, save(), Status::Ignored);
            host.settle_for_test()?;
            let feedback = host.frontend.save_load_feedback.clone().context("real reply lost Save feedback")?;
            assert_eq!(feedback, format!("Saved {}", h.destination.display()));
            std::fs::write(&h.destination, b"held Host gesture must not write again")?;
            for repeat in [true, false] {
                current(&mut host, save_press(repeat, player_shortcuts::primary_modifier()), Status::Ignored);
            }
            host.settle_for_test()?;
            assert_eq!(std::fs::read(&h.destination)?, b"held Host gesture must not write again");
            current(&mut host, release(Code::KeyS), Status::Captured);
            current(&mut host, save(), Status::Ignored);
            host.settle_for_test()?;
            let viewer = host.recover_for_test();
            copy_bytes(&h.destination, &viewer)?;
            assert_eq!(viewer.save_feedback.as_ref(), Some(&feedback));
            assert_eq!(origin(&viewer), &observed);
            assert!(viewer.copy_owner.same(&owner));
            assert_eq!(viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert_eq!(std::fs::read(&h.first)?, input);
            assert_eq!(transport(&viewer), state);
            assert_eq!(ordinal(&h.account), count);
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_both_adapters_recover_real_output_failure_and_retry_after_release() -> Result<()> {
        bounded(|| {
            for responsive in [false, true] {
                let mut h = Household::new()?;
                let blocker = h.files.0.join("blocked save parent");
                std::fs::write(&blocker, b"original output blocker")?;
                h.destination = blocker.join("household.json");
                h.viewer.args.save = h.destination.clone();
                let observed = origin(&h.viewer).clone();
                let before = saved(&h.viewer)?;
                let state = transport(&h.viewer);
                let input = std::fs::read(&h.first)?;
                let viewer = if responsive {
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    current(&mut host, save(), Status::Ignored);
                    host.settle_for_test()?;
                    assert!(host.frontend.error.is_none());
                    let feedback = host.frontend.save_load_feedback.as_ref().unwrap();
                    assert!(feedback.contains("Save failed") && feedback.contains("retry Save"));
                    assert_eq!(std::fs::read(&blocker)?, b"original output blocker");
                    assert!(!h.destination.exists());
                    std::fs::remove_file(&blocker)?;
                    current(&mut host, release(Code::KeyS), Status::Captured);
                    current(&mut host, save(), Status::Ignored);
                    host.settle_for_test()?;
                    host.recover_for_test()
                } else {
                    direct_event(&mut h.viewer, save(), Status::Ignored);
                    assert!(h.viewer.error.is_none());
                    let feedback = h.viewer.save_feedback.as_ref().unwrap();
                    assert!(feedback.contains("Save failed") && feedback.contains("retry Save"));
                    assert_eq!(saved(&h.viewer)?, before);
                    assert!(!h.destination.exists());
                    std::fs::remove_file(&blocker)?;
                    direct_event(&mut h.viewer, release(Code::KeyS), Status::Captured);
                    direct_event(&mut h.viewer, save(), Status::Ignored);
                    h.viewer
                };
                assert!(viewer.error.is_none());
                copy_bytes(&h.destination, &viewer)?;
                assert!(viewer.save_feedback.as_ref().unwrap().starts_with("Saved "));
                assert_eq!(origin(&viewer), &observed);
                assert_eq!(transport(&viewer), state);
                assert_eq!(std::fs::read(&h.first)?, input);
                assert_eq!(viewer.args.save.as_os_str(), h.destination.as_os_str());
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_both_adapters_refuse_whole_captured_modified_focus_and_overlay_gestures() -> Result<()> {
        bounded(|| {
            for responsive in [false, true] {
                for refusal in ["captured", "repeat", "plain", "extra", "foreign", "Options", "Diagnostics", "picker", "modal", "unfocused"] {
                    let mut h = Household::new()?;
                    let observed = origin(&h.viewer).clone();
                    let primary = player_shortcuts::primary_modifier();
                    let modifiers = match refusal {
                        "plain" => Modifiers::NONE,
                        "extra" => primary | Modifiers::ALT,
                        "foreign" => if cfg!(target_os = "macos") { Modifiers::CTRL } else { Modifiers::LOGO },
                        _ => primary,
                    };
                    let status = if refusal == "captured" { Status::Captured } else { Status::Ignored };
                    let viewer = if responsive {
                        let mut host = Host::from_viewer_for_test(h.viewer);
                        match refusal {
                            "Options" => send(&mut host, Message::Options),
                            "Diagnostics" => send(&mut host, Message::Diagnostics),
                            "picker" => host_open(&mut host),
                            "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                            "unfocused" => current(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                            _ => {}
                        }
                        current(&mut host, save_press(refusal == "repeat", modifiers), status);
                        // Test-only dismissal of unsubmitted shell ownership.
                        host.frontend.options_open = false;
                        host.frontend.diagnostics = false;
                        host.load_picker = None;
                        Arc::make_mut(&mut host.frontend.data).modal = false;
                        current(&mut host, iced::Event::Window(window::Event::Focused), Status::Captured);
                        current(&mut host, save(), Status::Ignored);
                        host.settle_for_test()?;
                        assert!(!h.destination.exists(), "refused gesture resumed: {refusal}");
                        current(&mut host, release(Code::KeyS), Status::Captured);
                        current(&mut host, save(), Status::Ignored);
                        host.settle_for_test()?;
                        host.recover_for_test()
                    } else {
                        match refusal {
                            "Options" => h.viewer.options_open = true,
                            "Diagnostics" => h.viewer.diagnostics = true,
                            "picker" => open(&mut h.viewer),
                            "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                            "unfocused" => direct_event(&mut h.viewer, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                            _ => {}
                        }
                        direct_event(&mut h.viewer, save_press(refusal == "repeat", modifiers), status);
                        h.viewer.options_open = false;
                        h.viewer.diagnostics = false;
                        h.viewer.load_picker = None;
                        h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = None;
                        direct_event(&mut h.viewer, iced::Event::Window(window::Event::Focused), Status::Captured);
                        direct_event(&mut h.viewer, save(), Status::Ignored);
                        assert!(!h.destination.exists(), "refused gesture resumed: {refusal}");
                        direct_event(&mut h.viewer, release(Code::KeyS), Status::Captured);
                        direct_event(&mut h.viewer, save(), Status::Ignored);
                        h.viewer
                    };
                    copy_bytes(&h.destination, &viewer)?;
                    assert_eq!(origin(&viewer), &observed);
                }
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_host_queue_keeps_origin_and_rechecks_current_shell_before_dispatch() -> Result<()> {
        bounded(|| {
            for boundary in ["Options", "Diagnostics", "focus", "confirmation", "owner", "close"] {
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                let resume = hold(&mut host, Message::Shift(false))?;
                current(&mut host, save(), Status::Ignored);
                assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(0, Message::Save)]));
                assert!(host.queued.front().unwrap().work.save_shortcut.is_some());
                match boundary {
                    "Options" => { send(&mut host, Message::Options); send(&mut host, Message::Options); }
                    "Diagnostics" => { send(&mut host, Message::Diagnostics); send(&mut host, Message::Diagnostics); }
                    "focus" => {
                        current(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured);
                        current(&mut host, iced::Event::Window(window::Event::Focused), Status::Captured);
                    }
                    "confirmation" => {
                        send(&mut host, Message::Options);
                        send(&mut host, Message::RequestApplicationChoice(neighborhood_usability::Choice::BuyPreview));
                    }
                    "owner" => send(&mut host, Message::Restart),
                    "close" => { drop(host.update(Event::Close(window::Id::unique()))); }
                    _ => unreachable!(),
                }
                resume.release()?;
                host.settle_for_test()?;
                assert!(!h.destination.exists(), "queued shortcut wrote after {boundary}");
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_stale_shell_events_preserve_current_owner_managed_save() -> Result<()> {
        bounded(|| {
            for submitted in [false, true] {
                for effect in [false, true] {
                    for boundary in ["Options", "Diagnostics", "ChooseLoad", "ChooseSaveCopy", "Restart"] {
                        let h = Household::new()?;
                        let mut host = Host::from_viewer_for_test(h.viewer);
                        host_open(&mut host);
                        host_edit(&mut host, &h.second);
                        host_submit(&mut host);
                        host.settle_for_test()?;
                        assert_eq!(host.generation(), 1);
                        assert!(!host.frontend.options_open && !host.frontend.diagnostics);
                        let resume = if submitted {
                            let (entered, resume) = host.gate_next_for_test();
                            let resume = Resume(Some(resume));
                            current(&mut host, save(), Status::Ignored);
                            entered.recv_timeout(std::time::Duration::from_secs(5))?;
                            assert!(host.queued.is_empty() && host.worker.as_ref().unwrap().busy());
                            resume
                        } else {
                            let resume = hold(&mut host, Message::Shift(false))?;
                            current(&mut host, save(), Status::Ignored);
                            assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(1, Message::Save)]));
                            resume
                        };
                        assert!(!h.destination.exists());
                        let message = match boundary {
                            "Options" => Message::Options,
                            "Diagnostics" => Message::Diagnostics,
                            "ChooseLoad" => Message::ChooseLoad,
                            "ChooseSaveCopy" => Message::ChooseSaveCopy,
                            _ => Message::Restart,
                        };
                        drop(host.update(if effect {
                            Event::Effect { generation: 0, message }
                        } else {
                            Event::Widget { generation: 0, message }
                        }));
                        assert_eq!(host.generation(), 1);
                        assert!(!host.frontend.options_open && !host.frontend.diagnostics);
                        assert!(host.load_picker.is_none());
                        resume.release()?;
                        host.settle_for_test()?;
                        assert!(h.destination.exists(),
                            "stale {boundary} suppressed current Save: submitted={submitted}, effect={effect}");
                        assert_eq!(host.frontend.save_load_feedback.as_deref(),
                            Some(format!("Saved {}", h.destination.display()).as_str()));
                        let viewer = host.recover_for_test();
                        copy_bytes(&h.destination, &viewer)?;
                        assert_eq!(origin(&viewer).requested_path, h.second.to_str().unwrap());
                        assert_eq!(std::fs::read(&h.second)?, h.second_bytes);
                        assert_eq!(viewer.args.save.as_os_str(), h.destination.as_os_str());
                    }
                }
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_queue_uses_current_publication_before_worker_dispatch() -> Result<()> {
        bounded(|| {
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            let (notify, completed) = std::sync::mpsc::sync_channel(1);
            host.notification_for_test(Box::new(move || { let _ = notify.send(()); }));
            let resume = hold(&mut host, Message::Shift(false))?;
            current(&mut host, save(), Status::Ignored);
            assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(0, Message::Save)]));
            resume.release()?;
            completed.recv_timeout(std::time::Duration::from_secs(5))?;
            drop(host.poll()); // real completion; deliberately do not pump yet
            assert!(!host.worker.as_ref().unwrap().busy());
            // Synthetic ownership boundary solely for the real pump guard.
            // Actual owner changes and engine rechecks have separate controls.
            Arc::make_mut(&mut host.frontend.data).load_picker_blocked = true;
            drop(host.pump());
            assert!(host.queued.is_empty() && !host.worker.as_ref().unwrap().busy());
            assert!(!h.destination.exists());
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_submitted_work_rechecks_revoked_shell_before_actual_engine_apply() -> Result<()> {
        bounded(|| {
            for boundary in ["Options", "Diagnostics", "focus", "close"] {
                let h = Household::new()?;
                let mut host = Host::from_viewer_for_test(h.viewer);
                let (entered, resume) = host.gate_next_for_test();
                let resume = Resume(Some(resume));
                current(&mut host, save(), Status::Ignored);
                entered.recv_timeout(std::time::Duration::from_secs(5))?;
                assert!(host.queued.is_empty() && host.worker.as_ref().unwrap().busy());
                assert!(!h.destination.exists());
                match boundary {
                    "Options" => send(&mut host, Message::Options),
                    "Diagnostics" => send(&mut host, Message::Diagnostics),
                    "focus" => current(&mut host, iced::Event::Window(window::Event::Unfocused), Status::Captured),
                    _ => { drop(host.update(Event::Close(window::Id::unique()))); }
                }
                resume.release()?;
                host.settle_for_test()?;
                assert!(!h.destination.exists(), "submitted shortcut wrote after {boundary}");
                assert!(host.frontend.save_load_feedback.is_none());
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_owner_rechecks_real_current_state_at_engine_apply_and_direct_dispatch() -> Result<()> {
        bounded(|| {
            for worker in [false, true] {
                for guard in ["modal", "selected", "pending", "menu", "fault", "confirmation", "Options", "Diagnostics", "absent"] {
                    let mut h = Household::new()?;
                    std::fs::write(&h.destination, b"last known good save")?;
                    match guard {
                        "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                        "selected" => h.viewer.selected = Some(4),
                        "pending" => h.viewer.viewport_enter_requested = true,
                        "menu" => h.viewer.menu_error = Some("current menu fault".into()),
                        "fault" => h.viewer.session.as_mut().unwrap().simulation.vm.threads.values_mut().next().unwrap().status =
                            sims_vm::Status::Faulted(sims_vm::Fault { message: "current VM fault".into(), stack: vec![] }),
                        "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation {
                            choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                        "Options" => h.viewer.options_open = true,
                        "Diagnostics" => h.viewer.diagnostics = true,
                        "absent" => h.viewer.replace_session(None),
                        _ => unreachable!(),
                    }
                    if worker {
                        let mut engine = Engine::new(h.viewer);
                        drop(engine.apply(Work { message: Message::Save, camera_shortcut: false,
                            save_shortcut: Some(Arc::new(AtomicBool::new(true))),
                            shell: Shell { diagnostics: guard == "Diagnostics", options_open: guard == "Options", revision: 0 },
                            popup: None, gate: None }).map_err(anyhow::Error::msg)?);
                        assert!(engine.viewer.save_feedback.is_none());
                    } else {
                        direct_event(&mut h.viewer, save(), Status::Ignored);
                        assert!(h.viewer.save_feedback.is_none());
                    }
                    assert_eq!(std::fs::read(&h.destination)?, b"last known good save", "{guard}");
                }
            }
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_real_owner_change_ahead_of_queued_intent_cannot_write() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            // The authored Viewer's supported no-device selection handler
            // retains an object selection deterministically, without a native
            // popup script deciding whether this fixture has interactions.
            h.viewer.device = None;
            assert!(!h.viewer.save_shortcut_blocked());
            let mut host = Host::from_viewer_for_test(h.viewer);
            let resume = hold(&mut host, Message::Select(Some(4)))?;
            current(&mut host, save(), Status::Ignored);
            assert!(matches!(host.player_shortcut_queue_for_test().as_slice(), [(0, Message::Save)]));
            resume.release()?;
            host.settle_for_test()?;
            assert!(!h.destination.exists());
            let viewer = host.recover_for_test();
            assert_eq!(viewer.selected, Some(4));
            assert!(viewer.save_feedback.is_none());
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_real_load_keeps_held_generation_and_stale_focus_release_fences() -> Result<()> {
        bounded(|| {
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            current(&mut host, save(), Status::Captured);
            host_open(&mut host);
            host_edit(&mut host, &h.second);
            host_submit(&mut host);
            host.settle_for_test()?;
            assert_eq!(host.generation(), 1);
            current(&mut host, save(), Status::Ignored); // raises release fence
            host_event(&mut host, 0, release(Code::KeyS), Status::Captured);
            current(&mut host, save(), Status::Ignored);
            assert!(!h.destination.exists());
            current(&mut host, release(Code::KeyS), Status::Captured);
            host_event(&mut host, 0, iced::Event::Window(window::Event::Unfocused), Status::Captured);
            host_event(&mut host, 0, iced::Event::Window(window::Event::Focused), Status::Captured);
            current(&mut host, save(), Status::Ignored);
            current(&mut host, release(Code::KeyS), Status::Captured);
            current(&mut host, iced::Event::Window(window::Event::Focused), Status::Captured);
            current(&mut host, save(), Status::Ignored);
            host.settle_for_test()?;
            let viewer = host.recover_for_test();
            copy_bytes(&h.destination, &viewer)?;
            assert_eq!(origin(&viewer).requested_path, h.second.to_str().unwrap());
            assert_eq!(std::fs::read(&h.second)?, h.second_bytes);
            assert_eq!(viewer.args.save.as_os_str(), h.destination.as_os_str());
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_direct_hold_survives_actual_input_replacement_until_physical_release() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            direct_event(&mut h.viewer, save(), Status::Captured);
            open(&mut h.viewer);
            edit(&mut h.viewer, &h.second);
            submit(&mut h.viewer);
            assert!(h.viewer.load_picker.is_none() && !h.viewer.options_open);
            direct_event(&mut h.viewer, save(), Status::Ignored);
            assert!(!h.destination.exists());
            direct_event(&mut h.viewer, release(Code::KeyS), Status::Captured);
            direct_event(&mut h.viewer, save(), Status::Ignored);
            copy_bytes(&h.destination, &h.viewer)?;
            assert_eq!(origin(&h.viewer).requested_path, h.second.to_str().unwrap());
            assert_eq!(std::fs::read(&h.second)?, h.second_bytes);
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_capture_and_worker_publication_faults_retain_existing_fatal_behavior() -> Result<()> {
        bounded(|| {
            for responsive in [false, true] {
                let mut h = Household::new()?;
                h.viewer.client = None; // actual capture failure, no output failure seam
                let viewer = if responsive {
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    current(&mut host, save(), Status::Ignored);
                    host.settle_for_test()?;
                    assert!(host.frontend.error.is_some());
                    host.recover_for_test()
                } else {
                    direct_event(&mut h.viewer, save(), Status::Ignored);
                    h.viewer
                };
                assert!(viewer.error.as_ref().unwrap().contains("dialog client absent"));
                assert!(viewer.save_feedback.is_none());
                assert!(!h.destination.exists());
            }
            let mut h = Household::new()?;
            h.viewer.loaded_input_test_fault = Some("worker_publication");
            let mut host = Host::from_viewer_for_test(h.viewer);
            current(&mut host, save(), Status::Ignored);
            assert!(host.settle_for_test().is_err());
            assert!(host.error.is_some());
            assert!(h.destination.is_file()); // write happened; publication did not confirm success
            assert!(host.frontend.save_load_feedback.is_none());
            Ok(())
        })
    }

    #[test]
    fn save_shortcut_existing_save_button_remains_available_in_options() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            drop(h.viewer.update(Message::Options));
            direct_event(&mut h.viewer, save(), Status::Ignored);
            assert!(!h.destination.exists());
            drop(h.viewer.update(Message::Save));
            copy_bytes(&h.destination, &h.viewer)?;
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            send(&mut host, Message::Options);
            current(&mut host, save(), Status::Ignored);
            assert!(!h.destination.exists());
            send(&mut host, Message::Save);
            host.settle_for_test()?;
            copy_bytes(&h.destination, &host.recover_for_test())?;
            Ok(())
        })
    }

    // Requires the existing offscreen Iced renderer when the parent runs it.
    // UNRUN: no window, native household or renderer qualification is claimed.
    #[test]
    fn save_shortcut_real_views_show_result_and_platform_guidance_in_both_adapters() -> Result<()> {
        bounded(|| {
            let mut h = Household::new()?;
            let mut driver = headless_input::Input::new()?;
            direct_event(&mut h.viewer, save(), Status::Ignored);
            let feedback = h.viewer.save_feedback.clone().unwrap();
            let observation = driver.element_observation(h.viewer.view(), [1024, 768])?;
            assert!(observation.texts.iter().any(|t| t.text == feedback && t.bounds.height > 0.));
            drop(h.viewer.update(Message::Options));
            let observation = driver.element_observation(h.viewer.view(), [1024, 768])?;
            assert!(observation.texts.iter().any(|t| t.text == player_shortcuts::save_guidance()));
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            current(&mut host, save(), Status::Ignored);
            host.settle_for_test()?;
            let feedback = host.frontend.save_load_feedback.clone().unwrap();
            let observation = driver.element_observation(host.view_messages(), [1024, 768])?;
            assert!(observation.texts.iter().any(|t| t.text == feedback && t.bounds.height > 0.));
            send(&mut host, Message::Options);
            let observation = driver.element_observation(host.view_messages(), [1024, 768])?;
            assert!(observation.texts.iter().any(|t| t.text == player_shortcuts::save_guidance()));
            Ok(())
        })
    }
}
