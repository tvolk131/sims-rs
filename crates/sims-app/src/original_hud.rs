//! Original installed artwork at its native logical size. This module only
//! reads published state and emits the same commands as the diagnostic UI.
use super::*;
use iced::advanced::{Widget, layout, renderer, widget::Tree};
use iced::widget::column;
use sims_vm::interface::view_control::Widget as Control;
#[path = "hud_cluster.rs"]
pub(super) mod cluster;
#[path = "original_motives.rs"]
pub(super) mod motives;
#[path = "tutorial_highlight.rs"]
pub(super) mod tutorial_highlight;

const BLUE: Color = Color::from_rgb8(5, 5, 68);
const CYAN: Color = Color::from_rgb8(0, 235, 235);

fn at<'a>(element: Element<'a, Message>, x: f32, y: f32) -> Element<'a, Message> {
    widget::pin(element).x(x).y(y).into()
}

// 53a0b6 draws one keyed image while the native panel is expanded. Its
// coordinates are relative to the universal cluster, not the content strip.
fn mode_trim(mode: i32, expanded: bool) -> Option<(i32, [f32; 2])> {
    if !expanded {
        return None;
    }
    match mode {
        0 => Some((4805, [113., 51.])),
        1 => Some((4804, [64., 35.])),
        2 => Some((4803, [7., 30.])),
        3 => Some((4806, [183., 83.])),
        _ => None,
    }
}

