//! Replacement frontend's ordered pause command:4cee4a, then4ce44f mask800040.
//! The real person refresh and C2 house registry run synchronously, with saved
//! continuations. This scoped host does not claim the complete4ce390 frame.
use super::{Session, notifications};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::lot_presenter::person_refresh;
use sims_vm::{
    interface::{state, transport as panel},
    scheduler::transport as lot,
};
mod publication;
use publication::Publication;
use sims_vm::interface::world_publication::Request as WorldRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub controls: state::State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<lot::Timing>,
    call: Option<Call>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    frame_publication: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    tick: u64,
    setter: panel::SetPause,
    publication: Option<Publication>,
    lot: Option<lot::Change>,
    people: Option<person_refresh::Cursor>,
    broadcast: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    speed: Option<sims_vm::scheduler::clock::Speed>,
    /// Replacement speed control: Live4cee4a(0), then4cee89, then publication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    speed_input: Option<i32>,
    /// A bound full frame consumes dirty bits after the setter returns. Older
    /// saved calls keep their scoped publication, including suspended children.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    frame_publication: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    WindowPause(u32),
    PauseAudio,
    ResumeAudio,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    Service(Service),
}
impl State {
    /// New unpublished adapter epoch around actual observed CPState/counters.
    /// No control initializer, input setter, timer reset or callback runs.
    pub(super) fn from_completed_inputs(controls: state::State, timing: lot::Timing) -> Self {
        Self {
            controls,
            timing: Some(timing),
            call: None,
            frame_publication: false,
        }
    }
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            !self.frame_publication || s.host.presentation.publication.is_some(),
            "transport lost its bound full-frame publisher"
        );
        self.controls.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.timing.is_some() != s.host.presentation.clock.is_some(),
            "transport elapsed counters lost their single owner"
        );
        let view = s
            .host
            .presentation
            .presenter
            .as_ref()
            .context("transport view absent")?;
        view.camera.as_ref().context("transport camera absent")?;
        // CPState+8 is the control-panel mode. The camera's historical
        // `application_mode` field is World+8, a separate native owner. The
        // captured live Goth cut has CPState=2 and World=0; equating them would
        // reject original state or force one of its values to be rewritten.
        ensure!(
            s.simulation.native_scheduler.is_some(),
            "transport requires native partial scheduling"
        );
        if let Some(c) = &self.call {
            ensure!(
                c.frame_publication == self.frame_publication,
                "transport publication policy changed during input"
            );
            ensure!(
                !c.frame_publication
                    || (s.host.presentation.publication.is_some() && c.publication.is_none()),
                "transport lost full-frame publisher"
            );
            c.setter.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                c.tick == s.simulation.vm.tick,
                "transport crossed a lot tick"
            );
            ensure!(
                !s.host
                    .presentation
                    .interface_update
                    .as_ref()
                    .is_some_and(super::interface::State::pending)
                    && !s.control_panel_pending()
                    && !s.camera_command_pending()
                    && !s.ui_wait_pending()
                    && !s.ui_portrait_pending()
                    && !s.player_input_pending()
                    && !s.view_refresh_pending()
                    && !s.load_completion_pending()
                    && !s.postload_pending()
                    && !s.simulation.selection_work_pending()
                    && !s.host.ordinary.updates_pending(),
                "transport overlaps synchronous work"
            );
            if let Some(p) = &c.publication {
                p.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    c.setter.complete() && c.people.is_none(),
                    "transport publication bypassed setter"
                );
            }
            if c.speed_input.is_some() {
                ensure!(
                    self.controls.speed_timers.is_some() && s.host.presentation.clock.is_some(),
                    "speed input lost its timer owners"
                );
            }
            if let Some(l) = &c.lot {
                l.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    matches!(
                        c.publication.as_ref().and_then(Publication::request),
                        Some(WorldRequest::Pause | WorldRequest::Resume)
                    ),
                    "lot transport lost parent"
                );
            }
            if let Some(people) = &c.people {
                ensure!(
                    c.setter.request() == Some(panel::Request::RefreshPeople),
                    "transport people lost caller"
                );
                people
                    .validate(&person_refresh::Live {
                        world: &s.simulation.vm.world,
                        size: 64,
                        level: view.view.floor.level,
                    })
                    .map_err(anyhow::Error::msg)?;
            }
            if let Some(speed) = &c.speed {
                speed.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    matches!(
                        c.publication.as_ref().and_then(Publication::request),
                        Some(WorldRequest::SetSpeed(_))
                    ),
                    "transport speed lost caller"
                );
            }
            if c.broadcast {
                ensure!(
                    (c.lot.as_ref().and_then(lot::Change::request)
                        == Some(lot::Request::Broadcast)
                        || c.speed.is_some())
                        && s.host
                            .presentation
                            .notifications
                            .as_ref()
                            .is_some_and(|n| n.dispatch.root_message() == Some((0xc2, 0))),
                    "transport notification lost its owner"
                );
            } else {
                ensure!(
                    !s.house_notification_pending(),
                    "unowned transport notification"
                );
            }
        }
        Ok(())
    }
}
impl Session {
    /// Explicit replacement-application policy: a full frame follows input
    /// setters. Restore never upgrades an older scoped transport caller.
    pub fn bind_frame_transport(&mut self) -> Result<()> {
        ensure!(
            !self.transport_pending()
                && !self.publication_pending()
                && self.host.presentation.publication.is_some(),
            "full transport frame is absent or busy"
        );
        self.host
            .presentation
            .transport
            .as_mut()
            .context("transport absent")?
            .frame_publication = true;
        Ok(())
    }
    pub fn transport_window_request(&self) -> Option<u32> {
        match self
            .host
            .presentation
            .transport
            .as_ref()?
            .call
            .as_ref()?
            .setter
            .request()?
        {
            panel::Request::WindowPause(value) => Some(value),
            _ => None,
        }
    }
    pub fn transport_pending(&self) -> bool {
        self.host
            .presentation
            .transport
            .as_ref()
            .is_some_and(State::pending)
    }
    /// Explicit fresh replacement frontend binding after house opening and
    /// Live publication. Never infer these process fields for an older save.
    pub fn bind_transport(&mut self, dirty: u32) -> Result<()> {
        ensure!(
            self.host.presentation.transport.is_none(),
            "transport already bound"
        );
        self.transport_idle()?;
        let g = &self.simulation.vm.world.globals;
        let mut controls =
            state::State::constructed(i32::from(*g.get(&16).context("lot speed absent")?));
        controls.mode = self
            .host
            .presentation
            .presenter
            .as_ref()
            .context("transport view absent")?
            .camera
            .as_ref()
            .context("transport camera absent")?
            .settings
            .application_mode;
        controls.paused = u8::from(*g.get(&17).context("lot pause absent")? != 0);
        controls.buy.dirty = dirty;
        let state = State {
            controls,
            timing: Some(lot::Timing::default()),
            call: None,
            frame_publication: false,
        };
        state.validate(self)?;
        self.host.presentation.transport = Some(state);
        Ok(())
    }
    pub(super) fn transport_idle(&self) -> Result<()> {
        ensure!(
            !self.player_input_pending(),
            "player input owns transport controls"
        );
        self.clock_idle()
    }
    pub(super) fn clock_idle(&self) -> Result<()> {
        ensure!(
            !self.clock_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.player_input_blocks_ticks()
                && !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_))),
            "pause command overlaps synchronous work"
        );
        Ok(())
    }
    pub fn begin_pause(&mut self, paused: bool) -> Result<()> {
        self.transport_idle()?;
        let state = self
            .host
            .presentation
            .transport
            .as_ref()
            .context("transport is not bound")?;
        state.validate(self)?;
        ensure!(
            state.controls.mode == 2,
            "pause command requires Live frontend"
        );
        self.host.presentation.transport.as_mut().unwrap().call = Some(Call {
            tick: self.simulation.vm.tick,
            setter: panel::SetPause::new(u32::from(paused)),
            publication: None,
            lot: None,
            people: None,
            broadcast: false,
            speed: None,
            speed_input: None,
            frame_publication: state.frame_publication,
        });
        Ok(())
    }
    /// Live speed controls resume before changing the raw CPState speed. The
    /// later lot setter clamps it; the control setter itself does not.
    pub fn begin_speed(&mut self, value: i32) -> Result<()> {
        ensure!(
            self.host.presentation.clock.is_some(),
            "speed control needs elapsed clock"
        );
        ensure!(
            self.host
                .presentation
                .transport
                .as_ref()
                .is_some_and(|t| t.controls.speed_timers.is_some()),
            "speed control timers absent"
        );
        self.begin_pause(false)?;
        self.host
            .presentation
            .transport
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .unwrap()
            .speed_input = Some(value);
        Ok(())
    }
    pub fn acknowledge_transport(&mut self, service: Service) -> Result<()> {
        self.host
            .presentation
            .transport
            .as_ref()
            .context("transport absent")?
            .validate(self)?;
        let c = self
            .host
            .presentation
            .transport
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .context("transport has no call")?;
        match service {
            Service::WindowPause(v) => c.setter.acknowledge(panel::Request::WindowPause(v)),
            Service::PauseAudio | Service::ResumeAudio => c
                .publication
                .as_mut()
                .context("transport publication absent")?
                .acknowledge(if service == Service::PauseAudio {
                    panel::UpdateRequest::PauseAudio
                } else {
                    panel::UpdateRequest::ResumeAudio
                }),
        }
        .map_err(anyhow::Error::msg)
    }
    pub fn advance_transport(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "transport needs a work budget");
        self.host
            .presentation
            .transport
            .as_ref()
            .context("transport absent")?
            .validate(self)?;
        let mut state = self.host.presentation.transport.take().unwrap();
        let result = self.advance_transport_inner(&mut state, budget);
        // Earlier native writes survive a later unsupported child or asset error.
        self.host.presentation.transport = Some(state);
        result
    }
    fn advance_transport_inner(&mut self, state: &mut State, budget: usize) -> Result<Progress> {
        for _ in 0..budget {
            let Some(c) = &mut state.call else {
                return Ok(Progress::Complete);
            };
            if !c.setter.complete() {
                match c
                    .setter
                    .advance(&mut state.controls)
                    .map_err(anyhow::Error::msg)?
                {
                    Some(panel::Request::WindowPause(v)) => {
                        return Ok(Progress::Service(Service::WindowPause(v)));
                    }
                    Some(panel::Request::RefreshPeople) => {
                        let people = c.people.get_or_insert_with(Default::default);
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
                            c.people = None;
                            c.setter
                                .acknowledge(panel::Request::RefreshPeople)
                                .map_err(anyhow::Error::msg)?;
                        }
                        continue;
                    }
                    None => (),
                }
            }
            if c.publication.is_none()
                && let Some(value) = c.speed_input
            {
                state
                    .controls
                    .set_speed(value)
                    .map_err(anyhow::Error::msg)?;
            }
            if c.frame_publication {
                state.call = None;
                return Ok(Progress::Complete);
            }
            let publication = c.publication.get_or_insert_with(Publication::new);
            let g = &mut self.simulation.vm.world.globals;
            let input = panel::Inputs {
                menu_present: false,
                menu_active: 0,
                closing: 0,
                lot_speed: *g.get(&16).context("lot speed absent")?,
            };
            let request = publication
                .advance(
                    &state.controls,
                    g.get_mut(&19).context("lot mode absent")?,
                    input,
                )
                .map_err(anyhow::Error::msg)?;
            match request {
                None => {
                    state.call = None;
                    return Ok(Progress::Complete);
                }
                Some(r @ panel::UpdateRequest::SetSpeed(value)) => {
                    let clock = self
                        .host
                        .presentation
                        .clock
                        .as_mut()
                        .context("original speed change requires its elapsed-clock owner")?;
                    let speed = c
                        .speed
                        .get_or_insert_with(|| sims_vm::scheduler::clock::Speed::new(value));
                    let (mut current, mut saved) = (g[&16], g[&18]);
                    let notice = speed
                        .advance(&mut clock.fields, &mut current, &mut saved)
                        .map_err(anyhow::Error::msg)?;
                    g.insert(16, current);
                    g.insert(18, saved);
                    if notice {
                        if !c.broadcast {
                            self.begin_clock_notice(0xc2, 0)?;
                            c.broadcast = true;
                        }
                        if self.advance_clock_notice()? {
                            speed.acknowledge().map_err(anyhow::Error::msg)?;
                            c.broadcast = false;
                        }
                    } else {
                        c.speed = None;
                        publication.acknowledge(r).map_err(anyhow::Error::msg)?;
                    }
                }
                Some(panel::UpdateRequest::PauseAudio) => {
                    return Ok(Progress::Service(Service::PauseAudio));
                }
                Some(panel::UpdateRequest::ResumeAudio) => {
                    return Ok(Progress::Service(Service::ResumeAudio));
                }
                Some(r @ (panel::UpdateRequest::Pause | panel::UpdateRequest::Resume)) => {
                    let call = c.lot.get_or_insert_with(|| {
                        lot::Change::new(if r == panel::UpdateRequest::Pause {
                            lot::Command::Pause
                        } else {
                            lot::Command::Resume
                        })
                    });
                    if call
                        .advance(
                            g.get_mut(&17).context("lot pause absent")?,
                            if let Some(clock) = &mut self.host.presentation.clock {
                                &mut clock.fields.timing
                            } else {
                                state.timing.as_mut().context("transport timing absent")?
                            },
                            0,
                        )
                        .map_err(anyhow::Error::msg)?
                        .is_some()
                    {
                        if !c.broadcast {
                            let n = self
                                .host
                                .presentation
                                .notifications
                                .as_ref()
                                .context("transport registry absent")?;
                            let registry = n.dispatch.registry.clone();
                            let fields = n.dispatch.fields.clone();
                            self.begin_house_notification(registry, fields, 0xc2, 0)?;
                            c.broadcast = true;
                        }
                        match self.advance_house_notification(self.retained_camera_input()?, 1)? {
                            notifications::Progress::Complete(_) => {
                                call.acknowledge(lot::Request::Broadcast)
                                    .map_err(anyhow::Error::msg)?;
                                c.broadcast = false;
                            }
                            notifications::Progress::Pending => (),
                            notifications::Progress::Service(s) => {
                                anyhow::bail!("unexpected original C2 house service: {s:?}")
                            }
                        }
                    } else {
                        c.lot = None;
                        publication.acknowledge(r).map_err(anyhow::Error::msg)?;
                    }
                }
            }
        }
        Ok(Progress::Pending)
    }
}
