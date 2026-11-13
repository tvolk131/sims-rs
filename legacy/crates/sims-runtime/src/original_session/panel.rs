//! Retained CPState expansion owner for the replacement frontend. Original
//! setter ordering lives in sims-vm; dirty-bit publication belongs to its real
//! widget backend. Historical snapshots do not acquire this owner on restore.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::interface::panel::{self, Fields, Inputs, Request, Setter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    fields: Fields,
    call: Option<Setter>,
    tick: u64,
}
impl State {
    pub fn value(&self) -> i32 {
        self.fields.value
    }
    pub fn pending(&self) -> bool {
        self.call.as_ref().is_some_and(|c| !c.complete()) || self.fields.flags & panel::DIRTY != 0
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            s.host
                .presentation
                .presenter
                .as_ref()
                .and_then(|p| p.camera.as_ref())
                .is_some(),
            "panel owner lost its camera binding"
        );
        if let Some(call) = &self.call {
            call.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            matches!(self.fields.value, 0 | 1),
            "frontend panel value unsupported"
        );
        ensure!(
            self.fields.flags & !panel::DIRTY == 0,
            "panel owner contains unrelated dirty bits"
        );
        if self.pending() {
            ensure!(
                self.tick == s.simulation.vm.tick,
                "panel update crossed a lot tick"
            );
            ensure!(
                !s.interface_update_pending()
                    && !s.ui_portrait_pending()
                    && !s.camera_command_pending()
                    && !s.ui_wait_pending()
                    && !s.postload_pending()
                    && !s.load_completion_pending()
                    && !s.view_refresh_pending()
                    && !s.house_notification_pending()
                    && !s.simulation.selection_work_pending()
                    && !s.host.ordinary.updates_pending()
                    && !s.host.presentation.awaiting_house_open
                    && s.simulation.vm.world.dialogs.modal.is_none()
                    && !s.simulation.vm.constructing_dialog()
                    && !s
                        .simulation
                        .vm
                        .threads
                        .values()
                        .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_))),
                "panel update overlaps synchronous work"
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Complete,
    Service(Request),
    Publish(i32),
}

impl Session {
    pub fn control_panel_pending(&self) -> bool {
        self.host
            .presentation
            .control_panel
            .as_ref()
            .is_some_and(State::pending)
    }
    /// Fresh replacement controls start collapsed, as CPState's original
    /// constructor does. The caller constructs the corresponding actual widget.
    pub fn bind_control_panel(&mut self) -> Result<()> {
        ensure!(
            self.host.presentation.control_panel.is_none(),
            "panel already bound"
        );
        let state = State {
            fields: Fields::constructed(0),
            call: None,
            tick: self.simulation.vm.tick,
        };
        ensure!(
            !self.interface_update_pending()
                && !self.ui_portrait_pending()
                && !self.camera_command_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.ui_wait_pending()
                && !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending()
                && self.simulation.vm.world.dialogs.modal.is_none()
                && !self.simulation.vm.constructing_dialog()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)))
                && !self.host.presentation.awaiting_house_open,
            "panel binding requires completed startup"
        );
        state.validate(self)?;
        self.host.presentation.control_panel = Some(state);
        Ok(())
    }
    pub fn begin_panel_toggle(&mut self, displayed: i32) -> Result<()> {
        let mode = self
            .host
            .presentation
            .presenter
            .as_ref()
            .and_then(|p| p.camera.as_ref())
            .context("panel camera absent")?
            .settings
            .application_mode;
        let value = panel::live_button_target(mode, displayed).map_err(anyhow::Error::msg)?;
        self.begin_panel_expansion(displayed, value)
    }
    /// Replacement sidebar disclosure uses the original4cef10 setter. This
    /// changes expansion only; it does not pretend to press the Live-mode key.
    pub fn begin_panel_expansion(&mut self, displayed: i32, value: i32) -> Result<()> {
        ensure!(matches!(value, 0 | 1), "unsupported panel expansion");
        ensure!(
            !self.player_input_pending(),
            "player input owns the live UI"
        );
        ensure!(
            !self.control_panel_pending() && !self.ui_portrait_pending(),
            "panel update already pending"
        );
        let current = self
            .host
            .presentation
            .control_panel
            .as_ref()
            .context("panel owner absent")?;
        ensure!(
            current.value() == displayed,
            "panel widget differs from committed state"
        );
        let next = State {
            fields: current.fields.clone(),
            call: Some(Setter::new(value, 0)),
            tick: self.simulation.vm.tick,
        };
        next.validate(self)?;
        self.host.presentation.control_panel = Some(next);
        Ok(())
    }
    pub fn advance_panel_update(&mut self) -> Result<Progress> {
        self.host
            .presentation
            .control_panel
            .as_ref()
            .context("panel owner absent")?
            .validate(self)?;
        let p = self.host.presentation.control_panel.as_mut().unwrap();
        if let Some(call) = &mut p.call {
            // Actual replacement registration has CPState but no legacy App+84
            // window hierarchy. Its4d3c39 branch is absent, never acknowledged
            // as though legacy child-window work had run.
            if let Some(request) = call
                .advance(
                    &mut p.fields,
                    Inputs {
                        app_present: true,
                        window_present: false,
                    },
                )
                .map_err(anyhow::Error::msg)?
            {
                return Ok(Progress::Service(request));
            }
        }
        Ok(panel::publication(&p.fields).map_or(Progress::Complete, Progress::Publish))
    }
    pub fn acknowledge_panel_service(&mut self, request: &Request) -> Result<()> {
        self.host
            .presentation
            .control_panel
            .as_ref()
            .context("panel owner absent")?
            .validate(self)?;
        self.host
            .presentation
            .control_panel
            .as_mut()
            .unwrap()
            .call
            .as_mut()
            .context("panel setter absent")?
            .acknowledge(request)
            .map_err(anyhow::Error::msg)
    }
    /// Called only after the backend changes the actual ViewControl property
    /// and widget visibility. Only this owner's dirty bit is consumed here.
    pub fn acknowledge_panel_publication(&mut self, value: i32) -> Result<()> {
        let p = self
            .host
            .presentation
            .control_panel
            .as_ref()
            .context("panel owner absent")?;
        p.validate(self)?;
        ensure!(
            p.call.as_ref().is_none_or(Setter::complete)
                && panel::publication(&p.fields) == Some(value),
            "panel publication differs"
        );
        self.host
            .presentation
            .control_panel
            .as_mut()
            .unwrap()
            .fields
            .flags &= !panel::DIRTY;
        Ok(())
    }
}