fn trim_layer(
    icons: &dialog_view::StaticArt,
    mode: i32,
    expanded: bool,
) -> Option<Element<'static, Message>> {
    let (id, [x, y]) = mode_trim(mode, expanded)?;
    let (width, height, frames) = icons.hud(id)?;
    Some(at(
        widget::image(frames[0].clone())
            .width(*width as f32)
            .height(*height as f32)
            .into(),
        x,
        y,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MoodStrip {
    resource: i32,
    source: [u32; 4],
    position: [i32; 2],
}

// 51b366 rounds the actual overall mood twice using x87, then clamps the
// segment count. Keep the recovered f32 scale, not a decimal approximation.
fn mood_segments(mood: f32) -> Option<i32> {
    let mood = f64::from(mood).round();
    if !mood.is_finite() || mood < f64::from(i32::MIN) || mood > f64::from(i32::MAX) {
        return None;
    }
    Some(
        ((mood + 100.) * f64::from(f32::from_bits(0x3d60_28c2)) - 5.)
            .round()
            .clamp(-5., 5.) as i32,
    )
}

fn mood_gauge_strips(mood: f32) -> Option<Vec<MoodStrip>> {
    let n = mood_segments(mood)?;
    // Native gadget origin193,50; green205,53 and red205,123. Positive
    // segments reveal the green strip upwards; negative reveal red downwards.
    Some(if n > 0 {
        let top = 25 - n as u32 * 5;
        vec![MoodStrip {
            resource: 4506,
            source: [0, top, 27, 25],
            position: [12, 3 + top as i32],
        }]
    } else if n < 0 {
        vec![MoodStrip {
            resource: 4510,
            source: [0, 0, 27, (-n) as u32 * 5],
            position: [12, 73],
        }]
    } else {
        vec![
            MoodStrip {
                resource: 4506,
                source: [0, 23, 27, 25],
                position: [12, 26],
            },
            MoodStrip {
                resource: 4510,
                source: [0, 0, 27, 2],
                position: [12, 73],
            },
        ]
    })
}

/// 4e986d -> 4e9753 -> 4ea6a6 resolves GUID/cache identity back through the
/// live registry. Duplicate GUIDs use the first live instance, even if the
/// selected flag is on a later one. Missing cache/live matches use zero mood.
fn gauge_mood(world: &sims_vm::World, selected: Option<i16>) -> Result<f32> {
    let Some(selected) = selected else {
        return Ok(0.);
    };
    let guid = world
        .objects
        .get(&selected)
        .and_then(|o| o.definition.as_ref())
        .context("gauge selected definition absent")?
        .guid;
    if guid == 0 || !world.image_cache.entries.iter().any(|e| e.guid == guid) {
        return Ok(0.);
    }
    for id in &world
        .person
        .as_ref()
        .context("gauge registry absent")?
        .person_order
    {
        let object = world.objects.get(id).context("gauge live person absent")?;
        if object
            .definition
            .as_ref()
            .context("gauge live definition absent")?
            .guid
            == guid
        {
            let mood = f32::from_bits(object.motives.context("gauge motive storage absent")?[3]);
            ensure!(mood.is_finite(), "non-finite gauge mood");
            return Ok(mood);
        }
    }
    Ok(0.)
}

pub(super) fn live_gauge_mood(session: &Session) -> Result<f32> {
    let sim = &session.simulation;
    if sim.vm.world.person.is_none() {
        return Ok(0.);
    }
    gauge_mood(
        &sim.vm.world,
        sim.native_selected_person().map_err(anyhow::Error::msg)?,
    )
}

pub(super) fn mood_gauge_layer(
    icons: &dialog_view::StaticArt,
    mood: f32,
) -> Option<Element<'static, Message>> {
    let mut layer = widget::stack![widget::Space::new().width(108).height(100)]
        .width(108)
        .height(100);
    for strip in mood_gauge_strips(mood)? {
        let height = strip.source[3] - strip.source[1];
        layer = layer.push(at(
            widget::image(icons.mood_strip(strip.resource, height)?)
                .width(27)
                .height(height as f32)
                .into(),
            strip.position[0] as f32,
            strip.position[1] as f32,
        ));
    }
    Some(layer.into())
}

pub(super) struct Thumbnails {
    catalog: sims_content::object_catalog::Catalog<'static>,
    images: std::collections::BTreeMap<u32, Result<widget::image::Handle, String>>,
    live: std::collections::BTreeMap<(String, u16), Result<u32, String>>,
}
impl Thumbnails {
    pub(super) fn image(&self, definition: u32) -> Option<widget::image::Handle> {
        self.images.get(&definition).and_then(|v| v.as_ref().ok()).cloned()
    }
    fn new(s: &Session) -> Result<Self> {
        Ok(Self {
            catalog: sims_content::object_catalog::Catalog::household(
                Arc::new(s.index().clone()),
                &s.house.neighborhood_source,
                Default::default(),
            )?,
            images: Default::default(),
            live: Default::default(),
        })
    }
    fn prepare(&mut self, product: &sims_content::object_catalog::products::Product) {
        if self.images.contains_key(&product.definition) {
            return;
        }
        let image = (|| -> Result<_> {
            ensure!(
                self.catalog
                    .portrait_definition(product.definition)
                    .map_err(anyhow::Error::msg)?
                    == product.source,
                "thumbnail definition differs"
            );
            let (raster, _) = sims_render::object_preview::software::dialog(
                &mut self.catalog,
                product.definition,
                sims_render::portrait::resize::Format::Rgb565,
                0xf81f,
            )?;
            let rgba = raster.rgba()?;
            Ok(widget::image::Handle::from_rgba(
                rgba.width(),
                rgba.height(),
                rgba.into_raw(),
            ))
        })();
        self.images
            .insert(product.definition, image.map_err(|e| format!("{e:#}")));
    }
    fn prepare_live(&mut self, definition: &sims_vm::Definition) {
        let key = (definition.source.clone(), definition.object);
        if self.live.contains_key(&key) {
            return;
        }
        let handle = match self.catalog.live_dialog_resource(definition) {
            Ok(handle) => handle,
            Err(error) => {
                self.live.insert(key, Err(error));
                return;
            }
        };
        if !self.images.contains_key(&handle) {
            let image = (|| -> Result<_> {
                let (raster, _) = sims_render::object_preview::software::dialog(
                    &mut self.catalog,
                    handle,
                    sims_render::portrait::resize::Format::Rgb565,
                    0xf81f,
                )?;
                let rgba = raster.rgba()?;
                Ok(widget::image::Handle::from_rgba(
                    rgba.width(),
                    rgba.height(),
                    rgba.into_raw(),
                ))
            })();
            self.images
                .insert(handle, image.map_err(|e| format!("{e:#}")));
        }
        self.live.insert(key, Ok(handle));
    }
    pub(super) fn live_image(&self, d: &sims_vm::Definition) -> Option<widget::image::Handle> {
        self.live
            .get(&(d.source.clone(), d.object))
            .and_then(|h| h.as_ref().ok())
            .and_then(|h| self.images.get(h))
            .and_then(|i| i.as_ref().ok())
            .cloned()
    }
}

struct Sprite {
    frames: Vec<widget::image::Handle>,
    size: Size,
    enabled: bool,
    selected: bool,
}
impl Widget<Message, iced::Theme, iced::Renderer> for Sprite {
    fn size(&self) -> Size<iced::Length> {
        Size::new(self.size.width.into(), self.size.height.into())
    }
    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(self.size.width, self.size.height, self.size))
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        let cell = if !self.enabled {
            3
        } else if self.selected {
            1
        } else if cursor.is_over(layout.bounds()) {
            2
        } else {
            0
        };
        let content: Element<'_, Message> = widget::image(self.frames[cell].clone())
            .width(self.size.width)
            .height(self.size.height)
            .into();
        content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
}

