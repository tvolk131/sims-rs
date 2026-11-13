//! Current original files and the same replacement camera backend used by Iced.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::Device,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
    software,
};
use sims_runtime::original_session::{Options, Progress, Session, completion, notifications};
use std::collections::BTreeSet;

#[test]
#[ignore = "requires private original Newbie files; headless actual camera device"]
fn fresh_tutorial_uses_actual_device_and_restores_house_open() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_FRESH_DEVICE_OUTPUT")?);
    ensure!(
        output.starts_with(root.join("cache")),
        "private cache output required"
    );
    std::fs::create_dir(&output).context("refusing to replace fresh device evidence")?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let source = "UserData/Houses/House07.iff";
    let mut s = Session::load(
        &index,
        source,
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            native_postload: true,
            native_load_completion: true,
            ..Default::default()
        },
    )?;
    let start = s.simulation.vm.tick;
    ensure!(
        s.simulation
            .vm
            .world
            .objects
            .values()
            .all(|o| o.native_keep_dynamic == Some(0)),
        "fresh object constructor byte lost during import/reset"
    );
    let mut device = Device::attach_for_load(&mut s, 1000, 740, 0)?;
    ensure!(
        s.host
            .presentation
            .presenter
            .as_ref()
            .unwrap()
            .inputs
            .keep_dynamic
            .is_empty()
    );
    let mut done = false;
    for _ in 0..30000 {
        let input = s.retained_camera_input()?;
        if s.advance_postload(input, 256)? {
            done = true;
            break;
        }
    }
    ensure!(done, "postload unfinished");
    eprintln!("fresh Newbie: original postload complete at tick {start}");
    device.refresh(&s)?;
    let seed = s.save();
    let device_seed = device.save(&s)?;
    let mut final_states = vec![];
    let mut reports = vec![];
    let mut welcome: Option<(Session, Device)> = None;
    for budget in [1, 256] {
        let mut s = Session::restore(&index, seed.clone())?;
        let mut device = Device::restore(&s, device_seed.clone())?;
        s.begin_load_completion(completion::Config {
            controller_suppressed: 0,
            mode: 0,
        })?;
        let mut checkpoints = BTreeSet::new();
        let mut services = vec![];
        let mut complete = false;
        for _ in 0..200000 {
            let input = s.retained_camera_input()?;
            let progress = s.advance_load_completion(input, budget)?;
            let key = match &progress {
                completion::Progress::Environment => Some("environment"),
                completion::Progress::Notification(notifications::Service::View(_)) => Some("view"),
                completion::Progress::Notification(notifications::Service::Camera(_)) => {
                    Some("camera")
                }
                completion::Progress::Notification(notifications::Service::Listener(_)) => {
                    Some("listener")
                }
                completion::Progress::Pending
                    if s.host
                        .presentation
                        .presenter
                        .as_ref()
                        .and_then(|p| p.pending_person())
                        .is_some() =>
                {
                    Some("people")
                }
                _ => None,
            };
            if let completion::Progress::Notification(service) = &progress {
                services.push(serde_json::to_value(service)?);
            }
            if budget == 1 && key.is_some_and(|key| checkpoints.insert(key)) {
                eprintln!("fresh Newbie: restore {key:?}");
                let state = serde_json::to_vec(&s.save())?;
                let saved_device = device.save(&s)?;
                let pixels = software::render(device.scene());
                s = Session::restore(&index, serde_json::from_slice(&state)?)?;
                device = Device::restore(&s, saved_device.clone())?;
                ensure!(
                    state == serde_json::to_vec(&s.save())?,
                    "restore changed session"
                );
                ensure!(saved_device == device.save(&s)?, "restore changed device");
                ensure!(
                    pixels == software::render(device.scene()),
                    "restore changed published pixels"
                );
            }
            if device.advance_load_completion(&mut s, budget)? {
                complete = true;
                break;
            }
        }
        ensure!(
            complete && !s.load_completion_pending() && !s.host.presentation.awaiting_house_open
        );
        ensure!(s.simulation.vm.tick == start, "loading advanced lot time");
        if budget == 1 {
            ensure!(
                checkpoints.contains("people"),
                "person-refresh restore not exercised"
            );
        }
        eprintln!("fresh Newbie: house opening complete, budget {budget}");
        final_states.push(serde_json::to_vec(&(s.save(), device.save(&s)?))?);
        reports.push(json!({"budget":budget,"restore_boundaries":checkpoints,"services":services}));
        welcome = Some((s, device));
    }
    ensure!(
        final_states[0] == final_states[1],
        "loading depends on slice budget/restore"
    );
    let (mut s, mut device) = welcome.unwrap();
    let resident = s.house.household.as_ref().unwrap().residents[0].object;
    let mut client = Client::new(&mut s)?;
    let mut reached = false;
    for _ in 0..1000 {
        if client.pending() {
            client.advance(&mut s, 256)?;
            continue;
        }
        match s.step()? {
            Progress::Tick => (),
            Progress::DialogConstruction(thread) => client.begin(
                &s,
                thread,
                Inputs::camera(OriginalHouse::native_camera(&s)?, 64)?,
            )?,
            Progress::Dialog => {
                reached = true;
                break;
            }
            other => anyhow::bail!("fresh tutorial stopped at {other:?}"),
        }
        client.collect(&s)?;
    }
    ensure!(reached, "welcome not reached");
    let modal = s
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| Some(r.id) == s.simulation.vm.world.dialogs.modal)
        .context("welcome absent")?
        .clone();
    ensure!(modal.bitmap_icon == Some(5018) && modal.buttons.len() == 1);
    let owner = s
        .simulation
        .vm
        .world
        .ui_owner
        .owner
        .context("tutorial controller absent")?;
    device.refresh(&s)?;
    let capture = |s: &Session, d: &Device, c: &Client| -> Result<Value> {
        Ok(
            json!({"session":s.save(),"camera_device":d.save(s)?,"dialog_client":c.save(s)?,
            "floor":1,"zoom":0.5,"resident":resident,"selected":null}),
        )
    };
    let saved = capture(&s, &device, &client)?;
    std::fs::write(output.join("welcome.json"), serde_json::to_vec(&saved)?)?;
    let restored = Session::restore(&index, serde_json::from_value(saved["session"].clone())?)?;
    let restored_device = Device::restore(
        &restored,
        serde_json::from_value(saved["camera_device"].clone())?,
    )?;
    let restored_client = Client::restore(
        &restored,
        serde_json::from_value(saved["dialog_client"].clone())?,
    )?;
    ensure!(saved == capture(&restored, &restored_device, &restored_client)?);
    ensure!(matches!(s.step()?, Progress::Dialog));
    ensure!(
        saved == capture(&s, &device, &client)?,
        "unanswered welcome advanced"
    );
    let pixels = software::render(device.scene());
    ensure!(pixels == software::render(restored_device.scene()));
    pixels.save(output.join("floor-1.png"))?;
    ensure!(
        saved == capture(&s, &device, &client)?,
        "render mutated state"
    );
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "source":source,"start_tick":start,"tick":s.simulation.vm.tick,"controller":owner,"modal":modal,
            "loading":reports,"slice_and_restore_match":true,"welcome_full_restore_and_pixels_match":true,
            "scope":"Fresh original import and native postload/completion with actual replacement scene, projection depth, controls and invalidation. Explicit frontend registration order/stationary camera and absent legacy terrain renderer. No retagging, empty nearby-coordinate input, tutorial attribute edits or injected UI events; protected routing remains inferred."
        }))?,
    )?;
    Ok(())
}
