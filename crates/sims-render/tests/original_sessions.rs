use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::{original_house::OriginalHouse, software};
use sims_runtime::original_session::{Options, Session};
#[path = "support/camera_command.rs"]
mod camera_command;
#[path = "support/camera_motion.rs"]
mod camera_motion;

#[test]
#[ignore = "original tutorial checkpoint; explicit device/audio/tool test sinks"]
fn original_camera_command_owns_shared_notifications_and_restores() -> Result<()> {
    camera_command::run()
}
#[path = "support/camera_session.rs"]
mod camera_session;
#[path = "support/indicator_opening_session.rs"]
mod indicator_opening_session;
#[path = "support/load_completion_session.rs"]
mod load_completion_session;
#[path = "support/notification_session.rs"]
mod notification_session;
#[path = "support/postload_session.rs"]
mod postload_session;

#[test]
#[ignore = "requires original houses; native cleanup, controller factory and later F6"]
fn original_load_completion_owns_live_cleanup_and_house_open() -> Result<()> {
    load_completion_session::run()
}

#[test]
#[ignore = "original Newbie assets; fresh current-program tutorial bootstrap with explicit host profile"]
fn fresh_newbie_tutorial_loads_original_controller_without_retagging() -> Result<()> {
    load_completion_session::fresh_newbie()
}
#[path = "support/selection_notifications.rs"]
mod selection_notifications;

#[test]
#[ignore = "requires original houses; native post-load tail and later explicit F6"]
fn original_postload_tail_owns_initialization_selection_and_star_callbacks() -> Result<()> {
    postload_session::run()
}

#[test]
#[ignore = "requires original houses; F6 indicator factories with saved graphics and nested E2"]
fn original_house_opening_owns_indicator_factories_and_created_callbacks() -> Result<()> {
    indicator_opening_session::run()
}

#[test]
#[ignore = "requires original houses; synchronous selection/deletion with explicit UI sinks"]
fn original_selection_waits_for_house_listeners_and_restores() -> Result<()> {
    selection_notifications::run()
}

#[test]
#[ignore = "requires original houses; live nested notifications with explicit device/UI sinks"]
fn original_house_broadcast_restores_viewer_and_nested_listeners() -> Result<()> {
    notification_session::run()
}
#[path = "support/house_open_session.rs"]
mod house_open_session;

#[test]
#[ignore = "requires private original houses; composed camera and explicit device services"]
fn original_f6_composed_camera_restores_and_continues() -> Result<()> {
    camera_session::run()
}

#[test]
#[ignore = "requires private original houses; retained F6 owner with explicit headless view services"]
fn original_f6_session_owner_restores_classification_and_indicator_drain() -> Result<()> {
    house_open_session::run()
}

#[test]
#[ignore = "requires private original house; restore inside F6 draw invalidation"]
fn original_f6_classification_child_restores_before_flag_write() -> Result<()> {
    house_open_session::classification_child()
}
#[path = "support/camera_transitions.rs"]
mod camera_transitions;

