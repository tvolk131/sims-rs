//! Authored only, ALL UNRUN. Real files, real Input, real Message::Load route.
//! Reuses the source-owned bounded Save fixture; no installed/private assets.
//!
//! Parent seams required (no old files are changed by this worker):
//! 1. Declare original_viewer::load_feedback, factor work-scoped
//!    Viewer::load_prepared_with_index(&mut self, Input, &Index) -> Result<()>,
//!    and add retained Viewer::load_feedback: Option<String>. Publish it
//!    through Published::status, retain it across refresh/frame, clear it on
//!    replacement/fail, and set success only after the common Load tail.
//! 2. Reject unhealthy explicit Load before preflight. Early typed rejection
//!    skips the consuming loader, common tail AND preparation_result. Existing
//!    loaded_input_test_fault="ui_load_common_prepare" catches tail entry.
//! 3. Extend the existing loaded_input_test_boundary hook with
//!    "load_native_prepare" immediately before native/client preparation in
//!    the consuming loader, and "load_postcommit" after all saved-owner
//!    assignments, before that loader returns. Keep "application_capture".
//! No fixture visibility change, new fixture file or new production test API
//! is needed. Pending-registration assertions detect accidental certification
//! of an older input even though its house/session is healthy.
use super::super::*;
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use sims_runtime::exploration as engine;
use std::path::PathBuf;

use super::super::super::support::window_session::loaded_input::Account;

#[path = "../save_feedback/fixture.rs"]
mod fixture;

struct Temporary {
    path: PathBuf,
}

impl Temporary {
    fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sims-load-feedback-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&path)?;
        std::fs::create_dir(path.join("original-data"))?;
        Ok(Self { path })
    }

    fn root(&self) -> PathBuf {
        self.path.join("original-data")
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn bounded(control: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(control)?
        .join()
        .map_err(|_| anyhow::anyhow!("authored Load feedback control panicked"))?
}

fn envelope(viewer: &Viewer) -> Result<Value> {
    let session = viewer.session.as_ref().context("test household absent")?;
    Ok(json!({
        "session":session.save_managed()?,
        "dialog_client":viewer.client.as_ref().context("test client absent")?.save_managed(session)?,
        "camera_device":viewer.device.as_ref().map(|device| device.save(session)).transpose()?,
        "floor":viewer.floor,"zoom":viewer.zoom,"resident":viewer.resident,
        "selected":viewer.selected,"tutorial_highlight":viewer.tutorial_highlight,
        "viewport_enter_requested":viewer.viewport_enter_requested,"move_inputs":viewer.move_inputs,
    }))
}

/// Full saved engine/client/device/application owners plus transport and input
/// observation. Presentation feedback itself is deliberately outside this cut.
fn owners(viewer: &Viewer) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&json!({
        "envelope":envelope(viewer)?,"application":viewer.application,
        "revision":viewer.application_revision,
        "input":viewer.session.as_ref().unwrap().loaded_input_origin(),
        "playing":viewer.playing,"speed":viewer.speed,"accumulator":viewer.accumulator,
        "pan":viewer.pan,"shift":viewer.shift,
    }))?)
}

fn origin(viewer: &Viewer) -> Result<&sims_runtime::original_session::player_input::Observation> {
    viewer.session.as_ref().context("test household absent")?
        .loaded_input_origin().context("real input origin absent")
}

struct Household {
    _temp: Temporary,
    viewer: Viewer,
    account: Account,
    value: Value,
    raw: Vec<u8>,
}

impl Household {
    fn new(prepared: bool) -> Result<Self> {
        let temp = Temporary::new()?;
        let destination = temp.path.join("retry.json");
        let mut viewer = fixture::viewer(&temp.root(), &destination)?;
        let value = envelope(&viewer)?;
        let raw = serde_json::to_vec(&value)?;
        let initial = temp.path.join("current.json");
        std::fs::write(&initial, &raw)?;
        let index = viewer.session.as_ref().unwrap().index().clone();
        // Register an actually empty slot, then enter the real observed loader.
        // Assigning a typed fixture after registration would corrupt accounting.
        viewer.replace_session(None);
        viewer.client = None;
        viewer.device = None;
        let account = Account::default();
        assert!(viewer.loaded_input.is_none());
        viewer.loaded_input = Some(account.register());
        viewer.load_with_index(&initial, &index)?;
        if prepared {
            viewer.args.save = initial;
            drop(viewer.update(Message::Load));
            ensure!(viewer.error.is_none(), "fixture Load failed: {:?}", viewer.error);
            assert_eq!(viewer.loaded_input_capture()["eligible"], true);
        }
        viewer.args.save = destination;
        viewer.load_feedback = None;
        viewer.status = "Ready to load".into();
        Ok(Self { _temp: temp, viewer, account, value, raw })
    }

