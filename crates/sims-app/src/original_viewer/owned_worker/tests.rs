use super::*;
use std::time::{Duration, Instant};

enum Command {
    Add(i32),
    Replace(i32),
    FailAfter(i32),
    Panic,
    Wait {
        started: mpsc::SyncSender<()>,
        resume: mpsc::Receiver<()>,
    },
}
struct Fake {
    value: i32,
    fail_publication: bool,
}
impl Owner for Fake {
    type Command = Command;
    type Reply = i32;
    type Published = i32;
    fn apply(&mut self, command: Command) -> Result<i32, String> {
        match command {
            Command::Add(n) => self.value += n,
            Command::Replace(n) => self.value = n,
            Command::FailAfter(n) => {
                self.value = n;
                return Err("after mutation".into());
            }
            Command::Panic => panic!("intentional test owner panic"),
            Command::Wait { started, resume } => {
                started.send(()).unwrap();
                resume.recv().unwrap();
                self.value += 1;
            }
        }
        Ok(self.value)
    }
    fn publish(&self) -> Result<i32, String> {
        if self.fail_publication {
            Err("publication failed".into())
        } else {
            Ok(self.value)
        }
    }
    fn replaces(command: &Command) -> bool {
        matches!(command, Command::Replace(_))
    }
}
fn start() -> Worker<Fake> {
    match Worker::start(Fake {
        value: 7,
        fail_publication: false,
    }) {
        Ok(w) => w,
        Err(_) => panic!("start failed"),
    }
}
fn submit(worker: &mut Worker<Fake>, command: Command) -> Ticket {
    match worker.submit(command) {
        Ok(t) => t,
        Err(_) => panic!("submit failed"),
    }
}
fn wait(worker: &mut Worker<Fake>) -> Completion<Fake> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match worker.poll() {
            Ok(Some(c)) => return c,
            Ok(None) => {}
            Err(e) => panic!("poll failed: {e:?}"),
        }
        assert!(Instant::now() < deadline, "worker did not complete");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn busy_owner_is_not_borrowed_and_publication_changes_atomically_after_completion() {
    let mut worker = start();
    let original = worker.published().clone();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let ticket = submit(
        &mut worker,
        Command::Wait {
            started: started_tx,
            resume: resume_rx,
        },
    );
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(worker.busy());
    assert!(worker.poll().unwrap().is_none());
    assert!(Arc::ptr_eq(&original, worker.published()));
    match worker.submit(Command::Add(5)) {
        Err(SubmitError::Busy(Command::Add(5))) => {}
        _ => panic!("busy command was not returned intact"),
    }
    resume_tx.send(()).unwrap();
    let completion = wait(&mut worker);
    assert_eq!(completion.ticket, ticket);
    assert_eq!(completion.reply.unwrap(), 8);
    assert_eq!(**worker.published(), 8);
    assert_eq!(*original, 7);
    assert!(!worker.busy());
    assert_eq!(worker.recover().ok().unwrap().value, 8);
}

#[test]
fn replacement_advances_generation_once_and_future_tickets_use_it() {
    let mut worker = start();
    let ticket = submit(&mut worker, Command::Replace(30));
    assert_eq!(ticket.generation, 0);
    let c = wait(&mut worker);
    assert_eq!(c.generation, 1);
    assert_eq!(worker.generation(), 1);
    let next = submit(&mut worker, Command::Add(2));
    assert_eq!(next.generation, 1);
    assert_eq!(wait(&mut worker).reply.unwrap(), 32);
    assert_eq!(worker.recover().ok().unwrap().value, 32);
}

#[test]
fn partial_apply_failure_preserves_the_last_publication_and_never_retries() {
    let mut worker = start();
    let original = worker.published().clone();
    submit(&mut worker, Command::FailAfter(22));
    let c = wait(&mut worker);
    assert!(c.reply.is_err() && c.publication.is_none() && c.quarantined);
    assert!(Arc::ptr_eq(&original, worker.published()));
    assert!(matches!(
        worker.submit(Command::Add(1)),
        Err(SubmitError::Quarantined(Command::Add(1)))
    ));
    assert_eq!(worker.recover().ok().unwrap().value, 22);
}

