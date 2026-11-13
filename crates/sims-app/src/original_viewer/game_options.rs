use super::*;
use iced::widget::column;
use sims_render::camera_device::panels::options::{Section, View};

pub(super) fn available(view: Option<&View>, revision: u64, visible: bool, blocked: bool) -> bool {
    !blocked && visible && view.is_some_and(|view| view.mode == 3 && view.revision == revision)
}

pub(super) fn control_resource(message: &Message) -> Option<i32> {
    match message {
        Message::GameOptions => Some(2009),
        Message::GameOptionsSection(_, section) => Some(section.resource()),
        Message::GameOptionsSave(_) => Some(4950),
        _ => None,
    }
}

pub(super) fn view<'view>(options: &'view View, art: Option<&'view dialog_view::HudArt>, ready: bool) -> Element<'view, Message> {
    let mut sections = row![].spacing(8);
    for section in Section::ALL {
        sections = sections.push(hud::icon(art.map(|art| art.static_art()), section.resource(), section.label(),
            ready.then_some(Message::GameOptionsSection(options.revision, section)), options.section == section));
    }
    let mut details = column![text(options.section.label()).size(16)].spacing(3);
    details = match options.section {
        Section::Graphics => details
            .push(text(format!("Floor {} · zoom {} · rotation {}", options.sources.floor,
                options.sources.zoom, options.sources.rotation)).size(12))
            .push(text("Camera controls remain available in the main control panel.").size(12)),
        Section::Sound => details.push(text("Audio playback and sound settings are not hosted by this renderer.").size(12)),
        Section::Play => details
            .push(text(match options.sources.free_will {
                Some(true) => "Free will: on",
                Some(false) => "Free will: off",
                None => "Free will setting unavailable",
            }).size(12))
            .push(text(format!("Simulation paused in Options · selected speed {}", options.speed)).size(12)),
    };
    column![
        row![sections, hud::icon(art.map(|art| art.static_art()), 4950, "Save household",
            ready.then_some(Message::GameOptionsSave(options.revision)), false)].spacing(16),
        details,
        row![button("Continue Live").on_press_maybe(ready.then_some(Message::Live)),
            button("Application save/load menu…").on_press(Message::Options)].spacing(8)
    ].spacing(6).into()
}

#[cfg(test)]
#[path = "game_options/tests.rs"]
mod tests;
