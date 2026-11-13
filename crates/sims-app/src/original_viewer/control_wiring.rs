//! Window-owned adapters. No filesystem work or alternate camera mutation.
use super::*;

pub(super) const COPY_CANCELLED: &str = "Copy cancelled. No new write was requested. Save and Continue destinations are unchanged.";

#[derive(Debug, Clone)]
pub(super) struct LoadCompletion {
    pub(super) ticket: load_picker::Ticket,
    /// None is issued only by the whole successful Load tail.
    pub(super) feedback: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct CopyCompletion {
    pub(super) ticket: load_picker::Ticket,
    /// None requires a confirmed write AND the complete healthy Viewer tail.
    pub(super) feedback: Option<String>,
}

pub(super) fn camera_command(action: camera_shortcuts::Action) -> Message {
    use camera_shortcuts::Action;
    match action {
        Action::RotateLeft => Message::RotateBack,
        Action::RotateRight => Message::Rotate,
        Action::ZoomIn => Message::ZoomIn,
        Action::ZoomOut => Message::ZoomOut,
        Action::Downstairs => Message::Floor(1),
        Action::Upstairs => Message::Floor(2),
    }
}

pub(super) fn relay_camera(state: &mut camera_shortcuts::State, input: player_shortcuts::Input,
    source_generation: u64, current_generation: u64)
{
    use player_shortcuts::Input;
    match input {
        Input::KeyReleased { physical_key } => state.release(physical_key, source_generation),
        Input::FocusLost => state.focus_lost(),
        Input::FocusGained if source_generation == current_generation => state.focus_gained(),
        _ => {}
    }
}

pub(super) fn picker_allows(message: &Message, modal: bool) -> bool {
    matches!(message, Message::LoadFrom(..) | Message::CopyTo(..) | Message::Frame(_) | Message::FocusLost
        | Message::DisplaySize(_) | Message::Shift(_))
        || (modal && matches!(message, Message::DialogKey(_) | Message::Dialog(..)))
}

pub(super) fn picker_overlay<'a>(base: Element<'a, Message>, picker: Option<&'a load_picker::Picker>,
    enabled: bool, modal: bool) -> Element<'a, Message>
{
    picker_overlay_with_recent_gate(base, picker, enabled, modal, enabled)
}

pub(super) fn picker_overlay_with_recent_gate<'a>(base: Element<'a, Message>, picker: Option<&'a load_picker::Picker>,
    enabled: bool, modal: bool, recent_enabled: bool) -> Element<'a, Message>
{
    if let Some(picker) = picker.filter(|_| !modal) {
        widget::stack![base, widget::opaque(container(picker.view_with_recent_gate(enabled, recent_enabled).map(Message::LoadPicker))
            .width(Fill).height(Fill).center_x(Fill).center_y(Fill)
            .style(hud::panel_style))].into()
    } else { base }
}

impl Viewer {
    pub(super) fn recent_choices_enabled(&self) -> bool {
        self.picker_enabled() && self.copy_focused && !self.player_shortcuts_blocked()
    }
    pub(super) fn copy_picker(&self) -> bool {
        self.load_picker.as_ref().is_some_and(|p| p.purpose() == load_picker::Purpose::Copy)
    }
    pub(super) fn copy_enabled(&self) -> bool {
        self.picker_enabled() && self.copy_focused && !self.player_shortcuts_blocked()
    }
    pub(super) fn finish_picker_copy(&mut self, completion: CopyCompletion) {
        if completion.ticket.purpose() != load_picker::Purpose::Copy { return; }
        if self.load_picker.as_mut().is_some_and(|p| p.settle(&completion.ticket, completion.feedback.clone()))
            && completion.feedback.is_none()
        { self.load_picker = None; }
        self.copy_completion = Some(completion);
    }
    pub(super) fn native_modal(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.simulation.vm.world.dialogs.modal.is_some())
    }
    pub(super) fn load_owner_blocked(&self) -> bool {
        self.error.is_some() || self.menu_error.is_some() || self.native_modal()
            || self.presentation_pending()
            || self.session.as_ref().is_none_or(|s| s.running_menu() || s.player_input_pending()
                || s.simulation.vm.threads.values().any(|t| matches!(t.status, sims_vm::Status::Faulted(_))))
    }
    pub(super) fn camera_shortcuts_blocked(&self) -> bool {
        self.player_shortcuts_blocked() || self.load_owner_blocked()
    }
    pub(super) fn save_shortcut_blocked(&self) -> bool {
        self.player_shortcuts_blocked() || self.load_owner_blocked()
            || self.options_open || self.diagnostics || self.picker_owns_input()
            || self.application_confirmation.is_some()
    }
    pub(super) fn camera_command_enabled(&self, message: &Message) -> bool {
        use sims_vm::interface::view_control::Widget;
        let kind = match message {
            Message::ZoomIn => Some(Widget::ZoomIn),
            Message::ZoomOut => Some(Widget::ZoomOut),
            Message::Floor(1) => Some(Widget::FloorOne),
            Message::Floor(2) => Some(Widget::FloorTwo),
            _ => None,
        };
        // Match the actual camera buttons' dispatch authority. Read the
        // current owner again after queueing; publication age grants no input.
        kind.and_then(|kind| self.device.as_ref().and_then(|d| d.panel_widgets())
            .map(|widgets| widgets.button(kind)))
            .is_none_or(|button| button.enabled)
    }
    // A completed fatal attempt remains retained, but must not cover or
    // intercept the existing error/restart controls. It becomes visible again
    // when a healthy owner has Options open; no other publication settles it.
    pub(super) fn picker_owns_input(&self) -> bool {
        self.load_picker.as_ref().is_some_and(|picker| picker.pending()
            || (self.options_open && !self.diagnostics))
            && self.error.is_none()
    }
    pub(super) fn picker_enabled(&self) -> bool {
        self.options_open && !self.diagnostics && self.application_confirmation.is_none()
            && !self.load_owner_blocked()
    }
    pub(super) fn load_seed(&self) -> &Path {
        self.last_load_path.as_deref().or(self.args.load.as_deref()).unwrap_or(&self.args.save)
    }
    pub(super) fn finish_picker_load(&mut self, completion: LoadCompletion) {
        if completion.ticket.purpose() != load_picker::Purpose::Load { return; }
        if self.load_picker.as_mut().is_some_and(|picker| picker.settle(&completion.ticket, completion.feedback.clone()))
            && completion.feedback.is_none()
        {
            self.load_picker = None;
        }
        // A worker-owned Viewer has no shell picker. Its real reply transports
        // this authority; Host consumes it only after Worker::poll accepts the
        // exact worker ticket and its coherent publication.
        self.load_completion = Some(completion);
    }
}

