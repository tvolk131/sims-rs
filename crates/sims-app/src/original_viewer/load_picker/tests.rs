//! Authored source-only tests; ALL UNRUN by Worker A. No files/assets needed.
use super::*;

/// Test-only delivery of real runtime actions, shared by the adapter controls.
/// No executor, renderer or application operation is run by the author. UNRUN.
pub(in crate::original_viewer) fn visit_opening_task<T: 'static>(
    task: iced::Task<T>, mut deliver: impl FnMut(iced_runtime::Action<T>),
) {
    use iced::futures::{FutureExt, StreamExt};
    if let Some(mut stream) = iced_runtime::task::into_stream(task) {
        loop {
            match stream.next().now_or_never() {
                Some(Some(action)) => deliver(action),
                Some(None) => break,
                None => panic!("opening task unexpectedly waited for non-UI work"),
            }
        }
    }
}

/// Real retained UserInterface, local clipboard, unavailable pointer. This
/// intentionally never opens a native window or touches the system clipboard.
pub(in crate::original_viewer) struct FocusUi {
    renderer: iced::Renderer,
    cache: Option<iced_runtime::user_interface::Cache>,
    clipboard: FocusClipboard,
}

#[derive(Default)]
struct FocusClipboard(String);
impl Clipboard for FocusClipboard {
    fn read(&self, _: iced::advanced::clipboard::Kind) -> Option<String> { Some(self.0.clone()) }
    fn write(&mut self, _: iced::advanced::clipboard::Kind, value: String) { self.0 = value; }
}

impl FocusUi {
    pub(in crate::original_viewer) fn new() -> anyhow::Result<Self> {
        use iced::advanced::renderer::Headless;
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            iced::Font::DEFAULT, iced::Pixels(16.), Some("wgpu"),
        )).ok_or_else(|| anyhow::anyhow!("headless Iced GPU adapter absent"))?;
        Ok(Self { renderer, cache: None, clipboard: FocusClipboard::default() })
    }

    pub(in crate::original_viewer) fn task<M, T: 'static>(
        &mut self, view: Element<'_, M>, task: iced::Task<T>,
    ) -> usize {
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            view, Size::new(1000., 760.), self.cache.take().unwrap_or_default(), &mut self.renderer,
        );
        let mut count = 0;
        visit_opening_task(task, |action| match action {
            iced_runtime::Action::Widget(mut operation) => {
                ui.operate(&self.renderer, operation.as_mut());
                assert!(matches!(operation.finish(), iced::advanced::widget::operation::Outcome::None));
                count += 1;
            }
            _ => panic!("picker opening produced an application or non-widget action"),
        });
        self.cache = Some(ui.into_cache());
        count
    }

    pub(in crate::original_viewer) fn events<M>(
        &mut self, view: Element<'_, M>, events: &[iced::Event],
    ) -> Vec<M> {
        self.events_at(view, events, mouse::Cursor::Unavailable)
    }

    fn events_at<M>(
        &mut self, view: Element<'_, M>, events: &[iced::Event], cursor: mouse::Cursor,
    ) -> Vec<M> {
        let mut ui = iced_runtime::user_interface::UserInterface::build(
            view, Size::new(1000., 760.), self.cache.take().unwrap_or_default(), &mut self.renderer,
        );
        let mut messages = Vec::new();
        ui.update(events, cursor, &mut self.renderer, &mut self.clipboard, &mut messages);
        self.cache = Some(ui.into_cache());
        messages
    }

    pub(in crate::original_viewer) fn click_at<M>(
        &mut self, view: Element<'_, M>, position: iced::Point,
    ) -> Vec<M> {
        self.events_at(view, &[
            iced::Event::Mouse(mouse::Event::CursorMoved { position }),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ], mouse::Cursor::Available(position))
    }

    pub(in crate::original_viewer) fn paste<M>(&mut self, view: Element<'_, M>, value: &str) -> Vec<M> {
        use iced::keyboard::{self, key, Key, Modifiers};
        self.clipboard.0 = value.into();
        self.events(view, &[
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::COMMAND)),
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Character("v".into()), modified_key: Key::Character("v".into()),
                physical_key: key::Physical::Code(key::Code::KeyV), location: keyboard::Location::Standard,
                modifiers: Modifiers::COMMAND, text: None, repeat: false,
            }),
            iced::Event::Keyboard(keyboard::Event::KeyReleased {
                key: Key::Character("v".into()), modified_key: Key::Character("v".into()),
                physical_key: key::Physical::Code(key::Code::KeyV), location: keyboard::Location::Standard,
                modifiers: Modifiers::COMMAND,
            }),
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::NONE)),
        ])
    }
}

fn focus_key(key: iced::keyboard::Key, code: iced::keyboard::key::Code, text: Option<&str>) -> iced::Event {
    use iced::keyboard::{self, key};
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(), modified_key: key, physical_key: key::Physical::Code(code),
        location: keyboard::Location::Standard, modifiers: keyboard::Modifiers::NONE,
        text: text.map(Into::into), repeat: false,
    })
}

pub(in crate::original_viewer) fn focus_enter() -> iced::Event {
    focus_key(iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter), iced::keyboard::key::Code::Enter, None)
}

fn focus_type(value: &str) -> Vec<iced::Event> {
    value.chars().map(|c| focus_key(iced::keyboard::Key::Character(c.to_string().into()),
        iced::keyboard::key::Code::KeyA, Some(&c.to_string()))).collect()
}

