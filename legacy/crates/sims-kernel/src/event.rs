//! Anonymous Rust-port event ownership. No names, inherited handles, native
//! handle adoption, APCs, process sharing or Windows kernel interoperation.
//!
//! `begin_wait` registers a real pending call immediately. A pulse releases
//! only registrations present at that instant; it leaves no future credit.
//! All waits use the host's monotonic clock. Explicit retirement requires no
//! pending waits. Dropping the owner aborts pending port calls with Retired;
//! this is defined port cleanup, not a claim about Windows close-during-wait.
use std::{
    sync::{Arc, Condvar, Mutex, Weak},
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

pub const MAX_PENDING_WAITS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeout {
    /// Zero polls immediately. The reserved DWORD MAX must use Infinite.
    Milliseconds(u32),
    Infinite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Retired,
    Poisoned,
    WaitCapacity,
    ClockRange,
    ReservedTimeout,
    LostRegistration,
    WaitsPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Signaled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Pending,
    Ready(Outcome),
}

#[derive(Debug)]
struct Registration {
    id: u64,
    released: bool,
    deadline: Option<Instant>,
}

#[derive(Debug)]
struct State {
    signaled: bool,
    retired: bool,
    next: u64,
    waits: Vec<Registration>,
}

#[derive(Debug)]
struct Inner {
    mode: ResetMode,
    state: Mutex<State>,
    changed: Condvar,
}

/// Opaque lifetime identity. It cannot keep an event open or construct/adopt
/// another event from an address, integer handle or serialized observation.
#[derive(Clone, Debug)]
pub struct Identity(Weak<Inner>);

impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Identity {}

/// The sole owning event handle. Pending waits retain their issuing object,
/// but cannot signal, duplicate, rebind or keep its handle open after Drop.
#[derive(Debug)]
pub struct Event {
    inner: Arc<Inner>,
}

impl Event {
    pub fn new(mode: ResetMode, initially_signaled: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                mode,
                state: Mutex::new(State {
                    signaled: initially_signaled,
                    retired: false,
                    next: 1,
                    waits: Vec::new(),
                }),
                changed: Condvar::new(),
            }),
        }
    }

    pub fn identity(&self) -> Identity {
        Identity(Arc::downgrade(&self.inner))
    }

    pub fn has_identity(&self, identity: &Identity) -> bool {
        Weak::ptr_eq(&Arc::downgrade(&self.inner), &identity.0)
    }

    /// Read-only lifetime check; unlike a zero-time wait this consumes no
    /// signal and registers no caller.
    pub fn check_live(&self) -> Result<(), Error> {
        let state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            Err(Error::Retired)
        } else {
            Ok(())
        }
    }

    pub fn begin_wait(&self, timeout: Timeout) -> Result<Wait, Error> {
        let deadline = match timeout {
            Timeout::Infinite => None,
            Timeout::Milliseconds(u32::MAX) => return Err(Error::ReservedTimeout),
            Timeout::Milliseconds(ms) => Some(
                Instant::now()
                    .checked_add(Duration::from_millis(u64::from(ms)))
                    .ok_or(Error::ClockRange)?,
            ),
        };
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            return Err(Error::Retired);
        }
        let completion = if state.signaled {
            if self.inner.mode == ResetMode::Auto {
                state.signaled = false;
            }
            Some(Ok(Outcome::Signaled))
        } else if timeout == Timeout::Milliseconds(0) {
            Some(Ok(Outcome::TimedOut))
        } else {
            None
        };
        let ticket = if completion.is_none() {
            if state.waits.len() == MAX_PENDING_WAITS {
                return Err(Error::WaitCapacity);
            }
            let next = state.next.checked_add(1).ok_or(Error::WaitCapacity)?;
            state
                .waits
                .try_reserve(1)
                .map_err(|_| Error::WaitCapacity)?;
            let id = state.next;
            state.waits.push(Registration {
                id,
                released: false,
                deadline,
            });
            state.next = next;
            Some(id)
        } else {
            None
        };
        Ok(Wait {
            inner: Arc::clone(&self.inner),
            ticket,
            deadline,
            completion,
        })
    }

    pub fn set(&self) -> Result<(), Error> {
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            return Err(Error::Retired);
        }
        let now = Instant::now();
        match self.inner.mode {
            ResetMode::Manual => {
                state.signaled = true;
                for wait in &mut state.waits {
                    if wait.deadline.is_none_or(|deadline| now < deadline) {
                        wait.released = true;
                    }
                }
            }
            ResetMode::Auto => {
                if let Some(wait) = state.waits.iter_mut().find(|wait| {
                    !wait.released && wait.deadline.is_none_or(|deadline| now < deadline)
                }) {
                    wait.released = true;
                    state.signaled = false;
                } else {
                    // Multiple sets without a waiter coalesce to one credit.
                    state.signaled = true;
                }
            }
        }
        self.inner.changed.notify_all();
        Ok(())
    }

    pub fn reset(&self) -> Result<(), Error> {
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            return Err(Error::Retired);
        }
        state.signaled = false;
        Ok(())
    }

    pub fn pulse(&self) -> Result<(), Error> {
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            return Err(Error::Retired);
        }
        let now = Instant::now();
        for wait in &mut state.waits {
            if !wait.released && wait.deadline.is_none_or(|deadline| now < deadline) {
                wait.released = true;
                if self.inner.mode == ResetMode::Auto {
                    break;
                }
            }
        }
        state.signaled = false;
        self.inner.changed.notify_all();
        Ok(())
    }

    /// Retain the live owner on failure; source cleanup must first settle or
    /// cancel its own waits. A repeated retirement fails instead of succeeding
    /// as if another handle had been closed.
    pub fn retire(&mut self) -> Result<(), Error> {
        let mut state = self.inner.state.lock().map_err(|_| Error::Poisoned)?;
        if state.retired {
            return Err(Error::Retired);
        }
        if !state.waits.is_empty() {
            return Err(Error::WaitsPending);
        }
        state.retired = true;
        state.signaled = false;
        Ok(())
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // Cleanup must wake blocked calls even if a previous client panicked
        // with the mutex held. Public operations still report poisoning.
        let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state.retired = true;
        state.signaled = false;
        self.inner.changed.notify_all();
    }
}

