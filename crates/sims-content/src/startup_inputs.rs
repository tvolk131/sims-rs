use crate::{Resource, avatar_mount::ArchiveInput, resource_scan::Scan};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_EVENTS: usize = 32_768;
pub(crate) const MAX_BYTES: usize = 1024 * 1024 * 1024;

#[derive(Clone)]
enum Event {
    Begin(u64),
    End(u64),
    Resource {
        scope: u64,
        resource: Resource,
        bytes: Arc<[u8]>,
    },
    Archive {
        path: PathBuf,
        identity: u32,
        input: Arc<ArchiveInput>,
    },
}

enum Prefix {
    Begin(u64),
    End(u64),
    Resource {
        scope: u64,
        resource: Resource,
        result: Result<Arc<[u8]>>,
    },
    Archive {
        path: PathBuf,
        identity: u32,
        result: io::Result<Arc<ArchiveInput>>,
    },
}

#[derive(Clone)]
pub(crate) struct Tape {
    events: Arc<[Event]>,
    retained_bytes: usize,
}

struct State {
    scans: BTreeMap<u64, Scan>,
    next_scope: u64,
    prefix: VecDeque<Prefix>,
    events: Vec<Event>,
    retained_bytes: usize,
    limit: usize,
    eligible: bool,
    fault: bool,
    finished: bool,
    #[cfg(test)]
    observer: Option<Observer>,
    #[cfg(test)]
    observed: usize,
}

#[cfg(test)]
type Observer = Box<dyn FnMut(usize, &Path, u64) + Send>;

impl State {
    #[cfg(test)]
    fn observe(&mut self, path: &Path, offset: u64) {
        if let Some(observer) = &mut self.observer {
            observer(self.observed, path, offset);
        }
        self.observed += 1;
    }

    fn record(&mut self, event: Event) {
        if !self.eligible {
            return;
        }
        let bytes = match &event {
            Event::Begin(_) | Event::End(_) => 32,
            Event::Resource {
                resource, bytes, ..
            } => bytes
                .len()
                .saturating_add(resource.locator.len())
                .saturating_add(resource.name.len())
                .saturating_add(resource.source.as_os_str().len())
                .saturating_add(256),
            Event::Archive { path, input, .. } => input
                .len()
                .saturating_add(path.as_os_str().len())
                .saturating_add(128),
        };
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        if self.retained_bytes > self.limit || self.events.len() >= MAX_EVENTS {
            self.ineligible();
        } else {
            self.events.push(event);
        }
    }

    fn ineligible(&mut self) {
        self.eligible = false;
        self.events.clear();
        self.retained_bytes = 0;
    }

    fn fail(&mut self) {
        self.fault = true;
        self.ineligible();
    }
}

#[derive(Clone)]
pub(crate) struct Session(Arc<Mutex<State>>);

pub(crate) enum Validation {
    Unchanged,
    Changed(Session),
}

fn same_resource(left: &Resource, right: &Resource) -> bool {
    left.locator == right.locator
        && left.name == right.name
        && left.source == right.source
        && left.offset == right.offset
        && left.size == right.size
        && left.unpacked_size == right.unpacked_size
        && left.archived == right.archived
        && left.owned == right.owned
}

impl Session {
    pub(crate) fn capture(limit: usize) -> Self {
        Self(Arc::new(Mutex::new(State {
            scans: BTreeMap::new(),
            next_scope: 1,
            prefix: VecDeque::new(),
            events: Vec::new(),
            retained_bytes: 0,
            limit,
            eligible: true,
            fault: false,
            finished: false,
            #[cfg(test)]
            observer: None,
            #[cfg(test)]
            observed: 0,
        })))
    }

    #[cfg(test)]
    pub(crate) fn observe_before(&self, observer: impl FnMut(usize, &Path, u64) + Send + 'static) {
        self.0.lock().unwrap().observer = Some(Box::new(observer));
    }

