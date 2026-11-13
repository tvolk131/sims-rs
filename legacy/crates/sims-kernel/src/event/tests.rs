use super::*;
use std::{sync::mpsc, thread};

fn pending(event: &Event) -> Wait {
    let mut wait = event.begin_wait(Timeout::Infinite).unwrap();
    assert_eq!(wait.poll(), Ok(Poll::Pending));
    wait
}

fn poll(event: &Event) -> Outcome {
    event
        .begin_wait(Timeout::Milliseconds(0))
        .unwrap()
        .wait()
        .unwrap()
}

#[test]
fn auto_reset_sets_coalesce_and_completed_retry_never_consumes_again() {
    let event = Event::new(ResetMode::Auto, false);
    assert_eq!(poll(&event), Outcome::TimedOut);
    event.set().unwrap();
    event.set().unwrap();
    let mut completed = event.begin_wait(Timeout::Milliseconds(0)).unwrap();
    assert_eq!(completed.wait(), Ok(Outcome::Signaled));
    assert_eq!(poll(&event), Outcome::TimedOut);
    event.set().unwrap();
    assert_eq!(completed.poll(), Ok(Poll::Ready(Outcome::Signaled)));
    assert_eq!(poll(&event), Outcome::Signaled);
    assert_eq!(poll(&event), Outcome::TimedOut);
}

#[test]
fn auto_reset_releases_exactly_one_registered_wait_per_signal() {
    let event = Event::new(ResetMode::Auto, false);
    let mut waits = [pending(&event), pending(&event)];
    event.set().unwrap();
    let first: Vec<_> = waits.iter_mut().map(Wait::poll).collect();
    assert_eq!(
        first
            .iter()
            .filter(|r| **r == Ok(Poll::Ready(Outcome::Signaled)))
            .count(),
        1
    );
    assert_eq!(first.iter().filter(|r| **r == Ok(Poll::Pending)).count(), 1);
    assert_eq!(poll(&event), Outcome::TimedOut);
    event.set().unwrap();
    assert!(
        waits
            .iter_mut()
            .all(|wait| wait.wait() == Ok(Outcome::Signaled))
    );
}

#[test]
fn manual_set_releases_existing_and_future_waits_until_reset() {
    let event = Event::new(ResetMode::Manual, false);
    let mut first = pending(&event);
    let mut second = pending(&event);
    event.set().unwrap();
    assert_eq!(poll(&event), Outcome::Signaled);
    assert_eq!(poll(&event), Outcome::Signaled);
    event.reset().unwrap();
    // Reset cannot revoke calls already released by the preceding set.
    assert_eq!(first.wait(), Ok(Outcome::Signaled));
    assert_eq!(second.wait(), Ok(Outcome::Signaled));
    assert_eq!(poll(&event), Outcome::TimedOut);
}

#[test]
fn initial_state_is_owned_and_events_with_equal_tickets_are_independent() {
    let first = Event::new(ResetMode::Auto, true);
    let second = Event::new(ResetMode::Auto, false);
    assert_eq!(poll(&first), Outcome::Signaled);
    let mut a = pending(&first);
    let mut b = pending(&second);
    first.set().unwrap();
    assert_eq!(a.wait(), Ok(Outcome::Signaled));
    assert_eq!(b.poll(), Ok(Poll::Pending));
    second.pulse().unwrap();
    assert_eq!(b.wait(), Ok(Outcome::Signaled));
}

#[test]
fn pulse_with_no_waiter_keeps_no_future_credit_and_clears_old_signal() {
    for mode in [ResetMode::Auto, ResetMode::Manual] {
        let event = Event::new(mode, true);
        event.pulse().unwrap();
        assert_eq!(poll(&event), Outcome::TimedOut);
        let mut future = pending(&event);
        assert_eq!(future.poll(), Ok(Poll::Pending));
    }
}