    fn write_retry(&mut self, raw: &[u8]) -> Result<()> {
        std::fs::write(&self.viewer.args.save, raw)?;
        Ok(())
    }
}

#[test]
fn load_preflight_real_missing_and_malformed_files_are_typed_and_skip_predicate() -> Result<()> {
    let temp = Temporary::new()?;
    for (name, raw) in [
        ("missing.json", None),
        ("truncated.json", Some(b"{".as_slice())),
        ("not-envelope.json", Some(b"[]".as_slice())),
        ("absent-session.json", Some(b"{}".as_slice())),
        ("bad-saved.json", Some(br#"{"session":{"version":"bad"}}"#.as_slice())),
    ] {
        let path = temp.path.join(name);
        if let Some(raw) = raw { std::fs::write(&path, raw)?; }
        let error = preflight(&path, |_| panic!("predicate ran before successful decode"))
            .err().context("invalid input admitted")?;
        assert!(matches!(error.downcast_ref::<PreflightFailure>().unwrap().operation, Operation::Read));
        if raw.is_none() {
            assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), std::io::ErrorKind::NotFound);
        }
        let feedback = recover(error.context("explicit Options Load"))?;
        assert!(feedback.contains(&path.display().to_string()));
        assert!(feedback.contains("read or decode"));
        assert!(feedback.contains("retry Load"));
    }
    Ok(())
}

#[test]
fn load_preflight_exact_house_identity_and_absent_original_are_not_normalized() -> Result<()> {
    bounded(house_identity_body)
}

fn house_identity_body() -> Result<()> {
    let mut household = Household::new(true)?;
    let exact = household.viewer.args.original.clone().context("original absent")?;
    assert_eq!(household.value["session"]["source"], exact);
    // Path equivalence, a different house and case-folding must not loosen the
    // precise Some(&saved.source) == args.original.as_ref() admission policy.
    for candidate in [
        Some(format!("./{exact}")), Some(exact.to_uppercase()),
        Some("UserData/Houses/House99.iff".into()), None,
    ] {
        let raw = household.raw.clone();
        household.write_retry(&raw)?;
        let mut calls = 0;
        let error = preflight(&household.viewer.args.save, |input| {
            calls += 1;
            Some(&input.saved().source) == candidate.as_ref()
        }).err().context("wrong house admitted")?;
        assert_eq!(calls, 1);
        assert!(matches!(error.downcast_ref::<PreflightFailure>().unwrap().operation, Operation::House));
        assert!(recover(error)?.contains("snapshot belongs to a different original house"));
    }
    Ok(())
}

#[test]
fn load_preflight_returns_same_owned_bytes_and_origin_even_if_path_is_replaced() -> Result<()> {
    bounded(same_input_body)
}

fn same_input_body() -> Result<()> {
    let mut household = Household::new(true)?;
    let mut raw = household.raw.clone(); raw.extend_from_slice(b" \n\t");
    household.write_retry(&raw)?;
    let path = household.viewer.args.save.clone();
    let expected = household.viewer.args.original.clone();
    let mut calls = 0;
    let input = preflight(&path, |input| {
        calls += 1;
        Some(&input.saved().source) == expected.as_ref()
    })?;
    assert_eq!(calls, 1);
    assert_eq!(input.raw_bytes(), raw);
    assert_eq!(input.observation().sha256, engine::digest(&raw));
    assert_eq!(input.observation().requested_path, path.to_string_lossy().as_ref());
    let observation = input.observation().clone();
    // Change the pathname AFTER preflight. Consuming the returned Input must
    // restore the original bytes and descriptor, without any second open.
    std::fs::remove_file(&path)?;
    std::fs::write(&path, b"replacement is not a player save")?;
    let index = household.viewer.session.as_ref().unwrap().index().clone();
    household.viewer.load_prepared_with_index(input, &index)?;
    assert_eq!(envelope(&household.viewer)?, household.value);
    assert_eq!(origin(&household.viewer)?, &observation);
    assert_eq!(std::fs::read(&path)?, b"replacement is not a player save");
    Ok(())
}

