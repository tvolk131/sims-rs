//! Actual existing authored Household/Files and admitted Input observations.
//! ALL UNRUN, including compilation. Parent filter: recent_loads_
use super::*;
use load_picker::tests::{FocusUi, focus_enter, visit_opening_task};

fn names(history: &recent_loads::History) -> Vec<String> {
    history.paths().iter().map(|p| p.to_str().unwrap().to_owned()).collect()
}
fn actions<T: 'static>(task: Task<T>) -> usize {
    let mut count = 0;
    visit_opening_task(task, |action| match action {
        iced_runtime::Action::Widget(_) => count += 1,
        _ => panic!("recent choice emitted non-widget work"),
    });
    count
}
fn host_task(host: &mut Host, message: Message) -> Task<Event> {
    host.update(Event::Widget { generation: host.generation(), message })
}
fn authored_paths(h: &Household) -> Result<Vec<PathBuf>> {
    (0..10).map(|i| {
        let path = h.files.0.join(format!("recent authored {i} α.json"));
        std::fs::write(&path, &h.second_bytes)?;
        Ok(path)
    }).collect()
}

#[test]
fn recent_loads_actual_direct_and_host_success_order_dedupe_evict_and_pure_capture() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            let mut h = Household::new()?;
            let paths = authored_paths(&h)?;
            let mut expected: Vec<String> = paths.iter().rev().take(8).map(|p| p.to_str().unwrap().into()).collect();
            // Household::new really loads first.json through Input AND the
            // ordinary complete Message::Load tail; this is not a fake seed.
            assert_eq!(names(&h.viewer.recent_loads), [h.first.to_str().unwrap()]);
            let count = ordinal(&h.account);
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                for path in &paths {
                    host_open(&mut host);
                    host_edit(&mut host, path);
                    host_submit(&mut host);
                    host.settle_for_test()?;
                    assert!(host.load_picker.is_none() && host.error.is_none());
                }
                assert_eq!(names(&host.recent_loads), expected);
                host_open(&mut host);
                host_edit(&mut host, &paths[7]);
                host_submit(&mut host);
                host.settle_for_test()?;
                expected.remove(2);
                expected.insert(0, paths[7].to_str().unwrap().into());
                assert_eq!(names(&host.recent_loads), expected);
                drop(host.view_messages());
                drop(host.view_messages());
                drop(host.observation_value());
                assert_eq!(names(&host.recent_loads), expected);
                h.viewer = host.recover_for_test();
            } else {
                for path in &paths {
                    open(&mut h.viewer);
                    edit(&mut h.viewer, path);
                    submit(&mut h.viewer);
                    assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
                }
                assert_eq!(names(&h.viewer.recent_loads), expected);
                open(&mut h.viewer);
                edit(&mut h.viewer, &paths[7]);
                submit(&mut h.viewer);
                expected.remove(2);
                expected.insert(0, paths[7].to_str().unwrap().into());
                assert_eq!(names(&h.viewer.recent_loads), expected);
                for _ in 0..3 { drop(Published::capture(&h.viewer)); drop(h.viewer.view()); }
                assert_eq!(names(&h.viewer.recent_loads), expected);
            }
            assert_eq!(ordinal(&h.account), count + 11);
            assert_eq!(origin(&h.viewer).requested_path, paths[7].to_str().unwrap());
            assert_eq!(origin(&h.viewer).sha256, engine::digest(&std::fs::read(&paths[7])?));
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert!(!h.destination.exists());
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_choice_only_edits_focuses_once_then_existing_submit_commits() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            let mut h = Household::new()?;
            open(&mut h.viewer); edit(&mut h.viewer, &h.second); submit(&mut h.viewer);
            open(&mut h.viewer); edit(&mut h.viewer, &h.first); submit(&mut h.viewer);
            let before = saved(&h.viewer)?;
            let observed = origin(&h.viewer).clone();
            let owner = h.viewer.copy_owner.clone();
            let count = ordinal(&h.account);
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_open(&mut host);
                let choice = host.load_picker.as_ref().unwrap().recent_event_for_test(1);
                assert_eq!(actions(host_task(&mut host, Message::LoadPicker(choice.clone()))), 2);
                assert_eq!(actions(host_task(&mut host, Message::LoadPicker(choice))), 0);
                assert_eq!(host.load_picker.as_ref().unwrap().value(), h.second.to_str().unwrap());
                assert!(host.copy_owner.same(&owner));
                assert!(!host.worker.as_ref().unwrap().busy() && host.queued.is_empty());
                assert_eq!(ordinal(&h.account), count);
                // Render never refocuses, and ordinary edits produce no task.
                drop(host.view_messages());
                let event = host.load_picker.as_ref().unwrap().edit_event_for_test(h.second.to_str().unwrap().into());
                assert_eq!(actions(host_task(&mut host, Message::LoadPicker(event))), 0);
                let retained = host.recover_for_test();
                assert_eq!(saved(&retained)?, before);
                assert_eq!(origin(&retained), &observed);
                let mut host = Host::from_viewer_for_test(retained);
                host.frontend.options_open = false;
                host_open(&mut host);
                let event = host.load_picker.as_ref().unwrap().recent_event_for_test(1);
                assert_eq!(actions(host_task(&mut host, Message::LoadPicker(event))), 2);
                host_submit(&mut host);
                host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads)[0], h.second.to_str().unwrap());
                h.viewer = host.recover_for_test();
            } else {
                open(&mut h.viewer);
                let choice = h.viewer.load_picker.as_ref().unwrap().recent_event_for_test(1);
                assert_eq!(actions(h.viewer.update(Message::LoadPicker(choice.clone()))), 2);
                assert_eq!(actions(h.viewer.update(Message::LoadPicker(choice))), 0);
                assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), h.second.to_str().unwrap());
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &observed);
                assert!(h.viewer.copy_owner.same(&owner));
                assert_eq!(ordinal(&h.account), count);
                submit(&mut h.viewer);
                assert_eq!(names(&h.viewer.recent_loads)[0], h.second.to_str().unwrap());
            }
            assert_eq!(ordinal(&h.account), count + 1);
            assert_eq!(origin(&h.viewer).requested_path, h.second.to_str().unwrap());
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
            assert!(!h.destination.exists());
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_moved_deleted_damaged_choice_keeps_healthy_input_and_save_then_retries() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            for damage in ["moved", "deleted", "damaged"] {
                let mut h = Household::new()?;
                open(&mut h.viewer); edit(&mut h.viewer, &h.second); submit(&mut h.viewer);
                open(&mut h.viewer); edit(&mut h.viewer, &h.first); submit(&mut h.viewer);
                let expected = names(&h.viewer.recent_loads);
                let before = saved(&h.viewer)?;
                let observed = origin(&h.viewer).clone();
                let owner = h.viewer.copy_owner.clone();
                let count = ordinal(&h.account);
                match damage {
                    "moved" => std::fs::rename(&h.second, h.files.0.join("moved.json"))?,
                    "deleted" => std::fs::remove_file(&h.second)?,
                    "damaged" => std::fs::write(&h.second, b"damaged JSON")?,
                    _ => unreachable!(),
                }
                if hosted {
                    let mut host = Host::from_viewer_for_test(h.viewer);
                    host_open(&mut host);
                    let event = host.load_picker.as_ref().unwrap().recent_event_for_test(1);
                    assert_eq!(actions(host_task(&mut host, Message::LoadPicker(event))), 2);
                    host_submit(&mut host); host.settle_for_test()?;
                    assert!(host.error.is_none() && host.frontend.error.is_none());
                    assert!(!host.load_picker.as_ref().unwrap().pending());
                    assert_eq!(names(&host.recent_loads), expected);
                    assert!(host.copy_owner.same(&owner));
                    assert_eq!(ordinal(&h.account), count);
                    h.viewer = host.recover_for_test();
                } else {
                    open(&mut h.viewer);
                    let event = h.viewer.load_picker.as_ref().unwrap().recent_event_for_test(1);
                    assert_eq!(actions(h.viewer.update(Message::LoadPicker(event))), 2);
                    submit(&mut h.viewer);
                    assert!(h.viewer.error.is_none());
                    assert_eq!(names(&h.viewer.recent_loads), expected);
                }
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &observed);
                assert!(h.viewer.copy_owner.same(&owner));
                assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
                assert_eq!(ordinal(&h.account), count);
                assert!(h.viewer.load_feedback.as_ref().unwrap().contains("Load"));
                std::fs::write(&h.second, &h.second_bytes)?;
                // The retry uses ordinary current-file preflight, never the
                // earlier successful bytes or any recent-path authority.
                if hosted {
                    // Recovering the fixture consumes Host's shell picker;
                    // reopen its same recent choice through the direct adapter.
                    h.viewer.options_open = true;
                    drop(h.viewer.update(Message::ChooseLoad));
                    let event = h.viewer.load_picker.as_ref().unwrap().recent_event_for_test(1);
                    drop(h.viewer.update(Message::LoadPicker(event)));
                }
                submit(&mut h.viewer);
                assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
                assert_eq!(names(&h.viewer.recent_loads)[0], h.second.to_str().unwrap());
                assert_eq!(ordinal(&h.account), count + 1);
            }
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_edits_attempt_cancel_save_and_copy_never_promote() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            let mut h = Household::new()?;
            let expected = names(&h.viewer.recent_loads);
            let observed = origin(&h.viewer).clone();
            let count = ordinal(&h.account);
            let output = h.files.0.join("successful copy.json");
            let failed_copy = h.files.0.join("copy is a directory.json");
            std::fs::create_dir(&failed_copy)?;
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_open(&mut host); host_edit(&mut host, &h.second);
                assert_eq!(names(&host.recent_loads), expected);
                send(&mut host, Message::PickerEscape);
                send(&mut host, Message::ChooseLoad); host_edit(&mut host, &h.files.0.join("absent.json"));
                host_submit(&mut host); host.settle_for_test()?;
                send(&mut host, Message::PickerEscape);
                send(&mut host, Message::ChooseSaveCopy); host_edit(&mut host, &failed_copy);
                host_submit(&mut host); host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads), expected);
                assert!(!host.load_picker.as_ref().unwrap().pending());
                host_edit(&mut host, &output);
                host_submit(&mut host); host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads), expected);
                send(&mut host, Message::ChooseSaveCopy);
                send(&mut host, Message::PickerEscape);
                send(&mut host, Message::Save); host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads), expected);
                h.viewer = host.recover_for_test();
            } else {
                open(&mut h.viewer); edit(&mut h.viewer, &h.second);
                assert_eq!(names(&h.viewer.recent_loads), expected);
                drop(h.viewer.update(Message::PickerEscape));
                // Options stayed open when cancelling the shell picker.
                drop(h.viewer.update(Message::ChooseLoad));
                edit(&mut h.viewer, &h.files.0.join("absent.json")); submit(&mut h.viewer);
                drop(h.viewer.update(Message::PickerEscape));
                drop(h.viewer.update(Message::ChooseSaveCopy)); edit(&mut h.viewer, &failed_copy); submit(&mut h.viewer);
                assert_eq!(names(&h.viewer.recent_loads), expected);
                assert!(!h.viewer.load_picker.as_ref().unwrap().pending());
                edit(&mut h.viewer, &output); submit(&mut h.viewer);
                assert_eq!(names(&h.viewer.recent_loads), expected);
                drop(h.viewer.update(Message::ChooseSaveCopy)); drop(h.viewer.update(Message::PickerEscape));
                drop(h.viewer.update(Message::Save));
            }
            assert_eq!(names(&h.viewer.recent_loads), expected);
            copy_bytes(&output, &h.viewer)?;
            assert_eq!(origin(&h.viewer), &observed);
            assert_eq!(ordinal(&h.account), count);
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_stale_effect_focus_modal_pending_and_queue_choices_are_refused() -> Result<()> {
    bounded(|| {
        for guard in ["modal", "pending", "focus", "diagnostics", "Options", "confirmation", "fault"] {
            let mut h = Household::new()?;
            open(&mut h.viewer);
            let event = h.viewer.load_picker.as_ref().unwrap().recent_event_for_test(0);
            let before = h.viewer.load_picker.as_ref().unwrap().value().to_owned();
            let count = ordinal(&h.account);
            match guard {
                "modal" => h.viewer.session.as_mut().unwrap().simulation.vm.world.dialogs.modal = Some(999),
                "pending" => h.viewer.viewport_enter_requested = true,
                "focus" => { drop(h.viewer.update(Message::FocusLost)); },
                "diagnostics" => h.viewer.diagnostics = true,
                "Options" => h.viewer.options_open = false,
                "confirmation" => h.viewer.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                "fault" => h.viewer.error = Some("authored fault".into()),
                _ => unreachable!(),
            }
            assert_eq!(actions(h.viewer.update(Message::LoadPicker(event))), 0, "direct {guard}");
            assert_eq!(h.viewer.load_picker.as_ref().unwrap().value(), before);
            assert_eq!(ordinal(&h.account), count);
        }
        for guard in ["generation", "effect", "modal", "pending", "focus", "diagnostics", "Options", "confirmation", "fault", "replacing", "closing", "busy", "queue"] {
            let h = Household::new()?;
            let mut host = Host::from_viewer_for_test(h.viewer);
            host_open(&mut host);
            let event = host.load_picker.as_ref().unwrap().recent_event_for_test(0);
            let before = host.load_picker.as_ref().unwrap().value().to_owned();
            let generation = host.generation();
            let mut release = None;
            if matches!(guard, "busy" | "queue") {
                let (entered, resume) = host.gate_next_for_test();
                // Go through the actual owner queue, without the picker fence.
                let picker = host.load_picker.take();
                send(&mut host, Message::Shift(true));
                host.load_picker = picker;
                entered.recv_timeout(std::time::Duration::from_secs(5))?;
                if guard == "queue" {
                    let picker = host.load_picker.take();
                    send(&mut host, Message::Shift(false));
                    host.load_picker = picker;
                    assert!(!host.queued.is_empty());
                }
                release = Some(resume);
            }
            match guard {
                "modal" => Arc::make_mut(&mut host.frontend.data).modal = true,
                "pending" => Arc::make_mut(&mut host.frontend.data).pending = true,
                "focus" => { send(&mut host, Message::FocusLost); },
                "diagnostics" => host.frontend.diagnostics = true,
                "Options" => host.frontend.options_open = false,
                "confirmation" => host.application_confirmation = Some(neighborhood_usability::Confirmation { choice: neighborhood_usability::Choice::BuyPreview, revision: 1 }),
                "fault" => host.error = Some("authored fault".into()),
                "replacing" => host.replacing = true,
                "closing" => host.closing = Some(window::Id::unique()),
                _ => {}
            }
            let message = Message::LoadPicker(event);
            let event = if guard == "effect" { Event::Effect { generation, message } }
                else { Event::Widget { generation: if guard == "generation" { generation + 1 } else { generation }, message } };
            assert_eq!(actions(host.update(event)), 0, "host {guard}");
            assert_eq!(host.load_picker.as_ref().unwrap().value(), before);
            if let Some(resume) = release { resume.send(())?; host.settle_for_test()?; }
            let _ = host.recover_for_test();
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_failed_tail_or_worker_publication_never_promotes() -> Result<()> {
    bounded(|| {
        for fault in ["load_native_prepare", "application_capture", "load_postcommit", "ui_load_common_prepare", "post_update_popup_counter", "worker_publication"] {
            let mut h = Household::new()?;
            let expected = names(&h.viewer.recent_loads);
            h.viewer.loaded_input_test_fault = Some(fault);
            if matches!(fault, "post_update_popup_counter" | "worker_publication") {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host_open(&mut host); host_edit(&mut host, &h.second); host_submit(&mut host);
                let error = host.settle_for_test().expect_err("injected worker failure must surface");
                assert!(error.to_string().contains(fault), "unexpected failure for {fault}: {error:#}");
                assert_eq!(names(&host.recent_loads), expected);
                assert!(host.error.is_some());
                let _ = host.recover_for_test();
            } else {
                open(&mut h.viewer); edit(&mut h.viewer, &h.second); submit(&mut h.viewer);
                assert!(h.viewer.error.is_some(), "fault injection did not reach {fault}");
                assert_eq!(names(&h.viewer.recent_loads), expected);
                assert!(h.viewer.committed_load.is_none());
            }
        }
        Ok(())
    })
}

#[test]
fn recent_loads_actual_host_in_flight_cancel_and_stale_choice_wait_for_confirmed_reply() -> Result<()> {
    bounded(|| {
        let h = Household::new()?;
        let count = ordinal(&h.account);
        let mut host = Host::from_viewer_for_test(h.viewer);
        let expected = names(&host.recent_loads);
        host_open(&mut host); host_edit(&mut host, &h.second);
        let stale = host.load_picker.as_ref().unwrap().recent_event_for_test(0);
        let (entered, resume) = host.gate_next_for_test();
        host_submit(&mut host);
        entered.recv_timeout(std::time::Duration::from_secs(5))?;
        assert!(host.in_flight_load.is_some() && host.load_picker.as_ref().unwrap().pending());
        assert_eq!(names(&host.recent_loads), expected);
        assert_eq!(ordinal(&h.account), count);
        assert_eq!(actions(host_task(&mut host, Message::LoadPicker(stale))), 0);
        send(&mut host, Message::PickerEscape);
        assert!(host.load_picker.as_ref().unwrap().pending());
        assert_eq!(names(&host.recent_loads), expected);
        resume.send(())?;
        host.settle_for_test()?;
        assert_eq!(names(&host.recent_loads)[0], h.second.to_str().unwrap());
        let owner = host.recover_for_test();
        assert_eq!(origin(&owner).requested_path, h.second.to_str().unwrap());
        assert_eq!(ordinal(&h.account), count + 1);
        assert_eq!(owner.args.save.as_os_str(), h.destination.as_os_str());
        Ok(())
    })
}

#[test]
fn recent_loads_actual_exact_alias_spelling_remains_distinct_in_both_adapters() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            let mut h = Household::new()?;
            let plain = h.files.0.join("second household α.json");
            assert_ne!(plain.as_os_str(), h.second.as_os_str());
            let expected = [plain.to_str().unwrap().to_owned(), h.second.to_str().unwrap().to_owned(), h.first.to_str().unwrap().to_owned()];
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                for path in [&h.second, &plain] {
                    host_open(&mut host); host_edit(&mut host, path); host_submit(&mut host);
                    host.settle_for_test()?;
                }
                assert_eq!(names(&host.recent_loads), expected);
                h.viewer = host.recover_for_test();
            } else {
                for path in [&h.second, &plain] { open(&mut h.viewer); edit(&mut h.viewer, path); submit(&mut h.viewer); }
                assert_eq!(names(&h.viewer.recent_loads), expected);
            }
            assert_eq!(origin(&h.viewer).requested_path, plain.to_str().unwrap());
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
        }
        Ok(())
    })
}

