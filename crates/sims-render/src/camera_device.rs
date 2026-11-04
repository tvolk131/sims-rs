//! Replacement renderer for the original camera command's device boundaries.
//!
//! The modern backend rebuilds the full scene (including its float depth
//! vertices and viewport clipping), instead of retaining native integer lookup
//! arrays, terrain-edge surfaces and dirty rectangles. Original camera and
//! notification ordering stays in Session. Pixel depth remains approximate:
//! this is NOT an implementation of the protected4c9ceb/4ca1e0 lookup producers.
use crate::{Camera, Scene, original_house::OriginalHouse};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::{
    Session,
    camera_command::{Change, Progress},
    completion, notifications,
};
use sims_sim::{
    lot_presenter::{camera, camera_command::Ui, centering, lifecycle, motion, open, transition},
    pose::portrait::view::Input,
};
use sims_vm::commander::house::Request as Listener;
use std::sync::Arc;

const SURFACE: u32 = 1;
mod bind;
pub mod mode;
mod normal_frame;
pub mod panels;
mod properties;
mod viewport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Frame {
    input: Input,
    floor: i32,
    tick: u64,
}
impl Frame {
    fn current(session: &Session) -> Result<Self> {
        let p = session
            .host
            .presentation
            .presenter
            .as_ref()
            .context("camera presenter absent")?;
        let s = &p.camera.as_ref().context("camera context absent")?.settings;
        ensure!(
            s.renderer_ready && s.graphics_enabled,
            "replacement camera device is not enabled"
        );
        let [left, top, right, bottom] = s.viewport;
        ensure!(
            left == 0 && top == 0 && right > 0 && bottom > 0,
            "local camera viewport unavailable"
        );
        Ok(Self {
            input: Input {
                viewport: [0, 0, right as u32, bottom as u32],
                center: p.view.origin,
                zoom: p.view.zoom,
                rotation: p.view.display_rotation,
            },
            floor: p.view.floor.level,
            tick: session.simulation.vm.tick,
        })
    }
    fn camera(self) -> Result<Camera> {
        Camera::from_native(self.input, self.floor)
    }
}

/// Saved device recipe. GPU allocations are reconstructed; native command
/// continuations, broadcasts and UI property writes live in the Session save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_cursor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    furniture_selection: Option<FurnitureSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_widgets: Option<panels::Widgets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode_window: Option<mode::Window>,
    /// Real replacement viewport ownership. Historical saves remain unbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    viewport: Option<viewport::Binding>,
    /// Actual replacement pause presentation; absent on historical devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport_pause: Option<u8>,
    /// Actual replacement ViewControl expansion property; absent on older saves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    panel_state: Option<i32>,
    source: String,
    version: u32,
    frame: Frame,
    published_frame: Frame,
    dirty: bool,
    generation: u64,
    published_generation: u64,
    controls: Ui,
    controls_generation: u64,
    interface_flags: u32,
    selected_person: Option<sims_vm::ObjectId>,
    /// Replacement drawing backend starts outside build/buy overlays. Missing
    /// historical storage stays unknown; it cannot acknowledge4c4222.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_mode: Option<u8>,
    /// A suspended person refresh cannot be rendered from its partial pose.
    /// Retain the actual last published image until the command can flush.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frozen_pixels: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    normal_frame: Option<normal_frame::Ownership>,
}

///Actual replacement selection toolbar, owned by4cf90b/4cf966 services.
///The captured label survives intermediate removal until native clear returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FurnitureSelection {
    pub object: u32,
    pub definition: u32,
    pub name: String,
    pub price: i32,
    pub move_allowed: bool,
    pub sell_allowed: bool,
}
pub struct Device {
    saved: Saved,
    renderer: OriginalHouse,
    scene: Arc<Scene>,
    working: Arc<Scene>,
    frozen: bool,
}
impl Device {
    /// Copy retained device ownership for an independent test branch. Scene
    /// data is immutable; subsequent refreshes replace each branch's Arc. The
    /// derived renderer cache starts empty and is never part of a game save.
    pub fn fork_for_exploration(&self) -> Self {
        Self {
            saved: self.saved.clone(),
            renderer: OriginalHouse::default(),
            scene: self.scene.clone(),
            working: self.working.clone(),
            frozen: self.frozen,
        }
    }

