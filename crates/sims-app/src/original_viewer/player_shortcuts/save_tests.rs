//! Authored Save routing/subscription controls: UNRUN, including compilation.
use super::*;
use keyboard::{Key, Location, Modifiers, key::{Code, Physical}};
use super::super::{Message, subscription_event};

fn press(modifiers: Modifiers, repeat: bool) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Character("s".into()), modified_key: Key::Character("s".into()),
        physical_key: Physical::Code(Code::KeyS), location: Location::Standard,
        modifiers, repeat, text: None,
    })
}
fn release(code: Code) -> Input {
    // Capture, logical remapping and changed modifiers must not hide a release.
    let event = Event::Keyboard(keyboard::Event::KeyReleased {
        key: Key::Unidentified, modified_key: Key::Character("X".into()),
        physical_key: Physical::Code(code), location: Location::Standard,
        modifiers: Modifiers::ALT | Modifiers::SHIFT,
    });
    match subscription_event(event, Status::Captured, window::Id::unique()).unwrap() {
        Message::PlayerShortcut(input) => input,
        other => panic!("release lost in production subscription: {other:?}"),
    }
}
fn input(event: Event, status: Status) -> Input {
    match subscription_event(event, status, window::Id::unique()).unwrap() {
        Message::PlayerShortcut(input) => input,
        other => panic!("Save candidate lost in production subscription: {other:?}"),
    }
}
fn context(generation: u64) -> Context { Context { generation, modal: false, blocked: false } }

#[test]
fn save_shortcut_subscription_uses_keys_and_modified_keys_never_text_or_physical_position() {
    for modified_only in [false, true] {
        let mut event = press(primary_modifier(), false);
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, text, .. }) = &mut event {
            *key = if modified_only { Key::Unidentified } else { Key::Character("S".into()) };
            *modified_key = if modified_only { Key::Character("s".into()) } else { Key::Unidentified };
            *text = Some("unrelated text".into());
        }
        for status in [Status::Captured, Status::Ignored] {
            assert!(matches!(input(event.clone(), status), Input::SavePressed {
                captured, primary_only: true, repeat: false, ..
            } if captured == (status == Status::Captured)));
        }
    }
    let mut event = press(primary_modifier(), false);
    if let Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, text, .. }) = &mut event {
        *key = Key::Unidentified;
        *modified_key = Key::Unidentified;
        *text = Some("s".into());
    }
    assert!(subscription_event(event, Status::Ignored, window::Id::unique()).is_none());
}

#[test]
fn save_shortcut_exact_platform_modifier_and_every_refused_down_consume_the_gesture() {
    let primary = primary_modifier();
    let foreign = if cfg!(target_os = "macos") { Modifiers::CTRL } else { Modifiers::LOGO };
    for modifiers in [Modifiers::NONE, foreign, primary | foreign,
        primary | Modifiers::ALT,
        primary | Modifiers::SHIFT | Modifiers::ALT]
    {
        let mut state = State::default();
        assert_eq!(state.route(input(press(modifiers, false), Status::Ignored), 4, context(4)), Decision::Ignore);
        assert_eq!(state.route(input(press(primary, false), Status::Ignored), 4, context(4)), Decision::Ignore);
        state.route(release(Code::KeyS), 4, context(4));
        assert_eq!(state.route(input(press(primary, false), Status::Ignored), 4, context(4)), Decision::Save);
    }

    // Shift is now the distinct Copy intent. It never emits Save or converts
    // a held S gesture into a second filesystem action.
    let mut state = State::default();
    assert_eq!(state.route(input(press(primary | Modifiers::SHIFT, false), Status::Ignored), 4, context(4)), Decision::SaveCopy);
    assert_eq!(state.route(input(press(primary, false), Status::Ignored), 4, context(4)), Decision::Ignore);
    state.route(release(Code::KeyS), 4, context(4));
    assert_eq!(state.route(input(press(primary, false), Status::Ignored), 4, context(4)), Decision::Save);
}

#[test]
fn save_shortcut_capture_repeat_modal_and_owner_blockers_never_resume_a_held_gesture() {
    for refusal in ["captured", "repeat", "modal", "blocked", "unfocused", "stale"] {
        let mut state = State::default();
        let mut current = context(4);
        current.modal = refusal == "modal";
        current.blocked = refusal == "blocked";
        if refusal == "unfocused" { state.route(Input::FocusLost, 4, current); }
        let candidate = input(press(primary_modifier(), refusal == "repeat"),
            if refusal == "captured" { Status::Captured } else { Status::Ignored });
        assert_eq!(state.route(candidate, if refusal == "stale" { 3 } else { 4 }, current), Decision::Ignore);
        state.route(Input::FocusGained, 4, context(4));
        for repeat in [true, false] {
            assert_eq!(state.route(input(press(primary_modifier(), repeat), Status::Ignored), 4, context(4)), Decision::Ignore);
        }
        state.route(release(Code::KeyS), 4, context(4));
        assert_eq!(state.route(input(press(primary_modifier(), false), Status::Ignored), 4, context(4)), Decision::Save);
    }
}

#[test]
fn save_shortcut_generation_release_fence_and_stale_focus_survive_owner_changes() {
    let mut state = State::default();
    let down = input(press(primary_modifier(), false), Status::Ignored);
    assert_eq!(state.route(down, 3, context(3)), Decision::Save);
    assert_eq!(state.route(down, 4, context(4)), Decision::Ignore);
    state.route(release(Code::KeyS), 3, context(4));
    assert_eq!(state.route(down, 4, context(4)), Decision::Ignore);
    state.route(release(Code::KeyS), 4, context(4));
    state.route(Input::FocusLost, 3, context(4));
    state.route(Input::FocusGained, 3, context(4));
    assert_eq!(state.route(down, 4, context(4)), Decision::Ignore);
    state.route(Input::FocusGained, 4, context(4));
    assert_eq!(state.route(down, 4, context(4)), Decision::Ignore);
    state.route(release(Code::KeyS), 4, context(4));
    assert_eq!(state.route(down, 4, context(4)), Decision::Save);
}

#[test]
fn save_shortcut_all_physical_gestures_are_bounded_and_overflow_never_rearms() {
    let mut state = State::default();
    let codes = [Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE,
        Code::KeyF, Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK,
        Code::KeyL, Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ];
    for code in codes {
        let mut event = press(primary_modifier(), false);
        if let Event::Keyboard(keyboard::Event::KeyPressed { physical_key, .. }) = &mut event {
            *physical_key = Physical::Code(code);
        }
        assert_eq!(state.route(input(event, Status::Captured), 1, context(1)), Decision::Ignore);
    }
    for code in codes { state.route(release(code), 2, context(2)); }
    state.route(Input::FocusLost, 2, context(2));
    state.route(Input::FocusGained, 2, context(2));
    assert_eq!(state.route(input(press(primary_modifier(), false), Status::Ignored), 2, context(2)), Decision::Ignore);
}

#[test]
fn save_shortcut_caption_and_options_guidance_match_platform_authority() {
    let primary = if cfg!(target_os = "macos") { "Command+S" } else { "Ctrl+S" };
    assert!(save_caption().contains(primary));
    assert!(save_guidance().contains(primary));
    assert!(save_guidance().contains("Close Options first"));
}
