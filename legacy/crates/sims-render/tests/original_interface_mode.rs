//! Actual loaded household and backend at every Live publication boundary.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{
    Options, Session, camera_command::Change, completion, interface::Progress,
};
use sims_vm::interface::Request;
use std::path::Path;

#[test]
#[ignore = "private Goth files; full session/device restore at each interface service"]
fn live_publication_restores_without_advancing_the_household() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = root.join(std::env::var("SIMS_INTERFACE_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace interface evidence")?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut s = Session::load(
        &index,
        "UserData/Houses/House05.iff",
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            native_postload: true,
            native_load_completion: true,
            ..Default::default()
        },
    )?;
    let mut device = Device::attach_for_load(&mut s, 1000, 740, 0)?;
    let mut done = false;
    for _ in 0..30000 {
        let input = s.retained_camera_input()?;
        if s.advance_postload(input, 256)? {
            done = true;
            break;
        }
    }
    ensure!(done);
    s.begin_load_completion(completion::Config {
        controller_suppressed: 0,
        mode: 0,
    })?;
    done = false;
    for _ in 0..30000 {
        if device.advance_load_completion(&mut s, 256)? {
            done = true;
            break;
        }
    }
    ensure!(done);
    ensure!(s.simulation.vm.world.globals.get(&19) == Some(&3));
    let initial = s.save();
    let initial_device = device.save(&s)?;
    let mut expected = s.simulation.clone();
    s.begin_live_interface_update()?;
    let mut stages = vec![];
    let mut complete = false;
    for n in 0..20 {
        if !s.interface_update_pending() {
            complete = true;
            break;
        }
        let bytes = serde_json::to_vec(&s.save())?;
        ensure!(s.step().is_err());
        ensure!(device.begin(&mut s, Change::Rotation(1)).is_err());
        ensure!(s.begin_ui_wait(None).is_err());
        ensure!(s.begin_selection(None).is_err());
        ensure!(s.bind_camera_ui(device.controls()).is_err());
        let camera = s.retained_camera_input()?;
        ensure!(s.advance_postload(camera, 1).is_err());
        ensure!(s.advance_load_completion(camera, 1).is_err());
        ensure!(s.advance_view_refresh(camera, 1).is_err());
        ensure!(s.advance_house_notification(camera, 1).is_err());
        ensure!(
            bytes == serde_json::to_vec(&s.save())?,
            "blocked work mutated session"
        );
        let progress = s.advance_interface_update(1)?;
        let request = s
            .host
            .presentation
            .interface_update
            .as_ref()
            .unwrap()
            .request()
            .cloned();
        if matches!(
            request,
            Some(Request::StopAudio | Request::PlayLiveBackground)
        ) || s.simulation.vm.world.globals.get(&19) == Some(&0)
        {
            expected.vm.world.globals.insert(19, 0);
        }
        ensure!(
            s.simulation == expected,
            "interface update changed unrelated simulation state"
        );
        let saved = s.save();
        let surface = device.save(&s)?;
        let pixels = software::render(device.scene());
        let mut resumed = Session::restore(&index, saved.clone())?;
        let mut restored_device = Device::restore(&resumed, surface.clone())?;
        ensure!(serde_json::to_vec(&saved)? == serde_json::to_vec(&resumed.save())?);
        ensure!(surface == restored_device.save(&resumed)?);
        ensure!(pixels == software::render(restored_device.scene()));
        if let Progress::Service(service) = &progress {
            if *service == Request::DrawingMode {
                // A historical backend with no retained flag cannot silently
                // acknowledge the conditional83 branch. Never change its save.
                let mut unknown = serde_json::to_value(&surface)?;
                unknown.as_object_mut().unwrap().remove("drawing_mode");
                let mut old_device = Device::restore(&resumed, serde_json::from_value(unknown)?)?;
                let before = serde_json::to_vec(&resumed.save())?;
                ensure!(old_device.advance_interface(&mut resumed, 1).is_err());
                ensure!(before == serde_json::to_vec(&resumed.save())?);
            }
            let a = device.advance_interface(&mut s, 1)?;
            let b = restored_device.advance_interface(&mut resumed, 1)?;
            ensure!(a == b);
            ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&resumed.save())?);
        }
        stages.push(
            json!({"index":n,"progress":format!("{progress:?}"),"request":request,
                          "script_mode":s.simulation.vm.world.globals[&19]}),
        );
    }
    ensure!(complete, "interface publication did not finish");
    device.refresh(&s)?;
    let mut fast = Session::restore(&index, initial)?;
    let mut fast_device = Device::restore(&fast, initial_device)?;
    fast.begin_live_interface_update()?;
    complete = false;
    for _ in 0..20 {
        if fast_device.advance_interface(&mut fast, 256)? {
            complete = true;
            break;
        }
    }
    ensure!(complete && serde_json::to_vec(&s.save())? == serde_json::to_vec(&fast.save())?);
    ensure!(s.simulation == expected && s.simulation.vm.world.globals[&19] == 0);
    ensure!(device.save(&s)? == fast_device.save(&fast)?);
    ensure!(software::render(device.scene()) == software::render(fast_device.scene()));
    software::render(device.scene()).save(output.join("live.png"))?;
    std::fs::write(
        output.join("session.json"),
        serde_json::to_vec(&json!({"session":s.save(),"camera_device":device.save(&s)?}))?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "tick":s.simulation.vm.tick,"stages":stages,"all_pending_stages_restore":true,
            "only_simulation_change_global19":true,"small_large_budget_identical":true,
            "restored_surface_pixels_match":true,"unknown_drawing_flag_rejected_without_mutation":true,
            "scope":"Actual Goth native loading and fresh Live renderer; mask2 interface pass only, no mode switching or full native frame claim."
        }))?,
    )?;
    Ok(())
}