#[test]
fn load_recover_preserves_unmarked_io_decode_restore_native_and_tail_errors() -> Result<()> {
    for boundary in ["startup", "restore", "native preparation", "generation safety", "publication", "registration", "postcommit", "common tail"] {
        let message = format!("Load failed for retry.json: {boundary}; retry Load");
        let error = anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::NotFound, message))
            .context(boundary);
        let before = format!("{error:#}");
        let error = recover(error).expect_err("text/ordinary I/O must not authorize recovery");
        assert_eq!(format!("{error:#}"), before);
        assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), std::io::ErrorKind::NotFound);
    }
    let error = serde_json::from_slice::<Value>(b"{").unwrap_err();
    assert!(recover(anyhow::Error::new(error).context("application-field decode")).is_err());
    Ok(())
}

fn reject(household: &mut Household) -> Result<String> {
    let viewer = &mut household.viewer;
    let before = owners(viewer)?;
    let times = (viewer.last, viewer.clock_origin);
    let observed = origin(viewer)?.clone();
    let account = household.account.inspect_for_test();
    // This existing test boundary must never be reached for input rejection.
    viewer.loaded_input_test_fault = Some("ui_load_common_prepare");
    drop(viewer.update(Message::Load));
    viewer.loaded_input_test_fault = None;
    ensure!(viewer.error.is_none(), "recoverable Load became fatal: {:?}", viewer.error);
    assert_eq!(owners(viewer)?, before, "rejected Load changed current owners");
    assert_eq!((viewer.last, viewer.clock_origin), times);
    assert_eq!(origin(viewer)?, &observed);
    let after = household.account.inspect_for_test();
    for field in ["transition_ordinal", "accounted_slot", "last_recorded_owner", "invalidation_reasons"] {
        assert_eq!(after[field], account[field], "rejection changed {field}");
    }
    let feedback = viewer.load_feedback.clone().context("dedicated Load feedback absent")?;
    assert!(feedback.contains("Load failed"));
    assert!(feedback.contains(&viewer.args.save.display().to_string()));
    assert!(feedback.contains("retry Load"));
    assert!(!feedback.contains("Saved household restored"));
    assert_eq!(ui_publication::Published::capture(viewer).status, feedback);
    Ok(feedback)
}

#[test]
fn load_message_rejects_missing_malformed_and_wrong_house_then_retries_same_path() -> Result<()> {
    bounded(rejection_retry_body)
}

fn rejection_retry_body() -> Result<()> {
    for invalid in ["missing", "malformed", "wrong-house"] {
        let mut household = Household::new(true)?;
        match invalid {
            "malformed" => household.write_retry(b"{")?,
            "wrong-house" => {
                let mut value = household.value.clone();
                value["session"]["source"] = json!("UserData/Houses/House99.iff");
                household.write_retry(&serde_json::to_vec(&value)?)?;
            }
            _ => {}
        }
        // Exercise the real Options > Load dispatch, without a native UI claim.
        drop(household.viewer.update(Message::Options));
        assert!(household.viewer.options_open);
        let feedback = reject(&mut household)?;
        if invalid == "wrong-house" { assert!(feedback.contains("different original house")); }
        let retained_origin = origin(&household.viewer)?.clone();
        household.viewer.prepare_and_publish()?;
        drop(household.viewer.update(Message::Frame(Instant::now())));
        assert!(household.viewer.error.is_none());
        assert_eq!(household.viewer.load_feedback.as_ref(), Some(&feedback));
        assert_eq!(ui_publication::Published::capture(&household.viewer).status, feedback);
        assert_eq!(origin(&household.viewer)?, &retained_origin);
        assert_eq!(household.viewer.loaded_input_capture()["eligible"], true);
        // A normal player command must still work on the retained household.
        drop(household.viewer.update(Message::Toggle));
        assert!(household.viewer.error.is_none() && household.viewer.playing);
        drop(household.viewer.update(Message::Toggle));
        assert!(household.viewer.error.is_none() && !household.viewer.playing);
        let ordinal = household.account.inspect_for_test()["transition_ordinal"].as_u64().unwrap();
        let mut retry = household.value.clone(); retry["zoom"] = json!(0.75);
        let mut raw = serde_json::to_vec(&retry)?; raw.extend_from_slice(b" \n");
        household.write_retry(&raw)?;
        drop(household.viewer.update(Message::Options));
        assert!(household.viewer.options_open);
        drop(household.viewer.update(Message::Load));
        ensure!(household.viewer.error.is_none(), "valid retry failed: {:?}", household.viewer.error);
        assert_eq!(household.viewer.zoom, 0.75);
        assert_eq!(origin(&household.viewer)?.sha256, engine::digest(&raw));
        assert_eq!(origin(&household.viewer)?.requested_path, household.viewer.args.save.to_string_lossy().as_ref());
        assert_eq!(household.account.inspect_for_test()["transition_ordinal"], ordinal + 1);
        assert_eq!(household.viewer.loaded_input_capture()["eligible"], true);
        let feedback = household.viewer.load_feedback.as_ref().context("Load success feedback absent")?;
        assert!(feedback.contains("Saved household restored"));
        assert!(!feedback.contains("Load failed"));
        assert_eq!(ui_publication::Published::capture(&household.viewer).status, *feedback);
    }
    Ok(())
}

