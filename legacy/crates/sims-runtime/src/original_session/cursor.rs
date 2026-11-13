//!4215e6's application tail: publish the actual view pointer, finish the shared
//!40760d E2 broadcast, then return the selected cursor's native object ID.
use super::{Session, notifications};
use anyhow::{Context, Result, ensure};
use sims_sim::pose::portrait::view::Input;
use sims_vm::ObjectId;

impl Session {
    pub(super) fn validate_cursor_focus(&self) -> Result<()> {
        let view = &self.host.presentation;
        if let Some(h) = view.object_focus {
            let id = self
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|r| r.object_for_handle(h))
                .context("view focus lifetime ended")?;
            ensure!(
                self.simulation.vm.world.objects.contains_key(&id),
                "view focus object absent"
            );
        }
        if let Some(f) = &self.simulation.cursor_factory {
            if let Some(h) = f.published_focus() {
                ensure!(
                    view.object_focus == Some(h),
                    "cursor factory lost its published view focus"
                );
            }
            if f.pending_created().is_some_and(|(_, started)| started) {
                ensure!(
                    view.notifications.is_some(),
                    "cursor broadcast lost its application owner"
                );
            }
        }
        if self.simulation.cursor_cleanup_focus_cleared() {
            ensure!(
                view.object_focus.is_none(),
                "cursor cleanup lost its cleared focus"
            );
        }
        Ok(())
    }
    ///4a67de(NULL), shared by42191c and42186b, follows all physical cleanup.
    pub fn service_cursor_cleanup_focus(&mut self) -> Result<()> {
        ensure!(
            self.simulation.pending_cursor_cleanup_focus(),
            "cursor cleanup has not reached focus clear"
        );
        self.validate_cursor_focus()?;
        self.host.presentation.object_focus = None;
        self.simulation
            .complete_cursor_cleanup_focus()
            .map_err(anyhow::Error::msg)
    }
    /// Service one application boundary; None retains the synchronous caller.
    /// This does not set Buy mode, deduct funds, drop, or destroy the cursor.
    pub fn service_cursor_factory_return(
        &mut self,
        camera: Input,
        budget: usize,
    ) -> Result<Option<ObjectId>> {
        ensure!(budget > 0, "cursor return requires a positive budget");
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.view_refresh_pending()
                && !self.host.ordinary.updates_pending(),
            "cursor return overlaps presentation work"
        );
        self.validate_cursor_focus()?;
        let f = self
            .simulation
            .cursor_factory
            .as_ref()
            .context("cursor factory absent")?;
        if let Some(h) = f.pending_focus() {
            ensure!(
                !self.house_notification_pending(),
                "cursor focus overlaps another notification"
            );
            //4a67de resolves the shared view, then4ac2f4 writes exactly+1bc.
            self.host.presentation.object_focus = Some(h);
            self.simulation
                .complete_cursor_factory_focus(h)
                .map_err(anyhow::Error::msg)?;
            return Ok(None);
        }
        if let Some((h, started)) = f.pending_created() {
            if !started {
                self.begin_cursor_broadcast(h)?;
            } else {
                match self.advance_house_notification(camera, budget)? {
                    notifications::Progress::Pending | notifications::Progress::Complete(_) => (),
                    notifications::Progress::Service(service) => {
                        anyhow::bail!("cursor E2 requires original listener service: {service:?}")
                    }
                }
            }
            return Ok(None);
        }
        ensure!(
            f.returned().is_some(),
            "cursor placement has not reached its application tail"
        );
        ensure!(
            !self.house_notification_pending(),
            "cursor notification has not returned"
        );
        self.simulation
            .finish_cursor_factory(&self.program)
            .map(Some)
            .map_err(anyhow::Error::msg)
    }
}
