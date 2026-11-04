//! Normal43fc94 tail, after ordered object visits. F6 remains a later explicit
//! caller: this test does not synthesize the intervening428041 load work.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    notifications::{Progress, Reply, Service},
    postload::State,
};
use sims_sim::{lot_selection::Request as Selection, pose::portrait::view::Input};
use sims_vm::{commander::house::Request, load_finalize::Request as TailCall};
use std::collections::BTreeSet;

pub fn run() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let camera = Input {
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
                native_postload: true,
                ..Default::default()
            },
        )?;
        let level = if name == "goth" { 2 } else { 1 };
        initial
            .simulation
            .vm
            .world
            .refresh
            .as_mut()
            .unwrap()
            .visible_level = level;
        // Explicit counterfactual, as in the original star-factory regression:
        // exercise Superstar refill without changing the loader's person data.
        if name == "newbie" {
            let id = initial.house.household.as_ref().unwrap().residents[0].object as i16;
            initial
                .simulation
                .vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(81, 10);
        }
        let (mut view, inputs, _order, retained) =
            super::camera_session::context(&initial, camera, level)?;
        view.floor.history.clear();
        initial.configure_registry_house_view(view, inputs, retained)?;
        let fields = super::notification_session::fields(
            initial
                .host
                .presentation
                .presenter
                .as_ref()
                .unwrap()
                .view
                .floor
                .visibility
                .include_visible,
        );
        let registry = super::notification_session::registry()?;
        let before = serde_json::to_vec(&initial.save())?;
        ensure!(
            initial
                .begin_house_notification(registry.clone(), fields.clone(), 0xf6, 0)
                .is_err()
        );
        ensure!(initial.step().is_err() && initial.begin_selection(None).is_err());
        ensure!(serde_json::to_vec(&initial.save())? == before);
        initial.begin_postload(registry, fields)?;
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
            let object_count = s.simulation.vm.world.objects.len();
            let mut restored = BTreeSet::new();
            let mut created = BTreeSet::new();
            let mut callbacks = vec![];
            let mut last = None;
            let mut done = false;
            for slice in 0..25000 {
                let owner = s.host.presentation.postload.as_ref().unwrap();
                let decoded: State = serde_json::from_slice(&serde_json::to_vec(owner)?)?;
                ensure!(&decoded == owner);
                decoded.validate(&s.simulation)?;
                let call = owner.pending();
                if call != last {
                    if let Some(call) = call {
                        callbacks.push(call);
                    }
                    last = call;
                }
                let point = match s.simulation.pending_native_selection() {
                    Some((Selection::FactoryGraphic(id), true)) => Some(("graphic", id)),
                    Some((Selection::CreationNotification(id), true)) => {
                        created.insert(id);
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
                        if call == Some(TailCall::Initialize) {
                            let indicators = &s.simulation.native_lot.as_ref().unwrap().indicators;
                            ensure!(
                                indicators.primary != Some(id) && indicators.secondary != Some(id)
                            );
                        }
                        Some(("created", id))
                    }
                    _ => match call {
                        Some(TailCall::Selection(_)) => Some(("selection", 0)),
                        Some(TailCall::Star(id)) => Some(("star", id)),
                        _ => None,
                    },
                };
                if budget == 1 && point.is_some_and(|p| restored.insert(p)) {
                    let point = point.unwrap();
                    let bytes = serde_json::to_vec(&s.save())?;
                    ensure!(s.step().is_err() && s.begin_selection(None).is_err());
                    ensure!(s.advance_selection(camera, 1).is_err());
                    ensure!(s.service_selection_assets(camera, 1).is_err());
                    ensure!(s.advance_view_refresh(camera, 1).is_err());
                    let (registry, fields) =
                        s.host.presentation.postload.as_ref().unwrap().context();
                    ensure!(
                        s.begin_house_notification(registry.clone(), fields.clone(), 0xf6, 0)
                            .is_err()
                    );
                    ensure!(serde_json::to_vec(&s.save())? == bytes);
                    if restored.len() == 1 {
                        let mut orphan = s.save();
                        orphan.view.postload = None;
                        ensure!(Session::restore(&index, orphan).is_err());
                    }
                    if point.0 == "created" {
                        let mut corrupt =
                            serde_json::to_value(s.host.presentation.postload.as_ref().unwrap())?;
                        corrupt["created"] = Value::Null;
                        ensure!(
                            serde_json::from_value::<State>(corrupt)?
                                .validate(&s.simulation)
                                .is_err()
                        );
                        ensure!(
                            s.simulation
                                .complete_indicator_created(point.1 + 1)
                                .is_err()
                        );
                    }
                    s = Session::restore(&index, serde_json::from_slice(&bytes)?)
                        .with_context(|| format!("{name} restore {point:?} slice {slice}"))?;
                    ensure!(serde_json::to_vec(&s.save())? == bytes);
                    println!("{name}: post-load restored {point:?}");
                }
                if s.advance_postload(camera, budget)
                    .with_context(|| format!("{name} post-load budget {budget} slice {slice}"))?
                {
                    done = true;
                    break;
                }
            }
            ensure!(done && !s.postload_pending() && s.host.presentation.awaiting_house_open);
            ensure!(
                before
                    == (
                        s.simulation.vm.tick,
                        s.simulation.vm.rng_state,
                        s.simulation.vm.lot_rng_state
                    )
            );
            let extra = if name == "newbie" { 6 } else { 2 };
            ensure!(s.simulation.vm.world.objects.len() == object_count + extra);
            if budget == 1 {
                ensure!(
                    created.len() == extra,
                    "{name} creation callbacks {created:?}"
                );
                let people = &s.simulation.vm.world.person.as_ref().unwrap().person_order;
                let mut expected = vec![
                    TailCall::Initialize,
                    TailCall::Selection(
                        s.simulation
                            .native_selected_person()
                            .map_err(anyhow::Error::msg)?,
                    ),
                ];
                expected.extend(people.iter().copied().map(TailCall::Star));
                ensure!(
                    callbacks == expected,
                    "{name} callback order differs: {callbacks:?}"
                );
            }
            let bytes = serde_json::to_vec(&s.save())?;
            ensure!(s.advance_postload(camera, budget)? && s.step().is_err());
            ensure!(serde_json::to_vec(&s.save())? == bytes);
            // Separate explicitly supplied F6. Native intermediate load services
            // and process/UI startup are not established by this composition.
            let (registry, fields) = s.host.presentation.postload.as_ref().unwrap().context();
            s.begin_house_notification(registry.clone(), fields.clone(), 0xf6, 0)?;
            let mut services = vec![];
            done = false;
            for _ in 0..25000 {
                match s.advance_house_notification(camera, budget)? {
                    Progress::Pending => (),
                    Progress::Complete(1) => {
                        done = true;
                        break;
                    }
                    Progress::Complete(n) => anyhow::bail!("F6 returned {n}"),
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
                            other => anyhow::bail!("engine work escaped F6: {other:?}"),
                        };
                        services.push(serde_json::to_value(&service)?);
                        s.acknowledge_house_notification(&service, reply)?;
                    }
                }
            }
            ensure!(done && !s.host.presentation.awaiting_house_open);
            runs.push(json!({"budget":budget,"restores":restored.len(),"created":created,"callbacks":callbacks,"services":services,"crt_rng":s.simulation.vm.crt_rng_state}));
            completed.push(s.save());
        }
        ensure!(runs[0]["services"] == runs[1]["services"]);
        ensure!(
            serde_json::to_vec(&completed[0])? == serde_json::to_vec(&completed[1])?,
            "{name} budget changes state"
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
        reports.push(json!({"house":name,"runs":runs,"continued_ticks":10,"pixel_views":2,"counterfactual_fame":name=="newbie","profile":"owned43fc94 tail; later explicit F6; supplied registry/view/device context"}));
        println!("{name}: post-load, explicit F6, ten ticks and two CPU views match");
    }
    gallery.save(root.join("cache/original-postload-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-postload-houses.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
