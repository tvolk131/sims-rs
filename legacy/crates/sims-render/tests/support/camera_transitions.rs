//! Borrowed original-house integration. Graphics/UI requests terminate at
//! explicit headless sinks; this does not claim automatic viewer dispatch.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    lot_environment::manager::Manager,
    lot_geometry::Lot,
    lot_presenter::{
        centering,
        hover::History,
        room_masks::{self, Camera, SplitRooms, View},
        scroll,
        transition::{Cursor, Reply, Request, State},
        visibility,
        walls::{construction::Graph, cutaway::Mask},
    },
};
use std::collections::BTreeMap;

const VIEWPORT: [i32; 4] = [0, 0, 1024, 768];
#[derive(Serialize, Deserialize)]
struct Retained {
    state: State,
    scroll: scroll::State,
    extrema: scroll::Extrema,
    masks: BTreeMap<u16, visibility::Room>,
    visibility: visibility::State,
    history: History,
    // Presentation plane only; the borrowed source lot remains read-only.
    tiles: Vec<u16>,
}
impl Retained {
    fn camera(&self) -> Result<Camera> {
        let mut camera =
            Camera::initialized(self.state.origin, self.state.zoom).map_err(anyhow::Error::msg)?;
        camera.tile_width = self.state.widths[0];
        camera.half_width = self.state.widths[1];
        camera.bottom = self.state.widths[2];
        Ok(camera)
    }
    fn refresh(&mut self, lot: &Lot, manager: &Manager, level: i32) -> Result<()> {
        room_masks::refresh(
            lot,
            manager,
            level,
            self.state.world_rotation as u8,
            self.state.world_bounds,
            &self.camera()?,
            &mut self.masks,
        )
        .map(|_| ())
        .map_err(anyhow::Error::msg)
    }
    fn scroll(&mut self, target: [i32; 2], immediate: u8) -> Result<usize> {
        self.scroll.origin = self.state.origin;
        self.scroll.pending = self.state.pending;
        let context = scroll::Context {
            camera: self.camera()?,
            size: self.state.world_size,
            screen: VIEWPORT,
            lot_origin: [0, 0],
            row_step: self.state.widths[3],
            maximum: Some(self.extrema.maximum.clone()),
            minimum: Some(self.extrema.minimum.clone()),
        };
        let mut cursor = scroll::Cursor::begin(
            &mut self.scroll,
            &context,
            target,
            immediate,
            self.state.graphics_enabled,
            self.state.world_rotation,
        )
        .map_err(anyhow::Error::msg)?;
        // Suspend at the renderer boundary with the complete enclosing state.
        let saved = serde_json::to_vec(&(&self, &cursor, &context))?;
        let (restored, resumed, restored_context): (Self, scroll::Cursor, scroll::Context) =
            serde_json::from_slice(&saved)?;
        ensure!(serde_json::to_vec(&(&restored, &resumed, &restored_context))? == saved);
        *self = restored;
        cursor = resumed;
        let restored_count = usize::from(cursor.request().is_some());
        if let Some(request) = cursor.request().cloned() {
            ensure!(request.origin == self.scroll.origin);
            cursor.acknowledge(&request).map_err(anyhow::Error::msg)?;
        }
        ensure!(cursor.complete());
        self.state.origin = self.scroll.origin;
        self.state.pending = self.scroll.pending;
        Ok(restored_count)
    }
}

