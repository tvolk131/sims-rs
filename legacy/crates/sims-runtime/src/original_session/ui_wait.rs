//! Original4764fe caller, preserving the captured object's synchronous named
//! callbacks. Polling cadence and actual widget observations belong to the app.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{
    Start, Status,
    ui_owner::wait::{self, Condition, Observation, Poll, Reply, Request},
};

use sims_sim::ui_wait::Callback;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct State {
    watch: wait::State,
    poll: Option<Poll>,
    tick: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Observe(Condition),
    Complete,
}
impl State {
    pub fn pending(&self) -> bool {
        self.poll.as_ref().is_some_and(|p| !p.complete())
    }
    pub fn baseline(&self) -> i32 {
        self.watch.baseline
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        if let Some(p) = &self.poll {
            p.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            !self.pending() || self.tick == s.simulation.vm.tick,
            "UI wait poll crossed a lot tick"
        );
        if let Some(c) = &s.simulation.ui_wait_callback {
            let name = match self.poll.as_ref().and_then(Poll::request) {
                Some(Request::Query(id)) if *id == c.object => "query wait event",
                Some(Request::Got(id)) if *id == c.object => "got wait event",
                _ => anyhow::bail!("UI wait callback lost its poll"),
            };
            ensure!(c.name() == name, "UI wait callback kind differs");
            c.validate(&s.simulation, &s.program)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}
impl Session {
    pub fn ui_wait_pending(&self) -> bool {
        self.host
            .presentation
            .ui_wait
            .as_ref()
            .is_some_and(State::pending)
    }
    pub fn pending_ui_observation(&self) -> Option<Condition> {
        match self
            .host
            .presentation
            .ui_wait
            .as_ref()?
            .poll
            .as_ref()?
            .request()?
        {
            Request::Observe(condition) => Some(condition.clone()),
            _ => None,
        }
    }
    pub fn begin_ui_wait(&mut self, focused_image_id: Option<i32>) -> Result<()> {
        ensure!(
            !self.player_input_blocks_ticks(),
            "player input owns the live UI"
        );
        ensure!(
            !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.camera_command_pending()
                && !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending()
                && !self.simulation.vm.constructing_dialog()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, Status::AwaitingUiEffect(_))),
            "UI wait poll overlaps synchronous work"
        );
        let poll = Poll::begin(self.simulation.vm.world.ui_owner.owner, focused_image_id)
            .map_err(anyhow::Error::msg)?;
        let state = self
            .host
            .presentation
            .ui_wait
            .get_or_insert_with(Default::default);
        state.poll = Some(poll);
        state.tick = self.simulation.vm.tick;
        Ok(())
    }
    pub fn acknowledge_ui_wait(
        &mut self,
        condition: &Condition,
        observation: Observation,
    ) -> Result<()> {
        ensure!(
            !self.ui_portrait_pending(),
            "portrait getter must return before observation acknowledgement"
        );
        let state = self
            .host
            .presentation
            .ui_wait
            .as_ref()
            .context("UI wait owner absent")?;
        state.validate(self)?;
        ensure!(
            self.simulation.ui_wait_callback.is_none(),
            "UI wait callback is active"
        );
        let state = self.host.presentation.ui_wait.as_mut().unwrap();
        state
            .poll
            .as_mut()
            .context("UI wait poll absent")?
            .acknowledge(
                &mut state.watch,
                &Request::Observe(condition.clone()),
                Reply::Observed(observation),
            )
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_ui_wait(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "UI wait poll requires a budget");
        ensure!(
            !self.ui_portrait_pending(),
            "portrait observation must return before UI wait polling"
        );
        self.host
            .presentation
            .ui_wait
            .as_ref()
            .context("UI wait owner absent")?
            .validate(self)?;
        let mut state = self.host.presentation.ui_wait.take().unwrap();
        let result = self.advance_ui_wait_inner(&mut state, budget);
        self.host.presentation.ui_wait = Some(state);
        result
    }
    fn advance_ui_wait_inner(&mut self, state: &mut State, budget: usize) -> Result<Progress> {
        for _ in 0..budget {
            let Some(request) = state.poll.as_ref().and_then(Poll::request).cloned() else {
                return Ok(Progress::Complete);
            };
            let reply = match &request {
                Request::PublishControl(value) => {
                    self.simulation.vm.world.globals.insert(12, *value);
                    Reply::Done
                }
                Request::Query(id) | Request::Got(id) => {
                    let name = if matches!(request, Request::Query(_)) {
                        "query wait event"
                    } else {
                        "got wait event"
                    };
                    if self.simulation.ui_wait_callback.is_none() {
                        if let Some((owner, bhav)) = self
                            .simulation
                            .vm
                            .resolve_named_tree(&self.program, *id, name)
                            .map_err(anyhow::Error::msg)?
                        {
                            let vm = &mut self.simulation.vm;
                            let thread = *vm
                                .object_threads
                                .get(id)
                                .context("UI wait object VM absent")?;
                            let t = &vm.threads[&thread];
                            ensure!(
                                t.callback.is_none()
                                    && !t.stack.is_empty()
                                    && !matches!(t.status, Status::Faulted(_))
                                    && vm.named_calls.is_empty(),
                                "UI wait needs an available existing object stack"
                            );
                            let depth = t.stack.len();
                            vm.begin_immediate_callback(
                                &self.program,
                                thread,
                                Start {
                                    owner: &owner,
                                    bhav,
                                    caller: *id,
                                    callee: *id,
                                    stack_object: 0,
                                    args: vm.world.objects[id].temps.to_vec(),
                                    check: true,
                                },
                            )
                            .map_err(anyhow::Error::msg)?;
                            self.simulation.ui_wait_callback = Some(Callback {
                                object: *id,
                                thread,
                                depth,
                                owner,
                                bhav,
                                query: matches!(request, Request::Query(_)),
                                tick: vm.tick,
                            });
                        }
                    }
                    if let Some(c) = &self.simulation.ui_wait_callback {
                        // A restored completed callback already has its result.
                        // Never run the suspended Main while consuming it.
                        if self.simulation.vm.threads[&c.thread].callback.is_some() {
                            self.simulation
                                .vm
                                .run_slice(&self.program, c.thread, 1)
                                .map_err(anyhow::Error::msg)?;
                        }
                        let t = &mut self.simulation.vm.threads.get_mut(&c.thread).unwrap();
                        if t.callback.is_some() {
                            ensure!(
                                t.status == Status::Ready,
                                "non-yielding UI wait callback requires its service owner: {:?}",
                                t.status
                            );
                            continue;
                        }
                        let returned = t
                            .callback_result
                            .take()
                            .context("UI wait callback result absent")?;
                        self.simulation.ui_wait_callback = None;
                        if matches!(request, Request::Query(_)) {
                            Reply::Query(returned.then(|| {
                                let t = self.simulation.vm.world.objects[id].temps;
                                [t[0], t[1]]
                            }))
                        } else {
                            Reply::Done
                        }
                    } else if matches!(request, Request::Query(_)) {
                        Reply::Query(None)
                    } else {
                        Reply::Done
                    }
                }
                Request::Observe(condition) => {
                    if *condition == Condition::PanelExpanded {
                        let Some(panel) = self.host.presentation.control_panel.as_ref() else {
                            return Ok(Progress::Observe(condition.clone()));
                        };
                        // Original4cfc6e reads CPState+230, not a synthetic event.
                        let value = panel.value();
                        state
                            .poll
                            .as_mut()
                            .unwrap()
                            .acknowledge(
                                &mut state.watch,
                                &request,
                                Reply::Observed(Observation::PanelExpansion(Some(value))),
                            )
                            .map_err(anyhow::Error::msg)?;
                        continue;
                    }
                    let p = self
                        .host
                        .presentation
                        .presenter
                        .as_ref()
                        .context("UI wait camera absent")?;
                    let camera = p
                        .camera
                        .as_ref()
                        .context("UI wait retained camera absent")?;
                    let counter = match condition {
                        Condition::Rotation => camera.settings.world_rotation as i32,
                        Condition::Scroll => camera.settings.attempts as i32,
                        _ => return Ok(Progress::Observe(condition.clone())),
                    };
                    Reply::Observed(Observation::Counter(counter))
                }
            };
            state
                .poll
                .as_mut()
                .unwrap()
                .acknowledge(&mut state.watch, &request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        state.validate(self)?;
        Ok(if state.pending() {
            Progress::Pending
        } else {
            Progress::Complete
        })
    }
}
