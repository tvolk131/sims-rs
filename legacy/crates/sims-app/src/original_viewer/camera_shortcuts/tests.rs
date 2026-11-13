//! Authored mapper/router controls, all UNRUN. No Session, assets or GUI.
//! Actual subscription/Host/direct Viewer behavior needs parent validation.

use super::*;
use keyboard::{Key, Location, Modifiers, key::{Code, Named, Physical}};

fn context(generation: u64, blocked: bool) -> Context {
    Context { generation, blocked }
}

fn press(key: Key, code: Code) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        modified_key: key.clone(),
        key,
        physical_key: Physical::Code(code),
        location: Location::Standard,
        modifiers: Modifiers::NONE,
        text: None,
        repeat: false,
    })
}

fn character(value: &str, code: Code) -> Event {
    press(Key::Character(value.into()), code)
}

fn named(key: Named, code: Code) -> Event {
    press(Key::Named(key), code)
}

fn flags(mut event: Event, modifiers: Modifiers, repeat: bool) -> Event {
    if let Event::Keyboard(keyboard::Event::KeyPressed {
        modifiers: actual_modifiers,
        repeat: actual_repeat,
        ..
    }) = &mut event {
        *actual_modifiers = modifiers;
        *actual_repeat = repeat;
    } else {
        panic!("test requires a press");
    }
    event
}

fn modified(mut event: Event, value: &str) -> Event {
    if let Event::Keyboard(keyboard::Event::KeyPressed { modified_key, .. }) = &mut event {
        *modified_key = Key::Character(value.into());
    } else {
        panic!("test requires a press");
    }
    event
}

fn input(event: &Event) -> Input {
    Input::from_iced(event, Status::Ignored).expect("camera candidate")
}

fn route(state: &mut State, event: &Event, generation: u64, current: Context) -> Option<Action> {
    state.route(input(event), generation, current)
}

fn cases() -> [(Event, Action); 7] {
    [
        (character(",", Code::Comma), Action::RotateLeft),
        (character(".", Code::Period), Action::RotateRight),
        (character("=", Code::Equal), Action::ZoomIn),
        (character("+", Code::Equal), Action::ZoomIn),
        (character("-", Code::Minus), Action::ZoomOut),
        (named(Named::PageDown, Code::PageDown), Action::Downstairs),
        (named(Named::PageUp, Code::PageUp), Action::Upstairs),
    ]
}

#[test]
fn mapper_and_router_cover_every_action_and_both_zoom_in_spellings() {
    for (event, action) in cases() {
        let candidate = input(&event);
        assert_eq!(candidate.action, action);
        assert!(!candidate.captured && !candidate.modified && !candidate.repeat);
        assert_eq!(State::default().route(candidate, 4, context(4, false)), Some(action));
    }
}

#[test]
fn mapper_accepts_normal_shift_plus_in_base_and_modified_key_forms() {
    for event in [
        modified(character("=", Code::Equal), "+"),
        character("+", Code::Equal),
        // A layout may put Shift+'+' on a different logical/physical base.
        modified(character("1", Code::Digit1), "+"),
    ] {
        let event = flags(event, Modifiers::SHIFT, false);
        let candidate = input(&event);
        assert_eq!(candidate.action, Action::ZoomIn);
        assert!(!candidate.modified);
        assert_eq!(State::default().route(candidate, 0, context(0, false)), Some(Action::ZoomIn));
    }
}

#[test]
fn mapper_uses_logical_numpad_plus_and_minus_without_physical_fallback() {
    for (value, code, action) in [
        ("+", Code::NumpadAdd, Action::ZoomIn),
        ("-", Code::NumpadSubtract, Action::ZoomOut),
    ] {
        let mut event = character(value, code);
        if let Event::Keyboard(keyboard::Event::KeyPressed { location, .. }) = &mut event {
            *location = Location::Numpad;
        }
        let candidate = input(&event);
        assert_eq!(candidate.physical_key, Physical::Code(code));
        assert_eq!(State::default().route(candidate, 3, context(3, false)), Some(action));
        assert!(Input::from_iced(&character("x", code), Status::Ignored).is_none());
    }
}

