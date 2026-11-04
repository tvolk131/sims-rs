//! Real Interface E4 ->4ce41b ->50a8ec, including saved nested panel ownership.
use super::*;
use clap::Parser;
use serde_json::json;
use std::collections::BTreeSet;

fn exact_value_equal(expected: &serde_json::Value, actual: &serde_json::Value) -> bool {
    match (expected, actual) {
        (serde_json::Value::Number(expected), serde_json::Value::Number(actual)) => {
            expected == actual
                && expected.as_f64().map(f64::to_bits) == actual.as_f64().map(f64::to_bits)
        }
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            expected.len() == actual.len()
                && expected
                    .iter()
                    .zip(actual)
                    .all(|(expected, actual)| exact_value_equal(expected, actual))
        }
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            expected.len() == actual.len()
                && expected.iter().all(|(field, expected)| {
                    actual
                        .get(field)
                        .is_some_and(|actual| exact_value_equal(expected, actual))
                })
        }
        _ => expected == actual,
    }
}

fn retained(v: &Viewer) -> Result<serde_json::Value> {
    let s = v.session.as_ref().unwrap();
    Ok(
        json!({"session":s.save_managed()?,"client":v.client.as_ref().unwrap().save_managed(s)?,
        "device":v.device.as_ref().unwrap().save(s)?}),
    )
}
fn cold(v: &mut Viewer) -> Result<()> {
    let before = retained(v)?;
    let index = v.session.as_ref().unwrap().index().clone();
    let s = Session::restore(&index, serde_json::from_value(before["session"].clone())?)?;
    let c = DialogClient::restore_managed(&s, serde_json::from_value(before["client"].clone())?)?;
    let d = CameraDevice::restore(&s, serde_json::from_value(before["device"].clone())?)?;
    v.session = Some(s);
    v.client = Some(c);
    v.device = Some(d);
    ensure!(
        exact_value_equal(&before, &retained(v)?),
        "property child changed across cold restore"
    );
    Ok(())
}
#[test]
#[ignore = "private Newbie managed Live checkpoint and offscreen Metal; no original guest"]
fn interface_selection_refresh_keeps_native_index_and_nested_panel_across_cold_restore()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("property test panicked")
}
fn run() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_PROPERTY_REFRESH_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
    let input = repo.join("cache/controller-live-newbie-20261022-reentry/live-session.json");
    let make = || -> Result<Viewer> {
        let args = Args::parse_from([
            "sims-room",
            "--original",
            "UserData/Houses/House07.iff",
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--load",
            input.to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut v, _) = Viewer::new(args);
        v.install(ready)?;
        let s = v.session.as_mut().unwrap();
        ensure!(s.host.presentation.property_refresh.is_none());
        ensure!(
            !s.host
                .presentation
                .notifications
                .as_ref()
                .unwrap()
                .dispatch
                .registry
                .nodes
                .values()
                .any(|node| node.handler == sims_vm::commander::house::Receiver::Interface),
            "historical restore silently installed Interface"
        );
        let before = s.simulation.clone();
        let saved_before = s.save_managed()?;
        for (view, renderer) in [(30000, 30000), (10100, 30001), (30000, 9000)] {
            ensure!(
                v.device
                    .as_mut()
                    .unwrap()
                    .bind_interface_properties(s, view, renderer)
                    .is_err(),
                "accepted aliased property binding"
            );
            ensure!(
                serde_json::to_value(&saved_before)? == serde_json::to_value(s.save_managed()?)?,
                "rejected property binding changed state"
            );
        }
        v.device
            .as_mut()
            .unwrap()
            .bind_interface_properties(s, 30000, 30001)?;
        s.bind_property_listener(8)?;
        ensure!(
            s.simulation == before,
            "frontend binding processed VM state"
        );
        Ok(v)
    };
    let (mut a, mut b) = (make()?, make()?);
    ensure!(retained(&a)? == retained(&b)?);
    let tick = a.session.as_ref().unwrap().simulation.vm.tick;
    let rng = |v: &Viewer| {
        let vm = &v.session.as_ref().unwrap().simulation.vm;
        (vm.rng_state, vm.lot_rng_state, vm.crt_rng_state)
    };
    let initial_rng = rng(&a);
    for v in [&mut a, &mut b] {
        v.session.as_mut().unwrap().begin_selection(Some(105))?;
    }
    let mut restores = BTreeSet::new();
    for slice in 0..10000 {
        for v in [&mut a, &mut b] {
            let s = v.session.as_mut().unwrap();
            if s.house_notification_pending() {
                v.device
                    .as_mut()
                    .unwrap()
                    .advance_notification_with_client(s, v.client.as_mut().unwrap(), 1)?;
            } else if s.simulation.selection_work_pending() {
                s.advance_selection(s.retained_camera_input()?, 1)?;
            }
            if !s.house_notification_pending() && !s.simulation.selection_work_pending() {
                // The player-selection handler publishes a frame when it
                // returns. Match that same pass on both viewers: cold restore
                // deliberately freezes the last surface while children run.
                let before = s.save_managed()?;
                v.refresh()?;
                ensure!(
                    serde_json::to_value(before)?
                        == serde_json::to_value(v.session.as_ref().unwrap().save_managed()?)?,
                    "selection return frame processed session/VM state"
                );
            }
        }
        let before = retained(&a)?;
        let actual = retained(&b)?;
        if before != actual {
            std::fs::write(
                output.join("first-expected.json"),
                serde_json::to_vec(&before)?,
            )?;
            std::fs::write(
                output.join("first-actual.json"),
                serde_json::to_vec(&actual)?,
            )?;
            anyhow::bail!("property first divergence at slice{slice}");
        }
        let label = b
            .session
            .as_ref()
            .unwrap()
            .host
            .presentation
            .property_refresh
            .as_ref()
            .unwrap()
            .panel_owner()
            .map(|_| {
                format!(
                    "{:?}",
                    b.device
                        .as_ref()
                        .unwrap()
                        .panel_widgets()
                        .unwrap()
                        .boundary()
                )
            });
        if let Some(label) = label.filter(|l| restores.insert(l.clone())) {
            let s = b.session.as_ref().unwrap();
            let saved = retained(&b)?;
            // Even a completed dispatcher belongs to the still-active native
            // child until its return is consumed. It cannot steal the outer
            // frame's owner just because its own instructions have finished.
            for (key, value) in [("caller", json!("Publication")), ("serial", json!(0))] {
                let mut bad = saved["device"].clone();
                bad["panel_widgets"]["call"][key] = value;
                ensure!(
                    CameraDevice::restore(s, serde_json::from_value(bad)?).is_err(),
                    "accepted foreign property panel {key}"
                );
            }
            ensure!(
                saved == retained(&b)?,
                "rejected property restore changed state"
            );
            cold(&mut b)?;
            eprintln!("restored property panel {label}");
        }
        ensure!(
            a.session.as_ref().unwrap().simulation.vm.tick == tick
                && rng(&a) == initial_rng
                && rng(&b) == initial_rng,
            "property selection advanced tick/RNG"
        );
        let s = a.session.as_ref().unwrap();
        if !s.house_notification_pending() && !s.simulation.selection_work_pending() {
            break;
        }
        ensure!(slice < 9999, "property selection exceeded slice budget");
    }
    for v in [&mut a, &mut b] {
        let s = v.session.as_mut().unwrap();
        ensure!(
            s.simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(105)
        );
        ensure!(
            s.host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .selected_person_index
                == Some(s.selected_portrait_index()?)
        );
        s.republish_interface()?;
    }
    for slice in 0..10000 {
        for v in [&mut a, &mut b] {
            let s = v.session.as_mut().unwrap();
            if s.publication_pending() {
                use sims_runtime::original_session::publication::Progress as P;
                let d = v.device.as_mut().unwrap();
                match d.advance_publication(s, 1)? {
                    P::Complete | P::Pending => (),
                    P::Panels(_)
                        if d.publish_replacement_people(s, v.client.as_mut().unwrap())? =>
                    {
                        ()
                    }
                    other => anyhow::bail!("property publication needs{other:?}"),
                }
            }
        }
        ensure!(
            retained(&a)? == retained(&b)?,
            "resumed publication differs at slice{slice}"
        );
        if !a.session.as_ref().unwrap().publication_pending() {
            break;
        }
        ensure!(slice < 9999, "property publication exceeded budget");
    }
    ensure!(
        !restores.is_empty(),
        "no nested property panel restore exercised"
    );
    for v in [&mut a, &mut b] {
        v.refresh()?;
    }
    ensure!(retained(&a)? == retained(&b)?);
    let s = a.session.as_ref().unwrap();
    ensure!(
        s.simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            == Some(105)
    );
    ensure!(
        a.client
            .as_ref()
            .unwrap()
            .live_panel()
            .and_then(|p| p.person())
            .map(|p| p.object)
            == Some(105)
    );
    let before = retained(&a)?;
    render_headless_element(&root, &output.join("selected.png"), a.view())?;
    ensure!(before == retained(&a)?, "draw processed property/VM state");
    std::fs::write(output.join("session.json"), serde_json::to_vec(&before)?)?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "input":input,"tick":tick,"selected":105,"cache_index":s.selected_portrait_index()?,
            "restored_panel_boundaries":restores,"full_retained_equality":true,"rng_unchanged":true,
            "draw_preserves_state":true,"gameplay_ticks_added":0,"strict_original_parity_ticks_added":0,
            "profile":"explicit new Interface listener and retained property backend; historical snapshot not silently upgraded"
        }))?,
    )?;
    Ok(())
}

fn fresh_time(v: &Viewer) -> serde_json::Value {
    let vm = &v.session.as_ref().unwrap().simulation.vm;
    json!([vm.tick, vm.rng_state, vm.lot_rng_state, vm.crt_rng_state])
}

fn fresh_selection(v: &Viewer, expected: Option<i16>) -> Result<serde_json::Value> {
    let s = v.session.as_ref().unwrap();
    ensure!(
        s.simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?
            == expected,
        "native selected person differs from {expected:?}"
    );
    let world = &s.simulation.vm.world;
    let flags = world
        .person
        .as_ref()
        .context("fresh person registry absent")?
        .person_order
        .iter()
        .map(|id| -> Result<_> {
            let flags = world.objects[id]
                .native_flags
                .context("fresh person flags absent")?;
            ensure!(
                (flags & 2 != 0) == (Some(*id) == expected),
                "native selection flag differs for person {id}"
            );
            Ok((*id, flags))
        })
        .collect::<Result<Vec<_>>>()?;
    let index = if let Some(person) = expected {
        let guid = world.objects[&person]
            .definition
            .as_ref()
            .context("selected person definition absent")?
            .guid;
        let index = sims_vm::interface::person_selection::cache_index(&world.image_cache, guid);
        ensure!(
            index >= 0,
            "selected resident is not in the actual portrait cache"
        );
        index
    } else {
        -1
    };
    ensure!(s.selected_portrait_index()? == index);
    let controls_index = s
        .host
        .presentation
        .transport
        .as_ref()
        .unwrap()
        .controls
        .selected_person_index;
    if controls_index != Some(index) {
        let output = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?
            .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
        std::fs::write(
            output.join("selection-owner-mismatch.json"),
            serde_json::to_vec_pretty(&json!({"expected_person":expected,
                "native_person":s.simulation.native_selected_person().map_err(anyhow::Error::msg)?,
                "native_flags":flags,"expected_cache_index":index,
                "controls_index":controls_index,"viewer_resident":v.resident,
                "pending":v.presentation_pending(),"tick_rng":fresh_time(v)}))?,
        )?;
        std::fs::write(
            output.join("selection-owner-mismatch-retained.json"),
            serde_json::to_vec(&retained(v)?)?,
        )?;
    }
    ensure!(
        controls_index == Some(index),
        "CPState differs from the actual selected portrait cache index: expected {index}, actual {controls_index:?}"
    );
    let publication = ui_publication::Published::capture(v);
    if let Some(person) = expected {
        ensure!(
            publication.resident == person
                && publication
                    .residents
                    .iter()
                    .any(|resident| resident.object == person && resident.enabled),
            "native selection differs from the published portrait selection border"
        );
    }
    ensure!(
        v.client
            .as_ref()
            .unwrap()
            .live_panel()
            .and_then(|panel| panel.person())
            .map(|person| person.object)
            == expected,
        "live panel person differs from native selection"
    );
    Ok(
        json!({"selected":expected,"native_flags":flags,"cache_index":index,
        "portrait_selection_border":publication.resident,
        "tutorial_overlays":publication.residents.iter().filter(|resident| resident.highlighted)
            .map(|resident| resident.object).collect::<Vec<_>>()}),
    )
}

fn frozen_surface_return(expected: &serde_json::Value, actual: &serde_json::Value) -> Result<()> {
    ensure!(
        exact_value_equal(&expected["session"], &actual["session"])
            && exact_value_equal(&expected["client"], &actual["client"]),
        "quiescent cold return changed session/client state"
    );
    let expected_device = expected["device"]
        .as_object()
        .context("expected device recipe absent")?;
    let actual_device = actual["device"]
        .as_object()
        .context("actual device recipe absent")?;
    let fields: BTreeSet<_> = expected_device.keys().chain(actual_device.keys()).collect();
    let differences = fields
        .into_iter()
        .filter(
            |field| match (expected_device.get(*field), actual_device.get(*field)) {
                (Some(expected), Some(actual)) => !exact_value_equal(expected, actual),
                _ => true,
            },
        )
        .map(String::as_str)
        .collect::<Vec<_>>();
    ensure!(
        differences == ["frozen_pixels"]
            && !expected_device.contains_key("frozen_pixels")
            && actual_device
                .get("frozen_pixels")
                .is_some_and(serde_json::Value::is_array),
        "quiescent device has unaccounted differences: {differences:?}"
    );
    Ok(())
}

#[test]
fn frozen_surface_return_accepts_only_unpublished_cold_pixels() -> Result<()> {
    let expected = json!({"session":{"tick":1},"client":{"owner":2},
        "device":{"generation":3,"published_generation":3}});
    let mut actual = expected.clone();
    actual["device"]["frozen_pixels"] = json!([0, 0, 0, 255]);
    frozen_surface_return(&expected, &actual)?;
    ensure!(expected["device"].get("frozen_pixels").is_none());
    ensure!(actual["device"]["frozen_pixels"] == json!([0, 0, 0, 255]));
    Ok(())
}

#[test]
fn frozen_surface_return_rejects_native_and_device_changes() {
    let expected = json!({"session":{"tick":1},"client":{"owner":2},
        "device":{"generation":3,"published_generation":3}});
    let mut frozen = expected.clone();
    frozen["device"]["frozen_pixels"] = json!([0, 0, 0, 255]);
    for field in ["/session/tick", "/client/owner", "/device/generation"] {
        let mut actual = frozen.clone();
        *actual.pointer_mut(field).unwrap() = json!(9);
        assert!(frozen_surface_return(&expected, &actual).is_err());
    }
    let mut actual = frozen.clone();
    actual["device"]["foreign_owner"] = json!(8);
    assert!(frozen_surface_return(&expected, &actual).is_err());
    actual = frozen;
    actual["device"]["frozen_pixels"] = serde_json::Value::Null;
    assert!(frozen_surface_return(&expected, &actual).is_err());
    assert!(frozen_surface_return(&expected, &expected).is_err());
}