#[test]
fn load_rejection_must_not_certify_a_previous_pending_registration() -> Result<()> {
    bounded(|| {
        let mut household = Household::new(false)?;
        assert_eq!(household.account.inspect_for_test()["accounted_slot"]["install"], "pending");
        reject(&mut household)?;
        assert_eq!(household.account.inspect_for_test()["accounted_slot"]["install"], "pending");
        assert_eq!(household.viewer.loaded_input_capture()["eligible"], false);
        Ok(())
    })
}

#[test]
fn load_message_refuses_existing_application_menu_and_vm_faults_without_healing() -> Result<()> {
    bounded(|| {
        for guard in ["application", "menu", "VM"] {
            for valid in [false, true] {
                let mut household = Household::new(true)?;
                if valid { let raw = household.raw.clone(); household.write_retry(&raw)?; }
                match guard {
                    "application" => household.viewer.fail("existing application fault"),
                    "menu" => household.viewer.menu_error = Some("existing menu fault".into()),
                    _ => household.viewer.session.as_mut().unwrap().simulation.vm.threads.values_mut()
                        .next().context("authored VM thread absent")?.status = sims_vm::Status::Faulted(sims_vm::Fault {
                            message: "existing VM fault".into(), stack: vec![],
                        }),
                }
                let saved = serde_json::to_vec(&household.viewer.session.as_ref().unwrap().save())?;
                let observed = origin(&household.viewer)?.clone();
                let ordinal = household.account.inspect_for_test()["transition_ordinal"].clone();
                drop(household.viewer.update(Message::Load));
                assert!(household.viewer.error.is_some(), "{guard}, valid={valid}");
                assert!(!household.viewer.playing);
                assert!(household.viewer.load_feedback.is_none(), "fault masked by recoverable feedback");
                assert_eq!(origin(&household.viewer)?, &observed);
                assert_eq!(household.account.inspect_for_test()["transition_ordinal"], ordinal);
                assert_eq!(serde_json::to_vec(&household.viewer.session.as_ref().unwrap().save())?, saved);
                if guard == "application" {
                    assert!(household.viewer.error.as_deref().unwrap().contains("existing application fault"));
                }
            }
        }
        Ok(())
    })
}

#[test]
fn load_message_restore_and_late_application_decodes_remain_fatal() -> Result<()> {
    bounded(|| {
        for field in ["restore", "dialog_client", "floor", "move_inputs"] {
            let mut household = Household::new(true)?;
            let before = owners(&household.viewer)?;
            let mut value = household.value.clone();
            match field {
                "restore" => value["session"]["version"] = json!(999),
                "dialog_client" => value["dialog_client"]["version"] = json!(999),
                "floor" => value["floor"] = json!(9),
                _ => value["move_inputs"] = json!("not an input queue"),
            }
            household.write_retry(&serde_json::to_vec(&value)?)?;
            // Prove the error lies AFTER this helper's successful admission.
            let expected = household.viewer.args.original.as_ref();
            drop(preflight(&household.viewer.args.save, |input| Some(&input.saved().source) == expected)?);
            drop(household.viewer.update(Message::Load));
            assert!(household.viewer.error.is_some(), "{field} incorrectly recovered");
            assert!(household.viewer.load_feedback.is_none());
            assert_eq!(owners(&household.viewer)?, before, "{field} committed an owner before staging completed");
            assert_eq!(household.viewer.loaded_input_capture()["eligible"], false);
        }
        Ok(())
    })
}

