use super::*;
use crate::{original_house::OriginalHouse, software};
use sims_sim::{lot_presenter::camera_command::Ui, pose::portrait::view::Input};

fn device() -> Result<Device> {
    let frame = Frame {
        input: Input {
            viewport: [0, 0, 1, 1],
            center: [0, 0],
            zoom: 2,
            rotation: 0,
        },
        floor: 1,
        tick: 88444,
    };
    let mut saved = Saved {
        tool_cursor: None,
        furniture_selection: None,
        panel_widgets: None,
        mode_window: None,
        viewport: None,
        transport_pause: None,
        panel_state: None,
        source: "normal-frame-surface".to_string(),
        version: 1,
        frame,
        published_frame: frame,
        dirty: false,
        generation: 28,
        published_generation: 28,
        controls: Ui {
            flags: 0,
            zoom: 2,
            rotation: 0,
            floor: 1,
        },
        controls_generation: 0,
        interface_flags: 0,
        selected_person: None,
        drawing_mode: Some(0),
        frozen_pixels: None,
        normal_frame: None,
    };
    saved.normal_frame = Some(Ownership::capture(&saved, frame.tick));
    let scene = Arc::new(software::rgba_scene(1, 1, vec![11, 22, 33, 255])?);
    Ok(Device {
        saved,
        renderer: OriginalHouse::default(),
        scene: scene.clone(),
        working: scene,
        frozen: true,
    })
}

#[test]
fn normal_frame_surface_rejects_missing_or_malformed_pixels() -> Result<()> {
    let mut saved = device()?.saved;
    ensure!(validate_pixels(&saved).is_err());
    for pixels in [vec![], vec![11, 22, 33], vec![11, 22, 33, 255, 0]] {
        saved.frozen_pixels = Some(pixels);
        ensure!(validate_pixels(&saved).is_err());
    }
    saved.frozen_pixels = Some(vec![11, 22, 33, 255]);
    validate_pixels(&saved)?;
    Ok(())
}

#[test]
fn normal_frame_surface_rejects_recipe_writes() -> Result<()> {
    let device = device()?;
    let owner = device.saved.normal_frame.unwrap();
    for field in [
        "frame",
        "published_frame",
        "dirty",
        "generation",
        "published_generation",
    ] {
        let mut changed = device.saved.clone();
        match field {
            "frame" => changed.frame.input.center[0] += 1,
            "published_frame" => changed.published_frame.floor += 1,
            "dirty" => changed.dirty = true,
            "generation" => changed.generation += 1,
            "published_generation" => changed.published_generation += 1,
            _ => unreachable!(),
        }
        ensure!(owner.validate(&changed, 88444).is_err(), "accepted {field}");
    }
    ensure!(owner.validate(&device.saved, 88445).is_err());
    Ok(())
}

