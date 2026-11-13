//! Real temporary outputs and the actual Viewer::update(Message::Save) route.
//! These controls use authored, owned resources; no installed game. The
//! separately opted-in options visibility control uses a headless GPU.
use super::super::*;
use super::*;
use clap::Parser;
use serde_json::{Value, json};
use sims_runtime::exploration as engine;

#[path = "fixture.rs"]
mod fixture;

#[test]
fn save_feedback_retains_output_result_across_refresh_and_ends_with_owner() -> Result<()> {
    bounded(|| {
        let temp = Temporary::new()?;
        let blocker = temp.path.join("blocked-retained-result");
        std::fs::write(&blocker, b"owned test blocker")?;
        let destination = blocker.join("household.json");
        let mut viewer = fixture::viewer(&temp.root(), &destination)?;
        drop(viewer.update(Message::Save));
        let result = viewer
            .save_feedback
            .clone()
            .context("Save outcome absent")?;
        assert!(result.contains("Save failed"));
        viewer.prepare_and_publish()?;
        drop(viewer.update(Message::Frame(Instant::now())));
        assert_eq!(viewer.save_feedback.as_ref(), Some(&result));
        assert_eq!(ui_publication::Published::capture(&viewer).status, result);
        viewer.replace_session(None);
        assert!(viewer.save_feedback.is_none());
        Ok(())
    })
}

#[test]
#[ignore = "synthetic Save options; opt-in headless Iced GPU, no private assets"]
fn save_feedback_synthetic_options_show_failure_and_success_in_play_view() -> Result<()> {
    bounded(|| {
        let temp = Temporary::new()?;
        let blocker = temp.path.join("blocked-options-parent");
        std::fs::write(&blocker, b"owned test blocker")?;
        let destination = blocker.join("household.json");
        let mut viewer = fixture::viewer(&temp.root(), &destination)?;
        let mut input = headless_input::Input::new()?;
        for retry in [false, true] {
            if retry {
                std::fs::remove_file(&blocker)?;
            }
            drop(viewer.update(Message::Save));
            assert!(viewer.error.is_none());
            let result = viewer
                .save_feedback
                .clone()
                .context("Save outcome absent")?;
            // Ordinary owner refreshes and frame messages must not erase the
            // output result before the player can read it.
            viewer.prepare_and_publish()?;
            drop(viewer.update(Message::Frame(Instant::now())));
            let frontend = ui_publication::view::Frontend {
                data: std::sync::Arc::new(ui_publication::Published::capture(&viewer)),
                diagnostics: false,
                options_open: true,
                widget_epoch: viewer.widget_epoch,
            };
            let observation = input.element_observation(frontend.view(), [1024, 768])?;
            let shown = observation
                .texts
                .iter()
                .find(|text| text.text == result)
                .context("normal application options must show the actual Save result")?;
            assert!(shown.bounds.height > 0. && shown.bounds.width > 0.);
            if retry {
                assert!(result.starts_with("Saved "));
                assert!(destination.is_file());
            } else {
                assert!(result.contains("Save failed"));
                assert!(result.contains("retry Save"));
                assert!(!destination.exists());
            }
        }
        Ok(())
    })
}

struct Temporary {
    path: PathBuf,
}