#[test]
fn auto_pulse_releases_one_present_waiter_and_never_a_later_waiter() {
    let event = Event::new(ResetMode::Auto, false);
    let mut waits = [pending(&event), pending(&event)];
    event.pulse().unwrap();
    let first: Vec<_> = waits.iter_mut().map(Wait::poll).collect();
    assert_eq!(
        first
            .iter()
            .filter(|r| **r == Ok(Poll::Ready(Outcome::Signaled)))
            .count(),
        1
    );
    assert_eq!(first.iter().filter(|r| **r == Ok(Poll::Pending)).count(), 1);
    let mut later = pending(&event);
    assert_eq!(later.poll(), Ok(Poll::Pending));
    event.pulse().unwrap();
    assert!(
        waits
            .iter_mut()
            .all(|wait| wait.wait() == Ok(Outcome::Signaled))
    );
    assert_eq!(later.poll(), Ok(Poll::Pending));
}

#[test]
fn manual_pulse_releases_all_present_waiters_and_no_future_waiter() {
    let event = Event::new(ResetMode::Manual, false);
    let mut waits = [pending(&event), pending(&event)];
    event.pulse().unwrap();
    let mut later = pending(&event);
    for wait in &mut waits {
        assert_eq!(wait.wait(), Ok(Outcome::Signaled));
    }
    assert_eq!(later.poll(), Ok(Poll::Pending));
    assert_eq!(poll(&event), Outcome::TimedOut);
}

#[test]
fn finite_wait_uses_condvar_timeout_and_retains_its_completed_result() {
    let event = Event::new(ResetMode::Auto, false);
    let mut wait = event.begin_wait(Timeout::Milliseconds(2)).unwrap();
    assert_eq!(wait.wait(), Ok(Outcome::TimedOut));
    event.set().unwrap();
    assert_eq!(wait.poll(), Ok(Poll::Ready(Outcome::TimedOut)));
    assert_eq!(poll(&event), Outcome::Signaled);
}

#[test]
fn an_expired_unpolled_registration_cannot_steal_a_later_signal() {
    for mode in [ResetMode::Auto, ResetMode::Manual] {
        let event = Event::new(mode, false);
        let mut expired = event.begin_wait(Timeout::Milliseconds(1)).unwrap();
        thread::sleep(Duration::from_millis(2));
        assert!(Instant::now() >= expired.deadline.unwrap());
        event.set().unwrap();
        assert_eq!(expired.wait(), Ok(Outcome::TimedOut));
        assert_eq!(poll(&event), Outcome::Signaled);
    }
}

#[test]
fn an_expired_unpolled_registration_cannot_steal_a_pulse() {
    let event = Event::new(ResetMode::Auto, false);
    let mut expired = event.begin_wait(Timeout::Milliseconds(1)).unwrap();
    let mut live = pending(&event);
    thread::sleep(Duration::from_millis(2));
    assert!(Instant::now() >= expired.deadline.unwrap());
    event.pulse().unwrap();
    assert_eq!(expired.wait(), Ok(Outcome::TimedOut));
    assert_eq!(live.wait(), Ok(Outcome::Signaled));
    assert_eq!(poll(&event), Outcome::TimedOut);
}

#[test]
fn dropping_a_wait_releases_registration_and_retirement_keeps_ownership_on_error() {
    let mut event = Event::new(ResetMode::Auto, false);
    let wait = pending(&event);
    assert_eq!(event.retire(), Err(Error::WaitsPending));
    drop(wait);
    let mut next = pending(&event);
    event.set().unwrap();
    assert_eq!(next.wait(), Ok(Outcome::Signaled));
    event.retire().unwrap();
    assert_eq!(event.retire(), Err(Error::Retired));
    assert_eq!(event.set(), Err(Error::Retired));
    assert_eq!(event.reset(), Err(Error::Retired));
    assert_eq!(event.pulse(), Err(Error::Retired));
    assert!(matches!(
        event.begin_wait(Timeout::Milliseconds(0)),
        Err(Error::Retired)
    ));
}

