//! Authored controls only: UNRUN. Production controls intentionally require
//! the parent subscription/Host wiring documented in player_shortcuts.rs.
use super::*;
use keyboard::{
    Key, Location, Modifiers,
    key::{Code, Named, Physical},
};

// New file-gesture controls are authored UNRUN, including compilation. Existing
// Save controls remain in save_tests.rs; its old Shift+S rejection expectation
// needs the parent to update that separately owned baseline test for Copy.
mod file_shortcuts {
    use super::*;
    use super::super::super::{Message, subscription_event};

    fn down(copy: bool, modifiers: Modifiers, repeat: bool) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(if copy { "s" } else { "o" }.into()),
            modified_key: Key::Character(if copy { "S" } else { "O" }.into()),
            physical_key: Physical::Code(if copy { Code::KeyS } else { Code::KeyO }),
            location: Location::Standard, modifiers, text: None, repeat,
        })
    }
    fn modifiers(copy: bool) -> Modifiers {
        primary_modifier() | if copy { Modifiers::SHIFT } else { Modifiers::NONE }
    }
    fn expected(copy: bool) -> Decision { if copy { Decision::SaveCopy } else { Decision::Load } }
    fn input(event: Event, status: Status) -> Input {
        match subscription_event(event, status, window::Id::unique()).unwrap() {
            Message::PlayerShortcut(input) => input,
            other => panic!("file gesture bypassed router: {other:?}"),
        }
    }
    fn up(code: Code) -> Input {
        input(Event::Keyboard(keyboard::Event::KeyReleased {
            key: Key::Unidentified, modified_key: Key::Character("x".into()),
            physical_key: Physical::Code(code), location: Location::Standard,
            modifiers: Modifiers::ALT,
        }), Status::Captured)
    }

    #[test]
    fn file_shortcut_router_exact_platform_modifiers_and_refused_downs_hold() {
        let foreign = if cfg!(target_os = "macos") { Modifiers::CTRL } else { Modifiers::LOGO };
        for copy in [false, true] {
            for rejected in [Modifiers::NONE, foreign, primary_modifier() | foreign,
                modifiers(copy) | Modifiers::ALT, modifiers(copy) | foreign]
            {
                let mut state = State::default();
                let now = context(4, false, false);
                assert_eq!(state.route(input(down(copy, rejected, false), Status::Ignored), 4, now), Decision::Ignore);
                assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 4, now), Decision::Ignore);
                state.route(up(if copy { Code::KeyS } else { Code::KeyO }), 4, now);
                assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 4, now), expected(copy));
            }
            // Shift is an extra modifier for Load; plain primary S retains Save.
            let mut state = State::default();
            assert_eq!(state.route(input(down(copy, primary_modifier(), false), Status::Ignored), 0,
                context(0, false, false)), if copy { Decision::Save } else { Decision::Load });
        }
        let mut state = State::default();
        assert_eq!(state.route(input(down(false, primary_modifier() | Modifiers::SHIFT, false), Status::Ignored), 0,
            context(0, false, false)), Decision::Ignore);
        let primary = if cfg!(target_os = "macos") { "Command" } else { "Ctrl" };
        assert!(file_guidance().contains(&format!("{primary}+O")));
        assert!(file_guidance().contains(&format!("{primary}+Shift+S")));
    }

    #[test]
    fn file_shortcut_router_shared_s_hold_never_converts_save_copy_or_plain_key() {
        let now = context(1, false, false);
        for first in [Modifiers::NONE, primary_modifier(), modifiers(true)] {
            let mut state = State::default();
            let initial = state.route(input(down(true, first, false), Status::Ignored), 1, now);
            assert_eq!(initial, if first.is_empty() { Decision::Ignore }
                else if first == primary_modifier() { Decision::Save } else { Decision::SaveCopy });
            for next in [primary_modifier(), modifiers(true), Modifiers::NONE] {
                for repeat in [true, false] {
                    assert_eq!(state.route(input(down(true, next, repeat), Status::Ignored), 1, now), Decision::Ignore);
                }
            }
            state.route(up(Code::KeyS), 1, now);
            assert_eq!(state.route(input(down(true, modifiers(true), false), Status::Ignored), 1, now), Decision::SaveCopy);
        }
    }

    #[test]
    fn file_shortcut_router_capture_repeat_modal_blocked_focus_generation_consume() {
        for copy in [false, true] {
            for refusal in ["captured", "repeat", "modal", "blocked", "unfocused", "stale"] {
                let mut state = State::default();
                let now = context(5, refusal == "modal", refusal == "blocked");
                if refusal == "unfocused" { state.route(Input::FocusLost, 5, now); }
                assert_eq!(state.route(input(down(copy, modifiers(copy), refusal == "repeat"),
                    if refusal == "captured" { Status::Captured } else { Status::Ignored }),
                    if refusal == "stale" { 4 } else { 5 }, now), Decision::Ignore);
                let current = context(6, false, false);
                state.route(Input::FocusLost, 5, current);
                state.route(Input::FocusGained, 5, current);
                state.route(up(if copy { Code::KeyS } else { Code::KeyO }), 5, current);
                assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 6, current), Decision::Ignore,
                    "stale focus gain reopened input");
                state.route(Input::FocusGained, 6, current);
                state.route(up(if copy { Code::KeyS } else { Code::KeyO }), 5, current);
                assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 6, current), Decision::Ignore,
                    "old-owner release rearmed the newer hold");
                state.route(up(if copy { Code::KeyS } else { Code::KeyO }), 6, current);
                assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 6, current), expected(copy));
            }
        }
    }

    #[test]
    fn file_shortcut_router_logical_remaps_use_either_key_and_physical_shared_release() {
        for copy in [false, true] {
            for modified_only in [false, true] {
                let mut event = down(copy, modifiers(copy), false);
                if let Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, physical_key, text, .. }) = &mut event {
                    if modified_only { *key = Key::Unidentified; } else { *modified_key = Key::Unidentified; }
                    *physical_key = Physical::Code(Code::KeyX);
                    *text = Some("unrelated text".into());
                }
                let mut state = State::default();
                let now = context(0, false, false);
                assert_eq!(state.route(input(event.clone(), Status::Ignored), 0, now), expected(copy));
                // Remap that same held physical key to the other file intent.
                let mut remap = down(!copy, modifiers(!copy), false);
                if let Event::Keyboard(keyboard::Event::KeyPressed { physical_key, .. }) = &mut remap {
                    *physical_key = Physical::Code(Code::KeyX);
                }
                state.route(up(Code::KeyO), 0, now);
                assert_eq!(state.route(input(remap.clone(), Status::Ignored), 0, now), Decision::Ignore);
                state.route(up(Code::KeyX), 0, now);
                assert_eq!(state.route(input(remap, Status::Ignored), 0, now), expected(!copy));
                if let Event::Keyboard(keyboard::Event::KeyPressed { key, modified_key, text, .. }) = &mut event {
                    *key = Key::Unidentified; *modified_key = Key::Unidentified;
                    *text = Some(if copy { "s" } else { "o" }.into());
                }
                assert!(subscription_event(event, Status::Ignored, window::Id::unique()).is_none());
            }
        }
    }

    #[test]
    fn file_shortcut_router_capacity_is_shared_saturates_and_survives_publications() {
        let mut state = State::default();
        let codes = [Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE,
            Code::KeyF, Code::KeyG, Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK,
            Code::KeyL, Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ];
        for (index, code) in codes.iter().enumerate() {
            let mut event = down(index % 2 == 0, Modifiers::NONE, false);
            if let Event::Keyboard(keyboard::Event::KeyPressed { physical_key, .. }) = &mut event {
                *physical_key = Physical::Code(*code);
            }
            assert_eq!(state.route(input(event, Status::Captured), 0, context(0, false, false)), Decision::Ignore);
        }
        for code in codes { state.route(up(code), 1, context(1, false, false)); }
        state.route(Input::FocusLost, 1, context(1, false, false));
        state.route(Input::FocusGained, 1, context(1, false, false));
        for copy in [false, true] {
            assert_eq!(state.route(input(down(copy, modifiers(copy), false), Status::Ignored), 1,
                context(1, false, false)), Decision::Ignore);
        }
        assert_eq!(state.route(input(down(true, primary_modifier(), false), Status::Ignored), 1,
            context(1, false, false)), Decision::Ignore, "saturation must still fence Save");
    }
}