// UNRUN, including compilation. Parent filter: picker_focus_ (with --ignored).
#[test]
#[ignore = "opt-in real Iced wgpu UI operations and synthetic keyboard/clipboard; UNRUN"]
fn picker_focus_widgets_replace_seed_once_enter_and_retry_both_purposes() -> anyhow::Result<()> {
    for copy in [false, true] {
        for paste in [false, true] {
            let mut picker = if copy { Picker::new_copy(Path::new("seed.json"), super::super::save_copy::Owner::new())? }
                else { Picker::new(Path::new("seed.json"))? };
            let mut ui = FocusUi::new()?;
            assert_eq!(ui.task(picker.view(true), picker.opening_task::<Event>()), 2);
            assert_eq!(picker.value(), "seed.json");
            assert!(!picker.pending() && !picker.closed);
            let expected = if paste { " ../家族//./copy e\u{301}.json " } else { "ab" };
            let events = if paste { ui.paste(picker.view(true), expected) }
                else { ui.events(picker.view(true), &focus_type(expected)) };
            assert!(!events.is_empty(), "typing required a pointer click");
            for event in events { assert!(matches!(picker.update(event), Outcome::None)); }
            assert_eq!(picker.value().as_bytes(), expected.as_bytes());
            // Repaint and an interior cursor edit must not reselect the seed.
            assert!(ui.events(picker.view(true), &[
                iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now())),
            ]).is_empty());
            let left = focus_key(iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft),
                iced::keyboard::key::Code::ArrowLeft, None);
            assert!(ui.events(picker.view(true), &[left]).is_empty());
            for event in ui.events(picker.view(true), &focus_type("!")) { picker.update(event); }
            let mut edited = expected.to_owned();
            let last = edited.char_indices().last().unwrap().0;
            edited.insert(last, '!');
            assert_eq!(picker.value(), edited);
            let mut events = ui.events(picker.view(true), &[focus_enter()]);
            assert_eq!(events.len(), 1);
            let ticket = match picker.update(events.remove(0)) {
                Outcome::Submit { path, ticket } => {
                    assert_eq!(path.as_os_str().to_str(), Some(edited.as_str()));
                    assert_eq!(ticket.purpose(), if copy { Purpose::Copy } else { Purpose::Load });
                    ticket
                }
                other => panic!("keyboard Enter did not use sealed submit: {other:?}"),
            };
            assert!(ui.events(picker.view(true), &focus_type("x")).is_empty());
            assert!(picker.settle(&ticket, Some("recoverable rejection".into())));
            // Retry neither reopens nor reselects: the interior cursor survives.
            for event in ui.events(picker.view(true), &focus_type("?")) { picker.update(event); }
            edited.insert(last + 1, '?');
            assert_eq!(picker.value(), edited);
            assert!(!picker.pending());
        }
    }
    Ok(())
}

#[test]
#[ignore = "opt-in real Iced wgpu delayed operations against replacement/unrelated fields; UNRUN"]
fn picker_focus_widgets_old_tasks_leave_new_picker_and_unrelated_field_untouched() -> anyhow::Result<()> {
    let mut old = Picker::new(Path::new("old.json"))?;
    let delayed = old.opening_task::<Event>();
    let cancel = old.cancel_event();
    assert!(matches!(old.update(cancel), Outcome::Cancelled));
    drop(old);
    let current = Picker::new_copy(Path::new("new.json"), super::super::save_copy::Owner::new())?;
    let mut ui = FocusUi::new()?;
    assert_eq!(ui.task(current.view(true), current.opening_task::<Event>()), 2);
    // Move the real current field's cursor away from its selected seed.
    assert!(ui.events(current.view(true), &[
        focus_key(iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft),
            iced::keyboard::key::Code::ArrowLeft, None),
    ]).is_empty());
    assert_eq!(ui.task(current.view(true), delayed), 2);
    let mut current = current;
    for event in ui.events(current.view(true), &focus_type("x")) { current.update(event); }
    assert_eq!(current.value(), "xnew.json", "old task stole focus or selection");

    let delayed = current.opening_task::<String>();
    drop(current);
    let unrelated = widget::Id::unique();
    let view = || text_input("unrelated", "other").id(unrelated.clone()).on_input(|s| s).into();
    assert_eq!(ui.task(view(), widget::operation::focus::<String>(unrelated.clone())), 1);
    assert_eq!(ui.task(view(), widget::operation::move_cursor_to_front::<String>(unrelated.clone())), 1);
    assert_eq!(ui.task(view(), delayed), 2);
    assert_eq!(ui.events(view(), &focus_type("x")), ["xother"]);
    Ok(())
}

fn edit(picker: &mut Picker, value: &str) {
    let event = picker.editor().changed(value.into());
    assert!(matches!(picker.update(event), Outcome::None));
}

fn submit(picker: &mut Picker) -> (PathBuf, Ticket) {
    let event = picker.event(Intent::Submit);
    match picker.update(event) {
        Outcome::Submit { path, ticket } => (path, ticket),
        other => panic!("expected a path request, got {other:?}"),
    }
}

// UNRUN. These controls authenticate choices; actual successful loads are
// exercised separately by worker_host::wiring_tests::recent_load_controls.
#[test]
fn recent_loads_picker_snapshot_exact_choice_stale_revision_pending_and_copy() -> anyhow::Result<()> {
    let mut history = super::super::recent_loads::History::default();
    history.record(Path::new("./a//家族.json"));
    history.record(Path::new("b.json"));
    let mut picker = Picker::new(Path::new("typed.json"))?.with_recent(&history);
    let id = picker.input_id.clone();
    let choice = picker.recent_event_for_test(1);
    assert!(choice.is_recent_choice());
    assert!(matches!(picker.update(choice.clone()), Outcome::FocusPath));
    assert_eq!(picker.value(), "./a//家族.json");
    assert_eq!(picker.input_id, id);
    assert!(!picker.pending());
    assert!(matches!(picker.update(choice), Outcome::None));
    history.record(Path::new("replacement.json"));
    assert_eq!(picker.recent.paths()[0].as_os_str(), "b.json", "live history changed a picker snapshot");
    let stale = picker.recent_event_for_test(0);
    edit(&mut picker, "manual.json");
    assert!(matches!(picker.update(stale), Outcome::None));
    assert_eq!(picker.value(), "manual.json");
    let choice = picker.recent_event_for_test(0);
    let (_, ticket) = submit(&mut picker);
    assert!(matches!(picker.update(choice), Outcome::None));
    assert!(picker.settle(&ticket, Some("retry".into())));
    let foreign = picker.recent_event_for_test(0);
    let mut replacement = Picker::new(Path::new("new.json"))?.with_recent(&history);
    assert!(matches!(replacement.update(foreign), Outcome::None));
    assert_eq!(replacement.value(), "new.json");
    let mut copy = Picker::new_copy(Path::new("copy.json"), super::super::save_copy::Owner::new())?
        .with_recent(&history);
    assert!(copy.recent.paths().is_empty());
    let choice = copy.recent_event_for_test(0);
    assert!(matches!(copy.update(choice), Outcome::None));
    assert_eq!(copy.value(), "copy.json");
    Ok(())
}