    /// Session owns rotation's person refresh; this device owns the subsequent
    /// renderer/UI/audio boundaries.
    pub fn begin(&self, session: &mut Session, change: Change) -> Result<()> {
        self.require_unowned_normal_frame(session)?;
        session.begin_camera_change(change, 0)
    }
    pub fn new(session: &Session) -> Result<Self> {
        let frame = Frame::current(session)?;
        let saved = Saved {
            tool_cursor: None,
            furniture_selection: None,
            panel_widgets: None,
            mode_window: None,
            viewport: Some(viewport::Binding::new(frame)?),
            transport_pause: None,
            panel_state: None,
            source: session.house.source.clone(),
            version: 1,
            frame,
            published_frame: frame,
            dirty: false,
            generation: 0,
            published_generation: 0,
            controls: session.host.presentation.camera_properties()?,
            controls_generation: 0,
            interface_flags: 0,
            selected_person: session
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?,
            drawing_mode: Some(0),
            frozen_pixels: None,
            normal_frame: None,
        };
        Self::restore(session, saved)
    }
    pub fn restore(session: &Session, mut saved: Saved) -> Result<Self> {
        normal_frame::validate(&saved, session)?;
        normal_frame::validate_pixels(&saved)?;
        mode::validate(session, &saved)?;
        panels::validate(session, &saved)?;
        properties::validate(session, &saved)?;
        if let Some(value) = session
            .host
            .presentation
            .publication
            .as_ref()
            .and_then(|p| p.drawing_written())
        {
            ensure!(
                saved.drawing_mode == Some(value),
                "publication drawing byte differs"
            );
        }
        if let Some(viewport) = &saved.viewport {
            viewport.validate(saved.frame)?;
        }
        match (&session.host.presentation.transport, saved.transport_pause) {
            (None, None) => (),
            (Some(state), Some(value)) => ensure!(
                saved.interface_flags == 0
                    && (saved.mode_window.is_some()
                        || state.pending()
                        || state.controls.paused == value),
                "transport widget differs from session"
            ),
            _ => anyhow::bail!("transport widget lost its session binding"),
        }
        match (
            session.host.presentation.control_panel.as_ref(),
            saved.panel_state,
        ) {
            (None, None) => (),
            (Some(p), Some(value)) => ensure!(
                matches!(value, 0 | 1)
                    && (p.pending()
                        || value == p.value()
                        || (session.publication_pending()
                            && saved
                                .panel_widgets
                                .as_ref()
                                .and_then(|w| w.presentation())
                                .and_then(|p| p.collapsed())
                                .is_some_and(|v| i32::from(v) == value))),
                "panel widget differs from session"
            ),
            _ => anyhow::bail!("panel widget lost its session binding"),
        }
        ensure!(
            saved.version == 1 && saved.source == session.house.source,
            "camera device belongs to another house/profile"
        );
        normal_frame::validate_tick(&saved, session.simulation.vm.tick)?;
        ensure!(
            saved.published_generation <= saved.generation,
            "invalid camera surface generation"
        );
        saved.frame.camera()?;
        saved.published_frame.camera()?;
        let mut renderer = OriginalHouse::default();
        let frozen = saved.frozen_pixels.is_some();
        let (working, scene) = if let Some(pixels) = saved.frozen_pixels.take() {
            let [_, _, w, h] = saved.published_frame.input.viewport;
            let scene = Arc::new(crate::software::rgba_scene(w, h, pixels)?);
            (scene.clone(), scene)
        } else {
            let working = Arc::new(renderer.scene(session, saved.frame.camera()?)?);
            let scene = if saved.frame == saved.published_frame {
                working.clone()
            } else {
                Arc::new(renderer.scene(session, saved.published_frame.camera()?)?)
            };
            (working, scene)
        };
        Ok(Self {
            saved,
            renderer,
            scene,
            working,
            frozen,
        })
    }
    pub fn save(&self, session: &Session) -> Result<Saved> {
        normal_frame::validate(&self.saved, session)?;
        ensure!(
            self.saved.normal_frame.is_none() || self.frozen,
            "normal-frame surface thawed before publication"
        );
        panels::validate(session, &self.saved)?;
        properties::validate(session, &self.saved)?;
        if let Some(viewport) = &self.saved.viewport {
            viewport.validate(self.saved.frame)?;
        }
        normal_frame::validate_tick(&self.saved, session.simulation.vm.tick)?;
        let mut saved = self.saved.clone();
        if session.camera_command_pending()
            || session.house_notification_pending()
            || session.interface_update_pending()
            || session.control_panel_pending()
            || session.ui_portrait_pending()
            || session.simulation.selection_work_pending()
            || session.player_input_blocks_ticks()
            || self.frozen
        {
            saved.frozen_pixels = Some(crate::software::render(&self.scene).into_raw());
        }
        Ok(saved)
    }
    pub fn scene(&self) -> &Arc<Scene> {
        &self.scene
    }
    ///Explicitly bind this renderer's hidden-roof policy for older device saves.
    ///This does not infer the original process's View+51 from a household file.
    pub fn bind_hidden_roof(&self, session: &mut Session) -> Result<()> {
        ensure!(!session.camera_command_pending(), "camera owns roof state");
        let camera = session
            .host
            .presentation
            .presenter
            .as_mut()
            .and_then(|p| p.camera.as_mut())
            .context("roof camera absent")?;
        ensure!(camera.settings.roof.is_none(), "roof policy already bound");
        camera.settings.roof = Some(0);
        Ok(())
    }
    pub fn generation(&self) -> u64 {
        self.saved.generation
    }
    pub fn published_generation(&self) -> u64 {
        self.saved.published_generation
    }
    pub fn controls(&self) -> Ui {
        self.saved.controls
    }

