//! IO-free saved-household path entry. Submit is a request, never a result.
//! Load keeps its existing API and text. Copy explicitly selects Purpose::Copy
//! and binds its one-shot path ticket to the retained household owner.
//!
//! Parent wiring (outside this worker's owned delta):
//! * Declare `#[path = "original_viewer/load_picker.rs"] mod load_picker;`.
//! * Retain one `Option<Picker>` per responsive Host/direct Viewer adapter. Add
//!   `Message::ChooseLoad`, `Message::LoadPicker(Event)` and, as needed,
//!   `Message::LoadFrom(PathBuf, Ticket)`. Both Options Load controls open it.
//! * Seed from the actual last requested Load path/current input observation's
//!   `requested_path`, falling back to the configured destination only when
//!   appropriate. Do not change `args.save` or cache a Saved/origin setter.
//! * Map `view(enabled)` through LoadPicker and route Escape via cancel_event.
//!   Apply CURRENT generation/owner/Options/dialog/fault/closing/replacement
//!   guards to every dispatch; `enabled` controls widgets, not parent authority.
//! * Classify LoadFrom as existing replacement work. Generalize the existing
//!   Load preflight to the selected path, preserving the same-house predicate.
//!   Move its exact owned Input into the consuming loader without reopening it.
//! * Carry the ticket through the existing guarded queue/completion path.
//!   Recoverable preflight rejection calls settle(ticket, Some(feedback));
//!   retain the path and picker for retry. After the ENTIRE successful Load
//!   tail, close only if settle(ticket, None) succeeds on the current picker.
//!   Foreign/stale tickets must never close a newer picker. Native/restore/
//!   publication failures retain their existing fatal handling, not recovery.
//! * Cancelled drops only this picker. Pending cancellation is refused here;
//!   parent must not close it as if an already consuming Load were cancelled.
//!
//! No Session, filesystem, configured Save destination or gameplay gate is
//! touched here. Both adapters retain and settle this same sealed picker.

