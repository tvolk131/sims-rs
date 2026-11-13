//! AUTHORED, UNRUN. Synthetic managed Ids and sealed pending completions;
//! bounded real-file writer/ack joins. No windows, callback invocation, native
//! getter, external verifier or kernel/execution/GUI acceptance is exercised.
use super::*;
use super::super::window_capture::Capture;
use super::super::window_identity::{QUERY_TIMEOUT, Unavailable};
use window_session::candidate_journal_fixture::Journal;
use iced::futures::{FutureExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

fn run_on_stack(test: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(test)?.join().unwrap()
}
fn loading_host(recording: bool) -> Host {
    use clap::Parser;
    let args = Args::parse_from(["sims-room", "--original", "UserData2/Houses/House09.iff"]);
    let (viewer, task) = Viewer::new(args);
    drop(task); // Never execute the content loader or a native task.
    let mut host = Host::from_viewer_for_test(viewer);
    host.window_capture = Capture::new(recording);
    host
}
fn assert_run<T: Send + 'static>(task: Task<T>, root: window::Id) {
    let mut stream = iced_runtime::task::into_stream(task).expect("exact Run task absent");
    match stream.next().now_or_never() {
        Some(Some(iced_runtime::Action::Window(iced_runtime::window::Action::Run(id, callback)))) => {
            assert_eq!(id, root);
            drop(callback); // Never call native code, even on an actual macOS test host.
        }
        _ => panic!("expected one immediate exact-window Run action"),
    }
    drop(stream);
}
fn assert_close(task: Task<Event>, root: window::Id) {
    let mut stream = iced_runtime::task::into_stream(task).expect("close task absent");
    assert!(matches!(stream.next().now_or_never(),
        Some(Some(iced_runtime::Action::Window(iced_runtime::window::Action::Close(id)))) if id == root));
}
fn open(capture: &mut Capture, root: window::Id, now: Instant) {
    let event = capture.opened_for_test(root);
    let outcome = capture.handle(event, now);
    assert!(outcome.close_requested.is_none());
    assert_run(outcome.task, root);
}
fn reply(capture: &Capture, number: u32, now: Instant) -> super::super::window_capture::Event {
    capture.pending_reply_for_test(Ok(number), now).expect("genuine pending query absent")
}
fn unavailable(capture: &Capture) {
    let value = capture.diagnostics();
    assert_eq!(value["status"], "UNAVAILABLE");
    assert!(value.get("window_number").is_none());
    assert!(value.get("generation").is_none());
}

struct FileJournal {
    path: std::path::PathBuf,
    journal: Arc<Mutex<Journal>>,
}
impl FileJournal {
    fn new(capacity: usize, budget: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("sims-host-window-{}-{}-{}.jsonl",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)));
        let journal = Arc::new(Mutex::new(Journal::new(&path, capacity, budget).unwrap()));
        Self { path, journal }
    }
    fn attach(&self, capture: &mut Capture) { capture.journal_for_test = Some(Arc::clone(&self.journal)); }
    fn finish(&self) { self.journal.lock().unwrap().finish().unwrap(); }
    fn rows(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.path).unwrap().lines()
            .map(|line| serde_json::from_str(line).unwrap()).collect()
    }
}
impl Drop for FileJournal {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

#[test]
fn real_host_open_route_emits_exact_run_and_binds_only_first_root() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        drop(host.subscription()); // Constructs the production open/close recipes.
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root); // Same sealed mapper as open_events().
        assert_run(host.update(Event::WindowCapture(event)), root);
        let event = host.window_capture.opened_for_test(window::Id::unique());
        assert_eq!(host.update(Event::WindowCapture(event)).units(), 0);
        let event = host.window_capture.opened_for_test(root);
        assert_eq!(host.update(Event::WindowCapture(event)).units(), 0);
        assert_eq!(host.window_capture.root_for_test(), Some(root));
        assert!(host.window_capture.has_pending_for_test());
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn distinct_host_seals_reject_equal_id_and_generation_messages() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        let foreign = Capture::new(true);
        assert!(!host.window_capture.same_seal_for_test(&foreign));
        let before = host.window_capture.seal_hash_for_test();
        assert_eq!(before, host.window_capture.seal_hash_for_test());
        let root = window::Id::unique();
        assert_eq!(host.update(Event::WindowCapture(foreign.opened_for_test(root))).units(), 0);
        assert_eq!(host.window_capture.root_for_test(), None);
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.update(Event::WindowCapture(event)), root);
        for event in [foreign.close_requested_for_test(root), foreign.closed_for_test(root)] {
            assert_eq!(host.update(Event::WindowCapture(event)).units(), 0);
        }
        assert!(!host.close_sent && host.closing.is_none());
        assert!(host.window_capture.has_pending_for_test());
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn foreign_window_close_does_not_close_or_invalidate_root() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let root = window::Id::unique();
    open(&mut capture, root, now);
    let other = window::Id::unique();
    for event in [capture.close_requested_for_test(other), capture.closed_for_test(other)] {
        let outcome = capture.handle(event, now);
        assert!(outcome.close_requested.is_none());
        assert_eq!(outcome.task.units(), 0);
        assert!(capture.has_pending_for_test());
    }
}

