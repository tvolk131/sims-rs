//! Stateful Iced event/render driver for opt-in offscreen integration tests.
//! Keep the widget Tree between events: rebuilding it loses button press state.
use super::*;
use iced::Theme;
use iced::advanced::{
    Layout, Shell, layout,
    renderer::{Headless, Renderer as _},
    widget::Tree,
};

#[derive(Debug, PartialEq, serde::Serialize)]
pub(super) struct ScrollOwner {
    pub bounds: [f32; 4],
    pub content: [f32; 4],
    pub translation: [f32; 2],
}
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub(super) struct WidgetOwners {
    pub scrolls: Vec<ScrollOwner>,
    pub focused: Vec<([f32; 4], bool)>,
}
impl iced::advanced::widget::Operation for WidgetOwners {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        content: iced::Rectangle,
        translation: iced::Vector,
        _: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        self.scrolls.push(ScrollOwner {
            bounds: [bounds.x, bounds.y, bounds.width, bounds.height],
            content: [content.x, content.y, content.width, content.height],
            translation: [translation.x, translation.y],
        });
    }
    fn focusable(
        &mut self,
        _: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        self.focused.push((
            [bounds.x, bounds.y, bounds.width, bounds.height],
            state.is_focused(),
        ));
    }
}

pub(super) struct Input {
    renderer: iced::Renderer,
    tree: Option<Tree>,
    cursor: iced::mouse::Cursor,
    window: window::Id,
    batch_cache: Option<iced_runtime::user_interface::Cache>,
}

pub(super) struct Batch {
    pub messages: Vec<Message>,
    pub statuses: Vec<iced::event::Status>,
    pub owners: WidgetOwners,
}

pub(super) struct ElementText {
    pub text: String,
    pub bounds: iced::Rectangle,
    pub container: Option<iced::Rectangle>,
}

pub(super) struct ElementObservation {
    pub layouts: Vec<iced::Rectangle>,
    pub texts: Vec<ElementText>,
    pub interaction: iced::mouse::Interaction,
    pub messages: Vec<Message>,
}

struct ObservedElement<'a> {
    content: Element<'a, Message>,
    layouts: &'a std::cell::RefCell<Vec<iced::Rectangle>>,
}

