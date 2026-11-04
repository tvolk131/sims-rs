//! Real-house composition with explicit clock inputs and headless graphics/UI.
//! Frame preparation and pending-displacement owners run with headless services.
//! Clock, ordinary drawing and its unlock boundary remain explicit test inputs.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    lot_environment::manager::Manager,
    lot_geometry::Lot,
    lot_presenter::{
        centering,
        hover::{Clear, History},
        lifecycle, motion,
        room_masks::{self, Camera, SplitRooms, View},
        scroll, visibility,
        walls::{BASIS, construction::Graph, cutaway::Mask},
    },
};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
struct Host {
    center: centering::State,
    curves: motion::State,
    floor: lifecycle::State,
    scroll: scroll::State,
    graph: Graph,
    masks: BTreeMap<u16, visibility::Room>,
    extrema: scroll::Extrema,
    rotation: u8,
}
#[derive(Default, Serialize)]
struct Counts {
    center: usize,
    floor: usize,
    clear: usize,
    scroll: usize,
    cancel: usize,
    samples: usize,
    frame: usize,
    displacement: usize,
    frame_sync: usize,
}
impl Host {
    fn camera(&self) -> Result<Camera> {
        Camera::initialized(self.center.origin, self.center.zoom as u8).map_err(anyhow::Error::msg)
    }
    fn refresh(&mut self, lot: &Lot, manager: &Manager, level: i32) -> Result<()> {
        room_masks::refresh(
            lot,
            manager,
            level,
            self.rotation,
            self.center.bounds,
            &self.camera()?,
            &mut self.masks,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    fn scroll(&mut self, target: [i32; 2], immediate: u8, counts: &mut Counts) -> Result<()> {
        self.scroll.origin = self.center.origin;
        self.scroll.pending = self.center.pending;
        let context = scroll::Context {
            camera: self.camera()?,
            size: 64,
            screen: self.center.viewport,
            lot_origin: [0, 0],
            row_step: 4 << self.center.zoom,
            maximum: Some(self.extrema.maximum.clone()),
            minimum: Some(self.extrema.minimum.clone()),
        };
        let mut cursor = scroll::Cursor::begin(
            &mut self.scroll,
            &context,
            target,
            immediate,
            true,
            u32::from(self.rotation),
        )
        .map_err(anyhow::Error::msg)?;
        let saved = serde_json::to_vec(&(&self, &cursor))?;
        let (host, resumed): (Self, scroll::Cursor) = serde_json::from_slice(&saved)?;
        ensure!(serde_json::to_vec(&(&host, &resumed))? == saved);
        *self = host;
        cursor = resumed;
        if let Some(request) = cursor.request().cloned() {
            counts.scroll += 1;
            cursor.acknowledge(&request).map_err(anyhow::Error::msg)?;
        }
        ensure!(cursor.complete());
        self.center.origin = self.scroll.origin;
        self.center.pending = self.scroll.pending;
        Ok(())
    }
    fn change_floor(
        &mut self,
        lot: &Lot,
        manager: &Manager,
        level: i32,
        parent: &centering::Cursor,
        counts: &mut Counts,
    ) -> Result<()> {
        let mut cursor =
            lifecycle::Cursor::floor(&self.floor, level, 0).map_err(anyhow::Error::msg)?;
        while let Some(request) = cursor.request() {
            let saved = serde_json::to_vec(&(&self, &cursor, parent))?;
            let (host, resumed, restored_parent): (Self, lifecycle::Cursor, centering::Cursor) =
                serde_json::from_slice(&saved)?;
            ensure!(
                serde_json::to_vec(&(&host, &resumed, &restored_parent))? == saved
                    && restored_parent == *parent
            );
            *self = host;
            cursor = resumed;
            counts.floor += 1;
            let reply = match request {
                lifecycle::Request::WallState { enabled, level } => {
                    self.graph
                        .state
                        .update(enabled, level, BASIS, |_| {})
                        .map_err(anyhow::Error::msg)?;
                    lifecycle::Reply::Done
                }
                lifecycle::Request::ClearMask => {
                    let mut clear = Clear::begin(lot.size, [0, 0]).map_err(anyhow::Error::msg)?;
                    while let Some(p) = clear
                        .advance(&mut self.floor.visibility, self.floor.level)
                        .map_err(anyhow::Error::msg)?
                    {
                        let saved = serde_json::to_vec(&(&self, &cursor, parent, &clear))?;
                        let (host, resumed, restored_parent, restored_clear): (
                            Self,
                            lifecycle::Cursor,
                            centering::Cursor,
                            Clear,
                        ) = serde_json::from_slice(&saved)?;
                        ensure!(
                            serde_json::to_vec(&(
                                &host,
                                &resumed,
                                &restored_parent,
                                &restored_clear
                            ))? == saved
                                && restored_parent == *parent
                        );
                        *self = host;
                        cursor = resumed;
                        clear = restored_clear;
                        ensure!(
                            clear
                                .advance(&mut self.floor.visibility, self.floor.level)
                                .map_err(anyhow::Error::msg)?
                                == Some(p)
                        );
                        clear.acknowledge(p).map_err(anyhow::Error::msg)?;
                        counts.clear += 1;
                    }
                    ensure!(clear.complete());
                    lifecycle::Reply::Done
                }
                lifecycle::Request::RefreshRooms { level } => {
                    self.refresh(lot, manager, level)?;
                    lifecycle::Reply::Done
                }
                lifecycle::Request::Compose { include_visible } => {
                    self.floor
                        .visibility
                        .update(visibility::Inputs {
                            size: 64,
                            rotation: self.rotation,
                            bounds: self.center.bounds,
                            visible_level: self.floor.level,
                            mode: 2,
                            include_visible,
                            visible_rooms: &self.floor.history.ids(),
                            rooms: &self.masks,
                            selected: None,
                        })
                        .map_err(anyhow::Error::msg)?;
                    lifecycle::Reply::Done
                }
                lifecycle::Request::ResolveSurface => lifecycle::Reply::Surface(7),
                lifecycle::Request::InvalidateView | lifecycle::Request::FlushSurface { .. } => {
                    lifecycle::Reply::Done
                }
            };
            cursor
                .acknowledge(&mut self.floor, &request, reply)
                .map_err(anyhow::Error::msg)?;
            self.center.level = self.floor.level;
        }
        ensure!(cursor.complete() && self.center.level == level);
        Ok(())
    }
    fn center(
        &mut self,
        lot: &Lot,
        manager: &Manager,
        point: [i32; 2],
        level: i32,
        style: u8,
        immediate: u8,
        now: u32,
        counts: &mut Counts,
    ) -> Result<()> {
        let mut cursor = centering::Cursor::begin(&self.center, point, level, style, immediate);
        while let Some(request) = cursor.request() {
            let saved = serde_json::to_vec(&(&self, &cursor))?;
            let (host, resumed): (Self, centering::Cursor) = serde_json::from_slice(&saved)?;
            ensure!(serde_json::to_vec(&(&host, &resumed))? == saved);
            *self = host;
            cursor = resumed;
            counts.center += 1;
            let reply = match request {
                centering::Request::ChangeFloor { level, flag } => {
                    ensure!(flag == 0);
                    self.change_floor(lot, manager, level, &cursor, counts)?;
                    centering::Reply::Done
                }
                centering::Request::RefreshFloorUi { level, flag } => {
                    ensure!(level == self.center.level && flag == 1);
                    centering::Reply::Done
                }
                centering::Request::Project { point, level, zoom } => {
                    ensure!(zoom == self.center.zoom);
                    let camera = self.camera()?;
                    let splits = SplitRooms::new();
                    let view = View {
                        size: lot.size,
                        rotation: self.rotation,
                        bounds: self.center.bounds,
                        camera: &camera,
                        heights: &lot.heights,
                        walls: &lot.floor(level).unwrap().source.walls,
                        splits: &splits,
                    };
                    centering::Reply::Screen(
                        view.fine_screen(point, level).map_err(anyhow::Error::msg)?,
                    )
                }
                centering::Request::Scroll {
                    position,
                    immediate,
                } => {
                    self.scroll(position, immediate, counts)?;
                    centering::Reply::Done
                }
                centering::Request::ConfigureMotion { .. }
                | centering::Request::SampleMotion { .. } => self
                    .curves
                    .center_service(&mut self.center, &request, now)
                    .map_err(anyhow::Error::msg)?,
            };
            cursor
                .acknowledge(&mut self.center, &request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(cursor.complete());
        Ok(())
    }
    fn samples(&mut self, ticks: &[u32], counts: &mut Counts) -> Result<Vec<[i32; 2]>> {
        let mut points = Vec::new();
        for &now in ticks {
            let before = self.curves.clone();
            let saved = serde_json::to_vec(&self)?;
            *self = serde_json::from_slice(&saved)?;
            ensure!(serde_json::to_vec(&self)? == saved);
            let mut uninterrupted = before;
            let point = [
                self.curves
                    .sample_axis(0, now)
                    .map_err(anyhow::Error::msg)?,
                self.curves
                    .sample_axis(1, now)
                    .map_err(anyhow::Error::msg)?,
            ];
            ensure!(
                point
                    == [
                        uninterrupted
                            .sample_axis(0, now)
                            .map_err(anyhow::Error::msg)?,
                        uninterrupted
                            .sample_axis(1, now)
                            .map_err(anyhow::Error::msg)?
                    ]
                    && self.curves == uninterrupted
            );
            self.center.motion_active = self.curves.active;
            counts.samples += 1;
            points.push(point);
            // Explicit host-requested scroll of the sampled point. Automatic
            // native frame timing/calls are not inferred from this composition.
            self.scroll(point, 1, counts)?;
        }
        Ok(points)
    }
    fn cancel(&mut self, counts: &mut Counts) -> Result<()> {
        let curves = [self.curves.x, self.curves.y];
        let mut cursor = motion::cancel::Cursor::begin(&mut self.center);
        if !cursor.complete() {
            self.curves.cancel();
        }
        while let Some(request) = cursor.request().cloned() {
            let saved = serde_json::to_vec(&(&self, &cursor))?;
            let (host, resumed): (Self, motion::cancel::Cursor) = serde_json::from_slice(&saved)?;
            ensure!(serde_json::to_vec(&(&host, &resumed))? == saved);
            *self = host;
            cursor = resumed;
            counts.cancel += 1;
            let reply = match request {
                motion::cancel::Request::ResolveSurface => motion::cancel::Reply::Surface(7),
                _ => motion::cancel::Reply::Done,
            };
            cursor
                .acknowledge(&request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(
            cursor.complete()
                && self.curves.active == self.center.motion_active
                && curves == [self.curves.x, self.curves.y]
        );
        Ok(())
    }
    fn frame(&mut self, now: u32, counts: &mut Counts) -> Result<serde_json::Value> {
        use motion::frame::{Cursor, Reply, Request, displacement};
        let mut cursor = Cursor::begin(&mut self.center);
        while let Some(request) = cursor.request().cloned() {
            let saved = serde_json::to_vec(&(&self, &cursor))?;
            let (host, resumed): (Self, Cursor) = serde_json::from_slice(&saved)?;
            ensure!(serde_json::to_vec(&(&host, &resumed))? == saved);
            *self = host;
            cursor = resumed;
            counts.frame += 1;
            let reply = match request {
                Request::SampleMotion { axis } => {
                    match self
                        .curves
                        .center_service(
                            &mut self.center,
                            &centering::Request::SampleMotion { axis },
                            now,
                        )
                        .map_err(anyhow::Error::msg)?
                    {
                        centering::Reply::Coordinate(value) => Reply::Coordinate(value),
                        _ => anyhow::bail!("unexpected frame sample reply"),
                    }
                }
                Request::Scroll {
                    position,
                    immediate,
                } => {
                    ensure!(immediate == 0);
                    self.scroll.deferred = self.center.motion_allowed;
                    self.scroll.locked = self.center.motion_disabled;
                    self.scroll(position, immediate, counts)?;
                    Reply::Done
                }
                Request::CancelMotion => {
                    self.cancel(counts)?;
                    Reply::Done
                }
                Request::Synchronize => {
                    counts.frame_sync += 1;
                    Reply::Done
                }
            };
            cursor
                .acknowledge(&mut self.center, &request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(cursor.complete() && self.center.motion_disabled == 1);
        let origin = self.center.origin;
        let pending = self.center.pending;
        let mut passes = displacement::Passes { count: 1, phase: 0 };
        if pending != [0, 0] {
            let mut draw = displacement::Cursor::begin(&self.center).map_err(anyhow::Error::msg)?;
            while let Some(request) = draw.request().cloned() {
                let saved = serde_json::to_vec(&(&self, &cursor, &draw, &passes))?;
                let (host, restored_frame, restored_draw, restored_passes): (
                    Self,
                    Cursor,
                    displacement::Cursor,
                    displacement::Passes,
                ) = serde_json::from_slice(&saved)?;
                ensure!(
                    serde_json::to_vec(&(
                        &host,
                        &restored_frame,
                        &restored_draw,
                        &restored_passes
                    ))? == saved
                );
                *self = host;
                cursor = restored_frame;
                draw = restored_draw;
                passes = restored_passes;
                counts.displacement += 1;
                if request == displacement::Request::Synchronize {
                    counts.frame_sync += 1;
                }
                let reply = if request == displacement::Request::DrawingEnabled {
                    displacement::Reply::Enabled(1)
                } else {
                    displacement::Reply::Done
                };
                draw.acknowledge(&mut self.center, &mut passes, &request, reply)
                    .map_err(anyhow::Error::msg)?;
            }
            ensure!(draw.complete() && self.center.pending == [0, 0]);
        }
        ensure!(self.center.origin == origin && self.center.motion_disabled == 1);
        let result = serde_json::json!({"clock":now,"origin":origin,"consumed_pending":pending,"passes":passes,
            "active":self.center.motion_active,"moving":self.center.moving,"mode":self.center.camera_mode});
        // The ordinary rendering loop is outside these two recovered blocks.
        // Supply its later unlock as an explicit host boundary, not an invented
        // early cancellation or an implicit side effect of the motion owner.
        self.center.motion_disabled = 0;
        self.scroll.locked = 0;
        self.scroll.origin = self.center.origin;
        self.scroll.pending = self.center.pending;
        Ok(result)
    }
}
pub fn exercise(lot: &Lot, manager: &Manager) -> Result<Vec<serde_json::Value>> {
    let mut passes = Vec::new();
    for rotation in 0..4 {
        for zoom in 1..=3 {
            let mut graph = Graph::from_lot(lot).map_err(anyhow::Error::msg)?;
            graph.rotate(rotation).map_err(anyhow::Error::msg)?;
            let mut host = Host {
                center: centering::State {
                    level: 1,
                    origin: [-317, 203],
                    pending: [13, -27],
                    viewport: [0, 0, 1024, 768],
                    bounds: [0, 63, 0, 63],
                    zoom,
                    graphics_ready: 1,
                    motion_active: 0,
                    motion_policy: 1,
                    motion_duration: 17,
                    duration_input: 17,
                    moving: 0,
                    motion_disabled: 0,
                    motion_allowed: 1,
                    camera_mode: 0,
                    application_value_bits: 1f32.to_bits(),
                    saved_value_bits: 0,
                },
                curves: motion::State {
                    x: [0; 4],
                    y: [0; 4],
                    active: 0,
                    policy: 1,
                    duration: 17,
                    speed_bits: 1f32.to_bits(),
                    rate_bits: 0,
                },
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
                scroll: scroll::State {
                    origin: [-317, 203],
                    pending: [13, -27],
                    deferred: 0,
                    locked: 0,
                    attempts: 0,
                    bottom_margin: 0,
                },
                graph,
                masks: BTreeMap::new(),
                extrema: scroll::terrain_extrema(64, [0, 0], rotation, &lot.heights)
                    .map_err(anyhow::Error::msg)?,
                rotation,
            };
            host.refresh(lot, manager, 0)?;
            for (&id, _) in manager
                .rooms
                .iter()
                .filter(|(id, r)| **id < 1024 && r.active)
                .take(3)
            {
                host.floor
                    .history
                    .insert(u32::from(id))
                    .map_err(anyhow::Error::msg)?;
            }
            host.floor
                .visibility
                .update(visibility::Inputs {
                    size: 64,
                    rotation,
                    bounds: [0, 63, 0, 63],
                    visible_level: 1,
                    mode: 2,
                    include_visible: 1,
                    visible_rooms: &host.floor.history.ids(),
                    rooms: &host.masks,
                    selected: None,
                })
                .map_err(anyhow::Error::msg)?;
            let mut counts = Counts::default();
            host.center(lot, manager, [512, 512], 2, 1, 0, 100, &mut counts)?;
            let first = host.samples(&[100, 104, 108], &mut counts)?;
            ensure!(host.curves.active == 1);
            host.center(lot, manager, [520, 504], 1, 1, 0, 108, &mut counts)?;
            let retargeted = host.samples(&[108, 112], &mut counts)?;
            host.cancel(&mut counts)?;
            host.center(lot, manager, [504, 520], 2, 0, 0, 120, &mut counts)?;
            let instant = host.samples(&[120, 121], &mut counts)?;
            ensure!(instant[0] == instant[1] && host.curves.active == 0);
            host.cancel(&mut counts)?;
            host.center(lot, manager, [0, 0], 1, 1, 1, 124, &mut counts)?;
            ensure!(
                host.center.camera_mode == 0
                    && host.center.moving == 0
                    && host.center.application_value_bits == 1f32.to_bits()
            );
            host.center(lot, manager, [512, 512], 2, 1, 0, 200, &mut counts)?;
            let mut frames = vec![host.frame(200, &mut counts)?, host.frame(208, &mut counts)?];
            host.center(lot, manager, [520, 504], 1, 1, 0, 208, &mut counts)?;
            frames.push(host.frame(217, &mut counts)?);
            frames.push(host.frame(225, &mut counts)?);
            ensure!(
                host.center.motion_active == 0
                    && host.center.moving == 1
                    && host.center.camera_mode == 1
            );
            frames.push(host.frame(226, &mut counts)?);
            ensure!(
                host.center.moving == 0
                    && host.center.camera_mode == 0
                    && host.center.pending == [0, 0]
            );
            passes.push(serde_json::json!({"rotation":rotation,"zoom":zoom,"counts":counts,"first":first,
                "retargeted":retargeted,"instant":instant,"final_level":host.center.level,"final_origin":host.center.origin,
                "frames":frames,"restored":true,"native_frame_camera_stages":true,
                "automatic_full_frame_dispatch":false,"ordinary_draw_unlock":"explicit host boundary"}));
        }
    }
    Ok(passes)
}