#[test]
fn close_intent_before_callback_is_terminal_and_precedes_host_close_task() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.update(Event::WindowCapture(event)), root);
        let late = reply(&host.window_capture, 91, Instant::now());
        let close = host.window_capture.close_requested_for_test(root);
        let task = host.update(Event::WindowCapture(close));
        unavailable(&host.window_capture);
        assert!(!host.window_capture.has_pending_for_test());
        assert_close(task, root);
        assert!(host.close_sent);
        drop(host.update(Event::WindowCapture(late)));
        let reopen = host.window_capture.opened_for_test(root);
        assert_eq!(host.update(Event::WindowCapture(reopen)).units(), 0);
        unavailable(&host.window_capture);
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn closed_without_intent_and_generation_changes_never_rebind() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let root = window::Id::unique();
    open(&mut capture, root, now);
    let late = reply(&capture, 91, now);
    let event = capture.closed_for_test(root);
    assert!(capture.handle(event, now).close_requested.is_none());
    assert_eq!(capture.synchronize(1, now).units(), 0);
    drop(capture.handle(late, now));
    let event = capture.opened_for_test(window::Id::unique());
    assert_eq!(capture.handle(event, now).task.units(), 0);
    assert_eq!(capture.root_for_test(), Some(root));
    unavailable(&capture);
}

#[test]
fn no_callback_tick_expiry_has_no_retry_until_actual_generation_change() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let root = window::Id::unique();
    open(&mut capture, root, now);
    let late = reply(&capture, 91, now);
    capture.tick(now + QUERY_TIMEOUT);
    assert!(!capture.has_pending_for_test());
    drop(capture.handle(late, now + QUERY_TIMEOUT));
    assert_eq!(capture.synchronize(0, now + QUERY_TIMEOUT).units(), 0);
    let event = capture.opened_for_test(root);
    assert_eq!(capture.handle(event, now + QUERY_TIMEOUT).task.units(), 0);
    assert_run(capture.synchronize(1, now + QUERY_TIMEOUT), root);
    assert_eq!(capture.synchronize(1, now + QUERY_TIMEOUT).units(), 0);
    unavailable(&capture);
}

#[test]
fn headless_host_frame_tick_expires_a_query_without_callback() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        let issued_at = Instant::now().checked_sub(QUERY_TIMEOUT + Duration::from_millis(1)).unwrap();
        assert_run(host.window_capture.handle(event, issued_at).task, root);
        drop(host.update(Event::Frame(Instant::now())));
        assert!(!host.window_capture.has_pending_for_test());
        unavailable(&host.window_capture);
        host.settle_for_test()?;
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn delayed_host_frame_cannot_admit_an_already_expired_ready_ack() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        Arc::make_mut(&mut host.frontend.data).window_observation = Some(Arc::new(serde_json::json!({})));
        let root = window::Id::unique();
        let issued_at = Instant::now().checked_sub(Duration::from_secs(3)).unwrap();
        let file = FileJournal::new(4, 4096);
        file.attach(&mut host.window_capture);
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.window_capture.handle(event, issued_at).task, root);
        let response = reply(&host.window_capture, 91, issued_at);
        drop(host.window_capture.handle(response, issued_at));
        file.finish(); // Exact real-file ack ready, but its deadline is in the past.
        assert_eq!(file.rows()[0]["data"]["window_id"], 91);
        unavailable(&host.window_capture);
        // Iced's redraw timestamp predates delivery. It must not be used as
        // the capture acknowledgement clock even though simulation retains it.
        drop(host.update(Event::Frame(issued_at)));
        unavailable(&host.window_capture);
        assert_eq!(host.observation_value().unwrap()["owned_window_candidate"]["status"], "UNAVAILABLE");
        host.settle_for_test()?;
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn actual_sealed_leaf_joins_real_file_ack_and_observational_diagnostics() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let file = FileJournal::new(4, 4096);
    file.attach(&mut capture);
    open(&mut capture, window::Id::unique(), now);
    let event = reply(&capture, 91, now);
    drop(capture.handle(event, now));
    unavailable(&capture);
    assert!(!capture.has_pending_for_test());
    assert!(std::fs::read(&file.path).unwrap().is_empty()); // Real queued row, not synced.
    file.finish(); // Actual bounded JSONL write/flush/sync_data/exact ack.
    unavailable(&capture); // Row presence still has no authority until poll.
    capture.tick(now);
    let diagnostics = capture.diagnostics();
    assert_eq!(diagnostics["status"], "DURABLY_RECORDED");
    assert_eq!(diagnostics["window_number"], 91);
    assert_eq!(diagnostics["generation"], 0);
    assert!(diagnostics["scope"].as_str().unwrap().contains("observational"));
    let rows = file.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["kind"], "owned_window_candidate");
    assert_eq!(rows[0]["data"]["window_id"], 91);
    assert_eq!(rows[0]["data"]["generation"], 0);
    assert_eq!(rows[1]["kind"], "session_end");
}

