use super::*;
use clap::Parser;
use sims_vm::ui_owner::wait::{Condition, Observation};

fn drain(v: &mut Viewer) -> Result<()> {
    for _ in 0..10000 {
        ensure!(v.error.is_none(), "presentation: {:?}", v.error);
        if !v.presentation_pending() {
            return Ok(());
        }
        let _ = v.update(Message::Frame(Instant::now()));
    }
    anyhow::bail!("presentation exceeded budget")
}

#[test]
#[ignore = "private original Newbie household and headless Iced Metal rendering"]
fn original_needs_widgets_continue_tutorial_and_restore_real_popup() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = repo.join("game-data/cabinet-extracted");
    let output = repo.join(std::env::var("SIMS_LIVE_WIDGET_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace live-widget evidence")?;
    let source = repo.join("cache/newbie-object-ui-checked/session.json");
    let save = output.join("session.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--inferred-routing",
        "--paused",
        "--root",
        root.to_str().unwrap(),
        "--load",
        source.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let ready = Arc::new(Ready::load(&args)?);
    let (mut v, _) = Viewer::new(args);
    v.install(ready)?;
    let start = v.session.as_ref().unwrap().simulation.vm.tick;
    ensure!(
        v.client.as_ref().unwrap().live_panel().is_none(),
        "historical restore invented widgets"
    );
    let before = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    v.client
        .as_mut()
        .unwrap()
        .bind_live_panel(v.session.as_ref().unwrap())?;
    ensure!(
        before == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
        "widget binding changed VM/session state"
    );
    let panel = v.client.as_ref().unwrap().live_panel().unwrap();
    ensure!(panel.selected() && panel.person().unwrap().object == 105);
    ensure!(
        panel.observation(&Condition::Control(4500))
            == Some(Observation::Control {
                image: Some(4500),
                state: 1
            })
    );
    ensure!(
        panel.observation(&Condition::Panel("motives".into())) == Some(Observation::Panel(None))
    );
    let saved = serde_json::to_value(
        v.client
            .as_ref()
            .unwrap()
            .save(v.session.as_ref().unwrap())?,
    )?;
    for pointer in [
        "/live_panel/state",
        "/live_panel/shown",
        "/live_panel/person/guid",
        "/live_panel/person/cache_index",
    ] {
        let mut corrupt = saved.clone();
        *corrupt.pointer_mut(pointer).unwrap() = match pointer {
            "/live_panel/shown" => serde_json::json!(false),
            "/live_panel/state" => serde_json::json!(3),
            "/live_panel/person/guid" => serde_json::json!(0),
            _ => serde_json::json!(u32::MAX),
        };
        ensure!(
            DialogClient::restore(
                v.session.as_ref().unwrap(),
                serde_json::from_value(corrupt)?
            )
            .is_err(),
            "corrupt widget accepted: {pointer}"
        );
    }
    super::person_tests::checkpoint(&mut v, &output, "needs-bound")?;
    render_headless_element(&root, &output.join("needs-bound.png"), v.view())?;
    ensure!(v.poll_application_ui()?);
    // Exercise visible replacement widgets before the next modal. This does
    // not stand in for the later original shower/popup tutorial lesson.
    if v.device.as_ref().unwrap().panel_state() == Some(1) {
        let _ = v.update(Message::Live);
        drain(&mut v)?;
    }
    ensure!(v.device.as_ref().unwrap().panel_state() == Some(0));
    super::person_tests::checkpoint(&mut v, &output, "before-motive-click")?;
    let before = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    let _ = v.update(Message::MotiveDetails(0));
    ensure!(v.error.is_none(), "motive click: {:?}", v.error);
    ensure!(v.client.as_ref().unwrap().live_panel().unwrap().popup() == Some(0));
    ensure!(
        before == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
        "motive click changed VM/session state"
    );
    super::person_tests::checkpoint(&mut v, &output, "motive-open")?;
    render_headless_element(&root, &output.join("motive-open.png"), v.view())?;
    // Real different-widget replacement, same-widget close, and reopen.
    let _ = v.update(Message::MotiveDetails(1));
    ensure!(v.client.as_ref().unwrap().live_panel().unwrap().popup() == Some(1));
    let _ = v.update(Message::MotiveDetails(1));
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .popup()
            .is_none()
    );
    let _ = v.update(Message::MotiveDetails(0));
    ensure!(v.client.as_ref().unwrap().live_panel().unwrap().popup() == Some(0));
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .observation(&Condition::Panel("motives".into()))
            == Some(Observation::Panel(Some("motives".into())))
    );
    let _ = v.update(Message::CloseMotiveDetails);
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .unwrap()
            .popup()
            .is_none()
    );
    let mut notices = vec![];
    let mut explanation = false;
    let mut preview = false;
    for _ in 0..5000 {
        let _ = v.update(Message::Step);
        if let Some(error) = &v.error {
            std::fs::write(output.join("boundary-error.txt"), error)?;
            anyhow::bail!("unexpected needs lesson boundary: {error}");
        }
        let s = v.session.as_ref().unwrap();
        if let Some(record) = s
            .simulation
            .vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.status == 1)
        {
            if !notices
                .iter()
                .any(|n: &serde_json::Value| n["id"] == record.id)
            {
                notices.push(serde_json::to_value(record)?);
            }
            if record.origin.bhav == 4125 && record.origin.instruction == 12 {
                ensure!(
                    record
                        .portrait
                        .as_ref()
                        .is_some_and(|p| p.rect == [0, 0, 120, 120])
                );
                preview = true;
                break;
            }
            if record.origin.bhav == 4125 && record.origin.instruction == 10 {
                let id = record.id;
                ensure!(
                    record.global_bitmap_icon
                        == Some(sims_vm::dialog::GlobalBitmap {
                            id: 4500,
                            button_cell: true
                        })
                );
                super::person_tests::checkpoint(&mut v, &output, "needs-explained")?;
                render_headless_element(&root, &output.join("needs-explained.png"), v.view())?;
                let _ = v.update(Message::Dialog(id, 2));
                ensure!(v.error.is_none());
                explanation = true;
            }
        }
    }
    ensure!(
        explanation && preview,
        "original Needs explanation / shower preview was not reached"
    );
    super::person_tests::checkpoint(&mut v, &output, "object-preview-published")?;
    let frozen = serde_json::to_vec(&v.session.as_ref().unwrap().save())?;
    render_headless_element(
        &root,
        &output.join("object-preview-published.png"),
        v.view(),
    )?;
    ensure!(
        frozen == serde_json::to_vec(&v.session.as_ref().unwrap().save())?,
        "render advanced shower instruction"
    );
    v.args.save = save;
    v.save()?;
    let tick = v.session.as_ref().unwrap().simulation.vm.tick;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "start":start,"tick":tick,"tutorial_ticks":tick-192923,
            "original_needs_lesson_completed":true,"original_shower_instruction_entered":true,
            "shower_instruction_published":true,"boundary":null,"original_location":[4125,12],"stack_object":107,
            "widget_binding_preserved_session":true,"motive_click_preserved_session":true,
            "popup_open_replace_close_reopen":true,"full_client_session_device_restores":5,
            "original_global_button_bitmap_restored_and_rendered":4500,
            "render_preserved_session":true,"historical_restore_did_not_create_widgets":true,
            "native_execution_this_turn":false,"corrupt_client_owners_rejected":4,
            "scope":"Actual replacement Iced widgets and original wait callbacks; full legacy Windows panel frame and native pixel equivalence remain unverified. Object preview uses the explicit Rust software endpoint. No claim of shower interaction completion.",
            "notices":notices
        }))?,
    )?;
    Ok(())
}