fn context(generation: u64, modal: bool, blocked: bool) -> Context {
    Context {
        generation,
        modal,
        blocked,
    }
}
fn press(repeat: bool, modifiers: Modifiers) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(Named::Space),
        modified_key: Key::Named(Named::Space),
        physical_key: Physical::Code(Code::Space),
        location: Location::Standard,
        modifiers,
        text: Some(" ".into()),
        repeat,
    })
}
fn release() -> Event {
    Event::Keyboard(keyboard::Event::KeyReleased {
        key: Key::Named(Named::Space),
        modified_key: Key::Named(Named::Space),
        physical_key: Physical::Code(Code::Space),
        location: Location::Standard,
        modifiers: Modifiers::NONE,
    })
}
fn route(state: &mut State, event: &Event, status: Status, context: Context) -> Decision {
    state.route(
        Input::from_iced(event, status).unwrap(),
        context.generation,
        context,
    )
}

// Authored regression UNRUN: a held Space must not swallow fresh Save intent,
// and the Save release must not rearm the still-held Space gesture.
#[test]
fn save_shortcut_has_independent_hold_from_existing_space_and_modal_routing() {
    let mut state = State::default();
    let current = context(0, false, false);
    assert_eq!(route(&mut state, &press(false, Modifiers::NONE), Status::Ignored, current), Decision::Toggle);
    let save = Input::SavePressed { physical_key: Physical::Code(Code::KeyS), captured: false,
        primary_only: true, repeat: false };
    assert_eq!(state.route(save, 0, current), Decision::Save);
    state.route(Input::KeyReleased { physical_key: Physical::Code(Code::KeyS) }, 0, current);
    assert_eq!(route(&mut state, &press(false, Modifiers::NONE), Status::Ignored, current), Decision::Ignore);
    assert_eq!(state.route(save, 0, current), Decision::Save);
    route(&mut state, &release(), Status::Captured, current);
    assert_eq!(route(&mut state, &press(false, Modifiers::NONE), Status::Ignored, context(0, true, false)), Decision::DialogSpace);
}

