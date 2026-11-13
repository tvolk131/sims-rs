//! Saved zoom/rotation/floor UI setter -> native camera -> house notifications.
//! Actual device/UI work remains explicit. No script zoom global is patched here.
use super::{Session, notifications, presenter};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    lot_presenter::{
        camera,
        camera_command::{Request, Ui, Zoom},
        person_refresh, transition,
    },
    pose::portrait::view::Input,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    publication: Option<u64>,
    tick: u64,
    setter: Zoom,
    camera: Option<camera::Cursor>,
    broadcast: Option<camera::Service>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    people: Option<person_refresh::Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    walls: Option<Walls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    portrait: Option<Portrait>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Walls {
    mode: u32,
    next: u8,
    roof: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Portrait {
    Center { point: [i32; 2], level: i32 },
    Roof,
}
impl Portrait {
    fn request(&self) -> sims_sim::lot_presenter::open::Request {
        match *self {
            Self::Center { point, level } => {
                sims_sim::lot_presenter::open::Request::CenterPerson { point, level }
            }
            Self::Roof => sims_sim::lot_presenter::open::Request::Roof {
                enabled: 0,
                force: 0,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Camera(camera::Service),
    Notification(notifications::Service),
    RefreshAudio,
    RefreshControls,
    Complete,
}
#[derive(Debug, Clone, Copy)]
pub enum Change {
    Zoom(u32),
    ZoomIn,
    ZoomOut,
    Rotation(u32),
    Floor(i32),
    Scroll([i32; 2]),
    CenterPerson { point: [i32; 2], level: i32 },
    MotionFrame,
}
impl super::View {
    /// CPState+0d aliases the second byte of the shared dirty word at+0c.
    /// Historical camera-only frontends keep their standalone byte. Once the
    /// CPState owner is bound, the serialized Ui.flags is just its last camera
    /// projection; callbacks/publication may have changed the shared word.
    pub fn camera_properties(&self) -> Result<Ui> {
        let mut ui = self.camera_ui.context("camera UI absent")?;
        if let Some(transport) = &self.transport {
            ui.flags = (transport.controls.buy.dirty >> 8) as u8;
        }
        Ok(ui)
    }
    pub(super) fn edit_camera_properties<T>(
        &mut self,
        edit: impl FnOnce(&mut Ui) -> T,
    ) -> Result<T> {
        let mut ui = self.camera_properties()?;
        let result = edit(&mut ui);
        // Publish the native stores before the caller exposes its next child.
        // Other bytes belong to the same owner and must remain untouched.
        if let Some(transport) = &mut self.transport {
            transport.controls.buy.dirty =
                (transport.controls.buy.dirty & !0xff00) | (u32::from(ui.flags) << 8);
        }
        self.camera_ui = Some(ui);
        Ok(result)
    }
}
impl State {
    pub(super) fn publication(
        tick: u64,
        owner: u64,
        request: sims_sim::lot_presenter::open::Request,
    ) -> Result<Self> {
        Ok(Self {
            publication: Some(owner),
            tick,
            setter: Zoom::publication(request).map_err(anyhow::Error::msg)?,
            camera: None,
            broadcast: None,
            people: None,
            walls: None,
            portrait: None,
        })
    }
    pub(super) fn publication_owner(&self) -> Option<u64> {
        self.publication
    }
    pub(super) fn release_publication(&mut self) -> Result<()> {
        ensure!(self.complete(), "publication camera still running");
        self.publication = None;
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.setter.complete() && self.walls.is_none() && self.portrait.is_none()
    }
    pub fn pending_camera(&self) -> Option<&camera::Service> {
        self.camera.as_ref().and_then(camera::Cursor::pending)
    }
    pub fn pending_person(&self) -> Option<sims_vm::ObjectId> {
        self.people
            .as_ref()
            .and_then(person_refresh::Cursor::pending)
    }
    pub fn uses_frontend_clock(&self) -> bool {
        self.portrait.is_some()
            || self.setter.request()
                == Some(&Request::Camera(sims_sim::lot_presenter::open::Request::MotionFrame))
    }
    fn advance_portrait_tail(&mut self) -> Result<bool> {
        ensure!(
            self.setter.complete()
                && self.camera.is_none()
                && self.broadcast.is_none()
                && self.people.is_none(),
            "portrait camera child has not returned"
        );
        if matches!(self.portrait, Some(Portrait::Center { .. })) {
            let next = Portrait::Roof;
            self.setter = Zoom::publication(next.request()).map_err(anyhow::Error::msg)?;
            self.portrait = Some(next);
            return Ok(true);
        }
        self.portrait = None;
        Ok(false)
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        self.setter.validate().map_err(anyhow::Error::msg)?;
        if let Some(portrait) = &self.portrait {
            ensure!(
                self.publication.is_none()
                    && self.walls.is_none()
                    && self.setter.direct()
                    && (self.setter.complete()
                        || self.setter.request() == Some(&Request::Camera(portrait.request()))),
                "portrait camera lost its native caller"
            );
            ensure!(
                session
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .and_then(|presenter| presenter.camera.as_ref())
                    .is_some_and(|camera| camera.settings.roof.is_some()),
                "portrait camera roof owner absent"
            );
        }
        if let Some(walls) = &self.walls {
            ensure!(
                self.publication.is_none()
                    && self.setter.direct()
                    && matches!(walls.mode, 1 | 2 | 4)
                    && (1..=3).contains(&walls.next),
                "wall camera command lost its native children"
            );
        }
        if let Some(owner) = self.publication {
            ensure!(
                self.setter.direct(),
                "publication camera acquired a setter tail"
            );
            let parent = session
                .host
                .presentation
                .publication
                .as_ref()
                .context("camera publication parent absent")?;
            ensure!(
                parent.camera_owner() == Some(owner),
                "camera publication owner differs"
            );
            if !self.complete() {
                ensure!(
                    self.setter.request()
                        == parent
                            .camera_request()
                            .as_ref()
                            .map(|r| Request::Camera(r.clone()))
                            .as_ref(),
                    "publication camera arguments differ"
                );
            }
        }
        let p = &session.host.presentation;
        ensure!(p.camera_ui.is_some(), "camera command UI absent");
        if self.complete() {
            ensure!(
                self.camera.is_none() && self.broadcast.is_none() && self.people.is_none(),
                "completed camera command retains work"
            );
            return Ok(());
        }
        ensure!(
            self.complete() || self.tick == session.simulation.vm.tick,
            "clock advanced inside camera command"
        );
        let view = p
            .presenter
            .as_ref()
            .context("camera command presenter absent")?;
        ensure!(view.complete(), "camera command overlaps F6 viewer");
        if let Some(people) = &self.people {
            ensure!(
                self.pending_camera()
                    == Some(&camera::Service::Transition(
                        transition::Request::RefreshDrawables
                    )),
                "camera person refresh lost its caller"
            );
            people
                .validate(&person_refresh::Live {
                    world: &session.simulation.vm.world,
                    size: 64,
                    level: view.view.floor.level,
                })
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(Request::Camera(request)) = self.setter.request() {
            if let sims_sim::lot_presenter::open::Request::CenterPerson { point, level } = request {
                ensure!(
                    self.portrait
                        == Some(Portrait::Center {
                            point: *point,
                            level: *level,
                        }),
                    "portrait centering request lost its caller"
                );
            }
            if matches!(request, sims_sim::lot_presenter::open::Request::MotionFrame) {
                ensure!(
                    self.publication.is_none()
                        && self.walls.is_none()
                        && self.portrait.is_none()
                        && self.setter.direct(),
                    "camera motion frame has another caller"
                );
            }
            if matches!(
                request,
                sims_sim::lot_presenter::open::Request::WallsDown(_)
                    | sims_sim::lot_presenter::open::Request::Cutaway(_)
                    | sims_sim::lot_presenter::open::Request::Roof { .. }
            ) {
                ensure!(
                    self.publication.is_some()
                        || self.walls.is_some()
                        || self.portrait == Some(Portrait::Roof),
                    "wall camera lost its publication caller"
                );
            }
            ensure!(
                matches!(
                    request,
                    sims_sim::lot_presenter::open::Request::Zoom {
                        zoom: 1..=3,
                        force: 0,
                        refresh_ui: 1
                    } | sims_sim::lot_presenter::open::Request::Rotate {
                        rotation: 0..=3,
                        force: 0,
                        refresh_ui: 1
                    } | sims_sim::lot_presenter::open::Request::Floor {
                        level: 1..=2,
                        force: 0
                    } | sims_sim::lot_presenter::open::Request::Scroll { immediate: 1, .. }
                        | sims_sim::lot_presenter::open::Request::CenterPerson { level: 1..=2, .. }
                        | sims_sim::lot_presenter::open::Request::MotionFrame
                        | sims_sim::lot_presenter::open::Request::WallsDown(_)
                        | sims_sim::lot_presenter::open::Request::Cutaway(_)
                        | sims_sim::lot_presenter::open::Request::Roof { force: 0, .. }
                ),
                "invalid camera setter request"
            );
        }
        if let Some(camera) = &self.camera {
            ensure!(
                self.setter.request() == Some(&Request::Camera(camera.request().clone())),
                "camera command lost its UI parent"
            );
            camera
                .validate(
                    view.camera.as_ref().context("camera context absent")?,
                    &view.view,
                )
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(service) = &self.broadcast {
            let camera::Service::Transition(transition::Request::Broadcast { event, value }) =
                service
            else {
                anyhow::bail!("invalid camera broadcast service")
            };
            ensure!(
                self.pending_camera() == Some(service),
                "camera broadcast lost its caller"
            );
            let n = p
                .notifications
                .as_ref()
                .context("camera broadcast absent")?;
            ensure!(
                n.dispatch.root_message() == Some((u32::from(*event), *value)),
                "camera broadcast message differs"
            );
        } else if !self.complete() {
            ensure!(
                !session.house_notification_pending(),
                "unowned notification during camera command"
            );
        }
        Ok(())
    }
}
impl Session {
    pub fn camera_command_pending(&self) -> bool {
        self.host
            .presentation
            .camera_command
            .as_ref()
            .is_some_and(|c| !c.complete())
    }
    pub fn camera_motion_active(&self) -> bool {
        self.host
            .presentation
            .presenter
            .as_ref()
            .is_some_and(|presenter| {
                presenter.camera.as_ref().is_some_and(|camera| {
                    camera.settings.motion.active != 0
                        || (camera.settings.moving != 0
                            && camera.settings.camera_mode == 1
                            && camera.settings.locked == 0)
                }) || presenter.view.pending != [0, 0]
            })
    }
    /// Bind the frontend's actual property state after constructing its controls.
    pub fn bind_camera_ui(&mut self, ui: Ui) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera UI is busy"
        );
        self.host.presentation.camera_ui = Some(ui);
        Ok(())
    }
    pub fn begin_camera_zoom(&mut self, zoom: u32, deferred: u32) -> Result<()> {
        self.begin_camera_change(Change::Zoom(zoom), deferred)
    }
    pub fn begin_camera_change(&mut self, change: Change, deferred: u32) -> Result<()> {
        self.ensure_camera_input_idle()?;
        ensure!(
            match change {
                Change::Zoom(n) => (1..=3).contains(&n),
                Change::Rotation(n) => n <= 3,
                Change::Floor(n) => (0..=2).contains(&n),
                Change::CenterPerson { level, .. } => (1..=2).contains(&level),
                Change::Scroll(_) | Change::ZoomIn | Change::ZoomOut | Change::MotionFrame => true,
            },
            "camera change outside initialized original view"
        );
        let p = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("camera command presenter absent")?;
        p.validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            p.camera.is_some() && self.host.presentation.notifications.is_some(),
            "camera command native context absent"
        );
        if matches!(change, Change::CenterPerson { .. }) {
            ensure!(
                p.camera.as_ref().unwrap().settings.roof.is_some(),
                "portrait camera roof owner absent"
            );
        }
        let house = self.house.number as i16;
        let setter = self
            .host
            .presentation
            .edit_camera_properties(|ui| match change {
                Change::Zoom(n) => Zoom::begin(ui, n, deferred),
                Change::ZoomIn => Zoom::step(ui, true, house, deferred),
                Change::ZoomOut => Zoom::step(ui, false, house, deferred),
                Change::Rotation(n) => Zoom::rotation(ui, n, deferred),
                Change::Floor(n) => Zoom::floor(ui, n, deferred),
                Change::Scroll(position) => Zoom::scroll(position),
                Change::CenterPerson { point, level } => Zoom::center_person(point, level),
                Change::MotionFrame => Zoom::motion_frame(),
            })?;
        self.host.presentation.camera_command = Some(State {
            publication: None,
            tick: self.simulation.vm.tick,
            setter,
            camera: None,
            broadcast: None,
            people: None,
            walls: None,
            portrait: match change {
                Change::CenterPerson { point, level } => Some(Portrait::Center { point, level }),
                _ => None,
            },
        });
        Ok(())
    }
    pub(super) fn begin_unbound_walls_change(&mut self, walls: u32) -> Result<()> {
        let presenter = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("wall camera presenter absent")?;
        presenter
            .validate_simulation(&self.simulation)
            .map_err(anyhow::Error::msg)?;
        let camera = presenter
            .camera
            .as_ref()
            .context("wall camera context absent")?;
        ensure!(
            self.host.presentation.notifications.is_some(),
            "wall camera listeners absent"
        );
        self.host.presentation.camera_command = Some(State {
            publication: None,
            tick: self.simulation.vm.tick,
            setter: Zoom::publication(sims_sim::lot_presenter::open::Request::WallsDown(u8::from(
                walls == 2,
            )))
            .map_err(anyhow::Error::msg)?,
            camera: None,
            broadcast: None,
            people: None,
            walls: Some(Walls {
                mode: walls,
                next: 1,
                roof: camera.settings.roof.is_some(),
            }),
            portrait: None,
        });
        Ok(())
    }
    pub(super) fn ensure_camera_input_idle(&self) -> Result<()> {
        ensure!(
            !self.player_input_pending(),
            "player input owns the live UI"
        );
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending(),
            "camera command overlaps active work"
        );
        ensure!(
            self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|thread| matches!(thread.status, sims_vm::Status::AwaitingUiEffect(_))),
            "presentation callback must finish before camera command"
        );
        Ok(())
    }
    fn publish_command_view(&mut self) -> Result<()> {
        let p = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("camera command presenter absent")?;
        self.simulation.vm.world.camera_rotation = p
            .camera
            .as_ref()
            .context("camera context absent")?
            .settings
            .world_rotation as u8;
        self.simulation
            .vm
            .world
            .refresh
            .as_mut()
            .context("camera view fields absent")?
            .visible_level = p.view.floor.level;
        // Dispatch retains a projection of this same native viewer byte. A
        // camera transition can change it before its next broadcast begins.
        let include_visible = p.view.floor.visibility.include_visible;
        if let Some(n) = &mut self.host.presentation.notifications {
            n.dispatch.fields.include_visible = include_visible;
        }
        Ok(())
    }
    pub fn acknowledge_camera_command(
        &mut self,
        request: &camera::Service,
        reply: camera::Reply,
    ) -> Result<()> {
        let state = self
            .host
            .presentation
            .camera_command
            .as_ref()
            .context("camera command absent")?;
        state.validate(self)?;
        ensure!(
            state.broadcast.is_none(),
            "camera notification must complete through its dispatcher"
        );
        ensure!(
            state.people.is_none(),
            "camera person refresh must complete through its owner"
        );
        let projection = if matches!(
            request,
            camera::Service::Transition(transition::Request::SynchronizeRenderer)
                | camera::Service::Scroll(_)
                | camera::Service::MotionFrame(
                    sims_sim::lot_presenter::motion::frame::Request::Synchronize
                )
                | camera::Service::Displacement(
                    sims_sim::lot_presenter::motion::frame::displacement::Request::Synchronize
                )
        ) {
            Some(self.retained_camera_input()?)
        } else {
            None
        };
        let state = self.host.presentation.camera_command.as_mut().unwrap();
        let p = self.host.presentation.presenter.as_mut().unwrap();
        state
            .camera
            .as_mut()
            .context("camera call absent")?
            .acknowledge(p.camera.as_mut().unwrap(), &mut p.view, request, reply)
            .map_err(anyhow::Error::msg)?;
        self.publish_command_view()?;
        if let Some(input) = projection {
            self.host.presentation.camera = Some(input);
        }
        Ok(())
    }
    /// The initialized retained viewer, also consumed by character visibility
    /// and geometry services after the renderer synchronization returns.
    pub fn retained_camera_input(&self) -> Result<Input> {
        let p = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("camera presenter absent")?;
        let s = &p
            .camera
            .as_ref()
            .context("camera settings absent")?
            .settings;
        let [left, top, right, bottom] = s.viewport;
        ensure!(
            left >= 0 && top >= 0 && right > left && bottom > top,
            "camera viewport unavailable"
        );
        Ok(Input {
            viewport: [
                left as u32,
                top as u32,
                (right - left) as u32,
                (bottom - top) as u32,
            ],
            center: p.view.origin,
            zoom: p.view.zoom,
            rotation: p.view.display_rotation,
        })
    }
    /// Only after the frontend has performed original57a9d4's audio refresh.
    pub fn complete_camera_audio(&mut self) -> Result<()> {
        self.complete_camera_setter_service(Request::RefreshAudio)
    }
    /// Execute4cee3d's selected-tool callback, then return to its setter.
    /// Retain the historical public/wire name; this is not a panel refresh.
    pub fn complete_camera_controls(&mut self) -> Result<()> {
        let state = self
            .host
            .presentation
            .camera_command
            .as_ref()
            .context("camera command absent")?;
        state.validate(self)?;
        ensure!(
            state.setter.request() == Some(&Request::RefreshControls),
            "camera selected-tool callback has no caller"
        );
        self.selected_tool_camera_changed()?;
        self.complete_camera_setter_service(Request::RefreshControls)
    }

    /// 4a7996 calls4cee08(level,1) after its physical floor change. The deferred
    /// setter still writes the property/dirty bit and invokes the selected tool.
    pub fn refresh_center_floor_ui(&mut self, level: i32, deferred: u8) -> Result<()> {
        ensure!(deferred == 1, "centering requires deferred floor setter");
        let mut setter = self
            .host
            .presentation
            .edit_camera_properties(|ui| Zoom::floor(ui, level, u32::from(deferred)))?;
        ensure!(
            setter.request() == Some(&Request::RefreshControls),
            "deferred floor setter differs"
        );
        self.selected_tool_camera_changed()?;
        setter
            .acknowledge(&Request::RefreshControls)
            .map_err(anyhow::Error::msg)
    }
    fn complete_camera_setter_service(&mut self, request: Request) -> Result<()> {
        let state = self
            .host
            .presentation
            .camera_command
            .as_ref()
            .context("camera command absent")?;
        state.validate(self)?;
        self.host
            .presentation
            .camera_command
            .as_mut()
            .unwrap()
            .setter
            .acknowledge(&request)
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_camera_command(&mut self, input: Input, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "camera command needs a positive work budget");
        self.host
            .presentation
            .camera_command
            .as_ref()
            .context("camera command absent")?
            .validate(self)?;
        let mut state = self.host.presentation.camera_command.take().unwrap();
        let result = self.advance_camera_command_inner(&mut state, input, budget);
        // Preserve earlier original writes if a subsequent asset/UI service fails.
        self.host.presentation.camera_command = Some(state);
        let progress = result?;
        self.host
            .presentation
            .camera_command
            .as_ref()
            .unwrap()
            .validate(self)?;
        Ok(progress)
    }
    /// Reply through the retained UI/camera caller, never bypass its broadcast.
    pub fn acknowledge_camera_notification(
        &mut self,
        service: &notifications::Service,
        reply: notifications::Reply,
    ) -> Result<()> {
        let state = self
            .host
            .presentation
            .camera_command
            .as_ref()
            .context("camera command absent")?;
        state.validate(self)?;
        ensure!(
            state.broadcast.is_some(),
            "camera has no pending notification"
        );
        let state = self.host.presentation.camera_command.take().unwrap();
        let result = self.acknowledge_house_notification(service, reply);
        self.host.presentation.camera_command = Some(state);
        result
    }
    fn advance_camera_command_inner(
        &mut self,
        state: &mut State,
        input: Input,
        budget: usize,
    ) -> Result<Progress> {
        for _ in 0..budget {
            if let Some(people) = &mut state.people {
                let level = self
                    .host
                    .presentation
                    .presenter
                    .as_ref()
                    .unwrap()
                    .view
                    .floor
                    .level;
                if self.advance_camera_people(people, level)? {
                    let p = self.host.presentation.presenter.as_mut().unwrap();
                    state
                        .camera
                        .as_mut()
                        .unwrap()
                        .acknowledge(
                            p.camera.as_mut().unwrap(),
                            &mut p.view,
                            &camera::Service::Transition(transition::Request::RefreshDrawables),
                            camera::Reply::Done,
                        )
                        .map_err(anyhow::Error::msg)?;
                    state.people = None;
                }
                continue;
            }
            if let Some(broadcast) = state.broadcast.clone() {
                match self.advance_house_notification(input, 1)? {
                    notifications::Progress::Pending => (),
                    notifications::Progress::Service(service) => {
                        return Ok(Progress::Notification(service));
                    }
                    notifications::Progress::Complete(_) => {
                        let p = self.host.presentation.presenter.as_mut().unwrap();
                        state
                            .camera
                            .as_mut()
                            .unwrap()
                            .acknowledge(
                                p.camera.as_mut().unwrap(),
                                &mut p.view,
                                &broadcast,
                                camera::Reply::Done,
                            )
                            .map_err(anyhow::Error::msg)?;
                        state.broadcast = None;
                    }
                }
                continue;
            }
            let Some(request) = state.setter.request().cloned() else {
                if state.advance_portrait_tail()? {
                    continue;
                }
                if let Some(walls) = &mut state.walls {
                    let request = match walls.next {
                        1 => sims_sim::lot_presenter::open::Request::Cutaway(u8::from(
                            walls.mode == 4,
                        )),
                        2 if walls.roof => sims_sim::lot_presenter::open::Request::Roof {
                            enabled: 0,
                            force: 0,
                        },
                        _ => {
                            state.walls = None;
                            return Ok(Progress::Complete);
                        }
                    };
                    walls.next += 1;
                    state.setter = Zoom::publication(request).map_err(anyhow::Error::msg)?;
                    continue;
                }
                return Ok(Progress::Complete);
            };
            let Request::Camera(request) = request else {
                return Ok(match request {
                    Request::RefreshAudio => Progress::RefreshAudio,
                    Request::RefreshControls => Progress::RefreshControls,
                    _ => unreachable!(),
                });
            };
            let selected = presenter::selected_for_visibility(&self.simulation)?;
            let p = self
                .host
                .presentation
                .presenter
                .as_mut()
                .context("camera command presenter absent")?;
            let retained = p.camera.as_mut().context("camera context absent")?;
            if state.camera.is_none() {
                state.camera = Some(
                    camera::Cursor::begin(request.clone(), retained, &mut p.view)
                        .map_err(anyhow::Error::msg)?,
                );
            }
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
            let result = state
                .camera
                .as_mut()
                .unwrap()
                .advance(retained, &mut p.view, &inputs, 1);
            self.publish_command_view()?;
            match result.map_err(anyhow::Error::msg)? {
                camera::Progress::Pending => (),
                camera::Progress::Complete => {
                    state.camera = None;
                    self.host
                        .presentation
                        .edit_camera_properties(|ui| {
                            state
                                .setter
                                .acknowledge_with_ui(&Request::Camera(request), ui)
                        })?
                        .map_err(anyhow::Error::msg)?;
                }
                camera::Progress::Service(service) => {
                    if let camera::Service::Roof(
                        sims_sim::lot_presenter::lifecycle::roof::Request::UiFlags(bits),
                    ) = service
                    {
                        self.host
                            .presentation
                            .transport
                            .as_mut()
                            .context("roof CPState absent")?
                            .controls
                            .mark_dirty(bits);
                        let p = self.host.presentation.presenter.as_mut().unwrap();
                        state
                            .camera
                            .as_mut()
                            .unwrap()
                            .acknowledge(
                                p.camera.as_mut().unwrap(),
                                &mut p.view,
                                &service,
                                camera::Reply::Done,
                            )
                            .map_err(anyhow::Error::msg)?;
                    } else if service
                        == camera::Service::Transition(transition::Request::RefreshDrawables)
                    {
                        ensure!(
                            self.simulation.vm.world.person.is_some(),
                            "camera person registry absent"
                        );
                        state.people = Some(person_refresh::Cursor::default());
                    } else if let camera::Service::Transition(transition::Request::Broadcast {
                        event,
                        value,
                    }) = &service
                    {
                        let n = self
                            .host
                            .presentation
                            .notifications
                            .as_ref()
                            .context("camera notification context absent")?;
                        self.begin_house_notification(
                            n.dispatch.registry.clone(),
                            n.dispatch.fields.clone(),
                            u32::from(*event),
                            *value,
                        )?;
                        state.broadcast = Some(service);
                    } else {
                        return Ok(Progress::Camera(service));
                    }
                }
            }
        }
        Ok(if state.complete() {
            Progress::Complete
        } else {
            Progress::Pending
        })
    }
}

#[cfg(test)]
mod portrait_tests {
    use super::*;

    fn center() -> State {
        State {
            publication: None,
            tick: 194373,
            setter: Zoom::center_person([616, 440], 1),
            camera: None,
            broadcast: None,
            people: None,
            walls: None,
            portrait: Some(Portrait::Center {
                point: [616, 440],
                level: 1,
            }),
        }
    }

    #[test]
    fn portrait_camera_retains_native_roof_tail_after_completed_center_restore() -> Result<()> {
        let mut state = center();
        assert!(state.uses_frontend_clock());
        assert!(!state.complete());
        let request = Request::Camera(state.portrait.as_ref().unwrap().request());
        assert_eq!(state.setter.request(), Some(&request));
        state.setter.acknowledge(&request).map_err(anyhow::Error::msg)?;
        state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
        assert!(!state.complete());
        assert!(state.advance_portrait_tail()?);
        assert_eq!(state.portrait, Some(Portrait::Roof));
        assert!(state.uses_frontend_clock());
        let roof = Request::Camera(Portrait::Roof.request());
        assert_eq!(state.setter.request(), Some(&roof));
        state.setter.acknowledge(&roof).map_err(anyhow::Error::msg)?;
        state = serde_json::from_slice(&serde_json::to_vec(&state)?)?;
        assert!(!state.complete());
        assert!(!state.advance_portrait_tail()?);
        assert!(state.complete());
        assert!(!state.uses_frontend_clock());
        assert!(!state.advance_portrait_tail()?);
        assert!(state.complete());
        Ok(())
    }

    #[test]
    fn portrait_camera_cannot_skip_its_unfinished_native_child() -> Result<()> {
        let mut state = center();
        let before = serde_json::to_vec(&state)?;
        assert_eq!(
            state.advance_portrait_tail().unwrap_err().to_string(),
            "portrait camera child has not returned"
        );
        assert_eq!(serde_json::to_vec(&state)?, before);
        Ok(())
    }
}