#[test]
fn normal_frame_surface_rejects_premature_publication_without_thawing() -> Result<()> {
    for (native_pending, view_pending) in [(true, true), (true, false), (false, true)] {
        let mut device = device()?;
        let before = device.saved.clone();
        let scene = device.scene.clone();
        let pixels = software::render(&scene).into_raw();
        ensure!(
            device
                .publish_normal_frame_surface(
                    before.frame,
                    scene.clone(),
                    88444,
                    native_pending,
                    view_pending,
                )
                .is_err()
        );
        ensure!(device.saved == before && device.frozen);
        ensure!(Arc::ptr_eq(&scene, &device.scene) && Arc::ptr_eq(&scene, &device.working));
        ensure!(software::render(device.scene()).into_raw() == pixels);
        device.publish_normal_frame_surface(before.frame, scene.clone(), 88444, false, false)?;
        ensure!(
            device.saved.generation == 29
                && device.saved.published_generation == 29
                && !device.normal_view_frame_pending()
                && !device.frozen
        );
        ensure!(
            device
                .publish_normal_frame_surface(before.frame, scene, 88444, false, false,)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn normal_frame_surface_generation_failure_preserves_valid_pixels_and_owner() -> Result<()> {
    let mut device = device()?;
    device.saved.generation = u64::MAX;
    device.saved.published_generation = u64::MAX;
    device.saved.normal_frame = Some(Ownership::capture(&device.saved, 88444));
    let before = device.saved.clone();
    let scene = device.scene.clone();
    let replacement = Arc::new(software::rgba_scene(1, 1, vec![44, 55, 66, 255])?);
    let pixels = software::render(&scene).into_raw();
    ensure!(
        device
            .publish_normal_frame_surface(before.frame, replacement, 88444, false, false,)
            .is_err()
    );
    ensure!(device.saved == before && device.frozen && device.normal_view_frame_pending());
    ensure!(Arc::ptr_eq(&scene, &device.scene) && Arc::ptr_eq(&scene, &device.working));
    ensure!(software::render(device.scene()).into_raw() == pixels);
    Ok(())
}

#[test]
fn normal_frame_surface_publishes_and_releases_exactly_once() -> Result<()> {
    let mut device = device()?;
    let before = device.saved.clone();
    let replacement = Arc::new(software::rgba_scene(1, 1, vec![44, 55, 66, 255])?);
    device.publish_normal_frame_surface(before.frame, replacement.clone(), 88444, false, false)?;
    let mut expected = before.clone();
    expected.generation = 29;
    expected.published_generation = 29;
    expected.normal_frame = None;
    ensure!(device.saved == expected && !device.frozen && !device.normal_view_frame_pending());
    ensure!(Arc::ptr_eq(&replacement, device.scene()));
    ensure!(software::render(device.scene()).into_raw() == vec![44, 55, 66, 255]);
    ensure!(
        device
            .publish_normal_frame_surface(before.frame, replacement.clone(), 88444, false, false,)
            .is_err()
    );
    ensure!(device.saved == expected && Arc::ptr_eq(&replacement, device.scene()));
    Ok(())
}

#[test]
fn normal_frame_surface_retains_prior_tick_recipe_through_serialization() -> Result<()> {
    let mut saved = device()?.saved;
    saved.normal_frame = Some(Ownership::capture(&saved, 88445));
    saved.frozen_pixels = Some(vec![11, 22, 33, 255]);
    let restored: Saved = serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
    ensure!(saved == restored && restored.frame.tick == 88444);
    validate_tick(&restored, 88445)?;
    validate_pixels(&restored)?;
    ensure!(validate_tick(&restored, 88444).is_err());
    ensure!(validate_tick(&restored, 88446).is_err());
    let mut ownerless = restored.clone();
    ownerless.normal_frame = None;
    ensure!(validate_tick(&ownerless, 88445).is_err());
    validate_tick(&ownerless, 88444)?;
    let mut mismatched = restored.clone();
    mismatched.published_frame.tick = 88443;
    mismatched.normal_frame = Some(Ownership::capture(&mismatched, 88445));
    ensure!(validate_tick(&mismatched, 88445).is_err());
    let mut future = restored;
    future.normal_frame = Some(Ownership::capture(&future, 88443));
    ensure!(validate_tick(&future, 88443).is_err());
    Ok(())
}

#[test]
fn normal_frame_surface_advanced_publication_installs_current_tick_once() -> Result<()> {
    let mut device = device()?;
    device.saved.normal_frame = Some(Ownership::capture(&device.saved, 88445));
    let before = device.saved.clone();
    let held = device.scene.clone();
    let replacement = Arc::new(software::rgba_scene(1, 1, vec![44, 55, 66, 255])?);
    for tick in [88443, 88444, 88446] {
        let mut wrong = before.frame;
        wrong.tick = tick;
        ensure!(
            device
                .publish_normal_frame_surface(wrong, replacement.clone(), 88445, false, false)
                .is_err()
        );
        ensure!(device.saved == before && device.frozen);
        ensure!(Arc::ptr_eq(&held, &device.scene));
    }
    let mut current = before.frame;
    current.tick = 88445;
    device.publish_normal_frame_surface(current, replacement.clone(), 88445, false, false)?;
    let mut expected = before;
    expected.frame = current;
    expected.published_frame = current;
    expected.generation = 29;
    expected.published_generation = 29;
    expected.normal_frame = None;
    ensure!(device.saved == expected && !device.frozen);
    ensure!(Arc::ptr_eq(&replacement, &device.scene));
    ensure!(
        device
            .publish_normal_frame_surface(current, held, 88445, false, false)
            .is_err()
    );
    ensure!(device.saved == expected);
    Ok(())
}
