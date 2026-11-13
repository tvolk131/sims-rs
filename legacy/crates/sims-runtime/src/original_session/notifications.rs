//! Original house broadcast running against the live VM and saved presenter.
//! The caller supplies the actual registry and UI context. Installing this
//! owner does not infer a startup order or a suspended original process.
use super::{Session, View, presenter};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    Simulation,
    lot_presenter::{Inputs, camera, open, transition},
    pose::portrait::view::Input,
};
use sims_vm::{
    commander::{
        Registry,
        house::{
            Fields, Receiver, Request,
            dispatch::{self, Dispatch},
        },
    },
    presenter::Order,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Listener(dispatch::Service),
    View(open::Request),
    Camera(camera::Service),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    View(open::State),
    Camera(camera::Reply),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Service(Service),
    Complete(u16),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Viewer {
    caller: dispatch::Service,
    nested: Option<camera::Service>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Initializer {
    caller: dispatch::Service,
    nested: Option<sims_vm::ObjectId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub dispatch: Dispatch,
    viewer: Option<Viewer>,
    pending: Option<Service>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection: Option<sims_sim::lot_selection::Notification>,
    /// Direct440b2d E2 broadcast, outside indicator initialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation: Option<sims_vm::ObjectId>,
    ///4215e6's E2 owns a cursor lifetime, independently of440b2d's owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor_creation: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initializer: Option<Initializer>,
}
fn broadcast(service: &camera::Service) -> Option<(u32, u32)> {
    if let camera::Service::Transition(transition::Request::Broadcast { event, value }) = service {
        Some((u32::from(*event), *value))
    } else {
        None
    }
}
fn internal(request: &Request) -> bool {
    matches!(
        request,
        Request::ViewerRefresh
            | Request::Compose { .. }
            | Request::InitializeIndicators
            | Request::Broadcast { .. }
            | Request::WorldCreated { .. }
    )
}
impl State {
    /// Establish a new unpublished Rust delivery owner from a qualified idle
    /// native boundary and the retained listener registry/fields. No event is
    /// dispatched and no completed event history is manufactured.
    pub(super) fn from_completed_registry(registry: Registry<Receiver>, fields: Fields) -> Result<Self> {
        Ok(Self {
            dispatch: Dispatch::idle(registry, fields).map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: None,
            creation: None,
            cursor_creation: None,
            initializer: None,
        })
    }
    pub fn complete(&self) -> bool {
        (self.dispatch.is_idle() || self.dispatch.returned().is_some())
            && self.selection.is_none()
            && self.initializer.is_none()
            && self.creation.is_none()
            && self.cursor_creation.is_none()
    }
    pub fn pending(&self) -> Option<&Service> {
        self.pending.as_ref()
    }
    pub(super) fn direct_factory_object(&self) -> Option<sims_vm::ObjectId> {
        self.creation
    }
    pub fn validate(&self, view: &View, simulation: &Simulation) -> Result<()> {
        self.dispatch.validate().map_err(anyhow::Error::msg)?;
        let presenter = view
            .presenter
            .as_ref()
            .context("notification presenter absent")?;
        presenter
            .validate_simulation(simulation)
            .map_err(anyhow::Error::msg)?;
        ensure!(presenter.camera.is_some(), "notification camera absent");
        if let Some(init) = &self.initializer {
            ensure!(
                self.viewer.is_none()
                    && self.selection.is_none()
                    && init.caller.request == Request::InitializeIndicators
                    && simulation
                        .native_selection
                        .as_ref()
                        .is_some_and(|e| e.is_initialization()),
                "notification initializer lost its caller"
            );
            if let Some(id) = init.nested {
                ensure!(
                    self.dispatch.external_parent()
                        == Some((init.caller.clone(), (0xe2, id as u32)))
                        && simulation.pending_native_selection()
                            == Some((
                                sims_sim::lot_selection::Request::CreationNotification(id),
                                true
                            )),
                    "indicator factory lost its E2 child"
                );
            } else {
                ensure!(
                    self.dispatch.pending().as_ref() == Some(&init.caller),
                    "indicator initializer lost its listener"
                );
            }
        } else {
            ensure!(
                !simulation
                    .native_selection
                    .as_ref()
                    .is_some_and(|e| e.is_initialization()),
                "orphan notification indicator initializer"
            );
        }
        if let Some((call, true)) = simulation.pending_selection_notification() {
            ensure!(
                self.selection == Some(call),
                "selection broadcast lost its caller"
            );
        }
        if let Some(call) = self.selection {
            ensure!(
                simulation.native_selection_broadcasts
                    && simulation.pending_selection_notification() == Some((call, true))
                    && self.dispatch.root_message()
                        == Some((sims_vm::selection::SIGNAL, call.payload as u32))
                    && self.viewer.is_none(),
                "selection broadcast caller differs"
            );
        }
        if let Some(id) = self.creation {
            ensure!(
                self.cursor_creation.is_none()
                    && self.selection.is_none()
                    && self.initializer.is_none()
                    && self.viewer.is_none()
                    && self.dispatch.root_message() == Some((0xe2, id as u32))
                    && simulation
                        .native_selection
                        .as_ref()
                        .and_then(sims_sim::lot_selection::Execution::direct_factory)
                        .is_some()
                    && simulation.pending_native_selection()
                        == Some((
                            sims_sim::lot_selection::Request::CreationNotification(id),
                            true
                        )),
                "direct factory broadcast lost its caller"
            );
        }
        let cursor = simulation
            .cursor_factory
            .as_ref()
            .and_then(sims_sim::lot_cursor::Factory::pending_created);
        if let Some(h) = self.cursor_creation {
            let id = simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.object_for_handle(h))
                .context("cursor notification lifetime ended")?;
            ensure!(
                self.selection.is_none()
                    && self.initializer.is_none()
                    && self.viewer.is_none()
                    && self.creation.is_none()
                    && cursor == Some((h, true))
                    && view.object_focus == Some(h)
                    && self.dispatch.root_message() == Some((0xe2, id as u32)),
                "cursor factory broadcast lost its caller"
            );
        } else {
            ensure!(
                cursor.is_none_or(|(_, started)| !started),
                "cursor broadcast lost its dispatcher"
            );
        }
        if !self.complete() && simulation.selection_work_pending() {
            ensure!(
                self.selection.is_some()
                    || self.creation.is_some()
                    || self.initializer.is_some()
                    || (self.viewer.is_some() && presenter.owns_indicator_drain()),
                "notification overlaps unrelated selection work"
            );
        }
        ensure!(
            self.dispatch.fields.include_visible == presenter.view.floor.visibility.include_visible,
            "notification visibility differs from shared viewer"
        );
        if let Some(viewer) = &self.viewer {
            ensure!(
                viewer.caller.request == Request::ViewerRefresh,
                "notification viewer caller differs"
            );
            if let Some(service) = &viewer.nested {
                let message = broadcast(service).context("nested camera message absent")?;
                ensure!(
                    self.dispatch.external_parent() == Some((viewer.caller.clone(), message)),
                    "notification camera lost its broadcast parent"
                );
                ensure!(
                    presenter.pending_camera() == Some(service),
                    "notification camera child differs"
                );
            } else {
                ensure!(
                    self.dispatch.pending().as_ref() == Some(&viewer.caller),
                    "notification viewer lost its parent"
                );
            }
        } else {
            ensure!(
                presenter.complete() || self.complete() && presenter.normal_frame_active(),
                "orphan notification viewer work"
            );
            ensure!(
                self.dispatch.external_parent().is_none()
                    || self
                        .initializer
                        .as_ref()
                        .is_some_and(|i| i.nested.is_some()),
                "orphan camera broadcast"
            );
        }
        if let Some(pending) = &self.pending {
            match pending {
                Service::Listener(service) => ensure!(
                    !internal(&service.request)
                        && self.dispatch.pending().as_ref() == Some(service),
                    "notification listener service differs"
                ),
                Service::View(request) => ensure!(
                    self.viewer.as_ref().is_some_and(|v| v.nested.is_none())
                        && *request == open::Request::InvalidateView
                        && presenter.request() == Some(request),
                    "notification view service differs"
                ),
                Service::Camera(service) => ensure!(
                    self.viewer.as_ref().is_some_and(|v| v.nested.is_none())
                        && broadcast(service).is_none()
                        && presenter.pending_camera() == Some(service),
                    "notification camera service differs"
                ),
            }
        }
        ensure!(
            !self.complete()
                || (self.viewer.is_none() && self.initializer.is_none() && self.pending.is_none()),
            "completed notification retains child work"
        );
        Ok(())
    }
    fn sync_view(&mut self, view: &View) {
        if let Some(p) = &view.presenter {
            self.dispatch.fields.include_visible = p.view.floor.visibility.include_visible;
        }
    }
}
impl Session {
    /// Bind the replacement application's actual listeners to an idle viewer.
    /// This does not dispatch F6 or claim original process startup has run.
    pub fn bind_house_listeners(
        &mut self,
        registry: Registry<Receiver>,
        fields: Fields,
    ) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.view_refresh_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending(),
            "cannot bind house listeners during pending work"
        );
        self.install_house_context(registry, fields)
    }
    pub(super) fn install_house_context(
        &mut self,
        registry: Registry<Receiver>,
        fields: Fields,
    ) -> Result<()> {
        ensure!(
            !self.house_notification_pending(),
            "cannot replace an active notification context"
        );
        let state = State {
            dispatch: Dispatch::idle(registry, fields).map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: None,
            creation: None,
            cursor_creation: None,
            initializer: None,
        };
        state.validate(&self.host.presentation, &self.simulation)?;
        self.host.presentation.notifications = Some(state);
        self.simulation.native_selection_broadcasts = true;
        Ok(())
    }
    pub fn house_notification_pending(&self) -> bool {
        self.host.presentation.object_budget.is_some()
            || self
                .host
                .presentation
                .notifications
                .as_ref()
                .is_some_and(|n| !n.complete())
    }
    /// Retain real host camera inputs without prematurely entering F6. The
    /// subsequent broadcast starts the viewer only when its listener is reached.
    pub fn configure_house_view(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Order,
        camera: camera::State,
    ) -> Result<()> {
        self.configure_house_view_order(view, inputs, Some(order), camera)
    }
    ///New registry-owned profile; no fresh viewer cache may replace it.
    pub fn configure_registry_house_view(
        &mut self,
        view: open::State,
        inputs: Inputs,
        camera: camera::State,
    ) -> Result<()> {
        ensure!(
            self.simulation.vm.world.registry_order.is_some(),
            "registry cache owner absent"
        );
        self.configure_house_view_order(view, inputs, None, camera)
    }
    fn configure_house_view_order(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Option<Order>,
        camera: camera::State,
    ) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_active(),
            "load completion owns the view context"
        );
        ensure!(!self.postload_pending(), "post-load owns its view context");
        ensure!(
            !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending(),
            "house view has pending work"
        );
        let state = presenter::State::idle(view, inputs, order, camera)?;
        state
            .validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        self.host.presentation.presenter = Some(state);
        if let Some(n) = &mut self.host.presentation.notifications {
            n.dispatch.fields.include_visible = self
                .host
                .presentation
                .presenter
                .as_ref()
                .unwrap()
                .view
                .floor
                .visibility
                .include_visible;
        }
        Ok(())
    }
    /// Explicit original40760d. Registry order and UI fields are host inputs,
    /// not guessed from imported object order. No ordinary tick runs inside it.
    pub fn begin_house_notification(
        &mut self,
        registry: Registry<Receiver>,
        fields: Fields,
        event: u32,
        value: u32,
    ) -> Result<()> {
        self.check_house_notification(event)?;
        self.install_house_notification(registry, fields, event, value)
    }
    pub(super) fn check_house_notification(&self, event: u32) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_pending(),
            "load completion owns house broadcasts"
        );
        ensure!(
            !self.postload_pending(),
            "post-load callbacks have not returned"
        );
        ensure!(
            !self.options.native_postload
                || !self.host.presentation.awaiting_house_open
                || self
                    .host
                    .presentation
                    .postload
                    .as_ref()
                    .is_some_and(super::postload::State::complete),
            "native post-load tail must precede house opening"
        );
        ensure!(
            !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending(),
            "house notification has pending work"
        );
        ensure!(
            !self.simulation.cursor_work_pending(),
            "cursor factory owns application notifications"
        );
        ensure!(
            [0xf6, 0xe4, 0xe2, 0xf9, 0x83, 0x84, 0xc2, 0x109, 0xda, 0xee].contains(&(event as u16)),
            "house message is not implemented"
        );
        ensure!(
            !self.host.presentation.awaiting_house_open || event as u16 == 0xf6,
            "deferred house requires its F6 opening broadcast"
        );
        Ok(())
    }
    pub(super) fn install_house_notification(
        &mut self,
        registry: Registry<Receiver>,
        fields: Fields,
        event: u32,
        value: u32,
    ) -> Result<()> {
        let state = State {
            dispatch: Dispatch::begin(registry, fields, event, value)
                .map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: None,
            creation: None,
            cursor_creation: None,
            initializer: None,
        };
        state.validate(&self.host.presentation, &self.simulation)?;
        self.host.presentation.notifications = Some(state);
        self.simulation.native_selection_broadcasts = true;
        Ok(())
    }
    /// Enter the captured E4 call from either ordinary selection or deletion's
    /// ClearSelection child. The parent remains pending until all listeners return.
    pub(super) fn begin_selection_broadcast(&mut self) -> Result<()> {
        ensure!(
            !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.host.ordinary.updates_pending(),
            "selection broadcast overlaps presentation work"
        );
        let (call, started) = self
            .simulation
            .pending_selection_notification()
            .context("selection notification absent")?;
        ensure!(
            !started && self.simulation.native_selection_broadcasts,
            "selection notification already started or dispatcher absent"
        );
        let previous = self
            .host
            .presentation
            .notifications
            .as_ref()
            .context("application notification registry absent")?;
        previous.validate(&self.host.presentation, &self.simulation)?;
        let state = State {
            dispatch: Dispatch::begin(
                previous.dispatch.registry.clone(),
                previous.dispatch.fields.clone(),
                sims_vm::selection::SIGNAL,
                call.payload as u32,
            )
            .map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: Some(call),
            creation: None,
            cursor_creation: None,
            initializer: None,
        };
        self.simulation
            .start_selection_notification(call)
            .map_err(anyhow::Error::msg)?;
        // Retain ownership before any validation failure is reported. No native
        // notification effects have run yet and the parent cannot disappear.
        self.host.presentation.notifications = Some(state);
        self.host
            .presentation
            .notifications
            .as_ref()
            .unwrap()
            .validate(&self.host.presentation, &self.simulation)
    }
    /// Dispatch the live application's E2 before direct440b2d may return.
    pub(super) fn begin_factory_broadcast(&mut self, id: sims_vm::ObjectId) -> Result<()> {
        ensure!(
            !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.host.ordinary.updates_pending(),
            "factory broadcast overlaps presentation work"
        );
        ensure!(
            self.simulation
                .native_selection
                .as_ref()
                .and_then(sims_sim::lot_selection::Execution::direct_factory)
                .is_some()
                && self.simulation.pending_native_selection()
                    == Some((
                        sims_sim::lot_selection::Request::CreationNotification(id),
                        false
                    )),
            "direct factory notification absent"
        );
        let previous = self
            .host
            .presentation
            .notifications
            .as_ref()
            .context("application notification registry absent")?;
        previous.validate(&self.host.presentation, &self.simulation)?;
        let state = State {
            dispatch: Dispatch::begin(
                previous.dispatch.registry.clone(),
                previous.dispatch.fields.clone(),
                0xe2,
                id as u32,
            )
            .map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: None,
            creation: Some(id),
            cursor_creation: None,
            initializer: None,
        };
        self.simulation
            .start_indicator_created(id)
            .map_err(anyhow::Error::msg)?;
        self.host.presentation.notifications = Some(state);
        self.host
            .presentation
            .notifications
            .as_ref()
            .unwrap()
            .validate(&self.host.presentation, &self.simulation)
    }
    pub(super) fn begin_cursor_broadcast(&mut self, cursor: u32) -> Result<()> {
        ensure!(
            !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.host.ordinary.updates_pending(),
            "cursor broadcast overlaps presentation work"
        );
        ensure!(
            self.simulation
                .cursor_factory
                .as_ref()
                .and_then(sims_sim::lot_cursor::Factory::pending_created)
                == Some((cursor, false))
                && self.host.presentation.object_focus == Some(cursor),
            "cursor notification or focus differs"
        );
        let id = self
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .and_then(|r| r.object_for_handle(cursor))
            .context("cursor notification lifetime ended")?;
        let previous = self
            .host
            .presentation
            .notifications
            .as_ref()
            .context("application notification registry absent")?;
        previous.validate(&self.host.presentation, &self.simulation)?;
        let state = State {
            dispatch: Dispatch::begin(
                previous.dispatch.registry.clone(),
                previous.dispatch.fields.clone(),
                0xe2,
                id as u32,
            )
            .map_err(anyhow::Error::msg)?,
            viewer: None,
            pending: None,
            selection: None,
            creation: None,
            cursor_creation: Some(cursor),
            initializer: None,
        };
        self.simulation
            .start_cursor_created(cursor)
            .map_err(anyhow::Error::msg)?;
        self.host.presentation.notifications = Some(state);
        self.host
            .presentation
            .notifications
            .as_ref()
            .unwrap()
            .validate(&self.host.presentation, &self.simulation)
    }
    fn finish_house_notification(&mut self, state: &mut State, value: u16) -> Result<Progress> {
        if state
            .dispatch
            .root_message()
            .is_some_and(|(e, _)| e as u16 == 0xf6)
        {
            self.host.presentation.awaiting_house_open = false;
        }
        if let Some(call) = state.selection {
            self.simulation
                .complete_selection_notification(call)
                .map_err(anyhow::Error::msg)?;
            state.selection = None;
        }
        if let Some(id) = state.creation {
            self.simulation
                .complete_indicator_created(id)
                .map_err(anyhow::Error::msg)?;
            state.creation = None;
        }
        if let Some(h) = state.cursor_creation {
            self.simulation
                .complete_cursor_created(h)
                .map_err(anyhow::Error::msg)?;
            state.cursor_creation = None;
        }
        Ok(Progress::Complete(value))
    }
    pub fn acknowledge_house_notification(
        &mut self,
        service: &Service,
        reply: Reply,
    ) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        self.host
            .presentation
            .notifications
            .as_ref()
            .context("house notification absent")?
            .validate(&self.host.presentation, &self.simulation)?;
        let mut state = self
            .host
            .presentation
            .notifications
            .take()
            .context("house notification absent")?;
        let result = (|| {
            state.validate(&self.host.presentation, &self.simulation)?;
            ensure!(
                state.pending.as_ref() == Some(service),
                "house notification acknowledgement mismatch"
            );
            match (service, reply) {
                (Service::Listener(call), Reply::Done) if !internal(&call.request) => {
                    state
                        .dispatch
                        .acknowledge(call)
                        .map_err(anyhow::Error::msg)?;
                }
                (Service::View(request), Reply::View(view)) => {
                    self.acknowledge_view_refresh(request, view)?
                }
                (Service::Camera(request), Reply::Camera(reply)) => {
                    self.acknowledge_view_camera_service(request, reply)?
                }
                _ => anyhow::bail!("house notification reply kind differs"),
            }
            state.pending = None;
            Ok(())
        })();
        state.sync_view(&self.host.presentation);
        self.host.presentation.notifications = Some(state);
        result
    }
    pub fn advance_house_notification(&mut self, camera: Input, budget: usize) -> Result<Progress> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(budget > 0, "house notification requires a positive budget");
        self.host
            .presentation
            .notifications
            .as_ref()
            .context("house notification absent")?
            .validate(&self.host.presentation, &self.simulation)?;
        let mut state = self
            .host
            .presentation
            .notifications
            .take()
            .context("house notification absent")?;
        // Restore the owner even on error: earlier original writes/callbacks
        // must not disappear because a later asset/device service failed.
        let result = self.advance_house_notification_inner(&mut state, camera, budget);
        state.sync_view(&self.host.presentation);
        self.host.presentation.notifications = Some(state);
        result
    }
    fn advance_house_notification_inner(
        &mut self,
        state: &mut State,
        camera: Input,
        budget: usize,
    ) -> Result<Progress> {
        state.validate(&self.host.presentation, &self.simulation)?;
        camera.build().map_err(anyhow::Error::msg)?;
        if let Some(service) = &state.pending {
            return Ok(Progress::Service(service.clone()));
        }
        for _ in 0..budget {
            if state
                .initializer
                .as_ref()
                .is_some_and(|i| i.nested.is_none())
            {
                let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                self.simulation
                    .advance_native_selection(&self.program, &ordinary, 1)
                    .map_err(anyhow::Error::msg)?;
                if self.simulation.native_selection.is_none() {
                    let caller = state.initializer.as_ref().unwrap().caller.clone();
                    state
                        .dispatch
                        .acknowledge(&caller)
                        .map_err(anyhow::Error::msg)?;
                    state.initializer = None;
                } else if let Some((
                    sims_sim::lot_selection::Request::CreationNotification(id),
                    false,
                )) = self.simulation.pending_native_selection()
                {
                    state
                        .dispatch
                        .nest(0xe2, id as u32)
                        .map_err(anyhow::Error::msg)?;
                    self.simulation
                        .start_indicator_created(id)
                        .map_err(anyhow::Error::msg)?;
                    state.initializer.as_mut().unwrap().nested = Some(id);
                } else {
                    self.service_selection_assets(camera, 1)?;
                }
            } else if state.viewer.as_ref().is_some_and(|v| v.nested.is_none()) {
                // Camera transitions may have changed these fields earlier in
                // this same work budget. The indicator drain reads the live
                // viewer, not the client input captured before entering F6.
                let view = &self.host.presentation.presenter.as_ref().unwrap().view;
                let live_camera = Input {
                    center: view.origin,
                    zoom: view.zoom,
                    rotation: view.display_rotation,
                    ..camera
                };
                let result = self.advance_view_refresh(live_camera, 1);
                state.sync_view(&self.host.presentation);
                match result? {
                    presenter::Progress::Pending => (),
                    presenter::Progress::Complete => {
                        let viewer = state.viewer.as_ref().unwrap();
                        state
                            .dispatch
                            .acknowledge(&viewer.caller)
                            .map_err(anyhow::Error::msg)?;
                        state.viewer = None;
                    }
                    presenter::Progress::Service(request) => {
                        state.pending = Some(Service::View(request))
                    }
                    presenter::Progress::CameraService(service) => {
                        if let Some((event, value)) = broadcast(&service) {
                            state
                                .dispatch
                                .nest(event, value)
                                .map_err(anyhow::Error::msg)?;
                            state.viewer.as_mut().unwrap().nested = Some(service);
                        } else {
                            state.pending = Some(Service::Camera(service));
                        }
                    }
                }
            } else {
                let selected = self
                    .simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    .is_some();
                match state
                    .dispatch
                    .advance(
                        &mut self.simulation.vm.world.globals,
                        selected,
                        1,
                        |_, _, _, _| {},
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    dispatch::Progress::Pending => (),
                    dispatch::Progress::Complete(value) => {
                        return self.finish_house_notification(state, value);
                    }
                    dispatch::Progress::NestedComplete(_) => {
                        if let Some(id) = state.initializer.as_ref().and_then(|i| i.nested) {
                            self.simulation
                                .complete_indicator_created(id)
                                .map_err(anyhow::Error::msg)?;
                            state.dispatch.finish_nested().map_err(anyhow::Error::msg)?;
                            state.initializer.as_mut().unwrap().nested = None;
                            continue;
                        }
                        let service = state
                            .viewer
                            .as_ref()
                            .and_then(|v| v.nested.as_ref())
                            .context("nested notification camera absent")?
                            .clone();
                        self.acknowledge_view_camera_service(&service, camera::Reply::Done)?;
                        state.dispatch.finish_nested().map_err(anyhow::Error::msg)?;
                        state.viewer.as_mut().unwrap().nested = None;
                    }
                    dispatch::Progress::Service(call) => match call.request {
                        Request::WorldCreated { value } => {
                            sims_vm::family::created(
                                &mut self.simulation.vm.world,
                                &self.program,
                                value,
                            )
                            .map_err(anyhow::Error::msg)?;
                            state
                                .dispatch
                                .acknowledge(&call)
                                .map_err(anyhow::Error::msg)?;
                        }
                        Request::ViewerRefresh => {
                            self.host
                                .presentation
                                .presenter
                                .as_mut()
                                .context("notification presenter absent")?
                                .restart()?;
                            state.viewer = Some(Viewer {
                                caller: call,
                                nested: None,
                            });
                        }
                        Request::Compose { include_visible } => {
                            let selected = presenter::selected_for_visibility(&self.simulation)?;
                            let p = self
                                .host
                                .presentation
                                .presenter
                                .as_mut()
                                .context("notification presenter absent")?;
                            let retained =
                                p.camera.as_ref().context("notification camera absent")?;
                            retained
                                .compose(
                                    &mut p.view,
                                    &camera::Inputs {
                                        roof_controls_present: self
                                            .host
                                            .presentation
                                            .transport
                                            .is_some(),
                                        lot: self
                                            .simulation
                                            .architecture
                                            .as_ref()
                                            .and_then(|a| a.original.as_ref())
                                            .context("notification lot absent")?,
                                        rooms: &self
                                            .simulation
                                            .room_environment
                                            .as_ref()
                                            .context("notification rooms absent")?
                                            .manager,
                                        application_mode: retained.settings.application_mode,
                                        selected: selected.as_ref(),
                                        clock: retained.settings.clock,
                                    },
                                    include_visible,
                                )
                                .map_err(anyhow::Error::msg)?;
                            state
                                .dispatch
                                .acknowledge(&call)
                                .map_err(anyhow::Error::msg)?;
                        }
                        Request::InitializeIndicators => {
                            self.simulation
                                .begin_native_indicator_initialization()
                                .map_err(anyhow::Error::msg)?;
                            state.initializer = Some(Initializer {
                                caller: call,
                                nested: None,
                            });
                        }
                        _ => state.pending = Some(Service::Listener(call)),
                    },
                }
                state.sync_view(&self.host.presentation);
            }
            if let Some(service) = &state.pending {
                return Ok(Progress::Service(service.clone()));
            }
        }
        state.validate(&self.host.presentation, &self.simulation)?;
        if let Some(value) = state.dispatch.returned() {
            self.finish_house_notification(state, value)
        } else {
            Ok(Progress::Pending)
        }
    }
}