#[test]
fn load_picker_state_initial_and_submitted_spelling_is_exact() -> anyhow::Result<()> {
    for value in [
        "./saves//../household.json",
        " leading and trailing .json ",
        "../résumé/家族.json",
        "./e\u{301}.json",
        "relative\\literal.json",
        " ",
        ".",
    ] {
        let mut picker = Picker::new(Path::new(value))?;
        assert_eq!(picker.value(), value);
        let (path, _) = submit(&mut picker);
        // Path equality can normalize components; compare the OS spelling.
        assert_eq!(path.as_os_str().to_str(), Some(value));
        assert_eq!(picker.value().as_bytes(), value.as_bytes());
        assert!(picker.pending());
    }
    Ok(())
}

#[test]
fn load_picker_state_edited_path_preserves_every_utf8_byte() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("initial.json"))?;
    let value = " ../家族//./save e\u{301}.json ";
    edit(&mut picker, value);
    let (path, _) = submit(&mut picker);
    assert_eq!(path.as_os_str().to_str().unwrap().as_bytes(), value.as_bytes());
    assert_eq!(picker.value(), value);
    Ok(())
}

#[cfg(unix)]
#[test]
fn load_picker_state_non_utf8_initial_path_is_an_error() {
    use std::os::unix::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_vec(b"bad-\xff.json".to_vec()));
    let error = Picker::new(&path).err().expect("lossy path accepted");
    assert!(error.to_string().contains("represented exactly"));
    assert!(error.to_string().contains("UTF-8"));
}

#[cfg(windows)]
#[test]
fn load_picker_state_non_utf8_initial_path_is_an_error() {
    use std::os::windows::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[0xd800]));
    assert!(Picker::new(&path).is_err());
}

#[test]
fn load_picker_state_empty_and_nul_are_rejected_with_feedback() -> anyhow::Result<()> {
    for value in ["", "bad\0.json", "\0"] {
        let mut picker = Picker::new(Path::new("valid.json"))?;
        edit(&mut picker, value);
        let event = picker.event(Intent::Submit);
        let repeated = event.clone();
        assert!(matches!(picker.update(event), Outcome::None));
        assert!(!picker.pending());
        assert_eq!(picker.value(), value);
        let feedback = picker.feedback.clone().expect("validation feedback absent");
        assert!(feedback.contains(if value.is_empty() { "Enter" } else { "NUL" }));
        assert!(matches!(picker.update(repeated), Outcome::None));
        assert_eq!(picker.feedback.as_deref(), Some(feedback.as_str()));
        edit(&mut picker, "repaired.json");
        assert!(picker.feedback.is_none());
        assert_eq!(submit(&mut picker).0.as_os_str(), "repaired.json");
    }
    Ok(())
}

#[test]
fn load_picker_state_empty_initial_field_can_be_edited() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new(""))?;
    let event = picker.event(Intent::Submit);
    assert!(matches!(picker.update(event), Outcome::None));
    assert!(picker.feedback.is_some());
    edit(&mut picker, "new.json");
    assert_eq!(submit(&mut picker).0.as_os_str(), "new.json");
    Ok(())
}

#[test]
fn load_picker_state_old_submit_and_cancel_cannot_act_on_changed_text() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a.json"))?;
    let submit_a = picker.event(Intent::Submit);
    let cancel_a = picker.cancel_event();
    edit(&mut picker, "b.json");
    assert!(matches!(picker.update(submit_a), Outcome::None));
    assert!(matches!(picker.update(cancel_a), Outcome::None));
    assert!(!picker.pending());
    assert!(!picker.closed);
    assert_eq!(submit(&mut picker).0.as_os_str(), "b.json");
    Ok(())
}

#[test]
fn load_picker_state_batched_edits_chain_but_old_enter_does_not() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new(""))?;
    let editor = picker.editor();
    let old_enter = picker.event(Intent::Submit);
    let events = [editor.changed("a".into()), editor.changed("ab".into()), editor.changed("abc".into())];
    for event in events {
        assert!(matches!(picker.update(event), Outcome::None));
    }
    assert_eq!(picker.value(), "abc");
    assert!(matches!(picker.update(old_enter), Outcome::None));
    assert!(!picker.pending());
    assert_eq!(submit(&mut picker).0.as_os_str(), "abc");
    Ok(())
}

#[test]
fn load_picker_state_equal_value_edit_does_not_strand_the_next_edit() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a"))?;
    let editor = picker.editor();
    let no_op = editor.changed("a".into());
    let next = editor.changed("ab".into());
    assert!(matches!(picker.update(no_op), Outcome::None));
    assert!(matches!(picker.update(next), Outcome::None));
    assert_eq!(picker.value(), "ab");
    Ok(())
}

#[test]
fn load_picker_state_competing_stale_editor_and_duplicate_edit_are_ignored() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a"))?;
    let old_editor = picker.editor();
    let current_editor = picker.editor();
    let accepted = current_editor.changed("ab".into());
    let duplicate = accepted.clone();
    assert!(matches!(picker.update(accepted), Outcome::None));
    assert!(matches!(picker.update(old_editor.changed("stale".into())), Outcome::None));
    assert!(matches!(picker.update(duplicate), Outcome::None));
    assert_eq!(picker.value(), "ab");
    assert!(matches!(picker.update(current_editor.changed("abc".into())), Outcome::None));
    assert_eq!(picker.value(), "abc");
    Ok(())
}