#[test]
fn frozen_surface_return_rejects_signed_zero_in_all_retained_owners() {
    let expected = json!({"session":{"phase_window":-0.0},"client":{"offset":-0.0},
        "device":{"offset":-0.0}});
    assert!(exact_value_equal(&expected, &expected));
    assert!(!exact_value_equal(&json!([-0.0]), &json!([0.0])));
    assert!(!exact_value_equal(&json!(1), &json!(1.0)));
    for field in ["/session/phase_window", "/client/offset", "/device/offset"] {
        let mut actual = expected.clone();
        *actual.pointer_mut(field).unwrap() = json!(0.0);
        assert!(!exact_value_equal(&expected, &actual));
        actual["device"]["frozen_pixels"] = json!([0, 0, 0, 255]);
        assert!(frozen_surface_return(&expected, &actual).is_err());
    }
}

fn fresh_layout_size(
    display: [u32; 2],
    panel_state: Option<i32>,
    mode: Option<i32>,
) -> Result<[u32; 2]> {
    ensure!(
        display
            .iter()
            .all(|dimension| (1..=4096).contains(dimension)),
        "fresh layout display dimensions unavailable"
    );
    let footer = if panel_state == Some(1) {
        0
    } else if mode == Some(1) {
        150
    } else {
        100
    };
    Ok([display[0], display[1].saturating_sub(footer).max(1)])
}

fn fresh_layout_height_target(scene: [u32; 2], target: [u32; 2]) -> Option<[u32; 2]> {
    (scene[0] == target[0] && (1..=4096).contains(&scene[1]) && scene[1] != target[1])
        .then_some(target)
}

fn fresh_layout_target(viewer: &Viewer) -> Result<Option<[u32; 2]>> {
    let session = viewer
        .session
        .as_ref()
        .context("fresh layout session absent")?;
    let device = viewer
        .device
        .as_ref()
        .context("fresh layout device absent")?;
    let mode = session
        .host
        .presentation
        .transport
        .as_ref()
        .map(|transport| transport.controls.mode);
    let target = fresh_layout_size(viewer.display_size, device.panel_state(), mode)?;
    if fresh_layout_height_target([device.scene().width, device.scene().height], target).is_none() {
        return Ok(None);
    }
    ensure!(
        !viewer.presentation_pending()
            && !viewer.diagnostics
            && !session.clock_pending()
            && session.simulation.vm.scheduled_object_visit.is_none()
            && viewer
                .client
                .as_ref()
                .is_some_and(|client| !client.pending()),
        "fresh layout completion overlaps native or preview work"
    );
    Ok(Some(target))
}

fn fresh_layout_expected(
    before: &serde_json::Value,
    target: [u32; 2],
) -> Result<serde_json::Value> {
    ensure!(
        target
            .iter()
            .all(|dimension| (1..=4096).contains(dimension)),
        "fresh layout target dimensions unavailable"
    );
    let paths = [
        "/view/camera/viewport",
        "/view/presenter/camera/settings/viewport",
        "/view/viewport",
    ];
    let previous = before
        .pointer(paths[0])
        .context("fresh layout camera viewport absent")?;
    let viewport: [u32; 4] = serde_json::from_value(previous.clone())?;
    ensure!(
        viewport[0] == 0
            && viewport[1] == 0
            && viewport[2] == target[0]
            && (1..=4096).contains(&viewport[3])
            && viewport[3] != target[1]
            && exact_value_equal(previous, &json!(viewport)),
        "fresh layout is not an initialized height-only change"
    );
    let mut expected = before.clone();
    for path in paths {
        ensure!(
            before
                .pointer(path)
                .is_some_and(|value| exact_value_equal(previous, value)),
            "fresh layout viewport aliases differ at {path}"
        );
        expected.pointer_mut(path).unwrap()[3] = json!(target[1]);
    }
    Ok(expected)
}

fn fresh_layout_fixture() -> serde_json::Value {
    json!({"view":{"camera":{"viewport":[0,0,1320,900]},
        "presenter":{"camera":{"settings":{"viewport":[0,0,1320,900]}}},
        "viewport":[0,0,1320,900],"selected":21,"cpstate":262},
        "simulation":{"tick":88444,"rng":[1045156283_u32,2528747409_u32,2284264158_u32],
            "environment":-0.0,"classification":1}})
}

#[test]
fn fresh_layout_completion_accepts_only_exact_height_change() -> Result<()> {
    let before = fresh_layout_fixture();
    let original = before.clone();
    let expected = fresh_layout_expected(&before, [1320, 800])?;
    let mut actual = before.clone();
    for path in [
        "/view/camera/viewport/3",
        "/view/presenter/camera/settings/viewport/3",
        "/view/viewport/3",
    ] {
        *actual.pointer_mut(path).unwrap() = json!(800);
    }
    ensure!(exact_value_equal(&expected, &actual));
    ensure!(exact_value_equal(&before, &original));
    ensure!(!exact_value_equal(&expected, &before));
    Ok(())
}

#[test]
fn fresh_layout_completion_rejects_extra_native_changes() -> Result<()> {
    let before = fresh_layout_fixture();
    let expected = fresh_layout_expected(&before, [1320, 800])?;
    for path in [
        "/view/selected",
        "/view/cpstate",
        "/simulation/tick",
        "/simulation/rng/0",
        "/simulation/rng/1",
        "/simulation/rng/2",
        "/simulation/environment",
        "/simulation/classification",
    ] {
        let mut actual = expected.clone();
        *actual.pointer_mut(path).unwrap() = json!(9);
        ensure!(!exact_value_equal(&expected, &actual), "accepted {path}");
    }
    let mut actual = expected.clone();
    actual["view"]["foreign_owner"] = json!(7);
    ensure!(!exact_value_equal(&expected, &actual));
    Ok(())
}

#[test]
fn fresh_layout_completion_rejects_wrong_geometry_or_aliases() -> Result<()> {
    let before = fresh_layout_fixture();
    let expected = fresh_layout_expected(&before, [1320, 800])?;
    for path in [
        "/view/camera/viewport",
        "/view/presenter/camera/settings/viewport",
        "/view/viewport",
    ] {
        for viewport in [json!([0, 0, 1320, 799]), json!([0, 0, 1319, 800])] {
            let mut actual = expected.clone();
            *actual.pointer_mut(path).unwrap() = viewport;
            ensure!(!exact_value_equal(&expected, &actual));
        }
        let mut inconsistent = before.clone();
        inconsistent.pointer_mut(path).unwrap()[3] = json!(899);
        ensure!(fresh_layout_expected(&inconsistent, [1320, 800]).is_err());
    }
    ensure!(fresh_layout_expected(&before, [1319, 800]).is_err());
    ensure!(fresh_layout_expected(&before, [1320, 900]).is_err());
    ensure!(fresh_layout_expected(&before, [1320, 0]).is_err());
    Ok(())
}

#[test]
fn fresh_layout_completion_rejects_numeric_and_signed_zero_changes() -> Result<()> {
    let before = fresh_layout_fixture();
    let expected = fresh_layout_expected(&before, [1320, 800])?;
    for (path, value) in [
        ("/view/camera/viewport/3", json!(800.0)),
        ("/view/presenter/camera/settings/viewport/3", json!(800.0)),
        ("/view/viewport/3", json!(800.0)),
        ("/view/viewport/0", json!(-0.0)),
        ("/simulation/environment", json!(0.0)),
    ] {
        let mut actual = expected.clone();
        *actual.pointer_mut(path).unwrap() = value;
        ensure!(!exact_value_equal(&expected, &actual), "accepted {path}");
    }
    Ok(())
}

#[test]
fn fresh_layout_completion_requires_initialized_integer_viewports() {
    let before = fresh_layout_fixture();
    for path in [
        "/view/camera/viewport",
        "/view/presenter/camera/settings/viewport",
        "/view/viewport",
    ] {
        for viewport in [
            serde_json::Value::Null,
            json!([0, 0, 1320]),
            json!([0, 0, 1320, 900.0]),
            json!([-0.0, 0, 1320, 900]),
        ] {
            let mut invalid = before.clone();
            *invalid.pointer_mut(path).unwrap() = viewport;
            assert!(fresh_layout_expected(&invalid, [1320, 800]).is_err());
        }
    }
}

#[test]
fn fresh_layout_target_uses_native_panel_and_display_geometry() -> Result<()> {
    ensure!(fresh_layout_size([1320, 900], Some(1), Some(2))? == [1320, 900]);
    ensure!(fresh_layout_size([1320, 900], Some(0), Some(2))? == [1320, 800]);
    ensure!(fresh_layout_size([1320, 900], Some(0), Some(1))? == [1320, 750]);
    ensure!(fresh_layout_size([1320, 900], None, None)? == [1320, 800]);
    ensure!(fresh_layout_size([1320, 50], Some(0), Some(1))? == [1320, 1]);
    ensure!(fresh_layout_size([0, 900], Some(0), Some(2)).is_err());
    ensure!(fresh_layout_size([1320, 4097], Some(0), Some(2)).is_err());
    Ok(())
}

#[test]
fn fresh_layout_initial_width_resize_keeps_original_return_probe() {
    assert_eq!(fresh_layout_height_target([1000, 740], [1320, 900]), None);
    assert_eq!(fresh_layout_height_target([1320, 900], [1320, 900]), None);
    assert_eq!(fresh_layout_height_target([1320, 0], [1320, 800]), None);
    assert_eq!(fresh_layout_height_target([1320, 4097], [1320, 800]), None);
    assert_eq!(
        fresh_layout_height_target([1320, 900], [1320, 800]),
        Some([1320, 800])
    );
    assert_eq!(
        fresh_layout_height_target([1320, 800], [1320, 900]),
        Some([1320, 900])
    );
}

fn fresh_normal_frame_members(objects: &[i16]) -> Result<BTreeSet<i16>> {
    let members: BTreeSet<_> = objects.iter().copied().collect();
    ensure!(
        objects.iter().all(|object| *object > 0) && members.len() == objects.len(),
        "normal-frame inventory contains null or duplicate objects"
    );
    Ok(members)
}

fn fresh_normal_frame_expected_state(
    before: &serde_json::Value,
    order: &sims_vm::presenter::RegistryOrder,
    lists: &sims_vm::presenter::Lists,
) -> Result<serde_json::Value> {
    let order_path = "/session/simulation/state/vm/world/registry_order";
    let lists_path = "/session/view/presenter/lists";
    let old_order_value = before
        .pointer(order_path)
        .context("frame registry absent")?;
    let old_lists_value = before.pointer(lists_path).context("frame lists absent")?;
    let old_order: sims_vm::presenter::RegistryOrder =
        serde_json::from_value(old_order_value.clone())?;
    let old_lists: sims_vm::presenter::Lists = serde_json::from_value(old_lists_value.clone())?;
    old_order.validate().map_err(anyhow::Error::msg)?;
    order.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        exact_value_equal(old_order_value, &serde_json::to_value(&old_order)?)
            && exact_value_equal(old_lists_value, &serde_json::to_value(&old_lists)?)
            && old_order.dirty
            && old_order.rotation.is_some()
            && !old_order.objects.is_empty()
            && !order.dirty
            && order.rotation == old_order.rotation,
        "normal-frame model requires an initialized dirty native cache"
    );
    let members = fresh_normal_frame_members(&old_order.objects)?;
    ensure!(
        members == fresh_normal_frame_members(&order.objects)?,
        "normal-frame registry membership changed"
    );
    for (previous, calculated) in [
        (&old_lists.dynamic, &lists.dynamic),
        (&old_lists.ordinary, &lists.ordinary),
        (&old_lists.special, &lists.special),
    ] {
        let calculated = fresh_normal_frame_members(calculated)?;
        ensure!(
            fresh_normal_frame_members(previous)? == calculated && calculated.is_subset(&members),
            "normal-frame classification membership changed"
        );
    }
    let mut expected = before.clone();
    *expected.pointer_mut(order_path).unwrap() = serde_json::to_value(order)?;
    *expected.pointer_mut(lists_path).unwrap() = serde_json::to_value(lists)?;
    Ok(expected)
}

fn fresh_normal_frame_model(
    viewer: &Viewer,
    before: &serde_json::Value,
) -> Result<serde_json::Value> {
    use sims_sim::lot_presenter::View;
    use sims_vm::presenter::{Classification, Lists};
    let session = viewer.session.as_ref().context("frame session absent")?;
    let presenter = session
        .host
        .presentation
        .presenter
        .as_ref()
        .context("frame presenter absent")?;
    presenter
        .validate_simulation(&session.simulation)
        .map_err(anyhow::Error::msg)?;
    ensure!(presenter.complete(), "frame presenter already running");
    ensure!(
        presenter.inputs.graphic_ready
            == session
                .host
                .ordinary
                .presenter_readiness(&session.simulation.vm, &session.program)?,
        "normal-frame model would change graphic readiness"
    );
    let links = &session
        .simulation
        .native_lot
        .as_ref()
        .context("frame lot absent")?
        .placement
        .links;
    let mut world = session.simulation.vm.world.clone();
    let world_before = serde_json::to_value(&world)?;
    let mut order = world
        .registry_order
        .clone()
        .context("frame registry absent")?;
    order
        .get(
            &View::new(&mut world, &session.program, links, &presenter.inputs, &[])
                .map_err(anyhow::Error::msg)?,
        )
        .map_err(anyhow::Error::msg)?;
    let mut classification = Classification::default();
    let mut lists = Lists::default();
    let request = classification
        .advance(
            &mut View::new(
                &mut world,
                &session.program,
                links,
                &presenter.inputs,
                &order.objects,
            )
            .map_err(anyhow::Error::msg)?,
            &mut lists,
        )
        .map_err(anyhow::Error::msg)?;
    ensure!(
        request.is_none()
            && classification.complete()
            && exact_value_equal(&world_before, &serde_json::to_value(&world)?),
        "normal-frame model requires invalidation or another world write"
    );
    fresh_normal_frame_expected_state(before, &order, &lists)
}

fn fresh_normal_frame_needed(viewer: &Viewer) -> Result<bool> {
    let session = viewer.session.as_ref().context("frame session absent")?;
    let Some(order) = session.simulation.vm.world.registry_order.as_ref() else {
        return Ok(false);
    };
    if !order.dirty || order.rotation.is_none() {
        return Ok(false);
    }
    let device = viewer.device.as_ref().context("frame device absent")?;
    let target = fresh_layout_size(
        viewer.display_size,
        device.panel_state(),
        session
            .host
            .presentation
            .transport
            .as_ref()
            .map(|transport| transport.controls.mode),
    )?;
    if [device.scene().width, device.scene().height] != target {
        return Ok(false);
    }
    ensure!(
        !viewer.presentation_pending()
            && !viewer.diagnostics
            && !session.view_refresh_pending()
            && !session.clock_pending()
            && session.simulation.vm.scheduled_object_visit.is_none()
            && viewer
                .client
                .as_ref()
                .is_some_and(|client| !client.pending()),
        "normal-frame completion overlaps native or preview work"
    );
    Ok(true)
}