#[test]
#[ignore = "requires private original houses; borrowed native wall construction and CPU rendering"]
fn original_house_wall_graphs_restore_without_changing_simulation_or_pixels() -> Result<()> {
    use sims_sim::lot_presenter::walls::{
        BASIS,
        construction::Graph,
        cutaway::{Cursor, History, Mask},
    };
    use sims_sim::lot_presenter::{hover, room_masks, visibility};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(2880, 1440);
    let mut report = vec![];
    for (column, name) in ["newbie", "goth", "mashuga"].into_iter().enumerate() {
        let saved = serde_json::from_slice(&std::fs::read(
            root.join(format!("cache/presenter-{name}-save.json")),
        )?)?;
        let s = Session::restore(&index, saved)?;
        let before = serde_json::to_vec(&s.save())?;
        let lot = s
            .simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap();
        let mut graph = Graph::from_lot(lot).map_err(anyhow::Error::msg)?;
        let mut pass_counts = vec![];
        for enabled in [0, 1, 0] {
            let mut count = 0;
            graph
                .state
                .update(enabled, 0, BASIS, |_| count += 1)
                .map_err(anyhow::Error::msg)?;
            pass_counts.push(count);
            let restored: Graph = serde_json::from_slice(&serde_json::to_vec(&graph)?)?;
            ensure!(restored == graph);
            restored.state.validate().map_err(anyhow::Error::msg)?;
        }
        let mut camera_passes = vec![];
        let mask_camera =
            room_masks::Camera::initialized([-317, 203], 1).map_err(anyhow::Error::msg)?;
        for rotation in [0, 1, 2, 3, 0] {
            graph.rotate(rotation).map_err(anyhow::Error::msg)?;
            let mut history = History::capture(&graph).map_err(anyhow::Error::msg)?;
            for level in 1..=2 {
                let rooms = room_masks::floor(lot, level, rotation, [0, 63, 0, 63], &mask_camera)
                    .map_err(anyhow::Error::msg)?;
                let restored_rooms: std::collections::BTreeMap<i16, Mask> =
                    serde_json::from_slice(&serde_json::to_vec(&rooms)?)?;
                ensure!(rooms == restored_rooms);
                // Native mouse history produces the room IDs. The post-base
                // event and application mode remain explicit services; this
                // does not invent automatic UI dispatch. Original inverse
                // projection supplies every physical point.
                let masks: std::collections::BTreeMap<u16, visibility::Room> = rooms
                    .iter()
                    .map(|(&id, mask)| {
                        (
                            id as u16,
                            visibility::Room {
                                outdoors: lot.floor(level).unwrap().metrics[&id].outdoors,
                                mask: mask.clone(),
                            },
                        )
                    })
                    .collect();
                let mut visibility = visibility::State {
                    enabled: 1,
                    dirty: 0,
                    include_visible: 0,
                    mask: Mask {
                        first_row: 0,
                        rows: vec![0; 64],
                    },
                };
                let mut view = hover::View {
                    history: hover::History::house_view(),
                    screen_bounds: [-32768, -32768, 32767, 32767],
                    mode: 2,
                    cutaway: 1,
                    level,
                    bottom: 16,
                    floor_height: 78,
                    pointer_active: 0,
                    pointer: [0, 0],
                };
                let geometry = hover::Geometry {
                    size: lot.size,
                    rotation,
                    bounds: [0, 63, 0, 63],
                    primary: &lot.floor(level).unwrap().rooms.primary,
                };
                let splits = room_masks::SplitRooms::new();
                let projection = room_masks::View {
                    size: lot.size,
                    rotation,
                    bounds: [0, 63, 0, 63],
                    camera: &mask_camera,
                    heights: &lot.heights,
                    walls: &lot.floor(level).unwrap().source.walls,
                    splits: &splits,
                };
                // Same screen input as the complete native house hover probe.
                let screen = [(203 + (31 + 32) * 8 - 78 * (level - 1) + 3) as i16, -340];
                let mut mouse =
                    hover::Mouse::begin(&mut view, screen, 0, 1).map_err(anyhow::Error::msg)?;
                let mut projection_restores = 0;
                while let Some(request) = mouse.request().cloned() {
                    (view, mouse) = serde_json::from_slice(&serde_json::to_vec(&(&view, &mouse))?)?;
                    mouse.validate(&view).map_err(anyhow::Error::msg)?;
                    ensure!(mouse.request() == Some(&request));
                    match request {
                        hover::Request::Project { screen, flag } => {
                            ensure!(flag == 1);
                            let point = projection
                                .fine_point(screen, view.level)
                                .map_err(anyhow::Error::msg)?;
                            mouse
                                .projected(&mut view, &geometry, screen, point)
                                .map_err(anyhow::Error::msg)?;
                            projection_restores += 1;
                        }
                        hover::Request::ComposeVisible => {
                            visibility
                                .update(visibility::Inputs {
                                    size: lot.size as i32,
                                    rotation,
                                    bounds: [0, 63, 0, 63],
                                    visible_level: level,
                                    mode: view.mode,
                                    include_visible: 1,
                                    visible_rooms: &view.history.ids(),
                                    rooms: &masks,
                                    selected: None,
                                })
                                .map_err(anyhow::Error::msg)?;
                            mouse
                                .acknowledge(&view, &request)
                                .map_err(anyhow::Error::msg)?;
                        }
                        hover::Request::ClearMask => {
                            clear_and_restore(&mut visibility, lot.size, level)?;
                            mouse
                                .acknowledge(&view, &request)
                                .map_err(anyhow::Error::msg)?;
                        }
                    }
                }
                ensure!(mouse.complete() && mouse.returned() == Some(1));
                let restored_visibility: visibility::State =
                    serde_json::from_slice(&serde_json::to_vec(&visibility)?)?;
                ensure!(visibility == restored_visibility);
                let mut cursor = Cursor::begin(&mut graph, &history, &visibility.mask, level)
                    .map_err(anyhow::Error::msg)?;
                let mut count = 0;
                while let Some(at) = cursor
                    .advance(&graph, &history)
                    .map_err(anyhow::Error::msg)?
                {
                    let saved = serde_json::to_vec(&(&graph, &history, &cursor))?;
                    (graph, history, cursor) = serde_json::from_slice(&saved)?;
                    ensure!(
                        cursor
                            .advance(&graph, &history)
                            .map_err(anyhow::Error::msg)?
                            == Some(at)
                    );
                    cursor
                        .acknowledge(&graph, &history, at)
                        .map_err(anyhow::Error::msg)?;
                    count += 1;
                }
                ensure!(cursor.complete());
                let clear_restores = clear_and_restore(&mut visibility, lot.size, level)?;
                camera_passes.push(serde_json::json!({
                    "rotation":rotation,"level":level,"callback_restores":count,
                    "projection_restores":projection_restores,"clear_restores":clear_restores,
                    "visible_rooms":view.history.ids()
                }));
            }
        }
        let lifecycle_passes = exercise_view_lifecycle(
            lot,
            &s.simulation
                .room_environment
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("imported room manager absent"))?
                .manager,
            &mask_camera,
            &mut graph,
        )?;
        let scroll_passes = exercise_view_scroll(
            lot,
            &s.simulation.room_environment.as_ref().unwrap().manager,
        )?;
        let transition_passes = camera_transitions::exercise(
            lot,
            &s.simulation.room_environment.as_ref().unwrap().manager,
            &mut graph,
        )?;
        let motion_passes = camera_motion::exercise(
            lot,
            &s.simulation.room_environment.as_ref().unwrap().manager,
        )?;
        ensure!(
            serde_json::to_vec(&s.save())? == before,
            "wall service changed simulation or assets"
        );
        let restored = Session::restore(&index, serde_json::from_slice(&before)?)?;
        let rebuilt = Graph::from_lot(
            restored
                .simulation
                .architecture
                .as_ref()
                .unwrap()
                .original
                .as_ref()
                .unwrap(),
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(rebuilt.geometry == graph.geometry);
        let mut renderer = OriginalHouse::default();
        let mut resumed = OriginalHouse::default();
        for rotation in 0..2 {
            let level = if name == "goth" && rotation == 1 {
                2
            } else {
                1
            };
            let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, level);
            let image = software::render(&renderer.scene(&s, camera)?);
            ensure!(image == software::render(&resumed.scene(&restored, camera)?));
            image::imageops::replace(
                &mut gallery,
                &image,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        report.push(serde_json::json!({"house":name,"corners":graph.state.corners.len(),"edges":graph.state.edges.len(),"pass_counts":pass_counts,"camera_passes":camera_passes,"lifecycle_passes":lifecycle_passes,"scroll_passes":scroll_passes,"transition_passes":transition_passes,"motion_passes":motion_passes,"mask_scope":"native mouse room history, room-mask composition, wall cutaway and mask clearing; original forward/inverse projection, mode/floor/zoom/rotation lifecycle, same-floor immediate centering, terrain extrema, bounds and scrolling with live room-manager masks; viewport and post-base event are explicit, renderer synchronization uses a headless sink; automatic viewer dispatch remains separate","graph_restore_equal":true,"simulation_assets_unchanged":true,"restored_pixel_views":2,"renderer_consumes_native_graph":false}));
        println!(
            "{name}: native two-floor rotation/cutaway and callback restores match; borrowed service leaves full state unchanged; two CPU views identical"
        );
    }
    gallery.save(root.join("cache/original-camera-frame-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-camera-frame-houses.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

fn exercise_view_scroll(
    lot: &sims_sim::lot_geometry::Lot,
    manager: &sims_sim::lot_environment::manager::Manager,
) -> Result<Vec<serde_json::Value>> {
    use sims_sim::lot_presenter::{room_masks, scroll};
    let mut passes = vec![];
    for rotation in 0..4 {
        let extrema = scroll::terrain_extrema(lot.size as i32, [0, 0], rotation, &lot.heights)
            .map_err(anyhow::Error::msg)?;
        for zoom in 1..=3 {
            let mut context = scroll::Context {
                camera: room_masks::Camera::initialized([-317, 203], zoom)
                    .map_err(anyhow::Error::msg)?,
                size: lot.size as i32,
                screen: [0, 0, 1024, 768],
                lot_origin: [0, 0],
                row_step: 4 << zoom,
                maximum: Some(extrema.maximum.clone()),
                minimum: Some(extrema.minimum.clone()),
            };
            let mut state = scroll::State {
                origin: context.camera.origin,
                pending: [0, 0],
                deferred: 0,
                locked: 0,
                attempts: 0,
                bottom_margin: 0,
            };
            for (target, deferred, immediate) in [
                ([512, 384], 0, 0),
                ([-9000, 9000], 0, 1),
                ([9011, -8999], 0, 1),
                ([256, 128], 1, 0),
                ([320, 240], 1, 1),
                ([320, 240], 0, 1),
            ] {
                state.deferred = deferred;
                let b = scroll::bounds(&context).map_err(anyhow::Error::msg)?;
                let mut cursor = scroll::Cursor::begin(
                    &mut state,
                    &context,
                    target,
                    immediate,
                    true,
                    u32::from(rotation),
                )
                .map_err(anyhow::Error::msg)?;
                // Retain pending deltas even when the deferred call has no
                // renderer request. The native renderer is an explicit sink.
                (state, context, cursor) =
                    serde_json::from_slice(&serde_json::to_vec(&(&state, &context, &cursor))?)?;
                let request = cursor.request().cloned();
                if let Some(request) = &request {
                    ensure!(request.origin == state.origin && request.zoom == zoom);
                    cursor.acknowledge(request).map_err(anyhow::Error::msg)?;
                }
                ensure!(cursor.complete());
                context.camera.origin = state.origin;
                let mut masks = std::collections::BTreeMap::new();
                room_masks::refresh(
                    lot,
                    manager,
                    0,
                    rotation,
                    [0, 63, 0, 63],
                    &context.camera,
                    &mut masks,
                )
                .map_err(anyhow::Error::msg)?;
                let mut points = vec![];
                let splits = room_masks::SplitRooms::new();
                for level in 1..=2 {
                    let view = room_masks::View {
                        size: lot.size,
                        rotation,
                        bounds: [0, 63, 0, 63],
                        camera: &context.camera,
                        heights: &lot.heights,
                        walls: &lot.floor(level).unwrap().source.walls,
                        splits: &splits,
                    };
                    points.push(
                        view.fine_point([384, 512], level)
                            .map_err(anyhow::Error::msg)?,
                    );
                }
                let restored: (
                    scroll::State,
                    scroll::Context,
                    std::collections::BTreeMap<u16, sims_sim::lot_presenter::visibility::Room>,
                ) = serde_json::from_slice(&serde_json::to_vec(&(&state, &context, &masks))?)?;
                ensure!(restored == (state.clone(), context.clone(), masks.clone()));
                passes.push(serde_json::json!({"rotation":rotation,"zoom":zoom,"target":target,
                    "deferred":deferred,"immediate":immediate,"bounds":b,"state":state,
                    "request":request,"restored":true,"rooms":masks.len(),"projection_samples":points}));
            }
        }
    }
    Ok(passes)
}

fn exercise_view_lifecycle(
    lot: &sims_sim::lot_geometry::Lot,
    manager: &sims_sim::lot_environment::manager::Manager,
    camera: &sims_sim::lot_presenter::room_masks::Camera,
    graph: &mut sims_sim::lot_presenter::walls::construction::Graph,
) -> Result<Vec<serde_json::Value>> {
    use sims_sim::lot_presenter::{
        hover::{Clear, History},
        lifecycle::{Cursor, Reply, Request, State},
        room_masks, visibility,
        walls::{BASIS, cutaway::Mask},
    };
    let mut passes = vec![];
    for rotation in [0, 1, 2, 3, 0] {
        graph.rotate(rotation).map_err(anyhow::Error::msg)?;
        let mut cache = std::collections::BTreeMap::new();
        room_masks::refresh(
            lot,
            manager,
            0,
            rotation,
            [0, 63, 0, 63],
            camera,
            &mut cache,
        )
        .map_err(anyhow::Error::msg)?;
        let mut state = State {
            level: 1,
            all_walls: 1,
            history: History::house_view(),
            visibility: visibility::State {
                enabled: 1,
                dirty: 0,
                include_visible: 0,
                mask: Mask {
                    first_row: 0,
                    rows: vec![0; 64],
                },
            },
        };
        // Explicit initial room history; the separate mouse check above covers
        // its production. Include real indoor masks before leaving floor one.
        for (&id, _) in manager
            .rooms
            .iter()
            .filter(|(id, r)| **id < 1024 && r.active)
            .take(3)
        {
            state
                .history
                .insert(u32::from(id))
                .map_err(anyhow::Error::msg)?;
        }
        state
            .visibility
            .update(visibility::Inputs {
                size: lot.size as i32,
                rotation,
                bounds: [0, 63, 0, 63],
                visible_level: state.level,
                mode: 2,
                include_visible: 1,
                visible_rooms: &state.history.ids(),
                rooms: &cache,
                selected: None,
            })
            .map_err(anyhow::Error::msg)?;
        for (operation, value, force) in [
            ("floor", 2, 0),
            ("floor", 2, 1),
            ("floor", 1, 0),
            ("floor", 1, 0),
            ("mode", 0, 0),
            ("mode", 0, 0),
            ("mode", 1, 0),
            ("mode", 129, 0),
            ("mode", 1, 0),
        ] {
            let mut cursor = if operation == "floor" {
                Cursor::floor(&state, value, force)
            } else {
                Cursor::cutaway(&mut state, value as u8)
            }
            .map_err(anyhow::Error::msg)?;
            let mut requests = vec![];
            let mut tiles = vec![];
            let mut refreshed = vec![];
            while let Some(request) = cursor.request() {
                let bytes = serde_json::to_vec(&(&*graph, &state, &cursor, &cache))?;
                (*graph, state, cursor, cache) = serde_json::from_slice(&bytes)?;
                cursor.validate(&state).map_err(anyhow::Error::msg)?;
                graph.validate().map_err(anyhow::Error::msg)?;
                ensure!(cursor.request() == Some(request.clone()));
                requests.push(request.clone());
                let reply = match request {
                    Request::WallState { enabled, level } => {
                        graph
                            .state
                            .update(enabled, level, BASIS, |_| {})
                            .map_err(anyhow::Error::msg)?;
                        Reply::Done
                    }
                    Request::ClearMask => {
                        let mut clear =
                            Clear::begin(lot.size, [0, 0]).map_err(anyhow::Error::msg)?;
                        while let Some(p) = clear
                            .advance(&mut state.visibility, state.level)
                            .map_err(anyhow::Error::msg)?
                        {
                            let bytes =
                                serde_json::to_vec(&(&*graph, &state, &cursor, &cache, &clear))?;
                            (*graph, state, cursor, cache, clear) = serde_json::from_slice(&bytes)?;
                            cursor.validate(&state).map_err(anyhow::Error::msg)?;
                            ensure!(cursor.request() == Some(request.clone()));
                            ensure!(
                                clear
                                    .advance(&mut state.visibility, state.level)
                                    .map_err(anyhow::Error::msg)?
                                    == Some(p)
                            );
                            tiles.push(p);
                            clear.acknowledge(p).map_err(anyhow::Error::msg)?;
                        }
                        ensure!(clear.complete());
                        Reply::Done
                    }
                    Request::RefreshRooms { level } => {
                        refreshed.extend(
                            room_masks::refresh(
                                lot,
                                manager,
                                level,
                                rotation,
                                [0, 63, 0, 63],
                                camera,
                                &mut cache,
                            )
                            .map_err(anyhow::Error::msg)?,
                        );
                        Reply::Done
                    }
                    Request::Compose { include_visible } => {
                        state
                            .visibility
                            .update(visibility::Inputs {
                                size: lot.size as i32,
                                rotation,
                                bounds: [0, 63, 0, 63],
                                visible_level: state.level,
                                mode: 2,
                                include_visible,
                                visible_rooms: &state.history.ids(),
                                rooms: &cache,
                                selected: None,
                            })
                            .map_err(anyhow::Error::msg)?;
                        Reply::Done
                    }
                    // Explicit headless presentation sink. This checks call
                    // ordering and retained tokens, not Windows drawing.
                    Request::InvalidateView | Request::FlushSurface { surface: 7 } => Reply::Done,
                    Request::ResolveSurface => Reply::Surface(7),
                    Request::FlushSurface { .. } => {
                        anyhow::bail!("unexpected headless surface token")
                    }
                };
                cursor
                    .acknowledge(&mut state, &request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
            cursor.validate(&state).map_err(anyhow::Error::msg)?;
            ensure!(cursor.complete());
            passes.push(serde_json::json!({"rotation":rotation,"operation":operation,"value":value,"force":force,
                "service_restores":requests.len(),"tile_restores":tiles.len(),"requests":requests,"tiles":tiles,
                "refreshed_rooms":refreshed,"final_level":state.level,"enabled":state.visibility.enabled}));
        }
    }
    Ok(passes)
}

fn clear_and_restore(
    state: &mut sims_sim::lot_presenter::visibility::State,
    size: usize,
    level: i32,
) -> Result<usize> {
    use sims_sim::lot_presenter::hover::Clear;
    let mut clear = Clear::begin(size, [0, 0]).map_err(anyhow::Error::msg)?;
    let mut count = 0;
    while let Some(point) = clear.advance(state, level).map_err(anyhow::Error::msg)? {
        let saved = serde_json::to_vec(&(&*state, &clear))?;
        (*state, clear) = serde_json::from_slice(&saved)?;
        ensure!(clear.advance(state, level).map_err(anyhow::Error::msg)? == Some(point));
        clear.acknowledge(point).map_err(anyhow::Error::msg)?;
        count += 1;
    }
    ensure!(clear.complete() && state.mask.rows.iter().all(|&row| row == 0));
    ensure!(state.include_visible == 0 && state.dirty == 1);
    Ok(count)
}

#[test]
#[ignore = "requires private original houses; explicit presenter service and CPU rendering"]
fn original_presenter_classification_loaded_houses_and_restored_pixels() -> Result<()> {
    use sims_sim::lot_presenter::{Inputs, View};
    use sims_vm::presenter::{Classification, Lists, Order};
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
        let mut s = Session::load(&index, source, Options::default())?;
        let inputs = Inputs {
            force_dynamic: false,
            graphic_ready: s
                .host
                .ordinary
                .presenter_readiness(&s.simulation.vm, &s.program)?,
            // Fresh constructor43189d zero. This fixture has no object-ID
            // debug overlay or imported suspended native presenter state.
            keep_dynamic: s
                .simulation
                .vm
                .world
                .objects
                .keys()
                .map(|&id| (id, 0))
                .collect(),
        };
        let before = s.simulation.vm.clone();
        let mut passes = vec![];
        for rotation in 0..4 {
            s.simulation.vm.world.camera_rotation = rotation;
            s.simulation
                .vm
                .world
                .refresh
                .as_mut()
                .unwrap()
                .visible_level = 2;
            // Explicit invalid-cache input, separate from the automatic F6
            // dispatcher and its preceding zoom/rotation/floor services.
            let mut cache = Order {
                dirty: true,
                rotation: u32::from(rotation),
                objects: vec![],
            };
            {
                let links = &s.simulation.native_lot.as_ref().unwrap().placement.links;
                cache
                    .get(
                        &View::new(&mut s.simulation.vm.world, &s.program, links, &inputs, &[])
                            .map_err(anyhow::Error::msg)?,
                    )
                    .map_err(anyhow::Error::msg)?;
            }
            let mut owner = Classification::default();
            let mut lists = Lists::default();
            let mut invalidations = vec![];
            loop {
                let links = &s.simulation.native_lot.as_ref().unwrap().placement.links;
                let request = owner
                    .advance(
                        &mut View::new(
                            &mut s.simulation.vm.world,
                            &s.program,
                            links,
                            &inputs,
                            &cache.objects,
                        )
                        .map_err(anyhow::Error::msg)?,
                        &mut lists,
                    )
                    .map_err(anyhow::Error::msg)?;
                let Some(id) = request else {
                    break;
                };
                // Retain the complete caller and world before acknowledging.
                let saved = s.save();
                let restored: (Classification, Order, Lists, Inputs) = serde_json::from_slice(
                    &serde_json::to_vec(&(&owner, &cache, &lists, &inputs))?,
                )?;
                owner = restored.0;
                cache = restored.1;
                lists = restored.2;
                ensure!(restored.3 == inputs);
                owner.validate().map_err(anyhow::Error::msg)?;
                s = Session::restore(
                    &index,
                    serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
                )?;
                let links = &s.simulation.native_lot.as_ref().unwrap().placement.links;
                View::new(
                    &mut s.simulation.vm.world,
                    &s.program,
                    links,
                    &inputs,
                    &cache.objects,
                )
                .map_err(anyhow::Error::msg)?
                .invalidate(id)
                .map_err(anyhow::Error::msg)?;
                invalidations.push(id);
                owner.acknowledge(id).map_err(anyhow::Error::msg)?;
            }
            ensure!(owner.complete());
            passes.push(serde_json::json!({"rotation":rotation,"order":cache.objects,"lists":lists,"invalidations":invalidations}));
        }
        let after = &s.simulation.vm;
        ensure!(
            before.tick == after.tick
                && before.rng_state == after.rng_state
                && before.lot_rng_state == after.lot_rng_state
                && before.crt_rng_state == after.crt_rng_state
        );
        ensure!(
            before.threads == after.threads,
            "presenter service changed VM execution"
        );
        let saved = s.save();
        let mut resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        ensure!(s.simulation == resumed.simulation && s.host.snapshot() == resumed.host.snapshot());
        for _ in 0..10 {
            ensure!(s.step()? == sims_runtime::original_session::Progress::Tick);
            ensure!(resumed.step()? == sims_runtime::original_session::Progress::Tick);
        }
        ensure!(s.simulation == resumed.simulation && s.host.snapshot() == resumed.host.snapshot());
        let mut original_renderer = OriginalHouse::default();
        let mut restored_renderer = OriginalHouse::default();
        for rotation in 0..2 {
            let level = if name == "goth" && rotation == 1 {
                2
            } else {
                1
            };
            let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, level);
            let image = software::render(&original_renderer.scene(&s, camera)?);
            ensure!(image == software::render(&restored_renderer.scene(&resumed, camera)?));
            image::imageops::replace(
                &mut gallery,
                &image,
                column as i64 * 960,
                i64::from(rotation) * 720,
            );
        }
        std::fs::write(
            root.join(format!("cache/presenter-{name}-save.json")),
            serde_json::to_vec(&s.save())?,
        )?;
        report.push(serde_json::json!({"house":name,"source":source,"passes":passes,"strict_ticks_after":10,"restored_state_equal":true,"restored_pixel_views":2}));
        println!(
            "{name}: four native classification rotations; +10 strict ticks match restore; two CPU views identical"
        );
    }
    gallery.save(root.join("cache/original-presenter-gallery.png"))?;
    std::fs::write(
        root.join("cache/native-presenter-houses.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private assets and original date/star refresh checkpoints"]
fn original_house_after_indicator_refresh_plays_and_restores_pixels() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(1920, 1440);
    for (row, name) in [
        "original-date-refresh-save.json",
        "original-star-refresh-save.json",
    ]
    .into_iter()
    .enumerate()
    {
        let saved = serde_json::from_slice(&std::fs::read(root.join("cache").join(name))?)?;
        let mut s = Session::restore(&index, saved)?;
        ensure!(s.simulation.native_selection.is_none());
        ensure!(s.simulation.vm.world.indicator_queues.is_empty());
        for _ in 0..10 {
            ensure!(s.step()? == sims_runtime::original_session::Progress::Tick);
        }
        let saved = s.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        let mut render = OriginalHouse::default();
        let mut resumed = OriginalHouse::default();
        for rotation in 0..2 {
            let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, 1);
            let image = software::render(&render.scene(&s, camera)?);
            ensure!(image == software::render(&resumed.scene(&restored, camera)?));
            image::imageops::replace(
                &mut gallery,
                &image,
                i64::from(rotation) * 960,
                row as i64 * 720,
            );
        }
        ensure!(s.simulation == saved.simulation.state && s.host.snapshot() == saved.assets);
    }
    gallery.save(root.join("cache/original-indicator-refresh-gallery.png"))?;
    println!(
        "date/star refresh checkpoints: +10 strict ticks each; four restored CPU household views identical; indicator drawing remains separate"
    );
    Ok(())
}

#[test]
#[ignore = "requires private assets and primary-indicator deletion checkpoint"]
fn original_house_after_ordinary_destruction_plays_and_restores_pixels() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let saved = serde_json::from_slice(&std::fs::read(
        root.join("cache/unowned-ordinary-deletion-save.json"),
    )?)?;
    let mut s = Session::restore(&index, saved)?;
    ensure!(s.simulation.retired_graphics.is_empty() && s.host.ordinary.retirements.is_empty());
    for _ in 0..10 {
        ensure!(s.step()? == sims_runtime::original_session::Progress::Tick);
    }
    let saved = s.save();
    let restored = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
    )?;
    let mut render = OriginalHouse::default();
    let mut resumed = OriginalHouse::default();
    let mut gallery = image::RgbaImage::new(1920, 720);
    for rotation in 0..2 {
        let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, 1);
        let image = software::render(&render.scene(&s, camera)?);
        ensure!(image == software::render(&resumed.scene(&restored, camera)?));
        image::imageops::replace(&mut gallery, &image, i64::from(rotation) * 960, 0);
    }
    ensure!(s.simulation == saved.simulation.state && s.host.snapshot() == saved.assets);
    gallery.save(root.join("cache/ordinary-destruction-gallery.png"))?;
    println!("ordinary destruction checkpoint: +10 strict ticks; two restored CPU views identical");
    Ok(())
}

#[test]
#[ignore = "requires private assets and person/pooled-indicator deletion checkpoints"]
fn original_house_after_selected_deletion_plays_and_renders_identically() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(3840, 1440);
    for (column, name) in [
        "selected-person-deletion-save.json",
        "selected-famous-person-deletion-save.json",
        "unselected-famous-person-deletion-save.json",
        "pooled-star-deletion-save.json",
    ]
    .into_iter()
    .enumerate()
    {
        let saved = serde_json::from_slice(&std::fs::read(root.join("cache").join(name))?)?;
        let mut s = Session::restore(&index, saved)?;
        ensure!(s.simulation.characters.len() == 1);
        let before: Vec<_> = s.simulation.characters.keys().copied().collect();
        for _ in 0..10 {
            ensure!(s.step()? == sims_runtime::original_session::Progress::Tick);
        }
        let created: Vec<_> = s
            .simulation
            .characters
            .keys()
            .filter(|id| !before.contains(id))
            .map(|&id| (id, &s.simulation.vm.world.objects[&id].definition))
            .collect();
        println!("{name}: new original-script people after +10: {created:?}");
        let saved = s.save();
        let resumed = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        ensure!(s.simulation == resumed.simulation && s.host.snapshot() == resumed.host.snapshot());
        let mut render = OriginalHouse::default();
        let mut restored = OriginalHouse::default();
        for rotation in 0..2 {
            let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, 1);
            let image = software::render(&render.scene(&s, camera)?);
            ensure!(image == software::render(&restored.scene(&resumed, camera)?));
            image::imageops::replace(
                &mut gallery,
                &image,
                column as i64 * 960,
                rotation as i64 * 720,
            );
        }
        ensure!(s.simulation == saved.simulation.state && s.host.snapshot() == saved.assets);
    }
    gallery.save(root.join("cache/selected-person-deletion-gallery.png"))?;
    println!(
        "Four person/indicator-deletion fixtures: +10 strict ticks; eight restored CPU views identical"
    );
    Ok(())
}