#[test]
fn full_and_disconnected_real_journal_queues_expose_no_candidate() {
    for disconnect in [false, true] {
        let now = Instant::now();
        let mut capture = Capture::new(true);
        let file = FileJournal::new(1, 4096);
        if disconnect { file.journal.lock().unwrap().disconnect(); }
        else { file.journal.lock().unwrap().fill(); }
        file.attach(&mut capture);
        open(&mut capture, window::Id::unique(), now);
        let event = reply(&capture, 91, now);
        drop(capture.handle(event.clone(), now));
        unavailable(&capture);
        assert!(!capture.has_pending_for_test());
        drop(capture.handle(event, now));
        assert_eq!(capture.synchronize(0, now).units(), 0);
        assert!(std::fs::read(&file.path).unwrap().is_empty());
        unavailable(&capture);
    }
}

#[test]
fn bounded_real_writer_failure_cannot_acknowledge_host_candidate() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let file = FileJournal::new(4, 1);
    file.attach(&mut capture);
    open(&mut capture, window::Id::unique(), now);
    let event = reply(&capture, 91, now);
    drop(capture.handle(event, now));
    assert!(file.journal.lock().unwrap().finish().is_err());
    capture.tick(now);
    unavailable(&capture);
    assert!(std::fs::read(&file.path).unwrap().len() <= 1);
}

#[test]
fn expired_publication_row_can_appear_later_without_availability() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let file = FileJournal::new(4, 4096);
    file.attach(&mut capture);
    open(&mut capture, window::Id::unique(), now);
    let event = reply(&capture, 91, now);
    drop(capture.handle(event, now));
    capture.tick(now + Duration::from_secs(2));
    unavailable(&capture);
    file.finish(); // Queued old candidate legitimately lands after its deadline.
    assert_eq!(file.rows()[0]["data"]["window_id"], 91);
    capture.tick(now); // Even a clock supplied earlier cannot resurrect it.
    unavailable(&capture);
}

#[test]
fn generation_and_close_before_ack_discard_the_exact_old_publication() {
    for close in [false, true] {
        let now = Instant::now();
        let mut capture = Capture::new(true);
        let file = FileJournal::new(4, 4096);
        file.attach(&mut capture);
        let root = window::Id::unique();
        open(&mut capture, root, now);
        let event = reply(&capture, 91, now);
        drop(capture.handle(event, now));
        if close {
            let event = capture.closed_for_test(root);
            drop(capture.handle(event, now));
        } else { assert_run(capture.synchronize(1, now), root); }
        file.finish();
        capture.tick(now);
        unavailable(&capture);
        assert_eq!(file.rows()[0]["data"]["generation"], 0);
    }
}

#[test]
fn stale_foreign_and_duplicate_replies_cannot_evict_current_query_or_ack() {
    let now = Instant::now();
    let root = window::Id::unique();
    let mut capture = Capture::new(true);
    let mut foreign = Capture::new(true);
    open(&mut capture, root, now);
    open(&mut foreign, root, now);
    let stale = reply(&capture, 11, now);
    let alien = reply(&foreign, 12, now);
    assert_run(capture.synchronize(1, now), root);
    for event in [stale, alien] {
        drop(capture.handle(event, now));
        assert!(capture.has_pending_for_test());
    }
    let file = FileJournal::new(4, 4096);
    file.attach(&mut capture);
    let current = reply(&capture, 91, now);
    drop(capture.handle(current.clone(), now));
    drop(capture.handle(current.clone(), now));
    unavailable(&capture);
    file.finish();
    capture.tick(now);
    assert_eq!(capture.diagnostics()["window_number"], 91);
    drop(capture.handle(current, now));
    assert_eq!(capture.diagnostics()["status"], "DURABLY_RECORDED");
    assert_eq!(file.rows().len(), 2); // Duplicate never enqueued another row.
}

