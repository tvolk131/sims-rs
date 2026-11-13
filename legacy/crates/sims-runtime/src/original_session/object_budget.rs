//! Direct object budget application with the retained household DA dispatcher.
//! The object caller owns its accumulator; it is acknowledged only after all
//! listeners and their actual UI services return. No simulation tick intervenes.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::edit_history::object::{budget, driver};
use sims_vm::{budget::Application, commander::house::dispatch::Dispatch};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    request: budget::Apply,
    caller: driver::BudgetCaller,
    application: Application,
    tick: u64,
}
impl State {
    pub(super) fn caller(&self) -> driver::BudgetCaller {
        self.caller
    }
    pub(super) fn notification_pending(&self) -> bool {
        self.application.pending().is_some()
    }
    pub fn validate(&self, session: &Session) -> Result<()> {
        ensure!(
            session.object_budget_input_available(self.caller)
                && !session.transport_pending()
                && !session.clock_pending()
                && !session.camera_command_pending()
                && !session.interface_update_pending()
                && !session.control_panel_pending()
                && !session.ui_wait_pending()
                && !session.ui_portrait_pending(),
            "object budget overlaps application work"
        );
        ensure!(
            self.tick == session.simulation.vm.tick,
            "object budget crossed a lot tick"
        );
        ensure!(
            self.application.input() == (self.request.category, self.request.amount),
            "object budget input differs from its caller"
        );
        if let Some(balance) = self.application.pending() {
            let notification = session
                .host
                .presentation
                .notifications
                .as_ref()
                .context("object budget lost its notification")?;
            ensure!(
                notification.dispatch.root_message() == Some((0xda, balance as u32)),
                "object budget lost its DA child"
            );
            notification.validate(&session.host.presentation, &session.simulation)?;
        } else {
            ensure!(
                self.application.complete(),
                "saved object budget has not entered its application"
            );
        }
        Ok(())
    }
}
impl Session {
    fn object_budget_input_available(&self, caller: driver::BudgetCaller) -> bool {
        !self.ground_input_pending()
            && !self.object_input_pending()
            && self.simulation.object_pickup.is_none()
            && self.simulation.object_placement.is_none()
            && self.simulation.cursor_factory.is_none()
            && (!self.move_tool_pending()
                || self
                    .host
                    .presentation
                    .move_tool
                    .as_ref()
                    .is_some_and(|m| m.budget_caller() == Some(caller)))
    }
    /// Enter the actual pending service from the concrete ObjectUndoable stack.
    pub fn begin_object_budget(&mut self, caller: &driver::Execution) -> Result<()> {
        let Some(driver::Service::Budget(request)) = caller.service() else {
            anyhow::bail!("object command has no pending budget application");
        };
        let identity = caller
            .budget_caller()
            .context("object budget caller identity absent")?;
        self.check_house_notification(0xda)?;
        ensure!(
            self.host.presentation.object_budget.is_none()
                && !self.player_input_pending()
                && !self.transport_pending()
                && !self.clock_pending(),
            "object budget overlaps application work"
        );
        let notification = self
            .host
            .presentation
            .notifications
            .as_ref()
            .context("object budget requires the registered house listeners")?;
        notification.validate(&self.host.presentation, &self.simulation)?;
        //Preflight the existing context before the native stores. No callbacks
        //or ticks can change it between this check and installing the DA child.
        let registry = notification.dispatch.registry.clone();
        let fields = notification.dispatch.fields.clone();
        Dispatch::begin(registry.clone(), fields.clone(), 0xda, 0).map_err(anyhow::Error::msg)?;
        let mut state = State {
            request,
            caller: identity,
            application: Application::new(request.category, request.amount),
            tick: self.simulation.vm.tick,
        };
        let notice = self
            .simulation
            .advance_object_budget(&mut state.application)
            .map_err(anyhow::Error::msg)?;
        if let Some(balance) = notice {
            self.install_house_notification(registry, fields, 0xda, balance as u32)?;
        }
        self.host.presentation.object_budget = Some(state);
        self.host
            .presentation
            .object_budget
            .as_ref()
            .unwrap()
            .validate(self)
    }
    ///Drive the notification through the normal camera/UI backend first. This
    ///only returns the46d91c child;4929fb still owns clearing the accumulator.
    pub fn finish_object_budget(&mut self, caller: &mut driver::Execution) -> Result<()> {
        let state = self
            .host
            .presentation
            .object_budget
            .as_ref()
            .context("object budget caller absent")?;
        state.validate(self)?;
        let request = driver::Service::Budget(state.request);
        ensure!(
            caller.service() == Some(request) && caller.budget_caller() == Some(state.caller),
            "object budget caller changed"
        );
        if state.application.pending().is_some() {
            ensure!(
                self.host
                    .presentation
                    .notifications
                    .as_ref()
                    .unwrap()
                    .dispatch
                    .returned()
                    .is_some(),
                "object budget listeners have not returned"
            );
        }
        caller
            .acknowledge(request, driver::Reply::Done)
            .map_err(anyhow::Error::msg)?;
        self.host.presentation.object_budget = None;
        Ok(())
    }
}
