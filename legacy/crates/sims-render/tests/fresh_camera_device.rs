use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Options, Progress, Session, camera_command::Change};

fn select(s: &mut Session, device: &mut Device, id: i16) -> Result<()> {
    s.begin_selection(Some(id))?;
    for _ in 0..10000 {
        if s.house_notification_pending() {
            device.advance_notification(s, 256)?;
        } else if s.advance_selection(s.retained_camera_input()?, 256)? {
            return Ok(());
        }
    }
    anyhow::bail!("selection did not finish")
}
fn command(s: &mut Session, device: &mut Device, change: Change) -> Result<()> {
    device
        .begin(s, change)
        .with_context(|| format!("begin {change:?}"))?;
    for _ in 0..30000 {
        if device
            .advance(s, 256)
            .with_context(|| format!("advance {change:?}"))?
        {
            return Ok(());
        }
    }
    anyhow::bail!("camera did not finish")
}
fn tick(s: &mut Session, device: &mut Device) -> Result<()> {
    for _ in 0..10000 {
        if s.house_notification_pending() {
            device.advance_notification(s, 256)?;
            continue;
        }
        match s.step()? {
            Progress::Tick => return Ok(()),
            Progress::HouseNotification => (),
            other => anyhow::bail!("unexpected early household boundary: {other:?}"),
        }
    }
    anyhow::bail!("household tick did not finish")
}

#[test]
#[ignore = "requires private original houses"]
fn fresh_three_house_camera_and_selection_resume_without_demo_view_fields() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(1920, 960);
    let mut report = vec![];
    for (column, (name, source)) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = Session::load(
            &index,
            source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let before = (
            s.simulation.vm.tick,
            s.simulation.vm.rng_state,
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        );
        let mut device = Device::attach(&mut s, 640, 480, 0)
            .with_context(|| format!("{name}: attach camera"))?;
        let residents = s
            .house
            .household
            .as_ref()
            .unwrap()
            .residents
            .iter()
            .map(|r| r.object as i16)
            .collect::<Vec<_>>();
        select(&mut s, &mut device, residents[0])
            .with_context(|| format!("{name}: select initial resident"))?;
        command(&mut s, &mut device, Change::Zoom(1))?;
        for rotation in [1, 2, 3, 0] {
            command(&mut s, &mut device, Change::Rotation(rotation))?;
        }
        command(&mut s, &mut device, Change::Zoom(2))?;
        command(&mut s, &mut device, Change::Floor(2))?;
        image::imageops::replace(
            &mut gallery,
            &software::render(device.scene()),
            column as i64 * 640,
            0,
        );
        command(&mut s, &mut device, Change::Floor(1))?;
        if residents.len() > 1 {
            select(&mut s, &mut device, residents[1])?;
        }
        ensure!(
            before
                == (
                    s.simulation.vm.tick,
                    s.simulation.vm.rng_state,
                    s.simulation.vm.lot_rng_state,
                    s.simulation.vm.crt_rng_state
                )
        );
        device.refresh(&s)?;
        let saved = serde_json::to_vec(&s.save())?;
        let device_saved = serde_json::to_vec(&device.save(&s)?)?;
        let pixels = software::render(device.scene());
        ensure!(serde_json::to_vec(&s.save())? == saved);
        let mut restored = Session::restore(&index, serde_json::from_slice(&saved)?)?;
        let mut restored_device =
            Device::restore(&restored, serde_json::from_slice(&device_saved)?)?;
        ensure!(software::render(restored_device.scene()) == pixels);
        ensure!(serde_json::to_vec(&restored.save())? == saved);
        for _ in 0..10 {
            tick(&mut s, &mut device)?;
            tick(&mut restored, &mut restored_device)?;
        }
        device.refresh(&s)?;
        restored_device.refresh(&restored)?;
        ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&restored.save())?);
        ensure!(device.save(&s)? == restored_device.save(&restored)?);
        let pixels = software::render(device.scene());
        ensure!(pixels == software::render(restored_device.scene()));
        image::imageops::replace(&mut gallery, &pixels, column as i64 * 640, 480);
        report.push(serde_json::json!({"house":name,"source":source,"ticks":10,
            "camera_commands":8,"resident_selection":residents.len().min(2),"restored_pixels_equal":true,"tick_rng_unchanged_by_camera":true}));
        println!(
            "{name}: real camera commands/selection, saved device, ten equal subsequent ticks"
        );
    }
    gallery.save(root.join("cache/fresh-camera-device-gallery.png"))?;
    std::fs::write(
        root.join("cache/fresh-camera-device-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