pub(super) fn icon<'a>(
    icons: Option<&dialog_view::StaticArt>,
    id: i32,
    label: &'a str,
    message: Option<Message>,
    selected: bool,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let content: Element<'a, Message> = if let Some((w, h, frames)) = icons.and_then(|i| i.hud(id))
    {
        Element::new(Sprite {
            frames: frames.clone(),
            size: Size::new(*w as f32, *h as f32),
            enabled,
            selected,
        })
    } else {
        text(label).size(13).into()
    };
    let b = button(content)
        .padding(0)
        .on_press_maybe(message)
        .style(|_, _| button::Style {
            background: None,
            text_color: Color::WHITE,
            ..Default::default()
        });
    widget::tooltip(
        b,
        container(text(label).size(13))
            .padding(6)
            .style(panel_style),
        widget::tooltip::Position::Top,
    )
    .into()
}
pub(super) fn panel_style(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(BLUE.into()),
        text_color: Some(Color::WHITE),
        border: iced::Border {
            color: Color::from_rgb8(98, 104, 173),
            width: 1.,
            radius: 5.into(),
        },
        ..Default::default()
    }
}
pub(super) fn text_button_style(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Color::from_rgb8(40, 49, 118)
            } else {
                BLUE
            }
            .into(),
        ),
        text_color: if status == button::Status::Disabled {
            Color::from_rgb8(110, 115, 153)
        } else {
            Color::WHITE
        },
        border: iced::Border {
            color: Color::from_rgb8(87, 108, 180),
            width: 1.,
            radius: 8.into(),
        },
        ..Default::default()
    }
}