#[test]
fn load_picker_state_out_of_order_edits_fail_closed() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new(""))?;
    let editor = picker.editor();
    let first = editor.changed("a".into());
    let second = editor.changed("ab".into());
    assert!(matches!(picker.update(second), Outcome::None));
    assert_eq!(picker.value(), "");
    assert!(matches!(picker.update(first), Outcome::None));
    assert_eq!(picker.value(), "a");
    Ok(())
}

#[test]
fn load_picker_state_all_event_kinds_are_private_to_their_picker() -> anyhow::Result<()> {
    let source = Picker::new(Path::new("same.json"))?;
    let mut target = Picker::new(Path::new("same.json"))?;
    let foreign = [
        source.editor().changed("foreign.json".into()),
        source.event(Intent::Submit),
        source.cancel_event(),
    ];
    drop(source);
    for event in foreign {
        assert!(matches!(target.update(event), Outcome::None));
        assert_eq!(target.value(), "same.json");
        assert!(!target.pending());
        assert!(!target.closed);
    }
    assert_eq!(submit(&mut target).0.as_os_str(), "same.json");
    Ok(())
}

#[test]
fn load_picker_state_pending_refuses_double_submit_edit_and_escape() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a.json"))?;
    let event = picker.event(Intent::Submit);
    let repeated = event.clone();
    let ticket = match picker.update(event) {
        Outcome::Submit { ticket, .. } => ticket,
        other => panic!("{other:?}"),
    };
    let fresh_submit = picker.event(Intent::Submit);
    let fresh_cancel = picker.cancel_event();
    let fresh_edit = picker.editor().changed("b.json".into());
    for event in [repeated, fresh_submit, fresh_cancel, fresh_edit] {
        assert!(matches!(picker.update(event), Outcome::None));
        assert!(picker.pending());
        assert_eq!(picker.value(), "a.json");
    }
    assert!(picker.settle(&ticket.clone(), Some("Read rejected".into())));
    Ok(())
}

#[test]
fn load_picker_state_foreign_ticket_does_not_settle_same_path() -> anyhow::Result<()> {
    let mut first = Picker::new(Path::new("same.json"))?;
    let mut second = Picker::new(Path::new("same.json"))?;
    let (_, first_ticket) = submit(&mut first);
    let (_, second_ticket) = submit(&mut second);
    assert!(!second.settle(&first_ticket, None));
    assert!(!second.settle(&first_ticket, Some("foreign rejection".into())));
    assert!(second.pending());
    assert!(!second.closed);
    assert!(second.feedback.is_none());
    assert!(second.settle(&second_ticket, Some("Own rejection".into())));
    assert!(first.pending());
    Ok(())
}

#[test]
fn load_picker_state_error_retry_retains_path_and_rejects_old_attempt() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new(" ./same.json "))?;
    let (_, first) = submit(&mut picker);
    let pending_submit = picker.event(Intent::Submit);
    let pending_cancel = picker.cancel_event();
    assert!(picker.settle(&first, Some("Malformed JSON; current household retained".into())));
    assert_eq!(picker.value(), " ./same.json ");
    assert!(!picker.pending());
    assert!(picker.controls_enabled(true));
    assert!(picker.feedback.as_deref().unwrap().contains("Malformed JSON"));
    assert!(matches!(picker.update(pending_submit), Outcome::None));
    assert!(matches!(picker.update(pending_cancel), Outcome::None));
    let (path, second) = submit(&mut picker);
    assert_eq!(path.as_os_str(), " ./same.json ");
    assert!(picker.feedback.is_none());
    assert!(!picker.settle(&first, None));
    assert!(!picker.settle(&first, Some("late first rejection".into())));
    assert!(picker.pending());
    assert!(picker.feedback.is_none());
    assert!(picker.settle(&second, Some("Missing file".into())));
    assert!(!picker.settle(&second, None));
    edit(&mut picker, "retry.json");
    assert!(picker.feedback.is_none());
    assert_eq!(submit(&mut picker).0.as_os_str(), "retry.json");
    Ok(())
}

#[test]
fn load_picker_state_success_is_terminal_and_cannot_settle_twice() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a.json"))?;
    let (_, ticket) = submit(&mut picker);
    assert!(picker.settle(&ticket, None));
    assert!(!picker.pending());
    assert!(!picker.controls_enabled(true));
    assert!(!picker.settle(&ticket, Some("late error".into())));
    let submit = picker.event(Intent::Submit);
    let cancel = picker.cancel_event();
    let edit = picker.editor().changed("b.json".into());
    for event in [submit, cancel, edit] {
        assert!(matches!(picker.update(event), Outcome::None));
    }
    assert_eq!(picker.value(), "a.json");
    Ok(())
}

#[test]
fn load_picker_state_cancel_before_submit_returns_no_path_and_is_terminal() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a.json"))?;
    let event = picker.cancel_event();
    let duplicate = event.clone();
    assert!(matches!(picker.update(event), Outcome::Cancelled));
    assert!(matches!(picker.update(duplicate), Outcome::None));
    let submit = picker.event(Intent::Submit);
    assert!(matches!(picker.update(submit), Outcome::None));
    assert_eq!(picker.value(), "a.json");
    assert!(!picker.pending());
    assert!(!picker.controls_enabled(true));
    Ok(())
}

#[test]
fn load_picker_state_keys_are_stable_on_repaint_and_change_on_edit_attempt_and_settlement() -> anyhow::Result<()> {
    let mut picker = Picker::new(Path::new("a.json"))?;
    let initial_seal = picker.seal.clone();
    let input_id = picker.input_id.clone();
    drop(picker.view(true));
    drop(picker.view(true));
    assert_eq!(picker.seal, initial_seal);
    assert_eq!(picker.input_id, input_id);
    assert!(!picker.controls_enabled(false));
    edit(&mut picker, "b.json");
    assert_ne!(picker.seal, initial_seal);
    let edited_seal = picker.seal.clone();
    let (_, ticket) = submit(&mut picker);
    assert_ne!(picker.seal, edited_seal);
    assert!(!picker.controls_enabled(true));
    let pending_seal = picker.seal.clone();
    assert!(picker.settle(&ticket, Some("retry".into())));
    assert_ne!(picker.seal, pending_seal);
    assert_eq!(picker.input_id, input_id);
    assert!(picker.controls_enabled(true));
    Ok(())
}

