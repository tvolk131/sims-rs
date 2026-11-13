//! Host-owned observation consumer. A durable historical row alone grants
//! nothing: the exact acknowledgement must still match this Host/root/world.
//! No discovery, execution authority, retries, window creation or root adoption.
use super::window_identity::{self, WindowIdentity};
use super::super::support::window_session::{self, CandidatePublication, PublicationStatus};
use iced::{Subscription, Task, window};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
struct Lifetime;

#[derive(Clone, Debug)]
struct Seal(Arc<Lifetime>);
impl PartialEq for Seal {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl Eq for Seal {}
impl Hash for Seal {
    fn hash<H: Hasher>(&self, state: &mut H) { Arc::as_ptr(&self.0).hash(state); }
}

/// Opaque Host-lifetime envelope. No constructor, managed Id or seal can be
/// supplied by an external caller or deserialized as observation authority.
#[derive(Clone, Debug)]
pub(super) struct Event {
    seal: Seal,
    kind: Kind,
}
#[derive(Clone, Debug)]
enum Kind {
    Opened(window::Id),
    CloseRequested(window::Id),
    Closed(window::Id),
    Reply(window_identity::Reply),
}
fn opened((seal, id): (Seal, window::Id)) -> Event { Event { seal, kind: Kind::Opened(id) } }
fn close_requested((seal, id): (Seal, window::Id)) -> Event { Event { seal, kind: Kind::CloseRequested(id) } }
fn closed((seal, id): (Seal, window::Id)) -> Event { Event { seal, kind: Kind::Closed(id) } }

struct Publication {
    seal: Seal,
    root: window::Id,
    generation: u64,
    number: u32,
    acknowledgement: CandidatePublication,
    status: PublicationStatus,
}

pub(super) struct Outcome {
    pub task: Task<Event>,
    pub close_requested: Option<window::Id>,
}
impl Outcome {
    fn idle() -> Self { Self { task: Task::none(), close_requested: None } }
}

pub(super) struct Capture {
    seal: Seal,
    recording: bool,
    root: Option<window::Id>,
    generation: u64,
    ended: bool,
    identity: Option<WindowIdentity>,
    publication: Option<Publication>,
    #[cfg(test)]
    pub(super) journal_for_test: Option<Arc<std::sync::Mutex<window_session::candidate_journal_fixture::Journal>>>,
}
impl Capture {
    pub fn new(recording: bool) -> Self {
        Self {
            seal: Seal(Arc::new(Lifetime)), recording, root: None,
            generation: 0, ended: false, identity: None, publication: None,
            #[cfg(test)]
            journal_for_test: None,
        }
    }

    pub fn subscription(&self) -> Subscription<Event> {
        // .with hashes the allocation identity, matching equality above. The
        // lifetime stays stable across Load and different Host subscriptions
        // cannot reuse each other's live recipe or queued lifecycle messages.
        Subscription::batch([
            window::open_events().with(self.seal.clone()).map(opened),
            window::close_requests().with(self.seal.clone()).map(close_requested),
            window::close_events().with(self.seal.clone()).map(closed),
        ])
    }

    fn request(&mut self, now: Instant) -> Task<Event> {
        let Some(identity) = &mut self.identity else { return Task::none(); };
        let Ok(task) = identity.request(now) else { return Task::none(); };
        let seal = self.seal.clone();
        task.map(move |reply| Event { seal: seal.clone(), kind: Kind::Reply(reply) })
    }

    /// Called immediately after every worker poll, before any incoming reply
    /// or durable ack is considered. Only an actual change permits one query.
    pub fn synchronize(&mut self, generation: u64, now: Instant) -> Task<Event> {
        if self.generation == generation { return Task::none(); }
        self.generation = generation;
        self.publication = None;
        let changed = self.identity.as_mut().is_some_and(|identity|
            identity.set_generation(generation) == Ok(true));
        if changed && !self.ended { self.request(now) } else { Task::none() }
    }

    fn current(&self, publication: &Publication) -> bool {
        !self.ended && self.recording && publication.seal == self.seal
            && Some(publication.root) == self.root
            && publication.generation == self.generation
            && self.identity.as_ref().is_some_and(|identity|
                identity.root() == publication.root && identity.generation() == publication.generation)
    }