#[test]
#[ignore = "requires private original houses; star factories and CPU rendering"]
fn native_star_refill_household_restores_and_renders() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut s = Session::load(
        &index,
        "UserData/Houses/House07.iff",
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    let people: Vec<_> = s
        .house
        .household
        .as_ref()
        .unwrap()
        .residents
        .iter()
        .map(|r| r.object as i16)
        .collect();
    let camera = OriginalHouse::camera(&s, 960, 720, 0, 0.55, 1);
    let size = s
        .simulation
        .architecture
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap()
        .size;
    let view = sims_render::dialog::client::Inputs::camera(camera, size as i32)?.view;
    // Explicit fame fixture, separate from unchanged original-house imports.
    s.simulation
        .vm
        .world
        .objects
        .get_mut(&people[0])
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .insert(81, 10);
    for id in people {
        s.begin_selection(Some(id))?;
        let mut done = false;
        for _ in 0..2000 {
            if s.advance_selection(view, 256)? {
                done = true;
                break;
            }
        }
        ensure!(done);
    }
    ensure!(s.simulation.native_indicator_pools.active.len() == 1);
    ensure!(s.simulation.native_indicator_pools.spare.len() == 3);
    for tick in 0..100 {
        if let Err(error) = s.step() {
            let waiting: Vec<_> = s
                .simulation
                .vm
                .threads
                .iter()
                .filter_map(|(id, t)| {
                    matches!(t.status, sims_vm::Status::AwaitingCreation(_))
                        .then_some((id, &t.status))
                })
                .collect();
            std::fs::write(
                repo.join("cache/star-refill-play-stop.json"),
                serde_json::to_vec(&s.save())?,
            )?;
            anyhow::bail!("star household +{tick}: {error}; creation: {waiting:?}");
        }
    }
    let saved = s.save();
    let restored = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
    )?;
    ensure!(s.simulation == restored.simulation && s.host.snapshot() == restored.host.snapshot());
    let mut render = OriginalHouse::default();
    let mut other = OriginalHouse::default();
    let mut gallery = image::RgbaImage::new(1920, 1440);
    for rotation in 0..2 {
        for floor in 1..=2 {
            let camera = OriginalHouse::camera(&s, 960, 720, rotation, 0.55, floor);
            let pixels = software::render(&render.scene(&s, camera)?);
            ensure!(pixels == software::render(&other.scene(&restored, camera)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 960,
                i64::from(floor - 1) * 720,
            );
        }
    }
    ensure!(s.simulation == saved.simulation.state && s.host.snapshot() == saved.assets);
    gallery.save(repo.join("cache/original-star-refill-house-gallery.png"))?;
    println!(
        "Star refill household: +100 explicit inferred ticks; four restored views identical; ordinary marker drawing remains separate"
    );
    Ok(())
}