    pub(crate) fn begin(&self) -> Result<u64> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("startup input lock poisoned"))?;
        ensure!(!state.finished, "startup input session already finished");
        let scope = if let Some(event) = state.prefix.pop_front() {
            match event {
                Prefix::Begin(scope) => scope,
                _ => {
                    state.fail();
                    bail!("startup input replay scope order changed");
                }
            }
        } else {
            let scope = state.next_scope;
            state.next_scope = scope
                .checked_add(1)
                .context("startup input scope exhausted")?;
            state.scans.insert(scope, Scan::default());
            scope
        };
        state.record(Event::Begin(scope));
        Ok(scope)
    }

    pub(crate) fn end(&self, scope: u64) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.finished {
            state.fail();
            return;
        }
        if let Some(event) = state.prefix.pop_front() {
            if !matches!(event, Prefix::End(expected) if expected == scope) {
                state.fail();
                return;
            }
        } else if state.scans.remove(&scope).is_none() {
            state.fail();
            return;
        }
        state.record(Event::End(scope));
    }

    pub(crate) fn read(&self, scope: u64, resource: &Resource) -> Result<Vec<u8>> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("startup input lock poisoned"))?;
        ensure!(!state.finished, "startup input session already finished");
        if let Some(event) = state.prefix.pop_front() {
            let Prefix::Resource {
                scope: expected_scope,
                resource: expected_resource,
                result,
            } = event
            else {
                state.fail();
                bail!("startup input replay operation changed");
            };
            if expected_scope != scope || !same_resource(resource, &expected_resource) {
                state.fail();
                bail!("startup input replay resource changed");
            }
            return match result {
                Ok(bytes) => {
                    state.record(Event::Resource {
                        scope,
                        resource: expected_resource,
                        bytes: bytes.clone(),
                    });
                    Ok(bytes.to_vec())
                }
                Err(error) => {
                    state.ineligible();
                    Err(error)
                }
            };
        }
        #[cfg(test)]
        state.observe(&resource.source, resource.offset);
        let result = state
            .scans
            .get_mut(&scope)
            .context("startup input scope absent")?
            .read(resource);
        match &result {
            Ok(bytes)
                if state.eligible
                    && bytes.len() <= state.limit.saturating_sub(state.retained_bytes) =>
            {
                state.record(Event::Resource {
                    scope,
                    resource: resource.clone(),
                    bytes: Arc::from(bytes.as_slice()),
                });
            }
            _ => state.ineligible(),
        }
        result
    }

    pub(crate) fn archive(
        &self,
        path: &Path,
        identity: u32,
        read: impl FnOnce() -> io::Result<ArchiveInput>,
    ) -> io::Result<ArchiveInput> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("startup input lock poisoned"))?;
        if state.finished {
            state.fail();
            return Err(io::Error::other("startup input session already finished"));
        }
        if let Some(event) = state.prefix.pop_front() {
            let Prefix::Archive {
                path: expected_path,
                identity: expected_identity,
                result,
            } = event
            else {
                state.fail();
                return Err(io::Error::other("startup input replay operation changed"));
            };
            if path != expected_path || identity != expected_identity {
                state.fail();
                return Err(io::Error::other("startup input replay archive changed"));
            }
            return match result {
                Ok(input) => {
                    state.record(Event::Archive {
                        path: expected_path,
                        identity,
                        input: input.clone(),
                    });
                    Ok((*input).clone())
                }
                Err(error) => {
                    state.ineligible();
                    Err(error)
                }
            };
        }
        #[cfg(test)]
        state.observe(path, 0);
        let result = read();
        match &result {
            Ok(input)
                if state.eligible
                    && input.len() <= state.limit.saturating_sub(state.retained_bytes) =>
            {
                state.record(Event::Archive {
                    path: path.into(),
                    identity,
                    input: Arc::new(input.clone()),
                });
            }
            _ => state.ineligible(),
        }
        result
    }

    pub(crate) fn finish(&self) -> Result<Option<Tape>> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("startup input lock poisoned"))?;
        ensure!(!state.finished, "startup input session already finished");
        state.finished = true;
        ensure!(
            !state.fault && state.prefix.is_empty() && state.scans.is_empty(),
            "startup input replay did not close cleanly"
        );
        Ok(state.eligible.then(|| Tape {
            events: std::mem::take(&mut state.events).into(),
            retained_bytes: state.retained_bytes,
        }))
    }
}