#[test]
fn routing_one_press_toggles_and_both_marked_and_unmarked_repeats_wait_for_release() {
    let mut state = State::default();
    let play = context(7, false, false);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );
    for repeat in [false, true, true, false] {
        assert_eq!(
            route(
                &mut state,
                &press(repeat, Modifiers::NONE),
                Status::Ignored,
                play
            ),
            Decision::Ignore
        );
    }
    assert_eq!(
        route(&mut state, &release(), Status::Captured, play),
        Decision::Ignore
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_modal_response_cannot_turn_into_gameplay_on_dismissal_or_owner_replacement() {
    let mut state = State::default();
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            context(3, true, false)
        ),
        Decision::DialogSpace
    );
    for next in [
        context(3, false, false),
        context(4, false, true),
        context(4, false, false),
    ] {
        for repeat in [true, false] {
            assert_eq!(
                route(
                    &mut state,
                    &press(repeat, Modifiers::NONE),
                    Status::Ignored,
                    next
                ),
                Decision::Ignore
            );
        }
    }
    route(
        &mut state,
        &release(),
        Status::Captured,
        context(4, false, true),
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            context(4, false, false)
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_every_modifier_and_capture_consume_the_whole_gesture_in_play_and_modal() {
    for modal in [false, true] {
        for modifiers in [
            Modifiers::SHIFT,
            Modifiers::CTRL,
            Modifiers::ALT,
            Modifiers::LOGO,
            Modifiers::SHIFT | Modifiers::CTRL | Modifiers::ALT | Modifiers::LOGO,
        ] {
            let mut state = State::default();
            let current = context(0, modal, false);
            assert_eq!(
                route(
                    &mut state,
                    &press(false, modifiers),
                    Status::Ignored,
                    current
                ),
                Decision::Ignore
            );
            assert_eq!(
                route(
                    &mut state,
                    &press(false, Modifiers::NONE),
                    Status::Ignored,
                    current
                ),
                Decision::Ignore
            );
            route(&mut state, &release(), Status::Captured, current);
            assert_eq!(
                route(
                    &mut state,
                    &press(false, Modifiers::NONE),
                    Status::Ignored,
                    current
                ),
                if modal {
                    Decision::DialogSpace
                } else {
                    Decision::Toggle
                }
            );
        }
        let mut state = State::default();
        let current = context(0, modal, false);
        assert_eq!(
            route(
                &mut state,
                &press(false, Modifiers::NONE),
                Status::Captured,
                current
            ),
            Decision::Ignore
        );
        assert_eq!(
            route(
                &mut state,
                &press(false, Modifiers::NONE),
                Status::Ignored,
                current
            ),
            Decision::Ignore
        );
        route(&mut state, &release(), Status::Captured, current);
        assert_eq!(
            route(
                &mut state,
                &press(false, Modifiers::NONE),
                Status::Ignored,
                current
            ),
            if modal {
                Decision::DialogSpace
            } else {
                Decision::Toggle
            }
        );
    }
}

#[test]
fn routing_overlay_press_and_release_are_observed_even_when_commands_are_blocked() {
    let mut state = State::default();
    let blocked = context(0, true, true);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            blocked
        ),
        Decision::Ignore
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            context(0, false, false)
        ),
        Decision::Ignore
    );
    route(&mut state, &release(), Status::Captured, blocked);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            context(0, false, false)
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_focus_loss_keeps_held_fence_and_orphan_repeat_does_not_resume() {
    let mut state = State::default();
    let play = context(0, false, false);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );
    route(
        &mut state,
        &Event::Window(window::Event::Unfocused),
        Status::Captured,
        play,
    );
    route(
        &mut state,
        &Event::Window(window::Event::Focused),
        Status::Captured,
        play,
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Ignore
    );
    route(&mut state, &release(), Status::Captured, play);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );

    let mut state = State::default();
    route(
        &mut state,
        &Event::Window(window::Event::Unfocused),
        Status::Captured,
        play,
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Ignore
    );
    route(&mut state, &release(), Status::Captured, play);
    route(
        &mut state,
        &Event::Window(window::Event::Focused),
        Status::Captured,
        play,
    );
    assert_eq!(
        route(
            &mut state,
            &press(true, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Ignore
    );
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Ignore
    );
    route(&mut state, &release(), Status::Captured, play);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_stale_press_and_release_cannot_trigger_or_rearm_a_new_owner_gesture() {
    let mut state = State::default();
    let current = context(9, false, false);
    let down = Input::from_iced(&press(false, Modifiers::NONE), Status::Ignored).unwrap();
    let up = Input::from_iced(&release(), Status::Captured).unwrap();
    assert_eq!(state.route(down, 8, current), Decision::Ignore);
    assert_eq!(state.route(down, 9, current), Decision::Ignore);
    // A captured old-owner release may end the old-owner hold.
    assert_eq!(state.route(up, 8, current), Decision::Ignore);
    assert_eq!(state.route(down, 9, current), Decision::Toggle);
    // That same late release cannot end a newer, distinct press.
    assert_eq!(state.route(up, 8, current), Decision::Ignore);
    assert_eq!(state.route(down, 9, current), Decision::Ignore);
    assert_eq!(state.route(up, 9, current), Decision::Ignore);
    assert_eq!(state.route(down, 9, current), Decision::Toggle);
}

#[test]
fn routing_stale_focus_gain_cannot_reopen_current_owner_input() {
    let mut state = State::default();
    let current = context(9, false, false);
    state.route(Input::FocusLost, 8, current);
    state.route(Input::FocusGained, 8, current);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            current
        ),
        Decision::Ignore
    );
    route(&mut state, &release(), Status::Captured, current);
    state.route(Input::FocusGained, 9, current);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            current
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_physical_release_survives_logical_remapping_and_unrelated_releases_do_not_rearm() {
    let mut state = State::default();
    let play = context(0, false, false);
    route(
        &mut state,
        &press(false, Modifiers::NONE),
        Status::Ignored,
        play,
    );
    let mut up = release();
    if let Event::Keyboard(keyboard::Event::KeyReleased {
        key,
        modified_key,
        physical_key,
        modifiers,
        ..
    }) = &mut up
    {
        *key = Key::Character("x".into());
        *modified_key = Key::Character("X".into());
        *physical_key = Physical::Code(Code::KeyX);
        *modifiers = Modifiers::SHIFT;
    }
    route(&mut state, &up, Status::Captured, play);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Ignore
    );
    if let Event::Keyboard(keyboard::Event::KeyReleased { physical_key, .. }) = &mut up {
        *physical_key = Physical::Code(Code::Space);
    }
    route(&mut state, &up, Status::Captured, play);
    assert_eq!(
        route(
            &mut state,
            &press(false, Modifiers::NONE),
            Status::Ignored,
            play
        ),
        Decision::Toggle
    );
}

