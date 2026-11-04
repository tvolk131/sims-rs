//! Retained F6 caller for original-house sessions. The client must actually
//! service camera/floor/wall requests before acknowledging them. Classification
//! and the final indicator drain borrow the live simulation and asset owners.
//! No ordinary tick or load silently dispatches this presentation operation.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    lot_presenter::{Inputs, View, camera, open, person_refresh, transition, visibility},
    pose::portrait::view::Input,
};
use sims_vm::presenter::{Classification, Lists, Order};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub view: open::State,
    pub inputs: Inputs,
    ///Legacy viewer-owned cache. New native profiles use World.registry_order
    ///and leave this absent; the two saved representations are never merged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Order>,
    pub lists: Lists,
    owner: open::Cursor,
    classification: Option<Classification>,
    drain_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<camera::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera_call: Option<camera::Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    people: Option<person_refresh::Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    normal_frame: Option<NormalFrame>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NormalFrame {
    tick: u64,
    phase: FramePhase,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FramePhase {
    Environment,
    Indicators,
    Classification,
}
impl NormalFrame {
    fn require_indicator_child(&self, tick: u64, drain_started: bool) -> Result<()> {
        ensure!(
            self.tick == tick && self.phase == FramePhase::Indicators && drain_started,
            "normal-frame indicator child lost its caller"
        );
        Ok(())
    }
}
#[cfg(test)]
#[path = "presenter/normal_frame_indicator_tests.rs"]
mod normal_frame_indicator_tests;
impl State {
    pub(super) fn idle(
        view: open::State,
        inputs: Inputs,
        order: Option<Order>,
        camera: camera::State,
    ) -> Result<Self> {
        let state = Self {
            view,
            inputs,
            order,
            lists: Lists::default(),
            owner: open::Cursor::default(),
            classification: None,
            drain_started: false,
            camera: Some(camera),
            camera_call: None,
            people: None,
            normal_frame: None,
        };
        state.validate().map_err(anyhow::Error::msg)?;
        Ok(state)
    }
    pub(super) fn restart(&mut self) -> Result<()> {
        ensure!(self.complete(), "house viewer already running");
        self.validate().map_err(anyhow::Error::msg)?;
        self.owner = open::Cursor::begin(&self.view).map_err(anyhow::Error::msg)?;
        Ok(())
    }
    pub(super) fn pending_camera(&self) -> Option<&camera::Service> {
        self.camera_call.as_ref().and_then(camera::Cursor::pending)
    }
    pub fn pending_person(&self) -> Option<sims_vm::ObjectId> {
        self.people
            .as_ref()
            .and_then(person_refresh::Cursor::pending)
    }
    pub(super) fn owns_indicator_drain(&self) -> bool {
        self.drain_started
    }
    pub fn validate(&self) -> Result<(), String> {
        self.view.floor.validate()?;
        if let Some(order) = &self.order {
            order.validate()?;
        }
        if let Some(camera) = &self.camera {
            camera.validate(&self.view)?;
        }
        if let Some(call) = &self.camera_call {
            call.validate(
                self.camera.as_ref().ok_or("camera context absent")?,
                &self.view,
            )?;
            if self.owner.request() != Some(call.request()) {
                return Err("orphan house-view camera call".into());
            }
        }
        if let Some(classify) = &self.classification {
            classify.validate()?;
            if self.owner.request() != Some(&open::Request::ClassifyObjects)
                && !self
                    .normal_frame
                    .as_ref()
                    .is_some_and(|f| f.phase == FramePhase::Classification)
            {
                return Err("orphan house-view classification".into());
            }
        }
        if self.people.is_some()
            && self.pending_camera()
                != Some(&camera::Service::Transition(
                    transition::Request::RefreshDrawables,
                ))
        {
            return Err("house-view person refresh lost its caller".into());
        }
        if self.drain_started
            && self.owner.request() != Some(&open::Request::DrainIndicators)
            && !self
                .normal_frame
                .as_ref()
                .is_some_and(|f| f.phase == FramePhase::Indicators)
        {
            return Err("orphan house-view indicator drain".into());
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.owner.complete() && self.normal_frame.is_none()
    }
    pub(super) fn normal_frame_active(&self) -> bool {
        self.normal_frame.is_some()
    }
    pub fn request(&self) -> Option<&open::Request> {
        self.owner.request()
    }
    pub fn validate_simulation(&self, simulation: &sims_sim::Simulation) -> Result<(), String> {
        self.validate()?;
        if self.order.is_some() == simulation.vm.world.registry_order.is_some() {
            return Err("house view must have exactly one spatial-cache owner".into());
        }
        simulation.vm.world.validate_registry_order()?;
        if let Some(frame) = &self.normal_frame {
            if !self.owner.complete() || self.order.is_some() || frame.tick != simulation.vm.tick {
                return Err("normal-frame classification lost its native caller".into());
            }
        }
        if let Some(people) = &self.people {
            people.validate(&person_refresh::Live {
                world: &simulation.vm.world,
                size: 64,
                level: self.view.floor.level,
            })?;
        }
        if let Some(camera) = &self.camera {
            if camera.settings.world_rotation != u32::from(simulation.vm.world.camera_rotation)
                || !simulation
                    .vm
                    .world
                    .refresh
                    .as_ref()
                    .is_some_and(|r| r.visible_level == self.view.floor.level)
            {
                return Err("retained camera differs from live world view".into());
            }
        }
        if self.drain_started
            && !simulation
                .native_selection
                .as_ref()
                .is_some_and(|s| s.is_refresh())
        {
            return Err("house-view indicator drain lost its child".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Service(open::Request),
    CameraService(camera::Service),
    Pending,
    Complete,
}
fn external(request: &open::Request) -> bool {
    !matches!(
        request,
        open::Request::ClassifyObjects | open::Request::DrainIndicators
    )
}
impl Session {
    /// Only the already-retained normal-frame Indicators child may coexist
    /// with a ready running menu. Synchronous input/query/factory work still
    /// excludes it, and the independent public selection API is unchanged.
    fn advance_normal_frame_indicators(
        &mut self,
        state: &State,
        camera: Input,
        budget: usize,
    ) -> Result<bool> {
        state
            .normal_frame
            .as_ref()
            .context("normal-frame owner absent")?
            .require_indicator_child(self.simulation.vm.tick, state.drain_started)?;
        ensure!(
            self.host.presentation.presenter.is_none()
                && self
                    .simulation
                    .native_selection
                    .as_ref()
                    .is_some_and(|s| s.is_refresh()),
            "normal-frame indicator selection owner absent"
        );
        ensure!(
            !self.player_input_blocks_ticks(),
            "synchronous player input owns normal-frame indicators"
        );
        self.advance_selection_work(camera, budget)
    }
    /// Shared4b1bda child of both F6 and UI rotation. Exactly one bounded
    /// character service per call; the enclosing caller owns acknowledgement.
    pub(super) fn advance_camera_people(
        &mut self,
        people: &mut person_refresh::Cursor,
        level: i32,
    ) -> Result<bool> {
        self.advance_camera_people_with_selection_notification(people, level, None)
    }
    pub(super) fn advance_camera_people_with_selection_notification(
        &mut self,
        people: &mut person_refresh::Cursor,
        level: i32,
        selection: Option<sims_sim::lot_selection::NotificationViewRefresh>,
    ) -> Result<bool> {
        let id = people
            .advance(&person_refresh::Live {
                world: &self.simulation.vm.world,
                size: 64,
                level,
            })
            .map_err(anyhow::Error::msg)?;
        let Some(id) = id else {
            return Ok(true);
        };
        let complete = match selection {
            Some(token) => self.simulation.refresh_selection_notification_view_person(
                &self.program,
                id,
                token,
                1,
            ),
            None => self.simulation.refresh_view_person(&self.program, id, 1),
        }
        .map_err(anyhow::Error::msg)?;
        if complete {
            people.acknowledge(id).map_err(anyhow::Error::msg)?;
        } else {
            self.host.service_view_person(&mut self.simulation, id)?;
        }
        Ok(false)
    }
    pub fn view_refresh_pending(&self) -> bool {
        self.host
            .presentation
            .presenter
            .as_ref()
            .is_some_and(|p| !p.complete())
    }
    /// Dispatch an explicit native F6 with the caller's actual view and registry
    /// cache. Native object+188 values belong to the caller; classification
    /// refreshes graphic readiness from the live asset owner before each slice.
    /// Neither is inferred from sprite presence or an imported suspended process.
    pub fn begin_view_refresh(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Order,
    ) -> Result<()> {
        self.begin_view_refresh_order(view, inputs, Some(order))
    }
    /// Explicit F6 using the live registry's retained cache, without replacing
    /// it with a newly constructed viewer cache.
    pub fn begin_registry_view_refresh(&mut self, view: open::State, inputs: Inputs) -> Result<()> {
        ensure!(
            self.simulation.vm.world.registry_order.is_some(),
            "registry cache owner absent"
        );
        self.begin_view_refresh_order(view, inputs, None)
    }
    fn begin_view_refresh_order(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Option<Order>,
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
            !self.load_completion_pending(),
            "load completion owns the viewer"
        );
        ensure!(
            !self.postload_pending(),
            "post-load must return before viewer refresh"
        );
        ensure!(
            !self.house_notification_pending(),
            "house notification owns the viewer"
        );
        ensure!(
            !self.view_refresh_pending(),
            "native view refresh already pending"
        );
        ensure!(
            !self.simulation.selection_work_pending(),
            "selection must finish before view refresh"
        );
        ensure!(
            !self.host.ordinary.updates_pending(),
            "ordinary graphics must finish before view refresh"
        );
        let owner = open::Cursor::begin(&view).map_err(anyhow::Error::msg)?;
        let state = State {
            view,
            inputs,
            order,
            //4aacb9 clears these when classification starts, after the camera
            //services. Beginning a repeated F6 does not clear them early.
            lists: self
                .host
                .presentation
                .presenter
                .as_ref()
                .map(|p| p.lists.clone())
                .unwrap_or_default(),
            owner,
            classification: None,
            drain_started: false,
            camera: None,
            camera_call: None,
            people: None,
            normal_frame: None,
        };
        state
            .validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        // This legacy explicit viewer entry replaces a completed notification
        // call record. A running broadcast is rejected above and can only enter
        // through its own retained viewer child.
        self.host.presentation.notifications = None;
        self.simulation.native_selection_broadcasts = false;
        self.host.presentation.presenter = Some(state);
        Ok(())
    }
    /// Supply the retained camera context before dispatching the view handler.
    /// This still requires real renderer/UI replies for its device services.
    pub fn begin_view_refresh_with_camera(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Order,
        camera: camera::State,
    ) -> Result<()> {
        self.begin_view_refresh_camera(view, inputs, Some(order), camera)
    }
    pub fn begin_registry_view_refresh_with_camera(
        &mut self,
        view: open::State,
        inputs: Inputs,
        camera: camera::State,
    ) -> Result<()> {
        ensure!(
            self.simulation.vm.world.registry_order.is_some(),
            "registry cache owner absent"
        );
        self.begin_view_refresh_camera(view, inputs, None, camera)
    }
    fn begin_view_refresh_camera(
        &mut self,
        view: open::State,
        inputs: Inputs,
        order: Option<Order>,
        camera: camera::State,
    ) -> Result<()> {
        camera.validate(&view).map_err(anyhow::Error::msg)?;
        ensure!(
            camera.settings.world_rotation == u32::from(self.simulation.vm.world.camera_rotation),
            "camera rotation differs from live world"
        );
        ensure!(
            self.simulation
                .vm
                .world
                .refresh
                .as_ref()
                .is_some_and(|r| r.visible_level == view.floor.level),
            "camera floor differs from live view"
        );
        self.begin_view_refresh_order(view, inputs, order)?;
        self.host.presentation.presenter.as_mut().unwrap().camera = Some(camera);
        Ok(())
    }
    pub fn acknowledge_view_camera_service(
        &mut self,
        request: &camera::Service,
        reply: camera::Reply,
    ) -> Result<()> {
        ensure!(
            *request != camera::Service::Transition(transition::Request::RefreshDrawables),
            "house-view person refresh requires its engine owner"
        );
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.house_notification_pending(),
            "house notification owns the viewer"
        );
        let state = self
            .host
            .presentation
            .presenter
            .as_mut()
            .context("native view refresh absent")?;
        state.validate().map_err(anyhow::Error::msg)?;
        let call = state
            .camera_call
            .as_mut()
            .context("native camera caller absent")?;
        call.acknowledge(
            state
                .camera
                .as_mut()
                .context("native camera context absent")?,
            &mut state.view,
            request,
            reply,
        )
        .map_err(anyhow::Error::msg)?;
        self.simulation.vm.world.camera_rotation =
            state.camera.as_ref().unwrap().settings.world_rotation as u8;
        self.simulation
            .vm
            .world
            .refresh
            .as_mut()
            .context("native camera view absent")?
            .visible_level = state.view.floor.level;
        Ok(())
    }
    /// Publish mutations from an actual completed client service. Internal
    /// classification/drain requests cannot be skipped through this interface.
    pub fn acknowledge_view_refresh(
        &mut self,
        request: &open::Request,
        mut view: open::State,
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
            !self.house_notification_pending(),
            "house notification owns the viewer"
        );
        ensure!(
            external(request),
            "house-view engine service requires its owner"
        );
        let state = self
            .host
            .presentation
            .presenter
            .as_mut()
            .context("native view refresh absent")?;
        state.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            state.camera.is_none() || *request == open::Request::InvalidateView,
            "house-view camera service requires its retained caller"
        );
        state
            .owner
            .acknowledge(&mut view, request)
            .map_err(anyhow::Error::msg)?;
        state.view = view;
        if state.camera.is_some() {
            self.simulation
                .vm
                .world
                .refresh
                .as_mut()
                .context("native camera view absent")?
                .visible_level = state.view.floor.level;
        }
        Ok(())
    }
    /// Advance the caller without simulation ticks. External services are left
    /// pending; internal callbacks are retained before another budget slice.
    pub fn advance_view_refresh(&mut self, camera: Input, budget: usize) -> Result<Progress> {
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
            "load completion owns the viewer"
        );
        ensure!(
            !self.postload_pending(),
            "post-load must return before viewer refresh"
        );
        ensure!(
            !self.house_notification_pending(),
            "house notification owns the viewer"
        );
        ensure!(budget > 0, "view refresh requires a positive budget");
        ensure!(
            !self
                .host
                .presentation
                .presenter
                .as_ref()
                .is_some_and(State::normal_frame_active)
                || !self.player_input_blocks_ticks(),
            "synchronous player input owns normal-frame refresh"
        );
        let mut state = self
            .host
            .presentation
            .presenter
            .take()
            .context("native view refresh absent")?;
        // Always put the owner back, including after a failed downstream asset
        // service. Native writes already made must not disappear on an error.
        let result = self.advance_view_refresh_inner(&mut state, camera, budget);
        self.host.presentation.presenter = Some(state);
        result
    }
    fn advance_view_refresh_inner(
        &mut self,
        state: &mut State,
        camera: Input,
        budget: usize,
    ) -> Result<Progress> {
        state
            .validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        for _ in 0..budget {
            if let Some(frame) = &state.normal_frame {
                match frame.phase {
                    FramePhase::Environment => {
                        self.update_view_environment()?;
                        state.normal_frame.as_mut().unwrap().phase = FramePhase::Indicators;
                    }
                    FramePhase::Indicators => {
                        ensure!(
                            !self.player_input_blocks_ticks(),
                            "synchronous player input owns normal-frame indicators"
                        );
                        if !state.drain_started {
                            camera.build().map_err(anyhow::Error::msg)?;
                            self.simulation
                                .begin_native_indicator_refresh(true)
                                .map_err(anyhow::Error::msg)?;
                            state.drain_started = true;
                        } else if self.advance_normal_frame_indicators(state, camera, 1)? {
                            state.drain_started = false;
                            state.normal_frame.as_mut().unwrap().phase = FramePhase::Classification;
                        }
                    }
                    FramePhase::Classification => {
                        if self.advance_object_classification(state)? {
                            state.normal_frame = None;
                            return Ok(Progress::Complete);
                        }
                    }
                }
                continue;
            }
            if let Some(people) = &mut state.people {
                if self.advance_camera_people(people, state.view.floor.level)? {
                    state
                        .camera_call
                        .as_mut()
                        .unwrap()
                        .acknowledge(
                            state.camera.as_mut().unwrap(),
                            &mut state.view,
                            &camera::Service::Transition(transition::Request::RefreshDrawables),
                            camera::Reply::Done,
                        )
                        .map_err(anyhow::Error::msg)?;
                    state.people = None;
                }
                continue;
            }
            let Some(request) = state.owner.request().cloned() else {
                return Ok(Progress::Complete);
            };
            if external(&request) {
                if state.camera.is_some() && request != open::Request::InvalidateView {
                    if state.camera_call.is_none() {
                        state.camera_call = Some(
                            camera::Cursor::begin(
                                request.clone(),
                                state.camera.as_mut().unwrap(),
                                &mut state.view,
                            )
                            .map_err(anyhow::Error::msg)?,
                        );
                    }
                    let selected = selected_for_visibility(&self.simulation)?;
                    let retained = state.camera.as_mut().unwrap();
                    let inputs = camera::Inputs {
                        roof_controls_present: self.host.presentation.transport.is_some(),
                        lot: self
                            .simulation
                            .architecture
                            .as_ref()
                            .and_then(|a| a.original.as_ref())
                            .context("camera lot absent")?,
                        rooms: &self
                            .simulation
                            .room_environment
                            .as_ref()
                            .context("camera room manager absent")?
                            .manager,
                        application_mode: retained.settings.application_mode,
                        selected: selected.as_ref(),
                        clock: retained.settings.clock,
                    };
                    let result = state.camera_call.as_mut().unwrap().advance(
                        retained,
                        &mut state.view,
                        &inputs,
                        1,
                    );
                    // Native field stores precede outward callbacks, including
                    // a nested floor switch. Keep the VM's borrowed view current.
                    self.simulation.vm.world.camera_rotation =
                        retained.settings.world_rotation as u8;
                    self.simulation
                        .vm
                        .world
                        .refresh
                        .as_mut()
                        .context("native camera view absent")?
                        .visible_level = state.view.floor.level;
                    match result.map_err(anyhow::Error::msg)? {
                        camera::Progress::Pending => (),
                        camera::Progress::Service(service) => {
                            if service
                                == camera::Service::Transition(
                                    transition::Request::RefreshDrawables,
                                )
                            {
                                ensure!(
                                    self.simulation.vm.world.person.is_some(),
                                    "house-view person registry absent"
                                );
                                state.people = Some(person_refresh::Cursor::default());
                            } else {
                                return Ok(Progress::CameraService(service));
                            }
                        }
                        camera::Progress::Complete => {
                            state.camera_call = None;
                            state
                                .owner
                                .acknowledge(&mut state.view, &request)
                                .map_err(anyhow::Error::msg)?;
                        }
                    }
                    continue;
                }
                return Ok(Progress::Service(request));
            }
            match request {
                open::Request::ClassifyObjects => {
                    if self.advance_object_classification(state)? {
                        state
                            .owner
                            .acknowledge(&mut state.view, &request)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                open::Request::DrainIndicators => {
                    if !state.drain_started {
                        camera.build().map_err(anyhow::Error::msg)?;
                        // This is the retained F6 child, which runs before
                        // house opening returns. The public entry is reserved
                        // for an independent drain after opening has finished.
                        self.simulation
                            .begin_native_indicator_refresh(true)
                            .map_err(anyhow::Error::msg)?;
                        state.drain_started = true;
                    } else if self.advance_selection(camera, 1)? {
                        state.drain_started = false;
                        state
                            .owner
                            .acknowledge(&mut state.view, &request)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                _ => unreachable!(),
            }
        }
        state.validate().map_err(anyhow::Error::msg)?;
        Ok(if state.complete() {
            Progress::Complete
        } else {
            Progress::Pending
        })
    }
    fn advance_object_classification(&mut self, state: &mut State) -> Result<bool> {
        //Camera callbacks can load/retire graphics. Query the live+180 owner.
        state.inputs.graphic_ready = self
            .host
            .ordinary
            .presenter_readiness(&self.simulation.vm, &self.program)?;
        let links = &self
            .simulation
            .native_lot
            .as_ref()
            .context("presenter lot absent")?
            .placement
            .links;
        let world = &mut self.simulation.vm.world;
        if state.classification.is_none() {
            if let Some(mut cache) = world.registry_order.take() {
                //The getter only reads its view. Put its owner back on errors,
                //including native partial rebuild state; never roll it back.
                let result = (|| {
                    let view = View::new(world, &self.program, links, &state.inputs, &[])
                        .map_err(anyhow::Error::msg)?;
                    cache.get(&view).map_err(anyhow::Error::msg)?;
                    Ok::<_, anyhow::Error>(())
                })();
                world.registry_order = Some(cache);
                result?;
            } else {
                state
                    .order
                    .as_mut()
                    .context("legacy presenter cache absent")?
                    .get(
                        &View::new(world, &self.program, links, &state.inputs, &[])
                            .map_err(anyhow::Error::msg)?,
                    )
                    .map_err(anyhow::Error::msg)?;
            }
            state.classification = Some(Classification::default());
        }
        //This small read-only ID list permits borrowing the live World for
        //invalidation. It is not another retained or authoritative cache.
        let order = match (&world.registry_order, &state.order) {
            (Some(cache), None) => cache.objects.clone(),
            (None, Some(cache)) => cache.objects.clone(),
            _ => anyhow::bail!("ambiguous presenter spatial-cache ownership"),
        };
        let mut view = View::new(world, &self.program, links, &state.inputs, &order)
            .map_err(anyhow::Error::msg)?;
        let child = state.classification.as_mut().unwrap();
        if let Some(id) = child.pending() {
            view.invalidate(id).map_err(anyhow::Error::msg)?;
            child.acknowledge(id).map_err(anyhow::Error::msg)?;
        } else if child
            .advance(&mut view, &mut state.lists)
            .map_err(anyhow::Error::msg)?
            .is_none()
        {
            ensure!(
                child.complete(),
                "presenter classification returned before completion"
            );
            state.classification = None;
            return Ok(true);
        }
        Ok(false)
    }
    ///Explicit4a8b01..4a8b1e normal-frame sequence: environment, indicator drain,
    ///then classification. Pure rendering/restoration never dispatch this.
    pub fn begin_normal_view_frame(&mut self) -> Result<()> {
        ensure!(
            !self.player_input_blocks_ticks(),
            "synchronous player input owns normal-frame refresh"
        );
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.clock_pending()
                && !self.postload_pending()
                && !self.load_completion_pending()
                && !self.host.ordinary.updates_pending()
                && self.simulation.vm.scheduled_object_visit.is_none(),
            "suspended caller owns normal-frame classification"
        );
        ensure!(
            self.simulation.vm.world.registry_order.is_some(),
            "normal-frame registry owner absent"
        );
        ensure!(
            !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.simulation.selection_work_pending(),
            "normal frame overlaps synchronous view work"
        );
        let state = self
            .host
            .presentation
            .presenter
            .as_mut()
            .context("normal-frame presenter absent")?;
        state
            .validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        ensure!(state.complete(), "normal-frame presenter already running");
        state.normal_frame = Some(NormalFrame {
            tick: self.simulation.vm.tick,
            phase: FramePhase::Environment,
        });
        Ok(())
    }
    pub fn normal_view_frame_pending(&self) -> bool {
        self.host
            .presentation
            .presenter
            .as_ref()
            .is_some_and(|p| p.normal_frame.is_some())
    }
    ///Bounded driver for an actual frontend frame. The resumable begin/advance
    ///API retains safe mid-frame checkpoints; sampling an image never calls it.
    pub fn prepare_view_frame(&mut self) -> Result<()> {
        if self.simulation.vm.world.registry_order.is_none() {
            self.update_view_environment()?;
            return Ok(());
        }
        if !self.normal_view_frame_pending() {
            self.begin_normal_view_frame()?;
        }
        for _ in 0..30000 {
            let camera = self.retained_camera_input()?;
            match self.advance_view_refresh(camera, 256)? {
                Progress::Complete => return Ok(()),
                Progress::Pending => (),
                p => anyhow::bail!("normal-frame requires an external view service: {p:?}"),
            }
        }
        anyhow::bail!("normal-frame classification budget exceeded")
    }
}

/// Original selected-person lookup and physical fields, shared by F6 camera
/// work and E4 visibility composition. Never substitute the UI selection cache.
pub(super) fn selected_for_visibility(
    simulation: &sims_sim::Simulation,
) -> Result<Option<visibility::Selected>> {
    simulation
        .native_selected_person()
        .map_err(anyhow::Error::msg)?
        .map(|id| -> Result<_> {
            let object = simulation
                .vm
                .world
                .objects
                .get(&id)
                .context("camera selected object absent")?;
            let p = simulation
                .vm
                .world
                .positions
                .get(&id)
                .context("camera selected position absent")?;
            Ok(visibility::Selected {
                room: *object
                    .data
                    .get(&29)
                    .context("camera selected room absent")? as u16,
                level: p.level,
                tile: [(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8],
            })
        })
        .transpose()
}
