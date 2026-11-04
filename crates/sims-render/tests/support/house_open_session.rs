//! Session ownership integration with explicit headless camera-service replies.
//! Native camera/floor/wall implementations have separate composition tests;
//! this test does not present these supplied replies as an automatic UI backend.
use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Session, presenter::Progress};
use sims_sim::lot_presenter::{
    Inputs, hover::History, lifecycle, open, visibility, walls::cutaway::Mask,
};
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
        let people: Vec<_> = initial
            .house
            .household
            .as_ref()
            .unwrap()
            .residents
            .iter()
            .map(|r| r.object as i16)
            .collect();
        let camera = sims_sim::pose::portrait::view::Input {
            viewport: [0, 0, 1000, 740],
            center: [0, 0],
            zoom: 2,
            rotation: 0,
        };
        initial.begin_selection(Some(people[0]))?;
        while !initial.advance_selection(camera, 10000)? {}
        // Controlled post-notification inputs exercise a nonempty drain and
        // original star factory. They do not alter the read-only house files.
        initial
            .simulation
            .vm
            .world
            .objects
            .get_mut(&people[0])
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(77, -people[1]);
        initial
            .simulation
            .vm
            .world
            .objects
            .get_mut(&people[1])
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(81, 10);
        initial.simulation.vm.world.indicator_queues = sims_vm::indicator_unregister::Queues {
            dates: vec![None, Some(people[0]), Some(people[0])],
            stars: vec![Some(people[1]), None, Some(people[1])],
            dirty: true,
        };
        let original = initial.save();
        ensure!(
            serde_json::to_value(&original.view)?
                .get("presenter")
                .is_none(),
            "ordinary old view serialization changed"
        );
        let mut results = vec![];
        let mut runs = vec![];
        for (budget, resume) in [(1, true), (10000, false)] {
            let mut session = Session::restore(&index, original.clone())?;
            let inputs = Inputs {
                force_dynamic: false,
                graphic_ready: session
                    .host
                    .ordinary
                    .presenter_readiness(&session.simulation.vm, &session.program)?,
                // Constructor43189d initializes this field to0 in fresh objects.
                keep_dynamic: session
                    .simulation
                    .vm
                    .world
                    .objects
                    .keys()
                    .map(|&id| (id, 0))
                    .collect(),
            };
            let level = if name == "goth" { 2 } else { 1 };
            let view = open::State {
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
                origin: [500, 370],
                pending: [13, -27],
                zoom: 2,
                display_rotation: 0,
                widths: [64, 32, 32, 16],
            };
            session.begin_registry_view_refresh(view, inputs)?;
            let saved = session.save();
            ensure!(session.advance_view_refresh(camera, 0).is_err());
            for request in [
                open::Request::ClassifyObjects,
                open::Request::DrainIndicators,
            ] {
                let view = session
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .unwrap()
                    .view
                    .clone();
                ensure!(session.acknowledge_view_refresh(&request, view).is_err());
            }
            ensure!(serde_json::to_vec(&session.save())? == serde_json::to_vec(&saved)?);
            let clock_rng = (
                session.simulation.vm.tick,
                session.simulation.vm.rng_state,
                session.simulation.vm.lot_rng_state,
            );
            let mut expected_crt = session.simulation.vm.crt_rng_state.unwrap();
            for _ in 0..128 {
                sims_vm::rng::next_crt(&mut expected_crt);
            }
            let mut restored = BTreeSet::new();
            let mut services = vec![];
            let mut child_callbacks = BTreeSet::new();
            let mut finished = false;
            for slice in 0..10000 {
                let owner = session.host.presentation.presenter.as_ref().unwrap();
                let value = serde_json::to_value(owner)?;
                if value["drain_started"] == true {
                    let child = session.simulation.native_selection.take();
                    ensure!(owner.validate_simulation(&session.simulation).is_err());
                    session.simulation.native_selection = child;
                }
                if !value["classification"].is_null() {
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
                let key = format!(
                    "{:?}:{}:{}:{:?}",
                    owner.request(),
                    value["classification"],
                    value["drain_started"],
                    session.simulation.pending_native_selection()
                );
                if let Some((request, started)) = session.simulation.pending_native_selection() {
                    child_callbacks.insert(format!("{request:?}:{started}"));
                }
                if resume && restored.insert(key) {
                    let saved = session.save();
                    // A pending synchronous presentation owner blocks both
                    // simulation ticks and unrelated selection/drain calls.
                    ensure!(session.step().is_err());
                    ensure!(session.begin_selection(Some(people[1])).is_err());
                    ensure!(session.begin_indicator_refresh().is_err());
                    ensure!(session.advance_selection(camera, 1).is_err());
                    ensure!(session.service_selection_assets(camera, 1).is_err());
                    ensure!(serde_json::to_vec(&session.save())? == serde_json::to_vec(&saved)?);
                    session = Session::restore(
                        &index,
                        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                    )
                    .with_context(|| format!("{name} F6 restore at slice{slice}"))?;
                    ensure!(serde_json::to_vec(&session.save())? == serde_json::to_vec(&saved)?);
                }
                match session.advance_view_refresh(camera, budget)? {
                    Progress::Service(request) => {
                        let mut view = session
                            .host
                            .presentation
                            .presenter
                            .as_ref()
                            .unwrap()
                            .view
                            .clone();
                        // Explicit presentation-service inputs for this session
                        // ownership check. No fake graphics effect is claimed.
                        match request {
                            open::Request::InvalidateView => (),
                            open::Request::Zoom {
                                zoom,
                                force,
                                refresh_ui,
                            } => ensure!(zoom == 2 && force == 1 && refresh_ui == 1),
                            open::Request::Rotate {
                                rotation,
                                force,
                                refresh_ui,
                            } => {
                                ensure!(rotation == 0 && force == 1 && refresh_ui == 1);
                                session.simulation.vm.world.camera_rotation = rotation as u8;
                            }
                            open::Request::Floor {
                                level: requested,
                                force,
                            } => {
                                ensure!(requested == level && force == 1);
                                session
                                    .simulation
                                    .vm
                                    .world
                                    .refresh
                                    .as_mut()
                                    .unwrap()
                                    .visible_level = requested;
                            }
                            open::Request::Scroll {
                                position,
                                immediate,
                            } => {
                                ensure!(immediate == 1 && position == view.origin);
                                view.pending = [0, 0];
                            }
                            open::Request::WallState {
                                enabled,
                                level: requested,
                            } => {
                                ensure!(enabled == 0 && requested == level);
                                // The retained begin-time cache is not the live
                                // ownership query used by4aacb9. It must be
                                // refreshed before classification, even after
                                // restoring at this callback boundary.
                                session
                                    .host
                                    .presentation
                                    .presenter
                                    .as_mut()
                                    .unwrap()
                                    .inputs
                                    .graphic_ready
                                    .clear();
                            }
                            other => anyhow::bail!("engine service escaped presenter: {other:?}"),
                        }
                        services.push(request.clone());
                        session.acknowledge_view_refresh(&request, view)?;
                    }
                    Progress::Pending => (),
                    Progress::CameraService(_) => anyhow::bail!("unexpected retained camera"),
                    Progress::Complete => {
                        finished = true;
                        break;
                    }
                }
            }
            ensure!(finished && !session.view_refresh_pending());
            ensure!(services.len() == 6 && session.simulation.vm.world.indicator_queues.is_empty());
            ensure!(
                clock_rng
                    == (
                        session.simulation.vm.tick,
                        session.simulation.vm.rng_state,
                        session.simulation.vm.lot_rng_state
                    )
            );
            ensure!(session.simulation.vm.crt_rng_state == Some(expected_crt));
            let state = session.host.presentation.presenter.as_ref().unwrap();
            state.validate().map_err(anyhow::Error::msg)?;
            ensure!(!state.inputs.graphic_ready.is_empty());
            let child_star = session.simulation.vm.world.objects[&people[1]].slots[1]
                .context("drain did not create original star")?;
            runs.push(serde_json::json!({"budget":budget,"resume":resume,"restores":restored.len(),"services":services,"indicator_callbacks":child_callbacks,"child_star":child_star,"lists":state.lists}));
            results.push(session.save());
            // A repeated notification retains the old lists until the native
            // classification stage actually clears them, after camera work.
            let previous = state.clone();
            match previous.order {
                Some(mut order) => {
                    order.dirty = true;
                    session.begin_view_refresh(previous.view, previous.inputs, order)?;
                }
                None => {
                    session.simulation.vm.world.invalidate_registry_order();
                    session.begin_registry_view_refresh(previous.view, previous.inputs)?;
                }
            }
            ensure!(session.host.presentation.presenter.as_ref().unwrap().lists == previous.lists);
        }
        ensure!(
            serde_json::to_vec(&results[0])? == serde_json::to_vec(&results[1])?,
            "{name} F6 restore or work budget changed state"
        );
        let mut a = Session::restore(&index, results.remove(0))?;
        let mut b = Session::restore(&index, results.remove(0))?;
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
            let camera = sims_render::original_house::OriginalHouse::camera(
                &a, 960, 720, rotation, 0.55, level,
            );
            let pixels = sims_render::software::render(&ra.scene(&a, camera)?);
            ensure!(pixels == sims_render::software::render(&rb.scene(&b, camera)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        report.push(serde_json::json!({"house":name,"runs":runs,"restored_state_equal":true,"continued_ticks":10,"restored_pixel_views":2,"camera_services":"explicit supplied headless replies; full camera composition and automatic startup dispatch separate"}));
        println!(
            "{name}: retained F6/classification/indicator owner matches after restore and10ticks"
        );
    }
    gallery.save(root.join("cache/original-house-open-session-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-house-open-session-houses.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Freshly loaded objects are already dynamic. Force one real, visible drawing
/// root through the opposite flag so the session test actually suspends inside
/// Classification's recursive DrawUpdate instead of only before/after it.
pub fn classification_child() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut initial = Session::load(&index, "UserData/Houses/House07.iff", Options::default())?;
    let world = &initial.simulation.vm.world;
    let target = world
        .person
        .as_ref()
        .unwrap()
        .object_order
        .iter()
        .copied()
        .find(|id| {
            let object = &world.objects[id];
            let p = &world.positions[id];
            let words = sims_vm::construction::definition_words(
                &initial.program,
                object.definition.as_ref().unwrap(),
            )
            .unwrap();
            object.container.is_none()
                && object.data.get(&34) == Some(&0)
                && (words[4] != 0 || words[10] != 0)
                && p.level == 1
                && (16..1008).contains(&p.x)
                && (16..1008).contains(&p.y)
                && world.refresh.as_ref().unwrap().draw[id].level == 1
        })
        .context("visible drawing root absent")?;
    let refresh = initial.simulation.vm.world.refresh.as_mut().unwrap();
    refresh.draw.get_mut(&target).unwrap().dynamic = false;
    refresh.invalidated.clear();
    let input = initial.save();
    let mut results = vec![];
    let mut counts = vec![];
    let camera = sims_sim::pose::portrait::view::Input {
        viewport: [0, 0, 800, 600],
        center: [0, 0],
        zoom: 2,
        rotation: 0,
    };
    for (budget, resume) in [(1, true), (10000, false)] {
        let mut s = Session::restore(&index, input.clone())?;
        s.begin_registry_view_refresh(
            open::State {
                floor: lifecycle::State {
                    level: 1,
                    all_walls: 0,
                    history: History::house_view(),
                    visibility: visibility::State {
                        enabled: 1,
                        dirty: 0,
                        include_visible: 1,
                        mask: Mask {
                            first_row: 0,
                            rows: vec![0; 64],
                        },
                    },
                },
                origin: [0, 0],
                pending: [0, 0],
                zoom: 2,
                display_rotation: 0,
                widths: [64, 32, 32, 16],
            },
            Inputs {
                force_dynamic: true,
                graphic_ready: Default::default(),
                keep_dynamic: s
                    .simulation
                    .vm
                    .world
                    .objects
                    .keys()
                    .map(|&id| (id, 0))
                    .collect(),
            },
        )?;
        let mut restored = BTreeSet::new();
        let mut finished = false;
        for _ in 0..1000 {
            let owner = s.host.presentation.presenter.as_ref().unwrap();
            let mut value = serde_json::to_value(owner)?;
            if resume
                && !value["classification"].is_null()
                && restored.insert(value["classification"].to_string())
            {
                ensure!(
                    !s.simulation.vm.world.refresh.as_ref().unwrap().draw[&target].dynamic,
                    "flag written before invalidation returned"
                );
                value["owner"]["request"] = serde_json::Value::Null;
                ensure!(
                    serde_json::from_value::<sims_runtime::original_session::presenter::State>(
                        value
                    )?
                    .validate()
                    .is_err()
                );
                let saved = s.save();
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                )?;
                ensure!(serde_json::to_vec(&s.save())? == serde_json::to_vec(&saved)?);
            }
            match s.advance_view_refresh(camera, budget)? {
                Progress::Service(request) => {
                    // Same explicit headless camera boundary as the larger
                    // three-house test. This check targets the nested child.
                    let view = s.host.presentation.presenter.as_ref().unwrap().view.clone();
                    s.acknowledge_view_refresh(&request, view)?;
                }
                Progress::Pending => (),
                Progress::CameraService(_) => anyhow::bail!("unexpected retained camera"),
                Progress::Complete => {
                    finished = true;
                    break;
                }
            }
        }
        ensure!(finished && (!resume || restored.len() >= 2));
        let refresh = s.simulation.vm.world.refresh.as_ref().unwrap();
        ensure!(refresh.draw[&target].dynamic && refresh.invalidated.contains(&target));
        let before = &input.simulation.state.vm;
        let after = &s.simulation.vm;
        ensure!(
            (
                before.tick,
                before.rng_state,
                before.lot_rng_state,
                before.crt_rng_state
            ) == (
                after.tick,
                after.rng_state,
                after.lot_rng_state,
                after.crt_rng_state
            )
        );
        counts.push(restored.len());
        results.push(s.save());
    }
    ensure!(serde_json::to_vec(&results[0])? == serde_json::to_vec(&results[1])?);
    std::fs::write(
        root.join("cache/native-house-open-classification-session.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"house":"newbie","target":target,"restores":counts[0],"state_equal":true,"tick_rng_unchanged":true,"invalidated_before_flag_write":true}),
        )?,
    )?;
    println!(
        "Newbie object{target}: {} full-session restores inside classification; flag/write order and budgets match",
        counts[0]
    );
    Ok(())
}
