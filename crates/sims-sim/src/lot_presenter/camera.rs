//! Saved composition of the native camera services called by house-view F6.
//! Projection, scrolling, floor masks and wall records run here. Device/UI
//! operations remain typed services; no successful graphics reply is invented.
use super::{
    centering, hover, lifecycle, motion, open, room_masks, scroll, transition, visibility, walls,
};
use crate::{lot_environment::manager::Manager, lot_geometry::Lot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[path = "camera_motion_frame.rs"]
pub mod motion_frame;
#[path = "portrait_centering.rs"]
pub mod portrait_centering;

/// Native process/view inputs supplied by the client. Shared origin, pending,
/// zoom, widths, level and visibility live only in the enclosing open::State.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    ///View+51. Historical saves keep this unknown until explicitly bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roof: Option<u8>,
    /// Current host inputs; never substituted with the simulation tick.
    pub clock: u32,
    pub application_mode: i32,
    pub viewport: [i32; 4],
    pub bounds: [i32; 4],
    pub world_rotation: u32,
    pub camera_mode: u8,
    pub moving: u8,
    /// View+68 controls whether centering can project a world point.
    pub projection_ready: u32,
    /// Renderer+4, tested by4a7b22/49b0d6 before49b0de.
    pub renderer_ready: bool,
    /// View+1c, distinct from the retained renderer's ready word.
    pub graphics_enabled: bool,
    pub ui_present: bool,
    pub skip_edges: u8,
    pub depth_scales: [u32; 4],
    pub floor_heights: Vec<i32>,
    pub deferred: u8,
    pub locked: u8,
    pub attempts: u32,
    pub bottom_margin: i32,
    pub motion: motion::State,
    pub duration_input: i32,
    pub application_value_bits: u32,
    pub saved_value_bits: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Actual cXWorld+10 for a retained native viewer. Historical authored
    /// profiles used the explicit 64-cell camera domain; absence keeps that
    /// profile rather than inferring a resize from the backing lot arrays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_size: Option<i32>,
    pub settings: Settings,
    pub graph: walls::construction::Graph,
    pub extrema: scroll::Extrema,
    pub rooms: BTreeMap<u16, visibility::Room>,
    /// Original rotation's floor-one presentation variants, never the source file.
    pub tiles: Vec<u16>,
}
pub struct Inputs<'a> {
    ///Fresh presence of process6f2a58 and its+b0 CPState, for4b1459.
    pub roof_controls_present: bool,
    pub lot: &'a Lot,
    pub rooms: &'a Manager,
    pub application_mode: i32,
    pub selected: Option<&'a visibility::Selected>,
    pub clock: u32,
}
impl State {
    pub fn from_lot(
        lot: &Lot,
        manager: &Manager,
        view: &open::State,
        settings: Settings,
    ) -> Result<Self, String> {
        if lot.size != 64 || settings.bounds != [0, 63, 0, 63] || settings.world_rotation > 3 {
            return Err("camera composition requires the original full-size lot bounds".into());
        }
        let mut graph = walls::construction::Graph::from_lot(lot)?;
        graph.rotate(settings.world_rotation as u8)?;
        let mut state = Self {
            world_size: None,
            extrema: scroll::terrain_extrema(
                64,
                [0, 0],
                settings.world_rotation as u8,
                &lot.heights,
            )?,
            graph,
            tiles: lot
                .floor(1)
                .ok_or("camera floor one absent")?
                .source
                .coverings
                .clone(),
            rooms: BTreeMap::new(),
            settings,
        };
        state.validate(view)?;
        room_masks::refresh(
            lot,
            manager,
            0,
            state.settings.world_rotation as u8,
            state.settings.bounds,
            &state.camera(view)?,
            &mut state.rooms,
        )?;
        Ok(state)
    }
    pub fn validate(&self, view: &open::State) -> Result<(), String> {
        view.floor.validate()?;
        self.transition(view)?.validate()?;
        self.graph.validate()?;
        let size = self.world_size.unwrap_or(64);
        if !(4..=64).contains(&size)
            || self.settings.bounds != [0, size - 1, 0, size - 1]
            || self.settings.world_rotation > 3
            || self.graph.rotation != self.settings.world_rotation as u8
            || self.settings.floor_heights.len() != 4
            || self.tiles.len() != 4096
            || self.extrema.maximum.len() != 128
            || self.extrema.minimum.len() != 128
            || self
                .rooms
                .values()
                .any(|r| r.mask.first_row != 0 || r.mask.rows.len() != 64)
        {
            return Err("invalid retained native camera context".into());
        }
        Ok(())
    }
    fn camera(&self, v: &open::State) -> Result<room_masks::Camera, String> {
        let zoom = u8::try_from(v.zoom).map_err(|_| "camera zoom unavailable")?;
        Ok(room_masks::Camera {
            origin: v.origin,
            zoom,
            tile_width: v.widths[0],
            half_width: v.widths[1],
            bottom: v.widths[2],
            floor_heights: self.settings.floor_heights.clone(),
        })
    }
    fn transition(&self, v: &open::State) -> Result<transition::State, String> {
        let s = &self.settings;
        Ok(transition::State {
            origin: v.origin,
            pending: v.pending,
            zoom: u8::try_from(v.zoom).map_err(|_| "camera zoom unavailable")?,
            widths: v.widths,
            level: v.floor.level,
            world_rotation: s.world_rotation,
            display_rotation: v.display_rotation,
            world_size: self.world_size.unwrap_or(64),
            world_bounds: s.bounds,
            camera_mode: s.camera_mode,
            moving: s.moving,
            skip_edges: s.skip_edges,
            cutaway: v.floor.visibility.enabled,
            include_visible: v.floor.visibility.include_visible,
            graphics_enabled: s.graphics_enabled,
            ui_present: s.ui_present,
            depth_scales: s.depth_scales,
        })
    }
    fn publish_transition(&mut self, v: &mut open::State, s: transition::State) {
        v.origin = s.origin;
        v.pending = s.pending;
        v.zoom = u32::from(s.zoom);
        v.widths = s.widths;
        v.floor.level = s.level;
        v.display_rotation = s.display_rotation;
        v.floor.visibility.enabled = s.cutaway;
        v.floor.visibility.include_visible = s.include_visible;
        // Remaining fields are service inputs, never stores by the transition.
    }
    fn center(&self, v: &open::State) -> centering::State {
        let s = &self.settings;
        centering::State {
            level: v.floor.level,
            origin: v.origin,
            pending: v.pending,
            viewport: s.viewport,
            bounds: s.bounds,
            zoom: v.zoom,
            graphics_ready: s.projection_ready,
            motion_active: s.motion.active,
            motion_policy: s.motion.policy,
            motion_duration: s.motion.duration,
            duration_input: s.duration_input,
            moving: s.moving,
            motion_disabled: s.locked,
            motion_allowed: s.deferred,
            camera_mode: s.camera_mode,
            application_value_bits: s.application_value_bits,
            saved_value_bits: s.saved_value_bits,
        }
    }
    fn publish_center(&mut self, v: &mut open::State, s: centering::State) {
        v.origin = s.origin;
        v.pending = s.pending;
        v.floor.level = s.level;
        let a = &mut self.settings;
        a.motion.active = s.motion_active;
        a.motion.policy = s.motion_policy;
        a.motion.duration = s.motion_duration;
        a.moving = s.moving;
        a.locked = s.motion_disabled;
        a.camera_mode = s.camera_mode;
        a.application_value_bits = s.application_value_bits;
        a.saved_value_bits = s.saved_value_bits;
    }
    fn scroll_context(&self, view: &open::State) -> Result<scroll::Context, String> {
        Ok(scroll::Context {
            camera: self.camera(view)?,
            size: 64,
            screen: self.settings.viewport,
            lot_origin: [0, 0],
            row_step: view.widths[3],
            maximum: Some(self.extrema.maximum.clone()),
            minimum: Some(self.extrema.minimum.clone()),
        })
    }
    fn projection<T>(
        &self,
        v: &open::State,
        i: &Inputs<'_>,
        level: i32,
        f: impl FnOnce(&room_masks::View<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let camera = self.camera(v)?;
        let floor = i.lot.floor(level).ok_or("camera projection floor absent")?;
        let splits = room_masks::SplitRooms::new();
        // Fine terrain projection does not query the split-room map.
        f(&room_masks::View {
            size: i.lot.size,
            rotation: self.settings.world_rotation as u8,
            bounds: self.settings.bounds,
            camera: &camera,
            heights: &i.lot.heights,
            walls: &floor.source.walls,
            splits: &splits,
        })
    }
    fn refresh(&mut self, v: &open::State, i: &Inputs<'_>, level: i32) -> Result<(), String> {
        room_masks::refresh(
            i.lot,
            i.rooms,
            level,
            self.settings.world_rotation as u8,
            self.settings.bounds,
            &self.camera(v)?,
            &mut self.rooms,
        )?;
        Ok(())
    }
    /// Original4ab992, also called directly by the viewer's E4 listener.
    /// The same retained room masks and live view fields serve both callers.
    pub fn compose(
        &self,
        v: &mut open::State,
        i: &Inputs<'_>,
        include_visible: u8,
    ) -> Result<(), String> {
        v.floor.visibility.update(visibility::Inputs {
            size: i.lot.size as i32,
            rotation: self.settings.world_rotation as u8,
            bounds: self.settings.bounds,
            visible_level: v.floor.level,
            mode: i.application_mode,
            include_visible,
            visible_rooms: &v.floor.history.ids(),
            rooms: &self.rooms,
            selected: i.selected,
        })
    }
    fn wall(&mut self, enabled: u8, level: i32) -> Result<(), String> {
        self.graph
            .state
            .update(enabled, level, walls::BASIS, |_| {})
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Roof(lifecycle::roof::Request),
    Transition(transition::Request),
    Floor(lifecycle::Request),
    Center(centering::Request),
    Portrait(portrait_centering::Request),
    MotionFrame(motion::frame::Request),
    Displacement(motion::frame::displacement::Request),
    Scroll(scroll::Synchronize),
    Cancel(motion::cancel::Request),
    InvalidateTile([i8; 3]),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Token(Option<u32>),
    Enabled(u8),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Service(Service),
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Action {
    WallsDown(u8),
    Cutaway(u8),
    Roof {
        enabled: u8,
        force: u8,
    },
    Transition(open::Request),
    Floor {
        level: i32,
        force: u8,
    },
    Center {
        point: [i32; 2],
        level: i32,
        style: u8,
        immediate: u8,
    },
    CenterPerson {
        point: [i32; 2],
        level: i32,
    },
    Frame,
    Scroll {
        position: [i32; 2],
        immediate: u8,
    },
    Cancel,
    Clear,
    Wall {
        enabled: u8,
        level: i32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Call {
    Roof(lifecycle::roof::Cursor),
    Transition(transition::Cursor),
    Floor(lifecycle::Cursor),
    Center(centering::Cursor),
    Portrait(portrait_centering::Cursor),
    Frame(motion_frame::Cursor),
    Scroll(scroll::Cursor),
    Cancel(motion::cancel::Cursor),
    Clear(hover::Clear),
    Wall(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Node {
    action: Action,
    call: Call,
    child: Option<Box<Node>>,
}
impl Node {
    fn begin(a: Action, s: &mut State, v: &mut open::State) -> Result<Self, String> {
        let call = match &a {
            Action::WallsDown(enabled) => {
                Call::Floor(lifecycle::Cursor::walls_down(&mut v.floor, *enabled)?)
            }
            Action::Cutaway(enabled) => {
                Call::Floor(lifecycle::Cursor::cutaway(&mut v.floor, *enabled)?)
            }
            Action::Roof { enabled, force } => {
                s.settings.roof.ok_or("roof byte is not bound")?;
                Call::Roof(lifecycle::roof::Cursor::new(
                    u32::from(*enabled),
                    u32::from(*force),
                ))
            }
            Action::Transition(open::Request::Zoom {
                zoom,
                force,
                refresh_ui,
            }) => Call::Transition(transition::Cursor::zoom(
                &s.transition(v)?,
                u8::try_from(*zoom).map_err(|_| "camera zoom unavailable")?,
                *force,
                *refresh_ui,
            )?),
            Action::Transition(open::Request::Rotate {
                rotation,
                force,
                refresh_ui,
            }) => Call::Transition(transition::Cursor::rotation(
                &s.transition(v)?,
                *rotation,
                *force,
                *refresh_ui,
            )?),
            Action::Floor { level, force } => {
                Call::Floor(lifecycle::Cursor::floor(&v.floor, *level, *force)?)
            }
            Action::Center {
                point,
                level,
                style,
                immediate,
            } => Call::Center(centering::Cursor::begin(
                &s.center(v),
                *point,
                *level,
                *style,
                *immediate,
            )),
            Action::CenterPerson { point, level } => {
                Call::Portrait(portrait_centering::Cursor::begin(
                    &s.center(v),
                    *point,
                    *level,
                    2,
                    1,
                    s.settings.world_rotation,
                ))
            }
            Action::Frame => {
                let mut state = s.center(v);
                let cursor = motion_frame::Cursor::begin(&mut state)?;
                s.publish_center(v, state);
                Call::Frame(cursor)
            }
            Action::Scroll {
                position,
                immediate,
            } => {
                let c = s.scroll_context(v)?;
                let mut state = scroll::State {
                    origin: v.origin,
                    pending: v.pending,
                    deferred: s.settings.deferred,
                    locked: s.settings.locked,
                    attempts: s.settings.attempts,
                    bottom_margin: s.settings.bottom_margin,
                };
                let result = scroll::Cursor::begin(
                    &mut state,
                    &c,
                    *position,
                    *immediate,
                    s.settings.renderer_ready,
                    s.settings.world_rotation,
                );
                v.origin = state.origin;
                v.pending = state.pending;
                s.settings.attempts = state.attempts;
                s.settings.bottom_margin = state.bottom_margin;
                Call::Scroll(result?)
            }
            Action::Cancel => {
                let mut state = s.center(v);
                let c = motion::cancel::Cursor::begin(&mut state);
                s.publish_center(v, state);
                Call::Cancel(c)
            }
            Action::Clear => Call::Clear(hover::Clear::begin(64, [0, 0])?),
            Action::Wall { .. } => Call::Wall(false),
            _ => return Err("unsupported camera action".into()),
        };
        Ok(Self {
            action: a,
            call,
            child: None,
        })
    }
    fn complete(&self) -> bool {
        self.child.is_none()
            && match &self.call {
                Call::Roof(c) => c.complete(),
                Call::Transition(c) => c.complete(),
                Call::Floor(c) => c.complete(),
                Call::Center(c) => c.complete(),
                Call::Portrait(c) => c.complete(),
                Call::Frame(c) => c.complete(),
                Call::Scroll(c) => c.complete(),
                Call::Cancel(c) => c.complete(),
                Call::Clear(c) => c.complete(),
                Call::Wall(done) => *done,
            }
    }
    fn child_action(&self) -> Option<Action> {
        match &self.call {
            Call::Transition(c) => match c.request()? {
                transition::Request::CancelMotion => Some(Action::Cancel),
                transition::Request::Scroll {
                    position,
                    immediate,
                } => Some(Action::Scroll {
                    position,
                    immediate,
                }),
                transition::Request::Center {
                    point,
                    level,
                    style,
                    immediate,
                } => Some(Action::Center {
                    point,
                    level,
                    style,
                    immediate,
                }),
                _ => None,
            },
            Call::Floor(c) => match c.request()? {
                lifecycle::Request::ClearMask => Some(Action::Clear),
                _ => None,
            },
            Call::Center(c) => match c.request()? {
                centering::Request::ChangeFloor { level, flag } => {
                    Some(Action::Floor { level, force: flag })
                }
                centering::Request::Scroll {
                    position,
                    immediate,
                } => Some(Action::Scroll {
                    position,
                    immediate,
                }),
                _ => None,
            },
            Call::Portrait(c) => match c.request()? {
                portrait_centering::Request::World {
                    point,
                    level,
                    style,
                    immediate,
                } => Some(Action::Center {
                    point,
                    level,
                    style,
                    immediate,
                }),
                portrait_centering::Request::Scroll {
                    position,
                    immediate,
                } => Some(Action::Scroll {
                    position,
                    immediate,
                }),
                _ => None,
            },
            Call::Frame(c) => match c.request()? {
                motion_frame::Request::Prepare(motion::frame::Request::Scroll {
                    position,
                    immediate,
                }) => Some(Action::Scroll {
                    position,
                    immediate,
                }),
                motion_frame::Request::Prepare(motion::frame::Request::CancelMotion) => {
                    Some(Action::Cancel)
                }
                _ => None,
            },
            _ => None,
        }
    }
    fn validate(&self, s: &State, v: &open::State, depth: usize) -> Result<(), String> {
        if depth > 5 {
            return Err("native camera call depth invalid".into());
        }
        match &self.call {
            Call::Roof(c) => {
                c.validate()?;
                s.settings.roof.ok_or("roof byte is not bound")?;
            }
            Call::Transition(c) => c.validate(&s.transition(v)?)?,
            Call::Floor(c) => c.validate(&v.floor)?,
            Call::Clear(c) => c.validate()?,
            _ => (),
        }
        let compatible = matches!(
            (&self.action, &self.call),
            (Action::Transition(_), Call::Transition(_))
                | (Action::WallsDown(_) | Action::Cutaway(_), Call::Floor(_))
                | (Action::Roof { .. }, Call::Roof(_))
                | (Action::Floor { .. }, Call::Floor(_))
                | (Action::Center { .. }, Call::Center(_))
                | (Action::CenterPerson { .. }, Call::Portrait(_))
                | (Action::Frame, Call::Frame(_))
                | (Action::Scroll { .. }, Call::Scroll(_))
                | (Action::Cancel, Call::Cancel(_))
                | (Action::Clear, Call::Clear(_))
                | (Action::Wall { .. }, Call::Wall(_))
        );
        if !compatible {
            return Err("camera call belongs to another action".into());
        }
        if let Some(child) = &self.child {
            if self.child_action().as_ref() != Some(&child.action) {
                return Err("orphan native camera child".into());
            }
            child.validate(s, v, depth + 1)?;
        }
        Ok(())
    }
    fn acknowledge_done(&mut self, s: &mut State, v: &mut open::State) -> Result<(), String> {
        match &mut self.call {
            Call::Transition(c) => {
                let mut a = s.transition(v)?;
                c.acknowledge(
                    &mut a,
                    &mut s.tiles,
                    &c.request().ok_or("camera parent complete")?,
                    transition::Reply::Done,
                )?;
                s.publish_transition(v, a);
            }
            Call::Floor(c) => c.acknowledge(
                &mut v.floor,
                &c.request().ok_or("camera floor complete")?,
                lifecycle::Reply::Done,
            )?,
            Call::Center(c) => {
                let mut a = s.center(v);
                c.acknowledge(
                    &mut a,
                    &c.request().ok_or("camera center complete")?,
                    centering::Reply::Done,
                )?;
                s.publish_center(v, a);
            }
            Call::Portrait(c) => {
                let mut state = s.center(v);
                c.acknowledge(
                    &mut state,
                    &c.request().ok_or("camera portrait complete")?,
                    portrait_centering::Reply::Done,
                )?;
                s.publish_center(v, state);
            }
            Call::Frame(c) => {
                let mut state = s.center(v);
                c.acknowledge(
                    &mut state,
                    &c.request().ok_or("camera motion frame complete")?,
                    motion_frame::Reply::Done,
                )?;
                s.publish_center(v, state);
            }
            _ => return Err("camera child has no caller".into()),
        }
        Ok(())
    }
    fn step(
        &mut self,
        s: &mut State,
        v: &mut open::State,
        i: &Inputs<'_>,
    ) -> Result<Progress, String> {
        if let Some(child) = &mut self.child {
            let result = child.step(s, v, i)?;
            if result == Progress::Complete {
                self.child = None;
                self.acknowledge_done(s, v)?;
                return Ok(Progress::Pending);
            }
            return Ok(result);
        }
        if self.complete() {
            return Ok(Progress::Complete);
        }
        if let Some(action) = self.child_action() {
            self.child = Some(Box::new(Self::begin(action, s, v)?));
            return Ok(Progress::Pending);
        }
        match &mut self.call {
            Call::Roof(c) => {
                if let Some(request) = c.advance(
                    s.settings.roof.as_mut().ok_or("roof byte is not bound")?,
                    &mut v.floor.visibility.dirty,
                    i.roof_controls_present,
                )? {
                    return Ok(Progress::Service(Service::Roof(request)));
                }
            }
            Call::Transition(c) => {
                let request = c.request().ok_or("camera transition request absent")?;
                let reply = match request {
                    transition::Request::SynchronizeRenderer if !s.settings.renderer_ready => {
                        transition::Reply::Done
                    }
                    transition::Request::MotionCoordinate { axis } => {
                        transition::Reply::Coordinate(
                            *s.settings
                                .motion
                                .destination()
                                .get(axis as usize)
                                .ok_or("camera motion axis invalid")?,
                        )
                    }
                    transition::Request::ResolveCenter => {
                        let mid = crate::placement::SearchView {
                            viewport: s.settings.viewport,
                            bounds: s.settings.bounds,
                            rotation: s.settings.world_rotation as i32,
                        }
                        .midpoint();
                        transition::Reply::Point(s.projection(v, i, v.floor.level, |p| {
                            p.fine_point(mid, v.floor.level)
                        })?)
                    }
                    transition::Request::RotateWorld { rotation, force } => {
                        //4791a6 rotates the four outer corners. For the full64
                        //grid the extrema remain0..63 at all four rotations.
                        s.settings.world_rotation = rotation & 3;
                        if force == 0 {
                            s.graph.rotate((rotation & 3) as u8)?;
                        }
                        transition::Reply::Done
                    }
                    transition::Request::TerrainExtrema => {
                        s.extrema = scroll::terrain_extrema(
                            64,
                            [0, 0],
                            s.settings.world_rotation as u8,
                            &i.lot.heights,
                        )?;
                        transition::Reply::Done
                    }
                    transition::Request::RefreshRooms { level } => {
                        s.refresh(v, i, level)?;
                        transition::Reply::Done
                    }
                    transition::Request::Compose { include_visible } => {
                        s.compose(v, i, include_visible)?;
                        transition::Reply::Done
                    }
                    _ => return Ok(Progress::Service(Service::Transition(request))),
                };
                let mut state = s.transition(v)?;
                c.acknowledge(&mut state, &mut s.tiles, &request, reply)?;
                s.publish_transition(v, state);
            }
            Call::Floor(c) => {
                let request = c.request().ok_or("camera floor request absent")?;
                match request {
                    lifecycle::Request::WallState { enabled, level } => s.wall(enabled, level)?,
                    lifecycle::Request::RefreshRooms { level } => s.refresh(v, i, level)?,
                    lifecycle::Request::Compose { include_visible } => {
                        s.compose(v, i, include_visible)?
                    }
                    _ => return Ok(Progress::Service(Service::Floor(request))),
                }
                c.acknowledge(&mut v.floor, &request, lifecycle::Reply::Done)?;
            }
            Call::Center(c) => {
                let request = c.request().ok_or("camera center request absent")?;
                let mut state = s.center(v);
                let reply = match request {
                    centering::Request::Project { point, level, zoom } => {
                        if zoom != v.zoom {
                            return Err("camera projection zoom changed".into());
                        }
                        centering::Reply::Screen(
                            s.projection(v, i, level, |p| p.fine_screen(point, level))?,
                        )
                    }
                    centering::Request::SampleMotion { .. }
                    | centering::Request::ConfigureMotion { .. } => s
                        .settings
                        .motion
                        .center_service(&mut state, &request, i.clock)?,
                    _ => return Ok(Progress::Service(Service::Center(request))),
                };
                c.acknowledge(&mut state, &request, reply)?;
                s.publish_center(v, state);
            }
            Call::Portrait(c) => {
                let request = c.request().ok_or("camera portrait request absent")?;
                let mut state = s.center(v);
                let reply = match request {
                    portrait_centering::Request::Project { point, level } => {
                        portrait_centering::Reply::Screen(
                            s.projection(v, i, level, |projection| {
                                projection.fine_screen(point, level)
                            })?,
                        )
                    }
                    portrait_centering::Request::Bounds { .. } => {
                        let bounds = scroll::bounds(&s.scroll_context(v)?)?;
                        s.settings.bottom_margin = bounds.bottom_margin;
                        portrait_centering::Reply::Bounds(bounds)
                    }
                    portrait_centering::Request::ConfigureMotion {
                        start,
                        target,
                        style,
                    } => {
                        s.settings.motion.center_service(
                            &mut state,
                            &centering::Request::ConfigureMotion {
                                start,
                                target,
                                style,
                            },
                            i.clock,
                        )?;
                        portrait_centering::Reply::Done
                    }
                    _ => return Ok(Progress::Service(Service::Portrait(request))),
                };
                c.acknowledge(&mut state, &request, reply)?;
                s.publish_center(v, state);
            }
            Call::Frame(c) => {
                let request = c.request().ok_or("camera motion frame request absent")?;
                let mut state = s.center(v);
                let reply = match &request {
                    motion_frame::Request::Prepare(motion::frame::Request::SampleMotion {
                        axis,
                    }) => {
                        let coordinate = s.settings.motion.sample_axis(*axis, i.clock)?;
                        state.motion_active = s.settings.motion.active;
                        motion_frame::Reply::Coordinate(coordinate)
                    }
                    motion_frame::Request::Prepare(motion::frame::Request::Synchronize)
                    | motion_frame::Request::Displacement(
                        motion::frame::displacement::Request::Synchronize,
                    ) if !s.settings.renderer_ready => motion_frame::Reply::Done,
                    motion_frame::Request::Prepare(request) => {
                        return Ok(Progress::Service(Service::MotionFrame(request.clone())));
                    }
                    motion_frame::Request::Displacement(request) => {
                        return Ok(Progress::Service(Service::Displacement(request.clone())));
                    }
                };
                c.acknowledge(&mut state, &request, reply)?;
                s.publish_center(v, state);
            }
            Call::Scroll(c) => {
                return Ok(Progress::Service(Service::Scroll(
                    c.request().ok_or("camera scroll request absent")?.clone(),
                )));
            }
            Call::Cancel(c) => {
                return Ok(Progress::Service(Service::Cancel(
                    c.request().ok_or("camera cancel request absent")?.clone(),
                )));
            }
            Call::Clear(c) => {
                if let Some(p) = c.advance(&mut v.floor.visibility, v.floor.level)? {
                    return Ok(Progress::Service(Service::InvalidateTile(p)));
                }
            }
            Call::Wall(done) => {
                let Action::Wall { enabled, level } = self.action else {
                    return Err("camera wall action absent".into());
                };
                s.wall(enabled, level)?;
                *done = true;
            }
        }
        Ok(Progress::Pending)
    }
    fn service(&self) -> Option<Service> {
        if let Some(c) = &self.child {
            return c.service();
        }
        match &self.call {
            Call::Roof(c) => c.request().map(Service::Roof),
            Call::Transition(c) => c.request().map(Service::Transition),
            Call::Floor(c) => c.request().map(Service::Floor),
            Call::Center(c) => c.request().map(Service::Center),
            Call::Portrait(c) => c.request().map(Service::Portrait),
            Call::Frame(c) => c.request().map(|request| match request {
                motion_frame::Request::Prepare(request) => Service::MotionFrame(request),
                motion_frame::Request::Displacement(request) => Service::Displacement(request),
            }),
            Call::Scroll(c) => c.request().cloned().map(Service::Scroll),
            Call::Cancel(c) => c.request().cloned().map(Service::Cancel),
            Call::Clear(c) => c.pending().map(Service::InvalidateTile),
            Call::Wall(_) => None,
        }
    }
    fn acknowledge(
        &mut self,
        s: &mut State,
        v: &mut open::State,
        request: &Service,
        reply: Reply,
    ) -> Result<(), String> {
        if let Some(c) = &mut self.child {
            return c.acknowledge(s, v, request, reply);
        }
        if self.service().as_ref() != Some(request) {
            return Err("camera service acknowledgement mismatch".into());
        }
        match (&mut self.call, request, reply) {
            (Call::Roof(c), Service::Roof(r), Reply::Done) => c.acknowledge(*r)?,
            (Call::Transition(c), Service::Transition(r), reply) => {
                let reply = match reply {
                    Reply::Done => transition::Reply::Done,
                    Reply::Token(t) => transition::Reply::Token(t),
                    _ => return Err("camera transition reply type mismatch".into()),
                };
                let mut state = s.transition(v)?;
                c.acknowledge(&mut state, &mut s.tiles, r, reply)?;
                s.publish_transition(v, state);
            }
            (Call::Floor(c), Service::Floor(r), reply) => {
                let reply = match reply {
                    Reply::Done => lifecycle::Reply::Done,
                    Reply::Token(Some(t)) => lifecycle::Reply::Surface(t),
                    _ => return Err("camera floor surface absent".into()),
                };
                c.acknowledge(&mut v.floor, r, reply)?;
            }
            (Call::Center(c), Service::Center(r), Reply::Done) => {
                let mut state = s.center(v);
                c.acknowledge(&mut state, r, centering::Reply::Done)?;
                s.publish_center(v, state);
            }
            (Call::Portrait(c), Service::Portrait(request), Reply::Done) => {
                let mut state = s.center(v);
                c.acknowledge(&mut state, request, portrait_centering::Reply::Done)?;
                s.publish_center(v, state);
            }
            (Call::Frame(c), request, reply) => {
                let request = match request {
                    Service::MotionFrame(request) => {
                        motion_frame::Request::Prepare(request.clone())
                    }
                    Service::Displacement(request) => {
                        motion_frame::Request::Displacement(request.clone())
                    }
                    _ => return Err("camera motion frame service mismatch".into()),
                };
                let reply = match reply {
                    Reply::Done => motion_frame::Reply::Done,
                    Reply::Enabled(enabled) => motion_frame::Reply::Enabled(enabled),
                    _ => return Err("camera motion frame reply type mismatch".into()),
                };
                let mut state = s.center(v);
                c.acknowledge(&mut state, &request, reply)?;
                s.publish_center(v, state);
            }
            (Call::Scroll(c), Service::Scroll(r), Reply::Done) => c.acknowledge(r)?,
            (Call::Cancel(c), Service::Cancel(r), reply) => {
                let reply = match reply {
                    Reply::Done => motion::cancel::Reply::Done,
                    Reply::Token(Some(t)) => motion::cancel::Reply::Surface(t),
                    _ => return Err("camera cancel surface absent".into()),
                };
                c.acknowledge(r, reply)?;
            }
            (Call::Clear(c), Service::InvalidateTile(p), Reply::Done) => c.acknowledge(*p)?,
            _ => return Err("camera service reply type mismatch".into()),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    request: open::Request,
    root: Node,
    pending: Option<Service>,
}
impl Cursor {
    pub fn begin(
        request: open::Request,
        s: &mut State,
        v: &mut open::State,
    ) -> Result<Self, String> {
        s.validate(v)?;
        let action = match &request {
            open::Request::WallsDown(enabled) => Action::WallsDown(*enabled),
            open::Request::Cutaway(enabled) => Action::Cutaway(*enabled),
            open::Request::Roof { enabled, force } => Action::Roof {
                enabled: *enabled,
                force: *force,
            },
            open::Request::Zoom { .. } | open::Request::Rotate { .. } => {
                Action::Transition(request.clone())
            }
            open::Request::Floor { level, force } => Action::Floor {
                level: *level,
                force: *force,
            },
            open::Request::CenterPerson { point, level } => Action::CenterPerson {
                point: *point,
                level: *level,
            },
            open::Request::MotionFrame => Action::Frame,
            open::Request::Scroll {
                position,
                immediate,
            } => Action::Scroll {
                position: *position,
                immediate: *immediate,
            },
            open::Request::WallState { enabled, level } => Action::Wall {
                enabled: *enabled,
                level: *level,
            },
            _ => return Err("house-view request is not a camera operation".into()),
        };
        Ok(Self {
            root: Node::begin(action, s, v)?,
            request,
            pending: None,
        })
    }
    pub fn request(&self) -> &open::Request {
        &self.request
    }
    pub fn complete(&self) -> bool {
        self.pending.is_none() && self.root.complete()
    }
    pub fn pending(&self) -> Option<&Service> {
        self.pending.as_ref()
    }
    pub fn validate(&self, s: &State, v: &open::State) -> Result<(), String> {
        s.validate(v)?;
        self.root.validate(s, v, 0)?;
        if self.pending.is_some() && self.pending != self.root.service() {
            return Err("orphan camera service".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        s: &mut State,
        v: &mut open::State,
        i: &Inputs<'_>,
        budget: usize,
    ) -> Result<Progress, String> {
        if budget == 0 {
            return Err("camera work requires a positive budget".into());
        }
        self.validate(s, v)?;
        if i.lot.size != 64 {
            return Err("camera lot size changed".into());
        }
        if let Some(p) = &self.pending {
            return Ok(Progress::Service(p.clone()));
        }
        for _ in 0..budget {
            match self.root.step(s, v, i)? {
                Progress::Service(p) => {
                    self.pending = Some(p.clone());
                    return Ok(Progress::Service(p));
                }
                Progress::Complete => return Ok(Progress::Complete),
                Progress::Pending => (),
            }
        }
        Ok(if self.complete() {
            Progress::Complete
        } else {
            Progress::Pending
        })
    }
    pub fn acknowledge(
        &mut self,
        s: &mut State,
        v: &mut open::State,
        request: &Service,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate(s, v)?;
        if self.pending.as_ref() != Some(request) {
            return Err("camera service was not requested".into());
        }
        self.root.acknowledge(s, v, request, reply)?;
        self.pending = None;
        Ok(())
    }
}
