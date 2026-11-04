//! Single mutable owner and a capacity-one work channel. Poll and drop never
//! wait for the owner. The last coherent immutable publication remains usable
//! while an indivisible original application update is running.
use std::sync::{Arc, mpsc};
use std::thread;

pub(super) trait Owner: Send + 'static {
    type Command: Send + 'static;
    type Reply: Send + 'static;
    type Published: Send + Sync + 'static;
    fn apply(&mut self, command: Self::Command) -> Result<Self::Reply, String>;
    fn publish(&self) -> Result<Self::Published, String>;
    fn replaces(command: &Self::Command) -> bool;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    pub generation: u64,
    pub serial: u64,
}
pub(super) struct Completion<O: Owner> {
    pub ticket: Ticket,
    pub generation: u64,
    pub reply: Result<O::Reply, String>,
    pub publication: Option<Arc<O::Published>>,
    pub quarantined: bool,
}
struct Request<O: Owner> {
    ticket: Ticket,
    command: O::Command,
    notify: Option<Box<dyn FnOnce() + Send>>,
}
pub(super) struct StartError<O> {
    pub owner: O,
    pub error: String,
}
pub(super) enum SubmitError<C> {
    Busy(C),
    Disconnected(C),
    Quarantined(C),
    Exhausted(C),
}
#[derive(Debug)]
pub(super) enum PollError {
    Disconnected,
    UnexpectedTicket {
        expected: Option<Ticket>,
        actual: Ticket,
    },
}
impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("owner disconnected"),
            Self::UnexpectedTicket { expected, actual } => {
                write!(f, "expected completion {expected:?}, received {actual:?}")
            }
        }
    }
}
pub(super) struct Worker<O: Owner> {
    requests: Option<mpsc::SyncSender<Request<O>>>,
    completions: mpsc::Receiver<Completion<O>>,
    #[cfg(test)]
    thread: Option<thread::JoinHandle<O>>,
    published: Arc<O::Published>,
    generation: u64,
    next_serial: u64,
    flight: Option<Ticket>,
    quarantined: bool,
    disconnected: bool,
}
impl<O: Owner> Worker<O> {
    pub fn start(owner: O) -> Result<Self, StartError<O>> {
        let published = match owner.publish() {
            Ok(p) => Arc::new(p),
            Err(error) => return Err(StartError { owner, error }),
        };
        let (initial_tx, initial_rx) = mpsc::sync_channel::<O>(1);
        let (requests, incoming) = mpsc::sync_channel::<Request<O>>(1);
        let (results, completions) = mpsc::channel();
        let thread = match thread::Builder::new()
            .name("sims-household-owner".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let mut owner = initial_rx.recv().expect("initial owner sender disappeared");
                let mut generation = 0u64;
                let mut quarantined = false;
                while let Ok(request) = incoming.recv() {
                    let Request {
                        ticket,
                        command,
                        notify,
                    } = request;
                    // A panic still terminates the owner. Close the result
                    // channel before waking the UI, so its one wake cannot
                    // race an incompletely disconnected sender.
                    let completion = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut publication = None;
                        let replacing = O::replaces(&command);
                        let reply = if ticket.generation != generation {
                            Err(format!(
                                "stale owner generation {}; current {}",
                                ticket.generation, generation
                            ))
                        } else if quarantined {
                            Err("engine worker requires explicit application restart".into())
                        } else if replacing && generation == u64::MAX {
                            quarantined = true;
                            Err("engine generation exhausted".into())
                        } else {
                            match owner.apply(command) {
                                Ok(reply) => match owner.publish() {
                                    Ok(p) => {
                                        if replacing {
                                            generation += 1;
                                        }
                                        publication = Some(Arc::new(p));
                                        Ok(reply)
                                    }
                                    Err(error) => {
                                        quarantined = true;
                                        Err(error)
                                    }
                                },
                                // No rollback or retry after a partially changed owner.
                                Err(error) => {
                                    quarantined = true;
                                    Err(error)
                                }
                            }
                        };
                        Completion {
                            ticket,
                            generation,
                            reply,
                            publication,
                            quarantined,
                        }
                    }));
                    match completion {
                        Ok(completion) => {
                            let sent = results.send(completion).is_ok();
                            if let Some(notify) = notify {
                                notify();
                            }
                            if !sent {
                                break;
                            }
                        }
                        Err(panic) => {
                            drop(results);
                            if let Some(notify) = notify {
                                notify();
                            }
                            std::panic::resume_unwind(panic);
                        }
                    }
                }
                owner
            }) {
            Ok(thread) => thread,
            Err(error) => {
                return Err(StartError {
                    owner,
                    error: error.to_string(),
                });
            }
        };
        if let Err(error) = initial_tx.send(owner) {
            let _ = thread.join();
            return Err(StartError {
                owner: error.0,
                error: "worker initialization failed".into(),
            });
        }
        // The UI never joins the owner, including during application drop.
        #[cfg(not(test))]
        drop(thread);
        Ok(Self {
            requests: Some(requests),
            completions,
            #[cfg(test)]
            thread: Some(thread),
            published,
            generation: 0,
            next_serial: 1,
            flight: None,
            quarantined: false,
            disconnected: false,
        })
    }
    pub fn published(&self) -> &Arc<O::Published> {
        &self.published
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn busy(&self) -> bool {
        self.flight.is_some()
    }
    #[cfg(test)]
    pub fn submit(&mut self, command: O::Command) -> Result<Ticket, SubmitError<O::Command>> {
        self.submit_notifying(command, None)
    }
    pub fn submit_notifying(
        &mut self,
        command: O::Command,
        notify: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<Ticket, SubmitError<O::Command>> {
        if self.disconnected {
            return Err(SubmitError::Disconnected(command));
        }
        if self.busy() {
            return Err(SubmitError::Busy(command));
        }
        if self.quarantined {
            return Err(SubmitError::Quarantined(command));
        }
        let Some(next) = self.next_serial.checked_add(1) else {
            return Err(SubmitError::Exhausted(command));
        };
        let ticket = Ticket {
            generation: self.generation,
            serial: self.next_serial,
        };
        let Some(sender) = &self.requests else {
            return Err(SubmitError::Disconnected(command));
        };
        match sender.try_send(Request {
            ticket,
            command,
            notify,
        }) {
            Ok(()) => {
                self.next_serial = next;
                self.flight = Some(ticket);
                Ok(ticket)
            }
            Err(mpsc::TrySendError::Full(r)) => Err(SubmitError::Busy(r.command)),
            Err(mpsc::TrySendError::Disconnected(r)) => {
                self.disconnected = true;
                Err(SubmitError::Disconnected(r.command))
            }
        }
    }
    pub fn poll(&mut self) -> Result<Option<Completion<O>>, PollError> {
        if self.disconnected {
            return Err(PollError::Disconnected);
        }
        match self.completions.try_recv() {
            Ok(completion) => {
                if self.flight != Some(completion.ticket) {
                    self.disconnected = true;
                    self.requests.take();
                    return Err(PollError::UnexpectedTicket {
                        expected: self.flight,
                        actual: completion.ticket,
                    });
                }
                if let Some(p) = &completion.publication {
                    self.published = p.clone();
                }
                self.generation = completion.generation;
                self.quarantined = completion.quarantined;
                self.flight = None;
                Ok(Some(completion))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.disconnected = true;
                // The sender has gone; no owner call can still complete.
                // Keep the last good publication, but do not leave Close
                // waiting forever for a ticket whose thread has exited.
                self.flight = None;
                self.requests.take();
                Err(PollError::Disconnected)
            }
        }
    }
    /// Explicit test/off-UI shutdown. Drop only disconnects and never joins.
    #[cfg(test)]
    pub fn recover(mut self) -> thread::Result<O> {
        self.requests.take();
        self.thread.take().expect("worker thread absent").join()
    }
}

#[cfg(test)]
#[path = "owned_worker/tests.rs"]
mod tests;
