//! Live40760d/F6 with actual saved viewer/camera and nested F9/84/E4.
//! Device/UI requests have explicit test sinks; startup registration is supplied.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    notifications::{Progress, Reply, Service, State},
};
use sims_sim::lot_presenter::{camera, centering, lifecycle, motion, open, transition};
use sims_vm::commander::{
    Registry,
    house::{Fields, Interface, Receiver, Request},
};
use std::collections::BTreeSet;

pub(super) fn registry() -> Result<Registry<Receiver>> {
    let mut r = Registry::default();
    // Supplied normal-order lead, not proof of a full running native startup.
    for (i, handler) in [
        Receiver::Interface,
        Receiver::Module,
        Receiver::World,
        Receiver::Lot,
        Receiver::Globals,
        Receiver::Objects,
        Receiver::Viewer,
        Receiver::Renderer,
    ]
    .into_iter()
    .enumerate()
    {
        let id = i as u32 + 1;
        r.construct(id).map_err(anyhow::Error::msg)?;
        r.bind(id, handler).map_err(anyhow::Error::msg)?;
    }
    Ok(r)
}
pub(super) fn fields(include_visible: u8) -> Fields {
    Fields {
        viewer_interface: true,
        include_visible,
        renderer_dirty: 0x84,
        ui: Interface {
            mask_owner_present: true,
            panel_active: true,
            shared_present: true,
            gated: 0,
            creation_gated: 0,
            dirty: 0x86,
        },
    }
}
pub(super) fn device_reply(service: &camera::Service) -> Result<camera::Reply> {
    Ok(match service {
        camera::Service::Transition(transition::Request::ResolveSurface)
        | camera::Service::Floor(lifecycle::Request::ResolveSurface)
        | camera::Service::Cancel(motion::cancel::Request::ResolveSurface) => {
            camera::Reply::Token(Some(7))
        }
        camera::Service::Transition(transition::Request::ResolveUi) => camera::Reply::Token(None),
        camera::Service::Transition(
            transition::Request::InitializeLookup { .. }
            | transition::Request::SynchronizeRenderer
            | transition::Request::RefreshDrawables
            | transition::Request::EdgeSurfaces
            | transition::Request::InvalidateView
            | transition::Request::RefreshUi
            | transition::Request::FinishRotation
            | transition::Request::DepthTable { .. }
            | transition::Request::FlushSurface { .. },
        )
        | camera::Service::Floor(
            lifecycle::Request::InvalidateView | lifecycle::Request::FlushSurface { .. },
        )
        | camera::Service::Cancel(
            motion::cancel::Request::Invalidate | motion::cancel::Request::FlushSurface { .. },
        )
        | camera::Service::Scroll(_)
        | camera::Service::InvalidateTile(_)
        | camera::Service::Center(centering::Request::RefreshFloorUi { .. }) => camera::Reply::Done,
        other => anyhow::bail!("internal camera operation escaped dispatcher: {other:?}"),
    })
}

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(2880, 1440);
    let mut reports = vec![];
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
        initial
            .simulation
            .vm
            .world
            .refresh
            .as_mut()
            .unwrap()
            .visible_level = level;
        let portrait = sims_sim::pose::portrait::view::Input {
            viewport: [0, 0, 1000, 740],
            center: [500, 370],
            zoom: 2,
            rotation: 0,
        };
        let person = initial.house.household.as_ref().unwrap().residents[0].object as i16;
        let target = (name == "newbie").then_some(person);
        initial.begin_selection(target)?;
        while !initial.advance_selection(portrait, 10000)? {}
        initial.simulation.vm.world.globals.insert(2, 1);
        if target.is_none() {
            initial.simulation.vm.world.globals.insert(3, -123);
        }
        initial.simulation.vm.world.globals.insert(26, 0);
        let initial = initial.save();
        let mut completed = vec![];
        let mut runs = vec![];
        for budget in [1, 10000] {
            let mut s = Session::restore(&index, initial.clone())?;
            let (mut view, inputs, _order, retained) =
                super::camera_session::context(&s, portrait, level)?;
            view.floor.history.clear();
            let fields = fields(view.floor.visibility.include_visible);
            s.configure_registry_house_view(view, inputs, retained)?;
            let before = (
                s.simulation.vm.tick,
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
                s.simulation.vm.crt_rng_state,
            );
            s.begin_house_notification(registry()?, fields, 0xf6, 0)?;
            let mut services = vec![];
            let mut restored = BTreeSet::new();
            let mut done = false;
            for slice in 0..25000 {
                let owner = s.host.presentation.notifications.as_ref().unwrap();
                let bytes = serde_json::to_vec(owner)?;
                let decoded: State = serde_json::from_slice(&bytes)?;
                ensure!(&decoded == owner);
                decoded.validate(&s.host.presentation, &s.simulation)?;
                let p = s.host.presentation.presenter.as_ref().unwrap();
                let c = if budget == 1 {
                    sims_sim::pose::portrait::view::Input {
                        center: p.view.origin,
                        zoom: p.view.zoom,
                        rotation: p.view.display_rotation,
                        ..portrait
                    }
                } else {
                    // A stale client camera cannot change the engine's current
                    // projection for callbacks after F6's camera transitions.
                    sims_sim::pose::portrait::view::Input {
                        center: [-9999, 7777],
                        zoom: 3,
                        rotation: 3,
                        ..portrait
                    }
                };
                match s.advance_house_notification(c, budget)? {
                    Progress::Pending => (),
                    Progress::Complete(n) => {
                        ensure!(n == 1);
                        done = true;
                        break;
                    }
                    Progress::Service(service) => {
                        let owner = s.host.presentation.notifications.as_ref().unwrap();
                        let before_owner = serde_json::to_vec(owner)?;
                        let before_globals = s.simulation.vm.world.globals.clone();
                        ensure!(
                            s.advance_house_notification(c, budget)?
                                == Progress::Service(service.clone())
                        );
                        ensure!(
                            serde_json::to_vec(
                                s.host.presentation.notifications.as_ref().unwrap()
                            )? == before_owner
                        );
                        ensure!(s.simulation.vm.world.globals == before_globals);
                        let (key, reply) = match &service {
                            Service::View(request) => {
                                ensure!(*request == open::Request::InvalidateView);
                                let saved = serde_json::to_vec(&s.save())?;
                                ensure!(s.step().is_err());
                                ensure!(s.begin_selection(Some(person)).is_err());
                                ensure!(s.begin_indicator_refresh().is_err());
                                ensure!(s.advance_selection(c, 1).is_err());
                                ensure!(s.service_selection_assets(c, 1).is_err());
                                ensure!(s.service_removal_graphic(c).is_err());
                                ensure!(s.advance_view_refresh(c, 1).is_err());
                                let v =
                                    s.host.presentation.presenter.as_ref().unwrap().view.clone();
                                ensure!(s.acknowledge_view_refresh(request, v.clone()).is_err());
                                ensure!(
                                    s.acknowledge_house_notification(&service, Reply::Done)
                                        .is_err()
                                );
                                ensure!(
                                    serde_json::to_vec(&s.save())? == saved,
                                    "bypass modified session"
                                );
                                (None, Reply::View(v))
                            }
                            Service::Camera(request) => {
                                ensure!(
                                    s.acknowledge_view_camera_service(request, camera::Reply::Done)
                                        .is_err()
                                );
                                (None, Reply::Camera(device_reply(request)?))
                            }
                            Service::Listener(call) => {
                                let key = match call.request {
                                    Request::ViewerZoom { value } => {
                                        ensure!(
                                            value == 2 && s.simulation.vm.world.globals[&2] == 1
                                        );
                                        ensure!(
                                            s.host
                                                .presentation
                                                .notifications
                                                .as_ref()
                                                .unwrap()
                                                .dispatch
                                                .depth()
                                                == 2
                                        );
                                        Some("zoom")
                                    }
                                    Request::ViewerRotation { value } => {
                                        ensure!(
                                            value == 0 && s.simulation.vm.world.globals[&2] == 2
                                        );
                                        Some("rotation")
                                    }
                                    Request::UiFlags { bits: 0x80000 } => {
                                        target.is_some().then_some("tail")
                                    }
                                    Request::UiRefresh if target.is_none() => Some("selection"),
                                    Request::UiFlags { bits: 8 }
                                    | Request::UiRefresh
                                    | Request::PanelRefresh
                                    | Request::SharedInterface
                                    | Request::PanelBefore
                                    | Request::PanelAfter => None,
                                    ref other => {
                                        anyhow::bail!("engine listener work escaped: {other:?}")
                                    }
                                };
                                (key, Reply::Done)
                            }
                        };
                        if budget == 1 && key.is_some_and(|k| restored.insert(k)) {
                            let saved = s.save();
                            // Orphaning a callback must reject the complete save,
                            // before accepting it as a resumable live operation.
                            let mut corrupt = serde_json::to_value(
                                s.host.presentation.notifications.as_ref().unwrap(),
                            )?;
                            if key == Some("zoom") {
                                corrupt["viewer"] = serde_json::Value::Null;
                                ensure!(
                                    serde_json::from_value::<State>(corrupt)?
                                        .validate(&s.host.presentation, &s.simulation)
                                        .is_err()
                                );
                            }
                            s = Session::restore(
                                &index,
                                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                            )
                            .with_context(|| format!("{name} notification restore at {slice}"))?;
                            ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&saved)?);
                            ensure!(
                                s.advance_house_notification(c, budget)?
                                    == Progress::Service(service.clone())
                            );
                        }
                        services.push(serde_json::json!({"service":service,"depth":s.host.presentation.notifications.as_ref().unwrap().dispatch.depth(),"globals":[s.simulation.vm.world.globals[&2],s.simulation.vm.world.globals[&3],s.simulation.vm.world.globals[&26]]}));
                        // Original UI handler rereads global26 after this callback.
                        if matches!(&service,Service::Listener(call) if call.request==Request::UiFlags { bits:0x80000 })
                        {
                            s.simulation.vm.world.globals.insert(26, -7);
                        }
                        s.acknowledge_house_notification(&service, reply)?;
                        ensure!(
                            s.acknowledge_house_notification(&service, Reply::Done)
                                .is_err()
                        );
                    }
                }
            }
            ensure!(done && !s.house_notification_pending() && !s.view_refresh_pending());
            ensure!(
                before
                    == (
                        s.simulation.vm.tick,
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state,
                        s.simulation.vm.crt_rng_state
                    ),
                "broadcast changed time/RNG"
            );
            ensure!(s.simulation.vm.world.globals[&2] == 2);
            ensure!(s.simulation.vm.world.globals[&3] == target.unwrap_or(0));
            ensure!(
                s.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == target
            );
            let state = s.host.presentation.notifications.as_ref().unwrap();
            ensure!(state.dispatch.fields.renderer_dirty == 1);
            ensure!(state.dispatch.fields.ui.dirty == if target.is_none() { 1 } else { 0x86 });
            ensure!(
                services
                    .iter()
                    .any(|e| e["service"]["Listener"]["request"] == "PanelAfter")
            );
            if budget == 1 {
                ensure!(restored.len() == 3, "restore coverage {restored:?}");
            }
            let saved = serde_json::to_vec(&s.save())?;
            ensure!(s.advance_house_notification(portrait, budget)? == Progress::Complete(1));
            ensure!(
                serde_json::to_vec(&s.save())? == saved,
                "returned notification repeated work"
            );
            runs.push(serde_json::json!({"budget":budget,"restores":restored.len(),"services":services,"tick_rng_unchanged":true}));
            completed.push(s.save());
        }
        ensure!(
            runs[0]["services"] == runs[1]["services"],
            "notification order changed with work budget"
        );
        ensure!(
            serde_json::to_vec(&completed[0])? == serde_json::to_vec(&completed[1])?,
            "{name} saved notification state diverged"
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
            let floor = if name == "goth" && rotation == 1 {
                2
            } else {
                1
            };
            let c = sims_render::original_house::OriginalHouse::camera(
                &a, 960, 720, rotation, 0.55, floor,
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
        if name == "newbie" {
            // E4 stores its narrowed payload independently of the selected
            // person's native flags. Its visibility getter must use those flags,
            // not reinterpret global3/the event payload as a person handle.
            let selected = a
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?;
            ensure!(selected.is_some());
            let p = a.host.presentation.presenter.as_mut().unwrap();
            ensure!(p.view.floor.visibility.enabled != 0);
            p.view.floor.visibility.mask.rows.fill(0);
            p.view.floor.visibility.dirty = 0;
            p.view.floor.visibility.include_visible = 0;
            a.host
                .presentation
                .notifications
                .as_mut()
                .unwrap()
                .dispatch
                .fields
                .include_visible = 0;
            a.begin_house_notification(registry()?, fields(0), 0xe4, 0xffff8001)?;
            let mut done = false;
            for _ in 0..100 {
                match a.advance_house_notification(portrait, 1000)? {
                    Progress::Pending => (),
                    Progress::Complete(1) => {
                        done = true;
                        break;
                    }
                    Progress::Service(ref service @ Service::Listener(ref call))
                        if matches!(
                            call.request,
                            Request::UiFlags { bits: 8 } | Request::UiRefresh
                        ) =>
                    {
                        a.acknowledge_house_notification(service, Reply::Done)?;
                    }
                    other => anyhow::bail!("unexpected root E4 work: {other:?}"),
                }
            }
            ensure!(done && a.simulation.vm.world.globals[&3] == -32767);
            ensure!(
                a.simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    == selected
            );
            ensure!(
                a.host
                    .presentation
                    .presenter
                    .as_ref()
                    .unwrap()
                    .view
                    .floor
                    .visibility
                    .mask
                    .rows
                    .iter()
                    .any(|n| *n != 0)
            );
        }
        // A missing primary runs both real factories, even with an old
        // secondary. The Objects-only fixture has no Globals/UI receiver.
        if name == "newbie" {
            let mut s = Session::restore(&index, initial)?;
            s.begin_selection(None)?;
            while !s.advance_selection(portrait, 10000)? {}
            let (view, inputs, _order, camera) =
                super::camera_session::context(&s, portrait, level)?;
            let f = fields(view.floor.visibility.include_visible);
            s.configure_registry_house_view(view, inputs, camera)?;
            let mut r = Registry::default();
            r.construct(1).map_err(anyhow::Error::msg)?;
            r.bind(1, Receiver::Objects).map_err(anyhow::Error::msg)?;
            s.simulation.native_lot.as_mut().unwrap().indicators.primary = None;
            s.begin_house_notification(r, f, 0xf6, 0)?;
            let g = s.simulation.vm.world.globals.clone();
            let count = s.simulation.vm.world.objects.len();
            let mut complete = false;
            for _ in 0..1000 {
                match s.advance_house_notification(portrait, 1000)? {
                    Progress::Pending => (),
                    Progress::Complete(1) => {
                        complete = true;
                        break;
                    }
                    other => anyhow::bail!("unexpected indicator constructor service {other:?}"),
                }
            }
            ensure!(
                complete && !s.house_notification_pending() && s.simulation.vm.world.globals == g
            );
            ensure!(
                s.simulation.vm.world.objects.len() == count + 2,
                "missing primary did not run both original factories"
            );
            s.host
                .presentation
                .notifications
                .as_ref()
                .unwrap()
                .validate(&s.host.presentation, &s.simulation)?;
        }
        reports.push(serde_json::json!({"house":name,"selected":target,"runs":runs,"continued_ticks":10,"pixel_views":2,"native_component_comparisons":"existing unadjusted fixtures; combined live host tested here","startup_registry":"explicit supplied order","device_ui":"explicit test sinks"}));
        println!(
            "{name}: live F6 with nested broadcasts, three full restores, ten subsequent ticks and two CPU views match"
        );
    }
    gallery.save(root.join("cache/original-notification-session-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-notification-session-houses.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