#[test]
fn routing_actual_iced_character_space_is_supported_but_text_or_other_keys_are_not_shortcuts() {
    let mut down = press(false, Modifiers::NONE);
    if let Event::Keyboard(keyboard::Event::KeyPressed {
        key, modified_key, ..
    }) = &mut down
    {
        *key = Key::Character(" ".into());
        *modified_key = Key::Character(" ".into());
    }
    let mut state = State::default();
    assert_eq!(
        route(&mut state, &down, Status::Ignored, context(0, false, false)),
        Decision::Toggle
    );
    for other in [
        Key::Named(Named::Enter),
        Key::Named(Named::Escape),
        Key::Named(Named::ArrowLeft),
        Key::Character("x".into()),
    ] {
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = &mut down {
            *key = other;
        }
        assert!(
            Input::from_iced(&down, Status::Ignored).is_none(),
            "text alone must not act as Space"
        );
    }
    assert!(
        Input::from_iced(
            &Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::SHIFT)),
            Status::Ignored
        )
        .is_none()
    );
}

// These controls enter the ACTUAL production subscription_event and Host::update.
// The synthetic publication only holds the existing transport queue at its
// pending boundary; it is not evidence of a loaded or running household.
mod production {
    use super::super::super::{
        Args, Message, Viewer, subscription_event,
        worker_host::{Event as HostEvent, Host},
    };
    use super::*;
    use anyhow::Result;
    use clap::Parser;