fn fresh_normal_frame_owned_state(
    before: &serde_json::Value,
    pixels: &serde_json::Value,
) -> Result<serde_json::Value> {
    let device = before["device"]
        .as_object()
        .context("frame device absent")?;
    let entry_tick = before
        .pointer("/session/simulation/state/vm/tick")
        .and_then(serde_json::Value::as_u64)
        .context("normal-frame entry tick absent")?;
    ensure!(
        !device.contains_key("normal_frame")
            && device["dirty"] == json!(false)
            && exact_value_equal(&device["frame"], &device["published_frame"])
            && device["frame"]["tick"]
                .as_u64()
                .is_some_and(|tick| tick <= entry_tick)
            && device["generation"].as_u64().is_some()
            && exact_value_equal(&device["generation"], &device["published_generation"]),
        "normal-frame requires a settled last-published recipe"
    );
    let viewport = device["published_frame"]["input"]["viewport"]
        .as_array()
        .context("published frame viewport absent")?;
    ensure!(viewport.len() == 4, "invalid published frame viewport");
    let width = viewport[2]
        .as_u64()
        .context("published width is not an integer")?;
    let height = viewport[3]
        .as_u64()
        .context("published height is not an integer")?;
    let pixels_array = pixels.as_array().context("published pixels absent")?;
    ensure!(
        width > 0
            && height > 0
            && width
                .checked_mul(height)
                .and_then(|area| area.checked_mul(4))
                == Some(u64::try_from(pixels_array.len())?)
            && pixels_array
                .iter()
                .all(|byte| byte.as_u64().is_some_and(|byte| byte <= 255))
            && device
                .get("frozen_pixels")
                .is_none_or(|saved| exact_value_equal(saved, pixels)),
        "normal-frame requires the exact last-published pixel bytes"
    );
    let mut expected = before.clone();
    expected["device"]["normal_frame"] = json!({
        "entry_tick":entry_tick,
        "frame":device["frame"], "published_frame":device["published_frame"],
        "dirty":device["dirty"], "generation":device["generation"],
        "published_generation":device["published_generation"]});
    expected["device"]["frozen_pixels"] = pixels.clone();
    Ok(expected)
}

fn fresh_normal_frame_publication_expected(
    native_expected: &serde_json::Value,
) -> Result<serde_json::Value> {
    let device = native_expected["device"]
        .as_object()
        .context("frame device absent")?;
    ensure!(
        !device.contains_key("normal_frame")
            && device["dirty"] == json!(false)
            && exact_value_equal(
                &device["frame"]["tick"],
                native_expected
                    .pointer("/session/simulation/state/vm/tick")
                    .context("normal-frame publication tick absent")?
            )
            && exact_value_equal(&device["frame"], &device["published_frame"])
            && exact_value_equal(&device["generation"], &device["published_generation"]),
        "normal-frame publication requires unchanged settled frame values"
    );
    let generation = device["generation"]
        .as_u64()
        .and_then(|generation| generation.checked_add(1))
        .context("normal-frame publication generation overflow")?;
    let mut expected = native_expected.clone();
    expected["device"]["generation"] = json!(generation);
    expected["device"]["published_generation"] = json!(generation);
    expected["device"]
        .as_object_mut()
        .unwrap()
        .remove("frozen_pixels");
    Ok(expected)
}

fn fresh_normal_frame_cut(
    before: &[serde_json::Value; 2],
    owned: &[serde_json::Value; 2],
    expected: &[serde_json::Value; 2],
    actual: &[serde_json::Value; 2],
    pending: [bool; 2],
) -> Result<()> {
    ensure!(
        pending[0] == pending[1],
        "normal-frame pending owners differ"
    );
    ensure!(
        exact_value_equal(&actual[0], &actual[1]),
        "paired raw normal-frame checkpoint differs"
    );
    if !pending[0] {
        for (expected, actual) in expected.iter().zip(actual) {
            ensure!(
                exact_value_equal(&expected["session"], &actual["session"]),
                "normal-frame session differs from independent native calculation"
            );
            ensure!(
                exact_value_equal(&expected["device"], &actual["device"]),
                "real normal-frame publication differs from exact device recipe"
            );
            ensure!(
                exact_value_equal(expected, actual),
                "published normal-frame differs from complete calculated state"
            );
        }
        return Ok(());
    }
    for (index, (before, actual)) in before.iter().zip(actual).enumerate() {
        for path in [
            "/session/view/selected",
            "/session/view/cpstate",
            "/session/simulation/state/vm/tick",
            "/session/simulation/state/vm/rng_state",
            "/session/simulation/state/vm/lot_rng_state",
            "/session/simulation/state/vm/crt_rng_state",
        ] {
            ensure!(
                match (before.pointer(path), actual.pointer(path)) {
                    (Some(expected), Some(actual)) => exact_value_equal(expected, actual),
                    (None, None) => true,
                    _ => false,
                },
                "pending normal-frame changed native input/clock at {path}"
            );
        }
        let before = before.as_object().context("frame retained state absent")?;
        let actual = actual.as_object().context("frame actual state absent")?;
        ensure!(
            before.len() == actual.len(),
            "frame retained owners changed"
        );
        for (owner, value) in before {
            let actual = actual.get(owner).context("frame retained owner absent")?;
            let expected = if owner == "device" {
                &owned[index]["device"]
            } else {
                value
            };
            ensure!(
                owner == "session" || exact_value_equal(expected, actual),
                "native normal-frame changed retained {owner}"
            );
        }
    }
    Ok(())
}

fn complete_fresh_normal_frame_pair(
    first: &mut Viewer,
    resumed: &mut Viewer,
    time: &serde_json::Value,
    restores: &mut BTreeSet<String>,
    phase: &str,
    slice: usize,
) -> Result<()> {
    use sims_runtime::original_session::presenter::Progress;
    let needed = fresh_normal_frame_needed(first)?;
    ensure!(
        needed == fresh_normal_frame_needed(resumed)?,
        "cold normal-frame admission differs during {phase} at slice {slice}"
    );
    if !needed {
        return Ok(());
    }
    let before = [retained(first)?, retained(resumed)?];
    let published_pixels = |viewer: &Viewer| -> Result<serde_json::Value> {
        Ok(serde_json::to_value(
            sims_render::software::render(
                viewer
                    .device
                    .as_ref()
                    .context("frame device absent")?
                    .scene(),
            )
            .into_raw(),
        )?)
    };
    let owned = [
        fresh_normal_frame_owned_state(&before[0], &published_pixels(first)?)?,
        fresh_normal_frame_owned_state(&before[1], &published_pixels(resumed)?)?,
    ];
    let expected = [
        fresh_normal_frame_publication_expected(&fresh_normal_frame_model(first, &before[0])?)?,
        fresh_normal_frame_publication_expected(&fresh_normal_frame_model(resumed, &before[1])?)?,
    ];
    let preserve_failure = |error: anyhow::Error,
                            actual: &[serde_json::Value; 2],
                            pending: [bool; 2],
                            step: Option<usize>| {
        let preserve = || -> Result<()> {
            let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?
                .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
            for (index, branch) in ["uninterrupted", "cold-restored"].into_iter().enumerate() {
                for (suffix, value) in [
                    ("before", &before[index]),
                    ("owned", &owned[index]),
                    ("expected", &expected[index]),
                    ("actual", &actual[index]),
                ] {
                    std::fs::write(
                        output.join(format!(
                            "normal-frame-{phase}-{slice}-{branch}-{suffix}.json"
                        )),
                        serde_json::to_vec(value)?,
                    )?;
                }
            }
            std::fs::write(
                output.join(format!("normal-frame-{phase}-{slice}-cut.json")),
                serde_json::to_vec_pretty(&json!({"phase":phase,"slice":slice,
                        "advance":step,"pending":pending,"error":format!("{error:#}"),
                        "tick_rng":time,"acceptance_rules_changed":false}))?,
            )?;
            Ok(())
        };
        match preserve() {
                Ok(()) => error.context(format!("normal-frame checkpoint {phase}/{slice}/{step:?}")),
                Err(failure) => error.context(format!(
                    "normal-frame checkpoint {phase}/{slice}/{step:?}; diagnostic preservation failed: {failure:#}"
                )),
            }
    };
    let check_cut = |actual: &[serde_json::Value; 2], pending: [bool; 2], step: Option<usize>| {
        fresh_normal_frame_cut(&before, &owned, &expected, actual, pending)
            .map_err(|error| preserve_failure(error, actual, pending, step))
    };
    let capture_failure =
        |error: anyhow::Error, first: &Viewer, resumed: &Viewer, step: Option<usize>| match (
            retained(first),
            retained(resumed),
        ) {
            (Ok(first_state), Ok(resumed_state)) => preserve_failure(
                error,
                &[first_state, resumed_state],
                [first.presentation_pending(), resumed.presentation_pending()],
                step,
            ),
            (first_state, resumed_state) => error.context(format!(
                "normal-frame diagnostic snapshot failed: {:?}; {:?}",
                first_state.err(),
                resumed_state.err(),
            )),
        };
    for viewer in [&mut *first, &mut *resumed] {
        viewer
            .device
            .as_mut()
            .unwrap()
            .begin_normal_view_frame(viewer.session.as_mut().unwrap())?;
    }
    check_cut(
        &[retained(first)?, retained(resumed)?],
        [first.presentation_pending(), resumed.presentation_pending()],
        None,
    )?;
    let mut child_free_classification = false;
    for step in 0..10000 {
        cold(resumed)
            .with_context(|| format!("normal-frame cold cut {phase}/{slice}/{step}"))
            .map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
        restores.insert("normal-frame".to_string());
        let advance = |viewer: &mut Viewer| {
            let session = viewer.session.as_mut().unwrap();
            viewer
                .device
                .as_mut()
                .unwrap()
                .advance_normal_view_frame(session, 1)
        };
        let progress =
            advance(first).map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
        let restored_progress =
            advance(resumed).map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
        ensure!(
            progress == restored_progress,
            "cold normal-frame progress differs during {phase} at slice {slice}, step {step}"
        );
        ensure!(
            matches!(progress, Progress::Pending | Progress::Complete),
            "normal-frame completion requires an unmodeled external service: {progress:?}"
        );
        let actual = [retained(first)?, retained(resumed)?];
        check_cut(
            &actual,
            [first.presentation_pending(), resumed.presentation_pending()],
            Some(step),
        )?;
        if actual[0].pointer("/session/view/presenter/normal_frame/phase")
            == Some(&json!("Classification"))
        {
            for viewer in [&*first, &*resumed] {
                ensure!(
                    !viewer
                        .session
                        .as_ref()
                        .unwrap()
                        .simulation
                        .selection_work_pending()
                        && viewer.presentation_pending(),
                    "child-free Classification lost its surface fence"
                );
            }
            restores.insert("normal-frame-classification".to_string());
            child_free_classification = true;
        }
        ensure!(
            exact_value_equal(&fresh_time(first), time)
                && exact_value_equal(&fresh_time(resumed), time),
            "normal-frame advanced tick/RNG during {phase} at slice {slice}, step {step}"
        );
        if progress == Progress::Complete {
            ensure!(
                child_free_classification,
                "normal-frame missed the Indicators-to-Classification cold cut"
            );
            cold(resumed)
                .context("normal-frame completed-native saveable handoff")
                .map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
            restores.insert("normal-frame-handoff".to_string());
            check_cut(
                &[retained(first)?, retained(resumed)?],
                [first.presentation_pending(), resumed.presentation_pending()],
                Some(step),
            )?;
            for viewer in [&mut *first, &mut *resumed] {
                let before = retained(viewer)?;
                ensure!(
                    !viewer.session.as_ref().unwrap().view_refresh_pending(),
                    "camera rejection did not reach the native-complete handoff"
                );
                ensure!(
                    viewer
                        .device
                        .as_ref()
                        .unwrap()
                        .begin(viewer.session.as_mut().unwrap(), Change::Rotation(1))
                        .is_err(),
                    "camera admission stole the normal-frame surface handoff"
                );
                ensure!(exact_value_equal(&before, &retained(viewer)?));
                ensure!(
                    viewer
                        .device
                        .as_mut()
                        .unwrap()
                        .advance(viewer.session.as_mut().unwrap(), 1)
                        .is_err(),
                    "camera advancement entered the normal-frame surface handoff"
                );
                ensure!(exact_value_equal(&before, &retained(viewer)?));
            }
            let publish = |viewer: &mut Viewer| -> Result<()> {
                let _ = viewer.update(Message::Step);
                ensure!(
                    viewer.error.is_none(),
                    "normal-frame Step failed: {:?}",
                    viewer.error
                );
                Ok(())
            };
            publish(first).map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
            publish(resumed).map_err(|error| capture_failure(error, first, resumed, Some(step)))?;
            let actual = [retained(first)?, retained(resumed)?];
            check_cut(
                &actual,
                [first.presentation_pending(), resumed.presentation_pending()],
                Some(step),
            )?;
            let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?
                .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
            for (index, branch) in ["uninterrupted", "cold-restored"].into_iter().enumerate() {
                if !exact_value_equal(&expected[index], &actual[index]) {
                    for (suffix, value) in [
                        ("before", &before[index]),
                        ("expected", &expected[index]),
                        ("actual", &actual[index]),
                    ] {
                        std::fs::write(
                            output.join(format!(
                                "normal-frame-{phase}-{slice}-{branch}-{suffix}.json"
                            )),
                            serde_json::to_vec(value)?,
                        )?;
                    }
                }
                ensure!(
                    exact_value_equal(&expected[index], &actual[index]),
                    "normal-frame differs from calculated native state and real publication during {phase} at slice {slice} ({branch})"
                );
            }
            for viewer in [&mut *first, &mut *resumed] {
                ensure!(
                    !viewer.presentation_pending()
                        && !viewer.session.as_ref().unwrap().view_refresh_pending(),
                    "normal-frame completion left pending work"
                );
            }
            std::fs::write(
                output.join(format!("normal-frame-{phase}-{slice}.json")),
                serde_json::to_vec_pretty(&json!({"phase":phase,"slice":slice,
                    "steps":step + 1,"budget_per_advance":1,"cold_restores":step + 2,
                    "whole_retained_states_match_calculation":true,"tick_rng":time,
                    "saveable_publication_handoff":true,"real_device_publications":1,
                    "event_resume":"Message::Step",
                    "camera_begin_and_advance_reject_without_state_change":true,
                    "indicators_classification_cold_cut":child_free_classification,
                    "modeled_paths":["/session/simulation/state/vm/world/registry_order",
                        "/session/view/presenter/lists","/device/generation",
                        "/device/published_generation","/device/frozen_pixels"]}))?,
            )?;
            return Ok(());
        }
    }
    anyhow::bail!("normal-frame completion exceeded bounded step budget")
}

fn fresh_normal_frame_fixture() -> (
    serde_json::Value,
    sims_vm::presenter::RegistryOrder,
    sims_vm::presenter::Lists,
) {
    (
        json!({"session":{"simulation":{"state":{"vm":{"tick":88444,
            "rng_state":1045156283_u32,"lot_rng_state":2528747409_u32,
            "crt_rng_state":2284264158_u32,"world":{"environment":-0.0,
                "registry_order":{"dirty":true,"rotation":0,"objects":[105,383,21]}}}}},
            "view":{"selected":21,"cpstate":262,"presenter":{"inputs":{"owner":7},
                "lists":{"dynamic":[21,105],"ordinary":[21,383],"special":[]}}}},
            "client":{"owner":2,"offset":-0.0},
            "device":{"generation":28,"published_generation":28,"dirty":false,
                "frame":{"input":{"viewport":[0,0,1,1],"center":[0,0],"zoom":2,
                    "rotation":0},"floor":1,"tick":88444},
                "published_frame":{"input":{"viewport":[0,0,1,1],"center":[0,0],"zoom":2,
                    "rotation":0},"floor":1,"tick":88444},"offset":-0.0}}),
        sims_vm::presenter::RegistryOrder {
            dirty: false,
            rotation: Some(0),
            objects: vec![21, 383, 105],
        },
        sims_vm::presenter::Lists {
            dynamic: vec![105, 21],
            ordinary: vec![383, 21],
            special: vec![],
        },
    )
}