#[test]
fn mapper_preserves_captured_repeat_and_modifier_bits_for_latching() {
    for captured in [false, true] {
        for repeat in [false, true] {
            let event = flags(character(",", Code::Comma), Modifiers::CTRL, repeat);
            let status = if captured { Status::Captured } else { Status::Ignored };
            let candidate = Input::from_iced(&event, status).unwrap();
            assert_eq!(candidate.captured, captured);
            assert_eq!(candidate.repeat, repeat);
            assert!(candidate.modified);
            assert_eq!(candidate.physical_key, Physical::Code(Code::Comma));
        }
    }
}

#[test]
fn mapper_excludes_other_keys_and_never_duplicates_space_release_or_focus_bookkeeping() {
    let release = Event::Keyboard(keyboard::Event::KeyReleased {
        key: Key::Character(",".into()),
        modified_key: Key::Character("<".into()),
        physical_key: Physical::Code(Code::Comma),
        location: Location::Standard,
        modifiers: Modifiers::SHIFT,
    });
    for event in [
        character("x", Code::KeyX),
        character("", Code::Comma),
        character(",,", Code::Comma),
        character("<", Code::Comma),
        character(">", Code::Period),
        character("_", Code::Minus),
        character(" ", Code::Space),
        named(Named::Space, Code::Space),
        named(Named::ArrowLeft, Code::ArrowLeft),
        named(Named::ArrowRight, Code::ArrowRight),
        named(Named::ArrowUp, Code::ArrowUp),
        named(Named::ArrowDown, Code::ArrowDown),
        named(Named::Enter, Code::Enter),
        named(Named::Escape, Code::Escape),
        release,
        Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::SHIFT)),
        Event::Window(iced::window::Event::Unfocused),
        Event::Window(iced::window::Event::Focused),
    ] {
        for status in [Status::Ignored, Status::Captured] {
            assert!(Input::from_iced(&event, status).is_none(), "{event:?}");
        }
    }
}

#[test]
fn routing_refuses_ctrl_alt_command_combinations_including_unknown_modifier_bits() {
    let unknown = Modifiers::from_bits_retain(!Modifiers::all().bits());
    assert!(!unknown.is_empty());
    for modifiers in [
        Modifiers::CTRL,
        Modifiers::ALT,
        Modifiers::LOGO,
        Modifiers::CTRL | Modifiers::ALT,
        Modifiers::SHIFT | Modifiers::CTRL,
        Modifiers::SHIFT | Modifiers::ALT,
        Modifiers::SHIFT | Modifiers::LOGO,
        Modifiers::all(),
        unknown,
        Modifiers::SHIFT | unknown,
    ] {
        for (event, action) in cases() {
            let mut state = State::default();
            let current = context(2, false);
            let forbidden = flags(event.clone(), modifiers, false);
            let candidate = input(&forbidden);
            assert!(candidate.modified, "{modifiers:?}");
            assert_eq!(state.route(candidate, 2, current), None);
            // Dropping modifiers alone cannot turn the same hold into a down.
            assert_eq!(route(&mut state, &event, 2, current), None);
            state.release(candidate.physical_key, 2);
            assert_eq!(route(&mut state, &event, 2, current), Some(action));
        }
        let plus = flags(modified(character("=", Code::Equal), "+"), modifiers, false);
        assert!(input(&plus).modified);
        assert_eq!(route(&mut State::default(), &plus, 2, context(2, false)), None);
    }
}

#[test]
fn routing_shift_is_allowed_only_when_it_produces_plus() {
    for event in [
        modified(character(",", Code::Comma), "<"),
        modified(character(".", Code::Period), ">"),
        modified(character("-", Code::Minus), "_"),
        character("=", Code::Equal),
        named(Named::PageDown, Code::PageDown),
        named(Named::PageUp, Code::PageUp),
    ] {
        let mut state = State::default();
        let ordinary = event.clone();
        let shifted = flags(event, Modifiers::SHIFT, false);
        let candidate = input(&shifted);
        assert!(candidate.modified);
        assert_eq!(state.route(candidate, 1, context(1, false)), None);
        assert_eq!(route(&mut state, &ordinary, 1, context(1, false)), None);
        state.release(candidate.physical_key, 1);
        assert_eq!(route(&mut state, &ordinary, 1, context(1, false)), Some(input(&ordinary).action));
    }
}