    fn bounded(control: fn() -> Result<()>) -> Result<()> {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(control)?
            .join()
            .map_err(|_| anyhow::anyhow!("authored Space control panicked"))?
    }
    fn loading() -> Viewer {
        let args = Args::parse_from([
            "sims-room",
            "--paused",
            "--original",
            "UserData2/Houses/House09.iff",
        ]);
        let (viewer, task) = Viewer::new(args);
        drop(task); // Never execute the private/real loader in these controls.
        viewer
    }
    fn pending_host() -> Host {
        let mut host = Host::from_viewer_for_test(loading());
        set_boundary(&mut host, false, false, None);
        host
    }
    fn set_boundary(host: &mut Host, modal: bool, blocked: bool, error: Option<String>) {
        let mut publication = host.publication_for_test().as_ref().clone();
        publication.session_present = true;
        publication.native_transport = true;
        publication.pending = true;
        publication.modal = modal;
        publication.player_shortcuts_blocked = blocked;
        publication.error = error;
        host.install_synthetic_publication_for_test(publication);
    }
    fn send(host: &mut Host, generation: u64, message: Message) {
        drop(host.update(HostEvent::Widget {
            generation,
            message,
        }));
    }
    fn command(host: &mut Host, message: Message) {
        let generation = host.current_generation_for_test();
        send(host, generation, message);
    }
    fn subscribed(host: &mut Host, generation: u64, event: Event, status: Status) {
        let message = subscription_event(event, status, window::Id::unique())
            .expect("release/focus/Space bookkeeping must reach Host even when captured");
        send(host, generation, message);
    }
    fn current(host: &mut Host, event: Event, status: Status) {
        let generation = host.current_generation_for_test();
        subscribed(host, generation, event, status);
    }
    fn toggle_queue(host: &Host, expected: usize) {
        let queue = host.player_shortcut_queue_for_test();
        assert_eq!(
            queue.len(),
            expected,
            "unexpected additional or missing worker command: {queue:?}"
        );
        assert!(queue.iter().all(|(generation, message)| *generation
            == host.current_generation_for_test()
            && matches!(message, Message::Toggle)));
        assert!(
            !host.publication_for_test().playing,
            "pending shortcut changed playing before owner dispatch"
        );
    }