/// Non-cloneable pending call with issuing-object identity, registration and
/// deadline. Poll/retry never registers twice or consumes a second signal.
#[derive(Debug)]
pub struct Wait {
    inner: Arc<Inner>,
    ticket: Option<u64>,
    deadline: Option<Instant>,
    completion: Option<Result<Outcome, Error>>,
}

impl Wait {
    fn resolve(&mut self, state: &mut State) -> Result<Poll, Error> {
        if let Some(result) = self.completion {
            return result.map(Poll::Ready);
        }
        let index = self
            .ticket
            .and_then(|id| state.waits.iter().position(|wait| wait.id == id));
        let result = if state.retired {
            Err(Error::Retired)
        } else if let Some(index) = index {
            if state.waits[index].released {
                Ok(Outcome::Signaled)
            } else if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                Ok(Outcome::TimedOut)
            } else {
                return Ok(Poll::Pending);
            }
        } else {
            Err(Error::LostRegistration)
        };
        if let Some(index) = index {
            state.waits.remove(index);
        }
        self.ticket = None;
        self.completion = Some(result);
        result.map(Poll::Ready)
    }

    pub fn poll(&mut self) -> Result<Poll, Error> {
        if let Some(result) = self.completion {
            return result.map(Poll::Ready);
        }
        let inner = Arc::clone(&self.inner);
        let mut state = inner.state.lock().map_err(|_| Error::Poisoned)?;
        self.resolve(&mut state)
    }

    pub fn wait(&mut self) -> Result<Outcome, Error> {
        self.wait_impl(|| {})
    }

    fn wait_impl(&mut self, mut before_sleep: impl FnMut()) -> Result<Outcome, Error> {
        if let Some(result) = self.completion {
            return result;
        }
        let inner = Arc::clone(&self.inner);
        let mut state = inner.state.lock().map_err(|_| Error::Poisoned)?;
        loop {
            if let Poll::Ready(result) = self.resolve(&mut state)? {
                return Ok(result);
            }
            before_sleep();
            state = match self.deadline {
                None => inner.changed.wait(state).map_err(|_| Error::Poisoned)?,
                Some(deadline) => {
                    let duration = deadline.saturating_duration_since(Instant::now());
                    inner
                        .changed
                        .wait_timeout(state, duration)
                        .map_err(|_| Error::Poisoned)?
                        .0
                }
            };
        }
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        if let Some(id) = self.ticket.take() {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = state.waits.iter().position(|wait| wait.id == id) {
                state.waits.remove(index);
            }
        }
    }
}
