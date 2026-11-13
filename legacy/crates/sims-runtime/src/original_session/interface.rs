//! Resumable mode-dirty publication for the replacement frontend's fixed Live
//! mode. Original4ce44f mask2 runs after opening and before household ticks.
//! This does not implement4cea5d mode changes or the entire4ce390 update frame.
use super::{Session, presenter::selected_for_visibility};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::lot_presenter::camera;
use sims_vm::interface::{Inputs, LiveModeUpdate, Request};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    call: LiveModeUpdate,
    tick: u64,
}
impl State {
    pub fn pending(&self) -> bool {
        !self.call.complete()
    }
    pub fn request(&self) -> Option<&Request> {
        self.call.request()
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        self.call.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            !self.pending() || self.tick == s.simulation.vm.tick,
            "interface update crossed a lot tick"
        );
        ensure!(
            !self.pending()
                || (!s.control_panel_pending()
                    && !s.ui_portrait_pending()
                    && !s.camera_command_pending()
                    && !s.ui_wait_pending()
                    && !s.load_completion_pending()
                    && !s.postload_pending()
                    && !s.view_refresh_pending()
                    && !s.house_notification_pending()
                    && !s.host.presentation.awaiting_house_open
                    && !s.host.ordinary.updates_pending()
                    && !s.simulation.selection_work_pending()
                    && s.simulation.vm.world.dialogs.modal.is_none()
                    && !s.simulation.vm.constructing_dialog()
                    && !s
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)))),
            "saved interface update overlaps synchronous work"
        );
        ensure!(
            s.host
                .presentation
                .presenter
                .as_ref()
                .and_then(|p| p.camera.as_ref())
                .is_some(),
            "interface update lost its camera"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Request),
}

impl Session {
    pub fn interface_update_pending(&self) -> bool {
        self.family_window_pending() || self.transport_pending()
            || self
                .host
                .presentation
                .interface_update
                .as_ref()
                .is_some_and(State::pending)
    }
    /// Explicit fresh frontend policy: Live, no application dialog/closing gate.
    /// Historical saves do not acquire this owner just by being restored.
    pub fn begin_live_interface_update(&mut self) -> Result<()> {
        ensure!(
            !self.player_input_pending(),
            "player input owns the live UI"
        );
        ensure!(
            self.host.presentation.interface_update.is_none(),
            "fresh interface publication already bound"
        );
        ensure!(
            !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.host.ordinary.updates_pending()
                && !self.simulation.selection_work_pending()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_))),
            "fresh interface update overlaps synchronous work"
        );
        let input = self.live_interface_inputs()?;
        ensure!(
            input.mode == 2,
            "fresh interface requires actual Live camera mode"
        );
        // The fresh application's Live transition calls416534 before its
        // mask-2 publication: invalidate rooms/routes, then publish valuation
        // and capabilities. Room scores rebuild at the later frame boundary.
        // This is not the complete legacy mode transition (see module scope).
        self.refresh_original_house()?;
        self.host.presentation.interface_update = Some(State {
            call: LiveModeUpdate::default(),
            tick: self.simulation.vm.tick,
        });
        Ok(())
    }
    fn live_interface_inputs(&self) -> Result<Inputs> {
        let p = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("interface presenter absent")?;
        let c = p.camera.as_ref().context("interface camera absent")?;
        Ok(Inputs {
            menu_present: false,
            menu_active: 0,
            closing: 0,
            mode: c.settings.application_mode,
            //4ce81c ->46d866 normalizes the whole signed word before4cee4a
            // stores Interface+40; truncating global17 to its low byte is wrong.
            paused: u8::from(
                *self
                    .simulation
                    .vm
                    .world
                    .globals
                    .get(&17)
                    .context("interface pause global absent")?
                    != 0,
            ),
            include_visible: p.view.floor.visibility.include_visible,
        })
    }
    pub fn acknowledge_interface_service(&mut self, request: &Request) -> Result<()> {
        ensure!(
            matches!(
                request,
                Request::DrawingMode
                    | Request::ResumeAudio
                    | Request::StopAudio
                    | Request::PlayLiveBackground
            ),
            "interface engine work cannot be externally acknowledged"
        );
        self.host
            .presentation
            .interface_update
            .as_ref()
            .context("interface owner absent")?
            .validate(self)?;
        self.host
            .presentation
            .interface_update
            .as_mut()
            .unwrap()
            .call
            .acknowledge(request)
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_interface_update(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "interface update requires a budget");
        self.host
            .presentation
            .interface_update
            .as_ref()
            .context("interface owner absent")?
            .validate(self)?;
        let mut state = self.host.presentation.interface_update.take().unwrap();
        let result = (|| -> Result<Progress> {
            for _ in 0..budget {
                let Some(request) = state
                    .call
                    .advance(self.live_interface_inputs()?)
                    .map_err(anyhow::Error::msg)?
                else {
                    return Ok(Progress::Complete);
                };
                match request {
                    Request::RoomMask(include_visible) => {
                        let selected = selected_for_visibility(&self.simulation)?;
                        let p = self.host.presentation.presenter.as_mut().unwrap();
                        let c = p.camera.as_ref().unwrap();
                        c.compose(
                            &mut p.view,
                            &camera::Inputs {
                                roof_controls_present: self.host.presentation.transport.is_some(),
                                lot: self
                                    .simulation
                                    .architecture
                                    .as_ref()
                                    .and_then(|a| a.original.as_ref())
                                    .context("interface lot absent")?,
                                rooms: &self
                                    .simulation
                                    .room_environment
                                    .as_ref()
                                    .context("interface rooms absent")?
                                    .manager,
                                application_mode: c.settings.application_mode,
                                selected: selected.as_ref(),
                                clock: c.settings.clock,
                            },
                            include_visible,
                        )
                        .map_err(anyhow::Error::msg)?;
                    }
                    Request::PublishLiveMode => {
                        *self
                            .simulation
                            .vm
                            .world
                            .globals
                            .get_mut(&19)
                            .context("interface mode global absent")? = 0;
                    }
                    _ => return Ok(Progress::Service(request)),
                }
                state
                    .call
                    .acknowledge(&request)
                    .map_err(anyhow::Error::msg)?;
            }
            Ok(if state.call.complete() {
                Progress::Complete
            } else {
                Progress::Pending
            })
        })();
        self.host.presentation.interface_update = Some(state);
        result
    }
}