#[test]
#[ignore = "requires private original houses; native selection and CPU rendering"]
fn native_selection_original_homes_restore_and_render_after_play() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut gallery = image::RgbaImage::new(2880, 1440);
    for (column, source) in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .enumerate()
    {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        let id = session.house.household.as_ref().unwrap().residents[0].object as i16;
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, 1);
        let size = session
            .simulation
            .architecture
            .as_ref()
            .unwrap()
            .original
            .as_ref()
            .unwrap()
            .size;
        let view = sims_render::dialog::client::Inputs::camera(camera, size as i32)?.view;
        session.begin_selection(Some(id))?;
        let mut complete = false;
        for _ in 0..2000 {
            if session.advance_selection(view, 256)? {
                complete = true;
                break;
            }
        }
        ensure!(complete);
        for _ in 0..100 {
            session.step()?;
        }
        let saved = session.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        ensure!(
            session.simulation == restored.simulation
                && session.host.snapshot() == restored.host.snapshot()
        );
        let mut renderer = OriginalHouse::default();
        let mut other = OriginalHouse::default();
        for floor in 1..=2 {
            let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, floor);
            let pixels = software::render(&renderer.scene(&session, camera)?);
            ensure!(pixels == software::render(&other.scene(&restored, camera)?));
            image::imageops::replace(
                &mut gallery,
                &pixels,
                column as i64 * 960,
                (floor as i64 - 1) * 720,
            );
        }
        ensure!(
            session.simulation == saved.simulation.state && session.host.snapshot() == saved.assets
        );
        println!("{source}: selected #{id}, +100 ticks, both floors restore identically");
    }
    gallery.save(repo.join("cache/original-selection-houses-gallery.png"))?;
    Ok(())
}

