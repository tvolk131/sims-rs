//! Retained Save/Load receipts in play view, without resizing the camera or
//! turning unrelated transient status into an always-visible gameplay overlay.
use super::*;

pub(super) fn view(message: &str, hud_height: u32) -> Element<'_, Message> {
    container(
        container(text(message).size(13))
            .padding(10)
            .max_width(680)
            .style(container::rounded_box),
    )
    .width(Fill)
    .height(Fill)
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Bottom)
    .padding(iced::Padding {
        bottom: hud_height as f32 + 10.,
        left: 12.,
        right: 12.,
        ..Default::default()
    })
    .into()
}
