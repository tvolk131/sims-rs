//! Native elapsed-frame caller over a loaded household. The caller supplies
//! application milliseconds explicitly; wall-clock acquisition is a frontend
//! responsibility. Historic sessions are not bound on restore.
use super::{Session, notifications};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::scheduler::clock::{self as native, speed_override};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborhoodRequest {
    pub id: i16,
    pub neighbor: i16,
    pub payload: u32,
    pub kind: i32,
    pub status: i32,
    pub age: i16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub fields: native::Fields,
    pub now: u32,
    pub stopping: u8,
    /// The fresh46cdc6 list starts empty. Nonempty requests require the actual
    /// neighborhood services; they must not be discarded or acknowledged.
    pub requests: Vec<NeighborhoodRequest>,
    call: Option<Call>,
    /// This exact capture has dormant INT3 bytes after its input-poll call.
    /// Its zero poll byte is qualified; it is not the later playable-poll body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_code: Option<CapturedCode>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapturedCode {
    Goth88448DormantProtectedPoll20261029,
    Goth89073DormantProtectedPoll20261031,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    frame: native::Frame,
    tick: u64,
    updates: u32,
    speed_override: Option<speed_override::Change>,
    notice: Option<speed_override::Notice>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Pending,
    /// The replacement event loop supplies whether input is waiting. The
    /// original PeekMessage call observes without removing that input.
    PollInput,
    Complete {
        updates: u32,
    },
    Household(super::Progress),
}
impl State {
    pub(super) fn from_completed_update(
        fields: native::Fields,
        now: u32,
        stopping: u8,
        tick: u64,
    ) -> Self {
        Self::from_completed_update_profile(
            fields,
            now,
            stopping,
            tick,
            CapturedCode::Goth88448DormantProtectedPoll20261029,
        )
    }
    pub(super) fn from_completed_update_profile(
        fields: native::Fields,
        now: u32,
        stopping: u8,
        tick: u64,
        profile: CapturedCode,
    ) -> Self {
        Self {
            fields,
            now,
            stopping,
            requests: vec![],
            call: Some(Call {
                frame: native::Frame::after_completed_update(),
                tick,
                updates: 0,
                speed_override: None,
                notice: None,
            }),
            captured_code: Some(profile),
        }
    }
    pub fn pending(&self) -> bool {
        self.call.is_some()
    }
    pub fn at_completed_update(&self) -> bool {
        self.call.as_ref().is_some_and(|c| {
            c.frame.at_completed_update() && c.speed_override.is_none() && c.notice.is_none()
        })
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        ensure!(
            self.captured_code.is_none() || self.fields.poll_messages == 0,
            "captured protected elapsed input-poll branch is not qualified for execution"
        );
        ensure!(
            session.simulation.native_scheduler.is_some(),
            "elapsed clock requires native registry scheduling"
        );
        let transport = session
            .host
            .presentation
            .transport
            .as_ref()
            .context("elapsed clock transport absent")?;
        ensure!(
            transport.timing.is_none(),
            "elapsed counters have two owners"
        );
        if let Some(c) = &self.call {
            c.frame.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                c.tick == session.simulation.vm.tick,
                "elapsed clock child crossed an unacknowledged lot tick"
            );
            ensure!(
                !session.transport_pending(),
                "elapsed frame overlaps a transport command"
            );
            if let Some(speed) = &c.speed_override {
                speed.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    match c.frame.request() {
                        Some(native::Request::Boost) =>
                            speed.command() == speed_override::Command::Boost(1000),
                        Some(native::Request::RestoreSpeed) =>
                            speed.command() == speed_override::Command::Restore,
                        _ => false,
                    },
                    "clock speed override lost parent"
                );
            }
            if let Some(notice) = c.notice {
                ensure!(
                    c.speed_override
                        .as_ref()
                        .is_some_and(|s| s.request() == Some(notice))
                        && session
                            .host
                            .presentation
                            .notifications
                            .as_ref()
                            .is_some_and(|n| n.dispatch.root_message()
                                == Some((notice.event, notice.argument))),
                    "clock notification lost its caller"
                );
            }
        }
        Ok(())
    }
}
impl Session {
    pub fn clock_pending(&self) -> bool {
        self.host
            .presentation
            .clock
            .as_ref()
            .is_some_and(State::pending)
    }
    /// Explicit fresh process owner after house opening. The original load
    /// already called46d697 on the imported speed; bind its scalar/timer state
    /// without replaying notifications or changing loaded script globals.
    pub fn bind_clock(&mut self, now: u32) -> Result<()> {
        ensure!(
            self.host.presentation.clock.is_none(),
            "elapsed clock already bound"
        );
        ensure!(
            self.simulation.native_scheduler.is_some(),
            "elapsed clock requires native registry scheduling"
        );
        self.transport_idle()?;
        self.host
            .presentation
            .transport
            .as_ref()
            .context("elapsed clock transport absent")?
            .validate(self)?;
        let speed = *self
            .simulation
            .vm
            .world
            .globals
            .get(&16)
            .context("clock speed absent")?;
        ensure!(
            (0..=1000).contains(&speed),
            "fresh elapsed clock speed was not initialized by load"
        );
        let period = native::period(i32::from(speed));
        let transport = self
            .host
            .presentation
            .transport
            .as_mut()
            .context("elapsed clock transport absent")?;
        let timing = transport
            .timing
            .take()
            .context("fresh clock elapsed counters absent")?;
        self.host.presentation.clock = Some(State {
            fields: native::Fields {
                timing,
                period,
                timer: native::Timer {
                    interval: period as u32,
                    limit: 30,
                    ..Default::default()
                },
                budget_d8: (period * 60 / 42).clamp(1, 60),
                budget_dc: 90,
                poll_messages: 0,
                publish: 0,
            },
            now,
            stopping: 0,
            requests: vec![],
            call: None,
            captured_code: None,
        });
        self.host
            .presentation
            .clock
            .as_ref()
            .unwrap()
            .validate(self)?;
        Ok(())
    }
    pub fn set_clock_time(&mut self, now: u32) -> Result<()> {
        self.host
            .presentation
            .clock
            .as_mut()
            .context("elapsed clock absent")?
            .now = now;
        Ok(())
    }
    pub fn begin_clock_frame(&mut self) -> Result<()> {
        ensure!(
            self.host.presentation.application_parent.is_none(),
            "retained application caller must return before another elapsed frame begins"
        );
        self.clock_idle()?;
        self.host
            .presentation
            .clock
            .as_ref()
            .context("elapsed clock absent")?
            .validate(self)?;
        self.host.presentation.clock.as_mut().unwrap().call = Some(Call {
            frame: native::Frame::default(),
            tick: self.simulation.vm.tick,
            updates: 0,
            speed_override: None,
            notice: None,
        });
        Ok(())
    }
    pub(super) fn begin_clock_notice(&mut self, event: u32, argument: u32) -> Result<()> {
        let n = self
            .host
            .presentation
            .notifications
            .as_ref()
            .context("clock notification registry absent")?;
        let registry = n.dispatch.registry.clone();
        let fields = n.dispatch.fields.clone();
        self.begin_house_notification(registry, fields, event, argument)
    }
    pub(super) fn advance_clock_notice(&mut self) -> Result<bool> {
        match self.advance_house_notification(self.retained_camera_input()?, 1)? {
            notifications::Progress::Complete(_) => Ok(true),
            notifications::Progress::Pending | notifications::Progress::Service(_) => Ok(false),
        }
    }
    pub fn advance_clock(&mut self, budget: usize) -> Result<Progress> {
        self.advance_clock_monitored(budget, None, None)
    }
    pub fn advance_clock_with_trace(
        &mut self,
        budget: usize,
        trace: &mut Vec<sims_vm::Trace>,
    ) -> Result<Progress> {
        self.advance_clock_monitored(budget, Some(trace), None)
    }
    pub fn advance_clock_with_dependent_observer(
        &mut self,
        budget: usize,
        trace: &mut Vec<sims_vm::Trace>,
        observer: &mut dyn FnMut(&Self) -> Result<()>,
    ) -> Result<Progress> {
        self.advance_clock_monitored(budget, Some(trace), Some(observer))
    }
    fn advance_clock_monitored(
        &mut self,
        budget: usize,
        mut trace: Option<&mut Vec<sims_vm::Trace>>,
        mut observer: Option<&mut dyn FnMut(&Self) -> Result<()>>,
    ) -> Result<Progress> {
        ensure!(budget > 0, "elapsed clock needs a work budget");
        if let Some(parent) = &self.host.presentation.application_parent {
            parent.validate(
                self.host.presentation.clock.as_ref(),
                self.simulation.vm.tick,
            )?;
            ensure!(
                parent.awaiting_elapsed(),
                "retained application is at its unexecuted post-clock return"
            );
        }
        self.host
            .presentation
            .clock
            .as_ref()
            .context("elapsed clock absent")?
            .validate(self)?;
        for _ in 0..budget {
            let state = self.host.presentation.clock.as_mut().unwrap();
            let call = state.call.as_mut().context("elapsed frame has not begun")?;
            let g = &mut self.simulation.vm.world.globals;
            let mut input = native::Inputs {
                now: state.now,
                stopping: state.stopping,
                step_word: *g.get(&11).context("clock step word absent")?,
            };
            let request = call
                .frame
                .advance(&mut state.fields, &mut input)
                .map_err(anyhow::Error::msg)?;
            *g.get_mut(&11).unwrap() = input.step_word;
            let Some(request) = request else {
                let updates = call.updates;
                ensure!(
                    !call.frame.trapped(),
                    "original elapsed clock diagnostic break"
                );
                state.call = None;
                if let Some(parent) = &mut self.host.presentation.application_parent {
                    parent.clock_returned()?;
                }
                return Ok(Progress::Complete { updates });
            };
            match request {
                native::Request::ApplicationState => {
                    //40745f is a residential-zoning query on the lot, not a
                    //generic application-ready flag.
                    let world = &self.simulation.vm.world;
                    let people = world.person.as_ref().context("clock neighborhood absent")?;
                    let lot = *world.globals.get(&10).context("clock lot number absent")?;
                    call.frame
                        .acknowledge(
                            request,
                            u32::from(sims_vm::zoning::is_residential(&people.lot_zoning, lot)),
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                native::Request::RegistryPredicate => {
                    let value = native::needs_normal_speed(&self.simulation.vm.world)
                        .map_err(anyhow::Error::msg)?;
                    call.frame
                        .acknowledge(request, u32::from(value))
                        .map_err(anyhow::Error::msg)?;
                }
                native::Request::Boost | native::Request::RestoreSpeed => {
                    let speed = call.speed_override.get_or_insert_with(|| {
                        speed_override::Change::new(if request == native::Request::Boost {
                            speed_override::Command::Boost(1000)
                        } else {
                            speed_override::Command::Restore
                        })
                    });
                    let (mut current, mut saved) = (
                        *g.get(&16).context("clock speed absent")?,
                        *g.get(&18).context("clock saved speed absent")?,
                    );
                    let notice = speed
                        .advance(&mut state.fields, &mut current, &mut saved)
                        .map_err(anyhow::Error::msg)?;
                    g.insert(16, current);
                    g.insert(18, saved);
                    if let Some(notice) = notice {
                        let started = call.notice.is_some();
                        if !started {
                            self.begin_clock_notice(notice.event, notice.argument)?;
                            self.host
                                .presentation
                                .clock
                                .as_mut()
                                .unwrap()
                                .call
                                .as_mut()
                                .unwrap()
                                .notice = Some(notice);
                        }
                        if self.advance_clock_notice()? {
                            let call = self
                                .host
                                .presentation
                                .clock
                                .as_mut()
                                .unwrap()
                                .call
                                .as_mut()
                                .unwrap();
                            call.speed_override
                                .as_mut()
                                .unwrap()
                                .acknowledge(notice)
                                .map_err(anyhow::Error::msg)?;
                            call.notice = None;
                        }
                    } else {
                        call.speed_override = None;
                        call.frame
                            .acknowledge(request, 0)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                native::Request::UpdateLot => {
                    // This branch returns immediately after this one child;
                    // the borrowed observers cannot outlive or alter it.
                    let result = self.step_for_clock_monitored(trace.take(), observer.take());
                    let call = self
                        .host
                        .presentation
                        .clock
                        .as_mut()
                        .unwrap()
                        .call
                        .as_mut()
                        .unwrap();
                    call.tick = self.simulation.vm.tick;
                    let result = result?;
                    if matches!(result, super::Progress::Tick | super::Progress::PartialPass) {
                        call.frame
                            .acknowledge(request, 0)
                            .map_err(anyhow::Error::msg)?;
                        call.updates = call
                            .updates
                            .checked_add(1)
                            .context("clock frame update count overflow")?;
                    }
                    return Ok(Progress::Household(result));
                }
                native::Request::ProcessRequests => {
                    ensure!(
                        state.requests.is_empty(),
                        "original neighborhood request queue requires its service host"
                    );
                    call.frame
                        .acknowledge(request, 0)
                        .map_err(anyhow::Error::msg)?;
                }
                native::Request::Publish => anyhow::bail!(
                    "original neighborhood publication requires its application service host"
                ),
                native::Request::PeekMessage => return Ok(Progress::PollInput),
                native::Request::DebugBreak => {
                    anyhow::bail!("original elapsed clock diagnostic break")
                }
            }
        }
        Ok(Progress::Pending)
    }

    pub fn acknowledge_clock_input(&mut self, pending: bool) -> Result<()> {
        self.host
            .presentation
            .clock
            .as_mut()
            .and_then(|s| s.call.as_mut())
            .context("elapsed input poll absent")?
            .frame
            .acknowledge(native::Request::PeekMessage, u32::from(pending))
            .map_err(anyhow::Error::msg)
    }
}