impl Tape {
    #[cfg(test)]
    pub(crate) fn same_reference(&self, reference: &Self) -> bool {
        Arc::ptr_eq(&self.events, &reference.events)
    }

    pub(crate) fn statistics(&self) -> (usize, usize) {
        (self.events.len(), self.retained_bytes)
    }

    pub(crate) fn validate(&self, limit: usize) -> Result<Validation> {
        self.validate_before(limit, |_, _| {})
    }

    fn validate_before(
        &self,
        limit: usize,
        mut before: impl FnMut(usize, &Event),
    ) -> Result<Validation> {
        let session = Session::capture(limit);
        self.validate_in(session, &mut before)
    }

    #[cfg(test)]
    pub(crate) fn validate_observing(
        &self,
        limit: usize,
        observer: impl FnMut(usize, &Path, u64) + Send + 'static,
    ) -> Result<Validation> {
        let session = Session::capture(limit);
        session.observe_before(observer);
        self.validate_in(session, |_, _| {})
    }

    fn validate_in(
        &self,
        session: Session,
        mut before: impl FnMut(usize, &Event),
    ) -> Result<Validation> {
        for (position, event) in self.events.iter().enumerate() {
            before(position, event);
            let mut state = session
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("startup input lock poisoned"))?;
            let (prefix, matches) = match event {
                Event::Begin(scope) => {
                    ensure!(
                        !state.scans.contains_key(scope),
                        "duplicate startup input scope"
                    );
                    state.next_scope = scope
                        .checked_add(1)
                        .context("startup input scope exhausted")?;
                    state.scans.insert(*scope, Scan::default());
                    (Prefix::Begin(*scope), true)
                }
                Event::End(scope) => {
                    ensure!(
                        state.scans.remove(scope).is_some(),
                        "startup input scope absent"
                    );
                    (Prefix::End(*scope), true)
                }
                Event::Resource {
                    scope,
                    resource,
                    bytes,
                } => {
                    #[cfg(test)]
                    state.observe(&resource.source, resource.offset);
                    let result = state
                        .scans
                        .get_mut(scope)
                        .context("startup input scope absent")?
                        .read(resource);
                    let matches = result
                        .as_ref()
                        .is_ok_and(|fresh| fresh.as_slice() == bytes.as_ref());
                    let result = result.map(|fresh| {
                        if matches {
                            bytes.clone()
                        } else {
                            Arc::from(fresh)
                        }
                    });
                    (
                        Prefix::Resource {
                            scope: *scope,
                            resource: resource.clone(),
                            result,
                        },
                        matches,
                    )
                }
                Event::Archive {
                    path,
                    identity,
                    input,
                } => {
                    #[cfg(test)]
                    state.observe(path, 0);
                    let result = crate::avatar_mount::read_archive_input(path);
                    let matches = result.as_ref().is_ok_and(|fresh| fresh == input.as_ref());
                    let result = result.map(|fresh| {
                        if matches {
                            input.clone()
                        } else {
                            Arc::new(fresh)
                        }
                    });
                    (
                        Prefix::Archive {
                            path: path.clone(),
                            identity: *identity,
                            result,
                        },
                        matches,
                    )
                }
            };
            state.prefix.push_back(prefix);
            if !matches {
                drop(state);
                return Ok(Validation::Changed(session));
            }
        }
        ensure!(
            session
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("startup input lock poisoned"))?
                .scans
                .is_empty(),
            "startup input scopes remain open"
        );
        Ok(Validation::Unchanged)
    }
}

#[cfg(test)]
mod tests;