#[test]
fn serial_exhaustion_rejects_without_running_the_owner() {
    let mut worker = start();
    worker.next_serial = u64::MAX;
    assert!(matches!(
        worker.submit(Command::Add(1)),
        Err(SubmitError::Exhausted(Command::Add(1)))
    ));
    assert!(!worker.busy());
    assert_eq!(worker.recover().ok().unwrap().value, 7);
}

#[test]
fn failed_initial_publication_returns_the_exact_unstarted_owner() {
    match Worker::start(Fake {
        value: 91,
        fail_publication: true,
    }) {
        Err(error) => {
            assert_eq!(error.owner.value, 91);
            assert_eq!(error.error, "publication failed");
        }
        Ok(_) => panic!("failed publication started a worker"),
    }
}

#[test]
fn unexpected_completion_is_rejected_without_installing_its_publication() {
    let mut worker = start();
    let (tx, rx) = mpsc::channel();
    worker.completions = rx;
    let wrong = Ticket {
        generation: 0,
        serial: 77,
    };
    tx.send(Completion {
        ticket: wrong,
        generation: 3,
        reply: Ok(999),
        publication: Some(Arc::new(999)),
        quarantined: false,
    })
    .ok()
    .unwrap();
    assert!(
        matches!(worker.poll(),Err(PollError::UnexpectedTicket {expected:None,actual}) if actual==wrong)
    );
    assert_eq!(**worker.published(), 7);
    assert_eq!(worker.generation(), 0);
    assert!(matches!(
        worker.submit(Command::Add(1)),
        Err(SubmitError::Disconnected(Command::Add(1)))
    ));
    assert_eq!(worker.recover().ok().unwrap().value, 7);
}

#[test]
fn drop_disconnects_without_waiting_for_an_indivisible_owner_call() {
    let mut worker = start();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    submit(
        &mut worker,
        Command::Wait {
            started: started_tx,
            resume: resume_rx,
        },
    );
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let dropper = thread::spawn(move || {
        drop(worker);
        done_tx.send(()).unwrap();
    });
    done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    resume_tx.send(()).unwrap();
    dropper.join().unwrap();
}

#[test]
fn exited_owner_does_not_leave_a_permanently_busy_ticket() {
    let mut worker = start();
    let publication = worker.published().clone();
    submit(&mut worker, Command::Panic);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match worker.poll() {
            Err(PollError::Disconnected) => break,
            Ok(None) => {}
            _ => panic!("unexpected owner result"),
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!worker.busy());
    assert!(Arc::ptr_eq(&publication, worker.published()));
    assert!(matches!(
        worker.submit(Command::Add(1)),
        Err(SubmitError::Disconnected(_))
    ));
    assert!(worker.recover().is_err());
}

#[test]
fn single_notification_follows_queued_completion_and_disconnect() {
    for panic in [false, true] {
        let mut worker = start();
        let (notify, received) = mpsc::sync_channel(1);
        let command = if panic {
            Command::Panic
        } else {
            Command::Add(2)
        };
        assert!(
            worker
                .submit_notifying(
                    command,
                    Some(Box::new(move || {
                        notify.send(()).unwrap();
                    }))
                )
                .is_ok()
        );
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        if panic {
            assert!(matches!(worker.poll(), Err(PollError::Disconnected)));
            assert!(!worker.busy());
            assert!(worker.recover().is_err());
        } else {
            assert_eq!(worker.poll().unwrap().unwrap().reply.unwrap(), 9);
            assert!(worker.poll().unwrap().is_none());
            assert_eq!(worker.recover().ok().unwrap().value, 9);
        }
        assert!(matches!(
            received.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
