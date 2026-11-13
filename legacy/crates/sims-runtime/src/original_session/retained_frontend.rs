//! Persistent native publication inputs, separate from a suspended Rust call.
use super::{Session, publication};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// Opaque CPState+54 identity. No platform window tree is imported.
    pub panel_owner: Option<u64>,
    pub menu_present: bool,
    pub menu_active: u8,
    pub closing: u8,
    pub walls: i32,
    pub tool_gate: u32,
}
impl Fields {
    pub(super) fn validate(&self, s: &Session) -> Result<()> {
        ensure!(
            self.panel_owner != Some(0),
            "retained panel has null identity"
        );
        ensure!(
            s.host.presentation.camera_ui.is_some()
                && s.host.presentation.transport.is_some()
                && s.host.presentation.retained_tool_inputs.is_some(),
            "retained frontend lost camera, CPState or tool owners"
        );
        Ok(())
    }
}
impl Session {
    /// Read persistent owners without constructing or starting a publication.
    pub fn retained_publication_context(&self) -> Result<publication::Context> {
        let p = &self.host.presentation;
        let c = p
            .frontend_context
            .as_ref()
            .context("retained publication context absent")?;
        c.validate(self)?;
        Ok(publication::Context {
            panel_owner: c.panel_owner,
            closing: c.closing,
            walls: c.walls,
            tool_gate: c.tool_gate,
            current_tool: p
                .current_tool
                .as_ref()
                .context("retained current tool absent")?
                .current(),
            selected_person_index: p
                .transport
                .as_ref()
                .unwrap()
                .controls
                .selected_person_index
                .context("retained portrait index absent")?
                as u32,
        })
    }
}