    /// Always invoked by Host updates (including frame Ticks without callbacks).
    /// Neither query expiry nor failed publication ever schedules a retry.
    pub fn tick(&mut self, now: Instant) {
        if let Some(identity) = &mut self.identity { identity.expire(now); }
        let current = self.publication.as_ref().is_some_and(|p| self.current(p));
        if !current { self.publication = None; return; }
        let publication = self.publication.as_mut().unwrap();
        publication.status = publication.acknowledgement.poll(now);
        if publication.status == PublicationStatus::Unavailable { self.publication = None; }
    }

    fn end(&mut self, id: window::Id, is_closed: bool) -> bool {
        if self.root != Some(id) { return false; }
        self.ended = true;
        self.publication = None;
        if let Some(identity) = &mut self.identity {
            if is_closed { identity.closed(id); } else { identity.close_intent(id); }
        }
        true
    }

    fn publish(&self, number: u32, generation: u64, _now: Instant) -> Option<CandidatePublication> {
        #[cfg(test)]
        if let Some(journal) = &self.journal_for_test {
            return journal.lock().unwrap().enqueue(number, generation, _now);
        }
        window_session::window_number_candidate(number, generation)
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Outcome {
        if event.seal != self.seal { return Outcome::idle(); }
        let mut outcome = Outcome::idle();
        match event.kind {
            Kind::Opened(id) => {
                if self.root.is_none() && !self.ended {
                    self.root = Some(id);
                    if self.recording {
                        self.identity = Some(WindowIdentity::new(id, self.generation));
                        outcome.task = self.request(now);
                    }
                }
            }
            Kind::CloseRequested(id) => {
                // Invalidate before the Host even considers its Save drain.
                if self.end(id, false) { outcome.close_requested = Some(id); }
            }
            Kind::Closed(id) => { self.end(id, true); }
            Kind::Reply(reply) => {
                if let Some(identity) = &mut self.identity
                    && let Ok(candidate) = identity.accept(reply, now)
                {
                    // Retain the exact ack and its association, never the leaf
                    // candidate. Queue success still exposes no window number.
                    let root = candidate.root();
                    let generation = candidate.generation();
                    let number = candidate.window_number();
                    drop(candidate);
                    self.publication = self.publish(number, generation, now).map(|acknowledgement|
                        Publication { seal: self.seal.clone(), root, generation, number,
                            acknowledgement, status: PublicationStatus::Pending });
                }
            }
        }
        outcome
    }

    /// Only observational diagnostics, never kernel/execution authority. No
    /// pending number is exposed, and historical journal rows are never read.
    pub fn diagnostics(&self) -> serde_json::Value {
        if let Some(p) = &self.publication
            && self.current(p) && p.status == PublicationStatus::DurablyRecorded
        {
            return serde_json::json!({"status":"DURABLY_RECORDED", "generation":p.generation,
                "window_number":p.number,
                "scope":"observational candidate; independent retained-execution and exact-window verification required"});
        }
        serde_json::json!({"status":"UNAVAILABLE", "scope":"observational candidate"})
    }
}

#[cfg(test)]
impl Capture {
    pub(super) fn opened_for_test(&self, id: window::Id) -> Event { opened((self.seal.clone(), id)) }
    pub(super) fn close_requested_for_test(&self, id: window::Id) -> Event { close_requested((self.seal.clone(), id)) }
    pub(super) fn closed_for_test(&self, id: window::Id) -> Event { closed((self.seal.clone(), id)) }
    pub(super) fn pending_reply_for_test(&self, result: Result<u32, window_identity::Unavailable>, now: Instant) -> Option<Event> {
        Some(Event { seal: self.seal.clone(), kind: Kind::Reply(self.identity.as_ref()?.pending_reply_for_test(result, now)?) })
    }
    pub(super) fn has_pending_for_test(&self) -> bool {
        self.identity.as_ref().is_some_and(WindowIdentity::has_pending_query)
    }
    pub(super) fn root_for_test(&self) -> Option<window::Id> { self.root }
    pub(super) fn same_seal_for_test(&self, other: &Self) -> bool { self.seal == other.seal }
    pub(super) fn seal_hash_for_test(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.seal.hash(&mut hash);
        hash.finish()
    }
}
