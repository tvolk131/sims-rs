//! Read-only native selection projection and explicit existing portrait actions.
//! No selection, tutorial acknowledgement, asset loading or engine work occurs here.
use super::{Message, Session, Viewer, hud, live_resident, ui_publication, widget_lifetimes};
use iced::{
    Color, Element, Fill,
    widget::{self, button, column, container, row, text},
};
use sims_render::camera_device::panels::Panel;
use sims_vm::interface::view_control::Widget;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "resident_selection/tests.rs"]
mod tests;
#[cfg(test)]
pub(super) use tests::assert_capture_is_pure_for_test;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Selection {
    Selected(i16),
    Unselected,
    #[default]
    Unavailable,
}
impl Selection {
    pub(crate) fn is_selected(self, id: i16) -> bool {
        self == Self::Selected(id)
    }

    fn project(
        canonical: bool,
        query: Result<Option<i16>, String>,
        live: impl Fn(i16) -> bool,
    ) -> Self {
        if !canonical {
            return Self::Unavailable;
        }
        match query {
            Ok(None) => Self::Unselected,
            Ok(Some(id)) if id > 0 && live(id) => Self::Selected(id),
            _ => Self::Unavailable,
        }
    }
}

/// Original sessions use the registry scheduler and imported native lot. An
/// authored fixture's person flags alone do not establish that ownership.
fn canonical(session: &Session) -> bool {
    session.simulation.native_scheduler.is_some()
        && session.simulation.fixture_driver.is_none()
        && session.simulation.native_lot.is_some()
        && session.simulation.vm.world.scheduler_storage.is_some()
}

/// Also usable by the retained direct Viewer's parent-owned portrait adapter.
pub(super) fn selected(session: &Session) -> Selection {
    if !canonical(session) {
        return Selection::Unavailable;
    }
    Selection::project(true, session.simulation.native_selected_person(), |id| {
        live_resident(session, id).is_some()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    object: i16,
    neighbor: i16,
    guid: u32,
    source: String,
    lifetime: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub state: Selection,
    blocked: bool,
    identities: BTreeMap<i16, Identity>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: Selection::Unavailable,
            blocked: true,
            identities: BTreeMap::new(),
        }
    }
}
impl Snapshot {
    pub(super) fn capture(app: &Viewer) -> Self {
        let Some(session) = &app.session else {
            return Self::default();
        };
        let state = selected(session);
        let identities = session.house.household.as_ref().into_iter()
            .flat_map(|household| &household.residents)
            .filter_map(|resident| {
                let object = resident.object as i16;
                let live = live_resident(session, object)?;
                let lifetime = session.simulation.vm.world.scheduler_storage.as_ref()?.handle(object)?;
                session.simulation.has_interaction_actor(object).then(|| (object, Identity {
                    object,
                    neighbor: live.neighbor,
                    guid: live.guid,
                    source: live.character_source.clone(),
                    lifetime,
                }))
            }).collect();
        Self {
            state,
            blocked: app.presentation_pending()
                || app.player_shortcuts_blocked()
                || session.player_input_pending()
                || session.simulation.vm.world.registry_ui_disabled
                || app.picker_owns_input()
                || app.application_confirmation.is_some(),
            identities,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Action {
    identity: Identity,
    label: String,
}

fn actions(frontend: &ui_publication::view::Frontend, shell_enabled: bool) -> Vec<Action> {
    let data = &frontend.data;
    if !shell_enabled || frontend.diagnostics || frontend.options_open
        || !data.session_present || data.resident_selection.blocked
        || data.pending || data.modal || data.error.is_some()
        || !data.root_visible || !data.people_visible || data.panel_state == Some(1)
        || data.active_panel.is_some_and(|panel| panel != Panel::People)
        || data.button(Widget::Live).is_some_and(|button| !button.enabled)
        || data.cluster.as_ref().is_none_or(|cluster| !cluster.values.ready
            || cluster.values.pending || cluster.values.collapsed()
            || !cluster.values.people_visible || cluster.values.mode_raw != Some(2))
        || data.menu.is_some() || data.native_popup.is_some() || data.dialog.is_some()
        || data.needs.as_ref().is_some_and(|panel| panel.popup().is_some())
        || data.save_load_feedback.is_some()
        || data.resident_selection.state == Selection::Unavailable
    {
        return vec![];
    }
    data.residents.iter().filter_map(|resident| {
        if !resident.enabled || resident.name.trim().is_empty()
            || (data.resident_selection.state != Selection::Unselected && !resident.highlighted)
        {
            return None;
        }
        let identity = data.resident_selection.identities.get(&resident.object)?;
        if identity.neighbor != resident.neighbor {
            return None;
        }
        Some(Action { identity: identity.clone(), label: format!("Select {}", resident.name) })
    }).collect()
}

fn control(action: &Action, enabled: bool) -> Element<'static, Message> {
    widget_lifetimes::retain_owner_key(
        (action.identity.clone(), enabled),
        button(text(action.label.clone()).size(13))
            .padding([4, 8])
            .style(hud::text_button_style)
            .on_press_maybe(enabled.then_some(Message::Resident(action.identity.object)))
            .into(),
    )
}

/// The shell supplies its own picker/replacement/closing gate. Buttons retain
/// the existing Message and Host generation envelope; no new command exists.
pub(super) fn overlay(
    frontend: &ui_publication::view::Frontend,
    shell_enabled: bool,
) -> Option<Element<'static, Message>> {
    let actions = actions(frontend, shell_enabled);
    if actions.is_empty() {
        return None;
    }
    let identities: Vec<_> = actions.iter().map(|action| action.identity.clone()).collect();
    let buttons = column(actions.chunks(4).map(|group| {
        row(group.iter().map(|action| control(action, true))).spacing(6).into()
    })).spacing(4);
    let content = if frontend.resident_selection.state == Selection::Unselected {
        column![text("Select a Sim to give commands").size(13), buttons].spacing(4)
    } else {
        column![buttons]
    };
    Some(container(widget_lifetimes::retain_owner_key(
        identities,
        container(content).padding(8).style(hud::panel_style).into(),
    ))
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Bottom)
        .padding(iced::Padding {
            bottom: frontend.hud_height.max(183) as f32 + 8.,
            ..Default::default()
        })
        .into())
}

pub(super) fn portrait_style(selection: Selection, id: i16, status: button::Status) -> button::Style {
    let mut style = hud::text_button_style(&iced::Theme::Dark, status);
    style.border.color = if selection.is_selected(id) {
        Color::from_rgb8(0, 235, 235)
    } else {
        Color::from_rgb8(87, 108, 180)
    };
    style.border.width = 2.;
    style
}

/// Parent-only direct-Viewer adapter; the returned tree borrows no temporary
/// publication. The parent still owns the direct shell's picker/overlay wiring.
pub(super) fn viewer_overlay(app: &Viewer, shell_enabled: bool) -> Option<Element<'static, Message>> {
    let frontend = ui_publication::view::Frontend {
        data: std::sync::Arc::new(ui_publication::Published::capture(app)),
        diagnostics: app.diagnostics,
        options_open: app.options_open,
        widget_epoch: app.widget_epoch,
    };
    overlay(&frontend, shell_enabled)
}