#[test]
fn load_picker_state_dispatch_data_is_send_and_debug_clone() {
    fn send<T: Send>() {}
    fn debug_clone<T: std::fmt::Debug + Clone>() {}
    send::<Picker>();
    send::<Event>();
    send::<Ticket>();
    send::<Outcome>();
    debug_clone::<Event>();
    debug_clone::<Ticket>();
}

// Use the existing source-owned Iced driver without requiring a new parent
// Message variant just to compile these tests. The map retains the exact sealed
// event in a test-local sink and emits an existing Message::Load as a sentinel.
// Driver messages are NEVER dispatched to Viewer/Host or an engine.
mod synthetic {
    use super::*;
    use super::super::super::{Message, headless_input};
    use iced::{keyboard::{self, key}, mouse, Point};
    use std::sync::Mutex;

    const SIZE: [u32; 2] = [700, 400];

    struct Widgets {
        input: headless_input::Input,
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Widgets {
        fn new() -> anyhow::Result<Self> {
            Ok(Self { input: headless_input::Input::new()?, events: Arc::new(Mutex::new(Vec::new())) })
        }

        fn view<'a>(&self, picker: &'a Picker, enabled: bool) -> Element<'a, Message> {
            let sink = self.events.clone();
            picker.view(enabled).map(move |event| {
                sink.lock().unwrap().push(event);
                Message::Load
            })
        }

        fn batch(&mut self, picker: &Picker, enabled: bool, events: &[iced::Event]) -> Vec<Event> {
            self.input.element_batch(self.view(picker, enabled), SIZE, events);
            std::mem::take(&mut *self.events.lock().unwrap())
        }

        fn batch_at(&mut self, picker: &Picker, size: [u32; 2], events: &[iced::Event]) -> Vec<Event> {
            self.input.element_batch(self.view(picker, true), size, events);
            std::mem::take(&mut *self.events.lock().unwrap())
        }

