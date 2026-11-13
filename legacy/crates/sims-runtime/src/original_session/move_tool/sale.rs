//!487e18 with actual permission, placement, detach, price, DA, Reset and
//!Deactivate children.57a54c remains an explicit frontend audio boundary.
use super::*;
use sims_sim::object_catalog::move_tool::{
    feedback,
    sell::{self, Feedback, Reply, Request},
};
use sims_vm::budget::Application;

impl sell::View for Live<'_> {
    fn underlying(&self, cursor: u32) -> Result<Option<u32>, String> {
        Ok(self.world.objects[&id(self.world, cursor)?]
            .cursor
            .as_ref()
            .ok_or("sale cursor absent")?
            .underlying)
    }
    fn set_payload_cost(&mut self, payload: u32, cost: i32) -> Result<(), String> {
        self.objects.get_mut(payload)?.cost = Some(cost);
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    call: sell::Call,
    budget: u32,
    application: Option<Application>,
    feedback: Option<feedback::Call>,
}
fn application_time(s: &Session) -> Result<u32> {
    Ok(s.host
        .presentation
        .presenter
        .as_ref()
        .and_then(|p| p.camera.as_ref())
        .context("sale application clock absent")?
        .settings
        .clock)
}
impl Execution {
    pub(super) fn request(&self) -> Option<Request> {
        self.call.request()
    }
    pub(super) fn complete(&self) -> bool {
        self.call.complete() && self.application.is_none() && self.feedback.is_none()
    }
    pub(super) fn owns_notification(&self) -> bool {
        self.application.is_some()
    }
    pub(super) fn validate(&self, s: &Session, state: &State) -> Result<()> {
        self.call.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            state
                .preview_fields
                .as_ref()
                .is_some_and(|f| f.budget == self.budget),
            "sale budget owner changed"
        );
        ensure!(
            state.history.budgets.0.contains_key(&self.budget),
            "sale budget absent"
        );
        ensure!(
            state.history.budgets.0[&self.budget].category == state.tool.tool.cursor,
            "sale category projections differ"
        );
        ensure!(
            self.application.is_none() || self.feedback.is_none(),
            "sale owns two children"
        );
        if let Some(a) = &self.application {
            let Some(Request::Budget { category, amount }) = self.call.request() else {
                anyhow::bail!("sale budget lost its caller")
            };
            ensure!(a.input() == (category, amount), "sale budget input changed");
            if let Some(balance) = a.pending() {
                ensure!(
                    s.host
                        .presentation
                        .notifications
                        .as_ref()
                        .is_some_and(|n| n.dispatch.root_message() == Some((0xda, balance as u32))),
                    "sale lost its DA child"
                );
            } else {
                ensure!(a.complete(), "sale budget application not entered");
            }
        }
        if let Some(f) = &self.feedback {
            f.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.call.request() == Some(Request::Feedback(f.kind())),
                "sale feedback caller changed"
            );
        }
        let dispatch = &state
            .call
            .as_ref()
            .context("sale Move caller absent")?
            .dispatch;
        if !dispatch.complete() {
            let owned = match self.call.request() {
                Some(Request::Detach(h)) => dispatch.owns_direct(cleanup::Request::ObjectMode(h)),
                Some(Request::Reset) => dispatch.owns_cleanup(cleanup::Kind::Reset),
                Some(Request::Deactivate) => dispatch.owns_cleanup(cleanup::Kind::Deactivate),
                _ => false,
            };
            ensure!(owned, "sale shared child changed");
        }
        Ok(())
    }
    pub(super) fn advance(&mut self, s: &mut Session, state: &mut State) -> Result<Progress> {
        ensure!(
            state.call.as_ref().unwrap().dispatch.complete()
                && state.call.as_ref().unwrap().child.is_none(),
            "sale advanced before its shared child returned"
        );
        if let Some(request) = self.call.request() {
            match request {
                Request::CanSell(h) => {
                    let target = id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?;
                    let allowed = sims_sim::lot_cursor::permission::check_sale(
                        &mut s.simulation,
                        target,
                        state.history.placement_cheat,
                    )
                    .map_err(anyhow::Error::msg)?;
                    self.call
                        .acknowledge(request, Reply::Bool(allowed))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Placed(h) => {
                    let placed = motion::capture(s, h)?.on_lot;
                    self.call
                        .acknowledge(request, Reply::Bool(placed))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Resale(h) => {
                    let price = pickup::resale(
                        s,
                        id(&s.simulation.vm.world, h).map_err(anyhow::Error::msg)?,
                    )? as i16;
                    self.call
                        .acknowledge(request, Reply::Price(price))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Detach(_) | Request::Reset | Request::Deactivate => {
                    self.call
                        .acknowledge(request, Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Budget { category, amount } => {
                    if self.application.is_none() {
                        s.check_house_notification(0xda)?;
                        let n = s
                            .host
                            .presentation
                            .notifications
                            .as_ref()
                            .context("sale house listeners absent")?;
                        let (registry, fields) =
                            (n.dispatch.registry.clone(), n.dispatch.fields.clone());
                        sims_vm::commander::house::dispatch::Dispatch::begin(
                            registry.clone(),
                            fields.clone(),
                            0xda,
                            0,
                        )
                        .map_err(anyhow::Error::msg)?;
                        let mut a = Application::new(category, amount);
                        if let Some(balance) = s
                            .simulation
                            .advance_object_budget(&mut a)
                            .map_err(anyhow::Error::msg)?
                        {
                            s.install_house_notification(registry, fields, 0xda, balance as u32)?;
                        }
                        self.application = Some(a);
                    } else {
                        let a = self.application.as_mut().unwrap();
                        if let Some(balance) = a.pending() {
                            match s.advance_house_notification(s.retained_camera_input()?, 1)? {
                                super::super::notifications::Progress::Pending => {
                                    return Ok(Progress::Pending);
                                }
                                super::super::notifications::Progress::Service(r) => {
                                    return Ok(Progress::Service(Service::Notification(r)));
                                }
                                super::super::notifications::Progress::Complete(_) => (),
                            }
                            a.acknowledge(balance).map_err(anyhow::Error::msg)?;
                        }
                        ensure!(a.complete(), "sale budget application incomplete");
                        self.call
                            .acknowledge(request, Reply::Done)
                            .map_err(anyhow::Error::msg)?;
                        self.application = None;
                    }
                }
                Request::Feedback(kind) => {
                    let f = self
                        .feedback
                        .get_or_insert_with(|| feedback::Call::new(kind));
                    let now = application_time(s)?;
                    let last = s
                        .host
                        .presentation
                        .tool_selection
                        .as_mut()
                        .context("sale tool selector absent")?
                        .feedback_time_mut(kind)?;
                    if let Some(sound) = f.advance(last, now).map_err(anyhow::Error::msg)? {
                        return Ok(Progress::Service(Service::Feedback(sound)));
                    }
                    if f.complete() {
                        self.call
                            .acknowledge(request, Reply::Done)
                            .map_err(anyhow::Error::msg)?;
                        self.feedback = None;
                    }
                }
            }
            return Ok(Progress::Pending);
        }
        let budget = state
            .history
            .budgets
            .0
            .get_mut(&self.budget)
            .context("sale budget owner absent")?;
        let mut live = Live {
            world: &s.simulation.vm.world,
            objects: &mut state.history.objects,
        };
        match self
            .call
            .advance(&mut state.tool, budget, &mut live)
            .map_err(anyhow::Error::msg)?
        {
            Some(Request::Detach(h)) => {
                state.call.as_mut().unwrap().dispatch = dispatch::Dispatch::detach(h)
            }
            Some(Request::Reset) => {
                state.call.as_mut().unwrap().dispatch =
                    dispatch::Dispatch::cleanup(cleanup::Kind::Reset)
            }
            Some(Request::Deactivate) => {
                state.call.as_mut().unwrap().dispatch =
                    dispatch::Dispatch::cleanup(cleanup::Kind::Deactivate)
            }
            _ => (),
        }
        Ok(if self.complete() {
            Progress::Complete
        } else {
            Progress::Pending
        })
    }
}
impl Session {
    pub fn begin_move_tool_sale(&mut self) -> Result<()> {
        self.check_move_tool_entry()?;
        ensure!(!self.publication_pending(), "sale overlaps publication");
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("sale Move absent")?;
        state.validate(self)?;
        ensure!(
            !state.pending() && matches!(state.tool.tool.mode, 1 | 2),
            "sale needs an idle ordinary furniture tool"
        );
        let tool = state
            .preview_fields
            .as_ref()
            .context("sale field owner absent")?
            .budget;
        ensure!(
            self.host
                .presentation
                .current_tool
                .as_ref()
                .and_then(|c| c.current())
                == Some(tool),
            "sale current tool changed"
        );
        let selector = self
            .host
            .presentation
            .tool_selection
            .as_mut()
            .context("sale tool selector absent")?;
        ensure!(
            matches!(
                selector.receivers().get(&tool),
                Some(super::super::tool_selection::Receiver::Move)
            ),
            "sale tool receiver differs"
        );
        selector.bind_sale_feedback();
        //Older safe held checkpoints could retain a stale duplicate category.
        //The native caller reads the tool's+24 word. Project that live value
        //on this new entry; never rewrite any already-published expense totals.
        let moving = self.host.presentation.move_tool.as_mut().unwrap();
        moving
            .history
            .budgets
            .0
            .get_mut(&tool)
            .context("sale budget owner absent")?
            .category = moving.tool.tool.cursor;
        self.host.presentation.move_tool.as_mut().unwrap().call = Some(super::Execution {
            owner: None,
            current: None,
            tick: self.simulation.vm.tick,
            dispatch: dispatch::Dispatch::returned(),
            child: None,
            retired_held: None,
            retired_payload: None,
            preview: None,
            motion: None,
            commit: None,
            pickup: None,
            button_owner: None,
            sale: Some(Execution {
                call: Default::default(),
                budget: tool,
                application: None,
                feedback: None,
            }),
        });
        self.host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .validate(self)
    }
    ///Explicit disabled-audio57a54c return. The feedback caller still performs
    ///its post-return clock read/store on a subsequent retained slice.
    pub fn acknowledge_move_sale_feedback_disabled_audio(&mut self, kind: Feedback) -> Result<()> {
        let state = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("sale Move absent")?;
        state.validate(self)?;
        let sale = self
            .host
            .presentation
            .move_tool
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .and_then(|c| c.sale.as_mut())
            .context("sale caller absent")?;
        ensure!(
            sale.call.request() == Some(Request::Feedback(kind)),
            "sale sound parent differs"
        );
        sale.feedback
            .as_mut()
            .context("sale sound caller absent")?
            .acknowledge(kind)
            .map_err(anyhow::Error::msg)
    }
}
