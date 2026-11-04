//! Single-owner read-only observation of the existing universal HUD inputs.
//! A pending owner is still pending. No native services, view preparation,
//! lookup warming, serialization, world clone or publication policy runs here.
use super::hud::cluster::{ButtonState, Buttons, Published, Values};
use super::*;
impl Viewer {
    pub(super) fn publish_hud_cluster(&mut self) {
        self.hud_cluster = self.observe_hud_cluster().map(Arc::new);
    }
    pub(super) fn observe_hud_cluster(&self) -> Option<Published> {
        let s = self.session.as_ref()?;
        let pending = self.presentation_pending();
        let widgets = self.device.as_ref().and_then(|d| d.panel_widgets());
        let button = |kind| {
            widgets.map(|w| {
                let b = w.button(kind);
                ButtonState {
                    enabled: b.enabled,
                    selected: b.selected,
                    auxiliary: b.auxiliary,
                }
            })
        };
        let texts = widgets.and_then(|w| w.texts());
        Some(Published {
            values: Values {
                pending,
                ready: !pending
                    && self.error.is_none()
                    && s.simulation.vm.world.dialogs.modal.is_none(),
                mode_raw: s
                    .host
                    .presentation
                    .transport
                    .as_ref()
                    .map(|t| t.controls.mode),
                panel_state_raw: self.device.as_ref().and_then(|d| d.panel_state()),
                can_show_live: s.host.presentation.control_panel.is_some(),
                can_show_buy: s.host.presentation.tool_selection.is_some(),
                floor: self.floor,
                native_clock: self.native_clock(),
                pause_raw: self.device.as_ref().and_then(CameraDevice::transport_pause),
                people_visible: self.people_panel_visible(),
                clock: household_text_label(s, texts.map(|t| t.clock()).unwrap_or_default(), true)
                    .unwrap_or_default(),
                funds: household_funds_label(s, texts.map(|t| t.funds()).unwrap_or_default())
                    .unwrap_or_default(),
                buttons: Buttons {
                    live: button(sims_vm::interface::view_control::Widget::Live),
                    buy: button(sims_vm::interface::view_control::Widget::Buy),
                    zoom_in: button(sims_vm::interface::view_control::Widget::ZoomIn),
                    zoom_out: button(sims_vm::interface::view_control::Widget::ZoomOut),
                    floor_one: button(sims_vm::interface::view_control::Widget::FloorOne),
                    floor_two: button(sims_vm::interface::view_control::Widget::FloorTwo),
                    normal: button(sims_vm::interface::view_control::Widget::Normal),
                    fast: button(sims_vm::interface::view_control::Widget::Fast),
                    ultra: button(sims_vm::interface::view_control::Widget::Ultra),
                },
            },
            art: self.art.clone(),
        })
    }
}
