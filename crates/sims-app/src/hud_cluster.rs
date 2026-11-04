//! Read-only universal HUD cluster: owned data and immutable artwork only.
//! No Viewer/Session/Client/Device, engine services or producer cache reads.
use super::{CYAN, Control, Message, at, icon, panel_style, trim_layer};
use crate::dialog_view::HudArt;
use iced::{
    Color, Element, Fill,
    widget::{self, button, container, text},
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ButtonState {
    pub enabled: bool,
    pub selected: bool,
    pub auxiliary: i32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Buttons {
    pub live: Option<ButtonState>,
    pub buy: Option<ButtonState>,
    pub zoom_in: Option<ButtonState>,
    pub zoom_out: Option<ButtonState>,
    pub floor_one: Option<ButtonState>,
    pub floor_two: Option<ButtonState>,
    pub normal: Option<ButtonState>,
    pub fast: Option<ButtonState>,
    pub ultra: Option<ButtonState>,
}
impl Buttons {
    fn get(&self, kind: Control) -> Option<ButtonState> {
        match kind {
            Control::Live => self.live,
            Control::Buy => self.buy,
            Control::ZoomIn => self.zoom_in,
            Control::ZoomOut => self.zoom_out,
            Control::FloorOne => self.floor_one,
            Control::FloorTwo => self.floor_two,
            Control::Normal => self.normal,
            Control::Fast => self.fast,
            Control::Ultra => self.ultra,
            _ => None,
        }
    }
}
/// These are existing replacement-UI observations, NOT native-call completion.
/// In particular raw mode and the Interface receiver fallback remain verbatim.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Values {
    pub pending: bool,
    pub ready: bool,
    pub mode_raw: Option<i32>,
    pub panel_state_raw: Option<i32>,
    pub can_show_live: bool,
    pub can_show_buy: bool,
    pub floor: i32,
    pub native_clock: bool,
    pub pause_raw: Option<u8>,
    pub people_visible: bool,
    pub clock: String,
    pub funds: String,
    pub buttons: Buttons,
}
impl Values {
    pub fn mode(&self) -> i32 {
        self.mode_raw.unwrap_or(2)
    }
    pub fn collapsed(&self) -> bool {
        self.panel_state_raw == Some(1)
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Published {
    pub values: Values,
    pub art: Option<Arc<HudArt>>,
}
fn control<'a>(
    state: &Published,
    art: Option<&'a HudArt>,
    id: i32,
    label: &'a str,
    message: Message,
    kind: Option<Control>,
    available: bool,
    selected: bool,
) -> Element<'a, Message> {
    let published = kind.and_then(|k| state.values.buttons.get(k));
    icon(
        art.map(HudArt::static_art),
        id,
        label,
        (available && published.is_none_or(|s| s.enabled)).then_some(message),
        published.map_or(selected, |s| s.selected),
    )
}
pub(crate) fn view(state: &Published, options_open: bool) -> Element<'_, Message> {
    let art = state.art.as_deref();
    let ready = state.values.ready;
    let mode = state.values.mode();
    let collapsed = state.values.collapsed();
    let mut cluster = widget::stack![background(art, 2037, 220., 183.)]
        .width(220)
        .height(183);
    if let Some(trim) = art.and_then(|icons| trim_layer(icons, mode, !collapsed)) {
        cluster = cluster.push(trim);
    }
    for (id, label, message, kind, available, selected, x, y) in [
        (
            2010,
            "Live mode",
            Message::Live,
            Some(Control::Live),
            state.values.can_show_live,
            mode == 2,
            10.,
            6.,
        ),
        (
            2008,
            "Buy mode",
            Message::Buy,
            Some(Control::Buy),
            state.values.can_show_buy,
            mode == 1,
            69.,
            20.,
        ),
        (
            2012,
            "Rotate left",
            Message::RotateBack,
            None,
            true,
            false,
            9.,
            127.,
        ),
        (
            2011,
            "Rotate right",
            Message::Rotate,
            None,
            true,
            false,
            43.,
            127.,
        ),
        (
            2013,
            "Zoom in",
            Message::ZoomIn,
            Some(Control::ZoomIn),
            true,
            false,
            26.,
            105.,
        ),
        (
            2014,
            "Zoom out",
            Message::ZoomOut,
            Some(Control::ZoomOut),
            true,
            false,
            26.,
            149.,
        ),
        (
            2023,
            "Downstairs",
            Message::Floor(1),
            Some(Control::FloorOne),
            true,
            state.values.floor == 1,
            5.,
            92.,
        ),
        (
            2024,
            "Upstairs",
            Message::Floor(2),
            Some(Control::FloorTwo),
            true,
            state.values.floor == 2,
            5.,
            77.,
        ),
    ] {
        cluster = cluster.push(at(
            control(
                state,
                art,
                id,
                label,
                message,
                kind,
                ready && available,
                selected,
            ),
            x,
            y,
        ));
    }
    cluster = cluster.push(at(
        icon(
            art.map(|art| art.static_art()),
            2003,
            "Build mode",
            None,
            false,
        ),
        117.,
        45.,
    ));
    cluster = cluster.push(at(
        icon(
            art.map(|art| art.static_art()),
            2009,
            "Application options",
            Some(Message::Options),
            options_open,
        ),
        193.,
        125.,
    ));
    let paused = state.values.pause_raw.is_some_and(|v| v != 0);
    let glyph: Element<'_, Message> = if let Some(icons) = art {
        widget::image(icons.pause(paused))
            .width(15)
            .height(15)
            .into()
    } else {
        text("Ⅱ")
            .size(12)
            .color(if paused {
                Color::from_rgb8(255, 0, 0)
            } else {
                CYAN
            })
            .into()
    };
    let pause = button(glyph)
        .width(15)
        .height(15)
        .padding(0)
        .on_press_maybe(ready.then_some(Message::Toggle))
        .style(|_, _| button::Style {
            background: None,
            ..Default::default()
        });
    cluster = cluster.push(at(pause.into(), 85., 135.));
    if state.values.native_clock {
        use sims_vm::interface::speed::{FAST, NORMAL, ULTRA};
        for (id, speed, label, kind, x) in [
            (4705, NORMAL, "Normal speed", Control::Normal, 105.),
            (4706, FAST, "Fast speed", Control::Fast, 120.),
            (4707, ULTRA, "Ultra speed", Control::Ultra, 141.),
        ] {
            cluster = cluster.push(at(
                control(
                    state,
                    art,
                    id,
                    label,
                    Message::NativeSpeed(speed),
                    Some(kind),
                    ready,
                    false,
                ),
                x,
                135.,
            ));
        }
    } else {
        cluster = cluster.push(at(
            button(text("▶").size(12))
                .padding(0)
                .on_press_maybe(ready.then_some(Message::Toggle))
                .into(),
            105.,
            135.,
        ));
    }
    let clock = &state.values.clock;
    let funds = &state.values.funds;
    cluster = cluster.push(at(
        container(text(clock).size(10).color(CYAN))
            .width(100)
            .center_x(100)
            .into(),
        75.,
        119.,
    ));
    cluster = cluster.push(at(
        container(text(funds).size(13).color(CYAN))
            .width(108)
            .align_x(iced::alignment::Horizontal::Right)
            .into(),
        70.,
        158.,
    ));
    widget::opaque(cluster).into()
}
pub(crate) fn background(
    art: Option<&HudArt>,
    id: i32,
    width: f32,
    height: f32,
) -> Element<'static, Message> {
    if let Some((_, _, frames)) = art.and_then(|i| i.hud(id)) {
        if width == 0.
            && let Some(edge) = art.and_then(|i| i.hud_edge(id))
        {
            // 51b366/5160f0 copy the source at native pixels; they do not
            // squeeze the804px background into the580px panel at800x600.
            // Beyond the original supported width, extend its final
            // column so a wider application never exposes clear space.
            return widget::stack![
                widget::image(edge)
                    .width(Fill)
                    .height(height)
                    .content_fit(iced::ContentFit::Fill),
                widget::image(frames[0].clone())
                    .width(Fill)
                    .height(height)
                    .content_fit(iced::ContentFit::None)
            ]
            .width(Fill)
            .height(height)
            .into();
        }
        let image = widget::image(frames[0].clone())
            .height(height)
            .content_fit(iced::ContentFit::Fill);
        if width == 0. {
            image.width(Fill).into()
        } else {
            image.width(width).into()
        }
    } else {
        container(widget::Space::new())
            .width(if width == 0. { Fill } else { width.into() })
            .height(height)
            .style(panel_style)
            .into()
    }
}