#[test]
fn load_message_native_precommit_postcommit_and_common_tail_errors_remain_fatal() -> Result<()> {
    bounded(|| {
        for fault in ["load_native_prepare", "application_capture", "load_postcommit", "ui_load_common_prepare"] {
            let mut household = Household::new(true)?;
            let old = origin(&household.viewer)?.clone();
            let before = household.account.inspect_for_test()["transition_ordinal"].as_u64().unwrap();
            let mut raw = household.raw.clone(); raw.extend_from_slice(b" \n");
            household.write_retry(&raw)?;
            household.viewer.loaded_input_test_fault = Some(fault);
            drop(household.viewer.update(Message::Load));
            assert!(household.viewer.error.as_deref().is_some_and(|error| error.contains(fault)), "{fault}");
            assert!(!household.viewer.playing);
            assert!(household.viewer.load_feedback.is_none(), "success published before {fault}");
            let capture = household.account.inspect_for_test();
            assert_eq!(household.viewer.loaded_input_capture()["eligible"], false);
            if matches!(fault, "load_postcommit" | "ui_load_common_prepare") {
                assert_eq!(origin(&household.viewer)?.sha256, engine::digest(&raw));
                assert_eq!(capture["transition_ordinal"], before + 1);
                assert_eq!(capture["accounted_slot"]["install"], "failed");
            } else {
                assert_eq!(origin(&household.viewer)?, &old);
                assert_eq!(capture["transition_ordinal"], before);
            }
        }
        Ok(())
    })
}

#[test]
fn load_worker_publication_and_post_update_failures_keep_old_frontend_and_failed_new_origin() -> Result<()> {
    bounded(|| {
        for fault in ["post_update_popup_counter", "worker_publication"] {
            let mut household = Household::new(true)?;
            let mut raw = household.raw.clone(); raw.extend_from_slice(b" \n");
            household.write_retry(&raw)?;
            household.viewer.loaded_input_test_fault = Some(fault);
            let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
            let old = host.publication_for_test().clone();
            let old_status = old.status.clone();
            drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
            let error = host.settle_for_test().err().context("worker failure incorrectly recovered")?;
            assert!(format!("{error:#}").contains(fault));
            assert!(Arc::ptr_eq(&old, host.publication_for_test()));
            assert_eq!(host.publication_for_test().status, old_status);
            let owner = host.recover_for_test();
            assert_eq!(origin(&owner)?.sha256, engine::digest(&raw));
            drop(owner);
            let terminal = household.account.terminal();
            assert_eq!(terminal["accounted_slot"]["input"]["sha256"], engine::digest(&raw));
            assert_eq!(terminal["accounted_slot"]["install"], "failed");
            assert_eq!(terminal["eligible"], false);
            // The immutable old frontend never receives either a retryable
            // input rejection or success for this failed publication.
            assert_eq!(old.status, old_status);
        }
        Ok(())
    })
}

#[test]
fn load_startup_missing_or_malformed_input_keeps_fatal_semantics() -> Result<()> {
    bounded(|| {
        for malformed in [false, true] {
            let mut household = Household::new(true)?;
            if malformed { household.write_retry(b"{")?; }
            household.viewer.args.load = Some(household.viewer.args.save.clone());
            let index = household.viewer.session.as_ref().unwrap().index().clone();
            let ready = Arc::new(Ready {
                restore_index: Some(index.clone()), session: Mutex::new(None),
                client: Mutex::new(None), device: Mutex::new(None),
                icons: Arc::new(dialog_view::Icons::load(&index)?),
            });
            drop(household.viewer.update(Message::Loaded(Ok(ready))));
            assert!(household.viewer.error.is_some());
            assert!(!household.viewer.playing);
            assert!(household.viewer.load_feedback.is_none());
            assert_eq!(household.viewer.loaded_input_capture()["eligible"], false);
        }
        Ok(())
    })
}

#[test]
fn load_feedback_lifetime_ends_on_owner_replacement_and_fatal_error() -> Result<()> {
    bounded(|| {
        for fail in [false, true] {
            let mut household = Household::new(true)?;
            reject(&mut household)?;
            if fail { household.viewer.fail("subsequent fatal owner error"); }
            else { household.viewer.replace_session(None); }
            assert!(household.viewer.load_feedback.is_none());
        }
        Ok(())
    })
}