    pub fn panel_state(&self) -> Option<i32> {
        self.saved
            .panel_widgets
            .as_ref()
            .and_then(|w| w.presentation())
            .and_then(|p| p.collapsed())
            .map(i32::from)
            .or(self.saved.panel_state)
    }
    pub fn transport_pause(&self) -> Option<u8> {
        self.saved.transport_pause
    }
    /// Fresh actual controls adopt the shared CPState fields. Older saved
    /// devices stay unbound instead of inventing missing process state.
    pub fn bind_transport(&mut self, session: &mut Session) -> Result<()> {
        ensure!(
            self.saved.transport_pause.is_none(),
            "transport widget already bound"
        );
        session.bind_transport(self.saved.interface_flags)?;
        self.saved.interface_flags = 0;
        self.saved.transport_pause = Some(
            session
                .host
                .presentation
                .transport
                .as_ref()
                .unwrap()
                .controls
                .paused,
        );
        Ok(())
    }
    pub fn advance_transport(&mut self, session: &mut Session, budget: usize) -> Result<bool> {
        use sims_runtime::original_session::transport::{Progress, Service};
        ensure!(
            self.saved.transport_pause.is_some(),
            "transport widget absent"
        );
        match session.advance_transport(budget)? {
            Progress::Complete => {
                self.refresh(session)?;
                return Ok(true);
            }
            Progress::Pending => (),
            Progress::Service(service) => {
                match service {
                    Service::WindowPause(value) => {
                        if self.saved.mode_window.is_some() {
                            if !self.advance_transport_window(session, value)? {
                                return Ok(false);
                            }
                        } else {
                            self.saved.transport_pause = Some(value as u8);
                        }
                    }
                    //57a7f2/57a8e3's original disabled-audio early returns.
                    Service::PauseAudio | Service::ResumeAudio => (),
                }
                session.acknowledge_transport(service)?;
            }
        }
        Ok(false)
    }
    pub fn begin_panel_toggle(&self, session: &mut Session) -> Result<()> {
        session.begin_panel_toggle(self.panel_state().context("panel widget is not bound")?)
    }
    pub fn begin_panel_expansion(&self, session: &mut Session, value: i32) -> Result<()> {
        session.begin_panel_expansion(
            self.panel_state().context("panel widget is not bound")?,
            value,
        )
    }
    /// Publish the native state into the real replacement widget property. The
    /// Iced sidebar does not occlude the retained camera surface, so its viewport
    /// stays fixed; legacy CPanel window geometry is not substituted here.
    pub fn advance_panel(&mut self, session: &mut Session) -> Result<bool> {
        use sims_runtime::original_session::panel::Progress;
        use sims_vm::interface::panel::Request;
        match session.advance_panel_update()? {
            Progress::Service(Request::Whoosh) => {
                // Existing audio-disabled backend: original57a54c's early return.
                session.acknowledge_panel_service(&Request::Whoosh)?;
            }
            Progress::Service(Request::WindowLayout(_)) => {
                anyhow::bail!("legacy application window layout needs its real owner")
            }
            Progress::Publish(value) => {
                self.publish_panel_display(value)?;
                session.acknowledge_panel_publication(value)?;
                self.refresh(session)?;
                return Ok(true);
            }
            Progress::Complete => return Ok(true),
        }
        Ok(false)
    }

    /// Original mode-dirty pass on this fixed-Live, audio-disabled backend.
    /// Room masks and script mode publication are owned by Session. A changed
    /// build drawing flag requires83 dispatch and is deliberately unsupported.
    pub fn advance_interface(&mut self, session: &mut Session, budget: usize) -> Result<bool> {
        use sims_runtime::original_session::interface::Progress;
        use sims_vm::interface::Request;
        match session.advance_interface_update(budget)? {
            Progress::Pending => (),
            Progress::Complete => {
                self.refresh(session)?;
                return Ok(true);
            }
            Progress::Service(request) => {
                match request {
                    Request::DrawingMode => ensure!(
                        self.saved.drawing_mode == Some(0),
                        "drawing mode needs its original83 notification owner"
                    ),
                    //57a8e3/57a6fb/57a54c all return at the disabled-audio gate.
                    Request::ResumeAudio | Request::StopAudio | Request::PlayLiveBackground => (),
                    _ => anyhow::bail!("interface engine work reached device: {request:?}"),
                }
                session.acknowledge_interface_service(&request)?;
            }
        }
        Ok(false)
    }

