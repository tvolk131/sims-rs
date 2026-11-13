//! Actual saved camera/floor/wall composition with explicit headless device
//! sinks. This does not claim an Iced or Windows graphics backend.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session, presenter::Progress};
use sims_sim::lot_presenter::{
    Inputs, camera, hover::History, lifecycle, motion, open, transition, visibility,
    walls::cutaway::Mask,
};
use sims_vm::presenter::Order;
use std::collections::BTreeSet;

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(2880, 1440);
    let mut report = vec![];
    for (column, (name, source)) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut initial = Session::load(&index, source, Options::default())?;
        let level = if name == "goth" { 2 } else { 1 };
        // Explicit client view inputs, not inferred suspended original state.
        initial
            .simulation
            .vm
            .world
            .refresh
            .as_mut()
            .unwrap()
            .visible_level = level;
        let person = initial.house.household.as_ref().unwrap().residents[0].object as i16;
        let portrait = sims_sim::pose::portrait::view::Input {
            viewport: [0, 0, 1000, 740],
            center: [500, 370],
            zoom: 2,
            rotation: 0,
        };
        initial.begin_selection(Some(person))?;
        while !initial.advance_selection(portrait, 10000)? {}
        let initial = initial.save();
        let mut completed = vec![];
        let mut runs = vec![];
        for budget in [1, 10000] {
            let mut session = Session::restore(&index, initial.clone())?;
            let (view, inputs, _order, retained) = context(&session, portrait, level)?;
            session.begin_registry_view_refresh_with_camera(view, inputs, retained)?;
            let before = (
                session.simulation.vm.tick,
                session.simulation.vm.rng_state,
                session.simulation.vm.lot_rng_state,
                session.simulation.vm.crt_rng_state,
            );
            let mut services = vec![];
            let mut restored = BTreeSet::new();
            let mut done = false;
            for slice in 0..20000 {
                let owner = session.host.presentation.presenter.as_ref().unwrap();
                let value = serde_json::to_value(owner)?;
                // Exercise serialization of every nested operation, including
                // interior work-budget boundaries, without loading assets anew.
                let bytes = serde_json::to_vec(owner)?;
                let decoded: sims_runtime::original_session::presenter::State =
                    serde_json::from_slice(&bytes)?;
                ensure!(&decoded == owner);
                decoded
                    .validate_simulation(&session.simulation)
                    .map_err(anyhow::Error::msg)?;
                if !value["camera_call"].is_null() {
                    let mut orphan = value.clone();
                    orphan["owner"]["request"] = serde_json::Value::Null;
                    ensure!(
                        serde_json::from_value::<sims_runtime::original_session::presenter::State>(
                            orphan
                        )?
                        .validate()
                        .is_err()
                    );
                }
                let p = sims_sim::pose::portrait::view::Input {
                    center: owner.view.origin,
                    zoom: owner.view.zoom,
                    rotation: owner.view.display_rotation,
                    ..portrait
                };
                match session.advance_view_refresh(p, budget)? {
                    Progress::Pending => (),
                    Progress::Complete => {
                        done = true;
                        break;
                    }
                    Progress::Service(request) => {
                        ensure!(
                            request == open::Request::InvalidateView,
                            "camera work escaped retained owner"
                        );
                        let view = session
                            .host
                            .presentation
                            .presenter
                            .as_ref()
                            .unwrap()
                            .view
                            .clone();
                        services.push(serde_json::json!({"outer":request}));
                        session.acknowledge_view_refresh(&request, view)?;
                    }
                    Progress::CameraService(service) => {
                        // Three different nested operations get full fresh-
                        // program restores; every other slice roundtrips above.
                        let key = match &service {
                            camera::Service::Scroll(_) => Some("scroll"),
                            camera::Service::Cancel(motion::cancel::Request::ResolveSurface) => {
                                Some("cancel")
                            }
                            camera::Service::Floor(lifecycle::Request::ResolveSurface) => {
                                Some("floor")
                            }
                            _ => None,
                        };
                        if budget == 1 && key.is_some_and(|k| restored.insert(k)) {
                            let saved = session.save();
                            ensure!(session.step().is_err());
                            ensure!(
                                serde_json::to_vec(&session.save())? == serde_json::to_vec(&saved)?
                            );
                            session = Session::restore(
                                &index,
                                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                            )
                            .with_context(|| format!("{name} camera restore at slice {slice}"))?;
                            ensure!(
                                serde_json::to_vec(&session.save())? == serde_json::to_vec(&saved)?
                            );
                            ensure!(
                                session.advance_view_refresh(p, budget)?
                                    == Progress::CameraService(service.clone())
                            );
                        }
                        let owner = session.host.presentation.presenter.as_ref().unwrap();
                        services.push(serde_json::json!({"service":service,"origin":owner.view.origin,"level":owner.view.floor.level,"rotation":owner.camera.as_ref().unwrap().settings.world_rotation}));
                        let request = owner.request().unwrap().clone();
                        let view = owner.view.clone();
                        ensure!(
                            session.acknowledge_view_refresh(&request, view).is_err(),
                            "camera caller bypassed"
                        );
                        // Explicit device/UI service sink. Camera math, floor
                        // masks, graph rotation/state and terrain run in Rust.
                        let reply = match &service {
                            camera::Service::Transition(transition::Request::ResolveSurface)
                            | camera::Service::Floor(lifecycle::Request::ResolveSurface)
                            | camera::Service::Cancel(motion::cancel::Request::ResolveSurface) => {
                                camera::Reply::Token(Some(7))
                            }
                            camera::Service::Transition(transition::Request::ResolveUi) => {
                                camera::Reply::Token(None)
                            }
                            camera::Service::Transition(
                                transition::Request::InitializeLookup { .. }
                                | transition::Request::SynchronizeRenderer
                                | transition::Request::RefreshDrawables
                                | transition::Request::EdgeSurfaces
                                | transition::Request::InvalidateView
                                | transition::Request::RefreshUi
                                | transition::Request::FinishRotation
                                | transition::Request::DepthTable { .. }
                                | transition::Request::Broadcast { .. }
                                | transition::Request::FlushSurface { .. },
                            )
                            | camera::Service::Floor(
                                lifecycle::Request::InvalidateView
                                | lifecycle::Request::FlushSurface { .. },
                            )
                            | camera::Service::Cancel(
                                motion::cancel::Request::Invalidate
                                | motion::cancel::Request::FlushSurface { .. },
                            )
                            | camera::Service::Scroll(_)
                            | camera::Service::InvalidateTile(_)
                            | camera::Service::Center(
                                sims_sim::lot_presenter::centering::Request::RefreshFloorUi {
                                    ..
                                },
                            ) => camera::Reply::Done,
                            other => anyhow::bail!("internal camera operation escaped: {other:?}"),
                        };
                        session.acknowledge_view_camera_service(&service, reply)?;
                    }
                }
            }
            ensure!(done && !session.view_refresh_pending());
            ensure!(
                before
                    == (
                        session.simulation.vm.tick,
                        session.simulation.vm.rng_state,
                        session.simulation.vm.lot_rng_state,
                        session.simulation.vm.crt_rng_state
                    ),
                "refresh changed clock/RNG"
            );
            if budget == 1 {
                ensure!(restored.len() == 3, "nested coverage {restored:?}");
            }
            runs.push(serde_json::json!({"budget":budget,"full_restores":restored.len(),"services":services,"ticks_rng_unchanged":true}));
            completed.push(session.save());
        }
        ensure!(runs[0]["services"] == runs[1]["services"]);
        ensure!(
            serde_json::to_vec(&completed[0])? == serde_json::to_vec(&completed[1])?,
            "{name} camera resume/budget changed state"
        );
        let mut a = Session::restore(&index, completed.remove(0))?;
        let mut b = Session::restore(&index, completed.remove(0))?;
        for _ in 0..10 {
            ensure!(a.step()? == sims_runtime::original_session::Progress::Tick);
            ensure!(b.step()? == sims_runtime::original_session::Progress::Tick);
        }
        ensure!(serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?);
        let mut ra = sims_render::original_house::OriginalHouse::default();
        let mut rb = sims_render::original_house::OriginalHouse::default();
        for rotation in 0..2 {
            let level = if name == "goth" && rotation == 1 {
                2
            } else {
                1
            };
            let c = sims_render::original_house::OriginalHouse::camera(
                &a, 960, 720, rotation, 0.55, level,
            );
            let pixels = sims_render::software::render(&ra.scene(&a, c)?);
            ensure!(pixels == sims_render::software::render(&rb.scene(&b, c)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        report.push(serde_json::json!({"house":name,"runs":runs,"continued_ticks":10,"restored_pixel_views":2,"state_equal":true,"device_services":"explicit headless sinks; no actual renderer/backend or full broadcast"}));
        println!(
            "{name}: composed camera/floor/wall owner, three full restores, ten ticks and two restored views match"
        );
    }
    gallery.save(root.join("cache/original-camera-composition-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-camera-composition-houses.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Explicit client inputs shared by the standalone viewer and full broadcast
/// regression. These are not imported suspended original camera settings.
pub fn context(
    session: &Session,
    portrait: sims_sim::pose::portrait::view::Input,
    level: i32,
) -> Result<(open::State, Inputs, Order, camera::State)> {
    let view = open::State {
        origin: [500, 370],
        pending: [13, -27],
        zoom: 2,
        display_rotation: 0,
        widths: [64, 32, 32, 16],
        floor: lifecycle::State {
            level,
            all_walls: 0,
            history: History {
                capacity: 3,
                rooms: vec![11, 12, 13],
            },
            visibility: visibility::State {
                enabled: 1,
                dirty: 0,
                include_visible: 1,
                mask: Mask {
                    first_row: 0,
                    rows: vec![u64::MAX; 64],
                },
            },
        },
    };
    let retained = camera::State::from_lot(
        session
            .simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap(),
        &session
            .simulation
            .room_environment
            .as_ref()
            .unwrap()
            .manager,
        &view,
        camera::Settings {
            roof: None,
            clock: 123,
            application_mode: 2,
            viewport: portrait.viewport.map(|n| n as i32),
            bounds: [0, 63, 0, 63],
            world_rotation: 0,
            camera_mode: 1,
            moving: 1,
            projection_ready: 1,
            renderer_ready: true,
            graphics_enabled: true,
            ui_present: false,
            skip_edges: 0,
            depth_scales: [0, 1f32.to_bits(), 2f32.to_bits(), 4f32.to_bits()],
            floor_heights: vec![23, 58, 116, 232],
            deferred: 1,
            locked: 0,
            attempts: 0,
            bottom_margin: 0,
            motion: motion::State {
                x: [400f32.to_bits(), 0, 0, 0],
                y: [200f32.to_bits(), 0, 0, 0],
                active: 1,
                policy: 1,
                duration: 17,
                speed_bits: 1f32.to_bits(),
                rate_bits: 1f32.to_bits(),
            },
            duration_input: 17,
            application_value_bits: 2f32.to_bits(),
            saved_value_bits: 3f32.to_bits(),
        },
    )
    .map_err(anyhow::Error::msg)?;
    let inputs = Inputs {
        force_dynamic: false,
        graphic_ready: session
            .host
            .ordinary
            .presenter_readiness(&session.simulation.vm, &session.program)?,
        keep_dynamic: session
            .simulation
            .vm
            .world
            .objects
            .keys()
            .map(|&id| (id, 0))
            .collect(),
    };
    Ok((
        view,
        inputs,
        Order {
            dirty: true,
            rotation: 0,
            objects: vec![],
        },
        retained,
    ))
}