#[test]
#[ignore = "requires private original houses and assets; CPU headless rendering"]
fn native_household_poses_render_without_advancing_and_restore_pixels() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut sheet = image::RgbaImage::new(2880, 1440);
    for (column, source) in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .enumerate()
    {
        let mut session = Session::load(
            &index,
            source,
            Options {
                routing: sims_sim::lot_route::path::Policy::InferredRectangles,
                ..Default::default()
            },
        )?;
        for _ in 0..100 {
            session.step()?;
        }
        let before = session.simulation.clone();
        let saved = session.save();
        let restored = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
        )?;
        let mut renderer = OriginalHouse::default();
        let mut resumed_renderer = OriginalHouse::default();
        for level in 1..=2 {
            let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, level);
            let scene = renderer.scene(&session, camera)?;
            let frame = software::render(&scene);
            ensure!(
                frame == software::render(&resumed_renderer.scene(&restored, camera)?),
                "{source} restored pixels differ"
            );
            ensure!(
                scene.draws.iter().any(|d| d
                    .object
                    .is_some_and(|id| session.simulation.characters.contains_key(&id))),
                "native residents missing from scene"
            );
            image::imageops::replace(
                &mut sheet,
                &frame,
                column as i64 * 960,
                (level as i64 - 1) * 720,
            );
        }
        ensure!(
            session.simulation == before && session.host.snapshot() == saved.assets,
            "rendering mutated live state"
        );
        println!("{source}: native poses at +100, both floors, restored pixels identical");
    }
    sheet.save(repo.join("cache/original-live-houses-gallery.png"))?;
    Ok(())
}