    ///The original picker deactivation writes the shared drawing byte. Move
    ///callbacks and their real notification children remain Session-owned.
    pub fn advance_tool_selection(&mut self, session: &mut Session, budget: usize) -> Result<bool> {
        use sims_runtime::original_session::tool_selection::Progress;
        match session.advance_tool_selection(budget)? {
            Progress::Pending => (),
            Progress::Complete => {
                session.finish_tool_selection()?;
                self.refresh(session)?;
                return Ok(true);
            }
            Progress::Service(service) => self.tool_selection_service(session, service, budget)?,
        }
        Ok(false)
    }
    fn tool_selection_service(
        &mut self,
        session: &mut Session,
        service: sims_runtime::original_session::tool_selection::Service,
        budget: usize,
    ) -> Result<()> {
        use sims_runtime::original_session::{move_tool, tool_selection::Service};
        match service {
            Service::CatalogProduct(request) => self.service_buy_product(session, request)?,
            Service::CatalogInput(request) => self.service_buy_input(session, request)?,
            Service::DrawingMode(value) => {
                self.saved.drawing_mode = Some(value);
                session.acknowledge_tool_drawing(value)?;
            }
            Service::Move(move_tool::Service::Input(r)) => self.move_input(session, r)?,
            Service::Move(move_tool::Service::Notification(_)) | Service::Notification(_) => {
                self.advance_notification(session, budget)?;
            }
            Service::Mode(sims_vm::interface::mode_change::Request::WindowPause {
                paused,
                show_overlay,
            }) => self.advance_mode_window(session, paused, show_overlay)?,
            Service::Mode(sims_vm::interface::mode_change::Request::RefreshObjects) => {
                self.refresh_definition_geometry(session)?;
                session.acknowledge_mode_definition_refresh()?;
            }
            Service::Mode(sims_vm::interface::mode_change::Request::TimedResourceUpdates(
                clock,
            )) => {
                //5a0585 expires unused animation/appearance allocations, not
                //skeletons or running animation state. Our immutable VM inputs
                //have session lifetimes. Rebuild this backend's live resource
                //set to reclaim stale render definitions/appearances instead
                //of inventing a second Windows cache/refcount object graph.
                self.prepare_resources(session, true)?;
                session.acknowledge_mode_timed_resources(clock)?;
            }
            other => anyhow::bail!("tool callback requires its input/service owner: {other:?}"),
        }
        Ok(())
    }
    pub fn advance_move_button(&mut self, s: &mut Session, budget: usize) -> Result<bool> {
        use sims_runtime::original_session::move_tool::{
            self,
            button::{Progress, Service},
        };
        ensure!(budget > 0, "button device needs a budget");
        for _ in 0..budget {
            match s.advance_move_button()? {
                Progress::Pending => (),
                Progress::Complete => {
                    s.finish_move_button()?;
                    self.refresh(s)?;
                    return Ok(true);
                }
                Progress::Service(Service::Move(move_tool::Service::Input(r))) => {
                    self.move_input(s, r)?
                }
                Progress::Service(
                    Service::Move(move_tool::Service::Notification(r)) | Service::Notification(r),
                ) => {
                    if self.listener(s, &r, 1)? {
                        s.acknowledge_house_notification(&r, notifications::Reply::Done)?;
                    }
                }
                Progress::Service(Service::Tool(r)) => self.tool_selection_service(s, r, 1)?,
                Progress::Service(Service::Move(move_tool::Service::Pickup(r))) => {
                    self.pickup_projection(s, r)?
                }
                Progress::Service(Service::Selection(r)) => {
                    self.select_furniture(r)?;
                    s.acknowledge_move_button_selection(r)?;
                }
                Progress::Service(Service::ClearProducts) => {
                    self.saved
                        .panel_widgets
                        .as_mut()
                        .context("button catalog widgets absent")?
                        .clear_product_selection()?;
                    s.acknowledge_move_button_products()?;
                }
                Progress::Service(r) => {
                    anyhow::bail!("button child requires its actual service: {r:?}")
                }
            }
        }
        Ok(false)
    }