impl Temporary {
    fn new() -> Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sims-save-feedback-{}-{stamp}", std::process::id()));
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

// Managed restore/capture has large retained values, as in loaded_input_tests.
fn bounded(control: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(control)?
        .join()
        .map_err(|_| anyhow::anyhow!("Save feedback control panicked"))?
}

fn owners(viewer: &Viewer) -> Result<Vec<u8>> {
    let session = viewer.session.as_ref().context("test household absent")?;
    Ok(serde_json::to_vec(&json!({
        "session":session.save(),
        "dialog_client":viewer.client.as_ref().context("test client absent")?.save(session)?,
        "camera_device":viewer.device.as_ref().map(|device| device.save(session)).transpose()?,
        "floor":viewer.floor,"zoom":viewer.zoom,"resident":viewer.resident,
        "selected":viewer.selected,"tutorial_highlight":viewer.tutorial_highlight,
        "viewport_enter_requested":viewer.viewport_enter_requested,"move_inputs":viewer.move_inputs,
    }))?)
}

fn dispatch_recoverable(viewer: &mut Viewer, destination: &Path) -> Result<()> {
    let before = owners(viewer)?;
    let transport = (
        viewer.playing,
        viewer.accumulator,
        viewer.last,
        viewer.clock_origin,
    );
    let revision = viewer.application_revision;
    drop(viewer.update(Message::Save));
    assert!(
        viewer.error.is_none(),
        "unexpected fatal error: {:?}",
        viewer.error
    );
    assert_eq!(
        owners(viewer)?,
        before,
        "output failure changed a retained owner"
    );
    assert_eq!(
        transport,
        (
            viewer.playing,
            viewer.accumulator,
            viewer.last,
            viewer.clock_origin
        )
    );
    assert_eq!(viewer.application_revision, revision);
    assert!(viewer.status.contains("Save failed"), "{}", viewer.status);
    assert!(viewer.status.contains(&destination.display().to_string()));
    assert!(viewer.status.contains("retry Save"));
    assert!(!viewer.status.starts_with("Saved "));
    assert!(
        viewer.scene.is_some(),
        "Save must still refresh and publish"
    );
    assert!(
        viewer.hud_cluster.is_some(),
        "update lifecycle publication was skipped"
    );
    Ok(())
}

#[test]
fn output_write_creates_directories_and_replaces_a_real_file_only_on_success() -> Result<()> {
    let temp = Temporary::new()?;
    let destination = temp.path.join("new-directory/household.json");
    write(&temp.root(), &destination, b"first complete save")?;
    assert_eq!(std::fs::read(&destination)?, b"first complete save");
    write(&temp.root(), &destination, b"second complete save")?;
    assert_eq!(std::fs::read(&destination)?, b"second complete save");
    assert_eq!(std::fs::read_dir(destination.parent().unwrap())?.count(), 1);
    Ok(())
}

#[test]
fn validation_preserves_protected_existing_file_and_io_errors_are_not_blanket_recovered()
-> Result<()> {
    let temp = Temporary::new()?;
    let destination = temp.root().join("original.iff");
    std::fs::write(&destination, b"known-good original bytes")?;
    let error = write(&temp.root(), &destination, b"replacement").unwrap_err();
    assert!(matches!(
        error.downcast_ref::<OutputFailure>().unwrap().operation,
        Operation::Validate
    ));
    assert!(recover(error)?.contains("No save was written"));
    assert_eq!(std::fs::read(destination)?, b"known-good original bytes");

    // The very same low-level error kind can originate in state/resource
    // capture. An unmarked I/O error must remain fatal, even with context.
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "cannot read engine resource",
    ))
    .context("managed state capture failed");
    let error = recover(error).expect_err("unmarked engine I/O must fall through");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(format!("{error:#}").contains("managed state capture failed"));
    Ok(())
}

#[test]
fn save_message_recovers_output_failures_and_retries_the_same_destination() -> Result<()> {
    bounded(output_failures_body)
}

