//! Actual retained text, pause-window and50a8ec backend for Interface4ce41b.
use super::{Device, Saved, mode, panels};
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::{
    Session,
    property_refresh::{Binding, Progress},
};
use sims_vm::commander::house::dispatch;

pub(super) fn validate(s: &Session, saved: &Saved) -> Result<()> {
    let Some(p) = s.host.presentation.property_refresh.as_ref() else {
        return Ok(());
    };
    p.validate(s)?;
    let w = saved
        .panel_widgets
        .as_ref()
        .context("property panel backend absent")?;
    w.validate_property_ids(p.binding.view, p.binding.renderer)?;
    let window = saved
        .mode_window
        .as_ref()
        .context("property mode window absent")?
        .panel_inputs()
        .0;
    ensure!(
        window != p.binding.view && window != p.binding.renderer,
        "property view/renderer aliases mode window"
    );
    ensure!(
        w.property_owner() == p.binding.panel
            && w.texts().is_some()
            && w.access().is_some()
            && w.presentation().is_some()
            && w.catalog().is_some()
            && saved.mode_window.is_some(),
        "property refresh backend differs or is incomplete"
    );
    panels::validate(s, saved)?;
    mode::validate(s, saved)
}
impl Device {
    pub fn advance_notification_with_client(
        &mut self,
        s: &mut Session,
        client: &mut crate::dialog::client::Client,
        budget: usize,
    ) -> Result<bool> {
        let complete = self.advance_notification(s, budget)?;
        if !complete
            && s.host
                .presentation
                .property_refresh
                .as_ref()
                .and_then(|p| p.panel_owner())
                .is_some()
        {
            self.publish_replacement_people(s, client)?;
        }
        Ok(complete)
    }
    ///Explicit attachment after real widget/history owners exist. This API
    ///does not change any old notification registry as a side effect of restore.
    pub fn bind_interface_properties(
        &mut self,
        s: &mut Session,
        view: u32,
        renderer: u32,
    ) -> Result<()> {
        let panel = self
            .property_panel_owner()
            .context("property panel absent")?;
        panels::validate(s, &self.saved)?;
        mode::validate(s, &self.saved)?;
        // Check all backend prerequisites before mutating the Session binding.
        let w = self.saved.panel_widgets.as_ref().unwrap();
        w.validate_property_ids(view, renderer)?;
        let window = self
            .saved
            .mode_window
            .as_ref()
            .context("property mode window absent")?
            .panel_inputs()
            .0;
        ensure!(
            window != view && window != renderer,
            "property view/renderer aliases mode window"
        );
        ensure!(
            w.texts().is_some()
                && w.access().is_some()
                && w.presentation().is_some()
                && w.catalog().is_some()
                && self.saved.mode_window.is_some(),
            "property backend not ready"
        );
        s.bind_property_refresh(Binding {
            panel,
            view,
            renderer,
        })?;
        validate(s, &self.saved)
    }
    pub(super) fn advance_property_listener(
        &mut self,
        s: &mut Session,
        caller: &dispatch::Service,
        budget: usize,
    ) -> Result<bool> {
        if !s
            .host
            .presentation
            .property_refresh
            .as_ref()
            .context("Interface property backend unbound")?
            .pending()
        {
            s.begin_property_refresh(caller)?;
        }
        validate(s, &self.saved)?;
        match s.advance_property_refresh(budget)? {
            Progress::Pending => (),
            Progress::Text {
                request,
                language,
                bytes,
            } => {
                self.property_text(request, language, bytes)?;
                s.acknowledge_property_text(request)?;
            }
            Progress::WindowPause(value) => {
                if self.advance_property_window(s, value)? {
                    s.acknowledge_property_window(value)?;
                }
            }
            Progress::Panels(owner) => {
                ensure!(
                    self.property_panel_owner() == Some(owner),
                    "property panel owner differs"
                );
                self.advance_panel_widgets(s, budget)?;
            }
            Progress::Complete => {
                s.complete_property_refresh(caller)?;
                self.capture_camera_controls(s)?;
                self.saved.selected_person = s
                    .simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?;
                return Ok(true);
            }
        }
        validate(s, &self.saved)?;
        Ok(false)
    }
}
