//! The original40726a caller, using retained factory, notification and physical
//!placement owners. A service budget does not stand in for a completed tick.
use super::{Session, notifications};
use anyhow::{Context, Result, ensure};

pub enum Progress {
    Pending,
    Service(notifications::Service),
    Complete,
}

impl Session {
    pub fn begin_original_controllers(&mut self) -> Result<()> {
        ensure!(
            !self.house_notification_pending(),
            "controller reconciliation overlaps a broadcast"
        );
        let definition = self
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("controller resource catalog absent")?
            .history
            .named_definition(b"comeandsee")
            .map_err(anyhow::Error::msg)?;
        if let Some(d) = &definition {
            sims_content::assets::load_object_guids(&self.index, &mut self.program, &[d.guid])?;
            self.simulation
                .architecture
                .as_mut()
                .context("controller architecture absent")?
                .bind_footprints(&self.program)
                .map_err(anyhow::Error::msg)?;
        }
        self.simulation
            .begin_controller_reconciliation(&self.program, definition)
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_original_controllers(&mut self, budget: usize) -> Result<Progress> {
        ensure!(budget > 0, "controller reconciliation needs a work budget");
        if self.house_notification_pending() {
            return Ok(
                match self.advance_house_notification(self.retained_camera_input()?, budget)? {
                    notifications::Progress::Service(s) => Progress::Service(s),
                    _ => Progress::Pending,
                },
            );
        }
        let ordinary = self.host.ordinary.graphics.keys().copied().collect();
        let (_, complete) = self
            .simulation
            .advance_controller_reconciliation(&self.program, &ordinary, budget)
            .map_err(anyhow::Error::msg)?;
        if complete {
            return Ok(Progress::Complete);
        }
        if self
            .simulation
            .controller_reconciliation
            .as_ref()
            .unwrap()
            .factory_pending()
        {
            if let Some((sims_sim::lot_selection::Request::CreationNotification(id), false)) =
                self.simulation.pending_native_selection()
            {
                self.begin_factory_broadcast(id)?;
            } else {
                self.service_selection_assets(self.retained_camera_input()?, budget)?;
            }
        } else if let Some(id) = self
            .simulation
            .controller_reconciliation
            .as_ref()
            .unwrap()
            .pending_graphic()
        {
            ensure!(
                !self.host.ordinary.graphics.contains_key(&id),
                "controller placement requires its ordinary graphics refresh: {id}"
            );
        }
        Ok(Progress::Pending)
    }
    pub fn finish_original_controllers(&mut self) -> Result<()> {
        ensure!(
            !self.house_notification_pending(),
            "controller broadcast has not returned"
        );
        self.simulation
            .finish_controller_reconciliation(&self.program)
            .map_err(anyhow::Error::msg)
    }
}