pub fn exercise(lot: &Lot, manager: &Manager, graph: &mut Graph) -> Result<Vec<serde_json::Value>> {
    ensure!(
        lot.size == 64,
        "this integration supplies the full-size world rotation service"
    );
    let mut passes = Vec::new();
    for level in 1..=2 {
        graph.rotate(0).map_err(anyhow::Error::msg)?;
        let initial_tiles = &lot.floor(1).unwrap().source.coverings;
        let mut r = Retained {
            state: State {
                origin: [-317, 203],
                pending: [0, 0],
                zoom: 1,
                widths: [32, 16, 16, 8],
                level,
                world_rotation: 0,
                display_rotation: 0,
                world_size: 64,
                world_bounds: [0, 63, 0, 63],
                camera_mode: 0,
                moving: 0,
                skip_edges: 0,
                cutaway: 1,
                include_visible: 1,
                graphics_enabled: true,
                ui_present: false,
                // Explicit renderer inputs, not recovered graphics defaults.
                depth_scales: [0, 1f32.to_bits(), 2f32.to_bits(), 4f32.to_bits()],
            },
            scroll: scroll::State {
                origin: [-317, 203],
                pending: [0, 0],
                deferred: 0,
                locked: 0,
                attempts: 0,
                bottom_margin: 0,
            },
            extrema: scroll::terrain_extrema(64, [0, 0], 0, &lot.heights)
                .map_err(anyhow::Error::msg)?,
            masks: BTreeMap::new(),
            visibility: visibility::State {
                enabled: 1,
                dirty: 0,
                include_visible: 1,
                mask: Mask {
                    first_row: 0,
                    rows: vec![0; 64],
                },
            },
            history: History::house_view(),
            tiles: initial_tiles.clone(),
        };
        r.refresh(lot, manager, 0)?;
        // Explicit initial history; mouse production is checked separately.
        for (&id, _) in manager
            .rooms
            .iter()
            .filter(|(id, room)| room.active && i32::from(**id / 1024) == level - 1)
            .take(3)
        {
            r.history
                .insert(u32::from(id))
                .map_err(anyhow::Error::msg)?;
        }
        for zoom in 1u8..=3 {
            for (rotation, value, force) in [
                (false, u32::from(zoom), 1),
                (false, u32::from(zoom), 0),
                (true, 1, 0),
                (true, 2, 0),
                (true, 3, 0),
                (true, 0, 0),
                (true, 0, 1),
                (true, 0, 0),
            ] {
                let mut cursor = if rotation {
                    Cursor::rotation(&r.state, value, force, 0)
                } else {
                    Cursor::zoom(&r.state, value as u8, force, 0)
                }
                .map_err(anyhow::Error::msg)?;
                let mut requests = Vec::new();
                let mut nested_scroll_restores = 0;
                let mut centering_restores = 0;
                while let Some(request) = cursor.request() {
                    let saved = serde_json::to_vec(&(&r, &cursor, &graph))?;
                    let (restored, resumed, restored_graph): (Retained, Cursor, Graph) =
                        serde_json::from_slice(&saved)?;
                    ensure!(serde_json::to_vec(&(&restored, &resumed, &restored_graph))? == saved);
                    r = restored;
                    cursor = resumed;
                    *graph = restored_graph;
                    cursor.validate(&r.state).map_err(anyhow::Error::msg)?;
                    ensure!(cursor.request() == Some(request.clone()));
                    let camera = r.camera()?;
                    let splits = SplitRooms::new();
                    let view = View {
                        size: lot.size,
                        rotation: r.state.world_rotation as u8,
                        bounds: r.state.world_bounds,
                        camera: &camera,
                        heights: &lot.heights,
                        walls: &lot.floor(level).unwrap().source.walls,
                        splits: &splits,
                    };
                    let reply = match request {
                        Request::ResolveCenter => Reply::Point(
                            view.fine_point([384, 512], level)
                                .map_err(anyhow::Error::msg)?,
                        ),
                        Request::Center {
                            point,
                            level: requested,
                            style,
                            immediate,
                        } => {
                            ensure!(requested == level && style == 0 && immediate == 1);
                            let mut center_state = centering::State {
                                level,
                                origin: r.state.origin,
                                pending: r.state.pending,
                                viewport: VIEWPORT,
                                bounds: r.state.world_bounds,
                                zoom: u32::from(r.state.zoom),
                                graphics_ready: 1,
                                motion_active: 0,
                                motion_policy: 0,
                                motion_duration: 0,
                                duration_input: 17,
                                moving: 0,
                                motion_disabled: 0,
                                motion_allowed: 1,
                                camera_mode: 0,
                                application_value_bits: 1f32.to_bits(),
                                saved_value_bits: 0,
                            };
                            let mut center = centering::Cursor::begin(
                                &center_state,
                                point,
                                level,
                                style,
                                immediate,
                            );
                            while let Some(service) = center.request() {
                                let saved = serde_json::to_vec(&(
                                    &r,
                                    &cursor,
                                    &graph,
                                    &center_state,
                                    &center,
                                ))?;
                                let (rr, cc, gg, ss, pp): (
                                    Retained,
                                    Cursor,
                                    Graph,
                                    centering::State,
                                    centering::Cursor,
                                ) = serde_json::from_slice(&saved)?;
                                ensure!(serde_json::to_vec(&(&rr, &cc, &gg, &ss, &pp))? == saved);
                                r = rr;
                                cursor = cc;
                                *graph = gg;
                                center_state = ss;
                                center = pp;
                                ensure!(center.request() == Some(service.clone()));
                                centering_restores += 1;
                                let answer = match service {
                                    centering::Request::Project {
                                        point,
                                        level: requested,
                                        zoom,
                                    } => {
                                        ensure!(
                                            requested == level && zoom == u32::from(camera.zoom)
                                        );
                                        centering::Reply::Screen(
                                            view.fine_screen(point, level)
                                                .map_err(anyhow::Error::msg)?,
                                        )
                                    }
                                    centering::Request::Scroll {
                                        position,
                                        immediate,
                                    } => {
                                        nested_scroll_restores += r.scroll(position, immediate)?;
                                        center_state.origin = r.state.origin;
                                        center_state.pending = r.state.pending;
                                        centering::Reply::Done
                                    }
                                    _ => anyhow::bail!(
                                        "unexpected floor/motion service in immediate same-floor house transition"
                                    ),
                                };
                                center
                                    .acknowledge(&mut center_state, &service, answer)
                                    .map_err(anyhow::Error::msg)?;
                            }
                            ensure!(center.complete());
                            Reply::Done
                        }
                        Request::Scroll {
                            position,
                            immediate,
                        } => {
                            nested_scroll_restores += r.scroll(position, immediate)?;
                            Reply::Done
                        }
                        Request::RotateWorld { rotation, force } => {
                            ensure!(force == 0);
                            // Full-size lots keep0..63 bounds under rotation.
                            // Smaller-world4791a6 bounds ownership is separate.
                            r.state.world_rotation = rotation & 3;
                            graph
                                .rotate((rotation & 3) as u8)
                                .map_err(anyhow::Error::msg)?;
                            Reply::Done
                        }
                        Request::TerrainExtrema => {
                            r.extrema = scroll::terrain_extrema(
                                64,
                                [0, 0],
                                r.state.world_rotation as u8,
                                &lot.heights,
                            )
                            .map_err(anyhow::Error::msg)?;
                            Reply::Done
                        }
                        Request::RefreshRooms { level } => {
                            r.refresh(lot, manager, level)?;
                            Reply::Done
                        }
                        Request::Compose { include_visible } => {
                            r.visibility.enabled = r.state.cutaway;
                            r.visibility
                                .update(visibility::Inputs {
                                    size: 64,
                                    rotation: r.state.world_rotation as u8,
                                    bounds: r.state.world_bounds,
                                    visible_level: level,
                                    mode: 2,
                                    include_visible,
                                    visible_rooms: &r.history.ids(),
                                    rooms: &r.masks,
                                    selected: None,
                                })
                                .map_err(anyhow::Error::msg)?;
                            r.state.cutaway = r.visibility.enabled;
                            r.state.include_visible = r.visibility.include_visible;
                            Reply::Done
                        }
                        Request::ResolveSurface => Reply::Token(Some(7)),
                        Request::ResolveUi => Reply::Token(None),
                        Request::MotionCoordinate { .. } | Request::CancelMotion => {
                            anyhow::bail!("unexpected motion service in stationary camera fixture")
                        }
                        Request::InitializeLookup { .. }
                        | Request::SynchronizeRenderer
                        | Request::RefreshDrawables
                        | Request::EdgeSurfaces
                        | Request::ResetUi { .. }
                        | Request::InvalidateView
                        | Request::DepthTable { .. }
                        | Request::Broadcast { .. }
                        | Request::RefreshUi
                        | Request::FlushSurface { .. }
                        | Request::FinishRotation => Reply::Done,
                    };
                    requests.push(request.clone());
                    cursor
                        .acknowledge(&mut r.state, &mut r.tiles, &request, reply)
                        .map_err(anyhow::Error::msg)?;
                }
                ensure!(cursor.complete());
                passes.push(serde_json::json!({"level":level,"zoom":zoom,"rotation_operation":rotation,
                    "value":value,"force":force,"requests":requests,"nested_scroll_restores":nested_scroll_restores,
                    "centering_restores":centering_restores,
                    "final_state":r.state,"scroll_attempts":r.scroll.attempts,"restored":true}));
            }
            ensure!(
                r.tiles == *initial_tiles,
                "full rotation cycle changed tile variants"
            );
        }
    }
    Ok(passes)
}