#[test]
#[ignore = "requires private original house assets; live graphics and retirement boundaries"]
fn mashuga_live_scene_survives_appearance_changes_and_character_retirement() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut session = Session::load(
        &index,
        "UserData2/Houses/House09.iff",
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    let start = session.simulation.vm.tick;
    let mut renderer = OriginalHouse::default();
    let mut sheet = image::RgbaImage::new(2880, 1440);
    for (i, tick) in [0, 100, 1585, 1946, 1993, 2703].into_iter().enumerate() {
        while session.simulation.vm.tick - start < tick {
            ensure!(session.step()? == sims_runtime::original_session::Progress::Tick);
        }
        let before = session.simulation.clone();
        let assets = session.host.snapshot();
        let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.55, 1);
        let scene = renderer.scene(&session, camera)?;
        let frame = software::render(&scene);
        ensure!(session.simulation == before && session.host.snapshot() == assets);
        image::imageops::replace(
            &mut sheet,
            &frame,
            (i % 3) as i64 * 960,
            (i / 3) as i64 * 720,
        );
        println!(
            "Mashuga +{tick}: cached scene renders {} native characters",
            session.simulation.characters.len()
        );
    }
    sheet.save(repo.join("cache/original-mashuga-live-scene-gallery.png"))?;
    Ok(())
}