#[test]
fn late_old_ack_cannot_publish_the_new_generations_number() {
    let now = Instant::now();
    let root = window::Id::unique();
    let mut capture = Capture::new(true);
    let old_file = FileJournal::new(4, 4096);
    old_file.attach(&mut capture);
    open(&mut capture, root, now);
    let old = reply(&capture, 11, now);
    drop(capture.handle(old, now));
    assert_run(capture.synchronize(1, now), root);
    let current_file = FileJournal::new(4, 4096);
    current_file.attach(&mut capture);
    let current = reply(&capture, 91, now);
    drop(capture.handle(current, now));
    old_file.finish();
    capture.tick(now);
    unavailable(&capture);
    current_file.finish();
    capture.tick(now);
    assert_eq!(capture.diagnostics()["window_number"], 91);
    assert_eq!(capture.diagnostics()["generation"], 1);
}

#[test]
fn recorded_candidate_clears_on_generation_or_close_and_never_selects_another_root() {
    for close in [false, true] {
        let now = Instant::now();
        let mut capture = Capture::new(true);
        let file = FileJournal::new(4, 4096);
        file.attach(&mut capture);
        let root = window::Id::unique();
        open(&mut capture, root, now);
        let event = reply(&capture, 91, now);
        drop(capture.handle(event, now));
        file.finish(); capture.tick(now);
        assert_eq!(capture.diagnostics()["status"], "DURABLY_RECORDED");
        if close {
            let event = capture.close_requested_for_test(root);
            drop(capture.handle(event, now));
        } else { assert_run(capture.synchronize(1, now), root); }
        unavailable(&capture);
        let event = capture.opened_for_test(window::Id::unique());
        assert_eq!(capture.handle(event, now).task.units(), 0);
        assert_eq!(capture.root_for_test(), Some(root));
    }
}

#[test]
fn unavailable_or_zero_leaf_result_never_queues_a_row() {
    for result in [Err(Unavailable::DetachedView), Ok(0)] {
        let now = Instant::now();
        let mut capture = Capture::new(true);
        let file = FileJournal::new(4, 4096);
        file.attach(&mut capture);
        open(&mut capture, window::Id::unique(), now);
        let event = capture.pending_reply_for_test(result, now).unwrap();
        drop(capture.handle(event, now));
        unavailable(&capture);
        file.finish();
        assert_eq!(file.rows().len(), 1); // Finalization only.
        assert_eq!(file.rows()[0]["kind"], "session_end");
    }
}

#[test]
fn unrecorded_host_preserves_controls_and_close_without_querying() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(false);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        assert_eq!(host.update(Event::WindowCapture(event)).units(), 0);
        assert!(!host.window_capture.has_pending_for_test());
        let generation = host.generation();
        drop(host.update(Event::Widget { generation, message: Message::Options }));
        assert!(host.frontend.options_open);
        drop(host.update(Event::Widget { generation, message: Message::Diagnostics }));
        assert!(host.frontend.diagnostics && !host.frontend.options_open);
        let event = host.window_capture.close_requested_for_test(root);
        assert_close(host.update(Event::WindowCapture(event)), root);
        unavailable(&host.window_capture);
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn real_host_generation_poll_precedes_stale_reply_and_old_ack() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        let old_file = FileJournal::new(4, 4096);
        old_file.attach(&mut host.window_capture);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.update(Event::WindowCapture(event)), root);
        let old_reply = reply(&host.window_capture, 11, Instant::now());
        drop(host.update(Event::WindowCapture(old_reply.clone())));
        // Restart an unloaded authored Viewer: real worker generation change,
        // with no loader execution or private/game/native assets.
        let (notified, completed) = std::sync::mpsc::sync_channel(1);
        host.notification_for_test(Box::new(move || { let _ = notified.send(()); }));
        drop(host.update(Event::Widget { generation: 0, message: Message::Restart }));
        completed.recv_timeout(Duration::from_secs(5))?;
        old_file.finish(); // Ack is ready before the UI polls the replacement.
        let task = host.update(Event::WindowCapture(old_reply));
        // This batch also contains the real Restart loader future. Do not
        // poll it: only inspect its two task units, then drop both effects.
        assert_eq!(task.units(), 2); // Fresh Run plus unchanged loader effect.
        drop(task);
        assert_eq!(host.generation(), 1);
        unavailable(&host.window_capture);
        assert!(host.window_capture.has_pending_for_test());
        assert_eq!(host.window_capture.root_for_test(), Some(root));
        // Further completion wakes cannot issue another query in generation1.
        assert_eq!(host.update(Event::Completion).units(), 0);
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn real_host_diagnostics_require_current_exact_ack_and_clear_on_close() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        Arc::make_mut(&mut host.frontend.data).window_observation = Some(Arc::new(serde_json::json!({})));
        let file = FileJournal::new(4, 4096);
        file.attach(&mut host.window_capture);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.update(Event::WindowCapture(event)), root);
        let event = reply(&host.window_capture, 91, Instant::now());
        drop(host.update(Event::WindowCapture(event)));
        assert!(host.observation_value().unwrap()["owned_window_candidate"].get("window_number").is_none());
        file.finish();
        assert!(host.observation_value().unwrap()["owned_window_candidate"].get("window_number").is_none());
        drop(host.update(Event::Completion));
        let observation = host.observation_value().unwrap();
        assert_eq!(observation["owned_window_candidate"]["status"], "DURABLY_RECORDED");
        assert_eq!(observation["owned_window_candidate"]["generation"], host.generation());
        assert_eq!(observation["owned_window_candidate"]["window_number"], 91);
        let event = host.window_capture.closed_for_test(root);
        drop(host.update(Event::WindowCapture(event)));
        assert!(host.observation_value().unwrap()["owned_window_candidate"].get("window_number").is_none());
        drop(host.recover_for_test());
        Ok(())
    })
}

