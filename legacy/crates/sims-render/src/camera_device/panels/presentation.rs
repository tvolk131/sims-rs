//! Replacement panel visibility and input scope. Iced owns window focus and
//! painting; hidden panel children are removed from its tree and cannot accept
//! stale commands. No Windows surfaces or animation allocations are retained.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    People,
    Buy,
    Build,
    Options,
    Auxiliary,
    Camera,
}
impl Panel {
    pub const ALL: [Self; 6] = [
        Self::People,
        Self::Buy,
        Self::Build,
        Self::Options,
        Self::Auxiliary,
        Self::Camera,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    visible: [bool; 6],
    /// None until this newly bound frontend has published expansion. Earlier
    /// checkpoints displayed the panel; no legacy widget field is inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collapsed: Option<bool>,
}
impl Presentation {
    /// The replacement frontend supplies its actual initially displayed panel.
    /// This is not a reconstruction of Windows state from a household file.
    pub fn new(initial: Panel) -> Self {
        Self {
            visible: Panel::ALL.map(|p| p == initial),
            collapsed: None,
        }
    }
    pub fn visible(&self, panel: Panel) -> bool {
        self.root_visible() && self.visible[panel as usize]
    }
    pub fn collapsed(&self) -> Option<bool> {
        self.collapsed
    }
    pub fn root_visible(&self) -> bool {
        self.collapsed != Some(true)
    }
    pub fn active(&self) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| self.visible(*p))
    }
    pub(super) fn set_visible(&mut self, panel: Panel, visible: bool) {
        self.visible[panel as usize] = visible;
    }
    pub(super) fn set_collapsed(&mut self, collapsed: bool) -> bool {
        let changed = self.collapsed != Some(collapsed);
        self.collapsed = Some(collapsed);
        changed
    }
}