#[test]
fn routing_captured_press_consumes_a_gesture_for_every_action() {
    for (event, action) in cases() {
        let mut state = State::default();
        let current = context(5, false);
        let captured = Input::from_iced(&event, Status::Captured).unwrap();
        assert_eq!(state.route(captured, 5, current), None);
        assert_eq!(route(&mut state, &event, 5, current), None);
        // Parent relays this release even when its original event was captured
        // or its logical key/modifiers no longer describe a camera shortcut.
        state.release(captured.physical_key, 5);
        assert_eq!(route(&mut state, &event, 5, current), Some(action));
    }
}

#[test]
fn routing_marked_and_unmarked_repeats_fire_once_per_physical_hold() {
    for (event, action) in cases() {
        let mut state = State::default();
        let current = context(1, false);
        assert_eq!(route(&mut state, &event, 1, current), Some(action));
        for repeat in [true, false, true, false] {
            let duplicate = flags(event.clone(), Modifiers::NONE, repeat);
            assert_eq!(route(&mut state, &duplicate, 1, current), None);
        }
        state.release(input(&event).physical_key, 1);
        assert_eq!(route(&mut state, &event, 1, current), Some(action));
    }
}

#[test]
fn routing_orphan_repeat_latches_before_refusal() {
    for (event, action) in cases() {
        let mut state = State::default();
        let current = context(1, false);
        let repeat = flags(event.clone(), Modifiers::NONE, true);
        assert_eq!(route(&mut state, &repeat, 1, current), None);
        assert_eq!(route(&mut state, &event, 1, current), None);
        state.release(input(&event).physical_key, 1);
        assert_eq!(route(&mut state, &event, 1, current), Some(action));
    }
}

#[test]
fn routing_simultaneous_physical_keys_keep_independent_latches() {
    let mut state = State::default();
    let current = context(6, false);
    // Omit the second Equal spelling, which deliberately shares a physical key.
    let events: Vec<_> = cases().into_iter().enumerate()
        .filter(|(index, _)| *index != 3).map(|(_, case)| case).collect();
    for (event, action) in &events {
        assert_eq!(route(&mut state, event, 6, current), Some(*action));
    }
    state.release(Physical::Code(Code::KeyX), 6);
    for (event, _) in &events {
        assert_eq!(route(&mut state, event, 6, current), None);
    }
    state.release(Physical::Code(Code::Comma), 6);
    assert_eq!(route(&mut state, &events[0].0, 6, current), Some(Action::RotateLeft));
    for (event, _) in &events[1..] {
        assert_eq!(route(&mut state, event, 6, current), None);
    }
}

#[test]
fn routing_equals_and_plus_share_a_physical_gesture_but_numpad_is_independent() {
    let mut state = State::default();
    let current = context(2, false);
    let equals = character("=", Code::Equal);
    let plus = flags(modified(equals.clone(), "+"), Modifiers::SHIFT, false);
    assert_eq!(route(&mut state, &equals, 2, current), Some(Action::ZoomIn));
    assert_eq!(route(&mut state, &plus, 2, current), None);
    assert_eq!(route(&mut state, &character("+", Code::NumpadAdd), 2, current), Some(Action::ZoomIn));
    state.release(Physical::Code(Code::NumpadAdd), 2);
    assert_eq!(route(&mut state, &plus, 2, current), None);
    state.release(Physical::Code(Code::Equal), 2);
    assert_eq!(route(&mut state, &plus, 2, current), Some(Action::ZoomIn));
}

#[test]
fn routing_logical_remapping_cannot_change_the_action_of_one_held_physical_key() {
    let mut state = State::default();
    let current = context(7, false);
    let down = character(",", Code::Comma);
    let remapped = named(Named::PageUp, Code::Comma);
    assert_eq!(route(&mut state, &down, 7, current), Some(Action::RotateLeft));
    assert_eq!(route(&mut state, &remapped, 7, current), None);
    state.release(Physical::Code(Code::PageUp), 7);
    assert_eq!(route(&mut state, &remapped, 7, current), None);
    state.release(Physical::Code(Code::Comma), 7);
    assert_eq!(route(&mut state, &remapped, 7, current), Some(Action::Upstairs));
}

