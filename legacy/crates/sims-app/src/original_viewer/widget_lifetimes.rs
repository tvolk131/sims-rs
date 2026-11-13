//! Retain the widget tree across focus/size changes, ending only button presses.
//! Scroll positions, text focus, child state and simulation owners are untouched.
use super::*;
use iced::Theme;
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree, tree},
};

pub(super) fn retain(epoch: u64, content: Element<'_, Message>) -> Element<'_, Message> {
    Element::new(Buttons {
        epoch,
        content,
        focus_epoch: true,
        key: (),
    })
}
/// Native owner epochs are independent of delayed focus subscription messages.
/// Apply this only to the owner's subtree; unrelated HUD presses stay intact.
pub(super) fn retain_owner(epoch: u64, content: Element<'_, Message>) -> Element<'_, Message> {
    Element::new(Buttons {
        epoch,
        content,
        focus_epoch: false,
        key: (),
    })
}
/// End only this owner's presses when its actual control/resource key changes.
/// Equality is exact; neither publication age nor a probabilistic hash is used.
pub(super) fn retain_owner_key<K: Clone + PartialEq + 'static>(
    key: K,
    content: Element<'_, Message>,
) -> Element<'_, Message> {
    Element::new(Buttons {
        epoch: 0,
        content,
        focus_epoch: false,
        key,
    })
}

struct Buttons<'a, K> {
    epoch: u64,
    content: Element<'a, Message>,
    focus_epoch: bool,
    key: K,
}
struct Epoch<K> {
    value: u64,
    key: K,
    /// Immediate widget resets awaiting the matching subscription messages.
    handled_unfocused: u64,
}

fn reset_buttons(tree: &mut Tree, button: &Element<'_, Message>) {
    if tree.tag == button.as_widget().tag() {
        // Use Iced's own default button state without naming its private State
        // type, changing children, or assuming a pressed-field memory layout.
        tree.state = button.as_widget().state();
    }
    for child in &mut tree.children {
        reset_buttons(child, button);
    }
}

impl<K: Clone + PartialEq + 'static> Widget<Message, Theme, iced::Renderer> for Buttons<'_, K> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Epoch<K>>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Epoch {
            value: self.epoch,
            key: self.key.clone(),
            handled_unfocused: 0,
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(self.content.as_widget());
        let previous = tree.state.downcast_mut::<Epoch<K>>();
        let changed_key = previous.key != self.key;
        if previous.value != self.epoch || changed_key {
            let losses = self.epoch.wrapping_sub(previous.value);
            if changed_key || losses > previous.handled_unfocused {
                // A captured overlay event can skip the base update. Keep
                // this fallback for losses the widget did not see directly.
                let button: Element<'_, Message> = button(widget::Space::new()).into();
                reset_buttons(&mut tree.children[0], &button);
            }
            previous.handled_unfocused = previous.handled_unfocused.saturating_sub(losses);
            previous.value = self.epoch;
            previous.key = self.key.clone();
        }
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
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(child.size(), vec![child])
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &iced::Rectangle,
    ) {
        let unfocused = matches!(event, iced::Event::Window(window::Event::Unfocused));
        if unfocused || matches!(event, iced::Event::Window(window::Event::Resized(_))) {
            // UserInterface::update processes a whole event batch before
            // Viewer receives subscription messages. Cancel now, before any
            // later release in this same batch can use the old pressed flag.
            // A resize likewise precedes its delayed DisplaySize message;
            // releasing a pre-resize press must not activate the old layout.
            let button: Element<'_, Message> = button(widget::Space::new()).into();
            reset_buttons(&mut tree.children[0], &button);
            if unfocused && self.focus_epoch {
                let state = tree.state.downcast_mut::<Epoch<K>>();
                state.handled_unfocused = state.handled_unfocused.saturating_add(1);
            }
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &iced::Rectangle,
        translation: iced::Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}
