//! F6's missing-cache branch owns both factories and synchronous E2 calls.
//! Normal43fc94 initialization precedes F6; this deferred profile isolates the
//! fallback and does not establish a complete native startup sequence.
//! Startup registry, view context and device/UI replies are explicit inputs.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    notifications::{Progress, Reply, Service, State},
};
use sims_sim::{lot_selection::Request as Selection, pose::portrait::view::Input};
use sims_vm::commander::house::Request;
use std::collections::BTreeSet;

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let portrait = Input {
        viewport: [0, 0, 1000, 740],
        center: [500, 370],
        zoom: 2,
        rotation: 0,
    };
    let mut reports = vec![];
    let mut gallery = image::RgbaImage::new(2880, 1440);
    for (column, (name, source)) in [
        ("newbie", "UserData/Houses/House07.iff"),
        ("goth", "UserData/Houses/House05.iff"),
        ("mashuga", "UserData2/Houses/House09.iff"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut initial = Session::load(
            &index,
            source,
            Options {
                defer_indicators: true,
                ..Default::default()
            },
        )?;
        ensure!(initial.host.presentation.awaiting_house_open);
        ensure!(initial.simulation.native_lot.as_ref().unwrap().indicators == Default::default());
        ensure!(initial.step().is_err() && initial.begin_selection(None).is_err());
        let level = if name == "goth" { 2 } else { 1 };
        initial
            .simulation
            .vm
            .world
            .refresh
            .as_mut()
            .unwrap()
            .visible_level = level;
        let (mut view, inputs, _order, retained) =
            super::camera_session::context(&initial, portrait, level)?;
        view.floor.history.clear();
        initial.configure_registry_house_view(view, inputs, retained)?;
        let initial = initial.save();
        let mut completed = vec![];
        let mut runs = vec![];
        for budget in [1, 10000] {
            let mut s = Session::restore(&index, initial.clone())?;
            let before = (
                s.simulation.vm.tick,
                s.simulation.vm.rng_state,
                s.simulation.vm.lot_rng_state,
            );
            let before_objects: BTreeSet<_> =
                s.simulation.vm.world.objects.keys().copied().collect();
            let fields = super::notification_session::fields(
                s.host
                    .presentation
                    .presenter
                    .as_ref()
                    .unwrap()
                    .view
                    .floor
                    .visibility
                    .include_visible,
            );
            s.begin_house_notification(super::notification_session::registry()?, fields, 0xf6, 0)?;
            let mut restored = BTreeSet::new();
            let mut created = BTreeSet::new();
            let mut services = vec![];
            let mut done = false;
            for slice in 0..25000 {
                let owner = s.host.presentation.notifications.as_ref().unwrap();
                let decoded: State = serde_json::from_slice(&serde_json::to_vec(owner)?)?;
                ensure!(&decoded == owner);
                decoded.validate(&s.host.presentation, &s.simulation)?;
                let point = match s.simulation.pending_native_selection() {
                    Some((Selection::FactoryGraphic(id), true)) => Some(("graphic", id)),
                    Some((Selection::CreationNotification(id), true)) => {
                        created.insert(id);
                        ensure!(owner.dispatch.depth() == 2);
                        let indicators = &s.simulation.native_lot.as_ref().unwrap().indicators;
                        ensure!(
                            indicators.primary != Some(id) && indicators.secondary != Some(id),
                            "cache published before E2 returned"
                        );
                        ensure!(s.host.ordinary.graphics.contains_key(&id));
                        ensure!(
                            s.host
                                .presentation
                                .presenter
                                .as_ref()
                                .unwrap()
                                .inputs
                                .keep_dynamic
                                .get(&id)
                                == Some(&0)
                        );
                        Some(("created", id))
                    }
                    _ => None,
                };
                if budget == 1 && point.is_some_and(|p| restored.insert(p)) {
                    let point = point.unwrap();
                    let bytes = serde_json::to_vec(&s.save())?;
                    ensure!(s.step().is_err() && s.begin_selection(None).is_err());
                    ensure!(s.advance_selection(portrait, 1).is_err());
                    ensure!(s.service_selection_assets(portrait, 1).is_err());
                    ensure!(serde_json::to_vec(&s.save())? == bytes);
                    let mut corrupt =
                        serde_json::to_value(s.host.presentation.notifications.as_ref().unwrap())?;
                    corrupt["initializer"] = Value::Null;
                    ensure!(
                        serde_json::from_value::<State>(corrupt)?
                            .validate(&s.host.presentation, &s.simulation)
                            .is_err()
                    );
                    if point.0 == "created" {
                        let mut corrupt = serde_json::to_value(
                            s.host.presentation.notifications.as_ref().unwrap(),
                        )?;
                        corrupt["initializer"]["nested"] = json!(point.1 + 1);
                        ensure!(
                            serde_json::from_value::<State>(corrupt)?
                                .validate(&s.host.presentation, &s.simulation)
                                .is_err()
                        );
                        ensure!(
                            s.simulation
                                .complete_indicator_created(point.1 + 1)
                                .is_err()
                        );
                    }
                    s = Session::restore(&index, serde_json::from_slice(&bytes)?)
                        .with_context(|| format!("{name} restore {point:?} at slice {slice}"))?;
                    ensure!(serde_json::to_vec(&s.save())? == bytes);
                    println!("{name}: restored {point:?}");
                }
                match s
                    .advance_house_notification(portrait, budget)
                    .with_context(|| format!("{name} budget {budget} slice {slice}"))?
                {
                    Progress::Pending => (),
                    Progress::Complete(1) => {
                        done = true;
                        break;
                    }
                    Progress::Complete(other) => anyhow::bail!("unexpected F6 result {other}"),
                    Progress::Service(service) => {
                        let reply = match &service {
                            Service::View(_) => Reply::View(
                                s.host.presentation.presenter.as_ref().unwrap().view.clone(),
                            ),
                            Service::Camera(request) => {
                                Reply::Camera(super::notification_session::device_reply(request)?)
                            }
                            Service::Listener(call)
                                if matches!(
                                    call.request,
                                    Request::ViewerZoom { .. }
                                        | Request::ViewerRotation { .. }
                                        | Request::UiFlags { .. }
                                        | Request::UiRefresh
                                        | Request::PanelRefresh
                                        | Request::SharedInterface
                                        | Request::PanelBefore
                                        | Request::PanelAfter
                                ) =>
                            {
                                Reply::Done
                            }
                            other => anyhow::bail!("engine work escaped F6 owner: {other:?}"),
                        };
                        services.push(serde_json::to_value(&service)?);
                        s.acknowledge_house_notification(&service, reply)?;
                    }
                }
            }
            ensure!(
                done && !s.house_notification_pending() && !s.host.presentation.awaiting_house_open
            );
            ensure!(
                before
                    == (
                        s.simulation.vm.tick,
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state
                    )
            );
            let indicators = &s.simulation.native_lot.as_ref().unwrap().indicators;
            let ids = [
                indicators.primary.context("primary absent")?,
                indicators.secondary.context("secondary absent")?,
            ];
            ensure!(s.simulation.vm.world.objects.len() == before_objects.len() + 2);
            for (id, guid) in ids.into_iter().zip(sims_vm::load_indicators::GUIDS) {
                ensure!(!before_objects.contains(&id));
                let o = &s.simulation.vm.world.objects[&id];
                ensure!(o.definition.as_ref().unwrap().guid == guid && o.data[&8] & 0x4000 != 0);
                ensure!(s.host.ordinary.graphics.contains_key(&id));
            }
            if budget == 1 {
                ensure!(
                    restored.len() == 4 && created == BTreeSet::from(ids),
                    "restore coverage {restored:?}"
                );
            }
            let bytes = serde_json::to_vec(&s.save())?;
            ensure!(s.advance_house_notification(portrait, budget)? == Progress::Complete(1));
            ensure!(serde_json::to_vec(&s.save())? == bytes);
            runs.push(json!({"budget":budget,"restores":restored.len(),"indicators":ids,"services":services,"crt_rng":s.simulation.vm.crt_rng_state}));
            completed.push(s.save());
        }
        ensure!(runs[0]["services"] == runs[1]["services"]);
        ensure!(
            serde_json::to_vec(&completed[0])? == serde_json::to_vec(&completed[1])?,
            "{name} state differs by budget"
        );
        let mut a = Session::restore(&index, completed.remove(0))?;
        let mut b = Session::restore(&index, completed.remove(0))?;
        for _ in 0..10 {
            ensure!(a.step()? == sims_runtime::original_session::Progress::Tick);
            ensure!(b.step()? == sims_runtime::original_session::Progress::Tick);
        }
        ensure!(serde_json::to_vec(&a.save())? == serde_json::to_vec(&b.save())?);
        let mut renderer = sims_render::original_house::OriginalHouse::default();
        let mut other = sims_render::original_house::OriginalHouse::default();
        for rotation in 0..2 {
            let floor = if name == "goth" && rotation == 1 {
                2
            } else {
                1
            };
            let camera = sims_render::original_house::OriginalHouse::camera(
                &a, 960, 720, rotation, 0.55, floor,
            );
            let pixels = sims_render::software::render(&renderer.scene(&a, camera)?);
            ensure!(pixels == sims_render::software::render(&other.scene(&b, camera)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        reports.push(json!({"house":name,"runs":runs,"continued_ticks":10,"pixel_views":2,"profile":"explicit F6 with deferred indicators; supplied registry/view/device context"}));
        println!(
            "{name}: two original factories, four full restores, ten continued ticks and two CPU views match"
        );
    }
    gallery.save(root.join("cache/original-indicator-opening-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-indicator-opening-houses.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