#[test]
fn fresh_normal_frame_completion_accepts_only_calculated_cache_and_lists() -> Result<()> {
    let (before, order, lists) = fresh_normal_frame_fixture();
    let original = before.clone();
    let expected = fresh_normal_frame_expected_state(&before, &order, &lists)?;
    let mut actual = before.clone();
    let registry = &mut actual["session"]["simulation"]["state"]["vm"]["world"]["registry_order"];
    registry["dirty"] = json!(false);
    registry["objects"] = json!([21, 383, 105]);
    actual["session"]["view"]["presenter"]["lists"] =
        json!({"dynamic":[105,21],"ordinary":[383,21],"special":[]});
    ensure!(exact_value_equal(&expected, &actual));
    ensure!(exact_value_equal(&before, &original));
    ensure!(!exact_value_equal(&expected, &before));
    Ok(())
}

#[test]
fn fresh_normal_frame_completion_rejects_extra_saved_state_changes() -> Result<()> {
    let (before, order, lists) = fresh_normal_frame_fixture();
    let expected = fresh_normal_frame_expected_state(&before, &order, &lists)?;
    for path in [
        "/session/view/selected",
        "/session/view/cpstate",
        "/session/view/presenter/inputs/owner",
        "/session/simulation/state/vm/tick",
        "/session/simulation/state/vm/rng_state",
        "/session/simulation/state/vm/lot_rng_state",
        "/session/simulation/state/vm/crt_rng_state",
        "/session/simulation/state/vm/world/environment",
        "/session/simulation/state/vm/world/registry_order/objects/0",
        "/session/view/presenter/lists/dynamic/0",
        "/session/view/presenter/lists/ordinary/0",
        "/client/owner",
        "/device/generation",
    ] {
        let mut actual = expected.clone();
        *actual.pointer_mut(path).unwrap() = json!(9);
        ensure!(!exact_value_equal(&expected, &actual), "accepted {path}");
    }
    let mut actual = expected.clone();
    actual["session"]["view"]["presenter"]["normal_frame"] =
        json!({"tick":88444,"phase":"Classification"});
    ensure!(!exact_value_equal(&expected, &actual));
    actual = expected.clone();
    actual["device"]["foreign_owner"] = json!(9);
    ensure!(!exact_value_equal(&expected, &actual));
    Ok(())
}

#[test]
fn fresh_normal_frame_completion_rejects_numeric_and_signed_zero_changes() -> Result<()> {
    let (before, order, lists) = fresh_normal_frame_fixture();
    let expected = fresh_normal_frame_expected_state(&before, &order, &lists)?;
    for (path, value) in [
        (
            "/session/simulation/state/vm/world/registry_order/rotation",
            json!(-0.0),
        ),
        (
            "/session/simulation/state/vm/world/registry_order/objects/0",
            json!(21.0),
        ),
        ("/session/view/presenter/lists/dynamic/0", json!(105.0)),
        ("/session/simulation/state/vm/world/environment", json!(0.0)),
        ("/client/offset", json!(0.0)),
        ("/device/offset", json!(0.0)),
    ] {
        let mut actual = expected.clone();
        *actual.pointer_mut(path).unwrap() = value;
        ensure!(!exact_value_equal(&expected, &actual), "accepted {path}");
    }
    Ok(())
}

#[test]
fn fresh_normal_frame_completion_rejects_uninitialized_or_noncanonical_inputs() -> Result<()> {
    let (before, order, lists) = fresh_normal_frame_fixture();
    for (path, value) in [
        (
            "/session/simulation/state/vm/world/registry_order/dirty",
            json!(false),
        ),
        (
            "/session/simulation/state/vm/world/registry_order/rotation",
            json!(null),
        ),
        (
            "/session/simulation/state/vm/world/registry_order/rotation",
            json!(0.0),
        ),
        (
            "/session/simulation/state/vm/world/registry_order/objects",
            json!([]),
        ),
        ("/session/view/presenter/lists/dynamic/0", json!(21.0)),
    ] {
        let mut invalid = before.clone();
        *invalid.pointer_mut(path).unwrap() = value;
        ensure!(fresh_normal_frame_expected_state(&invalid, &order, &lists).is_err());
    }
    let mut invalid = before;
    invalid["session"]["view"]["presenter"]["lists"]["foreign_owner"] = json!(7);
    ensure!(fresh_normal_frame_expected_state(&invalid, &order, &lists).is_err());
    Ok(())
}

#[test]
fn fresh_normal_frame_completion_rejects_inventory_changes_and_duplicates() -> Result<()> {
    let (before, order, lists) = fresh_normal_frame_fixture();
    for objects in [vec![21, 383, 7], vec![21, 383, 21], vec![21, 383, 0]] {
        let mut invalid = order.clone();
        invalid.objects = objects;
        ensure!(fresh_normal_frame_expected_state(&before, &invalid, &lists).is_err());
    }
    for rotation in [None, Some(1)] {
        let mut invalid = order.clone();
        invalid.rotation = rotation;
        ensure!(fresh_normal_frame_expected_state(&before, &invalid, &lists).is_err());
    }
    let mut invalid = order.clone();
    invalid.dirty = true;
    ensure!(fresh_normal_frame_expected_state(&before, &invalid, &lists).is_err());
    for dynamic in [vec![105, 7], vec![105, 105], vec![105]] {
        let mut invalid = lists.clone();
        invalid.dynamic = dynamic;
        ensure!(fresh_normal_frame_expected_state(&before, &order, &invalid).is_err());
    }
    let mut invalid = lists.clone();
    invalid.special.push(383);
    ensure!(fresh_normal_frame_expected_state(&before, &order, &invalid).is_err());
    Ok(())
}

#[test]
fn fresh_normal_frame_completion_keeps_pending_cuts_and_frozen_pixels_exact() -> Result<()> {
    let (before, owned, expected) = fresh_normal_frame_cut_fixture()?;
    let mut actual = owned.clone();
    for state in &mut actual {
        state["session"]["view"]["presenter"]["normal_frame"] =
            json!({"tick":88444,"phase":"Indicators"});
        state["session"]["view"]["presenter"]["drain_started"] = json!(true);
    }
    fresh_normal_frame_cut(&before, &owned, &expected, &actual, [true, true])?;
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &actual, [false, false]).is_err());
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &actual, [false, true]).is_err());
    for state in &mut actual {
        state["session"]["view"]["presenter"]["normal_frame"]["phase"] = json!("Classification");
        state["session"]["view"]["presenter"]["drain_started"] = json!(false);
    }
    fresh_normal_frame_cut(&before, &owned, &expected, &actual, [true, true])?;
    for branch in 0..2 {
        actual[branch]["session"] = expected[branch]["session"].clone();
    }
    fresh_normal_frame_cut(&before, &owned, &expected, &actual, [true, true])?;
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &actual, [false, false]).is_err());
    fresh_normal_frame_cut(&before, &owned, &expected, &expected, [false, false])?;
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &expected, [true, true]).is_err());
    Ok(())
}

fn fresh_normal_frame_cut_fixture() -> Result<(
    [serde_json::Value; 2],
    [serde_json::Value; 2],
    [serde_json::Value; 2],
)> {
    let (first, order, lists) = fresh_normal_frame_fixture();
    let mut resumed = first.clone();
    resumed["device"]["frozen_pixels"] = json!([0, 0, 0, 255]);
    frozen_surface_return(&first, &resumed)?;
    let before = [first, resumed];
    let owned = [
        fresh_normal_frame_owned_state(&before[0], &json!([0, 0, 0, 255]))?,
        fresh_normal_frame_owned_state(&before[1], &json!([0, 0, 0, 255]))?,
    ];
    let expected = [
        fresh_normal_frame_publication_expected(&fresh_normal_frame_expected_state(
            &before[0], &order, &lists,
        )?)?,
        fresh_normal_frame_publication_expected(&fresh_normal_frame_expected_state(
            &before[1], &order, &lists,
        )?)?,
    ];
    Ok((before, owned, expected))
}

#[test]
fn fresh_normal_frame_surface_rejects_missing_or_corrupt_exact_pixels() -> Result<()> {
    let (before, owned, expected) = fresh_normal_frame_cut_fixture()?;
    for pixels in [
        json!(null),
        json!([]),
        json!([0, 0, 0]),
        json!([1, 0, 0, 255]),
        json!([0.0, 0, 0, 255]),
    ] {
        for paired in [false, true] {
            let mut invalid = owned.clone();
            invalid[0]["device"]["frozen_pixels"] = pixels.clone();
            if paired {
                invalid[1]["device"]["frozen_pixels"] = pixels.clone();
            }
            ensure!(
                fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [true, true]).is_err()
            );
        }
    }
    let mut invalid = owned.clone();
    for branch in &mut invalid {
        branch["device"]
            .as_object_mut()
            .unwrap()
            .remove("frozen_pixels");
    }
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [true, true]).is_err());
    ensure!(fresh_normal_frame_owned_state(&before[1], &json!([1, 0, 0, 255])).is_err());
    Ok(())
}

#[test]
fn fresh_normal_frame_surface_rejects_premature_thaw_or_publication() -> Result<()> {
    let (before, owned, expected) = fresh_normal_frame_cut_fixture()?;
    for field in ["normal_frame", "frozen_pixels"] {
        let mut invalid = owned.clone();
        for branch in &mut invalid {
            branch["device"].as_object_mut().unwrap().remove(field);
        }
        ensure!(
            fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [true, true]).is_err()
        );
        ensure!(
            fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [false, false]).is_err()
        );
    }
    let mut invalid = owned.clone();
    for branch in &mut invalid {
        branch["device"] = expected[0]["device"].clone();
        branch["session"]["view"]["presenter"]["normal_frame"] =
            json!({"tick":88444,"phase":"Classification"});
    }
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [true, true]).is_err());
    ensure!(fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [false, false]).is_err());
    Ok(())
}

#[test]
fn fresh_normal_frame_surface_rejects_generation_frame_session_and_client_writes() -> Result<()> {
    let (before, owned, expected) = fresh_normal_frame_cut_fixture()?;
    for pending in [false, true] {
        for path in [
            "/client/owner",
            "/client/offset",
            "/device/generation",
            "/device/published_generation",
            "/device/frame/input/center/0",
            "/device/published_frame/tick",
            "/session/view/cpstate",
            "/session/simulation/state/vm/tick",
            "/session/simulation/state/vm/rng_state",
            "/session/simulation/state/vm/lot_rng_state",
            "/session/simulation/state/vm/crt_rng_state",
        ] {
            let mut invalid = if pending {
                owned.clone()
            } else {
                expected.clone()
            };
            *invalid[0].pointer_mut(path).unwrap() = json!(9);
            ensure!(
                fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [pending, pending])
                    .is_err()
            );
            *invalid[1].pointer_mut(path).unwrap() = json!(9);
            ensure!(
                fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [pending, pending])
                    .is_err()
            );
        }
    }
    for path in [
        "/client/offset",
        "/device/offset",
        "/session/simulation/state/vm/world/environment",
    ] {
        let mut invalid = expected.clone();
        for branch in &mut invalid {
            *branch.pointer_mut(path).unwrap() = json!(0.0);
        }
        ensure!(
            fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [false, false]).is_err()
        );
    }
    Ok(())
}

#[test]
fn fresh_normal_frame_publication_requires_exact_generation_and_unchanged_frames() -> Result<()> {
    let (before, owned, expected) = fresh_normal_frame_cut_fixture()?;
    ensure!(expected[0]["device"]["generation"] == json!(29));
    ensure!(expected[0]["device"]["published_generation"] == json!(29));
    ensure!(exact_value_equal(
        &before[0]["device"]["frame"],
        &expected[0]["device"]["frame"]
    ));
    ensure!(exact_value_equal(
        &before[0]["device"]["published_frame"],
        &expected[0]["device"]["published_frame"]
    ));
    for value in [json!(28), json!(30), json!(29.0)] {
        for field in ["generation", "published_generation"] {
            let mut invalid = expected.clone();
            for branch in &mut invalid {
                branch["device"][field] = value.clone();
            }
            ensure!(
                fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [false, false])
                    .is_err()
            );
        }
    }
    for field in ["frozen_pixels", "normal_frame", "foreign_owner"] {
        let mut invalid = expected.clone();
        for branch in &mut invalid {
            branch["device"][field] = json!([0, 0, 0, 255]);
        }
        ensure!(
            fresh_normal_frame_cut(&before, &owned, &expected, &invalid, [false, false]).is_err()
        );
    }
    Ok(())
}

#[test]
fn fresh_normal_frame_surface_pins_entry_tick_without_retagging_prior_pixels() -> Result<()> {
    let (mut before, _, _) = fresh_normal_frame_fixture();
    before["session"]["simulation"]["state"]["vm"]["tick"] = json!(88445);
    let owned = fresh_normal_frame_owned_state(&before, &json!([11, 22, 33, 255]))?;
    ensure!(owned["device"]["normal_frame"]["entry_tick"] == json!(88445));
    for field in ["frame", "published_frame"] {
        ensure!(exact_value_equal(
            &before["device"][field],
            &owned["device"][field]
        ));
    }
    ensure!(owned["device"]["frozen_pixels"] == json!([11, 22, 33, 255]));
    ensure!(fresh_normal_frame_publication_expected(&before).is_err());
    for tick in [json!(88443), json!(88445.0), json!(null)] {
        let mut invalid = before.clone();
        invalid["session"]["simulation"]["state"]["vm"]["tick"] = tick;
        ensure!(fresh_normal_frame_owned_state(&invalid, &json!([11, 22, 33, 255])).is_err());
    }
    Ok(())
}

struct FreshLayoutEnvironment<'a> {
    vm: &'a sims_vm::Vm,
    live: &'a sims_sim::lot_environment::live::Live,
    lot: &'a sims_sim::lot_geometry::Lot,
    links: &'a sims_sim::lot_links::Links,
    room_effects: &'a [sims_vm::person_loading::RoomEffect],
    clock_events: &'a [sims_sim::clock::ClockEvent],
    period: i16,
}

fn fresh_layout_event_ownership(
    live: Option<&sims_sim::lot_environment::live::Live>,
    room_effects: Option<&[sims_vm::person_loading::RoomEffect]>,
    clock_events: Option<&[sims_sim::clock::ClockEvent]>,
) -> Result<()> {
    let live = live.context("unsupported layout prefix: room environment owner absent")?;
    let room_effects =
        room_effects.context("unsupported layout prefix: room event owner absent")?;
    let clock_events =
        clock_events.context("unsupported layout prefix: clock event owner absent")?;
    ensure!(
        live.room_effects_consumed == room_effects.len()
            && live.clock_events_consumed == clock_events.len(),
        "unsupported layout prefix: stale or unconsumed room/clock events; model their ordered services before admitting this cut"
    );
    Ok(())
}