fn copy_at<'a>(element: Element<'a, Message>, x: f32, y: f32) -> Element<'a, Message> {
    widget::pin(element).x(x).y(y).into()
}

// Keep the parent-owned original HUD untouched; this Options adapter adds a
// separate row so the new action cannot overlap the original icon rectangles.
impl Viewer {
    pub(super) fn copy_application_options(&self) -> Element<'_, Message> {
        let mut menu = widget::stack![hud::cluster::background(self.art.as_deref(), 4910, 0., 100.)]
            .width(Fill)
            .height(100);
        // Native5160f0 separators and captured original icon rectangles.
        // The five unhosted actions remain disabled rather than dispatching
        // unrelated application commands through their original artwork.
        for (id, label, message, x, y) in [
            (4950, player_shortcuts::save_caption(), Some(Message::Save), 29., 6.),
            (4951, "Neighborhood (unavailable)", None, 88., 6.),
            (4952, "Exit game (unavailable)", None, 159., 6.),
            (4957, "Graphics options (unavailable)", None, 29., 58.),
            (4958, "Sound options (unavailable)", None, 89., 58.),
            (4959, "Play options (unavailable)", None, 151., 58.),
        ] {
            menu = menu.push(copy_at(
                hud::icon(
                    self.art.as_deref().map(|art| art.static_art()),
                    id,
                    label,
                    message,
                    false,
                ),
                x,
                y,
            ));
        }
        for (id, x, y) in [(4916, 16., 51.), (4917, 222., 6.)] {
            if let Some((w, h, frames)) = self.art.as_ref().and_then(|i| i.hud(id)) {
                menu = menu.push(copy_at(
                    widget::image(frames[0].clone())
                        .width(*w as f32)
                        .height(*h as f32)
                        .into(),
                    x,
                    y,
                ));
            }
        }
        menu = menu.push(copy_at(text("Application").size(13).into(), 229., 4.));
        // These are explicit application actions, not original icon aliases.
        for (label, message, x, y) in [
            ("Load", Message::ChooseLoad, 229., 25.),
            ("Details", Message::Diagnostics, 315., 25.),
            ("Restart", Message::Restart, 229., 55.),
            ("Close", Message::Options, 315., 55.),
        ] {
            menu = menu.push(copy_at(
                button(text(label).size(12))
                    .width(76)
                    .height(24)
                    .padding(0)
                    .on_press(message)
                    .style(hud::text_button_style)
                    .into(),
                x,
                y,
            ));
        }
        let menu = widget::column![
            text(player_shortcuts::save_guidance()).size(12),
            super::ui_publication::view::copy_action(self.copy_enabled() && self.load_picker.is_none()),
            super::neighborhood_usability::controls(
                &self.application,
                self.application_confirmation,
                self.session.is_some(),
            ),
            text(self.feedback_status()).size(12),
            menu
        ];
        container(widget::opaque(menu))
            .width(Fill)
            .height(Fill)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding(iced::Padding {
                left: 220.,
                ..Default::default()
            })
            .into()
    }
}