        fn wheel_at(&mut self, picker: &Picker, size: [u32; 2], point: Point, y: f32) {
            // These are distinct outer/list gestures. End the previous real
            // Iced scroll transaction via input, without sleeping or editing state.
            let emitted = self.batch_at(picker, size, &[
                iced::Event::Mouse(mouse::Event::CursorLeft),
                motion(point),
                iced::Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { x: 0., y },
                }),
            ]);
            assert!(emitted.is_empty(), "scrolling must not choose or submit a path");
        }

        fn outer_to_y(&mut self, picker: &Picker, size: [u32; 2], content_y: f32) {
            let batch = self.input.element_batch(self.view(picker, true), size, &[]);
            let outer = batch.owners.scrolls.first().expect("whole-picker scroll owner missing");
            assert!(outer.content[3] > outer.bounds[3], "short form did not overflow");
            // Wheel outside the nested list so it cannot consume this outer scroll.
            let mut y = outer.bounds[1] + 1.;
            if let Some(inner) = batch.owners.scrolls.get(1) {
                let top = inner.bounds[1] - outer.translation[1];
                let bottom = top + inner.bounds[3];
                if y >= top && y <= bottom { y = outer.bounds[1] + outer.bounds[3] - 1.; }
                assert!(y < top || y > bottom);
            }
            let point = Point::new(outer.bounds[0] + outer.bounds[2] / 2., y);
            let delta = content_y - outer.translation[1] - (outer.bounds[1] + outer.bounds[3] / 2.);
            self.wheel_at(picker, size, point, -delta);
        }


        fn button(&mut self, picker: &Picker, enabled: bool, label: &str) -> anyhow::Result<Point> {
            let observed = self.input.element_observation(self.view(picker, enabled), SIZE)?;
            let bounds = observed.texts.iter().find(|text| text.text == label)
                .ok_or_else(|| anyhow::anyhow!("button label absent: {label}"))?.bounds;
            Ok(bounds.center())
        }

        fn field(&mut self, picker: &Picker, enabled: bool) -> Point {
            let batch = self.input.element_batch(self.view(picker, enabled), SIZE, &[]);
            let [x, y, w, h] = batch.owners.focused.first().expect("text field absent").0;
            Point::new(x + w / 2., y + h / 2.)
        }

        fn click(&mut self, picker: &Picker, enabled: bool, point: Point) -> Vec<Event> {
            self.batch(picker, enabled, &[motion(point), pointer(true), pointer(false)])
        }
    }

    fn motion(position: Point) -> iced::Event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position })
    }

    fn pointer(down: bool) -> iced::Event {
        iced::Event::Mouse(if down {
            mouse::Event::ButtonPressed(mouse::Button::Left)
        } else {
            mouse::Event::ButtonReleased(mouse::Button::Left)
        })
    }

    fn key(key: keyboard::Key, code: key::Code, text: Option<&str>) -> iced::Event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(), modified_key: key,
            physical_key: key::Physical::Code(code), location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(), text: text.map(Into::into), repeat: false,
        })
    }

    fn enter() -> iced::Event {
        key(keyboard::Key::Named(key::Named::Enter), key::Code::Enter, None)
    }

    #[test]
    #[ignore = "opt-in real Iced wgpu retained recent buttons and bounded layout; UNRUN"]
    fn recent_loads_widgets_held_choice_repaint_reorder_replacement_disable_and_layout() -> anyhow::Result<()> {
        let mut history = super::super::super::recent_loads::History::default();
        for i in 0..8 { history.record(Path::new(&format!("{i}{}", "x".repeat(300)))); }
        // A short visible newest entry supplies a stable hit rectangle while
        // the remaining long entries must remain inside the short scroll area.
        history.record(Path::new("visible.json"));
        let mut picker = Picker::new(Path::new("manual.json"))?.with_recent(&history);
        let mut widgets = Widgets::new()?;
        let point = widgets.button(&picker, true, "visible.json")?;
        assert!(widgets.batch(&picker, true, &[motion(point), pointer(true)]).is_empty());
        assert!(widgets.batch(&picker, true, &[]).is_empty());
        let mut events = widgets.batch(&picker, true, &[pointer(false)]);
        assert_eq!(events.len(), 1, "ordinary repaint dropped a real held choice");
        assert!(matches!(picker.update(events.remove(0)), Outcome::FocusPath));
        assert_eq!(picker.value(), "visible.json");
        assert!(!picker.pending());
        let point = widgets.button(&picker, true, "visible.json")?;
        assert!(widgets.batch(&picker, true, &[motion(point), pointer(true)]).is_empty());
        history.record(Path::new("other.json"));
        picker = Picker::new(Path::new("replacement.json"))?.with_recent(&history);
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty(), "held click selected a replacement entry");
        assert_eq!(picker.value(), "replacement.json");
        let point = widgets.button(&picker, true, "other.json")?;
        assert!(widgets.batch(&picker, true, &[motion(point), pointer(true)]).is_empty());
        assert!(widgets.batch(&picker, false, &[]).is_empty());
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty());
        let shown = widgets.input.element_observation(widgets.view(&picker, true), [700, 600])?;
        for label in ["Path (used exactly as entered)", "Load selected file", "Cancel"] {
            let bounds = shown.texts.iter().find(|t| t.text == label).expect("manual controls absent").bounds;
            assert!(bounds.y >= 0. && bounds.y + bounds.height <= 600., "long history displaced {label}");
        }
        Ok(())
    }

    #[test]
    #[ignore = "opt-in headless Iced wgpu GPU adapter; no assets; Worker A leaves UNRUN"]
    fn load_picker_synthetic_widgets_typing_enter_pending_retry_and_cancel() -> anyhow::Result<()> {
        let mut picker = Picker::new(Path::new(""))?;
        let mut widgets = Widgets::new()?;
        let field = widgets.field(&picker, true);
        assert!(widgets.click(&picker, true, field).is_empty());
        let events = widgets.batch(&picker, true, &[
            key(keyboard::Key::Character("a".into()), key::Code::KeyA, Some("a")),
            key(keyboard::Key::Character("b".into()), key::Code::KeyB, Some("b")),
            enter(),
        ]);
        assert!(events.len() >= 2, "real text-input callbacks absent");
        for event in events {
            // Enter from the pre-edit view must not select the new path.
            assert!(matches!(picker.update(event), Outcome::None));
        }
        assert_eq!(picker.value(), "ab");
        let focus = widgets.input.element_batch(widgets.view(&picker, true), SIZE, &[]);
        assert!(focus.owners.focused.iter().any(|(_, focused)| *focused), "typing lost focus on repaint");
        let mut events = widgets.batch(&picker, true, &[enter()]);
        assert_eq!(events.len(), 1);
        let ticket = match picker.update(events.remove(0)) {
            Outcome::Submit { path, ticket } => {
                assert_eq!(path.as_os_str(), "ab");
                ticket
            }
            other => panic!("real Enter failed to submit: {other:?}"),
        };
        let cancel = widgets.button(&picker, true, "Cancel")?;
        assert!(widgets.click(&picker, true, cancel).is_empty());
        assert!(widgets.batch(&picker, true, &[
            key(keyboard::Key::Character("c".into()), key::Code::KeyC, Some("c")), enter(),
        ]).is_empty());
        assert_eq!(picker.value(), "ab");
        assert!(picker.settle(&ticket, Some("Missing saved household JSON".into())));
        let observed = widgets.input.element_observation(widgets.view(&picker, true), SIZE)?;
        assert!(observed.texts.iter().any(|text| text.text == "Missing saved household JSON"));
        let load = widgets.button(&picker, true, "Load selected file")?;
        let mut retry = widgets.click(&picker, true, load);
        assert_eq!(retry.len(), 1);
        let retry_ticket = match picker.update(retry.remove(0)) {
            Outcome::Submit { ticket, .. } => ticket,
            other => panic!("real retry failed: {other:?}"),
        };
        assert!(!picker.settle(&ticket, None));
        assert!(picker.settle(&retry_ticket, Some("Still missing".into())));
        let cancel = widgets.button(&picker, true, "Cancel")?;
        let mut events = widgets.click(&picker, true, cancel);
        assert_eq!(events.len(), 1);
        assert!(matches!(picker.update(events.remove(0)), Outcome::Cancelled));
        assert_eq!(picker.value(), "ab");
        Ok(())
    }

    #[test]
    #[ignore = "opt-in headless Iced wgpu GPU adapter; retained button cache; Worker A leaves UNRUN"]
    fn load_picker_synthetic_widgets_held_submit_repaint_edit_and_new_picker() -> anyhow::Result<()> {
        let mut picker = Picker::new(Path::new("a.json"))?;
        let mut widgets = Widgets::new()?;
        let load = widgets.button(&picker, true, "Load selected file")?;
        assert!(widgets.batch(&picker, true, &[motion(load), pointer(true)]).is_empty());
        // A normal repaint must preserve a legitimate held press.
        widgets.input.element_observation(widgets.view(&picker, true), SIZE)?;
        let mut events = widgets.batch(&picker, true, &[pointer(false)]);
        assert_eq!(events.len(), 1);
        let ticket = match picker.update(events.remove(0)) {
            Outcome::Submit { ticket, .. } => ticket,
            other => panic!("repaint erased held submit: {other:?}"),
        };
        assert!(picker.settle(&ticket, Some("retry".into())));
        let load = widgets.button(&picker, true, "Load selected file")?;
        assert!(widgets.batch(&picker, true, &[motion(load), pointer(true)]).is_empty());
        edit(&mut picker, "b.json");
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty(), "held submit transferred to edited path");
        assert!(!picker.pending());
        let load = widgets.button(&picker, true, "Load selected file")?;
        assert!(widgets.batch(&picker, true, &[motion(load), pointer(true)]).is_empty());
        picker = Picker::new(Path::new("b.json"))?;
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty(), "held submit transferred to new picker");
        assert!(widgets.batch(&picker, true, &[motion(load), pointer(true)]).is_empty());
        assert!(widgets.batch(&picker, false, &[]).is_empty());
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty(), "disabled control retained a press");
        Ok(())
    }

    #[test]
    #[ignore = "opt-in headless Iced wgpu GPU adapter; focus/resize batch controls; Worker A leaves UNRUN"]
    fn load_picker_synthetic_widgets_disabled_controls_and_immediate_focus_resize() -> anyhow::Result<()> {
        let mut picker = Picker::new(Path::new("a.json"))?;
        let mut widgets = Widgets::new()?;
        for label in ["Load selected file", "Cancel"] {
            let point = widgets.button(&picker, false, label)?;
            assert!(widgets.click(&picker, false, point).is_empty());
        }
        let field = widgets.field(&picker, true);
        assert!(widgets.click(&picker, false, field).is_empty());
        assert!(widgets.batch(&picker, false, &[
            key(keyboard::Key::Character("b".into()), key::Code::KeyB, Some("b")), enter(),
        ]).is_empty());
        for interrupt in [
            iced::Event::Window(window::Event::Unfocused),
            iced::Event::Window(window::Event::Resized(Size::new(700., 400.))),
        ] {
            for label in ["Load selected file", "Cancel"] {
                let point = widgets.button(&picker, true, label)?;
                assert!(widgets.batch(&picker, true, &[
                    motion(point), pointer(true), interrupt.clone(), pointer(false),
                ]).is_empty(), "release survived focus/resize within one batch");
            }
        }
        let cancel = widgets.button(&picker, true, "Cancel")?;
        let mut events = widgets.click(&picker, true, cancel);
        assert_eq!(events.len(), 1);
        assert!(matches!(picker.update(events.remove(0)), Outcome::Cancelled));
        assert_eq!(picker.value(), "a.json");
        Ok(())
    }

    #[test]
    #[ignore = "opt-in headless Iced wgpu; Copy held/captured/focus controls; ALL UNRUN"]
    fn save_copy_widgets_held_press_focus_and_mixed_picker_replacement_are_refused() -> anyhow::Result<()> {
        let owner = super::super::super::save_copy::Owner::new();
        let mut picker = Picker::new_copy(Path::new("copy.json"), owner.clone())?;
        let mut widgets = Widgets::new()?;
        // The heading and submit intentionally share the player action text;
        // select the actual button's final label in layout traversal order.
        let button = |widgets: &mut Widgets, picker: &Picker| -> anyhow::Result<Point> {
            let shown = widgets.input.element_observation(widgets.view(picker, true), SIZE)?;
            Ok(shown.texts.iter().rev().find(|t| t.text == "Save a Copy…").unwrap().bounds.center())
        };
        let submit = button(&mut widgets, &picker)?;
        assert!(widgets.batch(&picker, true, &[motion(submit), pointer(true)]).is_empty());
        picker.focus_lost();
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty());
        assert!(!picker.pending());
        let submit = button(&mut widgets, &picker)?;
        assert!(widgets.batch(&picker, true, &[motion(submit), pointer(true),
            iced::Event::Window(window::Event::Unfocused), pointer(false)]).is_empty());
        let submit = button(&mut widgets, &picker)?;
        assert!(widgets.batch(&picker, true, &[motion(submit), pointer(true)]).is_empty());
        // A Load field at identical text cannot adopt a held Copy press.
        picker = Picker::new(Path::new("copy.json"))?;
        assert!(widgets.batch(&picker, true, &[pointer(false)]).is_empty());
        assert!(!picker.pending());
        Ok(())
    }

    #[test]
    #[ignore = "opt-in retained Iced short-window scrolling, field entry, Enter and Cancel"]
    fn recent_loads_widgets_short_windows_reach_manual_controls_and_oldest_path() -> anyhow::Result<()> {
        let mut history = super::super::super::recent_loads::History::default();
        for index in 0..8 {
            history.record(Path::new(&format!("recent/{index}/{}household.json", "long folder name/".repeat(8))));
        }
        let oldest = history.paths().last().unwrap().to_str().unwrap().to_owned();
        for size in [[700, 400], [700, 240]] {
            let mut ui = Widgets::new()?;
            let mut picker = Picker::new(Path::new("manual.json"))?.with_recent(&history);
            let (_, retry) = submit(&mut picker);
            assert!(picker.settle(&retry, Some("Load refused; correct the exact path and retry.\n".repeat(12))));
            let batch = ui.input.element_batch(ui.view(&picker, true), size, &[]);
            assert_eq!(batch.owners.scrolls.len(), 2);
            let field = batch.owners.focused.first().expect("manual field missing").0;
            ui.outer_to_y(&picker, size, field[1] + field[3] / 2.);
            let batch = ui.input.element_batch(ui.view(&picker, true), size, &[]);
            let outer = &batch.owners.scrolls[0];
            let field = batch.owners.focused[0].0;
            let point = Point::new(field[0] + field[2] / 2., field[1] + field[3] / 2. - outer.translation[1]);
            assert!(point.y > outer.bounds[1] && point.y < outer.bounds[1] + outer.bounds[3]);
            assert!(ui.batch_at(&picker, size, &[motion(point), pointer(true), pointer(false)]).is_empty());
            let edits = ui.batch_at(&picker, size, &[key(keyboard::Key::Character("x".into()), key::Code::KeyX, Some("x"))]);
            assert_eq!(edits.len(), 1, "visible manual field must accept actual typing");
            for event in edits { assert!(matches!(picker.update(event), Outcome::None)); }
            assert!(picker.value().contains('x'));
            let expected_path = PathBuf::from(picker.value());
            let mut enter = ui.batch_at(&picker, size, &[super::focus_enter()]);
            assert_eq!(enter.len(), 1);
            let ticket = match picker.update(enter.remove(0)) {
                Outcome::Submit { path, ticket } => { assert_eq!(path, expected_path); ticket },
                other => panic!("Enter did not use the existing submit path: {other:?}"),
            };
            assert!(picker.settle(&ticket, Some("Load refused; correct the exact path and retry.\n".repeat(12))));
            ui.outer_to_y(&picker, size, 100000.);
            let observed = ui.input.element_observation(ui.view(&picker, true), size)?;
            for label in ["Load selected file", "Cancel"] {
                let bounds = observed.texts.iter().find(|t| t.text == label)
                    .ok_or_else(|| anyhow::anyhow!("{label} unreachable at {size:?}"))?.bounds;
                assert!(bounds.width > 0. && bounds.height > 0.);
            }
            let cancel = observed.texts.iter().find(|t| t.text == "Cancel").unwrap().bounds.center();
            let mut events = ui.batch_at(&picker, size, &[motion(cancel), pointer(true), pointer(false)]);
            assert_eq!(events.len(), 1);
            assert!(matches!(picker.update(events.remove(0)), Outcome::Cancelled));

            // A new picker also reaches the oldest item in its bounded nested list.
            ui.input.reset();
            let mut picker = Picker::new(Path::new("manual.json"))?.with_recent(&history);
            let batch = ui.input.element_batch(ui.view(&picker, true), size, &[]);
            let inner = &batch.owners.scrolls[1];
            let center_y = inner.bounds[1] + inner.bounds[3] / 2.;
            if batch.owners.scrolls[0].content[3] > batch.owners.scrolls[0].bounds[3] {
                ui.outer_to_y(&picker, size, center_y);
            }
            let batch = ui.input.element_batch(ui.view(&picker, true), size, &[]);
            let inner = &batch.owners.scrolls[1];
            let outer = &batch.owners.scrolls[0];
            let center = Point::new(inner.bounds[0] + inner.bounds[2] / 2., inner.bounds[1] + inner.bounds[3] / 2. - outer.translation[1]);
            ui.wheel_at(&picker, size, center, -100000.);
            let observed = ui.input.element_observation(ui.view(&picker, true), size)?;
            let point = observed.texts.iter().find(|t| t.text == oldest)
                .ok_or_else(|| anyhow::anyhow!("oldest recent path unreachable at {size:?}"))?.bounds.center();
            let mut choices = ui.batch_at(&picker, size, &[motion(point), pointer(true), pointer(false)]);
            assert_eq!(choices.len(), 1);
            assert!(matches!(picker.update(choices.remove(0)), Outcome::FocusPath));
            assert_eq!(picker.value(), oldest);
            assert!(!picker.pending());
        }
        Ok(())
    }

}