fn fresh_layout_environment_model(
    inputs: &FreshLayoutEnvironment<'_>,
    program: &sims_vm::Program,
) -> Result<(sims_vm::Vm, sims_sim::lot_environment::live::Live)> {
    use sims_sim::lot_environment::{Tuning, manager::Manager};
    fresh_layout_event_ownership(
        Some(inputs.live),
        Some(inputs.room_effects),
        Some(inputs.clock_events),
    )?;
    let manager = &inputs.live.manager;
    let geometry = Manager::new(
        inputs.lot,
        Tuning::from_program(program).map_err(anyhow::Error::msg)?,
        manager.settings,
    )
    .map_err(anyhow::Error::msg)?;
    let refresh = inputs
        .vm
        .world
        .refresh
        .as_ref()
        .context("unsupported layout prefix: room refresh owner absent")?;
    ensure!(
        manager.size == geometry.size
            && exact_value_equal(
                &serde_json::to_value(&manager.tuning)?,
                &serde_json::to_value(&geometry.tuning)?
            )
            && manager.rooms.keys().eq(geometry.rooms.keys())
            && manager.rooms.keys().eq(refresh.rooms.keys())
            && manager.rooms.iter().all(|(id, room)| {
                room.tiles == geometry.rooms[id].tiles
                    && room.outdoors == geometry.rooms[id].outdoors
            })
            && manager.light_maps.len() == inputs.lot.floors.len()
            && manager.light_maps.iter().all(|map| map.cells.len() == 4096)
            && manager.light_bytes.len() == inputs.lot.floors.len()
            && manager
                .light_bytes
                .iter()
                .all(|map| map.len() == manager.size * manager.size),
        "unsupported layout prefix: environment geometry/tuning/refresh ownership differs"
    );
    let mut vm = inputs.vm.clone();
    let mut live = inputs.live.clone();
    live.manager
        .update(&mut vm, program, inputs.lot, inputs.links, inputs.period)
        .map_err(anyhow::Error::msg)
        .context(
            "unsupported layout prefix: native contribution/location/lighting service failed",
        )?;
    fresh_layout_event_ownership(
        Some(&live),
        Some(inputs.room_effects),
        Some(inputs.clock_events),
    )?;
    let mut unchanged_vm = serde_json::to_value(&vm)?;
    unchanged_vm["world"] = serde_json::to_value(&inputs.vm.world)?;
    ensure!(
        exact_value_equal(&serde_json::to_value(inputs.vm)?, &unchanged_vm),
        "layout environment calculation changed tick/RNG or another VM owner"
    );
    Ok((vm, live))
}

fn fresh_layout_indicator_model(
    world: &sims_vm::World,
) -> Result<sims_vm::indicator_unregister::Queues> {
    world
        .validate_indicator_queues()
        .map_err(anyhow::Error::msg)?;
    let mut queues = world.indicator_queues.clone();
    ensure!(
        sims_vm::indicator_refresh::Drain::default()
            .advance(&mut queues)
            .map_err(anyhow::Error::msg)?
            .is_none(),
        "unsupported layout prefix: indicator callback requires its own factory/notification/service model"
    );
    Ok(queues)
}

fn fresh_layout_readiness_model(
    inputs: &sims_sim::lot_presenter::Inputs,
    ready: &std::collections::BTreeMap<i16, u8>,
) -> Result<()> {
    ensure!(
        &inputs.graphic_ready == ready,
        "unsupported layout prefix: model changed graphic readiness before classification"
    );
    Ok(())
}

fn fresh_layout_classification_model(
    world: &sims_vm::World,
    program: &sims_vm::Program,
    links: &sims_sim::lot_links::Links,
    inputs: &sims_sim::lot_presenter::Inputs,
    previous: &sims_vm::presenter::Lists,
) -> Result<sims_vm::presenter::Lists> {
    use sims_sim::lot_presenter::View;
    use sims_vm::presenter::{Classification, Lists};
    world
        .validate_registry_order()
        .map_err(anyhow::Error::msg)?;
    let mut order = world
        .registry_order
        .clone()
        .context("unsupported layout prefix: native registry owner absent")?;
    ensure!(
        !order.dirty && order.rotation.is_some(),
        "unsupported layout prefix: use the dirty-registry completion oracle before composing layout"
    );
    let mut calculated_world = world.clone();
    order
        .get(
            &View::new(&mut calculated_world, program, links, inputs, &[])
                .map_err(anyhow::Error::msg)?,
        )
        .map_err(anyhow::Error::msg)?;
    let mut classification = Classification::default();
    let mut lists = Lists::default();
    let request = classification
        .advance(
            &mut View::new(
                &mut calculated_world,
                program,
                links,
                inputs,
                &order.objects,
            )
            .map_err(anyhow::Error::msg)?,
            &mut lists,
        )
        .map_err(anyhow::Error::msg)?;
    ensure!(
        request.is_none()
            && classification.complete()
            && exact_value_equal(
                &serde_json::to_value(world)?,
                &serde_json::to_value(&calculated_world)?
            )
            && world.registry_order.as_ref() == Some(&order)
            && &lists == previous,
        "unsupported layout prefix: classification needs invalidation, cache/list changes or another world write"
    );
    Ok(lists)
}

fn fresh_layout_prefix_session(
    before: &serde_json::Value,
    simulation: &impl serde::Serialize,
    lists: &sims_vm::presenter::Lists,
) -> Result<serde_json::Value> {
    let mut expected = before.clone();
    *expected
        .pointer_mut("/simulation/state")
        .context("layout prefix saved simulation absent")? = serde_json::to_value(simulation)?;
    *expected
        .pointer_mut("/view/presenter/lists")
        .context("layout prefix saved classification absent")? = serde_json::to_value(lists)?;
    Ok(expected)
}

fn fresh_layout_native_prefix_model(
    viewer: &Viewer,
    before: &serde_json::Value,
) -> Result<serde_json::Value> {
    let session = viewer
        .session
        .as_ref()
        .context("layout prefix session absent")?;
    ensure!(
        !viewer.presentation_pending()
            && !session.player_input_blocks_ticks()
            && !session.camera_command_pending()
            && !session.interface_update_pending()
            && !session.control_panel_pending()
            && !session.ui_portrait_pending()
            && !session.ui_wait_pending()
            && !session.clock_pending()
            && !session.postload_pending()
            && !session.load_completion_pending()
            && !session.house_notification_pending()
            && !session.view_refresh_pending()
            && !session.simulation.selection_work_pending()
            && !session.host.ordinary.updates_pending()
            && session.simulation.vm.scheduled_object_visit.is_none()
            && viewer
                .device
                .as_ref()
                .is_some_and(|device| !device.normal_view_frame_pending()),
        "unsupported layout prefix: finish the suspended native/renderer/service owner first"
    );
    let presenter = session
        .host
        .presentation
        .presenter
        .as_ref()
        .context("layout prefix presenter absent")?;
    presenter
        .validate_simulation(&session.simulation)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        presenter.complete(),
        "layout prefix presenter already running"
    );
    session
        .retained_camera_input()?
        .build()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        exact_value_equal(before, &serde_json::to_value(session.save_managed()?)?),
        "layout prefix before-state is not the frozen saved session"
    );
    let source = &session.simulation;
    fresh_layout_event_ownership(
        source.room_environment.as_ref(),
        source
            .native_lot
            .as_ref()
            .map(|native| native.room_effects.as_slice()),
        source
            .household
            .as_ref()
            .map(|household| household.clock_events.as_slice()),
    )?;
    let native = source.native_lot.as_ref().unwrap();
    let household = source.household.as_ref().unwrap();
    let (vm, live) = fresh_layout_environment_model(
        &FreshLayoutEnvironment {
            vm: &source.vm,
            live: source.room_environment.as_ref().unwrap(),
            lot: source
                .architecture
                .as_ref()
                .and_then(|architecture| architecture.original.as_ref())
                .context("layout prefix original geometry absent")?,
            links: &native.placement.links,
            room_effects: &native.room_effects,
            clock_events: &household.clock_events,
            period: household.clock.period,
        },
        &session.program,
    )?;
    let mut simulation = source.clone();
    simulation.vm = vm;
    simulation.room_environment = Some(live);
    let queues = fresh_layout_indicator_model(&simulation.vm.world)?;
    let mut indicator_expected = simulation.clone();
    indicator_expected.vm.world.indicator_queues = queues;
    simulation
        .begin_native_indicator_refresh(true)
        .map_err(anyhow::Error::msg)?;
    let step = simulation
        .advance_native_selection(&session.program, &BTreeSet::new(), 256)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        !simulation.selection_work_pending()
            && step.trace.is_empty()
            && step.events.is_empty()
            && exact_value_equal(
                &serde_json::to_value(&indicator_expected)?,
                &serde_json::to_value(&simulation)?
            ),
        "unsupported layout prefix: callback-free indicator completion changed another owner"
    );
    fresh_layout_readiness_model(
        &presenter.inputs,
        &session
            .host
            .ordinary
            .presenter_readiness(&simulation.vm, &session.program)?,
    )?;
    let lists = fresh_layout_classification_model(
        &simulation.vm.world,
        &session.program,
        &native.placement.links,
        &presenter.inputs,
        &presenter.lists,
    )?;
    fresh_layout_prefix_session(before, &simulation, &lists)
}

fn fresh_layout_frame(session: &serde_json::Value) -> Result<serde_json::Value> {
    let presenter = session
        .pointer("/view/presenter")
        .context("layout frame presenter absent")?;
    let input = json!({
        "viewport":presenter.pointer("/camera/settings/viewport"),
        "center":presenter.pointer("/view/origin"),
        "zoom":presenter.pointer("/view/zoom"),
        "rotation":presenter.pointer("/view/display_rotation")
    });
    let native: sims_sim::pose::portrait::view::Input = serde_json::from_value(input.clone())?;
    native.build().map_err(anyhow::Error::msg)?;
    ensure!(
        exact_value_equal(&input, &serde_json::to_value(native)?),
        "layout frame camera numbers are not canonical"
    );
    let floor: i32 = serde_json::from_value(
        presenter
            .pointer("/view/floor/level")
            .context("layout frame floor absent")?
            .clone(),
    )?;
    let tick = session
        .pointer("/simulation/state/vm/tick")
        .and_then(serde_json::Value::as_u64)
        .context("layout frame tick absent")?;
    Ok(json!({"input":input,"floor":floor,"tick":tick}))
}

fn fresh_layout_retained_expected(
    before: &serde_json::Value,
    prefix: &serde_json::Value,
    display: [u32; 2],
    target: [u32; 2],
) -> Result<serde_json::Value> {
    use sims_sim::lot_presenter::viewport::{State, scales};
    ensure!(
        before.as_object().is_some_and(|owners| {
            owners.len() == 3 && owners.contains_key("client") && owners.contains_key("device")
        }) && display.iter().all(|size| (1..=4096).contains(size))
            && display[0] == target[0]
            && target[1] <= display[1],
        "layout retained owners or display dimensions differ"
    );
    let frame = fresh_layout_frame(&before["session"])?;
    ensure!(
        exact_value_equal(&frame, &fresh_layout_frame(prefix)?),
        "layout native prefix changed the retained camera or tick"
    );
    let session = fresh_layout_expected(prefix, target)?;
    let final_frame = fresh_layout_frame(&session)?;
    let device = before["device"]
        .as_object()
        .context("layout device recipe absent")?;
    ensure!(
        !device.contains_key("normal_frame")
            && device
                .get("dirty")
                .and_then(serde_json::Value::as_bool)
                .is_some()
            && exact_value_equal(&before["device"]["frame"], &frame)
            && exact_value_equal(&before["device"]["published_frame"], &frame)
            && exact_value_equal(
                &before["device"]["generation"],
                &before["device"]["published_generation"]
            ),
        "unsupported layout device: owner={}, dirty={:?}, current_frame={}, published_frame={}, generation={:?}, published_generation={:?}",
        device.contains_key("normal_frame"),
        device.get("dirty"),
        exact_value_equal(&before["device"]["frame"], &frame),
        exact_value_equal(&before["device"]["published_frame"], &frame),
        device.get("generation"),
        device.get("published_generation")
    );
    let generation = before["device"]["generation"]
        .as_u64()
        .and_then(|value| value.checked_add(2))
        .context("layout publication generations overflow")?;
    let mut binding = before["device"]["viewport"].clone();
    let original_state = binding["state"].clone();
    let mut state: State = serde_json::from_value(original_state.clone())?;
    ensure!(
        exact_value_equal(&original_state, &serde_json::to_value(&state)?),
        "layout viewport state has unknown or noncanonical fields"
    );
    let view = state.views.get(&1).context("layout viewport view absent")?;
    let renderer = state
        .renderers
        .get(&2)
        .context("layout viewport renderer absent")?;
    let window = state
        .windows
        .get(&3)
        .context("layout viewport window absent")?;
    let surface = state
        .surfaces
        .get(&4)
        .context("layout viewport display absent")?;
    let regions = state
        .targets
        .get(&5)
        .context("layout viewport target absent")?;
    let viewport: [i32; 4] = serde_json::from_value(frame["input"]["viewport"].clone())?;
    let [left, top, width, height] = viewport;
    ensure!(
        state.views.len() == 1
            && state.renderers.len() == 1
            && state.windows.len() == 1
            && state.surfaces.len() == 1
            && state.targets.len() == 1
            && state.target == 5
            && left == 0
            && top == 0
            && width == target[0] as i32
            && (1..=4096).contains(&height)
            && view.window == 3
            && view.renderer == 2
            && view.surface == 4
            && view.rectangle == viewport
            && renderer.window == 3
            && i32::from(renderer.width) == width
            && surface[0] == width
            && surface[1] >= height
            && window.origin == [0, 0]
            && window.size == [width, height]
            && window.rectangle == viewport
            && window.scale == scales(window.size)
            && window.backend == 6
            && window.result == 6
            && exact_value_equal(&binding["bound_size"], &json!([width, height]))
            && *regions == [viewport, [0, height, width, height * 2]],
        "layout viewport ownership or initial geometry differs"
    );
    let size = target.map(|value| value as i32);
    let rectangle = [0, 0, size[0], size[1]];
    state.surfaces.insert(4, display.map(|value| value as i32));
    state.renderers.get_mut(&2).unwrap().width = display[0] as i16;
    let window = state.windows.get_mut(&3).unwrap();
    window.size = size;
    window.rectangle = rectangle;
    window.scale = scales(size);
    let view = state.views.get_mut(&1).unwrap();
    view.rectangle = rectangle;
    view.retained = if display[1] == target[1] {
        [0, display[1] as i32 - height, size[0], display[1] as i32]
    } else {
        [0; 4]
    };
    state
        .targets
        .insert(5, [rectangle, [0, size[1], size[0], size[1] * 2]]);
    binding["bound_size"] = json!(size);
    binding["state"] = serde_json::to_value(state)?;
    let mut expected = before.clone();
    expected["session"] = session;
    expected["device"]["viewport"] = binding;
    expected["device"]["frame"] = final_frame.clone();
    expected["device"]["published_frame"] = final_frame;
    expected["device"]["generation"] = json!(generation);
    expected["device"]["published_generation"] = json!(generation);
    expected["device"]["dirty"] = json!(false);
    expected["device"]
        .as_object_mut()
        .unwrap()
        .remove("frozen_pixels");
    Ok(expected)
}

fn fresh_layout_accept(expected: &serde_json::Value, actual: &serde_json::Value) -> Result<()> {
    ensure!(
        exact_value_equal(expected, actual),
        "layout changed state outside the calculated session/device completion or pinned client"
    );
    Ok(())
}

struct FreshLayoutNativeFixture {
    vm: sims_vm::Vm,
    live: sims_sim::lot_environment::live::Live,
    lot: sims_sim::lot_geometry::Lot,
    links: sims_sim::lot_links::Links,
    program: sims_vm::Program,
    room_effects: Vec<sims_vm::person_loading::RoomEffect>,
    clock_events: Vec<sims_sim::clock::ClockEvent>,
}

impl FreshLayoutNativeFixture {
    fn head_visits(&self, room: u16, object: i16) -> Result<i32> {
        let visits = self
            .live
            .manager
            .rooms
            .get(&room)
            .context("fixture counted room absent")?
            .tiles
            .iter()
            .filter(|tile| {
                self.links
                    .head(&self.vm.world, tile.map(|coordinate| coordinate as i8))
                    == Some(object)
            })
            .count();
        i32::try_from(visits).context("fixture head visit count overflow")
    }

