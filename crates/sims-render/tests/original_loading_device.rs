//! Production startup driver with original Goth/Mashuga files and real device work.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_render::{
    camera_device::Device,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
    software,
};
use sims_runtime::original_session::{
    Options, Progress, Session, camera_command::Change, completion,
};
use std::path::Path;

fn wrapper(s: &Session, d: &Device, c: &Client) -> Result<Value> {
    Ok(
        json!({"session":s.save(),"camera_device":d.save(s)?,"dialog_client":c.save(s)?,
        "floor":d.controls().floor,"zoom":(1u32 << d.controls().zoom) as f32/8.,
        "resident":s.house.household.as_ref().unwrap().residents[0].object,"selected":null}),
    )
}
fn restored(index: &Index, s: &Session, d: &Device, c: &Client) -> Result<()> {
    let saved = wrapper(s, d, c)?;
    let resumed = Session::restore(index, serde_json::from_value(saved["session"].clone())?)?;
    let device = Device::restore(
        &resumed,
        serde_json::from_value(saved["camera_device"].clone())?,
    )?;
    let client = Client::restore(
        &resumed,
        serde_json::from_value(saved["dialog_client"].clone())?,
    )?;
    ensure!(
        saved == wrapper(&resumed, &device, &client)?,
        "whole startup checkpoint changed on restore"
    );
    ensure!(
        software::render(d.scene()) == software::render(device.scene()),
        "restored pixels differ"
    );
    Ok(())
}
#[test]
#[ignore = "private original Goth/Mashuga files; production startup driver, cameras and early play"]
fn goth_and_mashuga_open_with_actual_device_and_restore() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join(std::env::var("SIMS_ORIGINAL_LOADING_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output).context("refusing to replace startup evidence")?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for (name, source, floors) in [
        ("goth", "UserData/Houses/House05.iff", vec![1, 2, 1]),
        ("mashuga", "UserData2/Houses/House09.iff", vec![1]),
    ] {
        let dir = output.join(name);
        std::fs::create_dir(&dir)?;
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
        let mut device = match Device::open(
            &mut s,
            1000,
            740,
            0,
            completion::Config {
                controller_suppressed: 0,
                mode: 0,
            },
        ) {
            Ok(d) => d,
            Err(e) => {
                std::fs::write(
                    dir.join("loading-error.json"),
                    serde_json::to_vec(&json!({"error":format!("{e:#}"),"session":s.save()}))?,
                )?;
                return Err(e).with_context(|| format!("{name} startup"));
            }
        };
        let mut client = Client::new(&mut s)?;
        ensure!(
            !s.host.presentation.awaiting_house_open
                && !s.load_completion_pending()
                && !s.postload_pending()
        );
        ensure!(s.simulation.vm.tick == start);
        eprintln!("{name}: complete original engine/viewer startup at tick {start}");
        std::fs::write(
            dir.join("opened.json"),
            serde_json::to_vec(&wrapper(&s, &device, &client)?)?,
        )?;
        restored(&index, &s, &device, &client)?;
        let clock = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        for floor in floors {
            device.begin(&mut s, Change::Floor(floor))?;
            let mut done = false;
            for _ in 0..30000 {
                if device.advance(&mut s, 256)? {
                    done = true;
                    break;
                }
            }
            ensure!(done, "initial floor command unfinished");
            software::render(device.scene()).save(dir.join(format!("floor-{floor}.png")))?;
        }
        ensure!(
            clock
                == (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                    s.simulation.vm.crt_rng_state
                )
        );
        let mut ticks = 0;
        let mut modal = false;
        for _ in 0..30000 {
            if client.pending() {
                client.advance(&mut s, 256)?;
                continue;
            }
            if s.house_notification_pending() {
                device.advance_notification(&mut s, 256)?;
                continue;
            }
            if s.simulation.selection_work_pending() {
                let view = s.retained_camera_input()?;
                s.advance_selection(view, 256)?;
                continue;
            }
            if ticks >= 256 {
                break;
            }
            match s.step()? {
                Progress::Tick => ticks += 1,
                Progress::Dialog => {
                    modal = true;
                    break;
                }
                Progress::DialogConstruction(thread) => client.begin(
                    &s,
                    thread,
                    Inputs::camera(OriginalHouse::native_camera(&s)?, 64)?,
                )?,
                Progress::HouseNotification => (),
                other => anyhow::bail!(
                    "{name} needs its frontend service at tick{}: {other:?}",
                    s.simulation.vm.tick
                ),
            }
            client.collect(&s)?;
        }
        ensure!(ticks == 256 || modal, "early play work budget exhausted");
        device.refresh(&s)?;
        restored(&index, &s, &device, &client)?;
        std::fs::write(
            dir.join("played.json"),
            serde_json::to_vec(&wrapper(&s, &device, &client)?)?,
        )?;
        software::render(device.scene()).save(dir.join("played.png"))?;
        let report = json!({"house":name,"source":source,"start_tick":start,"tick":s.simulation.vm.tick,"ticks":ticks,
            "modal":modal,"initial_and_played_full_restore_and_pixels_match":true,
            "camera_commands_preserve_ticks_and_rng":true,"no_nearby_coordinate_override":true});
        std::fs::write(dir.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
        reports.push(report);
        eprintln!("{name}: +{ticks} ticks, modal={modal}, full restored state and pixels match");
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({"houses":reports,
        "scope":"Original files, native postload/completion and actual replacement camera backend. Seven hosted listeners; legacy cSimsApp Options/UI receiver absent, inferred rectangle routing and CRT seed explicit. No empty nearby-coordinate input, modal answers or suspended-original-stack resumption."}))?,
    )?;
    Ok(())
}