// Save a Copy controls: authored source only, ALL UNRUN.
#[test]
fn save_copy_picker_purposes_cannot_exchange_events_or_completion() -> anyhow::Result<()> {
    let owner = super::super::save_copy::Owner::new();
    let mut load = Picker::new(Path::new("same.json"))?;
    let mut copy = Picker::new_copy(Path::new("same.json"), owner.clone())?;
    let foreign_edit = load.editor().changed("wrong.json".into());
    let foreign_submit = load.event(Intent::Submit);
    let (_, load_ticket) = submit(&mut load);
    assert!(matches!(copy.update(foreign_edit), Outcome::None));
    assert!(matches!(copy.update(foreign_submit), Outcome::None));
    let (_, copy_ticket) = submit(&mut copy);
    assert_eq!(copy_ticket.purpose(), Purpose::Copy);
    assert!(!copy.settle(&load_ticket, None));
    assert!(!load.settle(&copy_ticket, None));
    assert!(!load_ticket.claim_copy(Path::new("same.json"), &owner));
    assert!(copy.pending() && load.pending());
    assert!(copy.settle(&copy_ticket, Some("Retry Save a Copy…".into())));
    assert!(load.pending());
    let (_, retry) = submit(&mut copy);
    assert!(!copy.settle(&copy_ticket, None));
    assert!(copy.pending());
    assert!(copy.settle(&retry, None));
    Ok(())
}

