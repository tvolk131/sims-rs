//! Camera intent only; the parent retains all engine and view mutations.
//!
//! Required parent wiring (outside this worker's two-file delta):
//! * Declare this module and `Message::CameraShortcut(Input)`. In the existing
//!   subscription mapper, keep `player_shortcuts::Input::from_iced` FIRST, then
//!   call this mapper BEFORE the captured-event and legacy keyboard guards.
//!   This mapper emits candidate presses only, including captured/repeat downs.
//! * Keep a separate default `State` on Host and on the direct Viewer adapter
//!   for each window's entire lifetime. Never reset it on publication, owner
//!   replacement, overlays or focus changes. Use only one adapter per event.
//! * The Space mapper already consumes ALL releases and focus notifications.
//!   In its existing dispatch branch, BEFORE generation/error/overlay/closing
//!   guards or an early Ignore return, relay `KeyReleased { physical_key }` to
//!   `release(physical_key, source_generation)` and FocusLost to `focus_lost()`.
//!   Relay FocusGained to `focus_gained()` ONLY if its source generation is
//!   current, as the existing Space policy does. Continue the existing Space
//!   route and FocusLost handler exactly once. No second release subscription.
//! * Route camera presses before those guards with the subscription generation
//!   and Context evaluated NOW. Block modal, Options, Diagnostics, picker,
//!   confirmation, fault/quarantine, closing, replacement and unavailable
//!   household states, including owner-side refusals published to Host.
//! * Map accepted actions to EXISTING messages: RotateLeft -> RotateBack,
//!   RotateRight -> Rotate, ZoomIn -> ZoomIn, ZoomOut -> ZoomOut,
//!   Downstairs -> Floor(1), Upstairs -> Floor(2). Feed those messages through
//!   ordinary generation-checked dispatch/queue and its source/widget/tutorial/
//!   engine guards. Do not queue raw CameraShortcut or mutate a view directly.
//! * Use `caption` on the existing camera buttons in both Viewer and published
//!   frontend. No readiness, native-camera or actual-controls claim follows
//!   until parent integration and validation. Authored tests are UNRUN here.

use iced::{Event, event::Status, keyboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    RotateLeft,
    RotateRight,
    ZoomIn,
    ZoomOut,
    Downstairs,
    Upstairs,
}

/// Opaque candidate press; authorization is deferred to State with live context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Input {
    action: Action,
    physical_key: keyboard::key::Physical,
    captured: bool,
    modified: bool,
    repeat: bool,
}

impl Input {
    pub(super) fn from_iced(event: &Event, status: Status) -> Option<Self> {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            modifiers,
            repeat,
            ..
        }) = event else {
            return None;
        };

        // Iced supplies both the base logical key and its modified form.
        // Shift+'=' normally produces '+'. Honor other layouts producing '+'
        // with Shift alone too; physical position never decides the action.
        let plus = matches!(modified_key.as_ref(), keyboard::Key::Character("+"));
        let action = if plus {
            Action::ZoomIn
        } else {
            match key.as_ref() {
                keyboard::Key::Character(",") => Action::RotateLeft,
                keyboard::Key::Character(".") => Action::RotateRight,
                keyboard::Key::Character("=" | "+") => Action::ZoomIn,
                keyboard::Key::Character("-") => Action::ZoomOut,
                keyboard::Key::Named(keyboard::key::Named::PageDown) => Action::Downstairs,
                keyboard::Key::Named(keyboard::key::Named::PageUp) => Action::Upstairs,
                _ => return None,
            }
        };
        let normal_shift_plus = plus && *modifiers == keyboard::Modifiers::SHIFT;
        Some(Self {
            action,
            physical_key: *physical_key,
            captured: status == Status::Captured,
            // Equality, rather than testing known forbidden bits, also refuses
            // unknown modifier bits. Every refused candidate still gets latched.
            modified: !modifiers.is_empty() && !normal_shift_plus,
            repeat: *repeat,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Context {
    pub(super) generation: u64,
    pub(super) blocked: bool,
}

const HELD_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    physical_key: keyboard::key::Physical,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct State {
    held: [Option<Held>; HELD_CAPACITY],
    focused: bool,
    // On overflow an untracked physical gesture may remain held indefinitely.
    // Refuse for the remaining window lifetime: neither an unrelated release
    // nor focus/owner changes can prove that gesture ended. Never evict a latch
    // or recover by clearing the table, which could authorize a held key.
    saturated: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            held: [None; HELD_CAPACITY],
            focused: true,
            saturated: false,
        }
    }
}

impl State {
    pub(super) fn route(
        &mut self,
        input: Input,
        source_generation: u64,
        current: Context,
    ) -> Option<Action> {
        if let Some(held) = self.held.iter_mut().flatten().find(|held| {
            held.physical_key == input.physical_key
        }) {
            // A newer observation of the same hold advances its release fence;
            // stale duplicates cannot lower it. No generation arithmetic wraps.
            held.generation = held.generation.max(source_generation);
            return None;
        }
        if self.saturated {
            return None;
        }
        let Some(slot) = self.held.iter_mut().find(|slot| slot.is_none()) else {
            self.saturated = true;
            return None;
        };
        // Bookkeep BEFORE refusing captured, modified, blocked, stale,
        // unfocused and orphan-repeat presses. A changed context is no new down.
        *slot = Some(Held {
            physical_key: input.physical_key,
            generation: source_generation,
        });
        if input.captured
            || input.modified
            || input.repeat
            || current.blocked
            || !self.focused
            || source_generation != current.generation
        {
            None
        } else {
            Some(input.action)
        }
    }

    pub(super) fn release(
        &mut self,
        key: keyboard::key::Physical,
        source_generation: u64,
    ) {
        for slot in &mut self.held {
            if slot.is_some_and(|held| {
                held.physical_key == key && source_generation >= held.generation
            }) {
                *slot = None;
                return;
            }
        }
    }

    pub(super) fn focus_lost(&mut self) {
        self.focused = false;
    }

    /// The parent must exclude stale FocusGained notifications before calling.
    pub(super) fn focus_gained(&mut self) {
        self.focused = true;
    }
}

pub(super) fn caption(action: Action) -> &'static str {
    match action {
        Action::RotateLeft => "Rotate left (,)",
        Action::RotateRight => "Rotate right (.)",
        Action::ZoomIn => "Zoom in (= / +)",
        Action::ZoomOut => "Zoom out (-)",
        Action::Downstairs => "Downstairs (PageDown)",
        Action::Upstairs => "Upstairs (PageUp)",
    }
}

#[cfg(test)]
#[path = "camera_shortcuts/tests.rs"]
mod tests;