    struct ReleaseOnDrop(Option<std::sync::mpsc::SyncSender<()>>);
    impl ReleaseOnDrop {
        fn release(mut self) -> Result<()> {
            self.0.take().unwrap().send(())?;
            Ok(())
        }
    }
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            if let Some(resume) = self.0.take() {
                let _ = resume.send(());
            }
        }
    }

    #[test]
    fn production_subscription_forwards_captured_release_repeat_and_focus_before_legacy_guards() {
        for (event, status) in [
            (press(false, Modifiers::NONE), Status::Captured),
            (press(true, Modifiers::CTRL), Status::Ignored),
            (release(), Status::Captured),
            (Event::Window(window::Event::Unfocused), Status::Captured),
            (Event::Window(window::Event::Focused), Status::Captured),
        ] {
            let expected = Input::from_iced(&event, status).unwrap();
            let message = subscription_event(event, status, window::Id::unique());
            assert!(matches!(message, Some(Message::PlayerShortcut(input)) if input == expected));
        }
        // Existing non-Space dialog/camera mappings remain production mappings.
        for (key, expected) in [(Named::Enter, 13), (Named::Escape, 27)] {
            let mut event = press(false, Modifiers::NONE);
            if let Event::Keyboard(keyboard::Event::KeyPressed { key: logical, .. }) = &mut event {
                *logical = Key::Named(key);
            }
            assert!(
                matches!(subscription_event(event.clone(), Status::Ignored, window::Id::unique()),
                Some(Message::DialogKey(code)) if code == expected)
            );
            let captured = subscription_event(event, Status::Captured, window::Id::unique());
            if key == Named::Escape {
                assert!(matches!(captured, Some(Message::PickerEscape)));
            } else {
                assert!(captured.is_none());
            }
        }
    }

    #[test]
    fn production_gameplay_space_uses_existing_pending_toggle_queue_once_per_release() -> Result<()>
    {
        bounded(|| {
            let mut host = pending_host();
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            for repeat in [true, false, true] {
                current(&mut host, press(repeat, Modifiers::NONE), Status::Ignored);
            }
            current(&mut host, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 2);
            // The shortcut's pending queue must be identical to direct Toggle,
            // rather than a raw keyboard message hidden behind native gates.
            let mut direct = pending_host();
            command(&mut direct, Message::Toggle);
            command(&mut direct, Message::Toggle);
            assert_eq!(
                format!("{:?}", host.player_shortcut_queue_for_test()),
                format!("{:?}", direct.player_shortcut_queue_for_test())
            );
            Ok(())
        })
    }

    #[test]
    fn production_captured_modified_and_overlay_held_space_never_become_gameplay_commands()
    -> Result<()> {
        bounded(|| {
            let mut host = pending_host();
            for (modifiers, status) in [
                (Modifiers::NONE, Status::Captured),
                (Modifiers::SHIFT, Status::Ignored),
                (Modifiers::CTRL, Status::Ignored),
                (Modifiers::ALT, Status::Ignored),
                (Modifiers::LOGO, Status::Ignored),
            ] {
                current(&mut host, press(false, modifiers), status);
                current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
                current(&mut host, release(), Status::Captured);
                toggle_queue(&host, 0);
            }
            for overlay in [Message::Options, Message::Diagnostics] {
                command(&mut host, overlay.clone());
                current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
                command(&mut host, overlay.clone());
                current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
                toggle_queue(&host, 0);
                // Release while the shell overlay is open must still rearm.
                command(&mut host, overlay.clone());
                current(&mut host, release(), Status::Captured);
                command(&mut host, overlay);
            }
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            Ok(())
        })
    }

    #[test]
    fn production_publication_overlay_fault_and_unready_guards_do_not_drop_release() -> Result<()> {
        bounded(|| {
            for (blocked, error) in [
                (true, None),
                (false, Some("pre-existing owner fault".to_owned())),
            ] {
                let mut host = pending_host();
                set_boundary(&mut host, false, blocked, error);
                current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
                current(&mut host, release(), Status::Captured);
                toggle_queue(&host, 0);
                set_boundary(&mut host, false, false, None);
                current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
                toggle_queue(&host, 1);
            }
            let mut host = Host::from_viewer_for_test(loading());
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            assert!(host.player_shortcut_queue_for_test().is_empty());
            assert!(host.frontend_idle_for_test());
            current(&mut host, release(), Status::Captured);
            set_boundary(&mut host, false, false, None);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            Ok(())
        })
    }

    #[test]
    fn production_stale_generation_bookkeeping_cannot_toggle_or_rearm_the_new_owner() -> Result<()>
    {
        bounded(|| {
            let mut host = Host::from_viewer_for_test(loading());
            command(&mut host, Message::Restart);
            host.settle_for_test()?; // Drops returned loader effects; no load.
            assert_eq!(host.current_generation_for_test(), 1);
            // Restart is still loading. Complete its actual failure path
            // before installing the deliberately synthetic queue boundary.
            // This is generation/latch evidence, not a loaded-household claim.
            let generation = host.current_generation_for_test();
            drop(host.update(HostEvent::Effect {
                generation, message: Message::Loaded(Err("authored loader not executed".into())),
            }));
            host.settle_for_test()?;
            set_boundary(&mut host, false, false, None);
            subscribed(&mut host, 0, press(false, Modifiers::NONE), Status::Ignored);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 0);
            subscribed(&mut host, 0, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            subscribed(&mut host, 0, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            current(&mut host, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 2);
            Ok(())
        })
    }

    #[test]
    fn production_held_space_survives_actual_owner_restart_until_captured_release() -> Result<()> {
        bounded(|| {
            let mut host = Host::from_viewer_for_test(loading());
            current(&mut host, press(false, Modifiers::NONE), Status::Captured);
            command(&mut host, Message::Restart);
            host.settle_for_test()?;
            assert_eq!(host.current_generation_for_test(), 1);
            let generation = host.current_generation_for_test();
            drop(host.update(HostEvent::Effect {
                generation, message: Message::Loaded(Err("authored loader not executed".into())),
            }));
            host.settle_for_test()?;
            set_boundary(&mut host, false, false, None);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 0);
            current(&mut host, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            toggle_queue(&host, 1);
            Ok(())
        })
    }

    #[test]
    fn production_focus_bookkeeping_keeps_the_hold_and_preserves_existing_focus_lost_dispatch()
    -> Result<()> {
        bounded(|| {
            let mut host = pending_host();
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            current(
                &mut host,
                Event::Window(window::Event::Unfocused),
                Status::Captured,
            );
            current(
                &mut host,
                Event::Window(window::Event::Focused),
                Status::Captured,
            );
            for repeat in [true, false] {
                current(&mut host, press(repeat, Modifiers::NONE), Status::Ignored);
            }
            let held = host.player_shortcut_queue_for_test();
            assert!(matches!(
                held.as_slice(),
                [(0, Message::Toggle), (0, Message::FocusLost)]
            ));
            current(&mut host, release(), Status::Captured);
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            let released = host.player_shortcut_queue_for_test();
            assert!(matches!(
                released.as_slice(),
                [
                    (0, Message::Toggle),
                    (0, Message::FocusLost),
                    (0, Message::Toggle)
                ]
            ));
            assert!(!host.publication_for_test().playing);
            Ok(())
        })
    }

    #[test]
    fn production_modal_space_queues_existing_dialog_key_without_a_toggle() -> Result<()> {
        bounded(|| {
            let mut host = pending_host();
            set_boundary(&mut host, true, false, None);
            // Hold ordinary owner work so the exact subsequent production
            // queue command is inspectable before any engine handler runs.
            let (entered, resume) = host.gate_next_for_test();
            let resume = ReleaseOnDrop(Some(resume));
            command(&mut host, Message::Shift(false));
            entered.recv_timeout(std::time::Duration::from_secs(5))?;
            current(&mut host, press(false, Modifiers::NONE), Status::Ignored);
            current(&mut host, press(true, Modifiers::NONE), Status::Ignored);
            let queued = host.player_shortcut_queue_for_test();
            let publication = host.publication_for_test().clone();
            resume.release()?;
            host.settle_for_test()?;
            assert!(matches!(queued.as_slice(), [(0, Message::DialogKey(32))]));
            assert!(!publication.playing);
            let owner = host.recover_for_test();
            assert!(!owner.playing && owner.session.is_none());
            // This verifies dispatch intent. Real modal-answer/native-pause
            // effects remain the parent's loaded-session acceptance controls.
            Ok(())
        })
    }
}