#[cfg(unix)]
#[test]
fn recent_loads_actual_non_utf8_legacy_request_obeys_filesystem_without_lossy_recent_entry() -> Result<()> {
    bounded(|| {
        use std::os::unix::ffi::OsStringExt;
        for hosted in [false, true] {
            let mut h = Household::new()?;
            let path = h.files.0.join(std::ffi::OsString::from_vec(b"input-\xff.json".to_vec()));
            // Darwin filesystems may reject invalid UTF-8 names (EILSEQ=92).
            // Still submit that exact legacy path and verify healthy refusal;
            // filesystems accepting it retain the genuine successful-load check.
            let created = match std::fs::write(&path, &h.second_bytes) {
                Ok(()) => true,
                Err(error) if cfg!(target_os = "macos") && error.raw_os_error() == Some(92) => false,
                Err(error) => return Err(error.into()),
            };
            let before = saved(&h.viewer)?;
            let before_origin = origin(&h.viewer).clone();
            let before_zoom = h.viewer.zoom;
            let expected = names(&h.viewer.recent_loads);
            let count = ordinal(&h.account);
            h.viewer.args.save = path;
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                send(&mut host, Message::Load); host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads), expected);
                h.viewer = host.recover_for_test();
            } else { drop(h.viewer.update(Message::Load)); }
            assert!(h.viewer.error.is_none());
            if created {
                assert_eq!(h.viewer.zoom, 0.75);
                assert_eq!(ordinal(&h.account), count + 1);
            } else {
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &before_origin);
                assert_eq!(h.viewer.zoom, before_zoom);
                assert_eq!(ordinal(&h.account), count);
                println!("Darwin refused non-UTF-8 filename; exact legacy Load refused with healthy input retained");
            }
            assert_eq!(names(&h.viewer.recent_loads), expected);
            h.viewer.args.save = h.destination.clone();
        }
        Ok(())
    })
}

