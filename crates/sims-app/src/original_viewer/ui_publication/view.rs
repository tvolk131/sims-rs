//! Pure Iced frontend. Its only owner is an immutable Published value.
//! There are no Session/Client/Device getters, locks, IO or engine callbacks.
use super::super::{
    Message, Viewport, camera_shortcuts, dialog_view, feedback_notice, hud, neighborhood_usability, player_shortcuts, published_control,
    resident_selection, widget_lifetimes,
};
use super::{Catalog, Published, QueueEntry};
use hud::motives;
use hud::tutorial_highlight;
use hud::{cluster, icon, panel_style, text_button_style};
use iced::{
    Color, Element, Fill,
    widget::{self, button, column, container, progress_bar, row, scrollable, slider, text},
};
use sims_vm::interface::view_control::Widget as Control;
use std::sync::Arc;
const CYAN: Color = Color::from_rgb8(0, 235, 235);
fn at<'a>(element: Element<'a, Message>, x: f32, y: f32) -> Element<'a, Message> {
    widget::pin(element).x(x).y(y).into()
}
pub(crate) struct Frontend {
    pub data: Arc<Published>,
    pub diagnostics: bool,
    pub options_open: bool,
    pub widget_epoch: u64,
}
impl std::ops::Deref for Frontend {
    type Target = Published;
    fn deref(&self) -> &Published {
        &self.data
    }
}
impl Frontend {
    pub fn view(&self) -> Element<'_, Message> {
        self.view_with_confirmation(None, self.session_present)
    }
    pub fn view_with_confirmation(
        &self,
        confirmation: Option<neighborhood_usability::Confirmation>,
        enabled: bool,
    ) -> Element<'_, Message> {
        self.view_with_selection_gate(confirmation, enabled, enabled)
    }
    /// The Host additionally knows about its Load picker and closing owner.
    pub fn view_with_selection_gate(
        &self,
        confirmation: Option<neighborhood_usability::Confirmation>,
        enabled: bool,
        selection_enabled: bool,
    ) -> Element<'_, Message> {
        self.view_with_copy_gate(confirmation, enabled, selection_enabled, enabled && selection_enabled)
    }
    pub(crate) fn view_with_copy_gate(
        &self, confirmation: Option<neighborhood_usability::Confirmation>, enabled: bool,
        selection_enabled: bool, copy_enabled: bool,
    ) -> Element<'_, Message> {
        widget_lifetimes::retain(
            self.widget_epoch,
            neighborhood_usability::recovery(
                self.view_content(confirmation, enabled, selection_enabled, copy_enabled),
                self.error.is_some(),
            ),
        )
    }
    fn diagnostic_widgets(&self) -> (Element<'_, Message>, Element<'_, Message>) {
        let mut toolbar = row![
            // Native bitmap2026 is a display control, not this transport button.
            published_control(
                if self.playing { "Pause" } else { "Play" },
                Message::Toggle,
                self.pause_button,
                true,
            ),
            button(if self.native_clock() {
                "Step frame"
            } else if self.native_transport() {
                "Step pass"
            } else {
                "Step"
            })
            .on_press(Message::Step)
        ]
        .spacing(8)
        .align_y(iced::Alignment::End);
        if self.panel_state.is_some() {
            toolbar = toolbar.push(self.control(2010, "Live Mode", Message::Live));
        }
        if self.can_buy {
            toolbar = toolbar.push(self.control(2008, "Buy Mode", Message::Buy));
        }
        if self.native_clock() {
            use sims_vm::interface::speed::{FAST, NORMAL, ULTRA};
            let current = self.native_speed.expect("native clock transport absent");
            for (speed, label) in [(NORMAL, "Normal"), (FAST, "Fast"), (ULTRA, "Ultra")] {
                use sims_vm::interface::view_control::Widget;
                let kind = match speed {
                    NORMAL => Widget::Normal,
                    FAST => Widget::Fast,
                    _ => Widget::Ultra,
                };
                toolbar = toolbar.push(published_control(
                    label,
                    Message::NativeSpeed(speed),
                    self.button(kind),
                    current == speed,
                ));
            }
        } else {
            for speed in [1., 2., 4., 16.] {
                toolbar = toolbar.push(
                    button(text(format!("{speed:.0}×")))
                        .on_press(Message::Speed(speed))
                        .style(if self.speed == speed {
                            button::primary
                        } else {
                            button::secondary
                        }),
                );
            }
        }
        toolbar = toolbar
            .push(button(player_shortcuts::save_caption()).on_press(Message::Save))
            .push(button("Load").on_press(Message::Load))
            .push(button("Restart").on_press(Message::Restart))
            .push(self.control(2012, camera_shortcuts::caption(camera_shortcuts::Action::RotateLeft), Message::RotateBack))
            .push(self.control(2011, camera_shortcuts::caption(camera_shortcuts::Action::RotateRight), Message::Rotate))
            .push(self.control(2013, camera_shortcuts::caption(camera_shortcuts::Action::ZoomIn), Message::ZoomIn))
            .push(self.control(2014, camera_shortcuts::caption(camera_shortcuts::Action::ZoomOut), Message::ZoomOut))
            .push(self.control(2023, camera_shortcuts::caption(camera_shortcuts::Action::Downstairs), Message::Floor(1)))
            .push(self.control(2024, camera_shortcuts::caption(camera_shortcuts::Action::Upstairs), Message::Floor(2)));
        let pause_display = self.pause;
        let status = if pause_display.is_some_and(|v| v != 0) {
            "PAUSED"
        } else if self.playing {
            "PLAYING"
        } else if pause_display.is_some() {
            // A diagnostic Step/load hold is separate from the lot pause word.
            "STOPPED"
        } else {
            "PAUSED"
        };
        let mut panel = column![text(status).size(14)].spacing(12);
        if self.session_present {
            if let Some(money) = &self.money {
                let mut funds = text(&money.funds);
                if money.highlighted {
                    funds = funds.color(Color::from_rgb8(70, 184, 159));
                }
                panel = panel.push(text(&money.clock));
                let mut row = row![].spacing(8);
                if let Some(secondary) = &money.secondary {
                    row = row.push(text(secondary));
                }
                if money.funds_visible {
                    row = row.push(funds);
                }
                panel = panel.push(row);
            } else if let Some(clock) = &self.clock_fallback {
                panel = panel.push(text(clock));
            }
            if self.people_visible {
                for r in &self.residents {
                    let portrait = self.art.as_ref().and_then(|i| i.resident(r.neighbor));
                    let content: Element<'_, Message> = if let Some(image) = portrait {
                        row![widget::image(image).width(45).height(45), text(&r.name)]
                            .spacing(10)
                            .align_y(iced::Alignment::Center)
                            .into()
                    } else {
                        text(&r.name).into()
                    };
                    if r.highlighted && let Some(art) = &self.art {
                        panel = panel.push(
                            widget::image(art.tutorial())
                                .width(50)
                                .height(50),
                        );
                    }
                    panel = panel.push(
                        button(content)
                            .on_press_maybe(r.enabled.then_some(Message::Resident(r.object)))
                            .style(if self.resident_selection.state.is_selected(r.object) {
                                button::primary
                            } else {
                                button::secondary
                            }),
                    );
                }
                if let Some(needs) = &self.needs {
                    if self.panel_state != Some(1) {
                        if self.tutorial_highlight == Some(4500)
                            && let Some(icons) = &self.art
                        {
                            panel =
                                panel.push(widget::image(icons.tutorial()).width(50).height(50));
                        }
                        panel = panel.push(
                            button("Needs")
                                .on_press_maybe(needs.person().map(|_| Message::Needs))
                                .style(if needs.selected() {
                                    button::primary
                                } else {
                                    button::secondary
                                }),
                        );
                        if needs.shown()
                            && let Some(person) = needs.person()
                        {
                            use sims_render::dialog::client::live_panel::MOTIVES;
                            for pair in (0..8).collect::<Vec<_>>().chunks(2) {
                                panel = panel.push(
                                    row(pair.iter().map(|&i| {
                                        let value = f32::from_bits(person.values[i]);
                                        button(
                                            column![
                                                text(MOTIVES[i].0).size(12),
                                                progress_bar(
                                                    0.0..=1.0,
                                                    ((value + 100.) / 200.).clamp(0., 1.)
                                                )
                                                .girth(7)
                                            ]
                                            .spacing(4),
                                        )
                                        .on_press(Message::MotiveDetails(i))
                                        .style(button::secondary)
                                        .width(Fill)
                                        .into()
                                    }))
                                    .spacing(10),
                                );
                            }
                        }
                    }
                } else if let Some(m) = &self.legacy_motives {
                    for pair in [
                        [("Hunger", 7), ("Energy", 5)],
                        [("Comfort", 6), ("Hygiene", 8)],
                        [("Bladder", 9), ("Fun", 15)],
                        [("Social", 14), ("Room", 13)],
                    ] {
                        panel = panel.push(
                            row(pair.into_iter().map(|(name, i)| {
                                column![
                                    text(name).size(12),
                                    progress_bar(
                                        0.0..=1.0,
                                        ((f32::from_bits(m[i]) + 100.) / 200.).clamp(0., 1.)
                                    )
                                    .girth(7)
                                ]
                                .width(Fill)
                                .spacing(4)
                                .into()
                            }))
                            .spacing(14),
                        );
                    }
                }
                if let Some(menu) = &self.menu {
                    panel = panel.push(text(&menu.diagnostic_title).size(18));
                    if menu.ready {
                        for choice in &menu.diagnostic {
                            panel = panel.push(widget_lifetimes::retain_owner(
                                self.native_popup_epoch,
                                button(text(&choice.label))
                                    .width(Fill)
                                    .on_press_maybe(
                                        choice.intent.as_ref().map(super::Intent::message),
                                    )
                                    .into(),
                            ));
                        }
                    } else {
                        panel = panel.push(text("Preparing interaction…").size(13));
                    }
                    if let Some(cancel) = &menu.cancel {
                        // The diagnostic fallback menu historically had no Cancel widget.
                        if !menu.preview {
                            panel = panel.push(widget_lifetimes::retain_owner(
                                self.native_popup_epoch,
                                button("Cancel").on_press(cancel.message()).into(),
                            ));
                        }
                    }
                    if let Some(error) = &menu.error {
                        panel = panel.push(text(error).size(12));
                    }
                } else {
                    panel = panel.push(text("Select an object in the room.").size(13));
                }
                panel = panel.push(text("INTERACTION QUEUE").size(13));
                panel = panel.push(self.queue_diagnostics());
            } else if let Some(active) = self.active_panel {
                use sims_render::camera_device::panels::Panel;
                panel = panel.push(
                    text(match active {
                        Panel::Buy => "Buy mode",
                        Panel::Build => "Build mode",
                        Panel::Options => "Options",
                        Panel::Camera => "Camera",
                        Panel::Auxiliary => "Details",
                        Panel::People => unreachable!(),
                    })
                    .size(18),
                );
                if active == Panel::Buy
                    && let Some(catalog) = &self.catalog
                {
                    if let Some(selected) = &self.furniture {
                        panel =
                            panel
                                .push(text(format!("Moving {}", selected.name)))
                                .push(text(format!("Value: ${}", selected.price)))
                                .push(button("Sell").on_press_maybe(self.sell_message()))
                                .push(button("Cancel move").on_press_maybe(
                                    (!self.pending).then_some(Message::DialogKey(27)),
                                ));
                    }
                    panel = panel.push(buy_catalog_view(catalog, !self.pending && !self.modal));
                }
            }
            if !self.root_visible {
                panel = panel.push(button("Show controls").on_press_maybe(
                    (!self.pending && !self.modal).then_some(Message::ExpandPanel),
                ));
            }
            panel = panel.push(text(self.tick_label.as_ref().unwrap()).size(12));
        }
        panel = panel
            .push(text("Zoom").size(12))
            .push(if let Some(zoom) = self.device_zoom {
                slider(1.0..=3.0, zoom as f32, |level| {
                    Message::Zoom((1u32 << level as u32) as f32 / 8.)
                })
                .step(1.0_f32)
            } else {
                slider(0.15..=1.5, self.zoom, Message::Zoom)
            });
        (toolbar.into(), panel.into())
    }
    fn view_content(
        &self,
        confirmation: Option<neighborhood_usability::Confirmation>,
        enabled: bool,
        selection_enabled: bool,
        copy_enabled: bool,
    ) -> Element<'_, Message> {
        // Play view never installs these controls. Avoid allocating and
        // formatting a complete sidebar only to drop it before layout.
        let diagnostics = self.diagnostics.then(|| self.diagnostic_widgets());
        self.view_with_diagnostics(diagnostics, confirmation, enabled, selection_enabled, copy_enabled)
    }
    /// Same-binary control retaining the former eager widget construction.
    #[cfg(test)]
    pub(crate) fn eager_view_for_test(&self) -> Element<'_, Message> {
        widget_lifetimes::retain(
            self.widget_epoch,
            self.view_with_diagnostics(
                Some(self.diagnostic_widgets()),
                None,
                self.session_present,
                self.session_present,
                self.session_present,
            ),
        )
    }
    fn view_with_diagnostics<'a>(
        &'a self,
        diagnostics: Option<(Element<'a, Message>, Element<'a, Message>)>,
        confirmation: Option<neighborhood_usability::Confirmation>,
        enabled: bool,
        selection_enabled: bool,
        copy_enabled: bool,
    ) -> Element<'a, Message> {
        let viewport: Element<_> = if let Some(scene) = &self.scene {
            container(
                widget::mouse_area(
                    widget::shader::Shader::new(Viewport(scene.clone()))
                        .width(Fill)
                        .height(Fill),
                )
                .on_enter(Message::ViewportEnter)
                .on_exit(Message::ViewportLeave)
                .interaction(if self.cursor_idle {
                    iced::mouse::Interaction::Idle
                } else {
                    iced::mouse::Interaction::None
                }),
            )
            .padding(iced::Padding {
                bottom: if self.diagnostics {
                    0.
                } else {
                    self.hud_height() as f32
                },
                ..Default::default()
            })
            .into()
        } else {
            container(text("Loading original assets…"))
                .center(Fill)
                .into()
        };
        // This popup is independent of the scrollable/collapsed bar strip.
        // Its original script-visible ownership must correspond to visible UI.
        let viewport: Element<_> = if let Some((person, index)) =
            self.needs.as_ref().and_then(|p| p.person().zip(p.popup()))
        {
            use sims_render::dialog::client::live_panel::MOTIVES;
            let detail = container(
                column![
                    text(format!("{} · {}", person.name, MOTIVES[index].0)).size(18),
                    text(format!("{:.0} / 100", f32::from_bits(person.values[index]))),
                    button("Close details").on_press(Message::CloseMotiveDetails),
                ]
                .spacing(10),
            )
            .padding(12)
            .width(250)
            .style(container::rounded_box);
            widget::stack![
                viewport,
                container(widget::opaque(detail))
                    .width(Fill)
                    .height(Fill)
                    .align_x(iced::alignment::Horizontal::Right)
                    .align_y(iced::alignment::Vertical::Top)
                    .padding(12)
            ]
            .into()
        } else {
            viewport
        };
        let viewport: Element<_> = widget::stack![
            viewport,
            container(self.queue_hud())
                .width(Fill)
                .padding(if self.diagnostics { 10 } else { 33 }),
        ]
        .into();
        let viewport: Element<_> = if let Some(menu) = self.interaction_popup() {
            widget::stack![viewport, container(widget::opaque(menu)).center(Fill)].into()
        } else {
            viewport
        };
        let page = if self.diagnostics {
            let (toolbar, panel) = diagnostics.expect("diagnostic controls absent");
            let title = &self.title;
            column![
            text(format!("The {title} household")).size(27),
            text(match self.policy {
                sims_sim::lot_route::path::Policy::Strict =>
                    "Original house · strict routing validation",
                sims_sim::lot_route::path::Policy::InferredRectangles =>
                    "Original house · experimental routing (native rectangle search unverified)",
            })
            .size(13),
            row![toolbar,button("Play view").on_press(Message::Diagnostics)].spacing(8),
            row![viewport, scrollable(panel).width(300)]
                .spacing(16)
                .height(Fill),
            text(&self.status).size(12)
        ]
            .padding(20)
            .spacing(14)
        } else {
            let mut play = widget::stack![
                viewport,
                container(self.household_hud())
                    .width(Fill)
                    .height(Fill)
                    .align_y(iced::alignment::Vertical::Bottom),
            ];
            if self.paused_indicator {
                play = play.push(
                    container(
                        text("Paused")
                            .font(iced::Font {
                                family: iced::font::Family::Name("Comic Sans MS"),
                                ..iced::Font::DEFAULT
                            })
                            .size(64)
                            .color(Color::from_rgb8(255, 0, 0)),
                    )
                    .width(Fill)
                    .align_x(iced::alignment::Horizontal::Center)
                    .padding(iced::Padding {
                        top: 8.,
                        ..Default::default()
                    }),
                );
            }
            play = play.push(self.tutorial_hud_overlay());
            if let Some(selection) = resident_selection::overlay(
                self, enabled && selection_enabled && confirmation.is_none(),
            ) {
                play = play.push(selection);
            }
            play = play.push(neighborhood_usability::notice(&self.application));
            if let Some(feedback) = &self.save_load_feedback {
                play = play.push(feedback_notice::view(feedback, self.hud_height()));
            }
            let play: Element<_> = play.into();
            let play = if self.options_open {
                widget::stack![
                    play,
                    self.application_options_with_confirmation(confirmation, enabled, copy_enabled)
                ]
                .into()
            } else {
                play
            };
            column![play].height(Fill)
        };
        let page: Element<_> = if let Some(error) = &self.error {
            //Feedback overlays the existing layout: an error must not steal
            //height from the shader and rescale its frozen camera surface.
            widget::stack![
                page,
                container(text(error).size(12).color(Color::from_rgb8(245, 131, 115)))
                    .width(Fill)
                    .height(Fill)
                    .align_y(iced::alignment::Vertical::Bottom)
            ]
            .into()
        } else {
            page.into()
        };
        if let (Some(record), Some(icons)) = (&self.dialog, &self.art) {
            return widget::stack![page, dialog_view::overlay(record, icons, Message::Dialog)]
                .into();
        }
        page.into()
    }
    fn control<'a>(&'a self, id: i32, label: &'a str, message: Message) -> Element<'a, Message> {
        let marker: Element<'a, Message> = if self.tutorial_highlight == Some(id) {
            self.art.as_ref().map_or_else(
                || widget::Space::new().width(50).height(50).into(),
                |icons| widget::image(icons.tutorial()).width(50).height(50).into(),
            )
        } else {
            widget::Space::new().width(50).height(50).into()
        };
        // Original5de13f centers the50px highlight three pixels above the
        // target. Iced owns control layout; this shows the initial atlas frame.
        use sims_vm::interface::view_control::Widget;
        let kind = match id {
            2010 => Some(Widget::Live),
            2008 => Some(Widget::Buy),
            2013 => Some(Widget::ZoomIn),
            2014 => Some(Widget::ZoomOut),
            2023 => Some(Widget::FloorOne),
            2024 => Some(Widget::FloorTwo),
            _ => None,
        };
        let published = kind.and_then(|kind| self.button(kind));
        column![marker, published_control(label, message, published, true)]
            .spacing(3)
            .align_x(iced::Alignment::Center)
            .into()
    }
    fn native_clock(&self) -> bool {
        self.data.native_clock
    }
    fn native_transport(&self) -> bool {
        self.data.native_transport
    }
    fn hud_height(&self) -> u32 {
        self.data.hud_height
    }
    fn sell_message(&self) -> Option<Message> {
        self.furniture
            .as_ref()
            .and_then(|f| f.sell.as_ref().map(super::Intent::message))
    }
    fn hud_control<'a>(
        &'a self,
        id: i32,
        label: &'a str,
        message: Message,
        kind: Option<Control>,
        available: bool,
        selected: bool,
    ) -> Element<'a, Message> {
        let published = kind.and_then(|k| self.button(k));
        let control = icon(
            self.art.as_deref().map(|art| art.static_art()),
            id,
            label,
            (available && published.is_none_or(|s| s.enabled)).then_some(message),
            published.map_or(selected, |s| s.selected),
        );
        control
    }
    /// Paint in the whole-window layer so indicators above Live/Buy are not
    /// clipped to the183px HUD cluster. The target is the actual hosted icon.
    pub(super) fn tutorial_hud_overlay(&self) -> Element<'_, Message> {
        widget::responsive(move |size| {
            let mut layer = widget::stack![widget::Space::new().width(Fill).height(Fill)]
                .width(Fill)
                .height(Fill);
            if let (Some(id), Some(icons)) = (self.tutorial_highlight, self.art.as_ref()) {
                let target = match id {
                    2010 => Some([10, size.height as i32 - 183 + 6]),
                    2008 => Some([69, size.height as i32 - 183 + 20]),
                    2011 => Some([43, size.height as i32 - 183 + 127]),
                    2012 => Some([9, size.height as i32 - 183 + 127]),
                    2013 => Some([26, size.height as i32 - 183 + 105]),
                    2014 => Some([26, size.height as i32 - 183 + 149]),
                    2023 => Some([5, size.height as i32 - 183 + 92]),
                    2024 => Some([5, size.height as i32 - 183 + 77]),
                    4500 if self.panel_state != Some(1) && self.needs.is_some() => {
                        Some([size.width as i32 - 388 + 4, size.height as i32 - 100 + 33])
                    }
                    _ => None,
                };
                if let Some(position) = target
                    && let Some((width, height, _)) = icons.hud(id)
                {
                    layer = layer.push(tutorial_highlight::layer(
                        icons,
                        position,
                        [*width, *height],
                    ));
                }
            }
            layer.into()
        })
        .into()
    }
    /// Native reference: 800x600 cWin rectangles captured on October 22.
    /// Only the artwork/layout changes here. Existing command owners still
    /// decide mode transitions, scheduler controls and callback ordering.
    pub(super) fn household_hud(&self) -> Element<'_, Message> {
        let Some(state) = self.cluster.as_deref() else {
            return widget::Space::new().into();
        };
        let ready = state.values.ready;
        let mode = state.values.mode();
        let collapsed = state.values.collapsed();
        let cluster = cluster::view(state, self.options_open);
        let mut hud = widget::stack![widget::Space::new().width(Fill).height(183)];
        if !collapsed {
            let content: Element<_> = if !state.values.people_visible && mode == 1 {
                container(scrollable(self.catalog_hud(ready)))
                    .height(150)
                    .into()
            } else {
                row![self.residents_hud(), self.needs_hud(ready)]
                    .align_y(iced::Alignment::Center)
                    .height(100)
                    .into()
            };
            let height = if mode == 1 { 150. } else { 100. };
            let strip = widget::stack![
                self.hud_background(if mode == 1 { 4918 } else { 4910 }, 0., height),
                widget::opaque(content)
            ]
            .width(Fill)
            .height(height);
            hud = hud.push(
                container(strip)
                    .padding(iced::Padding {
                        left: 220.,
                        ..Default::default()
                    })
                    .width(Fill)
                    .height(183)
                    .align_y(iced::alignment::Vertical::Bottom),
            );
        }
        hud.push(cluster).into()
    }
    fn hud_background(&self, id: i32, width: f32, height: f32) -> Element<'_, Message> {
        cluster::background(self.art.as_deref(), id, width, height)
    }
    pub(super) fn application_options(&self) -> Element<'_, Message> {
        self.application_options_with_confirmation(None, self.session_present, self.session_present)
    }
    fn application_options_with_confirmation(
        &self,
        confirmation: Option<neighborhood_usability::Confirmation>,
        enabled: bool,
        copy_enabled: bool,
    ) -> Element<'_, Message> {
        let mut menu = widget::stack![self.hud_background(4910, 0., 100.)]
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
            menu = menu.push(at(
                icon(
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
                menu = menu.push(at(
                    widget::image(frames[0].clone())
                        .width(*w as f32)
                        .height(*h as f32)
                        .into(),
                    x,
                    y,
                ));
            }
        }
        menu = menu.push(at(text("Application").size(13).into(), 229., 4.));
        // These are explicit application actions, not original icon aliases.
        for (label, message, x, y) in [
            ("Load", Message::ChooseLoad, 229., 25.),
            ("Details", Message::Diagnostics, 315., 25.),
            ("Restart", Message::Restart, 229., 55.),
            ("Close", Message::Options, 315., 55.),
        ] {
            menu = menu.push(at(
                button(text(label).size(12))
                    .width(76)
                    .height(24)
                    .padding(0)
                    .on_press(message)
                    .style(text_button_style)
                    .into(),
                x,
                y,
            ));
        }
        let menu = column![
            copy_action(copy_enabled && !self.load_picker_blocked && !self.player_shortcuts_blocked
                && self.error.is_none() && !self.modal && confirmation.is_none()),
            neighborhood_usability::controls(&self.application, confirmation, enabled),
            text("Space: pause/resume during play; answer an open dialog.").size(12),
            text(player_shortcuts::save_guidance()).size(12),
            container(text(&self.status).size(12))
                .width(Fill)
                .padding(8)
                .style(panel_style),
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
    #[cfg(test)]
    pub(crate) fn selection_widgets_for_test(&self, kind: u8) -> Element<'_, Message> {
        match kind {
            0 => self.residents_hud(),
            1 => self.needs_hud(true),
            _ => self.diagnostic_widgets().1,
        }
    }
    fn residents_hud(&self) -> Element<'_, Message> {
        let mut people = row![].spacing(0);
        if self.people_visible {
            for resident in &self.residents {
                let id = resident.object;
                let face: Element<'_, Message> = self
                    .art
                    .as_ref()
                    .and_then(|i| i.resident(resident.neighbor))
                    .map(|im| widget::image(im).width(45).height(45).into())
                    .unwrap_or_else(|| text(&resident.name).size(12).into());
                let mut item: Element<_> = button(face)
                    .padding(0)
                    .on_press_maybe(resident.enabled.then_some(Message::Resident(id)))
                    .style(move |_, status| {
                        resident_selection::portrait_style(self.resident_selection.state, id, status)
                    })
                    .into();
                if resident.highlighted
                    && let Some(icons) = &self.art
                {
                    item =
                        widget::stack![item, widget::image(icons.tutorial()).width(45).height(45)]
                            .into();
                }
                people = people.push(widget::tooltip(
                    item,
                    text(&resident.name),
                    widget::tooltip::Position::Top,
                ));
            }
        }
        container(
            scrollable(people).direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            )),
        )
        .padding([5, 9])
        .width(Fill)
        .height(100)
        .into()
    }
    fn needs_hud(&self, ready: bool) -> Element<'_, Message> {
        let Some(needs) = self.needs.as_ref() else {
            return widget::Space::new().into();
        };
        if self.panel_state == Some(1) {
            return container(
                button("Show controls")
                    .on_press_maybe(ready.then_some(Message::ExpandPanel))
                    .style(text_button_style),
            )
            .width(300)
            .into();
        }
        let mut bars = column![].spacing(1);
        if needs.shown()
            && let Some(person) = needs.person()
        {
            use sims_render::dialog::client::live_panel::MOTIVES;
            for pair in [[0, 1], [2, 5], [3, 6], [4, 7]] {
                bars = bars.push(
                    row(pair.into_iter().map(|i| {
                        let value = f32::from_bits(person.values[i]);
                        button(
                            column![
                                text(MOTIVES[i].0)
                                    .font(motives::FONT)
                                    .size(12)
                                    .line_height(1.)
                                    .width(100)
                                    .align_x(iced::alignment::Horizontal::Center),
                                container(Element::new(motives::Meter(value)))
                                    .width(100)
                                    .align_x(iced::alignment::Horizontal::Center)
                            ]
                            .spacing(1),
                        )
                        .padding(0)
                        .width(100)
                        .style(|_, _| button::Style {
                            background: None,
                            text_color: Color::WHITE,
                            ..Default::default()
                        })
                        .on_press_maybe(ready.then_some(Message::MotiveDetails(i)))
                        .into()
                    }))
                    .spacing(0),
                );
            }
        }
        let mut gadget = widget::stack![self.hud_background(4911, 108., 100.)]
            .width(108)
            .height(100);
        if let Some(layer) = self
            .mood
            .and_then(|mood| hud::mood_gauge_layer(self.art.as_ref()?.static_art(), mood))
        {
            gadget = gadget.push(layer);
        }
        gadget = gadget.push(at(
            self.hud_control(
                4500,
                "Needs",
                Message::Needs,
                None,
                ready && needs.person().is_some(),
                needs.selected(),
            ),
            4.,
            33.,
        ));
        // Other native subpanels do not yet have hosted command owners.
        for (id, label, x, y) in [
            (4505, "Interests", 44., 2.),
            (4504, "Relationships", 76., 2.),
            (4501, "Personality", 44., 34.),
            (4503, "Job", 76., 34.),
            (4507, "Gifts", 44., 66.),
            (4502, "House", 76., 66.),
        ] {
            gadget = gadget.push(at(
                icon(
                    self.art.as_deref().map(|art| art.static_art()),
                    id,
                    label,
                    None,
                    false,
                ),
                x,
                y,
            ));
        }
        let content: Element<'_, Message> = if needs.shown() && needs.person().is_some() {
            container(column![
                container(text("Needs").font(motives::FONT).size(14)).padding([0, 9]),
                bars.width(200)
            ])
            .padding([3, 0])
            .width(280)
            .into()
        } else {
            // The original hides its content panel without a selected person.
            // Do not invent a selection or replace it with instructional copy.
            widget::Space::new().width(280).height(100).into()
        };
        row![gadget, content]
            .align_y(iced::Alignment::Center)
            .width(388)
            .height(100)
            .into()
    }
    fn catalog_hud(&self, ready: bool) -> Element<'_, Message> {
        let Some(catalog) = self.catalog.as_ref() else {
            return text("Opening Buy catalog…").into();
        };
        if catalog.view().is_none() {
            return text("Opening catalog…").into();
        }
        let identity = Arc::new(catalog.identity());
        self.catalog_hud_with_keys(ready, [identity.clone(), identity.clone(), identity])
    }
    fn catalog_hud_with_keys<K: Clone + PartialEq + 'static>(
        &self,
        ready: bool,
        keys: [K; 3],
    ) -> Element<'_, Message> {
        let Some(catalog) = self.catalog.as_ref() else {
            return text("Opening Buy catalog…").into();
        };
        // Native filtering, ordering, selection and page revision remain with
        // the existing catalog owner; Iced changes only how they are arranged.
        use sims_vm::interface::buy::input::Event;
        let Some(view) = catalog.view() else {
            return text("Opening catalog…").into();
        };
        let nav = |event| Message::BuyNavigate(catalog.revision(), event);
        let mut categories = column![
            text(if view.heading.is_empty() {
                "Buy mode"
            } else {
                &view.heading
            })
            .size(16)
        ]
        .spacing(4);
        if view.subcatalog != 0 {
            categories = categories.push(
                button(text("‹ Categories").size(12))
                    .on_press_maybe(ready.then_some(nav(Event::Back)))
                    .style(text_button_style),
            );
        }
        for (i, (label, state)) in view.labels.iter().zip(view.categories).enumerate() {
            if state.visible {
                categories = categories.push(
                    button(text(label).size(12))
                        .padding([3, 7])
                        .width(Fill)
                        .on_press_maybe(
                            (ready && state.enabled).then_some(nav(Event::Category(i as u8))),
                        )
                        .style(move |theme, status| {
                            let mut style = text_button_style(theme, status);
                            if state.selected {
                                style.text_color = CYAN;
                            }
                            style
                        }),
                );
            }
        }
        let mut cells = column![].spacing(6);
        if view.subcatalog == 0 {
            for group in [[0, 1, 2, 3], [4, 5, 6, 7]] {
                cells = cells.push(
                    row(group
                        .into_iter()
                        .filter(|&i| view.categories[i].visible)
                        .map(|i| {
                            column![
                                icon(
                                    self.art.as_deref().map(|art| art.static_art()),
                                    150 + i as i32,
                                    &view.labels[i],
                                    (ready && view.categories[i].enabled)
                                        .then_some(nav(Event::Category(i as u8))),
                                    false
                                ),
                                text(&view.labels[i]).size(12)
                            ]
                            .align_x(iced::Alignment::Center)
                            .spacing(6)
                            .width(Fill)
                            .into()
                        }))
                    .spacing(15),
                );
            }
        } else {
            for chunk in view.products.chunks(4) {
                cells = cells.push(
                    row(chunk
                        .iter()
                        .filter_map(|&i| catalog.product(i).map(|p| (i, p)))
                        .map(|(i, p)| {
                            let image = catalog.images.get(&p.definition);
                            let preview: Element<_> = match image {
                                Some(image) => {
                                    widget::image(image.clone()).width(63).height(67).into()
                                }
                                _ => container(text("Preview\nunavailable").size(11))
                                    .width(63)
                                    .height(67)
                                    .center_x(63)
                                    .center_y(67)
                                    .into(),
                            };
                            let cell = button(
                                row![
                                    preview,
                                    column![
                                        text(p.display_name()).size(12),
                                        text(format!("${}", p.fields.price)).size(12).color(CYAN)
                                    ]
                                    .spacing(5)
                                    .width(Fill)
                                ]
                                .spacing(5)
                                .align_y(iced::Alignment::Center),
                            )
                            .width(Fill)
                            .padding(5)
                            .on_press_maybe(
                                ready.then_some(Message::BuyProduct(catalog.revision(), i)),
                            )
                            .style(move |theme, status| {
                                let mut style = text_button_style(theme, status);
                                if catalog.selected(i) {
                                    style.border.color = CYAN;
                                    style.border.width = 2.;
                                }
                                style
                            });
                            widget::tooltip(
                                cell,
                                container(text(p.display_description()).size(12))
                                    .padding(10)
                                    .max_width(340)
                                    .style(panel_style),
                                widget::tooltip::Position::Top,
                            )
                            .into()
                        }))
                    .spacing(6),
                );
            }
        }
        let navigation = row![
            button("‹")
                .on_press_maybe((ready && view.previous).then_some(nav(Event::Previous)))
                .style(text_button_style),
            text(format!("Page {}", view.page + 1)).size(12),
            button("›")
                .on_press_maybe((ready && view.next).then_some(nav(Event::Next)))
                .style(text_button_style),
            text(if catalog.insufficient_funds() {
                "Not enough household funds."
            } else {
                "Click to place · drag to rotate · Esc cancels"
            })
            .size(12),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center);
        let funds = self.funds.clone();
        let mut content = column![text(funds).size(16).color(CYAN)]
            .spacing(6)
            .width(Fill);
        if let Some(selected) = self.furniture.as_ref() {
            content = content.push(
                row![
                    text(format!("{} · ${}", selected.name, selected.price)).size(13),
                    button("Sell")
                        .on_press_maybe(ready.then(|| self.sell_message()).flatten())
                        .style(text_button_style),
                    button("Cancel move")
                        .on_press_maybe(ready.then_some(Message::DialogKey(27)))
                        .style(text_button_style),
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            );
        }
        let [category_key, product_key, navigation_key] = keys;
        row![
            scrollable(widget_lifetimes::retain_owner_key(
                category_key,
                categories.into()
            ))
            .width(155)
            .height(218),
            content
                .push(widget_lifetimes::retain_owner_key(
                    product_key,
                    cells.into()
                ))
                .push(widget_lifetimes::retain_owner_key(
                    navigation_key,
                    navigation.into()
                ))
        ]
        .spacing(14)
        .width(Fill)
        .into()
    }
    #[cfg(test)]
    pub(crate) fn construction_costs_for_test(
        &self,
        batch: usize,
        reverse: bool,
    ) -> Vec<(&'static str, f64)> {
        fn measure<T>(batch: usize, mut construct: impl FnMut() -> T) -> f64 {
            let started = std::time::Instant::now();
            for _ in 0..batch {
                std::hint::black_box(construct());
            }
            started.elapsed().as_secs_f64() * 1e6 / batch as f64
        }
        let ready = self
            .cluster
            .as_ref()
            .is_some_and(|cluster| cluster.values.ready);
        let mut names = vec!["household_hud", "cluster"];
        if self
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.view().is_some())
        {
            names.extend([
                "catalog_hud_unshared",
                "catalog_hud_shared",
                "catalog_keys_unshared",
                "catalog_keys_shared",
            ]);
        }
        if reverse {
            names.reverse();
        }
        names
            .into_iter()
            .map(|name| {
                let us = match name {
                    "household_hud" => measure(batch, || self.household_hud()),
                    "cluster" => measure(batch, || {
                        self.cluster
                            .as_deref()
                            .map(|state| cluster::view(state, self.options_open))
                    }),
                    "catalog_hud_unshared" => measure(batch, || {
                        let catalog = self.catalog.as_ref().unwrap();
                        self.catalog_hud_with_keys(
                            ready,
                            [catalog.identity(), catalog.identity(), catalog.identity()],
                        )
                    }),
                    "catalog_hud_shared" => measure(batch, || self.catalog_hud(ready)),
                    "catalog_keys_unshared" => measure(batch, || {
                        let catalog = self.catalog.as_ref().unwrap();
                        [catalog.identity(), catalog.identity(), catalog.identity()]
                    }),
                    "catalog_keys_shared" => measure(batch, || {
                        let identity = Arc::new(self.catalog.as_ref().unwrap().identity());
                        [identity.clone(), identity.clone(), identity]
                    }),
                    _ => unreachable!(),
                };
                (name, us)
            })
            .collect()
    }
    pub(super) fn interaction_popup(&self) -> Option<Element<'_, Message>> {
        if !self.session_present || !self.people_visible {
            return None;
        }
        let menu = self.menu.as_ref().filter(|m| m.popup_visible)?;
        let mut actions = column![text(&menu.title).size(17)].spacing(8);
        for choice in &menu.popup {
            actions = actions.push(
                button(text(&choice.label).size(15))
                    .width(Fill)
                    .style(text_button_style)
                    .on_press_maybe(choice.intent.as_ref().map(super::Intent::message)),
            );
        }
        if let Some(cancel) = &menu.cancel {
            actions = actions.push(
                button("Cancel")
                    .on_press(cancel.message())
                    .style(text_button_style),
            );
        }
        // Only this native popup's buttons end with its owner. The surrounding
        // HUD and application controls keep their own held input lifetimes.
        Some(widget_lifetimes::retain_owner(
            self.native_popup_epoch,
            container(scrollable(actions))
                .padding(14)
                .max_width(300)
                .max_height(330)
                .style(panel_style)
                .into(),
        ))
    }
    fn queue_cancel_message(&self, entry: &QueueEntry) -> Option<Message> {
        (self.session_present
            && self.people_visible
            && self.resident == entry.action.resident
            && !self.pending
            && !self.modal
            && self.error.is_none())
        .then(|| entry.action.cancel())
        .flatten()
        .map(|intent| intent.message())
    }
    fn queue_cancel_control<'a>(
        &'a self,
        entry: &'a QueueEntry,
        control: widget::Button<'a, Message>,
    ) -> Element<'a, Message> {
        let message = self.queue_cancel_message(entry);
        // Repaint age and labels do not end a valid held press. Replacing a
        // resident/action, changing its cancellation state or blocking input
        // does: an old pressed button must never cancel its new occupant.
        widget_lifetimes::retain_owner_key(
            (entry.action, message.is_some()),
            control.on_press_maybe(message).into(),
        )
    }
    fn queue_cancel_tooltip(&self, entry: &QueueEntry) -> String {
        let hint = if entry.action.cancel_requested {
            "Cancellation requested; waiting for the engine to remove the action."
        } else if self.queue_cancel_message(entry).is_some() {
            "Click to request cancellation."
        } else {
            "Cancellation is currently unavailable."
        };
        format!(
            "{} · {} · {} — {}",
            entry.label, entry.target, entry.action.status(), hint
        )
    }
    pub(super) fn queue_diagnostics(&self) -> Element<'_, Message> {
        let mut queue = column![].spacing(8);
        if self.people_visible {
            for entry in &self.queue {
                queue = queue.push(
                    row![
                        text(format!(
                            "{} · {} · {}",
                            entry.diagnostic_label, entry.target, entry.action.status()
                        ))
                        .size(13)
                        .width(Fill),
                        widget::tooltip(
                            self.queue_cancel_control(
                                entry,
                                button(if entry.action.cancel_requested {
                                    "Cancelling…"
                                } else {
                                    "Cancel"
                                }),
                            ),
                            text(self.queue_cancel_tooltip(entry)),
                            widget::tooltip::Position::Bottom,
                        ),
                    ]
                    .spacing(8),
                );
            }
        }
        queue.into()
    }
    pub(super) fn queue_hud(&self) -> Element<'_, Message> {
        let mut queue = row![].spacing(6);
        if self.people_visible {
            for action in &self.queue {
                let content: Element<'_, Message> = if let Some(image) = &action.image {
                    widget::image(image.clone())
                        .width(45)
                        .height(45)
                        .content_fit(iced::ContentFit::Contain)
                        .into()
                } else {
                    container(text("…").size(20)).center(45).into()
                };
                queue = queue.push(widget::tooltip(
                    self.queue_cancel_control(
                        action,
                        button(
                            column![
                                content,
                                text(if action.action.cancel_requested {
                                    "Cancelling…"
                                } else {
                                    "Cancel"
                                })
                                .size(11),
                            ]
                            .align_x(iced::Alignment::Center),
                        )
                        .padding(2)
                        .style(text_button_style),
                    ),
                    text(self.queue_cancel_tooltip(action)),
                    widget::tooltip::Position::Bottom,
                ));
            }
        }
        scrollable(queue)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .height(86)
            .into()
    }
}