fn output_failures_body() -> Result<()> {
    let temp = Temporary::new()?;
    // Validation failure on an existing protected original file.
    let protected = temp.root().join("last-known-good.json");
    std::fs::write(&protected, b"last-known-good")?;
    let mut viewer = fixture::viewer(&temp.root(), &protected)?;
    dispatch_recoverable(&mut viewer, &protected)?;
    assert!(viewer.status.contains("destination validation failed"));
    assert_eq!(std::fs::read(&protected)?, b"last-known-good");

    // Directory creation fails deterministically without relying on chmod or
    // disk exhaustion. Fix that filesystem obstacle and retry the SAME path.
    let blocker = temp.path.join("blocked-parent");
    std::fs::write(&blocker, b"existing unrelated file")?;
    let destination = blocker.join("household.json");
    viewer.args.save = destination.clone();
    dispatch_recoverable(&mut viewer, &destination)?;
    assert!(viewer.status.contains("destination directory"));
    assert_eq!(std::fs::read(&blocker)?, b"existing unrelated file");
    assert!(!destination.exists());
    std::fs::remove_file(&blocker)?;
    let before = owners(&viewer)?;
    let transport = (
        viewer.playing,
        viewer.accumulator,
        viewer.last,
        viewer.clock_origin,
    );
    drop(viewer.update(Message::Save));
    assert!(viewer.error.is_none(), "{:?}", viewer.error);
    let bytes = std::fs::read(&destination)?;
    let saved: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(saved["session"]["version"], 2);
    assert!(saved["session"]["managed"].is_object());
    assert_eq!(
        saved["session"],
        serde_json::to_value(viewer.session.as_ref().unwrap().save_managed()?)?
    );
    assert_eq!(
        saved["dialog_client"],
        serde_json::to_value(
            viewer
                .client
                .as_ref()
                .unwrap()
                .save_managed(viewer.session.as_ref().unwrap())?
        )?
    );
    assert_eq!(viewer.status, format!("Saved {}", destination.display()));
    assert_eq!(owners(&viewer)?, before);
    assert_eq!(
        transport,
        (
            viewer.playing,
            viewer.accumulator,
            viewer.last,
            viewer.clock_origin
        )
    );
    assert_eq!(
        std::fs::read_dir(&blocker)?.count(),
        1,
        "temporary output leaked"
    );
    assert_eq!(std::fs::read(&protected)?, b"last-known-good");
    Ok(())
}

#[test]
fn save_message_recovers_atomic_rename_failure_without_claiming_write_completion() -> Result<()> {
    bounded(atomic_failure_body)
}

fn atomic_failure_body() -> Result<()> {
    let temp = Temporary::new()?;
    let destination = temp.path.join("household.json");
    std::fs::create_dir(&destination)?;
    let sentinel = destination.join("known-good");
    std::fs::write(&sentinel, b"keep this directory and its contents")?;
    let mut viewer = fixture::viewer(&temp.root(), &destination)?;
    dispatch_recoverable(&mut viewer, &destination)?;
    assert!(viewer.status.contains("output write failed"));
    assert!(viewer.status.contains("Write completion was not confirmed"));
    assert_eq!(
        std::fs::read(&sentinel)?,
        b"keep this directory and its contents"
    );
    assert_eq!(
        std::fs::read_dir(&temp.path)?.count(),
        2,
        "atomic temporary file leaked"
    );
    std::fs::remove_file(sentinel)?;
    std::fs::remove_dir(&destination)?;
    drop(viewer.update(Message::Save));
    assert!(viewer.error.is_none());
    assert!(
        serde_json::from_slice::<Value>(&std::fs::read(&destination)?)?["session"]["managed"]
            .is_object()
    );
    assert_eq!(viewer.status, format!("Saved {}", destination.display()));
    Ok(())
}

#[test]
fn save_message_preserves_application_menu_and_faulted_vm_guards() -> Result<()> {
    bounded(guards_body)
}