    /// Rebuild actual sprite/mesh/material data, with the retained camera's
    /// viewport, origin, zoom and floor. This never runs a simulation slice.
    pub fn refresh(&mut self, session: &Session) -> Result<()> {
        ensure!(
            !session.camera_command_pending()
                && !session.interface_update_pending()
                && !session.control_panel_pending()
                && !session.ui_portrait_pending(),
            "camera command owns the surface publication"
        );
        if self.normal_view_frame_pending() {
            return self.finish_normal_view_frame(session);
        }
        self.prepare(session)?;
        self.publish();
        Ok(())
    }
    fn publish(&mut self) {
        self.scene = self.working.clone();
        self.saved.published_frame = self.saved.frame;
        self.saved.published_generation = self.saved.generation;
        self.saved.normal_frame = None;
        self.frozen = false;
    }
    fn prepare(&mut self, session: &Session) -> Result<()> {
        self.prepare_resources(session, false)
    }
    fn prepare_resources(&mut self, session: &Session, reclaim: bool) -> Result<()> {
        self.require_unowned_normal_frame(session)?;
        let (frame, scene) = self.build_surface(session, reclaim)?;
        self.install_surface(frame, scene)
    }
    fn build_surface(&mut self, session: &Session, reclaim: bool) -> Result<(Frame, Arc<Scene>)> {
        let frame = Frame::current(session)?;
        let scene = if reclaim {
            self.renderer.rebuild_resources(session, frame.camera()?)?
        } else {
            self.renderer.scene(session, frame.camera()?)?
        };
        ensure!(
            scene
                .vertices
                .iter()
                .all(|v| v.position.iter().all(|n| n.is_finite())),
            "nonfinite camera vertices"
        );
        Ok((frame, Arc::new(scene)))
    }
    fn install_surface(&mut self, frame: Frame, scene: Arc<Scene>) -> Result<()> {
        let generation = self
            .saved
            .generation
            .checked_add(1)
            .context("camera generation overflow")?;
        self.working = scene;
        self.saved.frame = frame;
        self.saved.dirty = false;
        self.saved.generation = generation;
        Ok(())
    }
    ///43dfc4/43dfeb release unused legacy definition payloads. Our decoded
    ///immutable resources use independent lifetimes and include every DGRP
    ///view. Both original callers rebuild the actual current view here; this
    ///does not publish the surface, touch scripts, or run a simulation slice.
    fn refresh_definition_geometry(&mut self, session: &Session) -> Result<()> {
        self.prepare(session)
    }
    fn controls_changed(&mut self, session: &Session) -> Result<()> {
        self.refresh_panel_text(session)?;
        self.capture_camera_controls(session)?;
        self.saved.selected_person = session
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?;
        // This is the actual4ce41b boundary, distinct from43dfc4 and4cee3d.
        // Property refresh alone cannot acknowledge its nested50a8ec call.
        ensure!(
            self.saved
                .panel_widgets
                .as_ref()
                .is_none_or(|w| w.texts().is_none() && w.access().is_none()),
            "panel properties refreshed; nested panel publication is not yet bound"
        );
        Ok(())
    }
    fn capture_camera_controls(&mut self, session: &Session) -> Result<()> {
        self.saved.controls = session.host.presentation.camera_properties()?;
        self.saved.controls_generation = self
            .saved
            .controls_generation
            .checked_add(1)
            .context("camera control generation overflow")?;
        Ok(())
    }
    fn listener(
        &mut self,
        session: &mut Session,
        service: &notifications::Service,
        budget: usize,
    ) -> Result<bool> {
        let notifications::Service::Listener(call) = service else {
            anyhow::bail!("non-listener camera notification {service:?}");
        };
        match call.request {
            Listener::ViewerInvalidate => self.saved.dirty = true,
            Listener::ViewerTerrain => {
                let lot = session
                    .simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("terrain lot absent")?;
                let camera = session
                    .host
                    .presentation
                    .presenter
                    .as_mut()
                    .and_then(|p| p.camera.as_mut())
                    .context("terrain camera absent")?;
                camera.extrema = sims_sim::lot_presenter::scroll::terrain_extrema(
                    i32::try_from(lot.size)?,
                    [0, 0],
                    camera.settings.world_rotation as u8,
                    &lot.heights,
                )
                .map_err(anyhow::Error::msg)?;
            }
            // cObjPickerTool inherits the proven empty50c25d camera callback.
            Listener::ViewerZoom { .. } | Listener::ViewerRotation { .. } => (),
            Listener::UiFlags { bits } => {
                if let Some(state) = &mut session.host.presentation.transport {
                    state.controls.mark_dirty(bits);
                } else {
                    self.saved.interface_flags |= bits;
                }
            }
            Listener::UiRefresh => return self.advance_property_listener(session, call, budget),
            _ => anyhow::bail!("camera notification needs its UI owner: {service:?}"),
        }
        Ok(true)
    }
    pub fn advance_notification(&mut self, session: &mut Session, budget: usize) -> Result<bool> {
        let input = session.retained_camera_input()?;
        match session.advance_house_notification(input, budget)? {
            notifications::Progress::Pending => (),
            notifications::Progress::Complete(_) => return Ok(true),
            notifications::Progress::Service(service) => {
                self.acknowledge_notification(session, &service, budget)?;
            }
        }
        Ok(!session.house_notification_pending())
    }
    fn acknowledge_notification(
        &mut self,
        session: &mut Session,
        request: &notifications::Service,
        budget: usize,
    ) -> Result<()> {
        let reply = match request {
            notifications::Service::View(open::Request::InvalidateView) => {
                // The replacement backend owns a whole-viewport scene. Retain
                // invalidation now; the original camera's flush publishes it.
                self.saved.dirty = true;
                notifications::Reply::View(
                    session
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .context("house-open presenter absent")?
                        .view
                        .clone(),
                )
            }
            notifications::Service::Camera(service) => {
                notifications::Reply::Camera(self.service(session, service)?)
            }
            notifications::Service::Listener(_) => {
                if !self.listener(session, request, budget)? {
                    return Ok(());
                }
                notifications::Reply::Done
            }
            _ => anyhow::bail!("house-open service needs its original owner: {request:?}"),
        };
        session.acknowledge_house_notification(request, reply)
    }
    /// Drive an explicitly started native completion through the actual modern
    /// device. There is no legacy terrain-renderer pointer in this backend;
    /// OriginalHouse owns the replacement terrain mesh. Other work is not skipped.
    pub fn advance_load_completion(
        &mut self,
        session: &mut Session,
        budget: usize,
    ) -> Result<bool> {
        let input = session.retained_camera_input()?;
        match session.advance_load_completion(input, budget)? {
            completion::Progress::Pending => (),
            completion::Progress::Environment => session.acknowledge_absent_terrain_renderer()?,
            completion::Progress::Notification(service) => {
                self.acknowledge_notification(session, &service, budget)?
            }
            completion::Progress::Complete => {
                self.refresh(session)?;
                self.controls_changed(session)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// These are actual replacement-renderer operations. Unsupported world or
    /// tool work stays pending; it cannot be acknowledged as a graphics no-op.
    fn service(
        &mut self,
        session: &mut Session,
        request: &camera::Service,
    ) -> Result<camera::Reply> {
        self.require_unowned_normal_frame(session)?;
        use camera::Service as S;
        use transition::Request as T;
        Ok(match request {
            S::Roof(lifecycle::roof::Request::InvalidateView) => {
                ensure!(
                    session
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .and_then(|p| p.camera.as_ref())
                        .and_then(|c| c.settings.roof)
                        == Some(0),
                    "visible roof rendering requires its original roof backend"
                );
                self.saved.dirty = true;
                camera::Reply::Done
            }
            S::Transition(T::ResolveSurface)
            | S::Floor(lifecycle::Request::ResolveSurface)
            | S::Cancel(motion::cancel::Request::ResolveSurface) => {
                camera::Reply::Token(Some(SURFACE))
            }
            S::Transition(T::ResolveUi) => {
                // No legacy popup/window tree is attached to this backend.
                camera::Reply::Token(None)
            }
            S::Transition(T::InitializeLookup { zoom }) => {
                ensure!(
                    u32::from(*zoom) == Frame::current(session)?.input.zoom,
                    "camera lookup zoom differs"
                );
                self.prepare(session)?; // Loads the actual original DGRP view.
                camera::Reply::Done
            }
            S::Transition(T::DepthTable { scale_bits }) => {
                let far = Frame::current(session)?
                    .input
                    .build()
                    .map_err(anyhow::Error::msg)?
                    .far;
                ensure!(
                    *scale_bits == far.to_bits(),
                    "camera depth scale is not its actual projection far plane"
                );
                // Rebuild the real float-depth vertex buffer. Do not compute an
                // unused native table or claim native integer pixel equivalence.
                self.prepare(session)?;
                camera::Reply::Done
            }
            S::Transition(T::SynchronizeRenderer | T::EdgeSurfaces) | S::Scroll(_) => {
                self.prepare(session)?;
                camera::Reply::Done
            }
            S::Transition(T::InvalidateView)
            | S::Floor(lifecycle::Request::InvalidateView)
            | S::Cancel(motion::cancel::Request::Invalidate)
            | S::InvalidateTile(_) => {
                self.saved.dirty = true;
                camera::Reply::Done
            }
            S::Transition(T::FlushSurface { token })
            | S::Cancel(motion::cancel::Request::FlushSurface { token })
            | S::Floor(lifecycle::Request::FlushSurface { surface: token }) => {
                ensure!(*token == SURFACE, "camera surface token differs");
                self.prepare(session)?;
                self.publish();
                camera::Reply::Done
            }
            S::Transition(T::RefreshUi) => {
                self.refresh_definition_geometry(session)?;
                camera::Reply::Done
            }
            S::Center(centering::Request::RefreshFloorUi { level, flag }) => {
                session.refresh_center_floor_ui(*level, *flag)?;
                self.capture_camera_controls(session)?;
                camera::Reply::Done
            }
            S::Transition(T::FinishRotation) => camera::Reply::Done, //57a9d4 disabled audio gate.
            _ => anyhow::bail!("camera service needs its original owner: {request:?}"),
        })
    }
    /// Bounded application command work. Audio uses the disabled57a9d4 gate;
    /// Session owns the selected picker/Move callback after each native setter.
    pub fn advance(&mut self, session: &mut Session, budget: usize) -> Result<bool> {
        self.require_unowned_normal_frame(session)?;
        let input = Frame::current(session)?.input;
        match session.advance_camera_command(input, budget)? {
            Progress::Pending => (),
            Progress::Complete => {
                self.unfreeze(session)?;
                return Ok(true);
            }
            Progress::Camera(request) => {
                let reply = self.service(session, &request)?;
                session.acknowledge_camera_command(&request, reply)?;
            }
            Progress::Notification(service) => {
                if self.listener(session, &service, budget)? {
                    session
                        .acknowledge_camera_notification(&service, notifications::Reply::Done)?;
                }
            }
            Progress::RefreshAudio => {
                session.complete_camera_audio()?; //57a9d4 disabled gate.
                self.capture_camera_controls(session)?;
            }
            Progress::RefreshControls => {
                session.complete_camera_controls()?;
                self.capture_camera_controls(session)?;
            }
        }
        if !session.camera_command_pending() {
            self.unfreeze(session)?;
        }
        Ok(!session.camera_command_pending())
    }
    ///Full frame caller; unsupported world/panel children remain explicit.
    pub fn advance_publication(
        &mut self,
        session: &mut Session,
        budget: usize,
    ) -> Result<sims_runtime::original_session::publication::Progress> {
        use sims_runtime::original_session::publication::Progress as P;
        use sims_vm::interface::world_publication::Request as W;
        match session.advance_interface_publication(budget)? {
            P::Panels(owner) if self.saved.panel_widgets.is_some() => {
                if !self.advance_panel_widgets(session, budget)? {
                    return Ok(P::Panels(owner));
                }
            }
            P::World(W::DrawingMode(_)) => {
                let drawing = self
                    .saved
                    .drawing_mode
                    .as_mut()
                    .context("drawing backend absent")?;
                session.advance_publication_drawing(drawing)?;
            }
            P::World(
                r @ (W::PauseAudio
                | W::ResumeAudio
                | W::RefreshAudio
                | W::StopMusic
                | W::PlayMusic(_)),
            ) => {
                //Actual57a7f2/57a8e3/57a9d4 and57a6fb/57a54c early gates:
                //this backend disables audio (+14 is zero).
                session.acknowledge_publication_audio(r)?;
            }
            P::Notification(service) => {
                if self.listener(session, &service, budget)? {
                    session.acknowledge_house_notification(&service, notifications::Reply::Done)?;
                } else if session
                    .host
                    .presentation
                    .property_refresh
                    .as_ref()
                    .and_then(|p| p.panel_owner())
                    .is_some()
                {
                    return Ok(P::Panels(
                        self.property_panel_owner().map(|o| u64::from(o.panel)),
                    ));
                }
            }
            P::Camera(progress) => match progress {
                Progress::Pending | Progress::Complete => (),
                Progress::Camera(request) => {
                    let reply = self.service(session, &request)?;
                    session.acknowledge_camera_command(&request, reply)?;
                }
                Progress::Notification(service) => {
                    if self.listener(session, &service, budget)? {
                        session.acknowledge_camera_notification(
                            &service,
                            notifications::Reply::Done,
                        )?;
                    } else if session
                        .host
                        .presentation
                        .property_refresh
                        .as_ref()
                        .and_then(|p| p.panel_owner())
                        .is_some()
                    {
                        return Ok(P::Panels(
                            self.property_panel_owner().map(|o| u64::from(o.panel)),
                        ));
                    }
                }
                Progress::RefreshAudio | Progress::RefreshControls => {
                    anyhow::bail!("direct publication camera acquired a UI setter tail")
                }
            },
            P::Tool(
                sims_runtime::original_session::current_tool::Progress::Pending
                | sims_runtime::original_session::current_tool::Progress::Complete,
            ) => (),
            P::Tool(sims_runtime::original_session::current_tool::Progress::Service(
                sims_runtime::original_session::current_tool::Service::Notification(service),
            )) => {
                if self.listener(session, &service, budget)? {
                    session.acknowledge_house_notification(&service, notifications::Reply::Done)?;
                } else if session
                    .host
                    .presentation
                    .property_refresh
                    .as_ref()
                    .and_then(|p| p.panel_owner())
                    .is_some()
                {
                    return Ok(P::Panels(
                        self.property_panel_owner().map(|o| u64::from(o.panel)),
                    ));
                }
            }
            P::Tool(sims_runtime::original_session::current_tool::Progress::Service(
                sims_runtime::original_session::current_tool::Service::Callback(
                    sims_vm::interface::current_tool::Request::Enter(id),
                ),
            )) => {
                self.saved.tool_cursor = Some(0);
                session.acknowledge_picker_enter(id)?;
            }
            P::Tool(sims_runtime::original_session::current_tool::Progress::Service(
                sims_runtime::original_session::current_tool::Service::Move(
                    sims_runtime::original_session::move_tool::Service::Notification(service),
                ),
            )) => {
                if self.listener(session, &service, budget)? {
                    session.acknowledge_house_notification(&service, notifications::Reply::Done)?;
                } else if session
                    .host
                    .presentation
                    .property_refresh
                    .as_ref()
                    .and_then(|p| p.panel_owner())
                    .is_some()
                {
                    return Ok(P::Panels(
                        self.property_panel_owner().map(|o| u64::from(o.panel)),
                    ));
                }
            }
            P::Tool(sims_runtime::original_session::current_tool::Progress::Service(
                sims_runtime::original_session::current_tool::Service::Move(
                    sims_runtime::original_session::move_tool::Service::Input(r),
                ),
            )) => self.move_input(session, r)?,
            other => return Ok(other),
        }
        Ok(P::Pending)
    }
    fn unfreeze(&mut self, session: &Session) -> Result<()> {
        self.require_unowned_normal_frame(session)?;
        if self.frozen {
            self.scene = Arc::new(
                self.renderer
                    .scene(session, self.saved.published_frame.camera()?)?,
            );
            self.working = if self.saved.frame == self.saved.published_frame {
                self.scene.clone()
            } else {
                Arc::new(self.renderer.scene(session, self.saved.frame.camera()?)?)
            };
            self.frozen = false;
        }
        Ok(())
    }
    pub fn tool_cursor(&self) -> Option<u32> {
        self.saved.tool_cursor
    }
    pub fn furniture_selection(&self) -> Option<&FurnitureSelection> {
        self.saved.furniture_selection.as_ref()
    }
    fn select_furniture(
        &mut self,
        r: sims_runtime::original_session::move_tool::pickup::Request,
    ) -> Result<()> {
        use sims_runtime::original_session::move_tool::pickup::Request;
        let Request::Select {
            object,
            definition,
            price,
            move_allowed,
            sell_allowed,
        } = r
        else {
            anyhow::bail!("selection requires selected furniture")
        };
        let catalog = self
            .panel_widgets()
            .and_then(|w| w.catalog())
            .context("selection catalog absent")?;
        let product = (0..catalog.product_count())
            .filter_map(|i| catalog.product(i))
            .find(|p| p.definition == definition)
            .context("selected furniture catalog text absent")?;
        self.saved.furniture_selection = Some(FurnitureSelection {
            object,
            definition,
            price,
            move_allowed,
            sell_allowed,
            name: product.display_name(),
        });
        Ok(())
    }
    fn pickup_projection(
        &mut self,
        s: &mut Session,
        r: sims_runtime::original_session::move_tool::pickup::Request,
    ) -> Result<()> {
        use sims_runtime::original_session::move_tool::pickup::{Reply, Request};
        let Request::Project { position, level } = r else {
            anyhow::bail!("pickup requires forward projection")
        };
        let camera = OriginalHouse::native_camera(s)?;
        let point = camera.screen(OriginalHouse::ground_point(s, position, level)?);
        s.acknowledge_move_pickup(
            r,
            Reply::Projected([point.y.round() as i16, point.x.round() as i16]),
        )
    }
    fn move_input(
        &mut self,
        s: &mut Session,
        r: sims_sim::object_catalog::move_tool::cleanup::Request,
    ) -> Result<()> {
        use sims_sim::object_catalog::move_tool::cleanup::{Reply, Request};
        match r {
            Request::SetCursor { value, .. } => {
                //The Iced viewport owns the replacement pointer presentation.
                //The native HWND identity is not a compatibility requirement.
                ensure!(value == 0, "unsupported Move cursor image {value}");
                self.saved.tool_cursor = Some(value);
                s.acknowledge_move_tool_input(r, Reply::Done)?;
            }
            Request::ClearSelection(owner) => {
                ensure!(
                    s.host
                        .presentation
                        .move_tool
                        .as_ref()
                        .is_some_and(|m| m.inputs.control_state == owner),
                    "selection control owner differs"
                );
                self.saved.furniture_selection = None;
                s.acknowledge_move_tool_input(r, Reply::Done)?;
            }
            _ => anyhow::bail!("Move input requires its real key/control owner: {r:?}"),
        }
        Ok(())
    }
    pub fn advance_move_tool(&mut self, s: &mut Session, budget: usize) -> Result<bool> {
        use sims_runtime::original_session::move_tool::{Progress, Service};
        ensure!(budget > 0, "Move device needs a work budget");
        for _ in 0..budget {
            match s.advance_move_tool(1)?.progress {
                Progress::Pending => (),
                Progress::Complete => {
                    s.finish_move_tool_cleanup()?;
                    self.refresh(s)?;
                    return Ok(true);
                }
                Progress::Service(Service::Input(r)) => self.move_input(s, r)?,
                Progress::Service(Service::Pickup(r)) => self.pickup_projection(s, r)?,
                Progress::Service(Service::Feedback(kind)) => {
                    s.acknowledge_move_sale_feedback_disabled_audio(kind)?;
                }
                Progress::Service(Service::Notification(r)) => {
                    if self.listener(s, &r, 1)? {
                        s.acknowledge_house_notification(&r, notifications::Reply::Done)?;
                    }
                }
                Progress::Service(r) => {
                    anyhow::bail!("Move child requires its engine service: {r:?}")
                }
            }
        }
        Ok(false)
    }
}
