//! Authored source-only reducer tests. No real native getter on fake pointers,
//! synthetic GUI claims, sleeps, runtime, journal or process/window discovery.
use super::*;

fn at(start: Instant, millis: u64) -> Instant {
    start.checked_add(Duration::from_millis(millis)).unwrap()
}

fn reply(ticket: Ticket, at: Instant, number: isize) -> Reply {
    ticket.complete(checked_window_number(number), at)
}

#[test]
fn exact_root_candidate_is_unrecorded_and_one_shot() {
    let start = Instant::now();
    let root = window::Id::unique();
    let mut state = WindowIdentity::new(root, 7);
    let issued = state.issue(start).unwrap();
    assert_eq!(issued.root, root);
    let response = reply(issued, at(start, 10), 91);
    let duplicate = response.clone();
    let candidate = state.accept(response, at(start, 20)).unwrap();
    assert_eq!(candidate.root(), root);
    assert_eq!(candidate.generation(), 7);
    assert_eq!(candidate.window_number(), 91);
    assert_eq!(candidate.publication_status(), "UNRECORDED");
    assert!(!state.has_pending_query());
    assert!(matches!(state.accept(duplicate, at(start, 21)), Err(Rejection::NoPendingQuery)));
}

#[test]
fn one_pending_query_refuses_overlap_but_allows_explicit_next_query() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let first = state.issue(start).unwrap();
    assert!(matches!(state.issue(at(start, 1)), Err(QueryError::Busy)));
    let repeated = reply(first.clone(), at(start, 2), 1);
    let _candidate = state.accept(repeated.clone(), at(start, 3)).unwrap();
    let second = state.issue(at(start, 4)).unwrap();
    assert_ne!(first.serial, second.serial);
    assert!(matches!(state.accept(repeated, at(start, 5)), Err(Rejection::StaleQuery)));
    assert!(state.has_pending_query());
    let _candidate = state.accept(reply(second, at(start, 6), 2), at(start, 7)).unwrap();
}

#[test]
fn generation_change_cancels_pending_and_keeps_exact_root_and_lifetime() {
    let start = Instant::now();
    let root = window::Id::unique();
    let mut state = WindowIdentity::new(root, 10);
    let old = state.issue(start).unwrap();
    assert_eq!(state.set_generation(11), Ok(true));
    assert!(!state.has_pending_query());
    assert_eq!(state.root(), root);
    assert_eq!(state.generation(), 11);
    let next = state.issue(at(start, 1)).unwrap();
    assert!(Arc::ptr_eq(&old.lifetime, &next.lifetime));
    assert!(matches!(state.accept(reply(old, at(start, 2), 50), at(start, 3)), Err(Rejection::StaleGeneration)));
    assert!(state.has_pending_query());
    let candidate = state.accept(reply(next, at(start, 4), 51), at(start, 5)).unwrap();
    assert_eq!(candidate.generation(), 11);
}

#[test]
fn equal_generation_preserves_pending_and_reused_generation_cannot_revive_ticket() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let old = state.issue(start).unwrap();
    assert_eq!(state.set_generation(0), Ok(false));
    assert!(state.has_pending_query());
    state.set_generation(1).unwrap();
    state.set_generation(0).unwrap();
    let current = state.issue(at(start, 1)).unwrap();
    assert!(matches!(state.accept(reply(old, at(start, 2), 1), at(start, 3)), Err(Rejection::StaleQuery)));
    assert!(state.has_pending_query());
    let _candidate = state.accept(reply(current, at(start, 4), 2), at(start, 5)).unwrap();
}

#[test]
fn foreign_state_same_root_and_serial_cannot_complete_local_query() {
    let start = Instant::now();
    let root = window::Id::unique();
    let mut local = WindowIdentity::new(root, 4);
    let mut foreign = WindowIdentity::new(root, 4);
    let ours = local.issue(start).unwrap();
    let theirs = foreign.issue(start).unwrap();
    assert_eq!(ours.serial, theirs.serial);
    assert!(matches!(local.accept(reply(theirs, at(start, 1), 1), at(start, 2)), Err(Rejection::ForeignLifetime)));
    assert!(local.has_pending_query());
    let _candidate = local.accept(reply(ours, at(start, 3), 2), at(start, 4)).unwrap();
}

#[test]
fn foreign_root_is_rejected_even_with_a_genuine_private_ticket() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let issued = state.issue(start).unwrap();
    // Only this private child test can corrupt an otherwise genuine ticket.
    let mut corrupted = issued.clone();
    corrupted.root = window::Id::unique();
    assert!(matches!(state.accept(reply(corrupted, at(start, 1), 1), at(start, 2)), Err(Rejection::ForeignWindow)));
    assert!(state.has_pending_query());
    let _candidate = state.accept(reply(issued, at(start, 3), 2), at(start, 4)).unwrap();
}