impl iced::advanced::Widget<Message, Theme, iced::Renderer> for ObservedElement<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        self.content.as_widget().state()
    }
    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }
    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }
    fn size(&self) -> Size<iced::Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<iced::Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        fn collect(layout: Layout<'_>, bounds: &mut Vec<iced::Rectangle>) {
            bounds.push(layout.bounds());
            for child in layout.children() {
                collect(child, bounds);
            }
        }
        let node = self.content.as_widget_mut().layout(tree, renderer, limits);
        let mut bounds = self.layouts.borrow_mut();
        bounds.clear();
        collect(Layout::new(&node), &mut bounds);
        node
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &iced::advanced::renderer::Style,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &iced::Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}
/// Test-driver diagnostics only. No machine-dependent pass/fail thresholds.
/// Headless screenshot exposes GPU execution/readback as one opaque operation.
#[derive(Debug, Default, serde::Serialize)]
pub(super) struct Costs {
    pub view_us: u128,
    pub build_layout_us: u128,
    pub widget_update_us: u128,
    pub owners_cache_subscription_us: u128,
    pub draw_us: u128,
    pub gpu_readback_us: u128,
}
/// Match physical window-size rounding while keeping widget layout and cursor
/// coordinates logical. This is a test-driver boundary, not a native window.
pub(super) fn physical_size(logical: [u32; 2], scale: f32) -> Result<Size<u32>> {
    ensure!(scale.is_finite() && scale > 0., "invalid display scale");
    let mut physical = [0; 2];
    for (axis, value) in logical.into_iter().enumerate() {
        ensure!(value > 0, "empty logical display size");
        let scaled = (f64::from(value) * f64::from(scale)).round();
        ensure!(
            scaled >= 1. && scaled <= f64::from(u32::MAX),
            "physical display size out of domain"
        );
        physical[axis] = scaled as u32;
    }
    Ok(Size::new(physical[0], physical[1]))
}
impl Input {
    pub(super) fn new() -> Result<Self> {
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.),
            Some("wgpu"),
        ))
        .context("headless Iced GPU adapter absent")?;
        Ok(Self {
            renderer,
            tree: None,
            cursor: iced::mouse::Cursor::Unavailable,
            window: window::Id::unique(),
            batch_cache: None,
        })
    }
    /// Start an independent generated campaign without constructing another GPU.
    /// Only test-driver widget/cursor caches are reset; no application state is touched.
    pub(super) fn reset(&mut self) {
        self.tree = None;
        self.cursor = iced::mouse::Cursor::Unavailable;
        self.batch_cache = None;
    }
    /// Restore probe-only cursor movement through the real widgets, without
    /// inventing any private MouseArea state or touching application input.
    pub(super) fn cursor_event(&self) -> iced::Event {
        match self.cursor.position() {
            Some(position) => iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }),
            None => iced::Event::Mouse(iced::mouse::Event::CursorLeft),
        }
    }
    pub(super) fn event(&mut self, viewer: &Viewer, event: iced::Event) -> Vec<Message> {
        self.widget_event(viewer, event).0
    }
    /// Dispatch widgets first, then the exact production subscription mapping.
    /// Widget-only helpers above remain useful when discovering controls without
    /// dispatching any application messages.
    pub(super) fn application_event(
        &mut self,
        viewer: &Viewer,
        event: iced::Event,
    ) -> Vec<Message> {
        let (mut messages, status) = self.widget_event(viewer, event.clone());
        if let Some(message) = subscription_event(event, status, self.window) {
            messages.push(message);
        }
        messages
    }
    /// The actual pinned runtime's batched update, retaining its own Cache.
    /// Winit supplies the final cursor state for the whole batch; widgets
    /// process all events before subscription outputs or Viewer updates.
    pub(super) fn application_batch(&mut self, viewer: &Viewer, events: &[iced::Event]) -> Batch {
        self.profiled_batch(viewer, events).0
    }
    pub(super) fn profiled_batch(
        &mut self,
        viewer: &Viewer,
        events: &[iced::Event],
    ) -> (Batch, Costs) {
        let start = Instant::now();
        let element = viewer.view();
        let view_us = start.elapsed().as_micros();
        let (batch, mut costs) = self.profiled_element_batch(element, viewer.display_size, events);
        costs.view_us = view_us;
        (batch, costs)
    }
    /// The same pinned Iced runtime driven by a detached immutable frontend.
    pub(super) fn element_batch(
        &mut self,
        element: Element<'_, Message>,
        display_size: [u32; 2],
        events: &[iced::Event],
    ) -> Batch {
        self.profiled_element_batch(element, display_size, events).0
    }
    pub(super) fn element_observation(
        &mut self,
        element: Element<'_, Message>,
        display_size: [u32; 2],
    ) -> Result<ElementObservation> {
        struct Texts {
            offset: iced::Vector,
            clip: iced::Rectangle,
            scroll: Option<(iced::Rectangle, iced::Vector)>,
            pending: Option<iced::Rectangle>,
            container: Option<iced::Rectangle>,
            texts: Vec<ElementText>,
        }
        impl iced::advanced::widget::Operation for Texts {
            fn traverse(
                &mut self,
                operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation),
            ) {
                let previous = (self.offset, self.clip, self.container);
                if let Some((bounds, translation)) = self.scroll.take() {
                    self.clip = self
                        .clip
                        .intersection(&(bounds + self.offset))
                        .unwrap_or_default();
                    self.offset = self.offset - translation;
                }
                if let Some(bounds) = self.pending.take() {
                    self.container = Some(bounds);
                }
                operate(self);
                (self.offset, self.clip, self.container) = previous;
            }
            fn container(
                &mut self,
                _: Option<&iced::advanced::widget::Id>,
                bounds: iced::Rectangle,
            ) {
                self.pending = (bounds + self.offset).intersection(&self.clip);
            }
            fn scrollable(
                &mut self,
                _: Option<&iced::advanced::widget::Id>,
                bounds: iced::Rectangle,
                _: iced::Rectangle,
                translation: iced::Vector,
                _: &mut dyn iced::advanced::widget::operation::Scrollable,
            ) {
                self.scroll = Some((bounds, translation));
            }
            fn text(
                &mut self,
                _: Option<&iced::advanced::widget::Id>,
                bounds: iced::Rectangle,
                text: &str,
            ) {
                if let Some(bounds) = (bounds + self.offset).intersection(&self.clip) {
                    self.texts.push(ElementText {
                        text: text.into(),
                        bounds,
                        container: self.container,
                    });
                }
            }
        }
        let layouts = std::cell::RefCell::new(Vec::new());
        let size = Size::new(display_size[0] as f32, display_size[1] as f32);
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            Element::new(ObservedElement {
                content: element,
                layouts: &layouts,
            }),
            size,
            self.batch_cache.take().unwrap_or_default(),
            &mut self.renderer,
        );
        let mut messages = Vec::new();
        let (state, _) = ui.update(
            &[iced::Event::Window(window::Event::RedrawRequested(
                Instant::now(),
            ))],
            self.cursor,
            &mut self.renderer,
            &mut iced::advanced::clipboard::Null,
            &mut messages,
        );
        let interaction = match state {
            iced_runtime::user_interface::State::Updated {
                mouse_interaction, ..
            } => mouse_interaction,
            iced_runtime::user_interface::State::Outdated => {
                anyhow::bail!("observed element layout outdated")
            }
        };
        let mut texts = Texts {
            offset: iced::Vector::ZERO,
            clip: iced::Rectangle::with_size(size),
            scroll: None,
            pending: None,
            container: None,
            texts: Vec::new(),
        };
        ui.operate(&self.renderer, &mut texts);
        let theme = super::room_theme();
        ui.draw(
            &mut self.renderer,
            &theme,
            &iced::advanced::renderer::Style {
                text_color: theme.palette().text,
            },
            self.cursor,
        );
        self.batch_cache = Some(ui.into_cache());
        Ok(ElementObservation {
            layouts: layouts.into_inner(),
            texts: texts.texts,
            interaction,
            messages,
        })
    }
    fn profiled_element_batch(
        &mut self,
        element: Element<'_, Message>,
        display_size: [u32; 2],
        events: &[iced::Event],
    ) -> (Batch, Costs) {
        let mut costs = Costs::default();
        for event in events {
            match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    self.cursor = iced::mouse::Cursor::Available(*position)
                }
                iced::Event::Mouse(iced::mouse::Event::CursorLeft) => {
                    self.cursor = iced::mouse::Cursor::Unavailable
                }
                _ => {}
            }
        }
        let started = Instant::now();
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            element,
            Size::new(display_size[0] as f32, display_size[1] as f32),
            self.batch_cache.take().unwrap_or_default(),
            &mut self.renderer,
        );
        costs.build_layout_us = started.elapsed().as_micros();
        let mut messages = vec![];
        let started = Instant::now();
        let (_, statuses) = ui.update(
            events,
            self.cursor,
            &mut self.renderer,
            &mut iced::advanced::clipboard::Null,
            &mut messages,
        );
        costs.widget_update_us = started.elapsed().as_micros();
        let started = Instant::now();
        let mut owners = WidgetOwners::default();
        ui.operate(&self.renderer, &mut owners);
        self.batch_cache = Some(ui.into_cache());
        for (event, status) in events.iter().zip(&statuses) {
            if let Some(message) = subscription_event(event.clone(), *status, self.window) {
                messages.push(message);
            }
        }
        costs.owners_cache_subscription_us = started.elapsed().as_micros();
        (
            Batch {
                messages,
                statuses,
                owners,
            },
            costs,
        )
    }
    /// Draw the same runtime cache used by application_batch, including its
    /// real advanced overlays. This does not apply any application messages.
    pub(super) fn batch_pixels(&mut self, viewer: &Viewer) -> Result<Vec<u8>> {
        Ok(self.profiled_pixels(viewer)?.0)
    }
    pub(super) fn profiled_pixels(&mut self, viewer: &Viewer) -> Result<(Vec<u8>, Costs)> {
        self.profiled_pixels_at_scale(viewer, 1.)
    }
    /// Actual offscreen rendering at a display scale. UserInterface still uses
    /// logical size and logical events, as in the pinned Iced Winit conversion.
    pub(super) fn profiled_pixels_at_scale(
        &mut self,
        viewer: &Viewer,
        scale: f32,
    ) -> Result<(Vec<u8>, Costs)> {
        let start = Instant::now();
        let element = viewer.view();
        let view_us = start.elapsed().as_micros();
        let (pixels, mut costs) = self.element_pixels(element, viewer.display_size, scale)?;
        costs.view_us = view_us;
        Ok((pixels, costs))
    }
    pub(super) fn element_pixels(
        &mut self,
        element: Element<'_, Message>,
        display_size: [u32; 2],
        scale: f32,
    ) -> Result<(Vec<u8>, Costs)> {
        let physical = physical_size(display_size, scale)?;
        let mut costs = Costs::default();
        let started = Instant::now();
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            element,
            Size::new(display_size[0] as f32, display_size[1] as f32),
            self.batch_cache.take().unwrap_or_default(),
            &mut self.renderer,
        );
        costs.build_layout_us = started.elapsed().as_micros();
        // Cache persists widget state, but not UserInterface's overlay layout.
        // Keep this exact UI between update and draw so real tooltips render.
        let mut messages = vec![];
        let started = Instant::now();
        ui.update(
            &[iced::Event::Window(window::Event::RedrawRequested(
                Instant::now(),
            ))],
            self.cursor,
            &mut self.renderer,
            &mut iced::advanced::clipboard::Null,
            &mut messages,
        );
        costs.widget_update_us = started.elapsed().as_micros();
        ensure!(
            messages.iter().all(|m| matches!(
                m,
                Message::ViewportEnter | Message::ViewportLeave | Message::ViewportMotion { .. }
            )),
            "runtime redraw activated a control: {messages:?}"
        );
        let theme = super::room_theme();
        let started = Instant::now();
        ui.draw(
            &mut self.renderer,
            &theme,
            &iced::advanced::renderer::Style {
                text_color: theme.palette().text,
            },
            self.cursor,
        );
        costs.draw_us = started.elapsed().as_micros();
        self.batch_cache = Some(ui.into_cache());
        let started = Instant::now();
        let pixels = Headless::screenshot(
            &mut self.renderer,
            physical,
            scale,
            theme.palette().background,
        );
        costs.gpu_readback_us = started.elapsed().as_micros();
        Ok((pixels, costs))
    }
    pub(super) fn batch_save(&mut self, viewer: &Viewer, path: &Path) -> Result<()> {
        image::save_buffer(
            path,
            &self.batch_pixels(viewer)?,
            viewer.display_size[0],
            viewer.display_size[1],
            image::ColorType::Rgba8,
        )?;
        Ok(())
    }
    /// Candidate hit points from the actual runtime cache, including nested
    /// scroll offsets/clipping. The older Tree-only helper is a separate UI;
    /// it cannot discover controls after application_batch has scrolled.
    pub(super) fn batch_centers(&mut self, viewer: &Viewer) -> Vec<[f32; 2]> {
        struct Centers {
            offset: iced::Vector,
            clip: iced::Rectangle,
            scroll: Option<(iced::Rectangle, iced::Vector)>,
            points: Vec<[f32; 2]>,
        }
        impl iced::advanced::widget::Operation for Centers {
            fn traverse(
                &mut self,
                operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation),
            ) {
                let (offset, clip) = (self.offset, self.clip);
                if let Some((bounds, translation)) = self.scroll.take() {
                    self.clip = self
                        .clip
                        .intersection(&(bounds + self.offset))
                        .unwrap_or_default();
                    self.offset = self.offset - translation;
                }
                operate(self);
                self.offset = offset;
                self.clip = clip;
            }
            fn container(
                &mut self,
                _: Option<&iced::advanced::widget::Id>,
                bounds: iced::Rectangle,
            ) {
                if let Some(visible) = (bounds + self.offset).intersection(&self.clip)
                    && visible.width > 0.
                    && visible.height > 0.
                {
                    let point = [visible.center_x(), visible.center_y()];
                    if !self.points.contains(&point) {
                        self.points.push(point);
                    }
                }
            }
            fn scrollable(
                &mut self,
                _: Option<&iced::advanced::widget::Id>,
                bounds: iced::Rectangle,
                _: iced::Rectangle,
                translation: iced::Vector,
                _: &mut dyn iced::advanced::widget::operation::Scrollable,
            ) {
                // Pinned Iced Scrollable::operate supplies this owner directly
                // before traversing its content. No private state is read.
                self.scroll = Some((bounds, translation));
            }
        }
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            viewer.view(),
            size,
            self.batch_cache.take().unwrap_or_default(),
            &mut self.renderer,
        );
        let mut centers = Centers {
            offset: iced::Vector::ZERO,
            clip: iced::Rectangle::with_size(size),
            scroll: None,
            points: vec![],
        };
        ui.operate(&self.renderer, &mut centers);
        self.batch_cache = Some(ui.into_cache());
        centers.points
    }
    fn widget_event(
        &mut self,
        viewer: &Viewer,
        event: iced::Event,
    ) -> (Vec<Message>, iced::event::Status) {
        match &event {
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                self.cursor = iced::mouse::Cursor::Available(*position);
            }
            iced::Event::Mouse(iced::mouse::Event::CursorLeft) => {
                self.cursor = iced::mouse::Cursor::Unavailable;
            }
            _ => {}
        }
        let mut element = viewer.view();
        let tree = self
            .tree
            .get_or_insert_with(|| Tree::new(element.as_widget()));
        tree.diff(element.as_widget());
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let bounds = iced::Rectangle::with_size(size);
        let node = element.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let mut messages = vec![];
        let mut shell = Shell::new(&mut messages);
        // Match Iced's overlay-first dispatch. Tooltips and other overlays must
        // have the opportunity to capture an event before the underlying page.
        if let Some(mut overlay) = element.as_widget_mut().overlay(
            tree,
            Layout::new(&node),
            &self.renderer,
            &bounds,
            iced::Vector::ZERO,
        ) {
            let layout = overlay.as_overlay_mut().layout(&self.renderer, size);
            overlay.as_overlay_mut().update(
                &event,
                Layout::new(&layout),
                self.cursor,
                &self.renderer,
                &mut iced::advanced::clipboard::Null,
                &mut shell,
            );
        }
        if !shell.is_event_captured() {
            element.as_widget_mut().update(
                tree,
                &event,
                Layout::new(&node),
                self.cursor,
                &self.renderer,
                &mut iced::advanced::clipboard::Null,
                &mut shell,
                &bounds,
            );
        }
        let status = if shell.is_event_captured() {
            iced::event::Status::Captured
        } else {
            iced::event::Status::Ignored
        };
        (messages, status)
    }
    pub(super) fn move_to(&mut self, viewer: &Viewer, point: [f32; 2]) -> Vec<Message> {
        self.event(
            viewer,
            iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(point[0], point[1]),
            }),
        )
    }
    pub(super) fn button(&mut self, viewer: &Viewer, down: bool) -> Vec<Message> {
        self.event(
            viewer,
            iced::Event::Mouse(if down {
                iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)
            } else {
                iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)
            }),
        )
    }
    pub(super) fn centers(&mut self, viewer: &Viewer) -> Vec<[f32; 2]> {
        let mut element = viewer.view();
        let tree = self
            .tree
            .get_or_insert_with(|| Tree::new(element.as_widget()));
        tree.diff(element.as_widget());
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let node = element.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        fn collect(layout: Layout<'_>, points: &mut Vec<[f32; 2]>) {
            let bounds = layout.bounds();
            if bounds.width > 0. && bounds.height > 0. {
                let point = [bounds.center_x(), bounds.center_y()];
                if !points.contains(&point) {
                    points.push(point);
                }
            }
            for child in layout.children() {
                collect(child, points);
            }
        }
        let mut points = vec![];
        collect(Layout::new(&node), &mut points);
        points
    }
    /// Read actual Iced scroll/focus owners through supported operations. This
    /// neither changes offsets nor reads private scroll-state fields.
    pub(super) fn widget_owners(&mut self, viewer: &Viewer) -> WidgetOwners {
        let mut element = viewer.view();
        let tree = self
            .tree
            .get_or_insert_with(|| Tree::new(element.as_widget()));
        tree.diff(element.as_widget());
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let node = element.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        let mut owners = WidgetOwners::default();
        element
            .as_widget_mut()
            .operate(tree, Layout::new(&node), &self.renderer, &mut owners);
        owners
    }
    /// Locate the actual shader in the retained widget tree. Overlay geometry
    /// must not resize it when menus or feedback are added to the same page.
    pub(super) fn viewport_bounds(&mut self, viewer: &Viewer) -> Result<[f32; 4]> {
        let shader: Element<'_, Message> = widget::shader::Shader::new(Viewport(
            viewer
                .scene
                .as_ref()
                .context("viewport scene absent")?
                .clone(),
        ))
        .into();
        let tag = shader.as_widget().tag();
        let mut element = viewer.view();
        let tree = self
            .tree
            .get_or_insert_with(|| Tree::new(element.as_widget()));
        tree.diff(element.as_widget());
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let node = element.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        fn find(
            tree: &Tree,
            node: &layout::Node,
            tag: iced::advanced::widget::tree::Tag,
            offset: iced::Vector,
        ) -> Option<iced::Rectangle> {
            let bounds = node.bounds();
            let position = offset + iced::Vector::new(bounds.x, bounds.y);
            if tree.tag == tag {
                return Some(iced::Rectangle {
                    x: position.x,
                    y: position.y,
                    ..bounds
                });
            }
            tree.children
                .iter()
                .zip(node.children())
                .find_map(|(t, n)| find(t, n, tag, position))
        }
        find(tree, &node, tag, iced::Vector::ZERO)
            .map(|bounds| [bounds.x, bounds.y, bounds.width, bounds.height])
            .context("viewport widget absent")
    }
    /// Includes layout, draw, GPU work and readback, but no PNG encoding or
    /// adapter construction. The same renderer and texture cache are reused.
    pub(super) fn pixels(&mut self, viewer: &Viewer) -> Result<Vec<u8>> {
        let messages = self.event(
            viewer,
            iced::Event::Window(window::Event::RedrawRequested(Instant::now())),
        );
        // A newly exposed overlay can emit enter/leave notifications on
        // redraw. A screenshot never dispatches these to the application;
        // activation messages still indicate a test-driver/rendering error.
        ensure!(
            messages.iter().all(|m| matches!(
                m,
                Message::ViewportEnter | Message::ViewportLeave | Message::ViewportMotion { .. }
            )),
            "redraw emitted activation input: {messages:?}"
        );
        let mut element = viewer.view();
        let tree = self.tree.as_mut().unwrap();
        tree.diff(element.as_widget());
        let size = Size::new(viewer.display_size[0] as f32, viewer.display_size[1] as f32);
        let bounds = iced::Rectangle::with_size(size);
        let node = element.as_widget_mut().layout(
            tree,
            &self.renderer,
            &layout::Limits::new(Size::ZERO, size),
        );
        self.renderer.reset(bounds);
        let theme = super::room_theme();
        let style = iced::advanced::renderer::Style {
            text_color: theme.palette().text,
        };
        element.as_widget().draw(
            tree,
            &mut self.renderer,
            &theme,
            &style,
            Layout::new(&node),
            self.cursor,
            &bounds,
        );
        if let Some(mut overlay) = element.as_widget_mut().overlay(
            tree,
            Layout::new(&node),
            &self.renderer,
            &bounds,
            iced::Vector::ZERO,
        ) {
            let layout = overlay.as_overlay_mut().layout(&self.renderer, size);
            overlay.as_overlay().draw(
                &mut self.renderer,
                &theme,
                &style,
                Layout::new(&layout),
                self.cursor,
            );
        }
        Ok(Headless::screenshot(
            &mut self.renderer,
            Size::new(viewer.display_size[0], viewer.display_size[1]),
            1.,
            theme.palette().background,
        ))
    }
    pub(super) fn save(&mut self, viewer: &Viewer, path: &Path) -> Result<()> {
        let pixels = self.pixels(viewer)?;
        image::save_buffer(
            path,
            &pixels,
            viewer.display_size[0],
            viewer.display_size[1],
            image::ColorType::Rgba8,
        )?;
        Ok(())
    }
}