use iced::{Element, Size, Theme, widget, window};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree, tree},
};
use iced::widget::{button, column, container, row, text, text_input};
use std::{cell::RefCell, path::{Path, PathBuf}, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Purpose { Load, Copy }

/// Equality compares allocations, never the equal unit values inside them.
/// Live events/tickets retain their allocation, so addresses cannot be reused
/// to authenticate a different picker, text revision or attempt.
#[derive(Debug, Clone)]
struct Identity(Arc<()>);

impl Identity {
    fn new() -> Self {
        Self(Arc::new(()))
    }
}

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Identity {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seal {
    picker: Identity,
    version: Identity,
}

/// Opaque, clonable completion authority for exactly one submitted attempt.
#[derive(Debug, Clone)]
pub(super) struct Ticket {
    picker: Identity,
    attempt: Identity,
    purpose: Purpose,
    path: PathBuf,
    owner: Option<super::save_copy::Owner>,
    consumed: Arc<std::sync::atomic::AtomicBool>,
}

impl Ticket {
    pub(super) fn purpose(&self) -> Purpose { self.purpose }
    pub(super) fn matches_path(&self, path: &Path) -> bool {
        self.path.as_os_str() == path.as_os_str()
    }
    /// Cloning a ticket retains the same one-shot authority; it cannot replay
    /// a write or transfer authority to another owner, path or picker purpose.
    pub(super) fn claim_copy(&self, path: &Path, owner: &super::save_copy::Owner) -> bool {
        self.purpose == Purpose::Copy && self.matches_path(path)
            && self.owner.as_ref().is_some_and(|bound| bound.same(owner))
            && !self.consumed.swap(true, std::sync::atomic::Ordering::AcqRel)
    }
}

#[derive(Debug, Clone)]
enum Intent {
    Edit { value: String, next: Identity },
    ChooseRecent { entry: usize },
    Submit,
    Cancel,
}

/// The payload's fields and constructors stay private. The parent can forward
/// or clone an event, but cannot adopt its identity for a different intent/path.
#[derive(Debug, Clone)]
pub(super) struct SealedInput {
    seal: Seal,
    intent: Intent,
}

#[derive(Debug, Clone)]
pub(super) enum Event {
    Input(SealedInput),
}

impl Event {
    pub(super) fn is_recent_choice(&self) -> bool {
        matches!(self, Self::Input(SealedInput { intent: Intent::ChooseRecent { .. }, .. }))
    }
}

#[derive(Debug)]
pub(super) enum Outcome {
    None,
    FocusPath,
    Submit { path: PathBuf, ticket: Ticket },
    Cancelled,
}

/// Non-Clone owner of the field and its private authentication identities.
pub(super) struct Picker {
    purpose: Purpose,
    owner: Option<super::save_copy::Owner>,
    seal: Seal,
    value: String,
    input_id: widget::Id,
    pending: Option<Ticket>,
    feedback: Option<String>,
    closed: bool,
    recent: super::recent_loads::History,
}

/// Each view's editor chains edits emitted in one Iced event batch. A second
/// edit authenticates against the first edit's resulting revision, rather than
/// losing fast typing because application updates run after the whole batch.
/// A competing/stale view cannot join this chain. Submit/Cancel deliberately
/// retain the view's ORIGINAL seal, so an old release cannot select new text.
struct Editor {
    picker: Identity,
    version: RefCell<Identity>,
}

impl Editor {
    fn changed(&self, value: String) -> Event {
        let next = Identity::new();
        let version = self.version.replace(next.clone());
        Event::Input(SealedInput {
            seal: Seal { picker: self.picker.clone(), version },
            intent: Intent::Edit { value, next },
        })
    }
}

/// Unlike Iced's global focus operation, a missing target must not unfocus
/// another field when an opening task outlives its picker.
struct FocusPath(widget::Id);

impl<T> Operation<T> for FocusPath {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn focusable(
        &mut self, id: Option<&widget::Id>, bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        if id == Some(&self.0) {
            iced::advanced::widget::operation::focusable::focus::<T>(self.0.clone())
                .focusable(id, bounds, state);
        }
    }
}

impl Picker {
    pub(super) fn new(initial: &Path) -> anyhow::Result<Self> {
        let value = initial.to_str().ok_or_else(|| anyhow::anyhow!(
            "The initial saved-household path cannot be represented exactly in a UTF-8 path field."
        ))?;
        Ok(Self {
            purpose: Purpose::Load,
            owner: None,
            seal: Seal { picker: Identity::new(), version: Identity::new() },
            value: value.to_owned(),
            input_id: widget::Id::unique(),
            pending: None,
            feedback: None,
            closed: false,
            recent: Default::default(),
        })
    }

    pub(super) fn new_copy(initial: &Path, owner: super::save_copy::Owner) -> anyhow::Result<Self> {
        let mut picker = Self::new(initial)?;
        picker.purpose = Purpose::Copy;
        picker.owner = Some(owner);
        Ok(picker)
    }

    pub(super) fn purpose(&self) -> Purpose { self.purpose }

    pub(super) fn with_recent(mut self, history: &super::recent_loads::History) -> Self {
        if self.purpose == Purpose::Load { self.recent = history.snapshot(); }
        self
    }

    /// Return once, only when the adapter accepts this new picker. Focusing
    /// moves Iced's cursor to the end, so selection must be the second task.
    pub(super) fn opening_task<Message: Send + 'static>(&self) -> iced::Task<Message> {
        iced::advanced::widget::operate(FocusPath(self.input_id.clone()))
            .chain(widget::operation::select_all(self.input_id.clone()))
    }

    pub(super) fn focus_lost(&mut self) { self.renew_version(); }

    pub(super) fn view(&self, enabled: bool) -> Element<'_, Event> {
        self.view_with_recent_gate(enabled, enabled)
    }

    pub(super) fn view_with_recent_gate(&self, enabled: bool, recent_enabled: bool) -> Element<'_, Event> {
        let enabled = self.controls_enabled(enabled);
        let mut input = text_input("Saved household JSON path", &self.value)
            .id(self.input_id.clone())
            .padding(10);
        if enabled {
            let editor = self.editor();
            input = input.on_input(move |value| editor.changed(value))
                .on_submit(self.event(Intent::Submit));
        }
        let controls = row![
            button(match (self.purpose, self.pending()) {
                (Purpose::Load, true) => "Loading…",
                (Purpose::Load, false) => "Load selected file",
                (Purpose::Copy, true) => "Saving copy…",
                (Purpose::Copy, false) => "Save a Copy…",
            })
                .on_press_maybe(enabled.then(|| self.event(Intent::Submit))),
            button("Cancel").on_press_maybe(enabled.then(|| self.cancel_event())),
        ].spacing(10);
        // Only button state resets on semantic changes; the text field's
        // stable ID, focus, selection and cursor survive edits and feedback.
        let controls = Element::new(Controls {
            key: ControlKey { seal: self.seal.clone(), enabled, entry: None },
            content: controls.into(),
        });
        let mut content = column![
            text(match self.purpose { Purpose::Load => "Load saved household", Purpose::Copy => "Save a Copy…" }).size(24),
            text(match self.purpose {
                Purpose::Load => "Select a saved household JSON for the current household.",
                Purpose::Copy => "Save the current household to another path. An existing destination is replaced. Save and Continue keep their current destinations.",
            }),
        ].spacing(12);
        if self.purpose == Purpose::Load && !self.recent.paths().is_empty() {
            let recent_enabled = enabled && recent_enabled;
            let choices = column(self.recent.paths().iter().enumerate().map(|(entry, path)| {
                // Snapshot entries never move within a picker. Reset held
                // buttons on picker/revision/enable/entry changes, including
                // replacement with a reordered snapshot at the same position.
                Element::new(Controls {
                    key: ControlKey { seal: self.seal.clone(), enabled: recent_enabled, entry: Some(entry) },
                    content: button(text(path.to_str().expect("exact UTF-8 history")))
                        .width(iced::Length::Fill)
                        .on_press_maybe(recent_enabled.then(|| self.event(Intent::ChooseRecent { entry })))
                        .into(),
                })
            })).spacing(6);
            content = content.push(text("Recent paths in this window").size(16))
                .push(text("Files may have moved or changed. Choose a path, then press Enter to load.").size(12))
                .push(widget::scrollable(choices).height(112));
        }
        content = content.extend([
            text("Path (used exactly as entered)").into(),
            input.into(),
            text(self.feedback.as_deref().unwrap_or("")).into(),
            controls,
        ]);
        // The entire form must remain reachable when the window is short.
        // Shrink keeps the normal dialog size and clamps its viewport to limits.
        container(widget::scrollable(content).height(iced::Length::Shrink))
            .padding(20).width(600).into()
    }

    pub(super) fn update(&mut self, event: Event) -> Outcome {
        let Event::Input(input) = event;
        if self.closed || self.pending() || input.seal != self.seal {
            return Outcome::None;
        }
        match input.intent {
            Intent::ChooseRecent { entry } => {
                if self.purpose != Purpose::Load { return Outcome::None; }
                let Some(path) = self.recent.paths().get(entry) else { return Outcome::None; };
                let Some(value) = path.to_str() else { return Outcome::None; };
                let mut chosen = String::new();
                if chosen.try_reserve_exact(value.len()).is_err() { return Outcome::None; }
                chosen.push_str(value);
                self.value = chosen;
                self.feedback = None;
                self.renew_version();
                Outcome::FocusPath
            }
            Intent::Edit { value, next } => {
                // Even an equal-value edit advances the chain. Otherwise an
                // Iced no-op edit could strand subsequent edits in its batch.
                if value != self.value {
                    self.feedback = None;
                }
                self.value = value;
                self.seal.version = next;
                Outcome::None
            }
            Intent::Submit => {
                self.renew_version();
                if self.value.is_empty() {
                    self.feedback = Some("Enter a saved household JSON path.".into());
                    return Outcome::None;
                }
                if self.value.contains('\0') {
                    self.feedback = Some("The saved household path cannot contain a NUL character.".into());
                    return Outcome::None;
                }
                let ticket = Ticket {
                    picker: self.seal.picker.clone(),
                    attempt: Identity::new(),
                    purpose: self.purpose,
                    path: PathBuf::from(&self.value),
                    owner: self.owner.clone(),
                    consumed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                };
                self.pending = Some(ticket.clone());
                self.feedback = None;
                Outcome::Submit { path: PathBuf::from(&self.value), ticket }
            }
            Intent::Cancel => {
                self.renew_version();
                self.closed = true;
                Outcome::Cancelled
            }
        }
    }

    /// Suitable for parent Escape routing; update still refuses pending/stale
    /// cancellation. The parent must inspect Outcome before closing anything.
    pub(super) fn cancel_event(&self) -> Event {
        self.event(Intent::Cancel)
    }

    pub(super) fn pending_ticket(&self, ticket: &Ticket) -> bool {
        self.pending.as_ref().is_some_and(|pending|
            ticket.purpose == self.purpose && ticket.picker == self.seal.picker && ticket.picker == pending.picker
                && ticket.attempt == pending.attempt)
    }

    #[cfg(test)]
    pub(super) fn edit_event_for_test(&self, value: String) -> Event { self.editor().changed(value) }
    #[cfg(test)]
    pub(super) fn submit_event_for_test(&self) -> Event { self.event(Intent::Submit) }
    #[cfg(test)]
    pub(super) fn recent_event_for_test(&self, entry: usize) -> Event { self.event(Intent::ChooseRecent { entry }) }

    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn value(&self) -> &str {
        &self.value
    }

    /// Some(feedback) reopens the SAME field for retry. None marks successful
    /// completion terminal until the parent drops it. A false return has no
    /// effect and must not authorize closing the current picker.
    pub(super) fn settle(&mut self, ticket: &Ticket, feedback: Option<String>) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        if ticket.purpose != self.purpose || ticket.picker != self.seal.picker || ticket.picker != pending.picker
            || ticket.attempt != pending.attempt
        {
            return false;
        }
        self.pending = None;
        self.closed = feedback.is_none();
        self.feedback = feedback;
        self.renew_version();
        true
    }

    fn event(&self, intent: Intent) -> Event {
        Event::Input(SealedInput { seal: self.seal.clone(), intent })
    }

    fn editor(&self) -> Editor {
        Editor { picker: self.seal.picker.clone(), version: RefCell::new(self.seal.version.clone()) }
    }

    fn renew_version(&mut self) {
        self.seal.version = Identity::new();
    }

    fn controls_enabled(&self, enabled: bool) -> bool {
        enabled && !self.pending() && !self.closed
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ControlKey {
    seal: Seal,
    enabled: bool,
    entry: Option<usize>,
}

/// Event-typed counterpart of the source-owned widget_lifetimes convention.
/// That helper accepts parent Message; this module cannot depend on an as-yet
/// unwired LoadPicker variant. Preserve the child tree and reset only Iced's
/// own button state, without assuming its private pressed-state layout.
struct Controls<'a> {
    key: ControlKey,
    content: Element<'a, Event>,
}