fn buy_catalog_view(catalog: &Catalog, ready: bool) -> Element<'_, Message> {
    use sims_vm::interface::buy::input::Event;
    let message = |event| Message::BuyNavigate(catalog.revision(), event);
    let mut view = column![].spacing(8);
    if let Some(v) = catalog.view() {
        if !v.heading.is_empty() {
            view = view.push(text(&v.heading).size(18));
        }
        if catalog.insufficient_funds() {
            view = view.push(text("Not enough household funds.").size(12));
        }
        view = view.push(
            text("Click to place furniture. Drag to rotate; Shift repeats; Esc cancels.").size(12),
        );
        if v.subcatalog != 0 {
            view = view
                .push(button("Categories").on_press_maybe(ready.then_some(message(Event::Back))));
        }
        for (i, (label, state)) in v.labels.iter().zip(v.categories).enumerate() {
            if state.visible {
                view = view.push(
                    button(label.as_str())
                        .on_press_maybe(
                            (ready && state.enabled).then_some(message(Event::Category(i as u8))),
                        )
                        .style(if state.selected {
                            button::primary
                        } else {
                            button::secondary
                        }),
                );
            }
        }
        for &index in &v.products {
            if let Some(product) = catalog.product(index) {
                view = view.push(
                    button(column![
                        text(product.display_name()),
                        text(format!("${}", product.fields.price)).size(12)
                    ])
                    .padding(8)
                    .width(Fill)
                    .style(if catalog.selected(index) {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press_maybe(
                        ready.then_some(Message::BuyProduct(catalog.revision(), index)),
                    ),
                );
            }
        }
        if v.subcatalog != 0 {
            view = view.push(text(format!("Page {}", v.page.wrapping_add(1))).size(12));
        }
        if v.previous || v.next {
            view = view.push(
                row![
                    button("Previous")
                        .on_press_maybe((ready && v.previous).then_some(message(Event::Previous))),
                    button("Next")
                        .on_press_maybe((ready && v.next).then_some(message(Event::Next))),
                ]
                .spacing(8),
            );
        }
    }
    widget_lifetimes::retain_owner_key(catalog.identity(), view.into())
}

/// An application action in its own row, outside the fixed original artwork.
pub(crate) fn copy_action(enabled: bool) -> Element<'static, Message> {
    column![button(text("Save a Copy…").size(13))
        .on_press_maybe(enabled.then_some(Message::ChooseSaveCopy))
        .padding(8),
        text(player_shortcuts::file_guidance()).size(12),
    ].into()
}
