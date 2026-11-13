//! Window-lifetime Space and file intents; existing handlers retain authority.
//!
//! The subscription forwards captured/repeat candidates and every physical
//! release/focus notification before legacy guards. Host and direct Viewer
//! bookkeep before checking current ownership, mapping accepted intent to the
//! existing Toggle, DialogKey(32), Save or file-picker handler. No raw input
//! enters the queue. Space retains its original transport/modal policy. File
//! intents share bounded physical holds and stricter owner guards in adapters.
//! Host retains shortcut origin and revocable shell authority through dispatch;
//! the engine rechecks the actual owner before invoking the managed Save path.
//! Neither latch is reset by a publication or an owner replacement.
//! Authored tests are UNRUN; no native/window readiness gate is supplied here.

use iced::{Event, event::Status, keyboard, window};

/// Minimal data forwarded by the production subscription, including captured
/// presses/releases. Physical identity pairs releases across logical remaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Input {
    SavePressed {
        physical_key: keyboard::key::Physical,
        captured: bool,
        primary_only: bool,
        repeat: bool,
    },
    OpenPressed {
        physical_key: keyboard::key::Physical,
        captured: bool,
        primary_only: bool,
        repeat: bool,
    },
    SaveCopyPressed {
        physical_key: keyboard::key::Physical,
        captured: bool,
        primary_shift_only: bool,
        repeat: bool,
    },
    SpacePressed {
        physical_key: keyboard::key::Physical,
        captured: bool,
        modified: bool,
        repeat: bool,
    },
    KeyReleased {
        physical_key: keyboard::key::Physical,
    },
    FocusLost,
    FocusGained,
}

impl Input {
    pub(super) fn from_iced(event: &Event, status: Status) -> Option<Self> {
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                modifiers,
                repeat,
                ..
            }) if [key.as_ref(), modified_key.as_ref()].iter().any(|key| {
                matches!(key, keyboard::Key::Character("s" | "S"))
            }) => {
                // Both meanings of S use the same physical hold. Adding Shift
                // to a held Save (or primary to plain S) cannot open Copy.
                if *modifiers == (primary_modifier() | keyboard::Modifiers::SHIFT) {
                    Some(Self::SaveCopyPressed {
                        physical_key: *physical_key,
                        captured: status == Status::Captured,
                        primary_shift_only: true,
                        repeat: *repeat,
                    })
                } else {
                    Some(Self::SavePressed {
                        physical_key: *physical_key,
                        captured: status == Status::Captured,
                        // Text payloads never confer shortcut authority.
                        primary_only: *modifiers == primary_modifier(),
                        repeat: *repeat,
                    })
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key, modified_key, physical_key, modifiers, repeat, ..
            }) if [key.as_ref(), modified_key.as_ref()].iter().any(|key| {
                matches!(key, keyboard::Key::Character("o" | "O"))
            }) => Some(Self::OpenPressed {
                physical_key: *physical_key,
                captured: status == Status::Captured,
                primary_only: *modifiers == primary_modifier(),
                repeat: *repeat,
            }),
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                repeat,
                ..
            }) if matches!(
                key.as_ref(),
                keyboard::Key::Named(keyboard::key::Named::Space) | keyboard::Key::Character(" ")
            ) =>
            {
                Some(Self::SpacePressed {
                    physical_key: *physical_key,
                    captured: status == Status::Captured,
                    modified: !modifiers.is_empty(),
                    // iced_core 0.14.0 has this actual field; iced_winit 0.14.1
                    // passes through winit's repeat bit. Also latch below because
                    // a duplicate keydown need not be marked as a repeat.
                    repeat: *repeat,
                })
            }
            // Forward ALL releases, regardless of capture/modifiers/logical
            // key. The key's logical meaning may change while it is held.
            Event::Keyboard(keyboard::Event::KeyReleased { physical_key, .. }) => {
                Some(Self::KeyReleased {
                    physical_key: *physical_key,
                })
            }
            Event::Window(window::Event::Unfocused) => Some(Self::FocusLost),
            Event::Window(window::Event::Focused) => Some(Self::FocusGained),
            _ => None,
        }
    }
}

/// Current ownership, supplied at dispatch time rather than frozen at press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Context {
    pub(super) generation: u64,
    pub(super) modal: bool,
    pub(super) blocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    Ignore,
    DialogSpace,
    Toggle,
    Save,
    Load,
    SaveCopy,
}

pub(super) fn primary_modifier() -> keyboard::Modifiers {
    if cfg!(target_os = "macos") { keyboard::Modifiers::LOGO } else { keyboard::Modifiers::CTRL }
}