#[test]
fn routing_blocked_overlay_and_owner_transitions_do_not_resume_a_held_key() {
    for (event, action) in cases() {
        let mut state = State::default();
        assert_eq!(route(&mut state, &event, 10, context(10, true)), None);
        for current in [context(10, false), context(11, true), context(11, false)] {
            assert_eq!(route(&mut state, &event, current.generation, current), None);
        }
        state.release(input(&event).physical_key, 10);
        assert_eq!(route(&mut state, &event, 11, context(11, false)), None);
        state.release(input(&event).physical_key, 11);
        assert_eq!(route(&mut state, &event, 11, context(11, false)), Some(action));
    }
}

#[test]
fn routing_live_context_refuses_an_input_mapped_while_the_overlay_was_closed() {
    let event = character(".", Code::Period);
    let candidate = input(&event);
    let mut state = State::default();
    assert_eq!(state.route(candidate, 3, context(3, true)), None);
    assert_eq!(state.route(candidate, 3, context(3, false)), None);
    state.release(candidate.physical_key, 3);
    assert_eq!(state.route(candidate, 3, context(3, false)), Some(Action::RotateRight));
}

#[test]
fn routing_stale_presses_and_duplicate_generations_cannot_rearm_newer_holds() {
    let event = character("-", Code::Minus);
    let candidate = input(&event);
    let mut state = State::default();
    let current = context(9, false);
    assert_eq!(state.route(candidate, 8, current), None);
    assert_eq!(state.route(candidate, 9, current), None);
    state.release(candidate.physical_key, 8);
    assert_eq!(state.route(candidate, 9, current), None);
    state.release(candidate.physical_key, 9);
    assert_eq!(state.route(candidate, 9, current), Some(Action::ZoomOut));
    // Neither an older duplicate nor its release lowers the newer latch fence.
    assert_eq!(state.route(candidate, 8, current), None);
    state.release(candidate.physical_key, 8);
    assert_eq!(state.route(candidate, 9, current), None);
    state.release(candidate.physical_key, 9);
    assert_eq!(state.route(candidate, 9, current), Some(Action::ZoomOut));
}

#[test]
fn routing_future_generation_is_refused_and_cannot_be_released_by_older_source() {
    let candidate = input(&named(Named::PageDown, Code::PageDown));
    let mut state = State::default();
    let current = context(9, false);
    assert_eq!(state.route(candidate, 10, current), None);
    state.release(candidate.physical_key, 9);
    assert_eq!(state.route(candidate, 9, current), None);
    state.release(candidate.physical_key, 10);
    assert_eq!(state.route(candidate, 9, current), Some(Action::Downstairs));
}

#[test]
fn routing_releases_use_matching_physical_identity_and_nonolder_generation() {
    let candidate = input(&named(Named::PageUp, Code::PageUp));
    let mut state = State::default();
    let current = context(4, false);
    assert_eq!(state.route(candidate, 4, current), Some(Action::Upstairs));
    state.release(Physical::Code(Code::PageDown), u64::MAX);
    state.release(candidate.physical_key, 3);
    assert_eq!(state.route(candidate, 4, current), None);
    // A release from a newer publication may end this same physical hold.
    state.release(candidate.physical_key, 5);
    assert_eq!(state.route(candidate, 5, context(5, false)), Some(Action::Upstairs));
}

#[test]
fn routing_focus_changes_preserve_holds_and_unfocused_presses_are_latched() {
    let mut state = State::default();
    let current = context(8, false);
    let left = input(&character(",", Code::Comma));
    let right = input(&character(".", Code::Period));
    assert_eq!(state.route(left, 8, current), Some(Action::RotateLeft));
    state.focus_lost();
    assert_eq!(state.route(right, 8, current), None);
    state.focus_gained();
    assert_eq!(state.route(left, 8, current), None);
    assert_eq!(state.route(right, 8, current), None);
    state.release(left.physical_key, 8);
    assert_eq!(state.route(left, 8, current), Some(Action::RotateLeft));
    assert_eq!(state.route(right, 8, current), None);
    state.release(right.physical_key, 8);
    assert_eq!(state.route(right, 8, current), Some(Action::RotateRight));
}

#[test]
fn routing_release_while_unfocused_and_blocked_does_not_enable_an_orphan_repeat() {
    let candidate = input(&character("=", Code::Equal));
    let mut state = State::default();
    assert_eq!(state.route(candidate, 2, context(2, false)), Some(Action::ZoomIn));
    state.focus_lost();
    state.release(candidate.physical_key, 2);
    assert_eq!(state.route(candidate, 3, context(3, true)), None);
    state.release(candidate.physical_key, 3);
    state.focus_gained();
    let repeat = Input { repeat: true, ..candidate };
    assert_eq!(state.route(repeat, 3, context(3, false)), None);
    assert_eq!(state.route(candidate, 3, context(3, false)), None);
    state.release(candidate.physical_key, 3);
    assert_eq!(state.route(candidate, 3, context(3, false)), Some(Action::ZoomIn));
}

