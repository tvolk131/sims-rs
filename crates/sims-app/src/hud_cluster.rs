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
    pub options: Option<ButtonState>,
    pub zoom_in: Option<ButtonState>,
    pub zoom_out: Option<ButtonState>,
    pub floor_one: Option<ButtonState>,
    pub floor_two: Option<ButtonState>,
    pub normal: Option<ButtonState>,
    pub fast: Option<ButtonState>,
    pub ultra: Option<ButtonState>,
    pub walls_down: Option<ButtonState>,
    pub walls_up: Option<ButtonState>,
    pub cutaway: Option<ButtonState>,
    pub walls: Option<i32>,
}
impl Buttons {
    fn get(&self, kind: Control) -> Option<ButtonState> {
        match kind {
            Control::Live => self.live,
            Control::Buy => self.buy,
            Control::Options => self.options,
            Control::ZoomIn => self.zoom_in,
            Control::ZoomOut => self.zoom_out,
            Control::FloorOne => self.floor_one,
            Control::FloorTwo => self.floor_two,
            Control::Normal => self.normal,
            Control::Fast => self.fast,
            Control::Ultra => self.ultra,
            Control::WallsDown => self.walls_down,
            Control::WallsUp => self.walls_up,
            Control::Cutaway => self.cutaway,
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct WallControl {
    pub id: i32,
    pub label: &'static str,
    pub walls: u32,
    pub widget: Control,
    pub x: f32,
    pub y: f32,
}
pub(crate) const WALL_CONTROLS: [WallControl; 3] = [
    WallControl {
        id: 2006,
        label: "Walls up",
        walls: 1,
        widget: Control::WallsDown,
        x: 56.,
        y: 80.,
    },
    WallControl {
        id: 2004,
        label: "Walls cutaway",
        walls: 4,
        widget: Control::WallsUp,
        x: 80.,
        y: 85.,
    },
    WallControl {
        id: 2007,
        label: "Walls down",
        walls: 2,
        widget: Control::Cutaway,
        x: 103.,
        y: 95.,
    },
];
pub(crate) fn wall_available(floor: i32, walls: u32) -> bool {
    let mask = if matches!(floor, 1 | 2) { 7 } else { 1 };
    walls & mask != 0
}
/// These are existing replacement-UI observations, NOT native-call completion.
/// In particular raw mode and the Interface receiver fallback remain verbatim.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Values {
    pub pending: bool,
    pub ready: bool,
    pub transport_ready: bool,
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
pub(crate) fn view(state: &Published, _options_open: bool) -> Element<'_, Message> {
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
    let walls = state.values.buttons.walls;
    for wall in WALL_CONTROLS {
        cluster = cluster.push(at(
            control(
                state,
                art,
                wall.id,
                wall.label,
                Message::Walls(wall.walls),
                Some(wall.widget),
                ready && walls.is_some() && wall_available(state.values.floor, wall.walls),
                walls == Some(wall.walls as i32),
            ),
            wall.x,
            wall.y,
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
        control(
            state,
            art,
            2009,
            "Game Options",
            Message::GameOptions,
            Some(Control::Options),
            ready && state.values.buttons.options.is_some(),
            false,
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
        .on_press_maybe(state.values.transport_ready.then_some(Message::Toggle))
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
                    state.values.transport_ready,
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

#[cfg(test)]
mod tests {
    use super::super::super::headless_input::Input;
    use super::*;
    use iced::{Point, mouse};

    #[test]
    fn pending_transport_clicks_reach_the_queue_and_keep_modal_and_widget_gates() {
        let mut state = Published {
            values: Values {
                pending: true,
                ready: false,
                transport_ready: true,
                mode_raw: Some(2),
                panel_state_raw: None,
                can_show_live: false,
                can_show_buy: false,
                floor: 1,
                native_clock: true,
                pause_raw: Some(1),
                people_visible: false,
                clock: String::new(),
                funds: String::new(),
                buttons: Buttons {
                    normal: Some(ButtonState {
                        enabled: true,
                        selected: false,
                        auxiliary: 0,
                    }),
                    ..Default::default()
                },
            },
            art: None,
        };
        let click = |ui: &mut Input, state: &Published, x| {
            ui.element_batch(
                view(state, false),
                [220, 183],
                &[
                    iced::Event::Mouse(mouse::Event::CursorMoved {
                        position: Point::new(x, 142.),
                    }),
                    iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ],
            )
            .messages
        };
        let mut input = Input::new().unwrap();
        assert!(matches!(
            click(&mut input, &state, 111.).as_slice(),
            [Message::NativeSpeed(_)]
        ));
        assert!(matches!(
            click(&mut input, &state, 92.).as_slice(),
            [Message::Toggle]
        ));
        state.values.transport_ready = false;
        assert!(click(&mut input, &state, 111.).is_empty());
        assert!(click(&mut input, &state, 92.).is_empty());
        state.values.transport_ready = true;
        state.values.buttons.normal.as_mut().unwrap().enabled = false;
        assert!(click(&mut input, &state, 111.).is_empty());
    }

    #[test]
    fn gui_wall_controls_emit_native_modes_at_their_cluster_rectangles() {
        let art = Arc::new(HudArt::synthetic_for_test(
            &[
                (2003, 39, 39),
                (2004, 21, 21),
                (2006, 21, 21),
                (2007, 22, 15),
                (2008, 45, 47),
                (2009, 23, 23),
                (2010, 50, 50),
                (2011, 27, 27),
                (2012, 27, 27),
                (2013, 27, 27),
                (2014, 27, 27),
                (2023, 21, 11),
                (2024, 22, 15),
                (2037, 220, 183),
            ],
            &[],
        ));
        let mut state = Published {
            values: Values {
                pending: false,
                ready: true,
                transport_ready: true,
                mode_raw: Some(2),
                panel_state_raw: None,
                can_show_live: true,
                can_show_buy: true,
                floor: 1,
                native_clock: true,
                pause_raw: Some(0),
                people_visible: false,
                clock: String::new(),
                funds: String::new(),
                buttons: Buttons {
                    walls: Some(4),
                    ..Default::default()
                },
            },
            art: Some(art),
        };
        let click = |ui: &mut Input, state: &Published, pixel_x, pixel_y| {
            ui.element_batch(
                view(state, false),
                [220, 183],
                &[
                    iced::Event::Mouse(mouse::Event::CursorMoved {
                        position: Point::new(pixel_x, pixel_y),
                    }),
                    iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ],
            )
            .messages
        };
        let walls = |ui: &mut Input, state: &Published| {
            [(66., 90.), (90., 95.), (114., 102.)].map(|(pixel_x, pixel_y)| {
                match click(ui, state, pixel_x, pixel_y).as_slice() {
                    [Message::Walls(walls)] => Some(*walls),
                    [] => None,
                    other => panic!("unexpected wall click messages {other:?}"),
                }
            })
        };
        let mut input = Input::new().unwrap();
        assert_eq!(walls(&mut input, &state), [Some(1), Some(4), Some(2)]);
        assert!(matches!(
            click(&mut input, &state, 18., 82.).as_slice(),
            [Message::Floor(2)]
        ));
        state.values.floor = 0;
        assert_eq!(walls(&mut input, &state), [Some(1), None, None]);
        state.values.floor = 2;
        state.values.buttons.walls_up = Some(ButtonState {
            enabled: false,
            selected: true,
            auxiliary: 0,
        });
        assert_eq!(walls(&mut input, &state), [Some(1), None, Some(2)]);
        state.values.buttons.walls_up = None;
        state.values.buttons.walls = None;
        assert_eq!(walls(&mut input, &state), [None; 3]);
        state.values.buttons.walls = Some(1);
        state.values.ready = false;
        assert_eq!(walls(&mut input, &state), [None; 3]);
    }

    #[test]
    fn gui_wall_availability_matches_view_control_floor_mask() {
        for floor in [0, 1, 2, 3] {
            let open = matches!(floor, 1 | 2);
            assert!(wall_available(floor, 1));
            assert_eq!(wall_available(floor, 2), open);
            assert_eq!(wall_available(floor, 4), open);
        }
        assert_eq!(
            WALL_CONTROLS.map(|wall| (wall.id, wall.walls, wall.widget)),
            [
                (2006, 1, Control::WallsDown),
                (2004, 4, Control::WallsUp),
                (2007, 2, Control::Cutaway),
            ]
        );
    }
}