#[test]
#[ignore = "opt-in real Iced wgpu path choice, focus operations and Enter with authored Household; UNRUN"]
fn recent_loads_widgets_both_adapters_choose_then_keyboard_enter_uses_real_input() -> Result<()> {
    bounded(|| {
        for hosted in [false, true] {
            let mut h = Household::new()?;
            open(&mut h.viewer); edit(&mut h.viewer, &h.second); submit(&mut h.viewer);
            open(&mut h.viewer); edit(&mut h.viewer, &h.first); submit(&mut h.viewer);
            let before = saved(&h.viewer)?;
            let observed = origin(&h.viewer).clone();
            let count = ordinal(&h.account);
            let mut ui = FocusUi::new()?;
            let mut observer = headless_input::Input::new()?;
            if hosted {
                let mut host = Host::from_viewer_for_test(h.viewer);
                host.frontend.options_open = true;
                let task = host_task(&mut host, Message::ChooseLoad);
                assert_eq!(ui.task(host.view_messages(), task), 2);
                let observed_ui = observer.element_observation(host.view_messages(), [1000, 760])?;
                let point = observed_ui.texts.iter().find(|t| t.text == h.second.to_str().unwrap())
                    .context("recent path button absent")?.bounds.center();
                let messages = ui.click_at(host.view_messages(), point);
                let mut accepted_choices = 0;
                for message in messages {
                    let choice = matches!(&message, Message::LoadPicker(event) if event.is_recent_choice());
                    if !choice { println!("auxiliary picker pointer message: {message:?}"); }
                    let task = host_task(&mut host, message);
                    let effects = ui.task(host.view_messages(), task);
                    if choice {
                        accepted_choices += 1;
                        assert_eq!(effects, 2);
                    } else {
                        assert_eq!(effects, 0, "background input must not perform work through the picker");
                    }
                }
                assert_eq!(accepted_choices, 1);
                assert_eq!(ordinal(&h.account), count);
                assert!(!host.worker.as_ref().unwrap().busy());
                let messages = ui.events(host.view_messages(), &[focus_enter()]);
                assert_eq!(messages.len(), 1);
                for message in messages { drop(host_task(&mut host, message)); }
                host.settle_for_test()?;
                assert_eq!(names(&host.recent_loads)[0], h.second.to_str().unwrap());
                h.viewer = host.recover_for_test();
            } else {
                open(&mut h.viewer);
                let observed_ui = observer.element_observation(h.viewer.view(), [1000, 760])?;
                let point = observed_ui.texts.iter().find(|t| t.text == h.second.to_str().unwrap())
                    .context("recent path button absent")?.bounds.center();
                let messages = ui.click_at(h.viewer.view(), point);
                let mut accepted_choices = 0;
                for message in messages {
                    let choice = matches!(&message, Message::LoadPicker(event) if event.is_recent_choice());
                    if !choice { println!("auxiliary picker pointer message: {message:?}"); }
                    let task = h.viewer.update(message);
                    let effects = ui.task(h.viewer.view(), task);
                    if choice {
                        accepted_choices += 1;
                        assert_eq!(effects, 2);
                    } else {
                        assert_eq!(effects, 0, "background input must not perform work through the picker");
                    }
                }
                assert_eq!(accepted_choices, 1);
                assert_eq!(saved(&h.viewer)?, before);
                assert_eq!(origin(&h.viewer), &observed);
                let messages = ui.events(h.viewer.view(), &[focus_enter()]);
                assert_eq!(messages.len(), 1);
                for message in messages { drop(h.viewer.update(message)); }
            }
            assert_eq!(origin(&h.viewer).requested_path, h.second.to_str().unwrap());
            assert_eq!(ordinal(&h.account), count + 1);
            assert!(h.viewer.error.is_none() && h.viewer.load_picker.is_none());
            assert_eq!(h.viewer.args.save.as_os_str(), h.destination.as_os_str());
        }
        Ok(())
    })
}