#[test]
fn load_recovered_household_space_reaches_actual_worker_transport() -> Result<()> {
    bounded(|| {
        use iced::{Event, event::Status, keyboard::{self, Key, Location, Modifiers, key::{Code, Named, Physical}}};
        let mut household = Household::new(true)?;
        reject(&mut household)?;
        let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
        // The shared Save fixture opens Diagnostics. Close that real overlay
        // before expecting a gameplay shortcut to reach transport.
        let close = host.ui_event_for_test(host.publication_for_test(), Message::Diagnostics);
        drop(host.update(close)); host.settle_for_test()?;
        let physical_key = Physical::Code(Code::Space);
        let down = Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Space), modified_key: Key::Named(Named::Space), physical_key,
            location: Location::Standard, modifiers: Modifiers::NONE, text: Some(" ".into()), repeat: false,
        });
        let up = Event::Keyboard(keyboard::Event::KeyReleased {
            key: Key::Named(Named::Space), modified_key: Key::Named(Named::Space), physical_key,
            location: Location::Standard, modifiers: Modifiers::NONE,
        });
        let send = |host: &mut worker_host::Host, event: Event, status: Status| -> Result<()> {
            let message = subscription_event(event, status, window::Id::unique()).context("Space observation absent")?;
            let command = host.ui_event_for_test(host.publication_for_test(), message);
            drop(host.update(command)); host.settle_for_test()?; Ok(())
        };
        assert!(!host.publication_for_test().playing);
        send(&mut host, down.clone(), Status::Ignored)?;
        assert!(host.publication_for_test().playing, "Space never reached loaded transport");
        send(&mut host, down.clone(), Status::Ignored)?;
        assert!(host.publication_for_test().playing, "held duplicate toggled twice");
        send(&mut host, up, Status::Captured)?;
        send(&mut host, down, Status::Ignored)?;
        assert!(!host.publication_for_test().playing);
        let viewer = host.recover_for_test();
        assert!(viewer.error.is_none());
        assert!(viewer.load_feedback.is_some(), "transport erased retained Load rejection");
        Ok(())
    })
}

#[test]
fn load_host_options_rejection_retains_owner_then_valid_retry_replaces_it() -> Result<()> {
    bounded(|| {
        let household = Household::new(true)?;
        let old = origin(&household.viewer)?.clone();
        let destination = household.viewer.args.save.clone();
        let raw = household.raw.clone();
        let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
        let send = |host: &mut worker_host::Host, message| -> Result<()> {
            let event = host.ui_event_for_test(host.publication_for_test(), message);
            drop(host.update(event)); host.settle_for_test()?; Ok(())
        };
        send(&mut host, Message::Options)?;
        send(&mut host, Message::Load)?;
        assert!(host.publication_for_test().error.is_none());
        assert!(host.publication_for_test().status.contains("retry Load"));
        let captured = host.loaded_input_publication_for_test().context("input publication absent")?;
        assert!(serde_json::to_string(&captured)?.contains(&old.sha256));
        send(&mut host, Message::Options)?;
        send(&mut host, Message::Toggle)?;
        assert!(host.publication_for_test().playing);
        send(&mut host, Message::Toggle)?;
        std::fs::write(&destination, &raw)?;
        send(&mut host, Message::Options)?;
        send(&mut host, Message::Load)?;
        assert!(host.publication_for_test().error.is_none());
        assert!(host.publication_for_test().status.contains("Saved household restored"));
        let viewer = host.recover_for_test();
        assert_eq!(origin(&viewer)?.sha256, engine::digest(&raw));
        Ok(())
    })
}

#[test]
fn load_household_selected_popup_blocks_space_in_real_publication_and_dispatch() -> Result<()> {
    bounded(|| {
        let mut household = Household::new(true)?;
        household.viewer.selected = Some(household.viewer.resident);
        let published = ui_publication::Published::capture(&household.viewer);
        assert!(published.menu.as_ref().is_some_and(|menu| menu.popup_visible));
        assert!(published.player_shortcuts_blocked);
        let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
        let input = player_shortcuts::Input::SpacePressed {
            physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::Space),
            captured: false, modified: false, repeat: false,
        };
        let event = host.ui_event_for_test(host.publication_for_test(), Message::PlayerShortcut(input));
        drop(host.update(event)); host.settle_for_test()?;
        assert!(!host.publication_for_test().playing);
        assert!(host.player_shortcut_queue_for_test().is_empty());
        drop(host.recover_for_test());
        Ok(())
    })
}