#[test]
fn sealed_close_preserves_accepted_worker_and_queue_drain() -> Result<()> {
    run_on_stack(|| {
        let mut host = loading_host(true);
        let root = window::Id::unique();
        let event = host.window_capture.opened_for_test(root);
        assert_run(host.update(Event::WindowCapture(event)), root);
        let late = reply(&host.window_capture, 91, Instant::now());
        let (entered, resume) = host.gate_next_for_test();
        drop(host.update(Event::Widget { generation: 0, message: Message::Shift(true) }));
        entered.recv_timeout(Duration::from_secs(5))?;
        drop(host.update(Event::Widget { generation: 0, message: Message::FocusLost }));
        let event = host.window_capture.close_requested_for_test(root);
        assert_eq!(host.update(Event::WindowCapture(event)).units(), 0);
        assert!(host.worker.as_ref().unwrap().busy());
        assert_eq!(host.queued.len(), 1);
        assert!(!host.close_sent);
        unavailable(&host.window_capture);
        drop(host.update(Event::WindowCapture(late)));
        resume.send(())?;
        host.settle_for_test()?;
        assert_close(host.close_when_idle(), root);
        let owner = host.recover_for_test();
        assert!(!owner.shift);
        assert_eq!(owner.widget_epoch, 1);
        Ok(())
    })
}

#[test]
fn ack_already_on_disk_but_first_polled_at_deadline_stays_unavailable() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let file = FileJournal::new(4, 4096);
    file.attach(&mut capture);
    open(&mut capture, window::Id::unique(), now);
    let event = reply(&capture, 91, now);
    drop(capture.handle(event, now));
    file.finish();
    assert_eq!(file.rows()[0]["data"]["window_id"], 91);
    capture.tick(now + Duration::from_secs(2));
    unavailable(&capture);
    capture.tick(now);
    unavailable(&capture);
}

#[test]
fn generation_number_reuse_cannot_revive_old_reply_or_publication() {
    let now = Instant::now();
    let mut capture = Capture::new(true);
    let root = window::Id::unique();
    let old_file = FileJournal::new(4, 4096);
    old_file.attach(&mut capture);
    open(&mut capture, root, now);
    let old_reply = reply(&capture, 11, now);
    drop(capture.handle(old_reply.clone(), now));
    assert_run(capture.synchronize(1, now), root);
    assert_run(capture.synchronize(0, now), root);
    let new_file = FileJournal::new(4, 4096);
    new_file.attach(&mut capture);
    drop(capture.handle(old_reply, now));
    assert!(capture.has_pending_for_test());
    let current = reply(&capture, 91, now);
    drop(capture.handle(current, now));
    old_file.finish(); capture.tick(now);
    unavailable(&capture);
    new_file.finish(); capture.tick(now);
    assert_eq!(capture.diagnostics()["window_number"], 91);
    assert_eq!(capture.diagnostics()["generation"], 0);
}