#[test]
#[ignore = "requires an explicit counterfactual replay capture after live person Reset"]
fn original_experiment_after_reset_restores_and_renders_a_live_house() -> Result<()> {
    use sims_runtime::original_session::{Saved, View};
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var(
        "SIMS_NEIGHBORHOOD_EXPERIMENT",
    )?)?)?;
    let label = input["label"].as_str().unwrap_or_default();
    ensure!(
        !label.is_empty()
            && label.len() <= 64
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "invalid experiment label"
    );
    let directory = repo
        .join("cache/neighborhood-input-experiments")
        .join(label);
    let capture: serde_json::Value = serde_json::from_slice(&std::fs::read(
        directory.join("original-live-character-final.json"),
    )?)?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(
        directory.join("original-live-character-prefix-inferred.json"),
    )?)?;
    ensure!(
        capture["neighborhood_input_experiment"] == input
            && report[0]["neighborhood_input_experiment"] == input
            && capture["source"] == input["source"]
            && report[0]["completed_live_resets"]
                .as_array()
                .is_some_and(|ids| !ids.is_empty()),
        "capture lacks matching experiment provenance or a completed Reset"
    );
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let source = capture["source"].as_str().unwrap();
    let house = sims_content::saves::House::load(&index, source)?;
    // This conversion is test-only. Keep both capture and images under the
    // explicit experiment label; never publish it as an ordinary original save.
    // Restore reconstructs modules/clips and verifies the captured fingerprint.
    let saved = Saved {
        version: 1,
        managed: None,
        source: source.into(),
        source_hashes: house.sources,
        options: Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            ..Default::default()
        },
        modules: serde_json::from_value(capture["program_modules"].clone())?,
        normalized_object_definitions: Default::default(),
        animations: serde_json::from_value(capture["program_animations"].clone())?,
        native_animation_absences: Default::default(),
        headline_catalog: false,
        simulation: serde_json::from_value(capture["simulation"].clone())?,
        assets: serde_json::from_value(capture["assets"].clone())?,
        view: View::default(),
    };
    ensure!(saved.assets.clothing.loaded(), "live clothing cache absent");
    let mut session = Session::restore(&index, saved.clone())?;
    let mut resumed = Session::restore(
        &index,
        serde_json::from_slice(&serde_json::to_vec(&session.save())?)?,
    )?;
    let simulation = serde_json::to_value(&saved.simulation)?;
    ensure!(
        serde_json::to_value(&session.save().simulation)? == simulation
            && session.host.snapshot() == saved.assets,
        "fresh-program restoration changed the diagnostic state"
    );
    let mut renderer = OriginalHouse::default();
    let mut restored_renderer = OriginalHouse::default();
    let mut gallery = image::RgbaImage::new(3840, 1440);
    for level in 1..=2 {
        for rotation in 0..4 {
            let camera = OriginalHouse::camera(&session, 960, 720, rotation, 0.55, level);
            let pixels = software::render(&renderer.scene(&session, camera)?);
            ensure!(
                pixels == software::render(&restored_renderer.scene(&resumed, camera)?),
                "post-Reset house pixels differ after restore"
            );
            pixels.save(directory.join(format!("live-house-{level}-{rotation}.png")))?;
            image::imageops::replace(
                &mut gallery,
                &pixels,
                i64::from(rotation) * 960,
                i64::from(level - 1) * 720,
            );
        }
    }
    ensure!(
        serde_json::to_value(&session.save().simulation)? == simulation
            && session.host.snapshot() == saved.assets,
        "rendering advanced the live household"
    );
    // A stopped frontier is a valid outcome here: both restored sessions must
    // expose the same next result/error and retain the same complete state.
    for _ in 0..3 {
        let actual = session.step().map_err(|e| format!("{e:#}"));
        let expected = resumed.step().map_err(|e| format!("{e:#}"));
        ensure!(actual == expected, "restored continuation differs");
        ensure!(
            session.simulation == resumed.simulation
                && session.host.snapshot() == resumed.host.snapshot(),
            "post-Reset session processing differs after restore"
        );
        if actual.is_err() || actual == Ok(sims_runtime::original_session::Progress::Dialog) {
            break;
        }
    }
    gallery.save(directory.join("live-house-gallery.png"))?;
    println!(
        "{label}: counterfactual post-Reset live house, eight restored views and fresh-program continuation match; not normal original-house progress"
    );
    Ok(())
}