impl Viewer {
    pub(super) fn hud_height(&self) -> u32 {
        if self
            .device
            .as_ref()
            .is_none_or(|d| d.panel_state() == Some(1))
        {
            return 0;
        }
        let mode = self
            .session
            .as_ref()
            .and_then(|s| s.host.presentation.transport.as_ref())
            .map_or(2, |t| t.controls.mode);
        if mode == 1 { 150 } else { 100 }
    }
    /// Separate presentation resource owner: thumbnail loading cannot allocate
    /// game objects or disturb the dialog client's retained resource handles.
    pub(super) fn prepare_catalog_thumbnails(&mut self) -> Result<()> {
        let catalog = self
            .device
            .as_ref()
            .and_then(|d| d.panel_widgets())
            .and_then(|w| w.catalog());
        let products: Vec<_> = catalog
            .into_iter()
            .flat_map(|c| {
                c.view()
                    .into_iter()
                    .flat_map(move |v| v.products.iter().filter_map(move |&i| c.product(i)))
            })
            .filter(|p| {
                self.thumbnails
                    .as_ref()
                    .is_none_or(|t| !t.images.contains_key(&p.definition))
            })
            .cloned()
            .collect();
        let definitions: Vec<_> = self
            .session
            .as_ref()
            .into_iter()
            .flat_map(|s| {
                s.simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|a| a.actors.get(&self.resident))
                    .into_iter()
                    .flat_map(|a| a.active.iter().chain(a.queue.pending.iter()))
                    .filter_map(|a| s.simulation.vm.world.objects.get(&a.icon_owner))
                    .filter(|o| o.person.is_none())
                    .filter_map(|o| o.definition.clone())
            })
            .collect();
        if products.is_empty() && definitions.is_empty() {
            return Ok(());
        }
        if self.thumbnails.is_none() {
            self.thumbnails = Some(Thumbnails::new(
                self.session.as_ref().context("thumbnail session absent")?,
            )?);
        }
        //These independent presentation images do not gate catalog commands.
        //Spread cold portrait decoding across UI frames; pending cells already
        //have their labels, prices and clickable product identity.
        for product in products.into_iter().take(2) {
            self.thumbnails.as_mut().unwrap().prepare(&product);
        }
        for d in definitions {
            self.thumbnails.as_mut().unwrap().prepare_live(&d);
        }
        Ok(())
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
        let published =
            kind.and_then(|k| self.device.as_ref()?.panel_widgets().map(|w| w.button(k)));
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
                    4500 if self
                        .device
                        .as_ref()
                        .is_none_or(|d| d.panel_state() != Some(1))
                        && self.client.as_ref().and_then(|c| c.live_panel()).is_some() =>
                    {
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
        let Some(state) = self.hud_cluster.as_deref() else {
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
        let mut menu = widget::stack![self.hud_background(4910, 0., 100.)]
            .width(Fill)
            .height(100);
        // Native5160f0 separators and captured original icon rectangles.
        // The five unhosted actions remain disabled rather than dispatching
        // unrelated application commands through their original artwork.
        for (id, label, message, x, y) in [
            (4950, "Save game", Some(Message::Save), 29., 6.),
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
            ("Load", Message::Load, 229., 25.),
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
            super::neighborhood_usability::controls(
                &self.application,
                self.application_confirmation,
                self.session.is_some(),
            ),
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
    fn residents_hud(&self) -> Element<'_, Message> {
        let s = self.session.as_ref().unwrap();
        let mut people = row![].spacing(0);
        if self.people_panel_visible()
            && let Some(h) = &s.house.household
        {
            for resident in &h.residents {
                let id = resident.object as i16;
                let face: Element<'_, Message> = self
                    .art
                    .as_ref()
                    .and_then(|i| i.resident(resident.neighbor))
                    .map(|im| widget::image(im).width(45).height(45).into())
                    .unwrap_or_else(|| text(&resident.name).size(12).into());
                let mut item: Element<_> = button(face)
                    .padding(0)
                    .on_press_maybe(
                        (live_resident(s, id).is_some() && s.simulation.has_interaction_actor(id))
                            .then_some(Message::Resident(id)),
                    )
                    .style(move |_, status| {
                        let mut style = text_button_style(&iced::Theme::Dark, status);
                        style.border.color = if id == self.resident {
                            CYAN
                        } else {
                            Color::from_rgb8(87, 108, 180)
                        };
                        style.border.width = 2.;
                        style
                    })
                    .into();
                if self
                    .client
                    .as_ref()
                    .and_then(|c| c.buttons())
                    .is_some_and(|b| b.highlighted(resident.neighbor))
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
        let Some(needs) = self.client.as_ref().and_then(|c| c.live_panel()) else {
            return widget::Space::new().into();
        };
        if self
            .device
            .as_ref()
            .is_some_and(|d| d.panel_state() == Some(1))
        {
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
        if let Some(layer) = self.session.as_ref().and_then(|s| {
            mood_gauge_layer(self.art.as_ref()?.static_art(), live_gauge_mood(s).ok()?)
        }) {
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
        let Some(catalog) = self
            .device
            .as_ref()
            .and_then(|d| d.panel_widgets())
            .and_then(|w| w.catalog())
        else {
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
                            let image = self
                                .thumbnails
                                .as_ref()
                                .and_then(|t| t.images.get(&p.definition));
                            let preview: Element<_> = match image {
                                Some(Ok(image)) => {
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
        let funds = household_funds_label(
            self.session.as_ref().unwrap(),
            self.device
                .as_ref()
                .and_then(|d| d.panel_widgets())
                .and_then(|w| w.texts())
                .map(|t| t.funds())
                .unwrap_or_default(),
        )
        .unwrap_or_default();
        let mut content = column![text(funds).size(16).color(CYAN)]
            .spacing(6)
            .width(Fill);
        if let Some(selected) = self.device.as_ref().and_then(|d| d.furniture_selection()) {
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
        row![
            scrollable(categories).width(155).height(218),
            content.push(cells).push(navigation)
        ]
        .spacing(14)
        .width(Fill)
        .into()
    }
    pub(super) fn interaction_popup(&self) -> Option<Element<'_, Message>> {
        let s = self.session.as_ref()?;
        if !self.people_panel_visible() {
            return None;
        }
        let (title, entries, cancel): (String, Vec<(String, Message)>, Option<Message>) =
            if let Some(input) = &s.host.presentation.ground_input {
                if !input.ready() && self.error.is_none() {
                    return None;
                }
                (
                    "Go here".into(),
                    input
                        .actions()
                        .iter()
                        .enumerate()
                        .map(|(i, a)| (self.action_label(a), Message::GroundChoice(i)))
                        .collect(),
                    input.can_cancel().then_some(Message::GroundCancel),
                )
            } else if let Some(input) = &s.host.presentation.object_input {
                if !input.ready() && self.error.is_none() {
                    return None;
                }
                (
                    self.label(input.target()),
                    input
                        .actions()
                        .iter()
                        .enumerate()
                        .map(|(i, a)| (self.action_label(a), Message::ObjectChoice(i)))
                        .collect(),
                    input.can_cancel().then_some(Message::ObjectCancel),
                )
            } else if let Some(target) = self.selected {
                (
                    self.label(target),
                    self.choices
                        .iter()
                        .filter(|c| c.available)
                        .map(|c| (c.label.clone(), Message::Queue(target, c.index)))
                        .collect(),
                    Some(Message::Select(None)),
                )
            } else {
                return None;
            };
        let mut actions = column![text(title).size(17)].spacing(8);
        for (label, message) in entries {
            actions = actions.push(
                button(text(label).size(15))
                    .width(Fill)
                    .style(text_button_style)
                    .on_press(message),
            );
        }
        if let Some(cancel) = cancel {
            actions = actions.push(button("Cancel").on_press(cancel).style(text_button_style));
        }
        Some(
            container(scrollable(actions))
                .padding(14)
                .max_width(300)
                .max_height(330)
                .style(panel_style)
                .into(),
        )
    }
    pub(super) fn action_label(&self, action: &sims_vm::action::Action) -> String {
        let s = self.session.as_ref().unwrap();
        action
            .label
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                s.simulation
                    .vm
                    .action_menu(&s.program, action.target)
                    .ok()
                    .and_then(|entries| {
                        entries
                            .iter()
                            .find(|e| e.id == action.menu_id)
                            .map(|e| e.label.clone())
                    })
            })
            .unwrap_or_else(|| "Interaction".into())
    }
    pub(super) fn queue_hud(&self) -> Element<'_, Message> {
        let mut queue = row![].spacing(6);
        if self.people_panel_visible()
            && let Some(s) = &self.session
            && live_resident(s, self.resident).is_some()
            && let Some(actor) = s
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|a| a.actors.get(&self.resident))
        {
            for action in actor.active.iter().chain(actor.queue.pending.iter()) {
                let label = self.action_label(action);
                let owner = s.simulation.vm.world.objects.get(&action.icon_owner);
                let image = owner
                    .and_then(|o| o.definition.as_ref())
                    .and_then(|d| self.thumbnails.as_ref()?.live_image(d))
                    .or_else(|| {
                        s.house
                            .household
                            .as_ref()?
                            .residents
                            .iter()
                            .find(|r| r.object as i16 == action.icon_owner)
                            .and_then(|r| self.art.as_ref()?.resident(r.neighbor as i16))
                    });
                let content: Element<'_, Message> = if let Some(image) = image {
                    widget::image(image)
                        .width(45)
                        .height(45)
                        .content_fit(iced::ContentFit::Contain)
                        .into()
                } else {
                    container(text("…").size(20)).center(45).into()
                };
                queue = queue.push(widget::tooltip(
                    button(content)
                        .padding(2)
                        .on_press(Message::Cancel(action.id.into()))
                        .style(text_button_style),
                    text(format!(
                        "{label} · {} — click to cancel",
                        self.label(action.target)
                    )),
                    widget::tooltip::Position::Bottom,
                ));
            }
        }
        scrollable(queue)
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .height(68)
            .into()
    }
}

#[cfg(test)]
#[path = "original_hud_trim_tests.rs"]
mod trim_tests;

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "original_hud_mood_tests.rs"]
mod mood_tests;

#[cfg(test)]
#[path = "original_hud_followup_tests.rs"]
mod followup_tests;

#[cfg(test)]
#[cfg(feature = "private-fixtures")]
#[path = "tutorial_presentation_tests.rs"]
#[rustfmt::skip]
mod tutorial_presentation_tests;