    fn calculate(&self) -> Result<(sims_vm::Vm, sims_sim::lot_environment::live::Live)> {
        fresh_layout_environment_model(
            &FreshLayoutEnvironment {
                vm: &self.vm,
                live: &self.live,
                lot: &self.lot,
                links: &self.links,
                room_effects: &self.room_effects,
                clock_events: &self.clock_events,
                period: 0,
            },
            &self.program,
        )
    }
}

fn fresh_layout_native_fixture() -> Result<FreshLayoutNativeFixture> {
    use sims_sim::lot_environment::{
        Tuning,
        live::Live,
        manager::{Manager, Settings},
    };
    use sims_sim::lot_geometry::{FloorSource, Lot};
    use sims_vm::{
        Object, Vm, World,
        autonomy::Placement,
        refresh::{RefreshState, RoomState},
    };
    let floor = FloorSource {
        coverings: vec![1; 16],
        walls: vec![Default::default(); 16],
        object_heads: vec![0; 16],
        flags: vec![0; 16],
    };
    let lot = Lot::rebuild(
        4,
        vec![[0; 4]; 16],
        vec![255; 16],
        vec![255; 16],
        vec![floor.clone(), floor],
    )
    .map_err(anyhow::Error::msg)?;
    let program = sims_vm::Program {
        globals: "prefix-fixture".into(),
        modules: std::collections::BTreeMap::from([(
            "prefix-fixture".into(),
            sims_vm::Module {
                object_definition_words: std::collections::BTreeMap::from([(7, vec![0; 108])]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let manager = Manager::new(
        &lot,
        Tuning::from_program(&program).map_err(anyhow::Error::msg)?,
        Settings {
            daylight_multiplier: 1.0,
            light_range: 1.0,
            rooms_enabled: true,
            models: [None; 6],
        },
    )
    .map_err(anyhow::Error::msg)?;
    let room = *manager
        .rooms
        .keys()
        .find(|id| **id < 1024)
        .context("fixture lower room absent")? as i16;
    let mut world = World {
        person: Some(Default::default()),
        registry_order: Some(sims_vm::presenter::RegistryOrder {
            dirty: false,
            rotation: Some(0),
            objects: vec![41, 57],
        }),
        refresh: Some(RefreshState {
            visible_level: 1,
            dirty_categories: 3,
            rooms: manager
                .rooms
                .keys()
                .map(|id| {
                    (
                        *id,
                        RoomState {
                            category: ((id >> 10) & 3) as u8,
                            dirty: true,
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    };
    for (id, parent, slot, hidden) in [(41, 0, -1, 0), (57, 41, 1, 1)] {
        world.objects.insert(
            id,
            Object {
                definition: Some(sims_vm::Definition {
                    source: "prefix-fixture".into(),
                    object: 7,
                    guid: 701,
                    slots: 1,
                }),
                slots: if parent == 0 { vec![Some(57)] } else { vec![] },
                data: std::collections::BTreeMap::from([
                    (7, 0),
                    (14, slot),
                    (16, 0),
                    (26, parent),
                    (29, room),
                    (34, hidden),
                    (39, 0),
                    (40, 0),
                    (51, 0),
                    (59, 0),
                ]),
                ..Default::default()
            },
        );
        world.positions.insert(
            id,
            Placement {
                x: 16,
                y: 16,
                level: 1,
            },
        );
    }
    let mut fixture = FreshLayoutNativeFixture {
        vm: Vm::new(world, 19),
        live: Live {
            manager,
            room_effects_consumed: 0,
            clock_events_consumed: 0,
        },
        lot,
        links: sims_sim::lot_links::Links {
            size: 4,
            heads: vec![vec![0; 16]; 2],
            next: std::collections::BTreeMap::from([(41, 0), (57, 0)]),
        },
        program,
        room_effects: vec![],
        clock_events: vec![],
    };
    fixture.links.heads[0][5] = 41;
    let (vm, live) = fixture.calculate()?;
    fixture.vm = vm;
    fixture.live = live;
    fixture
        .vm
        .world
        .objects
        .get_mut(&57)
        .unwrap()
        .data
        .insert(34, 0);
    fixture
        .live
        .manager
        .invalidate_active_rooms(&mut fixture.vm)
        .map_err(anyhow::Error::msg)?;
    Ok(fixture)
}

fn fresh_layout_native_composition_fixture() -> Result<(serde_json::Value, serde_json::Value)> {
    let fixture = fresh_layout_native_fixture()?;
    let (vm, live) = fixture.calculate()?;
    let mut before = fresh_layout_fixture();
    before["simulation"] =
        json!({"state":{"vm":fixture.vm,"room_environment":fixture.live,"other_owner":-0.0}});
    before["view"]["presenter"]["lists"] = json!({"dynamic":[],"ordinary":[41,57],"special":[]});
    before["view"]["presenter"]["view"] = json!({
        "origin":[572,66],"zoom":1,"display_rotation":0,"floor":{"level":1}
    });
    let lists = serde_json::from_value(before["view"]["presenter"]["lists"].clone())?;
    let mut calculated = before["simulation"]["state"].clone();
    calculated["vm"] = serde_json::to_value(vm)?;
    calculated["room_environment"] = serde_json::to_value(live)?;
    let prefix = fresh_layout_prefix_session(&before, &calculated, &lists)?;
    let frame = fresh_layout_frame(&before)?;
    let retained = json!({"session":before,"client":{"owner":2,"offset":-0.0},
        "device":{"owner":7,"offset":-0.0,"dirty":false,
            "frame":frame,"published_frame":frame,"generation":28,"published_generation":28,
            "viewport":{"bound_size":[1320,900],"state":{
                "views":{"1":{"window":3,"renderer":2,"surface":4,
                    "rectangle":[0,0,1320,900],"retained":[0,0,0,0]}},
                "renderers":{"2":{"window":3,"width":1320}},
                "windows":{"3":{"origin":[0,0],"size":[1320,900],
                    "scale":sims_sim::lot_presenter::viewport::scales([1320,900]),
                    "rectangle":[0,0,1320,900],"backend":6,"result":6}},
                "surfaces":{"4":[1320,900]},"targets":{"5":[[0,0,1320,900],[0,900,1320,1800]]},
                "target":5}}}});
    let expected = fresh_layout_retained_expected(&retained, &prefix, [1320, 900], [1320, 800])?;
    Ok((retained, expected))
}

#[test]
fn fresh_layout_native_prefix_recounts_contained_objects_from_frozen_inputs() -> Result<()> {
    let fixture = fresh_layout_native_fixture()?;
    let frozen = serde_json::to_value((&fixture.vm, &fixture.live, &fixture.lot, &fixture.links))?;
    let (vm, live) = fixture.calculate()?;
    let room = fixture.vm.world.objects[&57].data[&29] as u16;
    let visits = fixture.head_visits(room, 41)?;
    ensure!(visits > 1);
    ensure!(live.manager.rooms[&room].totals.objects == 2 * visits);
    ensure!(live.manager.rooms[&room].totals.good == 2 * visits);
    ensure!(live.manager.rooms[&room].totals != fixture.live.manager.rooms[&room].totals);
    let mut missing = fresh_layout_native_fixture()?;
    missing.vm.world.objects.get_mut(&41).unwrap().slots.clear();
    let (missing_vm, missing_live) = missing.calculate()?;
    ensure!(missing_live.manager.rooms[&room].totals.objects == visits);
    ensure!(!exact_value_equal(
        &serde_json::to_value((&vm, &live))?,
        &serde_json::to_value((&missing_vm, &missing_live))?
    ));
    ensure!(exact_value_equal(
        &frozen,
        &serde_json::to_value((&fixture.vm, &fixture.live, &fixture.lot, &fixture.links))?
    ));
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_skipped_recount_and_dirty_only_cleanup() -> Result<()> {
    let (before, expected) = fresh_layout_native_composition_fixture()?;
    let mut skipped = before.clone();
    skipped["session"] = fresh_layout_expected(&before["session"], [1320, 800])?;
    ensure!(!exact_value_equal(&expected, &skipped));
    skipped["session"]["simulation"]["state"]["vm"]["world"]["refresh"] =
        expected["session"]["simulation"]["state"]["vm"]["world"]["refresh"].clone();
    ensure!(!exact_value_equal(&expected, &skipped));
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_wrong_totals_scores_lights_and_aggregate() -> Result<()> {
    let (_, expected) = fresh_layout_native_composition_fixture()?;
    let manager = "/session/simulation/state/room_environment/manager";
    let room = expected
        .pointer(&format!("{manager}/rooms"))
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    for suffix in [
        format!("rooms/{room}/totals/objects"),
        format!("rooms/{room}/totals/good"),
        format!("rooms/{room}/totals/impact"),
        format!("rooms/{room}/scores/environment"),
        format!("rooms/{room}/scores/published_light"),
        "aggregate/outdoor_sum".into(),
        "aggregate/outdoor_environment".into(),
        "aggregate/dirty_floors".into(),
        "light_bytes/0/5".into(),
        "light_maps/0/cells/0/0".into(),
    ] {
        let path = format!("{manager}/{suffix}");
        let mut actual = expected.clone();
        let value = actual
            .pointer_mut(&path)
            .context("fixture score path absent")?;
        *value = json!(value.as_i64().context("fixture score not integer")? + 1);
        ensure!(
            fresh_layout_accept(&expected, &actual).is_err(),
            "accepted {path}"
        );
    }
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_preserves_inactive_and_unselected_floor_rooms() -> Result<()> {
    let mut fixture = fresh_layout_native_fixture()?;
    let upper = *fixture
        .live
        .manager
        .rooms
        .keys()
        .find(|id| (**id >> 10) & 3 == 1)
        .context("fixture upper room absent")?;
    fixture.vm.world.refresh.as_mut().unwrap().dirty_categories = 1;
    let (_, live) = fixture.calculate()?;
    ensure!(live.manager.rooms[&upper] == fixture.live.manager.rooms[&upper]);
    let mut wrong_floor = fresh_layout_native_fixture()?;
    wrong_floor
        .vm
        .world
        .refresh
        .as_mut()
        .unwrap()
        .dirty_categories = 2;
    ensure!(!exact_value_equal(
        &serde_json::to_value(fixture.calculate()?)?,
        &serde_json::to_value(wrong_floor.calculate()?)?
    ));
    let room = fixture.vm.world.objects[&57].data[&29] as u16;
    fixture.live.manager.rooms.get_mut(&room).unwrap().active = false;
    let (vm, live) = fixture.calculate()?;
    ensure!(live.manager.rooms[&room] == fixture.live.manager.rooms[&room]);
    ensure!(vm.world.refresh.as_ref().unwrap().rooms[&room].dirty);
    let mut wrong = live.clone();
    wrong.manager.rooms.get_mut(&room).unwrap().totals.objects += 1;
    ensure!(!exact_value_equal(
        &serde_json::to_value(&live)?,
        &serde_json::to_value(&wrong)?
    ));
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_preserves_f1_redirty_after_floor_cleanup() -> Result<()> {
    let mut fixture = fresh_layout_native_fixture()?;
    let room = fixture.vm.world.objects[&57].data[&29] as u16;
    fixture
        .live
        .manager
        .rooms
        .get_mut(&room)
        .unwrap()
        .scores
        .light = 0;
    let (vm, _) = fixture.calculate()?;
    let refresh = vm.world.refresh.as_ref().unwrap();
    ensure!(refresh.dirty_categories == 0 && refresh.rooms[&room].dirty);
    let mut normalized = vm.clone();
    normalized
        .world
        .refresh
        .as_mut()
        .unwrap()
        .rooms
        .get_mut(&room)
        .unwrap()
        .dirty = false;
    ensure!(!exact_value_equal(
        &serde_json::to_value(vm)?,
        &serde_json::to_value(normalized)?
    ));
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_models_light_services_and_rejects_incomplete_geometry() -> Result<()>
{
    let mut fixture = fresh_layout_native_fixture()?;
    let room = fixture.vm.world.objects[&57].data[&29] as u16;
    let object = fixture.vm.world.objects.get_mut(&57).unwrap();
    object.data.insert(40, 0x400);
    object.data.insert(51, 10);
    object.data.insert(16, 1);
    let visits = fixture.head_visits(room, 41)?;
    ensure!((1..=6).contains(&visits));
    let (_, live) = fixture.calculate()?;
    ensure!(live.manager.rooms[&room].totals.artificial_light == 10 * visits);
    ensure!(live.manager.rooms[&room].totals.lights == visits);
    ensure!(live.manager.rooms[&room].models.len() == usize::try_from(visits)?);
    ensure!(live.manager.light_maps != fixture.live.manager.light_maps);
    fixture.vm.world.positions.remove(&57);
    ensure!(fixture.calculate().is_err());
    fixture.live.manager.light_bytes.clear();
    ensure!(fixture.calculate().is_err());
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_missing_stale_and_unconsumed_event_owners() -> Result<()> {
    let mut fixture = fresh_layout_native_fixture()?;
    fixture
        .room_effects
        .push(sims_vm::person_loading::RoomEffect::Occupied {
            room: 0,
            occupied: true,
        });
    fixture
        .clock_events
        .push(sims_sim::clock::ClockEvent::Signal {
            kind: 0xdd,
            argument: 0,
        });
    for (rooms, clock) in [(0, 1), (1, 0), (2, 1), (1, 2)] {
        fixture.live.room_effects_consumed = rooms;
        fixture.live.clock_events_consumed = clock;
        ensure!(fixture.calculate().is_err());
    }
    fixture.live.room_effects_consumed = 1;
    fixture.live.clock_events_consumed = 1;
    fixture.calculate()?;
    for (live, rooms, clock) in [
        (
            None,
            Some(fixture.room_effects.as_slice()),
            Some(fixture.clock_events.as_slice()),
        ),
        (
            Some(&fixture.live),
            None,
            Some(fixture.clock_events.as_slice()),
        ),
        (
            Some(&fixture.live),
            Some(fixture.room_effects.as_slice()),
            None,
        ),
    ] {
        ensure!(fresh_layout_event_ownership(live, rooms, clock).is_err());
    }
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_unmodeled_indicator_callbacks_and_missing_inputs()
-> Result<()> {
    let mut fixture = fresh_layout_native_fixture()?;
    ensure!(fresh_layout_indicator_model(&fixture.vm.world)?.is_empty());
    fixture.vm.world.indicator_queues.dirty = true;
    ensure!(fresh_layout_indicator_model(&fixture.vm.world)?.is_empty());
    fixture.vm.world.indicator_queues.dates.push(Some(41));
    ensure!(fresh_layout_indicator_model(&fixture.vm.world).is_err());
    fixture.vm.world.indicator_queues.dates.clear();
    fixture.vm.world.indicator_queues.stars.push(None);
    ensure!(fresh_layout_indicator_model(&fixture.vm.world).is_err());
    fixture.vm.world.indicator_queues.stars = vec![Some(99)];
    ensure!(fresh_layout_indicator_model(&fixture.vm.world).is_err());
    fixture
        .vm
        .world
        .objects
        .get_mut(&57)
        .unwrap()
        .data
        .remove(&7);
    ensure!(fixture.calculate().is_err());
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_proves_clean_classification_and_rejects_other_boundaries()
-> Result<()> {
    let fixture = fresh_layout_native_fixture()?;
    let inputs = sims_sim::lot_presenter::Inputs {
        force_dynamic: false,
        graphic_ready: std::collections::BTreeMap::from([(41, 1), (57, 1)]),
        keep_dynamic: Default::default(),
    };
    let lists = sims_vm::presenter::Lists {
        dynamic: vec![],
        ordinary: vec![41, 57],
        special: vec![],
    };
    fresh_layout_classification_model(
        &fixture.vm.world,
        &fixture.program,
        &fixture.links,
        &inputs,
        &lists,
    )?;
    let mut wrong_lists = lists.clone();
    wrong_lists.ordinary.reverse();
    ensure!(
        fresh_layout_classification_model(
            &fixture.vm.world,
            &fixture.program,
            &fixture.links,
            &inputs,
            &wrong_lists
        )
        .is_err()
    );
    let mut wrong_world = fixture.vm.world.clone();
    wrong_world.registry_order.as_mut().unwrap().dirty = true;
    ensure!(
        fresh_layout_classification_model(
            &wrong_world,
            &fixture.program,
            &fixture.links,
            &inputs,
            &lists
        )
        .is_err()
    );
    wrong_world.registry_order = None;
    ensure!(
        fresh_layout_classification_model(
            &wrong_world,
            &fixture.program,
            &fixture.links,
            &inputs,
            &lists
        )
        .is_err()
    );
    let mut missing = inputs.clone();
    missing.graphic_ready.remove(&41);
    fresh_layout_readiness_model(&inputs, &inputs.graphic_ready)?;
    ensure!(fresh_layout_readiness_model(&inputs, &missing.graphic_ready).is_err());
    ensure!(
        fresh_layout_classification_model(
            &fixture.vm.world,
            &fixture.program,
            &fixture.links,
            &missing,
            &lists
        )
        .is_err()
    );
    let mut invalidation = inputs.clone();
    invalidation.force_dynamic = true;
    ensure!(
        fresh_layout_classification_model(
            &fixture.vm.world,
            &fixture.program,
            &fixture.links,
            &invalidation,
            &lists
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_unrelated_retained_and_event_writes() -> Result<()> {
    let (_, expected) = fresh_layout_native_composition_fixture()?;
    for path in [
        "/session/view/selected",
        "/session/view/cpstate",
        "/session/simulation/state/vm/tick",
        "/session/simulation/state/vm/rng_state",
        "/session/simulation/state/vm/lot_rng_state",
        "/session/simulation/state/vm/crt_rng_state",
        "/session/simulation/state/vm/world/globals/27",
        "/session/simulation/state/room_environment/room_effects_consumed",
        "/session/simulation/state/room_environment/clock_events_consumed",
        "/session/simulation/state/room_environment/manager/ambient",
        "/client/owner",
        "/device/owner",
    ] {
        let mut actual = expected.clone();
        *actual
            .pointer_mut(path)
            .context("fixture retained path absent")? = json!(999);
        ensure!(
            fresh_layout_accept(&expected, &actual).is_err(),
            "accepted {path}"
        );
    }
    for owner in ["session", "client", "device"] {
        let mut actual = expected.clone();
        actual[owner]["foreign_owner"] = json!(1);
        ensure!(fresh_layout_accept(&expected, &actual).is_err());
    }
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_pins_client_and_predicts_two_publications() -> Result<()> {
    let (before, expected) = fresh_layout_native_composition_fixture()?;
    fresh_layout_accept(&expected, &expected)?;
    ensure!(exact_value_equal(&before["client"], &expected["client"]));
    ensure!(expected["device"]["generation"] == json!(30));
    ensure!(expected["device"]["published_generation"] == json!(30));
    ensure!(expected["device"]["frame"]["input"]["viewport"] == json!([0, 0, 1320, 800]));
    ensure!(exact_value_equal(
        &expected["device"]["frame"],
        &expected["device"]["published_frame"]
    ));
    ensure!(expected["device"]["viewport"]["bound_size"] == json!([1320, 800]));
    ensure!(expected["device"]["viewport"]["state"]["surfaces"]["4"] == json!([1320, 900]));
    ensure!(
        expected["device"]["viewport"]["state"]["views"]["1"]["retained"] == json!([0, 0, 0, 0])
    );
    let expanded =
        fresh_layout_retained_expected(&before, &before["session"], [1320, 1000], [1320, 1000])?;
    ensure!(
        expanded["device"]["viewport"]["state"]["views"]["1"]["retained"]
            == json!([0, 100, 1320, 1000])
    );
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_finishes_dirty_surface_only_by_exact_publication() -> Result<()> {
    let (before, expected) = fresh_layout_native_composition_fixture()?;
    let mut dirty = before.clone();
    dirty["device"]["dirty"] = json!(true);
    let actual_expected =
        fresh_layout_retained_expected(&dirty, &before["session"], [1320, 900], [1320, 800])?;
    ensure!(actual_expected["device"]["dirty"] == json!(false));
    ensure!(actual_expected["device"]["generation"] == json!(30));
    ensure!(exact_value_equal(
        &actual_expected["device"],
        &expected["device"]
    ));
    let mut unpublished = actual_expected.clone();
    unpublished["device"]["dirty"] = json!(true);
    ensure!(fresh_layout_accept(&actual_expected, &unpublished).is_err());
    for invalid in [json!(1), json!(null), json!("true")] {
        dirty["device"]["dirty"] = invalid;
        ensure!(
            fresh_layout_retained_expected(&dirty, &before["session"], [1320, 900], [1320, 800])
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_identical_paired_client_or_device_corruption() -> Result<()> {
    let (_, expected) = fresh_layout_native_composition_fixture()?;
    for path in [
        "/client/owner",
        "/client/offset",
        "/device/owner",
        "/device/offset",
        "/device/generation",
        "/device/published_frame/tick",
        "/device/viewport/state/windows/3/scale/1",
    ] {
        let mut corrupted = expected.clone();
        *corrupted
            .pointer_mut(path)
            .context("paired corruption path absent")? = json!(999);
        let paired = [corrupted.clone(), corrupted];
        ensure!(exact_value_equal(&paired[0], &paired[1]));
        for actual in &paired {
            ensure!(
                fresh_layout_accept(&expected, actual).is_err(),
                "accepted paired {path}"
            );
        }
    }
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_rejects_wrong_surface_or_resize_completion() -> Result<()> {
    let (before, expected) = fresh_layout_native_composition_fixture()?;
    for path in [
        "/device/generation",
        "/device/published_generation",
        "/device/frame/input/viewport/3",
        "/device/published_frame/input/viewport/3",
        "/device/viewport/bound_size/1",
        "/device/viewport/state/windows/3/size/1",
        "/device/viewport/state/windows/3/scale/1",
        "/device/viewport/state/views/1/rectangle/3",
        "/device/viewport/state/views/1/retained/0",
        "/device/viewport/state/targets/5/1/3",
        "/device/viewport/state/surfaces/4/1",
    ] {
        let mut actual = expected.clone();
        *actual
            .pointer_mut(path)
            .context("resize completion path absent")? = json!(999);
        ensure!(
            fresh_layout_accept(&expected, &actual).is_err(),
            "accepted {path}"
        );
    }
    let mut actual = expected.clone();
    actual["device"]["frozen_pixels"] = json!([0, 0, 0, 0]);
    ensure!(fresh_layout_accept(&expected, &actual).is_err());
    let mut overflow = before.clone();
    overflow["device"]["generation"] = json!(u64::MAX);
    overflow["device"]["published_generation"] = json!(u64::MAX);
    ensure!(
        fresh_layout_retained_expected(&overflow, &overflow["session"], [1320, 900], [1320, 800])
            .is_err()
    );
    let mut wrong_owner = before.clone();
    wrong_owner["device"]["viewport"]["state"]["target"] = json!(6);
    ensure!(
        fresh_layout_retained_expected(
            &wrong_owner,
            &wrong_owner["session"],
            [1320, 900],
            [1320, 800]
        )
        .is_err()
    );
    let mut wrong_prefix = before["session"].clone();
    wrong_prefix["view"]["presenter"]["view"]["zoom"] = json!(2);
    ensure!(
        fresh_layout_retained_expected(&before, &wrong_prefix, [1320, 900], [1320, 800]).is_err()
    );
    Ok(())
}

#[test]
fn fresh_layout_native_prefix_keeps_numeric_types_signed_zero_and_viewport_aliases_exact()
-> Result<()> {
    let (before, expected) = fresh_layout_native_composition_fixture()?;
    for path in [
        "/session/view/camera/viewport/3",
        "/session/view/presenter/camera/settings/viewport/3",
        "/session/view/viewport/3",
        "/session/simulation/state/vm/tick",
        "/session/simulation/state/room_environment/manager/aggregate/outdoor_sum",
    ] {
        let mut actual = expected.clone();
        let number = actual.pointer_mut(path).unwrap();
        *number = json!(number.as_f64().unwrap());
        ensure!(
            !exact_value_equal(&expected, &actual),
            "accepted float at {path}"
        );
    }
    for path in [
        "/session/simulation/state/other_owner",
        "/client/offset",
        "/device/offset",
    ] {
        let mut actual = expected.clone();
        *actual.pointer_mut(path).unwrap() = json!(0.0);
        ensure!(
            !exact_value_equal(&expected, &actual),
            "accepted signed zero at {path}"
        );
    }
    let mut actual = expected.clone();
    actual["session"]["simulation"]["state"]["room_environment"]["manager"]["tuning"]["daylight_bonus"] =
        json!(-0.0);
    ensure!(!exact_value_equal(&expected, &actual));
    for path in [
        "/view/camera/viewport/3",
        "/view/presenter/camera/settings/viewport/3",
        "/view/viewport/3",
    ] {
        let mut actual = expected.clone();
        *actual["session"].pointer_mut(path).unwrap() = json!(799);
        ensure!(!exact_value_equal(&expected, &actual));
        let mut invalid = before["session"].clone();
        *invalid.pointer_mut(path).unwrap() = json!(899);
        ensure!(fresh_layout_expected(&invalid, [1320, 800]).is_err());
    }
    ensure!(fresh_layout_expected(&before["session"], [1319, 800]).is_err());
    Ok(())
}

fn drain_fresh_pair(
    first: &mut Viewer,
    resumed: &mut Viewer,
    time: &serde_json::Value,
    restores: &mut BTreeSet<String>,
    phase: &str,
) -> Result<()> {
    use sims_runtime::original_session::{notifications::Service, publication::Progress};
    let phase_started = Instant::now();
    eprintln!("FRESH_PHASE_BEGIN phase={phase}");
    let mut last_advance = "none";
    let mut last_restore = serde_json::Value::Null;
    for slice in 0..10000 {
        if slice % 16 == 0 {
            let session = first.session.as_ref().unwrap();
            eprintln!(
                "FRESH_PHASE_PROGRESS phase={phase} slice={slice} seconds={:.3} presentation={} selection={} notification={} publication={} camera={} native_frame={} renderer_frame={} last_advance={last_advance}",
                phase_started.elapsed().as_secs_f64(),
                first.presentation_pending(),
                session.simulation.selection_work_pending(),
                session.house_notification_pending(),
                session.publication_pending(),
                session.camera_command_pending(),
                session.normal_view_frame_pending(),
                first.device.as_ref().unwrap().normal_view_frame_pending()
            );
        }
        ensure!(
            first.error.is_none() && resumed.error.is_none(),
            "fresh input failed: {:?}, {:?}",
            first.error,
            resumed.error
        );
        ensure!(
            fresh_time(first) == *time && fresh_time(resumed) == *time,
            "frontend selection/mode work advanced tick/RNG at slice {slice}"
        );
        let first_pending = first.presentation_pending();
        let resumed_pending = resumed.presentation_pending();
        let expected = retained(first)?;
        let actual = retained(resumed)?;
        if !exact_value_equal(&expected, &actual) {
            let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()?
                .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
            let pending = |viewer: &Viewer| {
                let session = viewer.session.as_ref().unwrap();
                json!({"presentation":viewer.presentation_pending(),
                    "selection":session.simulation.selection_work_pending(),
                    "notification":session.house_notification_pending(),
                    "publication":session.publication_pending(),
                    "camera":session.camera_command_pending(),
                    "player_input":session.player_input_pending(),
                    "tick_rng":fresh_time(viewer)})
            };
            if !output.join("first-expected.json").exists() {
                std::fs::write(
                    output.join("first-expected.json"),
                    serde_json::to_vec(&expected)?,
                )?;
                std::fs::write(
                    output.join("first-actual.json"),
                    serde_json::to_vec(&actual)?,
                )?;
                std::fs::write(
                    output.join("first-cut.json"),
                    serde_json::to_vec_pretty(&json!({"phase":phase,"slice":slice,
                        "comparison_stage":if first_pending || resumed_pending {
                            "pending-cut"
                        } else {"before-return-refresh"},
                        "last_advance":last_advance,"last_restore":last_restore,
                        "expected":pending(first),"actual":pending(resumed)}))?,
                )?;
            }
            ensure!(
                !first_pending && !resumed_pending,
                "fresh pending continuation differs during {phase} at slice {slice}"
            );
            frozen_surface_return(&expected, &actual)
                .with_context(|| format!("fresh return differs during {phase} at slice {slice}"))?;
        }
        ensure!(
            first_pending == resumed_pending,
            "cold presentation completion differs during {phase} at slice {slice}"
        );
        if !first_pending {
            let layout = fresh_layout_target(first)?;
            ensure!(
                layout == fresh_layout_target(resumed)?,
                "cold frontend layout target differs during {phase} at slice {slice}"
            );
            if let Some(target) = layout {
                for (branch, viewer) in [
                    ("uninterrupted", &mut *first),
                    ("cold-restored", &mut *resumed),
                ] {
                    let before = retained(viewer)?;
                    let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../..")
                        .canonicalize()?
                        .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
                    let calculation = (|| -> Result<_> {
                        let prefix = fresh_layout_native_prefix_model(viewer, &before["session"])?;
                        let expected = fresh_layout_retained_expected(
                            &before,
                            &prefix,
                            viewer.display_size,
                            target,
                        )?;
                        Ok((prefix, expected))
                    })();
                    let (prefix, expected) = match calculation {
                        Ok(calculated) => calculated,
                        Err(error) => {
                            std::fs::write(
                                output.join(format!("layout-{phase}-{slice}-{branch}-precondition-before-retained.json")),
                                serde_json::to_vec(&before)?,
                            )?;
                            std::fs::write(
                                output.join(format!(
                                    "layout-{phase}-{slice}-{branch}-precondition.json"
                                )),
                                serde_json::to_vec_pretty(
                                    &json!({"phase":phase,"slice":slice,"branch":branch,
                                    "target":target,"display":viewer.display_size,"error":format!("{error:#}")}),
                                )?,
                            )?;
                            return Err(error.context(format!("fresh layout calculation during {phase} at slice {slice} ({branch})")));
                        }
                    };
                    viewer.refresh()?;
                    let after = retained(viewer)?;
                    if !exact_value_equal(&expected, &after) {
                        for (suffix, value) in [
                            ("before-session", &before["session"]),
                            ("prefix-session", &prefix),
                            ("expected-session", &expected["session"]),
                            ("after-session", &after["session"]),
                            ("before-retained", &before),
                            ("expected-retained", &expected),
                            ("after-retained", &after),
                        ] {
                            std::fs::write(
                                output
                                    .join(format!("layout-{phase}-{slice}-{branch}-{suffix}.json")),
                                serde_json::to_vec(value)?,
                            )?;
                        }
                    }
                    fresh_layout_accept(&expected, &after).with_context(|| format!(
                        "fresh layout differs from calculated native prefix, pinned client and viewport publication during {phase} at slice {slice} ({branch})"
                    ))?;
                    ensure!(
                        !viewer.presentation_pending()
                            && fresh_layout_target(viewer)?.is_none()
                            && fresh_time(viewer) == *time,
                        "fresh layout completion left work or changed tick/RNG during {phase} at slice {slice} ({branch})"
                    );
                    std::fs::write(
                        output.join(format!("layout-{phase}-{slice}-{branch}.json")),
                        serde_json::to_vec_pretty(&json!({"phase":phase,"slice":slice,
                            "branch":branch,"display":viewer.display_size,
                            "panel_state":viewer.device.as_ref().unwrap().panel_state(),
                            "target":target,"complete_session_matches_expected":true,
                            "complete_retained_matches_expected":true,"client_pinned":true,
                            "calculated_device_publications":2,
                            "tick_rng":fresh_time(viewer),
                            "native_prefix":"detached Environment -> callback-free Indicators -> unchanged clean Classification",
                            "native_arithmetic":"shared component primitives; not independent native parity",
                            "layout_changed_paths":["/view/camera/viewport/3",
                                "/view/presenter/camera/settings/viewport/3","/view/viewport/3"]}))?,
                    )?;
                }
                ensure!(
                    exact_value_equal(&retained(first)?, &retained(resumed)?),
                    "complete layout return state differs during {phase} at slice {slice}"
                );
            }
            eprintln!("FRESH_PHASE_NATIVE_COMPLETION phase={phase} slice={slice}");
            complete_fresh_normal_frame_pair(first, resumed, time, restores, phase, slice)?;
            eprintln!("FRESH_PHASE_RETURN_REFRESH phase={phase} slice={slice}");
            for (branch, viewer) in [
                ("uninterrupted", &mut *first),
                ("cold-restored", &mut *resumed),
            ] {
                let before =
                    serde_json::to_value(viewer.session.as_ref().unwrap().save_managed()?)?;
                viewer.refresh()?;
                let after = serde_json::to_value(viewer.session.as_ref().unwrap().save_managed()?)?;
                if !exact_value_equal(&before, &after) {
                    let output = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../..")
                        .canonicalize()?
                        .join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
                    std::fs::write(
                        output.join(format!(
                            "refresh-{phase}-{slice}-{branch}-before-session.json"
                        )),
                        serde_json::to_vec(&before)?,
                    )?;
                    std::fs::write(
                        output.join(format!(
                            "refresh-{phase}-{slice}-{branch}-after-session.json"
                        )),
                        serde_json::to_vec(&after)?,
                    )?;
                    std::fs::write(
                        output.join(format!("refresh-{phase}-{slice}-{branch}-cut.json")),
                        serde_json::to_vec_pretty(&json!({"phase":phase,"slice":slice,
                            "branch":branch,"last_advance":last_advance,
                            "last_restore":last_restore,"tick_rng":fresh_time(viewer),
                            "initial_normal_frame":phase == "fresh-initial"}))?,
                    )?;
                    ensure!(
                        phase == "fresh-initial",
                        "fresh return frame processed session state during {phase} at slice {slice} ({branch})"
                    );
                }
            }
            ensure!(
                fresh_time(first) == *time && fresh_time(resumed) == *time,
                "return frame advanced tick/RNG during {phase} at slice {slice}"
            );
            ensure!(
                exact_value_equal(&retained(first)?, &retained(resumed)?),
                "complete return-frame state differs during {phase} at slice {slice}"
            );
            eprintln!(
                "FRESH_PHASE_COMPLETE phase={phase} slice={slice} seconds={:.3}",
                phase_started.elapsed().as_secs_f64()
            );
            return Ok(());
        }
        let s = resumed.session.as_ref().unwrap();
        let mut labels = Vec::new();
        if s.simulation.selection_work_pending() {
            labels.push("selection".to_string());
        }
        if let Some(notice) = s.host.presentation.notifications.as_ref() {
            if s.house_notification_pending()
                && notice
                    .dispatch
                    .root_message()
                    .is_some_and(|(event, _)| event == sims_vm::selection::SIGNAL)
            {
                labels.push("notification:E4".to_string());
            }
            if matches!(
                notice.pending(),
                Some(Service::Camera(
                    sims_sim::lot_presenter::camera::Service::Floor(_)
                ))
            ) {
                labels.push("notification:Floor".to_string());
            }
        }
        if let Some(property) = s.host.presentation.property_refresh.as_ref() {
            if property.pending() {
                labels.push("property".to_string());
            }
            if property.panel_owner().is_some() {
                labels.push(format!(
                    "property-panel:{:?}",
                    resumed
                        .device
                        .as_ref()
                        .unwrap()
                        .panel_widgets()
                        .unwrap()
                        .boundary()
                ));
            }
        }
        if s.publication_pending() {
            labels.push("publication".to_string());
        }
        if s.camera_command_pending() {
            labels.push("camera".to_string());
        }
        let mut restore = false;
        for label in &labels {
            restore |= restores.insert(label.clone());
        }
        if restore {
            cold(resumed)?;
            last_restore = json!({"slice":slice,"labels":labels});
        }
        for viewer in [&mut *first, &mut *resumed] {
            let s = viewer.session.as_mut().unwrap();
            let device = viewer.device.as_mut().unwrap();
            if s.publication_pending() {
                last_advance = "publication";
                match device.advance_publication(s, 1)? {
                    Progress::Complete | Progress::Pending => (),
                    Progress::Panels(_)
                        if device
                            .publish_replacement_people(s, viewer.client.as_mut().unwrap())? =>
                    {
                        ()
                    }
                    other => anyhow::bail!("fresh publication needs {other:?}"),
                }
            } else if s.house_notification_pending() {
                last_advance = "notification";
                device.advance_notification_with_client(s, viewer.client.as_mut().unwrap(), 1)?;
            } else if s.simulation.selection_work_pending() {
                last_advance = "selection";
                s.advance_selection(s.retained_camera_input()?, 1)?;
            } else if s.camera_command_pending() {
                last_advance = "camera";
                device.advance(s, 1)?;
            } else {
                last_advance = "production-step";
                viewer.step_with_clock(false)?;
            }
        }
    }
    anyhow::bail!("fresh selection/property/publication exceeded slice budget")
}

#[test]
#[ignore = "private original Goth assets; fresh production frontend, no gameplay ticks"]
fn fresh_goth_native_selection_property_publication_survives_buy_live_and_cold_restore()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(fresh_goth)?
        .join()
        .expect("fresh Goth property test panicked")
}

fn fresh_goth() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_FRESH_PROPERTY_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let root = repo.join("game-data/cabinet-extracted");
    let make = || -> Result<Viewer> {
        let args = Args::parse_from([
            "sims-room",
            "--original",
            "UserData/Houses/House05.iff",
            "--buy-preview",
            "--inferred-routing",
            "--paused",
            "--root",
            root.to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let (mut viewer, _) = Viewer::new(args);
        viewer.install(ready)?;
        let s = viewer.session.as_ref().unwrap();
        ensure!(s.host.presentation.person_selection_override == Some(0));
        let binding = s
            .host
            .presentation
            .property_refresh
            .as_ref()
            .context("production fresh setup omitted property refresh")?
            .binding;
        ensure!(binding.view == 30000 && binding.renderer == 30001);
        let registry = &s
            .host
            .presentation
            .notifications
            .as_ref()
            .unwrap()
            .dispatch
            .registry;
        ensure!(registry.nodes[&8].handler == sims_vm::commander::house::Receiver::Interface);
        ensure!(
            registry
                .nodes
                .values()
                .filter(|node| node.handler == sims_vm::commander::house::Receiver::Interface)
                .count()
                == 1
        );
        Ok(viewer)
    };
    let (mut first, mut resumed) = (make()?, make()?);
    let time = fresh_time(&first);
    let mut restores = BTreeSet::new();
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "fresh-initial",
    )?;
    let residents = first
        .session
        .as_ref()
        .unwrap()
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .map(|resident| resident.object as i16)
        .collect::<Vec<_>>();
    ensure!(
        residents.len() > 1,
        "Goth household did not provide multiple residents"
    );
    let mut observations = Vec::new();
    let native_before = first
        .session
        .as_ref()
        .unwrap()
        .simulation
        .native_selected_person()
        .map_err(anyhow::Error::msg)?;
    let different_person = ui_publication::Published::capture(&first)
        .residents
        .iter()
        .find(|resident| {
            resident.enabled
                && residents.contains(&resident.object)
                && Some(resident.object) != native_before
        })
        .context("fresh Goth has no enabled different resident")?
        .object;
    for viewer in [&mut first, &mut resumed] {
        let publication = ui_publication::Published::capture(viewer);
        ensure!(
            publication.panel_state == Some(1) && !publication.people_visible,
            "fresh resident strip is not actually collapsed"
        );
        ensure!(
            viewer
                .session
                .as_ref()
                .unwrap()
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == native_before,
            "fresh branches disagree on native selection before hidden input"
        );
        let before = retained(viewer)?;
        let before_viewer = (
            viewer.resident,
            viewer.selected,
            viewer.error.clone(),
            viewer.menu_error.clone(),
            viewer.presentation_pending(),
        );
        let _ = viewer.update(Message::Resident(different_person));
        ensure!(
            exact_value_equal(&before, &retained(viewer)?)
                && before_viewer
                    == (
                        viewer.resident,
                        viewer.selected,
                        viewer.error.clone(),
                        viewer.menu_error.clone(),
                        viewer.presentation_pending(),
                    )
                && fresh_time(viewer) == time,
            "hidden resident input changed retained state, viewer ownership or tick/RNG"
        );
    }
    observations.push(
        json!({"phase":"hidden-resident-rejected", "target":different_person,
        "native_before":native_before, "full_retained_unchanged":true,
        "viewer_ownership_unchanged":true, "tick_rng_unchanged":true}),
    );
    for viewer in [&mut first, &mut resumed] {
        let publication = ui_publication::Published::capture(viewer);
        ensure!(
            viewer
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .mode
                == 2
                && publication.error.is_none()
                && !publication.pending
                && !publication.modal
                && publication
                    .button(sims_vm::interface::view_control::Widget::Live)
                    .is_some_and(|button| button.enabled),
            "real enabled Live disclosure is unavailable at the fresh boundary"
        );
        let _ = viewer.update(Message::Live);
    }
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "live-disclosure",
    )?;
    for viewer in [&mut first, &mut resumed] {
        let publication = ui_publication::Published::capture(viewer);
        ensure!(
            publication.panel_state == Some(0)
                && publication.root_visible
                && publication.people_visible
                && publication
                    .residents
                    .iter()
                    .any(|resident| resident.object == different_person && resident.enabled),
            "Live disclosure did not expose the enabled different resident"
        );
        let _ = viewer.update(Message::Resident(different_person));
    }
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "disclosed-different-resident",
    )?;
    observations.push(json!({"phase":"disclosed-different-resident",
        "state":fresh_selection(&first, Some(different_person))?}));
    fresh_selection(&resumed, Some(different_person))?;
    for &person in &residents {
        for viewer in [&mut first, &mut resumed] {
            let _ = viewer.update(Message::Resident(person));
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("resident-{person}"),
        )?;
        observations.push(
            json!({"phase":"resident-handler", "state":fresh_selection(&first, Some(person))
                .with_context(|| format!("resident-handler-{person}"))?}),
        );
        fresh_selection(&resumed, Some(person))
            .with_context(|| format!("resident-handler-{person}-cold"))?;
    }
    let person = *residents.last().unwrap();
    for target in [None, Some(person)] {
        for viewer in [&mut first, &mut resumed] {
            viewer.session.as_mut().unwrap().begin_selection(target)?;
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("direct-selection-{target:?}"),
        )?;
        fresh_selection(&first, target).with_context(|| format!("direct-selection-{target:?}"))?;
        for viewer in [&mut first, &mut resumed] {
            viewer.session.as_mut().unwrap().republish_interface()?;
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("republication-{target:?}"),
        )?;
        observations.push(
            json!({"phase":"budgeted-republication", "state":fresh_selection(&first, target)
                .with_context(|| format!("republication-{target:?}"))?}),
        );
    }
    for floor in [2, 1] {
        for viewer in [&mut first, &mut resumed] {
            let _ = viewer.update(Message::Floor(floor));
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("floor-{floor}"),
        )?;
        ensure!(first.device.as_ref().unwrap().controls().floor == floor);
        observations.push(
            json!({"phase":format!("floor-{floor}"), "state":fresh_selection(&first, Some(person))
            .with_context(|| format!("floor-{floor}"))?}),
        );
    }
    for (message, mode) in [(Message::Buy, 1), (Message::Live, 2)] {
        for viewer in [&mut first, &mut resumed] {
            let _ = viewer.update(message.clone());
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("mode-{mode}"),
        )?;
        ensure!(
            first
                .session
                .as_ref()
                .unwrap()
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .mode
                == mode
        );
        observations.push(
            json!({"phase":format!("mode-{mode}"), "state":fresh_selection(&first, Some(person))
                .with_context(|| format!("mode-{mode}"))?}),
        );
    }
    for &person in &residents {
        for viewer in [&mut first, &mut resumed] {
            let _ = viewer.update(Message::Resident(person));
        }
        drain_fresh_pair(
            &mut first,
            &mut resumed,
            &time,
            &mut restores,
            &format!("post-live-{person}"),
        )?;
        observations.push(json!({"phase":"post-live-reselection", "state":fresh_selection(&first, Some(person))?}));
    }
    for viewer in [&mut first, &mut resumed] {
        viewer.session.as_mut().unwrap().begin_selection(None)?;
    }
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "deselection",
    )?;
    for viewer in [&mut first, &mut resumed] {
        viewer.session.as_mut().unwrap().republish_interface()?;
    }
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "deselected-republication",
    )?;
    observations
        .push(json!({"phase":"genuine-deselection", "state":fresh_selection(&first, None)?}));
    for viewer in [&mut first, &mut resumed] {
        fresh_selection(viewer, None)?;
        let before = retained(viewer)?;
        let s = viewer.session.as_mut().unwrap();
        let people = &s.simulation.vm.world.person.as_ref().unwrap().person_order;
        let object = s
            .simulation
            .vm
            .world
            .objects
            .iter()
            .find(|(id, object)| !people.contains(id) && object.definition.is_some())
            .map(|(&id, _)| id)
            .context("fresh Goth object absent")?;
        ensure!(!s.begin_object_input(object)? && !s.begin_ground_input([440, 360], 1)?);
        ensure!(!s.player_input_pending() && !s.running_menu());
        ensure!(
            exact_value_equal(&before, &retained(viewer)?),
            "actor-less input changed retained state"
        );
        let _ = viewer.update(Message::Resident(person));
    }
    drain_fresh_pair(
        &mut first,
        &mut resumed,
        &time,
        &mut restores,
        "final-reselection",
    )?;
    fresh_selection(&first, Some(person))?;
    ensure!(
        restores.contains("selection")
            && restores.contains("notification:E4")
            && restores.contains("property")
            && restores.contains("publication")
            && restores.contains("camera")
            && restores
                .iter()
                .any(|label| label.starts_with("property-panel:")),
        "missing native selection/property/publication cold boundary: {restores:?}"
    );
    std::fs::write(
        output.join("session.json"),
        serde_json::to_vec(&retained(&first)?)?,
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "house":"UserData/Houses/House05.iff", "observations":observations,
            "cold_boundaries":restores, "tick_rng":time, "full_retained_equality":true,
            "no_selected_person_no_popup":true, "gameplay_ticks_added":0,
            "strict_original_parity_ticks_added":0,
            "profile":"fresh production buy-preview binding; native policy 0; no synthetic flags or actor fallback"
        }))?,
    )?;
    Ok(())
}