#[test]
fn close_intent_without_callback_is_terminal_and_other_windows_do_not_close_root() {
    let start = Instant::now();
    let root = window::Id::unique();
    let mut state = WindowIdentity::new(root, 2);
    let issued = state.issue(start).unwrap();
    assert!(!state.close_intent(window::Id::unique()));
    assert!(state.has_pending_query());
    assert!(state.close_intent(root));
    assert!(!state.has_pending_query());
    assert!(!state.expire(at(start, 3000)));
    assert!(matches!(state.accept(reply(issued, at(start, 1), 1), at(start, 2)), Err(Rejection::Closed)));
    assert!(matches!(state.issue(at(start, 3)), Err(QueryError::Closed)));
    assert_eq!(state.set_generation(3), Err(QueryError::Closed));
    assert!(state.closed(root));
}

#[test]
fn closed_event_before_callback_is_terminal_without_close_intent() {
    let start = Instant::now();
    let root = window::Id::unique();
    let mut state = WindowIdentity::new(root, 0);
    let issued = state.issue(start).unwrap();
    assert!(!state.closed(window::Id::unique()));
    assert!(state.closed(root));
    assert!(matches!(state.accept(reply(issued, at(start, 1), 1), at(start, 2)), Err(Rejection::Closed)));
    assert!(matches!(state.request(at(start, 3)), Err(QueryError::Closed)));
}

#[test]
fn no_callback_expires_at_deadline_without_automatic_retry() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let old = state.issue(start).unwrap();
    assert!(!state.expire(at(start, 1999)));
    assert!(state.expire(start + QUERY_TIMEOUT));
    assert!(!state.has_pending_query());
    assert!(!state.expire(at(start, 4000)));
    assert!(matches!(state.accept(reply(old.clone(), at(start, 1), 1), at(start, 2001)), Err(Rejection::NoPendingQuery)));
    let current = state.issue(at(start, 2002)).unwrap();
    assert!(matches!(state.accept(reply(old, at(start, 2003), 1), at(start, 2004)), Err(Rejection::StaleQuery)));
    let _candidate = state.accept(reply(current, at(start, 2005), 2), at(start, 2006)).unwrap();
}

#[test]
fn acceptance_checks_deadline_even_without_tick_and_includes_delivery_delay() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let issued = state.issue(start).unwrap();
    let early_callback = reply(issued, at(start, 1), 1);
    assert!(matches!(state.accept(early_callback, start + QUERY_TIMEOUT), Err(Rejection::Expired)));
    assert!(!state.has_pending_query());
    let next = state.issue(at(start, 2001)).unwrap();
    let deadline = at(start, 2001) + QUERY_TIMEOUT;
    assert!(matches!(state.accept(reply(next, deadline, 1), deadline), Err(Rejection::Expired)));
}

#[test]
fn explicit_request_after_expiry_replaces_pending_without_a_tick() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let old = state.issue(start).unwrap();
    let current = state.issue(start + QUERY_TIMEOUT).unwrap();
    assert!(current.serial > old.serial);
    assert!(matches!(state.accept(reply(old, at(start, 1), 1), start + QUERY_TIMEOUT), Err(Rejection::StaleQuery)));
    let _candidate = state.accept(reply(current, at(start, 2001), 2), at(start, 2002)).unwrap();
}

#[test]
fn invalid_callback_or_delivery_time_consumes_query_without_candidate() {
    let start = Instant::now();
    // Deliberately inconsistent supplied clocks, with no sleeps.
    for (observed, delivered) in [(0, 11), (12, 11), (10, 9)] {
        let mut state = WindowIdentity::new(window::Id::unique(), 0);
        let ticket = state.issue(at(start, 10)).unwrap();
        assert!(matches!(state.accept(reply(ticket, at(start, observed), 1), at(start, delivered)), Err(Rejection::InvalidTime)));
        assert!(!state.has_pending_query());
    }
}

#[test]
fn unavailable_observations_consume_only_matching_query_and_allow_explicit_retry() {
    let start = Instant::now();
    for failure in [
        Unavailable::Platform, Unavailable::NotMainThread, Unavailable::WindowHandle,
        Unavailable::HandlePlatform, Unavailable::Selector, Unavailable::DetachedView,
        Unavailable::NonPositiveNumber, Unavailable::NumberOutOfRange,
    ] {
        let mut state = WindowIdentity::new(window::Id::unique(), 0);
        let ticket = state.issue(start).unwrap();
        let response = ticket.complete(Err(failure), at(start, 1));
        assert!(matches!(state.accept(response, at(start, 2)), Err(Rejection::Unavailable(actual)) if actual == failure));
        assert!(!state.has_pending_query());
        assert!(state.issue(at(start, 3)).is_ok());
    }
}