fn player_controls_escape(status: iced::event::Status) -> Option<Message> {
    player_controls_escape_press(status, false)
}

fn player_controls_escape_press(status: iced::event::Status, repeat: bool) -> Option<Message> {
    use iced::{Event, keyboard::{self, Key, Location, Modifiers, key::{Code, Named, Physical}}};
    subscription_event(
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape), modified_key: Key::Named(Named::Escape),
            physical_key: Physical::Code(Code::Escape), location: Location::Standard,
            modifiers: Modifiers::NONE, text: None, repeat,
        }),
        status,
        window::Id::unique(),
    )
}

fn player_controls_send(host: &mut worker_host::Host, message: Message) -> Result<()> {
    let event = host.ui_event_for_test(host.publication_for_test(), message);
    drop(host.update(event));
    host.settle_for_test()?;
    ensure!(host.publication_for_test().error.is_none(), "player control failed: {:?}", host.publication_for_test().error);
    Ok(())
}

#[test]
fn player_controls_escape_closes_fallback_menu_through_actual_subscription_and_worker() -> Result<()> {
    bounded(|| {
        let mut household = Household::new(true)?;
        household.viewer.diagnostics = false;
        let resident = household.viewer.resident;
        let old_origin = origin(&household.viewer)?.clone();
        let before = serde_json::to_vec(&household.viewer.session.as_ref().unwrap().save_managed()?)?;
        let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
        player_controls_send(&mut host, Message::Select(Some(resident)))?;
        assert!(host.publication_for_test().menu.as_ref().is_some_and(|menu| menu.preview && menu.popup_visible));
        let publication = host.publication_for_test().clone();
        let captured = player_controls_escape(iced::event::Status::Captured)
            .context("picker-only Escape candidate absent")?;
        assert!(matches!(&captured, Message::PickerEscape));
        player_controls_send(&mut host, captured)?;
        assert!(Arc::ptr_eq(&publication, host.publication_for_test()),
            "captured Escape without a picker borrowed or published the owner");
        assert!(host.publication_for_test().menu.as_ref()
            .is_some_and(|menu| menu.preview && menu.popup_visible));
        player_controls_send(&mut host, player_controls_escape(iced::event::Status::Ignored).context("Escape observation absent")?)?;
        assert!(host.publication_for_test().menu.is_none());
        let viewer = host.recover_for_test();
        assert!(viewer.selected.is_none());
        assert_eq!(origin(&viewer)?, &old_origin);
        assert_eq!(serde_json::to_vec(&viewer.session.as_ref().unwrap().save_managed()?)?, before);
        assert!(!viewer.playing);
        Ok(())
    })
}

#[test]
fn player_controls_escape_closes_responsive_options_and_rejects_stale_generation() -> Result<()> {
    bounded(|| {
        let mut household = Household::new(true)?;
        household.viewer.diagnostics = false;
        let old_origin = origin(&household.viewer)?.clone();
        let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
        let publication = host.publication_for_test().clone();
        player_controls_send(&mut host, Message::Options)?;
        assert!(host.frontend_for_test().is_err());
        let generation = host.current_generation_for_test();
        drop(host.update(worker_host::Event::Widget {
            generation: generation + 1,
            message: player_controls_escape(iced::event::Status::Ignored).unwrap(),
        }));
        assert!(host.frontend_for_test().is_err(), "stale Escape closed current Options");
        player_controls_send(&mut host, player_controls_escape(iced::event::Status::Ignored).unwrap())?;
        assert!(host.frontend_for_test().is_ok());
        assert!(host.frontend_idle_for_test());
        assert!(Arc::ptr_eq(&publication, host.publication_for_test()), "shell-only close borrowed/published the owner");
        let viewer = host.recover_for_test();
        assert!(!viewer.options_open);
        assert_eq!(origin(&viewer)?, &old_origin);
        Ok(())
    })
}