fn reset_buttons(tree: &mut Tree) {
    let button: Element<'_, Event> = button(widget::Space::new()).into();
    fn reset(tree: &mut Tree, button: &Element<'_, Event>) {
        if tree.tag == button.as_widget().tag() {
            tree.state = button.as_widget().state();
        }
        for child in &mut tree.children {
            reset(child, button);
        }
    }
    reset(tree, &button);
}

impl Widget<Event, Theme, iced::Renderer> for Controls<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ControlKey>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(self.key.clone())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.content.as_widget())]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.children[0].diff(self.content.as_widget());
        let previous = tree.state.downcast_mut::<ControlKey>();
        if *previous != self.key {
            *previous = self.key.clone();
            reset_buttons(&mut tree.children[0]);
        }
    }
    fn size(&self) -> Size<iced::Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<iced::Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        let child = self.content.as_widget_mut().layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(child.size(), vec![child])
    }
    fn operate(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer, operation: &mut dyn Operation) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0], layout.children().next().unwrap(), renderer, operation,
        );
    }
    fn update(
        &mut self, tree: &mut Tree, event: &iced::Event, layout: Layout<'_>,
        cursor: mouse::Cursor, renderer: &iced::Renderer, clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Event>, viewport: &iced::Rectangle,
    ) {
        if matches!(event, iced::Event::Window(window::Event::Unfocused | window::Event::Resized(_))) {
            // End the press NOW: a later release in the same runtime batch
            // precedes parent focus/resize subscription bookkeeping.
            reset_buttons(&mut tree.children[0]);
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0], event, layout.children().next().unwrap(),
            cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn draw(
        &self, tree: &Tree, renderer: &mut iced::Renderer, theme: &Theme,
        style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0], renderer, theme, style,
            layout.children().next().unwrap(), cursor, viewport,
        );
    }
    fn mouse_interaction(
        &self, tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor,
        viewport: &iced::Rectangle, renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0], layout.children().next().unwrap(), cursor, viewport, renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self, tree: &'a mut Tree, layout: Layout<'a>, renderer: &iced::Renderer,
        viewport: &iced::Rectangle, translation: iced::Vector,
    ) -> Option<overlay::Element<'a, Event, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0], layout.children().next().unwrap(), renderer, viewport, translation,
        )
    }
}

#[cfg(test)]
#[path = "load_picker/tests.rs"]
pub(super) mod tests;