#[test]
fn checked_scalar_conversion_rejects_nonpositive_and_overflow() {
    assert_eq!(checked_window_number(isize::MIN), Err(Unavailable::NonPositiveNumber));
    assert_eq!(checked_window_number(-1), Err(Unavailable::NonPositiveNumber));
    assert_eq!(checked_window_number(0), Err(Unavailable::NonPositiveNumber));
    assert_eq!(checked_window_number(1), Ok(1));
    if let Ok(maximum) = isize::try_from(u32::MAX) {
        assert_eq!(checked_window_number(maximum), Ok(u32::MAX));
    }
    if let Ok(overflow) = isize::try_from(u64::from(u32::MAX) + 1) {
        assert_eq!(checked_window_number(overflow), Err(Unavailable::NumberOutOfRange));
        assert_eq!(checked_window_number(isize::MAX), Err(Unavailable::NumberOutOfRange));
    }
}

#[test]
fn zero_cannot_be_accepted_even_if_private_callback_helper_bypasses_conversion() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let ticket = state.issue(start).unwrap();
    assert!(matches!(state.accept(ticket.complete(Ok(0), at(start, 1)), at(start, 2)), Err(Rejection::Unavailable(Unavailable::NonPositiveNumber))));
    assert!(!state.has_pending_query());
}

#[test]
fn serial_overflow_permanently_exhausts_lifetime_and_rejects_late_reply() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    // Boundary setup remains private; no public counter setter/reset exists.
    state.next_serial = u64::MAX - 1;
    let old = state.issue(start).unwrap();
    assert_eq!(old.serial, u64::MAX - 1);
    assert!(state.expire(start + QUERY_TIMEOUT));
    assert!(matches!(state.issue(at(start, 2001)), Err(QueryError::Exhausted)));
    assert!(matches!(state.accept(reply(old, at(start, 1), 1), at(start, 2002)), Err(Rejection::Exhausted)));
    assert_eq!(state.set_generation(1), Err(QueryError::Exhausted));
    assert!(matches!(state.issue(at(start, 4000)), Err(QueryError::Exhausted)));
    assert!(!state.has_pending_query());
}

#[test]
fn dropped_unrecorded_candidate_is_not_cached_or_recoverable_from_replayed_reply() {
    let start = Instant::now();
    let mut state = WindowIdentity::new(window::Id::unique(), 0);
    let ticket = state.issue(start).unwrap();
    let response = reply(ticket, at(start, 1), 1);
    let replay = response.clone();
    // Models only a parent dropping a value, not actual journal acceptance.
    drop(state.accept(response, at(start, 2)).unwrap());
    assert!(matches!(state.accept(replay, at(start, 3)), Err(Rejection::NoPendingQuery)));
    state.set_generation(1).unwrap();
    assert!(!state.has_pending_query());
}

#[test]
fn request_constructs_exact_iced_action_without_running_callback_or_native_getter() {
    use iced::futures::{FutureExt, StreamExt};
    let start = Instant::now();
    let root = window::Id::unique();
    let mut state = WindowIdentity::new(root, 0);
    // Inspect the first action with a single nonblocking poll. Do not invoke
    // the Run closure, an executor, an event loop or a native getter.
    let task: Task<Reply> = state.request(start).unwrap();
    assert!(state.has_pending_query());
    assert_eq!(state.root(), root);
    assert!(matches!(state.request(at(start, 1)), Err(QueryError::Busy)));
    let mut stream = iced_runtime::task::into_stream(task).unwrap();
    let Some(Some(iced_runtime::Action::Window(iced_runtime::window::Action::Run(actual, callback)))) =
        stream.next().now_or_never()
    else { panic!("request must yield an exact managed-window Run action immediately"); };
    assert_eq!(actual, root);
    drop(callback);
    drop(stream);
    assert!(state.expire(start + QUERY_TIMEOUT));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_returns_unavailable_before_requesting_any_handle() {
    use window::raw_window_handle::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};
    struct Untouched;
    impl HasWindowHandle for Untouched {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            panic!("unsupported platform must not request a window handle")
        }
    }
    impl HasDisplayHandle for Untouched {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            panic!("unsupported platform must not request a display handle")
        }
    }
    // No pointers or native objects exist; platform guard must precede handles.
    assert_eq!(native_window_number(&Untouched), Err(Unavailable::Platform));
}