pub(super) fn save_caption() -> &'static str {
    if cfg!(target_os = "macos") { "Save game (Command+S)" } else { "Save game (Ctrl+S)" }
}

pub(super) fn save_guidance() -> &'static str {
    if cfg!(target_os = "macos") {
        "During ordinary play, Command+S saves to the current Save destination. Close Options first."
    } else {
        "During ordinary play, Ctrl+S saves to the current Save destination. Close Options first."
    }
}

pub(super) fn file_guidance() -> &'static str {
    if cfg!(target_os = "macos") {
        "Command+O: Load; Command+Shift+S: Save a Copy."
    } else {
        "Ctrl+O: Load; Ctrl+Shift+S: Save a Copy."
    }
}

// One history for all file intents, keyed by physical identity, including S
// changing to O under a logical remap. Retain the existing capacity/refusal.
const SAVE_HELD_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held {
    physical_key: keyboard::key::Physical,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct State {
    held: Option<Held>,
    save_held: [Option<Held>; SAVE_HELD_CAPACITY],
    save_saturated: bool,
    focused: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            held: None,
            save_held: [None; SAVE_HELD_CAPACITY],
            save_saturated: false,
            // The initial application window accepts input without requiring
            // an initial Focused notification. Unfocused explicitly fences it.
            focused: true,
        }
    }
}

impl State {
    /// Bookkeep first; authorize only a fresh, current, unmodified press.
    ///
    /// The held latch deliberately survives modal/overlay/generation/focus
    /// changes. A lost release outside the window can swallow the next press
    /// until its release, which is safer than resuming on a held gesture.
    /// Worker generations are monotonic and fail closed on exhaustion.
    pub(super) fn route(
        &mut self,
        input: Input,
        source_generation: u64,
        context: Context,
    ) -> Decision {
        match input {
            Input::KeyReleased { physical_key } => {
                if self.held.is_some_and(|held| {
                    held.physical_key == physical_key && source_generation >= held.generation
                }) {
                    self.held = None;
                }
                for slot in &mut self.save_held {
                    if slot.is_some_and(|held| held.physical_key == physical_key
                        && source_generation >= held.generation)
                    {
                        *slot = None;
                    }
                }
                Decision::Ignore
            }
            Input::FocusLost => {
                self.focused = false;
                Decision::Ignore
            }
            Input::FocusGained => {
                // A stale focus notification cannot reopen the new owner's
                // keyboard. Focus loss, conversely, always fences input.
                if source_generation == context.generation {
                    self.focused = true;
                }
                Decision::Ignore
            }
            Input::SavePressed { physical_key, captured, primary_only: exact_modifiers, repeat }
            | Input::OpenPressed { physical_key, captured, primary_only: exact_modifiers, repeat }
            | Input::SaveCopyPressed { physical_key, captured, primary_shift_only: exact_modifiers, repeat } => {
                if let Some(held) = self.save_held.iter_mut().flatten()
                    .find(|held| held.physical_key == physical_key)
                {
                    held.generation = held.generation.max(source_generation);
                    return Decision::Ignore;
                }
                if self.save_saturated { return Decision::Ignore; }
                let Some(slot) = self.save_held.iter_mut().find(|slot| slot.is_none()) else {
                    // Never evict an unproven held gesture. Overflow remains
                    // fail closed for this window, including owner replacement.
                    self.save_saturated = true;
                    return Decision::Ignore;
                };
                *slot = Some(Held { physical_key, generation: source_generation });
                if captured || !exact_modifiers || repeat || !self.focused
                    || source_generation != context.generation || context.blocked || context.modal
                {
                    Decision::Ignore
                } else {
                    match input {
                        Input::SavePressed { .. } => Decision::Save,
                        Input::OpenPressed { .. } => Decision::Load,
                        Input::SaveCopyPressed { .. } => Decision::SaveCopy,
                        _ => unreachable!(),
                    }
                }
            }
            Input::SpacePressed {
                physical_key,
                captured,
                modified,
                repeat,
            } => {
                if self.held.is_some() {
                    return Decision::Ignore;
                }
                // Even captured, modified, stale, unfocused, blocked and
                // orphan-repeat downs consume this physical gesture. Dropping
                // a modifier or dismissing an overlay does not create a press.
                self.held = Some(Held {
                    physical_key,
                    generation: source_generation,
                });
                if captured
                    || modified
                    || repeat
                    || !self.focused
                    || source_generation != context.generation
                    || context.blocked
                {
                    Decision::Ignore
                } else if context.modal {
                    Decision::DialogSpace
                } else {
                    Decision::Toggle
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "player_shortcuts/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "player_shortcuts/save_tests.rs"]
mod save_tests;