fn guards_body() -> Result<()> {
    let temp = Temporary::new()?;
    let destination = temp.path.join("known-good.json");
    std::fs::write(&destination, b"last-known-good player save")?;
    for guard in ["application error", "menu error", "faulted VM"] {
        let mut viewer = fixture::viewer(&temp.root(), &destination)?;
        match guard {
            "application error" => viewer.fail("existing application error"),
            "menu error" => viewer.menu_error = Some("existing menu fault".into()),
            _ => {
                viewer
                    .session
                    .as_mut()
                    .unwrap()
                    .simulation
                    .vm
                    .threads
                    .values_mut()
                    .next()
                    .unwrap()
                    .status = sims_vm::Status::Faulted(sims_vm::Fault {
                    message: "existing VM fault".into(),
                    stack: vec![],
                })
            }
        }
        let before = serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?;
        drop(viewer.update(Message::Save));
        assert!(
            viewer.error.as_deref().unwrap().contains(guard),
            "{:?}",
            viewer.error
        );
        assert!(!viewer.playing && viewer.accumulator == 0.);
        assert_eq!(
            serde_json::to_vec(&viewer.session.as_ref().unwrap().save())?,
            before
        );
        assert_eq!(std::fs::read(&destination)?, b"last-known-good player save");
        assert!(viewer.scene.is_none(), "Save guard must precede refresh");
        assert!(
            !viewer.status.contains("Save failed"),
            "guard incorrectly recovered as output"
        );
    }
    Ok(())
}

#[test]
fn save_message_keeps_refresh_and_managed_capture_failures_fatal_before_any_output() -> Result<()> {
    bounded(preparation_failures_body)
}