#[test]
fn player_controls_escape_cancels_confirmation_before_closing_options_in_both_adapters() -> Result<()> {
    bounded(|| {
        for threaded in [false, true] {
            let mut household = Household::new(true)?;
            household.viewer.diagnostics = false;
            let before = serde_json::to_vec(&household.viewer.application)?;
            let policy = household.viewer.args.neighborhood_policy.clone();
            if threaded {
                let mut host = worker_host::Host::from_viewer_for_test(household.viewer);
                player_controls_send(&mut host, Message::Options)?;
                player_controls_send(&mut host, Message::RequestApplicationChoice(neighborhood_usability::Choice::InstalledHouses))?;
                player_controls_send(&mut host, player_controls_escape(iced::event::Status::Ignored).unwrap())?;
                assert!(host.frontend_for_test().is_err(), "first Escape also closed underlying Options");
                assert!(player_controls_escape_press(iced::event::Status::Ignored, true).is_none(),
                    "held Escape reached underlying Options");
                player_controls_send(&mut host, player_controls_escape(iced::event::Status::Ignored).unwrap())?;
                assert!(host.frontend_for_test().is_ok(), "confirmation retained input after Escape");
                household.viewer = host.recover_for_test();
            } else {
                drop(household.viewer.update(Message::Options));
                drop(household.viewer.update(Message::RequestApplicationChoice(neighborhood_usability::Choice::InstalledHouses)));
                assert!(household.viewer.application_confirmation.is_some());
                drop(household.viewer.update(player_controls_escape(iced::event::Status::Ignored).unwrap()));
                assert!(household.viewer.application_confirmation.is_none());
                assert!(household.viewer.options_open);
                assert!(player_controls_escape_press(iced::event::Status::Ignored, true).is_none(),
                    "held Escape reached underlying Options");
                drop(household.viewer.update(player_controls_escape(iced::event::Status::Ignored).unwrap()));
            }
            assert!(!household.viewer.options_open);
            assert_eq!(serde_json::to_vec(&household.viewer.application)?, before);
            assert_eq!(household.viewer.args.neighborhood_policy, policy);
            assert!(household.viewer.session.is_some());
        }
        Ok(())
    })
}


#[test]
fn player_controls_feedback_publication_contains_only_retained_receipts() -> Result<()> {
    bounded(|| {
        let mut household = Household::new(true)?;
        household.viewer.status = "Transient simulation progress".into();
        assert!(ui_publication::Published::capture(&household.viewer).save_load_feedback.is_none());
        let feedback = reject(&mut household)?;
        assert_eq!(ui_publication::Published::capture(&household.viewer).save_load_feedback.as_deref(), Some(feedback.as_str()));
        household.viewer.fail("subsequent fatal owner error");
        assert!(ui_publication::Published::capture(&household.viewer).save_load_feedback.is_none());
        Ok(())
    })
}

#[test]
#[ignore = "requires an offscreen Iced GPU adapter; no native-window claim"]
fn player_controls_feedback_is_rendered_with_options_closed_in_both_views() -> Result<()> {
    bounded(|| {
        let mut input = headless_input::Input::new()?;
        for operation in [0, 1, 2] {
            let mut household = Household::new(true)?;
            match operation {
                0 => { reject(&mut household)?; }
                1 => { drop(household.viewer.update(Message::Save)); }
                _ => {
                    household.viewer.args.save = household.viewer.args.root.join("protected-save.json");
                    drop(household.viewer.update(Message::Save));
                }
            }
            assert!(household.viewer.error.is_none());
            let receipt = household.viewer.retained_feedback().context("real Save/Load receipt absent")?.to_owned();
            household.viewer.diagnostics = false;
            household.viewer.options_open = false;
            let publication = ui_publication::Published::capture(&household.viewer);
            let hud_height = publication.hud_height;
            let frontend = ui_publication::view::Frontend {
                data: Arc::new(publication), diagnostics: false, options_open: false, widget_epoch: 0,
            };
            for direct in [false, true] {
                input.reset();
                let view = if direct { household.viewer.view() } else { frontend.view() };
                let observation = input.element_observation(view, [1320, 900])?;
                let visible = observation.texts.iter().find(|text| text.text == receipt)
                    .context("retained receipt missing from actual play widget text")?;
                assert!(visible.bounds.width > 0. && visible.bounds.height > 0.);
                assert!(visible.bounds.y >= 0. && visible.bounds.y + visible.bounds.height <= 900. - hud_height as f32);
            }
        }
        Ok(())
    })
}