#[test]
fn wait_capacity_failure_preserves_existing_waits_and_recovers_after_cancel() {
    let event = Event::new(ResetMode::Manual, false);
    let mut waits: Vec<_> = (0..MAX_PENDING_WAITS).map(|_| pending(&event)).collect();
    assert!(matches!(
        event.begin_wait(Timeout::Infinite),
        Err(Error::WaitCapacity)
    ));
    waits.pop();
    waits.push(pending(&event));
    event.set().unwrap();
    assert!(
        waits
            .iter_mut()
            .all(|wait| wait.wait() == Ok(Outcome::Signaled))
    );
}

#[test]
fn actual_blocked_condvar_wait_wakes_from_set() {
    let event = Event::new(ResetMode::Auto, false);
    let mut wait = event.begin_wait(Timeout::Milliseconds(5000)).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut tx = Some(tx);
        wait.wait_impl(|| {
            if let Some(tx) = tx.take() {
                tx.send(()).unwrap();
            }
        })
    });
    // The hook runs with the event mutex held immediately before Condvar wait.
    // set must acquire that mutex after the actual atomic sleep/unlock.
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    event.set().unwrap();
    assert_eq!(worker.join().unwrap(), Ok(Outcome::Signaled));
}

#[test]
fn owner_drop_aborts_an_actual_blocked_wait_without_detaching_a_thread() {
    let event = Event::new(ResetMode::Auto, false);
    let mut wait = event.begin_wait(Timeout::Milliseconds(5000)).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut tx = Some(tx);
        wait.wait_impl(|| {
            if let Some(tx) = tx.take() {
                tx.send(()).unwrap();
            }
        })
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(event);
    assert_eq!(worker.join().unwrap(), Err(Error::Retired));
}

#[test]
fn reserved_timeout_rejection_does_not_consume_an_available_signal() {
    let event = Event::new(ResetMode::Auto, true);
    assert!(matches!(
        event.begin_wait(Timeout::Milliseconds(u32::MAX)),
        Err(Error::ReservedTimeout)
    ));
    assert_eq!(poll(&event), Outcome::Signaled);
}

#[test]
fn poisoned_state_fails_closed_but_a_staged_return_is_stable() {
    let event = Event::new(ResetMode::Auto, true);
    let mut completed = event.begin_wait(Timeout::Infinite).unwrap();
    let inner = Arc::clone(&event.inner);
    assert!(
        thread::spawn(move || {
            let _guard = inner.state.lock().unwrap();
            panic!("authored poison control");
        })
        .join()
        .is_err()
    );
    assert_eq!(completed.wait(), Ok(Outcome::Signaled));
    assert_eq!(event.set(), Err(Error::Poisoned));
    assert_eq!(event.pulse(), Err(Error::Poisoned));
    assert!(matches!(
        event.begin_wait(Timeout::Infinite),
        Err(Error::Poisoned)
    ));
}

#[test]
fn lifetime_identity_never_joins_equal_events_or_keeps_an_owner_live() {
    let first = Event::new(ResetMode::Auto, false);
    let second = Event::new(ResetMode::Auto, false);
    let identity = first.identity();
    assert_eq!(identity, identity.clone());
    assert_ne!(identity, second.identity());
    assert!(first.has_identity(&identity));
    assert!(!second.has_identity(&identity));
    let mut waiting = pending(&first);
    drop(first);
    assert_eq!(waiting.poll(), Err(Error::Retired));
    assert!(!second.has_identity(&identity));
}

#[test]
fn read_only_live_checks_do_not_consume_signals_or_release_registrations() {
    let mut event = Event::new(ResetMode::Auto, true);
    event.check_live().unwrap();
    event.check_live().unwrap();
    assert_eq!(poll(&event), Outcome::Signaled);
    let waiting = pending(&event);
    event.check_live().unwrap();
    assert_eq!(event.retire(), Err(Error::WaitsPending));
    drop(waiting);
    event.retire().unwrap();
    assert_eq!(event.check_live(), Err(Error::Retired));
}