#[test]
fn routing_generation_fences_use_comparison_without_wrapping_at_u64_max() {
    let candidate = input(&character(",", Code::Comma));
    let mut state = State::default();
    let current = context(u64::MAX, false);
    assert_eq!(state.route(candidate, u64::MAX, current), Some(Action::RotateLeft));
    assert_eq!(state.route(candidate, 0, current), None);
    state.release(candidate.physical_key, 0);
    assert_eq!(state.route(candidate, u64::MAX, current), None);
    state.release(candidate.physical_key, u64::MAX);
    assert_eq!(state.route(candidate, 0, current), None);
    state.release(candidate.physical_key, 0);
    assert_eq!(state.route(candidate, u64::MAX, current), Some(Action::RotateLeft));
}

const CAPACITY_KEYS: [Code; HELD_CAPACITY] = [
    Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD,
    Code::KeyE, Code::KeyF, Code::KeyG, Code::KeyH,
    Code::KeyI, Code::KeyJ, Code::KeyK, Code::KeyL,
    Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP,
];

#[test]
fn routing_capacity_can_reuse_a_released_slot_without_evicting_other_holds() {
    let mut state = State::default();
    let current = context(5, false);
    for code in CAPACITY_KEYS {
        assert_eq!(route(&mut state, &character(",", code), 5, current), Some(Action::RotateLeft));
    }
    state.release(Physical::Code(Code::KeyA), 5);
    assert_eq!(route(&mut state, &character(".", Code::KeyQ), 5, current), Some(Action::RotateRight));
    assert!(!state.saturated);
    for code in &CAPACITY_KEYS[1..] {
        assert_eq!(route(&mut state, &character(".", *code), 5, current), None);
    }
}

#[test]
fn routing_capacity_overflow_fails_closed_for_window_lifetime_without_eviction() {
    let mut state = State::default();
    let current = context(5, false);
    for code in CAPACITY_KEYS {
        assert_eq!(route(&mut state, &character(",", code), 5, current), Some(Action::RotateLeft));
    }
    let before = state.held;
    let overflow = character(".", Code::KeyQ);
    assert_eq!(route(&mut state, &overflow, 5, current), None);
    assert_eq!(state.held, before);
    assert!(state.saturated);
    // Losing the untracked overflow gesture's release cannot resume it when
    // tracked keys end, nor when publication/owner/focus state changes.
    for code in CAPACITY_KEYS {
        state.release(Physical::Code(code), 6);
    }
    state.focus_lost();
    state.focus_gained();
    for _ in 0..128 {
        assert_eq!(route(&mut state, &overflow, 6, context(6, false)), None);
    }
    state.release(Physical::Code(Code::KeyQ), 6);
    assert_eq!(route(&mut state, &character("=", Code::Equal), 6, context(6, false)), None);
    assert!(state.saturated);
    assert_eq!(state.held.len(), HELD_CAPACITY);
    assert!(state.held.iter().all(Option::is_none));
}

#[test]
fn captions_name_the_existing_command_and_its_shortcut() {
    for (action, expected) in [
        (Action::RotateLeft, "Rotate left (,)"),
        (Action::RotateRight, "Rotate right (.)"),
        (Action::ZoomIn, "Zoom in (= / +)"),
        (Action::ZoomOut, "Zoom out (-)"),
        (Action::Downstairs, "Downstairs (PageDown)"),
        (Action::Upstairs, "Upstairs (PageUp)"),
    ] {
        assert_eq!(caption(action), expected);
    }
}

#[test]
fn api_types_support_parent_message_dispatch_without_io_or_session() {
    fn dispatch_traits<T: std::fmt::Debug + Clone + Copy + Send + Sync>() {}
    fn state_traits<T: std::fmt::Debug + Clone + Default + Send + Sync>() {}
    dispatch_traits::<Action>();
    dispatch_traits::<Input>();
    dispatch_traits::<Context>();
    state_traits::<State>();
}