#[test]
fn save_copy_picker_ticket_is_path_owner_bound_and_single_use_across_clones() -> anyhow::Result<()> {
    let owner = super::super::save_copy::Owner::new();
    let other = super::super::save_copy::Owner::new();
    let path = Path::new(" ../家族//copy e\u{301}.json ");
    let mut copy = Picker::new_copy(path, owner.clone())?;
    let (selected, ticket) = submit(&mut copy);
    assert_eq!(selected.as_os_str(), path.as_os_str());
    assert!(!ticket.claim_copy(Path::new("other.json"), &owner));
    assert!(!ticket.claim_copy(path, &other));
    let replay = ticket.clone();
    assert!(ticket.claim_copy(path, &owner));
    assert!(!replay.claim_copy(path, &owner));
    // A claim proves consumption, not completion. Pending cancel is refused.
    let cancel = copy.cancel_event();
    assert!(matches!(copy.update(cancel), Outcome::None));
    assert!(copy.pending());
    Ok(())
}

#[test]
fn save_copy_picker_batch_focus_retry_and_idle_cancel_preserve_the_editor() -> anyhow::Result<()> {
    let mut copy = Picker::new_copy(Path::new(""), super::super::save_copy::Owner::new())?;
    let id = copy.input_id.clone();
    let editor = copy.editor();
    let edits = [editor.changed("家族".into()), editor.changed("家族 copy.json ".into())];
    for event in edits { assert!(matches!(copy.update(event), Outcome::None)); }
    let held_submit = copy.event(Intent::Submit);
    copy.focus_lost();
    assert!(matches!(copy.update(held_submit), Outcome::None));
    let (path, ticket) = submit(&mut copy);
    assert_eq!(path.as_os_str(), "家族 copy.json ");
    assert!(copy.settle(&ticket, Some("No copy written. Retry Save a Copy….".into())));
    assert_eq!(copy.value(), "家族 copy.json ");
    assert_eq!(copy.input_id, id);
    let cancel = copy.cancel_event();
    assert!(matches!(copy.update(cancel), Outcome::Cancelled));
    assert!(!copy.pending());
    Ok(())

}