fn preparation_failures_body() -> Result<()> {
    let temp = Temporary::new()?;
    let destination = temp.path.join("missing-parent/known-good.json");
    let mut viewer = fixture::viewer(&temp.root(), &destination)?;
    viewer.client = None;
    drop(viewer.update(Message::Save));
    assert!(
        viewer
            .error
            .as_deref()
            .unwrap()
            .contains("dialog client absent")
    );
    assert!(!viewer.playing && viewer.accumulator == 0.);
    assert!(
        !destination.parent().unwrap().exists(),
        "refresh error touched output"
    );

    let mut viewer = fixture::viewer(&temp.root(), &destination)?;
    let source = viewer.session.as_ref().unwrap().house.source.clone();
    viewer
        .session
        .as_mut()
        .unwrap()
        .house
        .sources
        .insert(source, "changed import digest".into());
    // An invalid destination must not mask the genuine capture failure.
    viewer.args.save = temp.root().join("protected.json");
    std::fs::write(&viewer.args.save, b"last-known-good")?;
    drop(viewer.update(Message::Save));
    assert!(
        viewer
            .error
            .as_deref()
            .unwrap()
            .contains("original import source changed before ownership capture"),
        "{:?}",
        viewer.error
    );
    assert!(!viewer.playing && viewer.accumulator == 0.);
    assert_eq!(std::fs::read(&viewer.args.save)?, b"last-known-good");
    assert!(!viewer.status.contains("Save failed"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn output_validation_does_not_follow_a_symlink_over_a_known_good_save() -> Result<()> {
    let temp = Temporary::new()?;
    let known_good = temp.path.join("known-good.json");
    let link = temp.path.join("symlink.json");
    std::fs::write(&known_good, b"last-known-good")?;
    std::os::unix::fs::symlink(&known_good, &link)?;
    let error = write(&temp.root(), &link, b"replacement").unwrap_err();
    assert!(matches!(
        error.downcast_ref::<OutputFailure>().unwrap().operation,
        Operation::Validate
    ));
    assert!(recover(error)?.contains("No save was written"));
    assert_eq!(std::fs::read(known_good)?, b"last-known-good");
    assert!(std::fs::symlink_metadata(link)?.file_type().is_symlink());
    Ok(())
}

#[test]
fn queue_cancel_owner_duplicate_requests_revalidate_without_faulting() -> Result<()> {
    bounded(queue_duplicates_body)
}

// The Save fixture deliberately has no live-person UI binding. These controls
// exercise the actual exclusive-owner cancellation handler; the headless Iced
// controls separately exercise widget-to-message delivery without bypassing the
// production live-resident, modal or pending-input gates.
fn queue_duplicates_body() -> Result<()> {
    let temp = Temporary::new()?;
    let mut viewer = fixture::viewer(&temp.root(), &temp.path.join("unused.json"))?;
    let resident = viewer.resident;
    let session = viewer.session.as_mut().unwrap();
    assert!(session.simulation.native_lot.is_some());
    session.simulation.interactions.entry(resident).or_default();
    let actions = session
        .simulation
        .vm
        .world
        .actions
        .get_or_insert_with(Default::default);
    actions
        .actors
        .entry(resident)
        .or_default()
        .queue
        .pending
        .push_back(sims_vm::action::Action {
            id: 77,
            target: 0,
            label: Some("Authored pending action".into()),
            ..Default::default()
        });
    queue_cancel::apply(&mut viewer, 77)?;
    assert!(viewer.error.is_none(), "{:?}", viewer.error);
    assert_eq!(viewer.status, "Cancellation requested.");
    assert!(
        viewer
            .session
            .as_ref()
            .unwrap()
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .unwrap()
            .actors[&resident]
            .queue
            .pending
            .is_empty()
    );
    let before = owners(&viewer)?;
    let transport = (
        viewer.playing,
        viewer.accumulator,
        viewer.last,
        viewer.clock_origin,
    );
    for id in [77, 77, 78] {
        queue_cancel::apply(&mut viewer, id)?;
        assert!(viewer.error.is_none(), "{:?}", viewer.error);
        assert_eq!(viewer.status, "That interaction is no longer queued.");
        assert_eq!(
            owners(&viewer)?,
            before,
            "stale request changed an owner or repeated a callback"
        );
        assert_eq!(
            (
                viewer.playing,
                viewer.accumulator,
                viewer.last,
                viewer.clock_origin
            ),
            transport
        );
    }
    Ok(())
}

#[test]
fn queue_cancel_owner_existing_request_stays_visible_and_missing_services_still_fail() -> Result<()>
{
    bounded(queue_existing_body)
}

fn queue_existing_body() -> Result<()> {
    let temp = Temporary::new()?;
    let mut viewer = fixture::viewer(&temp.root(), &temp.path.join("unused.json"))?;
    let resident = viewer.resident;
    let session = viewer.session.as_mut().unwrap();
    session.simulation.interactions.entry(resident).or_default();
    session
        .simulation
        .vm
        .world
        .actions
        .get_or_insert_with(Default::default)
        .actors
        .entry(resident)
        .or_default()
        .active = Some(sims_vm::action::Action {
        id: 99,
        target: 0,
        label: Some("Authored cancelling action".into()),
        flags: sims_vm::action::CANCELLED,
        ..Default::default()
    });
    let before = owners(&viewer)?;
    for _ in 0..2 {
        queue_cancel::apply(&mut viewer, 99)?;
        assert!(viewer.error.is_none(), "{:?}", viewer.error);
        assert_eq!(viewer.status, "Cancellation already requested.");
        assert_eq!(owners(&viewer)?, before);
        assert_eq!(
            viewer
                .session
                .as_ref()
                .unwrap()
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .unwrap()
                .actors[&resident]
                .active
                .as_ref()
                .unwrap()
                .id,
            99
        );
    }
    let mut viewer = fixture::viewer(&temp.root(), &temp.path.join("unused.json"))?;
    let resident = viewer.resident;
    let simulation = &mut viewer.session.as_mut().unwrap().simulation;
    simulation.interactions.entry(resident).or_default();
    simulation.vm.world.actions = None;
    let error = queue_cancel::apply(&mut viewer, 99)
        .expect_err("missing services must stay an engine failure");
    assert!(format!("{error:#}").contains("native action services unavailable"));
    Ok(())
}

// Save a Copy output-boundary controls, ALL UNRUN.
#[test]
fn save_copy_output_uses_real_atomic_replacement_and_protects_canonical_aliases() -> Result<()> {
    let temp = Temporary::new()?;
    let canonical = temp.path.join("canonical.json");
    let copy = temp.path.join("new directory/家族 copy.json");
    std::fs::write(&canonical, b"canonical stays")?;
    write_copy(&temp.root(), &copy, &canonical, None, b"first copy")?;
    assert_eq!(std::fs::read(&copy)?, b"first copy");
    write_copy(&temp.root(), &copy, &canonical, None, b"replacement copy")?;
    assert_eq!(std::fs::read(&copy)?, b"replacement copy");
    let rejected = write_copy(&temp.root(), &canonical, &canonical, None, b"must not replace").unwrap_err();
    let feedback = recover_copy(rejected)?;
    assert!(feedback.contains("configured Save target"));
    assert!(feedback.contains("retry Save a Copy…"));
    assert!(!feedback.contains("then retry Save."));
    assert_eq!(std::fs::read(&canonical)?, b"canonical stays");
    assert_eq!(std::fs::read_dir(copy.parent().unwrap())?.count(), 1);
    Ok(())
}

#[test]
fn save_copy_output_recovery_is_typed_and_preserves_existing_restrictions() -> Result<()> {
    let temp = Temporary::new()?;
    let canonical = temp.path.join("canonical.json");
    let blocker = temp.path.join("blocked");
    std::fs::write(&blocker, b"blocker")?;
    for path in [temp.root().join("protected.json"), temp.path.join("../traversal.json"), blocker.join("copy.json")] {
        let feedback = recover_copy(write_copy(&temp.root(), &path, &canonical, None, b"bytes").unwrap_err())?;
        assert!(feedback.contains("retry Save a Copy…"));
        assert!(!feedback.starts_with("Saved copy"));
    }
    // An invalid fixed Save destination does not make a different path invalid.
    let alternate = temp.path.join("healthy copy.json");
    write_copy(&temp.root(), &alternate, &blocker.join("canonical.json"), None, b"healthy")?;
    assert_eq!(std::fs::read(&alternate)?, b"healthy");
    let untyped = anyhow::Error::new(std::io::Error::other("capture resource read failed"));
    assert!(recover_copy(untyped).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn save_copy_output_refuses_existing_parent_aliases_and_symlink_leaves() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = Temporary::new()?;
    let directory = temp.path.join("real");
    std::fs::create_dir(&directory)?;
    let alias = temp.path.join("alias");
    symlink(&directory, &alias)?;
    let canonical = directory.join("canonical.json");
    // The protected canonical target need not exist yet.
    assert!(recover_copy(write_copy(&temp.root(), &alias.join("canonical.json"), &canonical, None, b"bad").unwrap_err())?.contains("configured Save target"));
    let leaf = temp.path.join("dangling.json");
    symlink(temp.path.join("absent.json"), &leaf)?;
    assert!(recover_copy(write_copy(&temp.root(), &leaf, &canonical, None, b"bad").unwrap_err())?.contains("symlink"));
    assert!(!canonical.exists());
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn save_copy_output_refuses_case_and_normalization_aliases_of_absent_targets() -> Result<()> {
    let temp = Temporary::new()?;
    let canonical = temp.path.join("Pending café/Café.JSON");
    for alias in [
        temp.path.join("PENDING CAFÉ/CAFÉ.json"),
        temp.path.join("Pending cafe\u{301}/Cafe\u{301}.JSON"),
    ] {
        assert!(!canonical.exists() && !alias.exists());
        let error = write_copy(&temp.root(), &alias, &canonical, None, b"must not write")
            .expect_err("a different spelling must not create the configured save");
        assert!(recover_copy(error)?.contains("configured Save target"));
        assert!(!canonical.parent().unwrap().exists());
        assert!(!alias.parent().unwrap().exists());
    }
    // Nonliteral comparison preserves diacritics; an actual different name
    // is still a usable copy even if the fixed destination cannot be created.
    let different = temp.path.join("copy cafe.json");
    write_copy(&temp.root(), &different, &canonical, None, b"actual copy")?;
    assert_eq!(std::fs::read(different)?, b"actual copy");
    assert!(!canonical.exists());
    Ok(())
}
